//! Phase 111 Task 5 — Daemon-mode Slack multi-channel pump.
//!
//! Mirrors the Phase 19 `telegram_daemon_frontend.rs` and the
//! Phase 111 Task 3 `discord_daemon_frontend.rs` patterns.
//! Drives a multi-channel Slack frontend over the daemon IPC
//! channel: one outer `next_message` loop pumps the Socket Mode
//! WebSocket (via the now-live `SlackMorphismTransport` from
//! Task 4), per-partition routing fans out to inner mailbox
//! tasks, each inner task submits turns through a
//! `DaemonSession` instead of constructing an agent.
//!
//! ## Three data points for the daemon-frontend shape
//!
//! Telegram (Phase 19), Discord (Phase 111 Task 3), and Slack
//! (Phase 111 Task 5) now all run the same daemon-frontend
//! shape. The Q-block at Phase 111 Task 2 sign-off (Q2a —
//! mirror Phase 19 exactly) anticipated this: if Slack
//! converges on the same shape, the **shared substrate
//! question** raised in the open doc gets answered
//! affirmatively at three data points. A future small
//! refactor could lift the per-route routing logic into a
//! shared helper; Phase 111 ships three siblings per the
//! adapter-pattern doc's "extract only when forced" rule.
//!
//! ## Partition key — `(team_id, channel_id)` vs. `u64` /
//! `i64`
//!
//! Slack's partition key is a string (`"{team_id}:{channel_id}"`
//! per Phase 108 Q3a) where Discord and Telegram used `u64`
//! and `i64` respectively. The routing `HashMap` is keyed
//! on `String` here; the shape is otherwise identical.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use aivyx_core::{
    CancellationToken, ChannelContext, ChannelError, ChannelPlatform, SessionId, StreamEvent,
    TurnOutcome,
};
use aivyx_slack::transport::{
    IncomingMessage, OutgoingMessage, SlackMorphismTransport, SlackTransport,
};

use crate::daemon_client::DaemonSession;
use crate::daemon_ipc::{FrontendType, StreamEventPayload};
use crate::daemon_server::DaemonError;
use crate::gate_command;
use crate::team_command;
use crate::team_dispatch;

// ---------------------------------------------------------------------------
// SlackDaemonChannel — identity stub for the daemon's ChannelFactory
// ---------------------------------------------------------------------------

/// Lightweight `ChannelContext` stub that reports `SemiTrusted`
/// trust tier and `Slack` platform. Used by the daemon's
/// `ChannelFactory` when a `FrontendType::Slack` connection
/// arrives. Same shape as `TelegramDaemonChannel` and
/// `DiscordDaemonChannel`; `stream_event` and `finalize` are
/// no-ops because the daemon-side `IpcChannelBridge` handles
/// forwarding events over IPC.
pub struct SlackDaemonChannel {
    session: SessionId,
    /// Rotated per turn by [`reset_cancellation`] and fired by
    /// [`cancel_inflight`]. See `TelegramDaemonChannel::token` —
    /// same C1+H1 audit fix.
    token: Mutex<CancellationToken>,
}

impl SlackDaemonChannel {
    pub fn new() -> Self {
        SlackDaemonChannel {
            session: SessionId::new(),
            token: Mutex::new(CancellationToken::new()),
        }
    }
}

impl Default for SlackDaemonChannel {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ChannelContext for SlackDaemonChannel {
    fn channel_name(&self) -> &str {
        "aivyx-slack-daemon"
    }

    fn platform(&self) -> ChannelPlatform {
        ChannelPlatform::Slack
    }

    fn trust_tier(&self) -> aivyx_capability::TrustTier {
        aivyx_capability::TrustTier::SemiTrusted
    }

    fn session_id(&self) -> SessionId {
        self.session
    }

    async fn stream_event(&self, _event: StreamEvent<'_>) -> Result<(), ChannelError> {
        Ok(())
    }

    async fn finalize(&self, _outcome: &TurnOutcome) -> Result<(), ChannelError> {
        Ok(())
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.token.lock().expect("token mutex poisoned").clone()
    }

    fn reset_cancellation(&self) {
        let mut slot = self.token.lock().expect("token mutex poisoned");
        *slot = CancellationToken::new();
    }

