//! Phase 108 Task 6 — scripted end-to-end suite.
//!
//! Drives the Slack adapter's outer multiplexer and inner
//! mailbox tasks against [`ScriptedTransport`] — never the
//! network. Three tests, mirroring the Discord precedent's
//! Phase 107 coverage shape but with the Slack-specific
//! `(team_id, channel_id)` partition wrinkle pinned hard:
//!
//! 1. [`slack_session_smoke_e2e`] — one partition, two
//!    scripted inbound messages, two real agent turns. Two
//!    `send_message` captures land on the scripted transport.
//!
//! 2. [`slack_session_two_partitions_persistent_e2e`] — two
//!    distinct `(team_id, channel_id)` pairs **including the
//!    cross-team-same-channel_id case** (the load-bearing
//!    four-data-point assertion). Proves the Q3a
//!    partition-key stringification cleanly separates the
//!    two even when `channel_id` collides across teams.
//!
//! 3. [`slack_session_shutdown_drains_inflight_turns`] —
//!    multi-channel multiplexer's shutdown-drain contract for
//!    Slack: one turn completes, shutdown fires, every
//!    per-partition sender drops, inner tasks resolve to
//!    `SlackSessionReport`.
//!
//! Real-protocol smoke against a live Slack bot is **deferred
//! to the Channel Activation Milestone** per
//! `docs/ADAPTER_PATTERN.md` checklist item 7. The
//! `SlackMorphismTransport` callback-state-passing deferral
//! lands alongside the Phase 107 daemon-frontend follow-on.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;

use aivyx_audit::{AuditBridge, HmacChainLog};
use aivyx_capability::{CapabilitySet, Scope};
use aivyx_core::{AuditHook, CancellationToken, ToolRegistry};
use aivyx_crypto::MasterKey;
use aivyx_llm::{
    LlmError, LlmMessage, LlmProvider, LlmRequest, LlmStepEnd, LlmStream, LlmStreamEvent,
    LlmUsage,
};
use aivyx_storage::{RedbStorage, Storage, StorageConfig};

use crate::session::{
    run_slack_session_with_transport, SlackMultiSessionReport, SlackSessionConfig,
};
use crate::transport::{IncomingMessage, ScriptedTransport};

// ---------------------------------------------------------------------------
// Scripted LLM provider — identical to the Discord precedent.
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
        assert!(!request.messages.is_empty());
        assert!(matches!(request.messages[0], LlmMessage::User { .. }));
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
// Scratch storage helper — fresh tmp dir per test for parallel safety.
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
    let parent = PathBuf::from(tmp).join(format!("aivyx-slack-{suffix}-{pid}-{nanos}"));
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

fn slack_session_config(storage: Arc<dyn Storage>) -> SlackSessionConfig {
    SlackSessionConfig {
        model: "claude-haiku-4-5-20251001".to_string(),
        system_prompt: "slack test".to_string(),
        max_tokens: 128,
        capabilities: CapabilitySet::from_scopes([
            Scope::parse("memory.read").expect("memory.read parses"),
        ]),
        tools: Arc::new(ToolRegistry::new(Vec::new())),
        storage,
        tool_allowlist: None,
        memory_topic_prefix: None,
    }
}

fn sample_inbound(team_id: &str, channel_id: &str, text: &str) -> IncomingMessage {
    IncomingMessage {
        team_id: team_id.to_string(),
        channel_id: channel_id.to_string(),
        user_id: "U001".to_string(),
        text: text.to_string(),
        message_ts: "1700000000.000100".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Test 1 — smoke e2e: one (team, channel), two scripted turns.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn slack_session_smoke_e2e() {
    let (storage, parent) = scratch_storage("smoke").await;

    let provider: Arc<dyn LlmProvider> = Arc::new(ScriptedProvider {
        queue: StdMutex::new(
            vec![
                final_step(&["Hello, ", "slack!"], "Hello, slack!"),
                final_step(&["Bye!"], "Bye!"),
            ]
            .into(),
        ),
    });
    let audit_bridge = Arc::new(AuditBridge::new(HmacChainLog::new([42u8; 32].to_vec())));
    let audit: Arc<dyn AuditHook> = audit_bridge.clone();

    let transport = Arc::new(ScriptedTransport::with_queue(vec![
        sample_inbound("T01", "C42", "first"),
        sample_inbound("T01", "C42", "second"),
    ]));

    let config = slack_session_config(Arc::clone(&storage));

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

    let report: SlackMultiSessionReport = tokio::time::timeout(
        Duration::from_secs(5),
        run_slack_session_with_transport(
            "aivyx-slack-test",
            Arc::clone(&transport),
            config,
            provider,
            audit,
            shutdown,
        ),
    )
    .await
    .expect("run_slack_session_with_transport must exit within the 5s test bound")
    .expect("run_slack_session_with_transport must return Ok");

    assert_eq!(
        report.total_turns(),
        2,
        "two inbound messages must each drive one turn: {report:?}",
    );
    assert_eq!(
        report
            .turns_by_partition
            .get("T01:C42")
            .copied()
            .unwrap_or(0),
        2,
        "both turns must aggregate under partition T01:C42",
    );

    let sent = transport.sent().await;
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].channel_id, "C42");
    assert_eq!(sent[1].channel_id, "C42");
    assert!(sent[0].text.contains("Hello, slack!"));
    assert!(sent[1].text.contains("Bye!"));

    let _ = std::fs::remove_dir_all(&parent);
}

