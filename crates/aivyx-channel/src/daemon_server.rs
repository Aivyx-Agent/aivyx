//! Production daemon server — Phase 17 Task 2.
//!
//! Listens on a Unix domain socket, accepts connections, reads IPC
//! frames, dispatches turns through the provided agent, and streams
//! `DaemonMessage` frames back. Supports multi-turn sessions (the
//! connection stays open across turns) and graceful shutdown via a
//! `CancellationToken`.
//!
//! Phase 16 shipped the single-turn PoC; Phase 17 Task 2 extends it
//! to multi-turn with graceful shutdown. Auto-spawn, CLI integration,
//! and multi-connection are Phase 17 Task 3 concerns.

use std::path::Path;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

use aivyx_core::{Agent, CancellationToken, ChannelContext, Message, StreamEvent, TurnOutcome};

use crate::daemon_ipc::{
    decode_frame, encode_frame, DaemonLifecycleEvent, DaemonMessage, FrameError, FrontendMessage,
    StreamEventPayload, PROTOCOL_VERSION,
};

/// Run the daemon server.
///
/// Binds the Unix socket at `socket_path`, accepts one connection,
/// and serves turns in a loop until the frontend disconnects or
/// `shutdown` is cancelled. Sends `DaemonReady` on connect,
/// `ShuttingDown` on graceful shutdown.
///
/// The `shutdown` token allows external code (signal handlers, tests)
/// to trigger a graceful shutdown. When cancelled, the daemon finishes
/// any in-flight turn, sends `ShuttingDown`, and returns.
pub async fn run_daemon<C: ChannelContext>(
    socket_path: &Path,
    agent: Arc<dyn Agent>,
    channel: Arc<C>,
    shutdown: CancellationToken,
) -> Result<(), String> {
    let _ = std::fs::remove_file(socket_path);

    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create socket parent dir: {e}"))?;
    }

    let listener = UnixListener::bind(socket_path)
        .map_err(|e| format!("failed to bind daemon socket at {}: {e}", socket_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(socket_path, perms)
            .map_err(|e| format!("failed to set socket permissions: {e}"))?;
    }

    // Accept one connection (multi-connection is Task 3 scope).
    let (stream, _addr) = tokio::select! {
        result = listener.accept() => {
            result.map_err(|e| format!("failed to accept connection: {e}"))?
        }
        _ = shutdown.cancelled() => {
            return Ok(());
        }
    };
    let (mut reader, mut writer) = stream.into_split();

    let ready = DaemonLifecycleEvent::DaemonReady {
        version: PROTOCOL_VERSION.into(),
    };
    let frame = encode_frame(&ready).map_err(|e| format!("encode DaemonReady: {e}"))?;
    writer
        .write_all(&frame)
        .await
        .map_err(|e| format!("write DaemonReady: {e}"))?;

    let mut buf = Vec::with_capacity(4096);
    let mut _session_id: Option<String> = None;

    loop {
        // Check shutdown between loop iterations.
        if shutdown.is_cancelled() {
            send_shutting_down(&mut writer, "shutdown requested").await;
            return Ok(());
        }

        let mut tmp = [0u8; 4096];
        let n = tokio::select! {
            result = reader.read(&mut tmp) => {
                result.map_err(|e| format!("read error: {e}"))?
            }
            _ = shutdown.cancelled() => {
                send_shutting_down(&mut writer, "shutdown requested").await;
                return Ok(());
            }
        };
        if n == 0 {
            break; // Frontend disconnected.
        }
        buf.extend_from_slice(&tmp[..n]);

        loop {
            match decode_frame::<FrontendMessage>(&buf) {
                Ok((msg, consumed)) => {
                    buf.drain(..consumed);
                    match msg {
                        FrontendMessage::StartSession { role: _ } => {
                            let sid = aivyx_core::SessionId::new().to_string();
                            _session_id = Some(sid.clone());
                            let resp = DaemonMessage::SessionStarted { session_id: sid };
                            let frame = encode_frame(&resp)
                                .map_err(|e| format!("encode SessionStarted: {e}"))?;
                            writer
                                .write_all(&frame)
                                .await
                                .map_err(|e| format!("write SessionStarted: {e}"))?;
                        }
                        FrontendMessage::SubmitInput {
                            session_id: sid,
                            text,
                        } => {
                            let msg = Message::text(aivyx_core::SessionId::new(), text);

                            let bridge = IpcChannelBridge {
                                inner: Arc::clone(&channel),
                                writer: Arc::new(tokio::sync::Mutex::new(writer)),
                                session_id: sid.clone(),
                            };

                            let outcome = agent.turn(msg, &bridge).await;

                            writer = Arc::try_unwrap(bridge.writer)
                                .map_err(|_| "writer arc still shared".to_string())?
                                .into_inner();

                            let outcome_str = format_outcome(&outcome);

                            let resp = DaemonMessage::TurnComplete {
                                session_id: sid,
                                outcome: outcome_str,
                            };
                            let frame = encode_frame(&resp)
                                .map_err(|e| format!("encode TurnComplete: {e}"))?;
                            writer
                                .write_all(&frame)
                                .await
                                .map_err(|e| format!("write TurnComplete: {e}"))?;

                            // Multi-turn: continue the loop instead of returning.
                        }
                        FrontendMessage::Disconnect => {
                            return Ok(());
                        }
                        FrontendMessage::CancelTurn { session_id: _sid } => {
                            // Cancellation wired through CancellationToken on
                            // the channel bridge; the turn loop checks it between
                            // LLM steps. For now, cancellation is a no-op at the
                            // daemon dispatch level — the bridge's inner channel
                            // already exposes the token.
                        }
                    }
                }
                Err(FrameError::IncompleteBuf) => break,
                Err(e) => {
                    let err_resp = DaemonMessage::Error {
                        code: "invalid_message".into(),
                        message: e.to_string(),
                    };
                    let frame = encode_frame(&err_resp).unwrap_or_default();
                    let _ = writer.write_all(&frame).await;
                    return Err(format!("frame decode error: {e}"));
                }
            }
        }
    }

    Ok(())
}

