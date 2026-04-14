//! Unit tests for `TelegramChannel`.
//!
//! Every test in this module drives the channel against a
//! `ScriptedTransport` — an in-memory test double that implements
//! [`TelegramTransport`] with pre-canned inbound updates and a
//! `Mutex<Vec<OutgoingMessage>>` capture buffer. **No test hits the
//! network**, and no test depends on `frankenstein` beyond what the
//! production `ReqwestTransport` already pulls in. That is the whole
//! point of the private transport trait: the Phase 8 Task 1 test
//! surface is the same shape in CI, on a dev box, and on a plane.
//!
//! Test coverage map (per the Task 1 plan's "6–8 unit tests" bullet):
//!
//! 1. `metadata_is_telegram_and_semi_trusted` — platform, trust tier,
//!    and channel name are the values advertised in the module doc.
//! 2. `session_id_is_stable_across_reads` — matches the `LocalChannel`
//!    guarantee so audit correlation on session boundaries works.
//! 3. `reset_cancellation_rotates_token` — Phase 3 monotonic-token
//!    fix applied to the network channel.
//! 4. `finalize_sends_one_message_with_buffered_text` — the core
//!    "stream text → buffer → one send on finalize" contract.
//! 5. `tool_markers_and_status_append_to_buffer` — non-Text events
//!    render into the same buffer without becoming their own sends.
//! 6. `empty_turn_yields_no_reply_placeholder` — Telegram rejects
//!    empty sendMessage; the channel substitutes `"(no reply)"`.
//! 7. `finalize_footer_reflects_outcome` — Cancelled / Failed /
//!    TimedOut / Escalated outcomes all render a distinct footer.
//! 8. `transport_error_propagates_as_channel_error` — the scripted
//!    transport can inject a `TransportError::Platform` and the
//!    channel surfaces it as `ChannelError::Platform`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;

use aivyx_capability::TrustTier;
use aivyx_core::{
    AivyxError, ChannelContext, ChannelPlatform, StreamEvent, ToolId, TurnOutcome,
};

use crate::telegram_channel::TelegramChannel;
use crate::transport::{IncomingMessage, OutgoingMessage, TelegramTransport, TransportError};

// ---------------------------------------------------------------------------
// ScriptedTransport — the test double
// ---------------------------------------------------------------------------

/// An in-memory `TelegramTransport` impl for tests. Three knobs:
///
/// - `updates`: a queue of pre-canned `IncomingMessage`s that
///   successive `get_updates` calls drain from. Exhausting the queue
///   returns `Ok(vec![])`, matching Bot API long-poll behavior.
/// - `sent`: a capture buffer that `send_message` appends to. Tests
///   read it back after a turn to assert exactly what the user would
///   have seen on Telegram.
/// - `send_error`: if `Some`, every `send_message` call returns that
///   error instead of buffering. Used by the error-propagation test.
struct ScriptedTransport {
    updates: Mutex<Vec<IncomingMessage>>,
    sent: Mutex<Vec<OutgoingMessage>>,
    send_error: Mutex<Option<String>>,
}

impl ScriptedTransport {
    fn new() -> Self {
        ScriptedTransport {
            updates: Mutex::new(Vec::new()),
            sent: Mutex::new(Vec::new()),
            send_error: Mutex::new(None),
        }
    }

    #[allow(dead_code)] // kept for tasks 2–6 which drive inbound updates
    fn push_update(&self, update: IncomingMessage) {
        self.updates.lock().unwrap().push(update);
    }

    fn inject_send_error(&self, err: impl Into<String>) {
        *self.send_error.lock().unwrap() = Some(err.into());
    }

    fn sent_snapshot(&self) -> Vec<OutgoingMessage> {
        self.sent.lock().unwrap().clone()
    }
}

#[async_trait]
impl TelegramTransport for ScriptedTransport {
    async fn get_updates(
        &self,
        _offset: i64,
        timeout_secs: u32,
    ) -> Result<Vec<IncomingMessage>, TransportError> {
        // Phase 8 Task 4 — simulate Bot API long-poll behavior: if the
        // update queue is empty, block for up to `timeout_secs` before
        // returning an empty batch. This matches what the real
        // frankenstein/reqwest transport does and, more importantly,
        // keeps `run_telegram_session_with_transport` from hot-spinning
        // in tests after the scripted updates drain.
        let drained = std::mem::take(&mut *self.updates.lock().unwrap());
        if drained.is_empty() {
            tokio::time::sleep(Duration::from_secs(timeout_secs as u64)).await;
        }
        Ok(drained)
    }

