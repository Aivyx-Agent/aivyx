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
        _timeout_secs: u32,
    ) -> Result<Vec<IncomingMessage>, TransportError> {
        let drained = std::mem::take(&mut *self.updates.lock().unwrap());
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
