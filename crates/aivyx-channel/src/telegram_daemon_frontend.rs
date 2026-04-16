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
use std::sync::Arc;

use aivyx_core::{
    CancellationToken, ChannelContext, ChannelError, ChannelPlatform, SessionId, StreamEvent,
    TurnOutcome,
};
use aivyx_telegram::transport::{IncomingMessage, OutgoingMessage, ReqwestTransport, TelegramTransport};

use crate::daemon_client::DaemonSession;
use crate::daemon_ipc::{FrontendType, StreamEventPayload};

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
    token: CancellationToken,
}

impl TelegramDaemonChannel {
    pub fn new() -> Self {
        TelegramDaemonChannel {
            session: SessionId::new(),
            token: CancellationToken::new(),
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
        self.token.clone()
    }
}

// ---------------------------------------------------------------------------
// Multi-chat pump
// ---------------------------------------------------------------------------

struct ChatRoute {
    sender: tokio::sync::mpsc::Sender<IncomingMessage>,
    handle: tokio::task::JoinHandle<Result<(), String>>,
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
) -> Result<(), String> {
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
) -> Result<(), String> {
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

        if let Some(gate_cmd) = parse_gate_command(msg.text.trim()) {
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
                .map_err(|e| format!("send_message to chat {chat_id}: {e}"))?;
            continue;
        }

        let (events, _outcome) = session.submit_input(msg.text).await?;

        let buf = render_events_for_telegram(&events);

        transport
            .send_message(OutgoingMessage {
                chat_id,
                text: buf,
            })
            .await
            .map_err(|e| format!("send_message to chat {chat_id}: {e}"))?;
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

struct GateCommand {
    mission_id: String,
    gate_id: String,
    approved: bool,
}

fn parse_gate_command(text: &str) -> Option<GateCommand> {
    let parts: Vec<&str> = text.split_whitespace().collect();
    if parts.len() != 3 {
        return None;
    }
    let approved = match parts[0] {
        "/approve" => true,
        "/reject" => false,
        _ => return None,
    };
    Some(GateCommand {
        mission_id: parts[1].to_string(),
        gate_id: parts[2].to_string(),
        approved,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon_ipc::StreamEventPayload;

    #[test]
    fn parse_approve_command() {
        let cmd = parse_gate_command("/approve m-001 g-abc").unwrap();
        assert!(cmd.approved);
        assert_eq!(cmd.mission_id, "m-001");
        assert_eq!(cmd.gate_id, "g-abc");
    }

    #[test]
    fn parse_reject_command() {
        let cmd = parse_gate_command("/reject m-002 g-xyz").unwrap();
        assert!(!cmd.approved);
        assert_eq!(cmd.mission_id, "m-002");
        assert_eq!(cmd.gate_id, "g-xyz");
    }

    #[test]
    fn parse_unknown_command_returns_none() {
        assert!(parse_gate_command("/cancel").is_none());
        assert!(parse_gate_command("/approve m-001").is_none());
        assert!(parse_gate_command("hello world").is_none());
    }

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
}