/// Backward-compatible alias for Phase 16 tests.
pub async fn run_poc_daemon<C: ChannelContext>(
    socket_path: &Path,
    agent: Arc<dyn Agent>,
    channel: Arc<C>,
) -> Result<(), String> {
    let shutdown = CancellationToken::new();
    run_daemon(socket_path, agent, channel, shutdown).await
}

async fn send_shutting_down(writer: &mut tokio::net::unix::OwnedWriteHalf, reason: &str) {
    let event = DaemonLifecycleEvent::ShuttingDown {
        reason: reason.to_string(),
    };
    if let Ok(frame) = encode_frame(&event) {
        let _ = writer.write_all(&frame).await;
    }
}

fn format_outcome(outcome: &TurnOutcome) -> String {
    match outcome {
        TurnOutcome::Completed { final_message, .. } => {
            format!("completed: {final_message}")
        }
        TurnOutcome::Failed(e) => format!("failed: {e}"),
        TurnOutcome::Cancelled { .. } => "cancelled".into(),
        TurnOutcome::TimedOut { .. } => "timed out".into(),
        TurnOutcome::Escalated { reason, .. } => {
            format!("escalated: {reason}")
        }
    }
}

// ---------------------------------------------------------------------------
// IpcChannelBridge — forwards StreamEvents over IPC
// ---------------------------------------------------------------------------

struct IpcChannelBridge<C: ChannelContext> {
    inner: Arc<C>,
    writer: Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>,
    session_id: String,
}

#[async_trait::async_trait]
impl<C: ChannelContext> ChannelContext for IpcChannelBridge<C> {
    fn channel_name(&self) -> &str {
        self.inner.channel_name()
    }

    fn platform(&self) -> aivyx_core::ChannelPlatform {
        self.inner.platform()
    }

    fn trust_tier(&self) -> aivyx_capability::TrustTier {
        self.inner.trust_tier()
    }

    fn session_id(&self) -> aivyx_core::SessionId {
        self.inner.session_id()
    }

    async fn stream_event(&self, event: StreamEvent<'_>) -> Result<(), aivyx_core::ChannelError> {
        let payload = stream_event_to_payload(&event);
        let msg = DaemonMessage::StreamEvent {
            session_id: self.session_id.clone(),
            event: payload,
        };
        let frame = encode_frame(&msg)
            .map_err(|e| aivyx_core::ChannelError::Send(format!("encode StreamEvent: {e}")))?;
        let mut w = self.writer.lock().await;
        w.write_all(&frame)
            .await
            .map_err(|e| aivyx_core::ChannelError::Send(format!("write StreamEvent: {e}")))?;
        Ok(())
    }

    async fn finalize(&self, _outcome: &TurnOutcome) -> Result<(), aivyx_core::ChannelError> {
        Ok(())
    }

    fn cancellation_token(&self) -> aivyx_core::CancellationToken {
        self.inner.cancellation_token()
    }
}

fn stream_event_to_payload(event: &StreamEvent<'_>) -> StreamEventPayload {
    match event {
        StreamEvent::Text(text) => StreamEventPayload::Text {
            text: (*text).to_string(),
        },
        StreamEvent::Status(status) => StreamEventPayload::Status {
            status: (*status).to_string(),
        },
        StreamEvent::ToolCallStarted {
            tool,
            tool_name,
            input,
        } => StreamEventPayload::ToolCallStarted {
            tool_id: tool.to_string(),
            tool_name: (*tool_name).to_string(),
            input: (*input).clone(),
        },
        StreamEvent::ToolCallFinished {
            tool,
            tool_name,
            outcome_summary,
        } => StreamEventPayload::ToolCallFinished {
            tool_id: tool.to_string(),
            tool_name: (*tool_name).to_string(),
            outcome_summary: (*outcome_summary).to_string(),
        },
        StreamEvent::ToolOutput {
            tool,
            tool_name,
            chunk,
        } => StreamEventPayload::ToolOutput {
            tool_id: tool.to_string(),
            tool_name: (*tool_name).to_string(),
            chunk: (*chunk).to_string(),
        },
        StreamEvent::Attachment { .. } => StreamEventPayload::Status {
            status: "[attachment not supported over IPC]".to_string(),
        },
    }
}
