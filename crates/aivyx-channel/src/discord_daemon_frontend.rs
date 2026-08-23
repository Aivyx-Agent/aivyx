//! Phase 111 Task 3 — Daemon-mode Discord multi-channel pump.
//!
//! Mirrors the Phase 19 `telegram_daemon_frontend.rs` pattern
//! exactly per Q2a sign-off. Drives a multi-channel Discord
//! frontend over the daemon IPC channel: one outer
//! `next_message` loop pumps the twilight-rs Gateway,
//! per-channel routing fans out to inner mailbox tasks, each
//! inner task submits turns through a `DaemonSession` instead
//! of constructing an agent.
//!
//! The Phase 107 carve-out that this module closes: the
//! in-process path (Phase 107 Task 5) wired `--channel
//! discord` end-to-end through `run_discord_session`, but the
//! daemon-mode-over-IPC path was deferred. Phase 111 ships it,
//! mirroring the proven Telegram pattern at two data points
//! confirming the daemon-frontend shape is reusable across
//! SemiTrusted adapters.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use aivyx_core::{
    CancellationToken, ChannelContext, ChannelError, ChannelPlatform, SessionId, StreamEvent,
    TurnOutcome,
};
use aivyx_discord::transport::{
    DiscordTransport, IncomingMessage, OutgoingMessage, TwilightTransport,
};

use crate::daemon_client::DaemonSession;
use crate::daemon_ipc::{FrontendType, StreamEventPayload};
use crate::daemon_server::DaemonError;
use crate::gate_command;
use crate::team_command;
use crate::team_dispatch;

// ---------------------------------------------------------------------------
// DiscordDaemonChannel — identity stub for the daemon's ChannelFactory
// ---------------------------------------------------------------------------

/// Lightweight `ChannelContext` stub that reports `SemiTrusted`
/// trust tier and `Discord` platform. Used by the daemon's
/// `ChannelFactory` when a `FrontendType::Discord` connection
/// arrives. The stub's `stream_event` and `finalize` are
/// no-ops — the daemon-side `IpcChannelBridge` handles
/// forwarding events over IPC; this struct exists only so the
/// daemon's `ChannelFactory` has a `ChannelContext` to hand to
/// `ConcreteAgent` at construction time.
pub struct DiscordDaemonChannel {
    session: SessionId,
    /// Rotated per turn by [`reset_cancellation`] and fired by
    /// [`cancel_inflight`]. See `TelegramDaemonChannel::token` —
    /// same C1+H1 audit fix.
    token: Mutex<CancellationToken>,
}

impl DiscordDaemonChannel {
    pub fn new() -> Self {
        DiscordDaemonChannel {
            session: SessionId::new(),
            token: Mutex::new(CancellationToken::new()),
        }
    }
}

impl Default for DiscordDaemonChannel {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ChannelContext for DiscordDaemonChannel {
    fn channel_name(&self) -> &str {
        "aivyx-discord-daemon"
    }

    fn platform(&self) -> ChannelPlatform {
        ChannelPlatform::Discord
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

struct ChannelRoute {
    sender: tokio::sync::mpsc::Sender<IncomingMessage>,
    handle: tokio::task::JoinHandle<Result<(), DaemonError>>,
}

/// Drive a multi-channel Discord frontend over the daemon IPC
/// channel. Mirrors `run_telegram_daemon_multi_session` from
/// Phase 19 exactly — same outer-loop shape, same per-route
/// inner-task spawn, same shutdown drain.
pub async fn run_discord_daemon_multi_session(
    transport: Arc<TwilightTransport>,
    socket_path: PathBuf,
    role: Option<String>,
    shutdown: CancellationToken,
) -> Result<(), DaemonError> {
    let mut routes: HashMap<u64, ChannelRoute> = HashMap::new();

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
                        "aivyx-discord(daemon): next_message failed ({e}); backing off 1s"
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
            }
        };

        let channel_id = msg.channel_id;

        let route = routes.entry(channel_id).or_insert_with(|| {
            let (tx, rx) = tokio::sync::mpsc::channel(32);
            let transport_clone = Arc::clone(&transport);
            let sp = socket_path.clone();
            let role_clone = role.clone();
            let shutdown_clone = shutdown.clone();
            let handle = tokio::spawn(async move {
                run_discord_daemon_channel_task(
                    transport_clone,
                    channel_id,
                    sp,
                    role_clone,
                    rx,
                    shutdown_clone,
                )
                .await
            });
            ChannelRoute { sender: tx, handle }
        });