    fn cancel_inflight(&self) {
        self.token.lock().expect("token mutex poisoned").cancel();
    }
}

// ---------------------------------------------------------------------------
// Multi-channel pump
// ---------------------------------------------------------------------------

struct PartitionRoute {
    sender: tokio::sync::mpsc::Sender<IncomingMessage>,
    handle: tokio::task::JoinHandle<Result<(), DaemonError>>,
}

/// Drive a multi-channel Slack frontend over the daemon IPC
/// channel. Mirrors `run_discord_daemon_multi_session` (Phase
/// 111 Task 3) and `run_telegram_daemon_multi_session` (Phase
/// 19); the only platform-specific surface is the partition
/// key — Slack uses `String` for `(team_id, channel_id)`
/// rather than Discord's `u64` or Telegram's `i64`.
pub async fn run_slack_daemon_multi_session(
    transport: Arc<SlackMorphismTransport>,
    socket_path: PathBuf,
    role: Option<String>,
    shutdown: CancellationToken,
) -> Result<(), DaemonError> {
    let mut routes: HashMap<String, PartitionRoute> = HashMap::new();

    loop {
        if shutdown.is_cancelled() {
            break;
        }

        let msg = tokio::select! {
            biased;
            _ = shutdown.cancelled() => break,
            res = transport.next_message() => match res {
                Ok(m) => m,
                Err(e) => {
                    eprintln!(
                        "aivyx-slack(daemon): next_message failed ({e}); backing off 1s"
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
            }
        };

        let partition = msg.partition_key();

        let route = routes.entry(partition.clone()).or_insert_with(|| {
            let (tx, rx) = tokio::sync::mpsc::channel(32);
            let transport_clone = Arc::clone(&transport);
            let sp = socket_path.clone();
            let role_clone = role.clone();
            let shutdown_clone = shutdown.clone();
            let partition_for_task = partition.clone();
            let handle = tokio::spawn(async move {
                run_slack_daemon_partition_task(
                    transport_clone,
                    partition_for_task,
                    sp,
                    role_clone,
                    rx,
                    shutdown_clone,
                )
                .await
            });
            PartitionRoute { sender: tx, handle }
        });

        if let Err(e) = route.sender.send(msg).await {
            eprintln!(
                "aivyx-slack(daemon): partition {partition} mailbox send failed ({e}); dropping route"
            );
            routes.remove(&partition);
        }
    }

    let drained: Vec<(String, PartitionRoute)> = routes.drain().collect();
    for (_partition, PartitionRoute { sender, handle }) in drained {
        drop(sender);
        match handle.await {
            Ok(Err(e)) => eprintln!("aivyx-slack(daemon): inner task error: {e}"),
            Err(e) => eprintln!("aivyx-slack(daemon): inner task join failed: {e}"),
            Ok(Ok(())) => {}
        }
    }

    Ok(())
}

/// Per-partition inner task. Mirrors
/// `run_discord_daemon_channel_task` (Phase 111 Task 3) and
/// `run_telegram_daemon_chat_task` (Phase 19) — the only
/// per-adapter difference is the channel-id type used to
/// route outbound `OutgoingMessage`s.
async fn run_slack_daemon_partition_task(
    transport: Arc<SlackMorphismTransport>,
    partition: String,
    socket_path: PathBuf,
    role: Option<String>,
    mut mailbox: tokio::sync::mpsc::Receiver<IncomingMessage>,
    shutdown: CancellationToken,
) -> Result<(), DaemonError> {
    let mut session = DaemonSession::connect(
        &socket_path,
        role,
        Some(FrontendType::Slack),
    )
    .await?;

    loop {
        let msg = tokio::select! {
            biased;
            _ = shutdown.cancelled() => break,
            maybe_msg = mailbox.recv() => match maybe_msg {
                Some(m) => m,
                None => break,
            }
        };

        if msg.text.trim() == "/cancel" {
            let _ = session.cancel_turn().await;
            continue;
        }

        if let Some(gate_cmd) = gate_command::parse(msg.text.trim()) {
            let result = session
                .resolve_gate(gate_cmd.mission_id, gate_cmd.gate_id, gate_cmd.approved)
                .await;
            let reply = match result {
                Ok(()) => {
                    let status = if gate_cmd.approved { "approved" } else { "rejected" };
                    format!("✓ Gate {status}.")
                }
                Err(e) => format!("✗ Gate resolve failed: {e}"),
            };
            transport
                .send_message(OutgoingMessage {
                    channel_id: msg.channel_id.clone(),
                    text: reply,
                })
                .await
                .map_err(|e| {
                    DaemonError::Internal(format!(
                        "send_message to partition {partition}: {e}"
                    ))
                })?;
            continue;
        }

        if let Some(team_cmd) = team_command::parse(msg.text.trim()) {
            let reply = team_dispatch::dispatch(&socket_path, team_cmd).await;
            transport
                .send_message(OutgoingMessage {
                    channel_id: msg.channel_id.clone(),
                    text: reply,
                })
                .await
                .map_err(|e| {
                    DaemonError::Internal(format!(
                        "send_message to partition {partition}: {e}"
                    ))
                })?;
            continue;
        }

        let (events, _outcome) = session.submit_input(msg.text).await?;
        let buf = render_events_for_slack(&events);

        transport
            .send_message(OutgoingMessage {
                channel_id: msg.channel_id.clone(),
                text: buf,
            })
            .await
            .map_err(|e| {
                DaemonError::Internal(format!(
                    "send_message to partition {partition}: {e}"
                ))
            })?;
    }

    let _ = session.disconnect().await;
    Ok(())
}

/// Render accumulated `StreamEventPayload` events into a
/// single Slack message. Byte-identical to
/// `render_events_for_discord` (Phase 111 Task 3) and
/// structurally identical to `render_events_for_telegram`
/// (Phase 19) — the three SemiTrusted adapters all produce
/// the same in-message UX deliberately, so an operator
/// switching between them sees no surprise UI deltas.
pub(crate) fn render_events_for_slack(events: &[StreamEventPayload]) -> String {
    let mut buf = String::new();
    for event in events {
        match event {
            StreamEventPayload::Text { text } => {
                buf.push_str(text);
            }
            StreamEventPayload::Status { status } => {
                if !buf.is_empty() && !buf.ends_with('\n') {
                    buf.push('\n');
                }
                buf.push_str("… ");
                buf.push_str(status);
                buf.push('\n');
            }
            StreamEventPayload::ToolCallStarted { tool_name, .. } => {
                if !buf.is_empty() && !buf.ends_with('\n') {
                    buf.push('\n');
                }
                buf.push_str("→ ");
                buf.push_str(tool_name);
                buf.push('\n');
            }
            StreamEventPayload::ToolCallFinished {
                tool_name,
                outcome_summary,
                ..
            } => {
                if !buf.is_empty() && !buf.ends_with('\n') {
                    buf.push('\n');
                }
                buf.push_str("← ");
                buf.push_str(tool_name);
                buf.push(' ');
                buf.push_str(outcome_summary);
                buf.push('\n');
            }
            StreamEventPayload::ToolOutput { .. } => {}
            StreamEventPayload::ApprovalGate {
                mission_id,
                gate_id,
                reason,
                ..
            } => {
                if !buf.is_empty() && !buf.ends_with('\n') {
                    buf.push('\n');
                }
                buf.push_str(&format!(
                    "⚑ APPROVAL GATE [{mission_id}/{gate_id}]: {reason}\n\
                     Reply /approve {mission_id} {gate_id}\n\
                     or    /reject  {mission_id} {gate_id}\n"
                ));
            }
        }
    }

    if buf.trim().is_empty() {
        "(no reply)".to_string()
    } else {
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon_ipc::StreamEventPayload;

    #[test]
    fn slack_daemon_channel_reports_correct_identity() {
        let c = SlackDaemonChannel::new();
        assert_eq!(c.channel_name(), "aivyx-slack-daemon");
        assert_eq!(c.platform(), ChannelPlatform::Slack);
        assert_eq!(c.trust_tier(), aivyx_capability::TrustTier::SemiTrusted);
    }

    // Audit C1+H1 regression — same coverage as the
    // Telegram + Discord stubs.
    #[test]
    fn cancel_inflight_then_reset_yields_fresh_token() {
        let c = SlackDaemonChannel::new();
        let stale = c.cancellation_token();
        assert!(!stale.is_cancelled());
        c.cancel_inflight();
        assert!(stale.is_cancelled(), "cancel_inflight fires the live token");
        c.reset_cancellation();
        assert!(
            !c.cancellation_token().is_cancelled(),
            "reset_cancellation installs a fresh token"
        );
        assert!(stale.is_cancelled(), "the pre-reset token stays cancelled");
    }

    #[test]
    fn render_events_text_concatenates() {
        let events = vec![
            StreamEventPayload::Text { text: "hello ".into() },
            StreamEventPayload::Text { text: "slack".into() },
        ];
        assert_eq!(render_events_for_slack(&events), "hello slack");
    }

    #[test]
    fn render_events_tool_call_arrows_render_consistently() {
        let events = vec![
            StreamEventPayload::ToolCallStarted {
                tool_id: "tid".into(),
                tool_name: "memory.read".into(),
                input: serde_json::Value::Null,
            },
            StreamEventPayload::ToolCallFinished {
                tool_id: "tid".into(),
                tool_name: "memory.read".into(),
                outcome_summary: "ok".into(),
            },
        ];
        let out = render_events_for_slack(&events);
        assert!(out.contains("→ memory.read"));
        assert!(out.contains("← memory.read ok"));
    }

    #[test]
    fn render_events_empty_payload_returns_no_reply_placeholder() {
        assert_eq!(render_events_for_slack(&[]), "(no reply)");
    }

    #[test]
    fn render_events_byte_identical_to_discord_renderer() {
        // Three-data-point sanity check: the Slack and Discord
        // renderers must produce byte-identical output for the
        // same StreamEventPayload sequence. If a future divergence
        // ships (e.g. Slack-specific markdown), this test breaks
        // and forces the divergence to be documented.
        let events = vec![
            StreamEventPayload::Text { text: "result: ".into() },
            StreamEventPayload::ToolCallStarted {
                tool_id: "tid".into(),
                tool_name: "fs.read".into(),
                input: serde_json::Value::Null,
            },
            StreamEventPayload::Status { status: "thinking".into() },
            StreamEventPayload::ApprovalGate {
                mission_id: "m-001".into(),
                gate_id: "g-abc".into(),
                scope: Some("fs.write".into()),
                reason: "write a file".into(),
            },
        ];
        let slack_out = render_events_for_slack(&events);
        let discord_out =
            crate::discord_daemon_frontend::render_events_for_discord(&events);
        assert_eq!(slack_out, discord_out);
    }
}
