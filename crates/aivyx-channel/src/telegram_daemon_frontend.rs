//! Daemon-mode Telegram multi-chat pump — Phase 19 Task 3/4.
//!
//! Drives a multi-chat Telegram frontend over the daemon IPC channel.
//! The outer `get_updates` loop retains the same structure as the
//! in-process `run_telegram_multi_session`: one cursor, per-chat
//! routing, lazy spawn of inner tasks. Each inner task submits turns
//! through a `DaemonSession` instead of constructing an agent.
//!
//! Extracted from the binary in Task 4 to keep `aivyx.rs` under
//! the 2400-line threshold.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use aivyx_core::{
    CancellationToken, ChannelContext, ChannelError, ChannelPlatform, SessionId, StreamEvent,
    TurnOutcome,
};
use aivyx_telegram::transport::{IncomingMessage, OutgoingMessage, ReqwestTransport, TelegramTransport};

use crate::daemon_client::DaemonSession;
use crate::daemon_ipc::{FrontendType, StreamEventPayload};
use crate::daemon_server::DaemonError;
use crate::gate_command;

const LONG_POLL_TIMEOUT_SECS: u32 = 25;

// ---------------------------------------------------------------------------
// TelegramDaemonChannel — identity stub for the daemon's ChannelFactory
// ---------------------------------------------------------------------------

/// Lightweight `ChannelContext` stub that returns `SemiTrusted` trust
/// tier and `Telegram` platform. Used by the daemon's `ChannelFactory`
/// when a `FrontendType::Telegram` connection arrives. The stub's
/// `stream_event` and `finalize` are no-ops — the `IpcChannelBridge`
/// handles forwarding those over IPC.
pub struct TelegramDaemonChannel {
    session: SessionId,
    /// Rotated per turn by [`reset_cancellation`] and fired by
    /// [`cancel_inflight`]. Wrapped in `Mutex` so the daemon can
    /// install a fresh token between turns (C1+H1 fix from the
    /// Agent Loop audit — `CancellationToken` is monotonic, so a
    /// single timeout would otherwise brick every subsequent
    /// turn in this session).
    token: Mutex<CancellationToken>,
}

impl TelegramDaemonChannel {
    pub fn new() -> Self {
        TelegramDaemonChannel {
            session: SessionId::new(),
            token: Mutex::new(CancellationToken::new()),
        }
    }
}

impl Default for TelegramDaemonChannel {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ChannelContext for TelegramDaemonChannel {
    fn channel_name(&self) -> &str {
        "aivyx-telegram-daemon"
    }