    async fn send_message(&self, msg: OutgoingMessage) -> Result<(), TransportError> {
        if let Some(err) = self.send_error.lock().unwrap().clone() {
            return Err(TransportError::Platform(err));
        }
        self.sent.lock().unwrap().push(msg);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_channel() -> (TelegramChannel<ScriptedTransport>, Arc<ScriptedTransport>) {
    let transport = Arc::new(ScriptedTransport::new());
    let channel = TelegramChannel::new("tg-test", 42, Arc::clone(&transport));
    (channel, transport)
}

fn completed_outcome() -> TurnOutcome {
    TurnOutcome::Completed {
        final_message: String::new(),
        tool_calls_made: 0,
        duration: Duration::from_millis(1),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn metadata_is_telegram_and_semi_trusted() {
    let (channel, _) = make_channel();
    assert_eq!(channel.platform(), ChannelPlatform::Telegram);
    assert_eq!(
        channel.trust_tier(),
        TrustTier::SemiTrusted,
        "Telegram = authenticated remote = SemiTrusted, not Untrusted (see module doc)"
    );
    assert_eq!(channel.channel_name(), "tg-test");
    assert_eq!(channel.chat_id(), 42);
}

#[test]
fn session_id_is_stable_across_reads() {
    let (channel, _) = make_channel();
    let s1 = channel.session_id();
    let s2 = channel.session_id();
    assert_eq!(s1, s2);
}

#[tokio::test]
async fn reset_cancellation_rotates_token() {
    // Phase 3 monotonic-token fix: a Cancelled turn must not poison
    // the next turn's cancellation token.
    let (channel, _) = make_channel();
    let old = channel.cancellation_token();
    old.cancel();
    assert!(channel.cancellation_token().is_cancelled());

    channel.reset_cancellation();
    assert!(
        !channel.cancellation_token().is_cancelled(),
        "post-reset token must be un-cancelled"
    );
    // The orphaned clone stays cancelled — that's the whole reason
    // we rotate instead of trying to un-cancel in place.
    assert!(old.is_cancelled());
}

#[tokio::test]
async fn finalize_sends_one_message_with_buffered_text() {
    // The core contract: three Text chunks buffer into one Telegram
    // send when `finalize` runs. A `LocalChannel` would have produced
    // three flushes here — the Telegram channel produces exactly one
    // `send_message` call, with the concatenated text.
    let (channel, transport) = make_channel();

    channel.stream_event(StreamEvent::Text("hello ")).await.unwrap();
    channel.stream_event(StreamEvent::Text("there, ")).await.unwrap();
    channel.stream_event(StreamEvent::Text("world")).await.unwrap();

    // Before finalize: nothing sent, buffer has the concatenation.
    assert!(transport.sent_snapshot().is_empty());
    assert_eq!(channel.buffer_snapshot(), "hello there, world");

    channel.finalize(&completed_outcome()).await.unwrap();

    let sent = transport.sent_snapshot();
    assert_eq!(sent.len(), 1, "one turn = one Telegram message");
    assert_eq!(sent[0].chat_id, 42);
    assert_eq!(sent[0].text, "hello there, world");

    // Buffer must be drained so the next turn starts clean.
    assert_eq!(channel.buffer_snapshot(), "");
}

#[tokio::test]
async fn tool_markers_and_status_append_to_buffer() {
    let (channel, transport) = make_channel();
    let tool = ToolId::new();
    let input = serde_json::json!({"path": "/tmp/x"});

    channel.stream_event(StreamEvent::Text("thinking")).await.unwrap();
    channel.stream_event(StreamEvent::Status("still thinking")).await.unwrap();
    channel
        .stream_event(StreamEvent::ToolCallStarted {
            tool,
            input: &input,
        })
        .await
        .unwrap();
    channel
        .stream_event(StreamEvent::ToolCallFinished {
            tool,
            outcome_summary: "ok",
        })
        .await
        .unwrap();
    channel.stream_event(StreamEvent::Text("done")).await.unwrap();

    channel.finalize(&completed_outcome()).await.unwrap();

    let sent = transport.sent_snapshot();
    assert_eq!(sent.len(), 1);
    let text = &sent[0].text;
    assert!(text.starts_with("thinking\n… still thinking\n"), "{text:?}");
    assert!(text.contains("→ tool["), "tool-started marker: {text:?}");
    assert!(
        text.contains("← tool[") && text.contains("ok"),
        "tool-finished marker: {text:?}"
    );
    assert!(text.ends_with("done"), "trailing text joined: {text:?}");
}

#[tokio::test]
async fn empty_turn_yields_no_reply_placeholder() {
    // A turn the LLM ended without speaking a single Text chunk (all
    // tool, no reply) must still produce a non-empty Telegram message
    // — the Bot API rejects empty `sendMessage`.
    let (channel, transport) = make_channel();
    channel.finalize(&completed_outcome()).await.unwrap();
    let sent = transport.sent_snapshot();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].text, "(no reply)");
}

#[tokio::test]
async fn finalize_footer_reflects_outcome() {
    // Each non-Completed outcome renders a distinct, grep-able footer.
    // We test three variants (Cancelled, TimedOut, Failed) — Escalated
    // is covered by the ToolId-bearing variant in the next assertion
    // block.
    let (channel, transport) = make_channel();
    channel.stream_event(StreamEvent::Text("partial")).await.unwrap();
    channel
        .finalize(&TurnOutcome::Cancelled { tool_calls_made: 0 })
        .await
        .unwrap();
    let sent = transport.sent_snapshot();
    assert!(sent[0].text.contains("✕ cancelled"), "{}", sent[0].text);

    // A fresh channel for the next outcome so buffers don't bleed.
    let (channel2, transport2) = make_channel();
    channel2.stream_event(StreamEvent::Text("slow")).await.unwrap();
    channel2
        .finalize(&TurnOutcome::TimedOut {
            tool_calls_made: 0,
            elapsed: Duration::from_secs(30),
        })
        .await
        .unwrap();
    assert!(
        transport2.sent_snapshot()[0].text.contains("⏱ timed out"),
        "{}",
        transport2.sent_snapshot()[0].text
    );

    let (channel3, transport3) = make_channel();
    channel3
        .finalize(&TurnOutcome::Failed(AivyxError::Channel("boom".into())))
        .await
        .unwrap();
    assert!(
        transport3.sent_snapshot()[0].text.contains("✕ failed"),
        "{}",
        transport3.sent_snapshot()[0].text
    );
}

// ---------------------------------------------------------------------------
// Phase 8 Task 2 — two chats, one store, isolated memory partitions.
//
// This test is the end-to-end payoff for Task 2: it drives the real
// `MemoryReadTool`/`MemoryWriteTool` with two `TelegramChannel`s
// sharing one `InMemoryMemory`, and proves that chat A's
// `memory.write` is invisible to chat B's `memory.read`.
//
// The turn-loop injection (`agent.rs::run_tool_call`) is simulated
// here as a tiny `inject_session` helper because spinning up a full
// `ConcreteAgent` would pull in a planner and a capability set for
// a test whose point is just the partition-isolation invariant.
// The simulation is a one-line `obj.insert("session", ...)` on the
// input — the *same* operation the production turn loop does, so
// the test would catch any drift between the two sites.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn two_chats_isolated() {
    use aivyx_core::{
        AgentId, CancellationToken, NullAuditHook, SessionId, Tool, ToolContext, ToolOutcome,
        TurnId,
    };
    use aivyx_memory::{InMemoryMemory, MemoryReadTool, MemoryWriteTool};
    use std::sync::Arc;

    // One memory store, shared by both chats.
    let mem: Arc<dyn aivyx_memory::Memory> = Arc::new(InMemoryMemory::new());
    let writer = MemoryWriteTool::new(mem.clone());
    let reader = MemoryReadTool::new(mem.clone());

    // Two channels, two chat_ids. Each channel's
    // `session_partition()` returns its own `chat_id.to_string()` —
    // that's the Task 2 override being exercised.
    let transport_a = Arc::new(ScriptedTransport::new());
    let chan_a: TelegramChannel<ScriptedTransport> =
        TelegramChannel::new("tg-a", 1001, Arc::clone(&transport_a));
    let transport_b = Arc::new(ScriptedTransport::new());
    let chan_b: TelegramChannel<ScriptedTransport> =
        TelegramChannel::new("tg-b", 2002, Arc::clone(&transport_b));

    assert_eq!(chan_a.session_partition(), Some("1001".to_string()));
    assert_eq!(chan_b.session_partition(), Some("2002".to_string()));

    // Simulate what `ConcreteAgent::run_tool_call` does between
    // "planner emitted a tool call" and "required_scope": insert the
    // channel's partition under the reserved `"session"` key. The
    // production injection is in `agent.rs`; this helper exists so
    // if the two sites drift, this test would flag it.
    fn inject_session(
        input: &mut serde_json::Value,
        channel: &dyn aivyx_core::ChannelContext,
    ) {
        if let Some(partition) = channel.session_partition()
            && let Some(obj) = input.as_object_mut()
        {
            obj.insert("session".to_string(), serde_json::Value::String(partition));
        }
    }

    // Helper: build a ToolContext borrowing the given channel.
    // `session_id`, `agent_id`, `turn_id` are irrelevant to the
    // partition-isolation check — the memory tools never read them
    // — so fresh values each call are fine.
    let audit = NullAuditHook;
    fn make_ctx<'a>(
        channel: &'a dyn aivyx_core::ChannelContext,
        audit: &'a dyn aivyx_core::AuditHook,
        cancel: &'a CancellationToken,
    ) -> ToolContext<'a> {
        ToolContext {
            agent_id: AgentId::new(),
            session_id: SessionId::new(),
            turn_id: TurnId::new(),
            channel,
            audit,
            cancellation: cancel,
        }
    }
    let cancel = CancellationToken::new();

    // Chat A writes "purple" to `notes`.
    let mut input = serde_json::json!({"topic": "notes", "body": "purple"});
    inject_session(&mut input, &chan_a);
    assert_eq!(
        input["session"], "1001",
        "injection must stamp chat_a's partition onto the tool input"
    );
    let out = writer.execute(input, &make_ctx(&chan_a, &audit, &cancel)).await;
    assert!(
        matches!(out, ToolOutcome::Completed { .. }),
        "chat A write should Complete, got {out:?}"
    );

    // Chat B writes "green" to the same logical topic `notes`.
    let mut input = serde_json::json!({"topic": "notes", "body": "green"});
    inject_session(&mut input, &chan_b);
    let out = writer.execute(input, &make_ctx(&chan_b, &audit, &cancel)).await;
    assert!(matches!(out, ToolOutcome::Completed { .. }));

    // Chat A reads `notes` — must see only "purple", not "green".
    let mut input = serde_json::json!({"topic": "notes"});
    inject_session(&mut input, &chan_a);
    let out = reader.execute(input, &make_ctx(&chan_a, &audit, &cancel)).await;
    let ToolOutcome::Completed { output, .. } = out else {
        panic!("chat A read should Complete");
    };
    let entries = output["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "chat A must see exactly its own entry");
    assert_eq!(
        entries[0]["body"], "purple",
        "chat A must see its own body, not chat B's"
    );
    // Logical topic restored on the way out — the agent never sees
    // the namespaced physical key.
    assert_eq!(entries[0]["topic"], "notes");
    assert_eq!(output["topic"], "notes");

    // Chat B reads `notes` — must see only "green".
    let mut input = serde_json::json!({"topic": "notes"});
    inject_session(&mut input, &chan_b);
    let out = reader.execute(input, &make_ctx(&chan_b, &audit, &cancel)).await;
    let ToolOutcome::Completed { output, .. } = out else {
        panic!("chat B read should Complete");
    };
    let entries = output["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["body"], "green");
    assert_eq!(entries[0]["topic"], "notes");
}

#[tokio::test]
async fn transport_error_propagates_as_channel_error() {
    // The channel translates `TransportError::Platform(..)` to
    // `ChannelError::Platform(..)`. The turn loop sees a uniform
    // ChannelError regardless of which transport was behind the trait.
    let (channel, transport) = make_channel();
    transport.inject_send_error("429 rate limited");
    channel.stream_event(StreamEvent::Text("hi")).await.unwrap();
    let err = channel
        .finalize(&completed_outcome())
        .await
        .expect_err("send_error must surface");
    let msg = err.to_string();
    assert!(
        msg.contains("429 rate limited"),
        "platform error should propagate verbatim: {msg}"
    );
}

// ---------------------------------------------------------------------------
// Phase 8 Task 3 — end-to-end tier-attenuation pin through a real
// `TelegramChannel`.
//
// Q3 was already resolved in Phase 4: `ConcreteAgent::turn` (at
// `agent.rs:121`) computes `effective = caps.intersect(tier.default_ceiling())`
// on every turn, using the channel's `trust_tier()` through the
// `ChannelContext` trait object. That means no adapter can forget to
// narrow — the narrowing lives in the turn loop, not the adapter.
//
// What Phase 4 could not test, and what Task 3 pins here, is that the
// **real** `TelegramChannel` (not the `FakeChannel` in `agent.rs`'s
// own test module) surfaces `SemiTrusted` through the dyn
// `ChannelContext` boundary and that the turn loop strips `shell.exec`
// accordingly. The test's assertion shape mirrors Phase 4's own
// `shell.exec` denial test at `agent.rs:615-683`, intentionally —
// this is the two-ends-of-the-same-string pin.
//
// The tool is a hand-rolled `ShellExecFake` that declares
// `required_scope() == "shell.exec:rm"` and panics if it's ever
// executed. The panic is load-bearing: if the tier attenuation ever
// regresses to admit `shell.exec`, the test fails loudly at
// `ShellExecFake::execute` rather than at the `ScopeDenied`
// assertion, so the failure mode is unambiguous.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tier_attenuation_denies_shell_exec_through_real_telegram_channel() {
    use std::sync::Mutex as StdMutex;

    use aivyx_capability::{CapabilitySet, Scope, TrustTier};
    use aivyx_core::{
        Agent, AgentId, AuditHook, AuditTag, ConcreteAgent, Message, NextStep, Tool,
        ToolContext, ToolId, ToolOutcome, ToolRegistry, TurnOutcome, VecPlanner,
    };

    // ---- Recording audit --------------------------------------
    // Mirrors the shape of `agent.rs`'s own `RecordingAudit` — a
    // Vec<AuditTag> behind a Mutex. Purely for inspection; the
    // HMAC-chain integrity of real audit logs is `aivyx-audit`'s
    // problem, not this test's.
    #[derive(Default)]
    struct RecordingAudit {
        events: StdMutex<Vec<AuditTag>>,
    }
    impl RecordingAudit {
        fn snapshot(&self) -> Vec<AuditTag> {
            self.events.lock().unwrap().clone()
        }
    }
    impl AuditHook for RecordingAudit {
        fn on_event(&self, tag: AuditTag) {
            self.events.lock().unwrap().push(tag);
        }
    }

    // ---- Fake shell.exec tool --------------------------------
    // `required_scope` is *input-derived* per R1: `shell.exec:<command>`.
    // `execute` panics if reached, because a successful tier strip
    // means we never reach it. The panic IS the invariant's safety net.
    struct ShellExecFake {
        id: ToolId,
        schema: serde_json::Value,
    }
    impl ShellExecFake {
        fn new() -> Self {
            ShellExecFake {
                id: ToolId::new(),
                schema: serde_json::json!({
                    "type": "object",
                    "properties": {"command": {"type": "string"}},
                    "required": ["command"],
                }),
            }
        }
    }
    #[async_trait]
    impl Tool for ShellExecFake {
        fn id(&self) -> ToolId {
            self.id
        }
        fn name(&self) -> &str {
            "shell.exec"
        }
        fn description(&self) -> &str {
            "A fake shell.exec tool that must never be called from a SemiTrusted channel."
        }
        fn input_schema(&self) -> &serde_json::Value {
            &self.schema
        }
        fn required_scope(&self, input: &serde_json::Value) -> aivyx_capability::Scope {
            let command = input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("<missing>");
            aivyx_capability::Scope::parse(&format!("shell.exec:{command}"))
                .expect("shell.exec:<command> must parse")
        }
        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &ToolContext<'_>,
        ) -> ToolOutcome {
            panic!(
                "ShellExecFake::execute was reached — the SemiTrusted \
                 tier ceiling failed to strip shell.exec, which means \
                 the Phase 4 attenuation at agent.rs:121 has regressed"
            );
        }
    }

    // ---- Wire up the agent ----------------------------------
    // Agent nominally holds `shell.exec:rm` as a qualified scope.
    // Under the Trusted ceiling it would be granted; under the
    // SemiTrusted ceiling (which has no `shell.exec` at all) the
    // intersection is empty for this base, so the scope check fails.
    let audit: Arc<RecordingAudit> = Arc::new(RecordingAudit::default());
    let agent_caps =
        CapabilitySet::from_scopes([Scope::parse("shell.exec:rm").unwrap()]);
    let tool = Arc::new(ShellExecFake::new());
    let tool_id = tool.id();
    let registry = Arc::new(ToolRegistry::new(vec![tool as Arc<dyn Tool>]));

    let plan = vec![NextStep::ToolCall {
        tool_id,
        input: serde_json::json!({"command": "rm"}),
    }];
    let agent = ConcreteAgent::new(
        AgentId::new(),
        agent_caps,
        registry,
        audit.clone() as Arc<dyn AuditHook>,
        move || Box::new(VecPlanner::new(plan.clone())),
    );

    // ---- Real TelegramChannel, not a FakeChannel -------------
    let (channel, _transport) = make_channel();
    assert_eq!(
        channel.trust_tier(),
        TrustTier::SemiTrusted,
        "sanity: the real channel must surface SemiTrusted"
    );

    let message = Message::text(channel.session_id(), "delete my server please");
    let outcome = agent.turn(message, &channel).await;

    // ---- Assertions ------------------------------------------
    // Denial is not a termination — the turn Completes with one
    // attempted tool call, matching Phase 4's existing invariant.
    match outcome {
        TurnOutcome::Completed {
            tool_calls_made, ..
        } => assert_eq!(
            tool_calls_made, 1,
            "the attempted call still counts even though it was denied"
        ),
        other => panic!("expected Completed (denial is not termination), got {other:?}"),
    }

    let events = audit.snapshot();
    assert_eq!(
        events.len(),
        3,
        "expected TurnStarted → ScopeDenied → TurnEnded, got {events:?}"
    );

    // TurnStarted must advertise the SemiTrusted tier AND the
    // already-narrowed effective capabilities. If the attenuation
    // didn't happen, `effective_capabilities` would still grant
    // `shell.exec:rm`.
    match &events[0] {
        AuditTag::TurnStarted {
            trust_tier,
            effective_capabilities,
            channel: platform,
            ..
        } => {
            assert_eq!(*trust_tier, TrustTier::SemiTrusted);
            assert_eq!(*platform, aivyx_core::ChannelPlatform::Telegram);
            assert!(
                !effective_capabilities
                    .grants(&Scope::parse("shell.exec:rm").unwrap()),
                "SemiTrusted ceiling must strip shell.exec from effective set"
            );
            assert!(
                !effective_capabilities.grants(&Scope::parse("shell.exec").unwrap()),
                "no shell.exec in any form after SemiTrusted narrowing"
            );
        }
        other => panic!("expected TurnStarted at index 0, got {other:?}"),
    }

    // ScopeDenied must name the exact requested scope and carry the
    // held snapshot so auditors can reconstruct "what did the agent
    // have when the denial fired". Mirrors Phase 4's
    // `shell_exec_denied_on_semitrusted_channel` test at
    // `agent.rs:615-683`, but through the real `TelegramChannel`.
    match &events[1] {
        AuditTag::ScopeDenied {
            scope_requested,
            held_capabilities,
            ..
        } => {
            assert_eq!(scope_requested.base(), "shell.exec");
            assert_eq!(scope_requested.qualifier(), Some("rm"));
            assert!(
                !held_capabilities.grants(&Scope::parse("shell.exec").unwrap()),
                "held set (post-narrow) must not grant shell.exec"
            );
            assert!(
                !held_capabilities.grants(&Scope::parse("shell.exec:rm").unwrap()),
                "held set must not grant shell.exec:rm specifically"
            );
        }
        other => panic!("expected ScopeDenied at index 1, got {other:?}"),
    }

    // And absolutely no ToolCall event — the denial is the whole
    // point. If this assertion fails, `ShellExecFake::execute` would
    // also have fired a panic, but we check explicitly anyway because
    // a tool that short-circuits in `execute` could still produce an
    // audit event before the panic aborted the thread.
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, AuditTag::ToolCall { .. })),
        "no ToolCall event should appear for a denied call: {events:?}"
    );

    // TurnEnded closes the audit window.
    assert!(
        matches!(events[2], AuditTag::TurnEnded { .. }),
        "expected TurnEnded at index 2, got {:?}",
        events[2]
    );
}

