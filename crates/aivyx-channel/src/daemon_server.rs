//! Minimal PoC daemon server — Phase 16 Task 3.
//!
//! Listens on a Unix domain socket, accepts one connection, reads
//! IPC frames, dispatches one turn through the provided agent, and
//! streams `DaemonMessage` frames back. This is the smallest shape
//! that proves the Phase 16 IPC protocol carries a turn end-to-end.
//!
//! **Not production-ready.** Single-connection, single-turn, no
//! crash recovery, no graceful shutdown, no auto-spawn. All of those
//! are Phase 17+ concerns per `docs/PHASE_16.md` non-goals.

use std::path::Path;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

use aivyx_core::{Agent, ChannelContext, Message, StreamEvent, TurnOutcome};

use crate::daemon_ipc::{
    decode_frame, encode_frame, DaemonLifecycleEvent, DaemonMessage, FrameError, FrontendMessage,
    StreamEventPayload,
};

/// Run a single-connection PoC daemon server.
///
/// Binds the Unix socket at `socket_path`, sends `DaemonReady` on
/// connect, processes one `StartSession` + one `SubmitInput`, runs
/// the turn through `agent`, streams events back, sends
/// `TurnComplete`, and returns. The caller is responsible for
/// cleaning up the socket file.
pub async fn run_poc_daemon<C: ChannelContext>(
    socket_path: &Path,
    agent: Arc<dyn Agent>,
    channel: Arc<C>,
) -> Result<(), String> {
    // Remove stale socket if present.
    let _ = std::fs::remove_file(socket_path);

    // Create parent directory if needed.
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create socket parent dir: {e}"))?;
    }

    let listener = UnixListener::bind(socket_path)
        .map_err(|e| format!("failed to bind daemon socket at {}: {e}", socket_path.display()))?;

    // Set socket permissions to 0600 per P4.4.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(socket_path, perms)
            .map_err(|e| format!("failed to set socket permissions: {e}"))?;
    }

    let (stream, _addr) = listener
        .accept()
        .await
        .map_err(|e| format!("failed to accept connection: {e}"))?;
    let (mut reader, mut writer) = stream.into_split();

    // Send DaemonReady lifecycle event.
    let ready = DaemonLifecycleEvent::DaemonReady {
        version: "0.1".into(),
    };
    let frame = encode_frame(&ready).map_err(|e| format!("encode DaemonReady: {e}"))?;
    writer
        .write_all(&frame)
        .await
        .map_err(|e| format!("write DaemonReady: {e}"))?;

    // Read frames in a loop.
    let mut buf = Vec::with_capacity(4096);
    let mut _session_id: Option<String> = None;

    loop {
        // Read more data.
        let mut tmp = [0u8; 4096];
        let n = reader
            .read(&mut tmp)
            .await
            .map_err(|e| format!("read error: {e}"))?;
        if n == 0 {
            break; // Connection closed.
        }
        buf.extend_from_slice(&tmp[..n]);

        // Try to decode frames from the buffer.
        loop {
            match decode_frame::<FrontendMessage>(&buf) {
                Ok((msg, consumed)) => {
                    buf.drain(..consumed);
                    match msg {
                        FrontendMessage::StartSession { role: _ } => {
                            let sid = aivyx_core::SessionId::new().to_string();
                            _session_id = Some(sid.clone());
                            let resp = DaemonMessage::SessionStarted {
                                session_id: sid,
                            };
                            let frame = encode_frame(&resp)
                                .map_err(|e| format!("encode SessionStarted: {e}"))?;
                            writer
                                .write_all(&frame)
                                .await
                                .map_err(|e| format!("write SessionStarted: {e}"))?;
                        }
                        FrontendMessage::SubmitInput { session_id: sid, text } => {
                            let msg = Message::text(
                                aivyx_core::SessionId::new(),
                                text,
                            );

                            // Run the turn. We use an IpcChannelBridge
                            // that forwards StreamEvents over the socket.
                            let bridge = IpcChannelBridge {
                                inner: Arc::clone(&channel),
                                writer: Arc::new(tokio::sync::Mutex::new(writer)),
                                session_id: sid.clone(),
                            };

                            let outcome = agent.turn(msg, &bridge).await;

                            // Reclaim the writer from the bridge.
                            writer = Arc::try_unwrap(bridge.writer)
                                .map_err(|_| "writer arc still shared".to_string())?
                                .into_inner();

                            let outcome_str = match &outcome {
                                TurnOutcome::Completed { final_message, .. } => {
                                    format!("completed: {final_message}")
                                }
                                TurnOutcome::Failed(e) => format!("failed: {e}"),
                                TurnOutcome::Cancelled { .. } => "cancelled".into(),
                                TurnOutcome::TimedOut { .. } => "timed out".into(),
                                TurnOutcome::Escalated { reason, .. } => {
                                    format!("escalated: {reason}")
                                }
                            };

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

                            // PoC: exit after one turn.
                            return Ok(());
                        }
                        FrontendMessage::Disconnect => {
                            return Ok(());
                        }
                        FrontendMessage::CancelTurn { .. } => {
                            // PoC: ignore cancel.
                        }
                    }
                }
                Err(FrameError::IncompleteBuf) => break, // Need more data.
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

/// A `ChannelContext` bridge that forwards `StreamEvent`s over IPC.
///
/// Per Q2 resolution (a): `ChannelContext` is unchanged. The daemon
/// constructs this bridge that implements the existing trait by
/// serializing each event into a `DaemonMessage::StreamEvent` frame
/// and writing it to the IPC socket. The turn loop does not know it's
/// talking to a remote frontend.
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
        // The daemon sends TurnComplete separately; finalize is a no-op
        // on the IPC bridge. The inner channel's finalize is not called
        // because the frontend handles rendering.
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
        StreamEvent::Attachment { .. } => {
            // Phase 16 PoC: attachments are not supported over IPC.
            StreamEventPayload::Status {
                status: "[attachment not supported over IPC]".to_string(),
            }
        }
    }
}