    fn platform(&self) -> ChannelPlatform {
        ChannelPlatform::Telegram
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
// Multi-chat pump
// ---------------------------------------------------------------------------

struct ChatRoute {
    sender: tokio::sync::mpsc::Sender<IncomingMessage>,
    handle: tokio::task::JoinHandle<Result<(), DaemonError>>,
}

/// Drive a multi-chat Telegram frontend over the daemon IPC channel.
///
/// Streamed `StreamEventPayload` events are accumulated per turn and
/// sent as a single Telegram message at `TurnComplete` (Q3→(b)).
/// `/cancel` is forwarded as `CancelTurn` over IPC.
pub async fn run_telegram_daemon_multi_session(
    transport: Arc<ReqwestTransport>,
    chat_filter: Option<i64>,
    socket_path: PathBuf,
    role: Option<String>,
    shutdown: CancellationToken,
) -> Result<(), DaemonError> {
    let mut routes: HashMap<i64, ChatRoute> = HashMap::new();
    let mut offset: i64 = 0;

    loop {
        if shutdown.is_cancelled() {
            break;
        }

        let updates = tokio::select! {
            biased;
            _ = shutdown.cancelled() => break,
            res = transport.get_updates(offset, LONG_POLL_TIMEOUT_SECS) => match res {
                Ok(batch) => batch,
                Err(e) => {
                    eprintln!("aivyx-telegram(daemon): get_updates failed ({e}); backing off 1s");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
            }
        };

        if updates.is_empty() {
            continue;
        }

        for msg in updates {
            offset = offset.max(msg.update_id + 1);

            if let Some(allowed) = chat_filter {
                if msg.chat_id != allowed {
                    continue;
                }
            }

            let chat_id = msg.chat_id;

            let route = routes.entry(chat_id).or_insert_with(|| {
                let (tx, rx) = tokio::sync::mpsc::channel(32);
                let transport_clone = Arc::clone(&transport);
                let sp = socket_path.clone();
                let role_clone = role.clone();
                let shutdown_clone = shutdown.clone();
                let handle = tokio::spawn(async move {
                    run_telegram_daemon_chat_task(
                        transport_clone,
                        chat_id,
                        sp,
                        role_clone,
                        rx,
                        shutdown_clone,
                    )
                    .await
                });
                ChatRoute { sender: tx, handle }
            });

            if let Err(e) = route.sender.send(msg).await {
                eprintln!(
                    "aivyx-telegram(daemon): chat {chat_id} mailbox send failed ({e}); dropping route"
                );
                routes.remove(&chat_id);
            }
        }
    }

    let drained: Vec<(i64, ChatRoute)> = routes.drain().collect();
    for (_chat_id, ChatRoute { sender, handle }) in drained {
        drop(sender);
        match handle.await {
            Ok(Err(e)) => eprintln!("aivyx-telegram(daemon): inner task error: {e}"),
            Err(e) => eprintln!("aivyx-telegram(daemon): inner task join failed: {e}"),
            Ok(Ok(())) => {}
        }
    }

    Ok(())
}

/// Per-chat inner task: connect a `DaemonSession`, submit turns,
/// accumulate streamed events, send one Telegram message per turn.
async fn run_telegram_daemon_chat_task(
    transport: Arc<ReqwestTransport>,
    chat_id: i64,
    socket_path: PathBuf,
    role: Option<String>,
    mut mailbox: tokio::sync::mpsc::Receiver<IncomingMessage>,
    shutdown: CancellationToken,
) -> Result<(), DaemonError> {
    let mut session = DaemonSession::connect(
        &socket_path,
        role,
        Some(FrontendType::Telegram),
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
                .send_message(OutgoingMessage { chat_id, text: reply })
                .await
                .map_err(|e| DaemonError::Internal(format!(
                    "send_message to chat {chat_id}: {e}"
                )))?;
            continue;
        }

        // Phase 45 — forward image data through IPC when present.
        let (events, _outcome) = if let Some(ref img) = msg.image {
            use base64::Engine;
            let encoder = base64::engine::general_purpose::STANDARD;
            let att = crate::daemon_ipc::IpcAttachment {
                media_type: img.media_type.clone(),
                data_base64: encoder.encode(&img.data),
                filename: None,
            };
            session
                .submit_input_with_attachments(msg.text, vec![att])
                .await?
        } else {
            session.submit_input(msg.text).await?
        };

        let buf = render_events_for_telegram(&events);

        transport
            .send_message(OutgoingMessage {
                chat_id,
                text: buf,
            })
            .await
            .map_err(|e| DaemonError::Internal(format!(
                "send_message to chat {chat_id}: {e}"
            )))?;
    }

    let _ = session.disconnect().await;
    Ok(())
}

/// Render accumulated `StreamEventPayload` events into a single
/// Telegram message. Matches `TelegramChannel`'s `append_event`
/// rendering style (tool arrows, status markers) rather than the
/// CLI's `render_for_cli`.
fn render_events_for_telegram(events: &[StreamEventPayload]) -> String {
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

// `parse_gate_command` lived here through Phases 19–110. Phase 111
// extracted it into `crate::gate_command` so the Discord and Slack
// daemon-frontends could share the parser. Three-data-point
// reuse — see Phase 111 PHASE_111.md Q-block, Q2a sign-off.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon_ipc::StreamEventPayload;

    #[test]
    fn approval_gate_renders_with_reply_hint() {
        let events = vec![StreamEventPayload::ApprovalGate {
            mission_id: "m-001".into(),
            gate_id: "g-abc".into(),
            reason: "deploy?".into(),
            scope: None,
        }];
        let rendered = render_events_for_telegram(&events);
        assert!(rendered.contains("APPROVAL GATE"));
        assert!(rendered.contains("/approve m-001 g-abc"));
        assert!(rendered.contains("/reject  m-001 g-abc"));
    }

    // Audit C1+H1 regression — see DESIGN.md D1 turn-loop
    // contract and the Agent Loop audit notes. The stub
    // must (a) fire its token on `cancel_inflight` so the
    // daemon's CancelTurn handler actually cancels the
    // in-flight turn, and (b) rotate its token on
    // `reset_cancellation` so a prior cancel does not
    // pre-cancel turn N+1.

    #[test]
    fn cancel_inflight_cancels_the_current_token() {
        let ch = TelegramDaemonChannel::new();
        let token = ch.cancellation_token();
        assert!(!token.is_cancelled(), "fresh token must not be cancelled");
        ch.cancel_inflight();
        assert!(
            token.is_cancelled(),
            "after cancel_inflight, the snapshot token must report cancelled"
        );
    }

    #[test]
    fn reset_cancellation_installs_a_fresh_token() {
        let ch = TelegramDaemonChannel::new();
        ch.cancel_inflight();
        let stale = ch.cancellation_token();
        assert!(stale.is_cancelled(), "post-cancel token is cancelled");
        ch.reset_cancellation();
        let fresh = ch.cancellation_token();
        assert!(
            !fresh.is_cancelled(),
            "after reset_cancellation the new token must be un-cancelled"
        );
        // The old snapshot stays cancelled (monotonic), confirming
        // that reset swapped in a separate token rather than
        // un-cancelling the existing one.
        assert!(
            stale.is_cancelled(),
            "the pre-reset token must remain cancelled"
        );
    }
}
