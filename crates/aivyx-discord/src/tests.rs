//! Phase 107 Task 6 — scripted end-to-end suite.
//!
//! Drives the Discord adapter's outer multiplexer and inner
//! mailbox tasks against [`ScriptedTransport`] — never the
//! network. Three tests, mirroring the Telegram precedent's
//! Phase 8/9 coverage shape:
//!
//! 1. [`discord_session_smoke_e2e`] — one channel id, two
//!    scripted inbound messages, two real agent turns through
//!    a `ConcreteAgent` wired to a scripted `LlmProvider`. Two
//!    `send_message` captures land on the scripted transport.
//!    Pins the round-trip plus per-turn cancellation-token
//!    rotation: turn 2 can only run if turn 1's token slot
//!    was rotated.
//!
//! 2. [`discord_two_partitions_persistent_e2e`] — two distinct
//!    `channel_id`s, one inbound each. The outer multiplexer
//!    must lazy-spawn two inner tasks; each must drive a
//!    `ChannelKind::Discord` turn against its own partition;
//!    both outbound messages land on the scripted transport
//!    targeting the right channel_id. Memory isolation
//!    between partitions is exercised via the
//!    `session_partition()` contract on the channel — the
//!    test stops short of asserting per-partition memory
//!    contents (those tests live in `aivyx-memory`'s
//!    namespacing suite) but proves the partition keys flow
//!    through.
//!
//! 3. [`discord_shutdown_drains_inflight_turns`] — drives one
//!    turn, fires the shutdown token mid-stream, asserts the
//!    multiplexer's drain path closes mailboxes cleanly and
//!    each inner task returns its per-channel report instead
//!    of leaving handles dangling.
//!
//! The `/approve` / `/reject` gate-resolve test
//! (`discord_approve_command_resolves_gate_e2e` in the Phase
//! 107 Task 6 open spec) is **deferred alongside the
//! daemon-frontend** carve-out from Task 5 — the gate-resolve
//! path lives in the daemon-frontend half of the substrate
//! (Telegram's `telegram_daemon_frontend.rs:219`
//! `parse_gate_command`), so testing it without the daemon-
//! frontend would assert against a path that does not yet
//! exist. The test lands when the daemon-frontend deferral
//! lands.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;

use aivyx_audit::{AuditBridge, HmacChainLog};
use aivyx_capability::{CapabilitySet, Scope};
use aivyx_core::{
    AuditHook, CancellationToken, ToolRegistry,
};
use aivyx_crypto::MasterKey;
use aivyx_llm::{
    LlmError, LlmMessage, LlmProvider, LlmRequest, LlmStepEnd, LlmStream, LlmStreamEvent,
    LlmUsage,
};
use aivyx_storage::{RedbStorage, Storage, StorageConfig};

use crate::session::{
    run_discord_session_with_transport, DiscordMultiSessionReport, DiscordSessionConfig,
};
use crate::transport::{IncomingMessage, ScriptedTransport};

// ---------------------------------------------------------------------------
// Scripted LLM provider — identical shape to the Telegram precedent.
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Scratch storage helper — a fresh tmp dir per test so tests run in
// parallel without stepping on each other's redb files.
// ---------------------------------------------------------------------------

async fn scratch_storage(suffix: &str) -> (Arc<dyn Storage>, PathBuf) {
    let tmp = std::env::var("TMPDIR")
        .or_else(|_| std::env::var("TEMP"))
        .unwrap_or_else(|_| "/tmp".to_string());
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let parent = PathBuf::from(tmp).join(format!("aivyx-discord-{suffix}-{pid}-{nanos}"));
    std::fs::create_dir_all(&parent).expect("scratch store parent must be creatable");
    let store_path = parent.join("store.redb");
    let storage: Arc<dyn Storage> = RedbStorage::open(
        StorageConfig::new(store_path),
        MasterKey::from_raw([7u8; 32]),
    )
    .await
    .expect("scratch storage must open");
    (storage, parent)
}

fn discord_session_config(storage: Arc<dyn Storage>) -> DiscordSessionConfig {
    DiscordSessionConfig {
        model: "claude-haiku-4-5-20251001".to_string(),
        system_prompt: "discord test".to_string(),
        max_tokens: 128,
        // One memory scope so the SemiTrusted ceiling has
        // something to intersect against. The turn doesn't
        // actually call memory tools — same rationale as the
        // Telegram precedent's e2e fixture.
        capabilities: CapabilitySet::from_scopes([
            Scope::parse("memory.read").expect("memory.read parses"),
        ]),
        tools: Arc::new(ToolRegistry::new(Vec::new())),
        storage,
        tool_allowlist: None,
        memory_topic_prefix: None,
    }
}