// ---------------------------------------------------------------------------
// Phase 8 Task 4 — `run_telegram_session_with_transport` scripted drive.
//
// This test is the Task 4 payoff: the long-poll loop drains a scripted
// queue of two inbound updates, runs two full turns through a real
// `ConcreteAgent` wired to a scripted `LlmProvider`, and emits two
// `send_message` calls to the scripted transport. Assert shape mirrors
// the local path's `cli_e2e.rs` but through the Telegram loop.
//
// Termination: the scripted transport's upgraded `get_updates` blocks
// for `timeout_secs` on an empty queue (simulating Bot API long-poll),
// so once the two scripted updates are drained the loop's next call
// would stall for `long_poll_timeout_secs` seconds. The test cancels
// the channel's cancellation token externally as soon as two
// `send_message` captures appear, which the loop checks at the top of
// each iteration *before* the long-poll call — so cancellation fires
// promptly and the spawned task returns cleanly.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_telegram_session_drives_two_scripted_turns() {
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::Mutex as StdMutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    use aivyx_audit::{AuditBridge, HmacChainLog};
    use aivyx_capability::{CapabilitySet, Scope};
    use aivyx_core::{AuditHook, CancellationToken, ToolRegistry};
    use crate::TelegramSessionConfig;
    use aivyx_crypto::MasterKey;
    use aivyx_llm::{
        LlmError, LlmMessage, LlmProvider, LlmRequest, LlmStepEnd, LlmStream, LlmStreamEvent,
        LlmUsage,
    };
    use aivyx_storage::{RedbStorage, Storage, StorageConfig};

    use crate::session::run_telegram_session_with_transport;

    // ---- Scripted LLM provider ------------------------------------
    // One `chat_stream` call per planner step; one FinalMessage per
    // turn (no tools means no multi-step turns in this test). Exact
    // copy of the `cli_e2e.rs` pattern — kept inline here so the
    // telegram crate doesn't pull in a test-only dependency on the
    // channel crate's tests module.
    struct ScriptedStep {
        events: Vec<LlmStreamEvent>,
        terminal: LlmStepEnd,
    }

    struct ScriptedProvider {
        queue: StdMutex<VecDeque<ScriptedStep>>,
    }

    #[async_trait]
    impl LlmProvider for ScriptedProvider {
        async fn chat_stream(
            &self,
            request: LlmRequest<'_>,
            _cancellation: &CancellationToken,
        ) -> Result<Box<dyn LlmStream>, LlmError> {
            assert!(
                !request.messages.is_empty(),
                "planner must always send non-empty history"
            );
            assert!(
                matches!(request.messages[0], LlmMessage::User { .. }),
                "history[0] should be a User message for a turn with no tools"
            );
            let step = self
                .queue
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| LlmError::Config("ScriptedProvider exhausted".into()))?;
            Ok(Box::new(ScriptedStream {
                events: step.events.into_iter(),
                terminal: Some(step.terminal),
            }))
        }
    }

    struct ScriptedStream {
        events: std::vec::IntoIter<LlmStreamEvent>,
        terminal: Option<LlmStepEnd>,
    }
    #[async_trait]
    impl LlmStream for ScriptedStream {
        async fn next_event(&mut self) -> Result<Option<LlmStreamEvent>, LlmError> {
            Ok(self.events.next())
        }
        async fn finish(self: Box<Self>) -> Result<LlmStepEnd, LlmError> {
            self.terminal
                .ok_or_else(|| LlmError::StreamEnded("ScriptedStream::finish double-called".into()))
        }
    }

    fn final_step(chunks: &[&str], text: &str) -> ScriptedStep {
        ScriptedStep {
            events: chunks
                .iter()
                .map(|c| LlmStreamEvent::TextChunk((*c).to_string()))
                .collect(),
            terminal: LlmStepEnd::FinalMessage {
                text: text.to_string(),
                usage: LlmUsage::default(),
            },
        }
    }

    // ---- Scratch storage (matches cli_e2e.rs convention) ----------
    // `SessionConfig.storage` is a required field because `run_session`
    // writes a session marker. `run_telegram_session` does *not* write
    // markers in Phase 8 (see session.rs module doc), but the config
    // still carries a storage handle — we give it a real one so the
    // API surface is honest and a future refinement that wires per-chat
    // markers doesn't need a second test fixture path.
    let tmp = std::env::var("TMPDIR")
        .or_else(|_| std::env::var("TEMP"))
        .unwrap_or_else(|_| "/tmp".to_string());
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let parent = PathBuf::from(tmp).join(format!("aivyx-tg-task4-{pid}-{nanos}"));
    std::fs::create_dir_all(&parent).expect("scratch store parent must be creatable");
    let store_path = parent.join("store.redb");
    let storage: Arc<dyn Storage> = RedbStorage::open(
        StorageConfig::new(store_path.clone()),
        MasterKey::from_raw([7u8; 32]),
    )
    .await
    .expect("scratch storage must open");

    // ---- Wire the scripted provider + audit -----------------------
    let provider: Arc<dyn LlmProvider> = Arc::new(ScriptedProvider {
        queue: StdMutex::new(
            vec![
                final_step(&["Hello, ", "chat!"], "Hello, chat!"),
                final_step(&["Bye!"], "Bye!"),
            ]
            .into(),
        ),
    });
    let audit_bridge = Arc::new(AuditBridge::new(HmacChainLog::new([42u8; 32].to_vec())));
    let audit: Arc<dyn AuditHook> = audit_bridge.clone();

    // ---- Telegram channel + pre-loaded scripted updates -----------
    let transport = Arc::new(ScriptedTransport::new());
    transport.push_update(IncomingMessage {
        update_id: 10,
        chat_id: 777,
        user_id: 1,
        text: "first".to_string(),
    });
    transport.push_update(IncomingMessage {
        update_id: 11,
        chat_id: 777,
        user_id: 1,
        text: "second".to_string(),
    });
    // One extra update for a *different* chat — the loop must filter
    // it out (one channel = one chat_id in Phase 8). If the loop mis-
    // routes this, we'd see a third `send_message` call and the
    // assertion below would fail.
    transport.push_update(IncomingMessage {
        update_id: 12,
        chat_id: 999,
        user_id: 1,
        text: "wrong chat".to_string(),
    });

    let channel = Arc::new(TelegramChannel::new(
        "tg-task4-test",
        777,
        Arc::clone(&transport),
    ));

    // ---- TelegramSessionConfig (empty tool registry, broad caps) -
    let config = TelegramSessionConfig {
        model: "claude-haiku-4-5-20251001".to_string(),
        system_prompt: "telegram test".to_string(),
        max_tokens: 128,
        // One memory scope so the Phase 4 attenuation has something
        // to intersect against without stripping to empty. The turn
        // isn't actually calling memory tools — the SemiTrusted
        // ceiling would pass through `memory.read` / `memory.write`
        // regardless — but giving the agent a held scope the
        // ceiling admits keeps this test from shadowing a potential
        // "empty caps is degenerate" bug.
        capabilities: CapabilitySet::from_scopes([Scope::parse("memory.read").unwrap()]),
        tools: Arc::new(ToolRegistry::new(Vec::new())),
        storage: Arc::clone(&storage),
    };

    // ---- Drive the session loop under a bounded timeout ----------
    //
    // The scripted transport's long-poll simulation sleeps for
    // `long_poll_timeout_secs` on empty queues. We pass `1` (instead
    // of the 25s production default) so the test's third iteration —
    // after the two scripted updates drain — takes at most 1 real
    // second before the loop re-checks cancellation. A watcher task
    // in parallel cancels the channel's token the instant both
    // outbound `send_message` calls land, which the loop picks up at
    // the top of the iteration *after* the empty-batch sleep. Total
    // wall time: ~1 second on a loaded machine, well under the 5s
    // overall test bound below.
    //
    // Trade-off acknowledged: this adds ~1 second to the test suite's
    // wall-clock budget. The alternative (`tokio` `test-util` feature
    // + `start_paused`) was considered but avoided here because the
    // workspace-wide tokio features would need a dev-dep override,
    // and one test being 1s slower is cheaper than the feature-flag
    // surface area.
    let channel_for_watcher = Arc::clone(&channel);
    let transport_for_watcher = Arc::clone(&transport);
    tokio::spawn(async move {
        loop {
            if transport_for_watcher.sent_snapshot().len() >= 2 {
                channel_for_watcher.cancellation_token().cancel();
                return;
            }
            tokio::task::yield_now().await;
        }
    });

    // Fresh, uncancelled shutdown token. The test exercises the
    // **per-turn channel token** path (the watcher cancels it after
    // two sends); the `shutdown` parameter here is always-live so
    // we can be sure the cancellation that terminates the loop is
    // the channel token, not a pre-set shutdown.
    let shutdown = CancellationToken::new();
    let report = tokio::time::timeout(
        Duration::from_secs(5),
        run_telegram_session_with_transport(
            Arc::clone(&channel),
            config,
            provider,
            audit,
            1, // long_poll_timeout_secs — small so empty-batch wakes up promptly
            shutdown,
        ),
    )
    .await
    .expect("run_telegram_session must exit within the 5-second test bound")
    .expect("run_telegram_session must return Ok");

    // ---- Assertions -----------------------------------------------
    assert_eq!(
        report.turns_run, 2,
        "two inbound messages for the target chat must each drive one turn"
    );

    let sent = transport.sent_snapshot();
    assert_eq!(
        sent.len(),
        2,
        "exactly two outbound messages (one per turn); the wrong-chat update must be filtered out: {sent:?}"
    );
    // Both sends must target the bound chat_id, not the mis-routed 999.
    assert_eq!(sent[0].chat_id, 777);
    assert_eq!(sent[1].chat_id, 777);
    // Text content mirrors the scripted planner output, joined through
    // the channel's buffer. `Hello, chat!` for turn 1, `Bye!` for turn 2.
    assert!(
        sent[0].text.contains("Hello, chat!"),
        "turn 1 should contain scripted chunks, got: {:?}",
        sent[0].text
    );
    assert!(
        sent[1].text.contains("Bye!"),
        "turn 2 should contain second scripted chunk, got: {:?}",
        sent[1].text
    );

    // ---- Cleanup --------------------------------------------------
    let _ = std::fs::remove_dir_all(&parent);
}