// ---------------------------------------------------------------------------
// Test 2 — two partitions persistent e2e (with cross-team same-channel_id).
//
// **Load-bearing four-data-point assertion.** Phase 108 Q3a
// pinned `format!("{team_id}:{channel_id}")` as the partition
// key precisely to handle the case where the same channel_id
// shows up under two different team_ids (rare but possible —
// a Slack bot installed in two workspaces can encounter
// colliding ids). This test pushes a message for
// (TEAM_A, CSHARED) and another for (TEAM_B, CSHARED); the
// outer multiplexer must lazy-spawn two distinct inner tasks
// keyed on the partition string, not on channel_id alone.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn slack_session_two_partitions_persistent_e2e() {
    let (storage, parent) = scratch_storage("two-partitions").await;

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

    // Same channel_id under two different team_ids — the
    // Q3a load-bearing case.
    let transport = Arc::new(ScriptedTransport::with_queue(vec![
        sample_inbound("TEAM_A", "CSHARED", "ping-alpha"),
        sample_inbound("TEAM_B", "CSHARED", "ping-beta"),
    ]));

    let config = slack_session_config(Arc::clone(&storage));

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
        run_slack_session_with_transport(
            "aivyx-slack-test",
            Arc::clone(&transport),
            config,
            provider,
            audit,
            shutdown,
        ),
    )
    .await
    .expect("two-partition session must exit within the 5s test bound")
    .expect("two-partition session must return Ok");

    // Two turns total, one per distinct partition.
    assert_eq!(report.total_turns(), 2);
    assert_eq!(
        report
            .turns_by_partition
            .get("TEAM_A:CSHARED")
            .copied()
            .unwrap_or(0),
        1,
        "TEAM_A:CSHARED ran exactly one turn",
    );
    assert_eq!(
        report
            .turns_by_partition
            .get("TEAM_B:CSHARED")
            .copied()
            .unwrap_or(0),
        1,
        "TEAM_B:CSHARED ran exactly one turn — distinct partition from TEAM_A:CSHARED",
    );

    // Audit chain saw two TurnStarted entries — one per
    // partition. The four-data-point check passes: two
    // partitions with colliding channel_ids partition
    // distinctly, the audit chain records both.
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
        "two distinct partitions must produce two TurnStarted audit events",
    );

    let _ = std::fs::remove_dir_all(&parent);
}

// ---------------------------------------------------------------------------
// Test 3 — shutdown drains in-flight turns.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn slack_session_shutdown_drains_inflight_turns() {
    let (storage, parent) = scratch_storage("shutdown").await;

    let provider: Arc<dyn LlmProvider> = Arc::new(ScriptedProvider {
        queue: StdMutex::new(vec![final_step(&["only ", "reply"], "only reply")].into()),
    });
    let audit_bridge = Arc::new(AuditBridge::new(HmacChainLog::new([42u8; 32].to_vec())));
    let audit: Arc<dyn AuditHook> = audit_bridge.clone();

    let transport = Arc::new(ScriptedTransport::with_queue(vec![sample_inbound(
        "T01", "C55", "hello",
    )]));

    let config = slack_session_config(Arc::clone(&storage));

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
        run_slack_session_with_transport(
            "aivyx-slack-test",
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

    assert_eq!(report.total_turns(), 1);
    assert_eq!(
        report.turns_by_partition.get("T01:C55").copied().unwrap_or(0),
        1,
    );

    let sent = transport.sent().await;
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].channel_id, "C55");
    assert!(sent[0].text.contains("only reply"));

    let _ = std::fs::remove_dir_all(&parent);
}