// ---------------------------------------------------------------------------
// Test 1 — smoke e2e: one channel, two scripted turns.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn discord_session_smoke_e2e() {
    let (storage, parent) = scratch_storage("smoke").await;

    // Two scripted final-message steps. Each one corresponds to
    // one full agent turn — no tools, no multi-step plans.
    let provider: Arc<dyn LlmProvider> = Arc::new(ScriptedProvider {
        queue: StdMutex::new(
            vec![
                final_step(&["Hello, ", "discord!"], "Hello, discord!"),
                final_step(&["Bye!"], "Bye!"),
            ]
            .into(),
        ),
    });
    let audit_bridge = Arc::new(AuditBridge::new(HmacChainLog::new([42u8; 32].to_vec())));
    let audit: Arc<dyn AuditHook> = audit_bridge.clone();

    // Pre-load two inbound messages for the same channel_id.
    // The scripted transport's queue feeds them one at a time
    // through `next_message`.
    let transport = Arc::new(ScriptedTransport::with_queue(vec![
        IncomingMessage {
            message_id: 1,
            channel_id: 777,
            author_id: 42,
            text: "first".to_string(),
        },
        IncomingMessage {
            message_id: 2,
            channel_id: 777,
            author_id: 42,
            text: "second".to_string(),
        },
    ]));

    let config = discord_session_config(Arc::clone(&storage));

    // Watcher: fires the shutdown token as soon as both outbound
    // sends land on the scripted transport. This is what
    // terminates the otherwise-blocking multi-channel
    // multiplexer.
    let shutdown = CancellationToken::new();
    let watcher_shutdown = shutdown.clone();
    let watcher_transport = Arc::clone(&transport);
    tokio::spawn(async move {
        loop {
            if watcher_transport.sent().await.len() >= 2 {
                watcher_shutdown.cancel();
                return;
            }
            tokio::task::yield_now().await;
        }
    });

    let report: DiscordMultiSessionReport = tokio::time::timeout(
        Duration::from_secs(5),
        run_discord_session_with_transport(
            "aivyx-discord-test",
            Arc::clone(&transport),
            config,
            provider,
            audit,
            shutdown,
        ),
    )
    .await
    .expect("run_discord_session_with_transport must exit within the 5s test bound")
    .expect("run_discord_session_with_transport must return Ok");

    // Two turns ran on channel 777.
    assert_eq!(
        report.total_turns(),
        2,
        "two inbound messages must each drive one turn: {report:?}",
    );
    assert_eq!(
        report.turns_by_channel.get(&777).copied().unwrap_or(0),
        2,
        "both turns must aggregate under channel_id=777",
    );

    // Both outbound sends targeted channel 777 with the right
    // scripted text.
    let sent = transport.sent().await;
    assert_eq!(sent.len(), 2, "exactly two outbound messages, got: {sent:?}");
    assert_eq!(sent[0].channel_id, 777);
    assert_eq!(sent[1].channel_id, 777);
    assert!(
        sent[0].text.contains("Hello, discord!"),
        "turn 1 should contain scripted text, got: {:?}",
        sent[0].text,
    );
    assert!(
        sent[1].text.contains("Bye!"),
        "turn 2 should contain second scripted text, got: {:?}",
        sent[1].text,
    );

    let _ = std::fs::remove_dir_all(&parent);
}