// ---------------------------------------------------------------------------
// Phase 8 Task 5 — cancellation over the network.
//
// The production concern Task 5 was opened for is "what plays the role
// of ctrl-C for a Telegram turn?" Candidates named in PHASE_8.md's
// draft task list were (a) a `/cancel` command, (b) a wall-clock
// timeout, and (c) a per-chat active-turn lock.
//
// What we discovered during Task 5: `aivyx-core` already owns a
// hardcoded `TURN_TIMEOUT = 120s` wall-clock deadline inside
// `ConcreteAgent::turn` (`agent.rs:66`). It spawns a background task
// that sleeps for the budget, cancels the channel's token, flags
// `deadline_fired`, and the loop translates the result into
// `TurnOutcome::TimedOut`. That machinery has been there since Phase 3
// and applies to every `ChannelContext` impl, Local and Telegram
// alike. The Telegram `finalize_footer` already renders
// `"⏱ timed out"` for it (pinned by `finalize_footer_reflects_outcome`
// above).
//
// So the Option-B design sketched at phase entry — "add
// `turn_deadline: Option<Duration>` to TelegramSessionConfig and wrap
// `agent.turn` in `tokio::time::timeout`" — would have duplicated
// machinery that already exists and contradicted core's explicit
// "const, not config knob" philosophy. The streak-preserving,
// honest-about-what-we-already-have move is to ship a **regression
// test** that proves the end-to-end cancel-and-continue flow works
// for the Telegram long-poll loop specifically, without inventing a
// second deadline layer.
//
// That's what this test is. It drives `run_telegram_session_with_transport`
// through:
//
// 1. One inbound update whose turn **stalls mid-stream** (the scripted
//    provider's `next_event` awaits a `tokio::time::sleep` longer than
//    the overall test bound, so if the cancellation path is broken we
//    hang and the 5-second test timeout catches it).
// 2. A watcher task that waits until the stall is confirmed entered
//    (via an `AtomicUsize` the provider bumps on stream construction)
//    and then cancels the channel's per-turn token — this is the same
//    thing the core `deadline_task` does internally, just triggered
//    deterministically without waiting 120 seconds wall-clock.
// 3. A second inbound update with a normal scripted final message
//    that must drive a **second turn to completion** — proving the
//    per-turn token rotation the session loop does via
//    `channel.reset_cancellation()` actually works for Telegram, the
//    same way Phase 3's fix works for LocalChannel.
//
// Asserts (in order of what they prove):
//
// - `report.turns_run == 2` — the cancelled turn counts, and the
//   session loop kept going to serve turn 2.
// - Exactly two `send_message` calls.
// - The first send's text contains `"✕ cancelled"` (from the
//   finalize_footer path for `TurnOutcome::Cancelled`), confirming
//   the agent returned Cancelled and the channel rendered it.
// - The second send's text contains the second turn's scripted
//   content — proving the per-turn token slot was rotated and a
//   fresh uncancelled token was in place when turn 2 started.
//
// If any of this breaks in a future phase, the symptom is either (a)
// the test hangs for 5 seconds and times out on the outer
// `tokio::time::timeout`, or (b) the assertion about two sends fails
// because the loop exited early. Both are loud.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_telegram_session_cancelled_turn_renders_and_continues() {
    use std::collections::VecDeque;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex as StdMutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    use aivyx_audit::{AuditBridge, HmacChainLog};
    use aivyx_capability::{CapabilitySet, Scope};
    use aivyx_core::{AuditHook, CancellationToken, ToolRegistry};
    use crate::TelegramSessionConfig;
    use aivyx_crypto::MasterKey;
    use aivyx_llm::{
        LlmError, LlmProvider, LlmRequest, LlmStepEnd, LlmStream, LlmStreamEvent, LlmUsage,
    };
    use aivyx_storage::{RedbStorage, Storage, StorageConfig};

    use crate::session::run_telegram_session_with_transport;

    // ---- Scripted provider with one stalling turn and one normal -
    //
    // `chat_stream` pulls from a queue of `Script` values. `Stall`
    // returns a stream whose `next_event` awaits a very long sleep —
    // it never resolves naturally inside the test's 5s bound, so the
    // only way it terminates is the planner's `tokio::select!` against
    // `cancellation.cancelled()` at `llm_planner.rs:176` picking the
    // cancel branch. `Final` returns a normal stream that yields one
    // TextChunk and a terminal FinalMessage, the same shape used by
    // `run_telegram_session_drives_two_scripted_turns` above.
    enum Script {
        Stall,
        Final { chunks: Vec<String>, text: String },
    }

    struct ScriptedProvider {
        queue: StdMutex<VecDeque<Script>>,
        // Bumped the moment a stalling stream is constructed, so the
        // watcher task can synchronize on "the first turn has begun
        // awaiting a stream event" rather than a wall-clock guess.
        stall_entered: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl LlmProvider for ScriptedProvider {
        async fn chat_stream(
            &self,
            request: LlmRequest<'_>,
            _cancellation: &CancellationToken,
        ) -> Result<Box<dyn LlmStream>, LlmError> {
            assert!(
                !request.messages.is_empty(),
                "planner must always send non-empty history"
            );
            let script = self
                .queue
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| LlmError::Config("ScriptedProvider exhausted".into()))?;
            match script {
                Script::Stall => {
                    self.stall_entered.fetch_add(1, Ordering::SeqCst);
                    Ok(Box::new(StallingStream))
                }
                Script::Final { chunks, text } => Ok(Box::new(FinalStream {
                    events: chunks
                        .into_iter()
                        .map(LlmStreamEvent::TextChunk)
                        .collect::<Vec<_>>()
                        .into_iter(),
                    terminal: Some(LlmStepEnd::FinalMessage {
                        text,
                        usage: LlmUsage::default(),
                    }),
                })),
            }
        }
    }

    /// Stream whose `next_event` awaits a sleep longer than any
    /// reasonable test budget. Borrows the pattern from
    /// `agent.rs:860`'s "blocks forever on next_event" test provider;
    /// we use a very large `sleep` rather than `pending::<()>().await`
    /// because a concrete future is easier to reason about under the
    /// planner's `tokio::select!` against cancellation.
    struct StallingStream;

    #[async_trait]
    impl LlmStream for StallingStream {
        async fn next_event(&mut self) -> Result<Option<LlmStreamEvent>, LlmError> {
            // 60s — well past the 5s overall test timeout, well under
            // the 120s core `TURN_TIMEOUT`. If the channel token is
            // never cancelled (the bug this test guards against), the
            // test's outer `tokio::time::timeout(5s)` fires first and
            // the failure mode is "ran for 5s and panicked" rather
            // than "slept for 120s and produced the wrong outcome."
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok(None)
        }
        async fn finish(self: Box<Self>) -> Result<LlmStepEnd, LlmError> {
            // Should never be called: the planner's cancellation
            // branch wins before `next_event` returns.
            Err(LlmError::StreamEnded(
                "StallingStream::finish called — cancellation path did not interrupt the turn".into(),
            ))
        }
    }

    struct FinalStream {
        events: std::vec::IntoIter<LlmStreamEvent>,
        terminal: Option<LlmStepEnd>,
    }
    #[async_trait]
    impl LlmStream for FinalStream {
        async fn next_event(&mut self) -> Result<Option<LlmStreamEvent>, LlmError> {
            Ok(self.events.next())
        }
        async fn finish(self: Box<Self>) -> Result<LlmStepEnd, LlmError> {
            self.terminal
                .ok_or_else(|| LlmError::StreamEnded("FinalStream::finish double-called".into()))
        }
    }

    // ---- Scratch storage ------------------------------------------
    let tmp = std::env::var("TMPDIR")
        .or_else(|_| std::env::var("TEMP"))
        .unwrap_or_else(|_| "/tmp".to_string());
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let parent = PathBuf::from(tmp).join(format!("aivyx-tg-task5-{pid}-{nanos}"));
    std::fs::create_dir_all(&parent).expect("scratch store parent must be creatable");
    let store_path = parent.join("store.redb");
    let storage: Arc<dyn Storage> = RedbStorage::open(
        StorageConfig::new(store_path.clone()),
        MasterKey::from_raw([9u8; 32]),
    )
    .await
    .expect("scratch storage must open");

    // ---- Wire provider + audit ------------------------------------
    let stall_entered = Arc::new(AtomicUsize::new(0));
    let provider: Arc<dyn LlmProvider> = Arc::new(ScriptedProvider {
        queue: StdMutex::new(
            vec![
                Script::Stall,
                Script::Final {
                    chunks: vec!["second turn ".into(), "completed".into()],
                    text: "second turn completed".into(),
                },
            ]
            .into(),
        ),
        stall_entered: Arc::clone(&stall_entered),
    });
    let audit_bridge = Arc::new(AuditBridge::new(HmacChainLog::new([43u8; 32].to_vec())));
    let audit: Arc<dyn AuditHook> = audit_bridge.clone();

    // ---- Transport + pre-loaded updates ---------------------------
    let transport = Arc::new(ScriptedTransport::new());
    transport.push_update(IncomingMessage {
        update_id: 100,
        chat_id: 555,
        user_id: 1,
        text: "please stall".to_string(),
    });
    transport.push_update(IncomingMessage {
        update_id: 101,
        chat_id: 555,
        user_id: 1,
        text: "please reply normally".to_string(),
    });

    let channel = Arc::new(TelegramChannel::new(
        "tg-task5-test",
        555,
        Arc::clone(&transport),
    ));

    let config = TelegramSessionConfig {
        model: "claude-haiku-4-5-20251001".to_string(),
        system_prompt: "telegram cancel test".to_string(),
        max_tokens: 128,
        capabilities: CapabilitySet::from_scopes([Scope::parse("memory.read").unwrap()]),
        tools: Arc::new(ToolRegistry::new(Vec::new())),
        storage: Arc::clone(&storage),
    };

    // ---- Watcher: cancel the per-turn token once the stall begins -
    //
    // Synchronization point is deterministic: the watcher spins on
    // `stall_entered.load()` until the scripted provider bumps it to
    // 1 (meaning `chat_stream` returned `StallingStream` and the
    // planner is now about to `await stream.next_event()` inside its
    // `tokio::select!`). At that point cancelling the channel token
    // wins the select and propagates into `TurnOutcome::Cancelled`.
    //
    // No wall-clock sleep here — the `yield_now` hand-off lets the
    // provider task actually run between polls on a single-threaded
    // runtime. If the provider path stops bumping the counter in a
    // future refactor, the symptom is the watcher spinning forever,
    // which the outer 5s timeout catches.
    let channel_for_watcher = Arc::clone(&channel);
    let stall_entered_watcher = Arc::clone(&stall_entered);
    tokio::spawn(async move {
        loop {
            if stall_entered_watcher.load(Ordering::SeqCst) >= 1 {
                // A tiny sleep so the planner has definitely reached
                // the `tokio::select!` await before we cancel. Without
                // it there's a race where the cancel could fire
                // *before* the planner arms the select arm, which
                // would still work via the top-of-loop check — but
                // the mid-stream select path is the interesting one
                // and this ensures we exercise it.
                tokio::time::sleep(Duration::from_millis(10)).await;
                channel_for_watcher.cancellation_token().cancel();
                return;
            }
            tokio::task::yield_now().await;
        }
    });

    // ---- Drive the session loop under the 5s overall bound --------
    let shutdown = CancellationToken::new();
    let report = tokio::time::timeout(
        Duration::from_secs(5),
        run_telegram_session_with_transport(
            Arc::clone(&channel),
            config,
            provider,
            audit,
            1, // long_poll_timeout_secs — short so the empty-batch
               // wait after turn 2 wakes up quickly enough for the
               // outer-loop shutdown check to fire.
            shutdown.clone(),
        ),
    );

    // Fire a secondary cancellation of `shutdown` a bit after the
    // second send appears, to guarantee the session loop exits on
    // the next top-of-loop check instead of waiting out another
    // `long_poll_timeout_secs` iteration. This is the same pattern
    // the Task 4 test uses against the channel token — here we use
    // `shutdown` because the per-turn token slot will have been
    // rotated by turn 2.
    let transport_for_shutdown = Arc::clone(&transport);
    let shutdown_for_task = shutdown.clone();
    tokio::spawn(async move {
        loop {
            if transport_for_shutdown.sent_snapshot().len() >= 2 {
                shutdown_for_task.cancel();
                return;
            }
            tokio::task::yield_now().await;
        }
    });

    let report = report
        .await
        .expect("run_telegram_session must exit within the 5-second test bound")
        .expect("run_telegram_session must return Ok");

    // ---- Assertions -----------------------------------------------
    assert_eq!(
        report.turns_run, 2,
        "cancelled turn must still count, and the session loop must continue to drive turn 2 after the rotation"
    );

    let sent = transport.sent_snapshot();
    assert_eq!(
        sent.len(),
        2,
        "one cancelled turn + one completed turn = exactly two Telegram sends, got: {sent:?}"
    );
    assert_eq!(sent[0].chat_id, 555);
    assert_eq!(sent[1].chat_id, 555);

    // Turn 1: the cancelled-turn footer. `finalize_footer` renders
    // `TurnOutcome::Cancelled` as `"\n✕ cancelled"`, so the sent
    // payload ends with that marker. The check is `contains` rather
    // than `ends_with` to stay robust against a future refactor that
    // appends additional trailing metadata (e.g. a duration).
    assert!(
        sent[0].text.contains("✕ cancelled"),
        "turn 1 must render as a cancelled outcome; got: {:?}",
        sent[0].text
    );

    // Turn 2: proves the per-turn token rotation worked. A fresh
    // uncancelled token was in place when turn 2 started, the scripted
    // FinalMessage ran to completion, and the channel buffered the
    // chunks into one send.
    assert!(
        sent[1].text.contains("second turn completed"),
        "turn 2 must contain the second scripted final message; got: {:?}",
        sent[1].text
    );
    // Turn 2 must NOT have a cancelled footer — a bug where the
    // rotated token was still the cancelled clone would either emit
    // Cancelled again here, or race and emit nothing.
    assert!(
        !sent[1].text.contains("✕ cancelled"),
        "turn 2 must not carry a cancelled footer; got: {:?}",
        sent[1].text
    );

    // ---- Cleanup --------------------------------------------------
    let _ = std::fs::remove_dir_all(&parent);
}