        if let Err(e) = route.sender.send(msg).await {
            eprintln!(
                "aivyx-discord(daemon): channel {channel_id} mailbox send failed ({e}); dropping route"
            );
            routes.remove(&channel_id);
        }
    }

    let drained: Vec<(u64, ChannelRoute)> = routes.drain().collect();
    for (_channel_id, ChannelRoute { sender, handle }) in drained {
        drop(sender);
        match handle.await {
            Ok(Err(e)) => eprintln!("aivyx-discord(daemon): inner task error: {e}"),
            Err(e) => eprintln!("aivyx-discord(daemon): inner task join failed: {e}"),
            Ok(Ok(())) => {}
        }
    }

    Ok(())
}

/// Per-channel inner task: connect a `DaemonSession`, submit
/// turns, accumulate streamed events, send one Discord message
/// per turn. Mirrors `run_telegram_daemon_chat_task` from Phase
/// 19.
async fn run_discord_daemon_channel_task(
    transport: Arc<TwilightTransport>,
    channel_id: u64,
    socket_path: PathBuf,
    role: Option<String>,
    mut mailbox: tokio::sync::mpsc::Receiver<IncomingMessage>,
    shutdown: CancellationToken,
) -> Result<(), DaemonError> {
    let mut session = DaemonSession::connect(
        &socket_path,
        role,
        Some(FrontendType::Discord),
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
                .send_message(OutgoingMessage { channel_id, text: reply })
                .await
                .map_err(|e| {
                    DaemonError::Internal(format!(
                        "send_message to channel {channel_id}: {e}"
                    ))
                })?;
            continue;
        }

        if let Some(team_cmd) = team_command::parse(msg.text.trim()) {
            let reply = team_dispatch::dispatch(&socket_path, team_cmd).await;
            transport
                .send_message(OutgoingMessage { channel_id, text: reply })
                .await
                .map_err(|e| {
                    DaemonError::Internal(format!(
                        "send_message to channel {channel_id}: {e}"
                    ))
                })?;
            continue;
        }

        let (events, _outcome) = session.submit_input(msg.text).await?;
        let buf = render_events_for_discord(&events);

        transport
            .send_message(OutgoingMessage {
                channel_id,
                text: buf,
            })
            .await
            .map_err(|e| {
                DaemonError::Internal(format!(
                    "send_message to channel {channel_id}: {e}"
                ))
            })?;
    }

    let _ = session.disconnect().await;
    Ok(())
}

/// Render accumulated `StreamEventPayload` events into a single
/// Discord message. Matches `DiscordChannel::append_event`'s
/// rendering style (Phase 107 Task 4) so the in-process and
/// daemon-mode paths produce byte-identical Discord messages
/// for the same turn outcome.
pub(crate) fn render_events_for_discord(events: &[StreamEventPayload]) -> String {
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
    fn discord_daemon_channel_reports_correct_identity() {
        let c = DiscordDaemonChannel::new();
        assert_eq!(c.channel_name(), "aivyx-discord-daemon");
        assert_eq!(c.platform(), ChannelPlatform::Discord);
        assert_eq!(c.trust_tier(), aivyx_capability::TrustTier::SemiTrusted);
    }

    // Audit C1+H1 regression — daemon /cancel must fire
    // the stub's token, and per-turn reset must un-stick
    // it. Same shape as TelegramDaemonChannel's tests.
    #[test]
    fn cancel_inflight_then_reset_yields_fresh_token() {
        let c = DiscordDaemonChannel::new();
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
            StreamEventPayload::Text { text: "discord".into() },
        ];
        assert_eq!(render_events_for_discord(&events), "hello discord");
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
        let out = render_events_for_discord(&events);
        assert!(out.contains("→ memory.read"));
        assert!(out.contains("← memory.read ok"));
    }

    #[test]
    fn render_events_empty_payload_returns_no_reply_placeholder() {
        assert_eq!(render_events_for_discord(&[]), "(no reply)");
    }

    #[test]
    fn render_events_approval_gate_includes_text_command_hints() {
        let events = vec![StreamEventPayload::ApprovalGate {
            mission_id: "m-001".into(),
            gate_id: "g-abc".into(),
            scope: Some("fs.delete".into()),
            reason: "delete a file".into(),
        }];
        let out = render_events_for_discord(&events);
        assert!(out.contains("⚑ APPROVAL GATE [m-001/g-abc]"));
        assert!(out.contains("Reply /approve m-001 g-abc"));
        assert!(out.contains("or    /reject  m-001 g-abc"));
    }
}