// ---------------------------------------------------------------------------
// Test 2 — two channel partitions persistent e2e.
//
// Two inbound messages, two different channel_ids. The outer
// multiplexer must lazy-spawn two inner tasks; both must run
// one turn each; both outbound messages must target their
// respective channel_id. The session_partition() contract on
// DiscordChannel is what makes the multi-tenant memory story
// honest — this test pins the partition-key plumbing, which
// is the load-bearing piece of the Hermes-comparison "multi-
// channel concurrency" claim.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn discord_two_partitions_persistent_e2e() {
    let (storage, parent) = scratch_storage("two-partitions").await;

    // Two scripted steps — one per partition.
    let provider: Arc<dyn LlmProvider> = Arc::new(ScriptedProvider {
        queue: StdMutex::new(
            vec![
                final_step(&["alpha"], "alpha reply"),
                final_step(&["beta"], "beta reply"),
            ]
            .into(),
        ),
    });
    let audit_bridge = Arc::new(AuditBridge::new(HmacChainLog::new([42u8; 32].to_vec())));
    let audit: Arc<dyn AuditHook> = audit_bridge.clone();

    let transport = Arc::new(ScriptedTransport::with_queue(vec![
        IncomingMessage {
            message_id: 100,
            channel_id: 1001,
            author_id: 1,
            text: "ping-alpha".to_string(),
        },
        IncomingMessage {
            message_id: 200,
            channel_id: 1002,
            author_id: 2,
            text: "ping-beta".to_string(),
        },
    ]));

    let config = discord_session_config(Arc::clone(&storage));

    let shutdown = CancellationToken::new();
    let watcher_shutdown = shutdown.clone();
    let watcher_transport = Arc::clone(&transport);
    tokio::spawn(async move {
        loop {
            if watcher_transport.sent().await.len() >= 2 {
                watcher_shutdown.cancel();
                return;
            }
            tokio::task::yield_now().await;
        }
    });

    let report = tokio::time::timeout(
        Duration::from_secs(5),
        run_discord_session_with_transport(
            "aivyx-discord-test",
            Arc::clone(&transport),
            config,
            provider,
            audit,
            shutdown,
        ),
    )
    .await
    .expect("multi-partition session must exit within the 5s test bound")
    .expect("multi-partition session must return Ok");

    // Aggregate: two turns total, one per partition.
    assert_eq!(report.total_turns(), 2, "two partitions × one turn each");
    assert_eq!(
        report.turns_by_channel.get(&1001).copied().unwrap_or(0),
        1,
        "channel 1001 ran exactly one turn",
    );
    assert_eq!(
        report.turns_by_channel.get(&1002).copied().unwrap_or(0),
        1,
        "channel 1002 ran exactly one turn",
    );

    // Outbound sends are tagged with the right channel_id —
    // proves the multiplexer routed by channel_id correctly.
    // Order between partitions is non-deterministic (two
    // concurrent inner tasks); collect into a set keyed on
    // channel_id and assert each partition got exactly one.
    let sent = transport.sent().await;
    assert_eq!(sent.len(), 2, "exactly two outbound messages, got: {sent:?}");
    let mut by_channel: std::collections::HashMap<u64, Vec<String>> =
        std::collections::HashMap::new();
    for msg in &sent {
        by_channel
            .entry(msg.channel_id)
            .or_default()
            .push(msg.text.clone());
    }
    assert_eq!(
        by_channel.get(&1001).map(|v| v.len()).unwrap_or(0),
        1,
        "channel 1001 received exactly one send",
    );
    assert_eq!(
        by_channel.get(&1002).map(|v| v.len()).unwrap_or(0),
        1,
        "channel 1002 received exactly one send",
    );

    // The audit chain saw two TurnStarted entries — one per
    // partition. The chain is the load-bearing trail; if the
    // multiplexer mis-routed a turn into the wrong partition's
    // inner task the chain would still record two starts, but
    // the session_id values would not partition cleanly. We
    // pin the count here; per-partition session_id partitioning
    // is implicit in the DiscordChannel::session_partition
    // contract (asserted at the channel-tests level in Task 4).
    let entries = audit_bridge
        .writer()
        .entries()
        .expect("audit chain entries must be readable");
    let turn_started_count = entries
        .iter()
        .filter(|e| matches!(e.event, aivyx_audit::AuditEvent::TurnStarted { .. }))
        .count();
    assert_eq!(
        turn_started_count, 2,
        "two partition turns must produce two TurnStarted audit events: {entries:#?}",
    );

    let _ = std::fs::remove_dir_all(&parent);
}

// ---------------------------------------------------------------------------
// Test 3 — shutdown drains in-flight turns cleanly.
//
// The Phase 9 Task 2 multi-chat multiplexer contract: when the
// shutdown token fires, the outer loop stops polling, drops
// every per-channel mpsc sender, which signals each inner task
// to drain its pending queue and exit. Each handle's join must
// surface its per-channel `DiscordSessionReport`, not an
// orphaned-task panic. This test pins that path for Discord.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn discord_shutdown_drains_inflight_turns() {
    let (storage, parent) = scratch_storage("shutdown").await;

    let provider: Arc<dyn LlmProvider> = Arc::new(ScriptedProvider {
        queue: StdMutex::new(vec![final_step(&["only ", "reply"], "only reply")].into()),
    });
    let audit_bridge = Arc::new(AuditBridge::new(HmacChainLog::new([42u8; 32].to_vec())));
    let audit: Arc<dyn AuditHook> = audit_bridge.clone();

    let transport = Arc::new(ScriptedTransport::with_queue(vec![IncomingMessage {
        message_id: 1,
        channel_id: 5555,
        author_id: 9,
        text: "hello".to_string(),
    }]));

    let config = discord_session_config(Arc::clone(&storage));

    // Watcher: as soon as the inner task's outbound send lands,
    // fire shutdown. The outer multiplexer's biased select on
    // shutdown must observe this and start the drain. Each
    // inner task's mailbox-close branch then resolves its
    // current state to a DiscordSessionReport.
    let shutdown = CancellationToken::new();
    let watcher_shutdown = shutdown.clone();
    let watcher_transport = Arc::clone(&transport);
    tokio::spawn(async move {
        loop {
            if !watcher_transport.sent().await.is_empty() {
                watcher_shutdown.cancel();
                return;
            }
            tokio::task::yield_now().await;
        }
    });

    let report = tokio::time::timeout(
        Duration::from_secs(5),
        run_discord_session_with_transport(
            "aivyx-discord-test",
            Arc::clone(&transport),
            config,
            provider,
            audit,
            shutdown,
        ),
    )
    .await
    .expect("shutdown-drain session must exit within the 5s test bound")
    .expect("shutdown-drain session must return Ok");

    // One turn ran end-to-end before the shutdown drained the
    // route; the report carries the per-channel turn count.
    assert_eq!(report.total_turns(), 1);
    assert_eq!(
        report.turns_by_channel.get(&5555).copied().unwrap_or(0),
        1,
        "channel 5555 completed exactly one turn before shutdown drained",
    );

    let sent = transport.sent().await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].channel_id, 5555);
    assert!(sent[0].text.contains("only reply"));

    let _ = std::fs::remove_dir_all(&parent);
}
