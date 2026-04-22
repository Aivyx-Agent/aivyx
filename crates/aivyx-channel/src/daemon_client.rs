//! Daemon client — Phase 17 Task 4.
//!
//! Connects to a running daemon over the Unix domain socket,
//! manages a session, and supports multi-turn interaction. Includes
//! auto-spawn logic: if no daemon is listening, spawns one via
//! `aivyx daemon run` and waits for the socket to appear.
//!
//! Phase 16 shipped the single-turn PoC (`run_poc_client`).
//! Phase 17 Task 2 added multi-turn on the server side.
//! Phase 17 Task 4 adds multi-turn on the client side plus
//! auto-spawn.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use std::sync::Arc;

use crate::daemon_ipc::{
    decode_frame, encode_frame, DaemonEnvelope, FrameError, FrontendMessage, FrontendType,
    IpcAttachment, StreamEventPayload,
};
use crate::daemon_server::DaemonError;

/// Result of a single PoC daemon turn (Phase 16 shape, kept for
/// backward compatibility with the existing e2e test).
#[derive(Debug)]
pub struct DaemonTurnResult {
    pub session_id: String,
    pub events: Vec<StreamEventPayload>,
    pub outcome: String,
    pub daemon_version: Option<String>,
}

/// A connected, session-aware daemon client that supports multi-turn
/// interaction. Created by [`DaemonSession::connect`].
pub struct DaemonSession {
    reader: tokio::net::unix::OwnedReadHalf,
    writer: Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>,
    buf: Vec<u8>,
    pub session_id: String,
    pub daemon_version: Option<String>,
}

impl DaemonSession {
    /// Connect to a running daemon, read `DaemonReady`, send
    /// `StartSession`, and return a session handle ready for
    /// `submit_input` calls.
    pub async fn connect(
        socket_path: &Path,
        role: Option<String>,
        frontend_type: Option<FrontendType>,
    ) -> Result<Self, DaemonError> {
        let stream = UnixStream::connect(socket_path).await?;
        let (mut reader, writer) = stream.into_split();
        let mut buf = Vec::with_capacity(4096);

        // Read DaemonReady.
        read_more(&mut reader, &mut buf).await?;
        let daemon_version: Option<String> =
            match decode_frame::<DaemonEnvelope>(&buf) {
                Ok((DaemonEnvelope::DaemonReady { version }, consumed)) => {
                    buf.drain(..consumed);
                    Some(version)
                }
                Ok((other, _)) => {
                    return Err(DaemonError::Protocol(format!(
                        "expected DaemonReady, got {other:?}"
                    )));
                }
                Err(e) => return Err(e.into()),
            };

        // Send StartSession.
        let start = FrontendMessage::StartSession { role, frontend_type };
        let frame = encode_frame(&start)?;
        let writer = Arc::new(tokio::sync::Mutex::new(writer));
        {
            let mut w = writer.lock().await;
            w.write_all(&frame).await?;
        }

        // Read SessionStarted.
        let session_id: String = loop {
            match decode_frame::<DaemonEnvelope>(&buf) {
                Ok((
                    DaemonEnvelope::SessionStarted { session_id: sid },
                    consumed,
                )) => {
                    buf.drain(..consumed);
                    break sid;
                }
                Ok((DaemonEnvelope::Error { code, message }, _)) => {
                    return Err(DaemonError::Protocol(format!(
                        "daemon error ({code}): {message}"
                    )));
                }
                Err(FrameError::IncompleteBuf) => {
                    read_more(&mut reader, &mut buf).await?;
                }
                Ok((other, _)) => {
                    return Err(DaemonError::Protocol(format!(
                        "expected SessionStarted, got {other:?}"
                    )));
                }
                Err(e) => return Err(e.into()),
            }
        };

        Ok(DaemonSession {
            reader,
            writer,
            buf,
            session_id,
            daemon_version,
        })
    }

    pub async fn submit_input_for_mission(
        &mut self,
        text: String,
        mission_id: String,
    ) -> Result<(Vec<StreamEventPayload>, String), DaemonError> {
        let submit = FrontendMessage::SubmitInput {
            session_id: self.session_id.clone(),
            text,
            mission_id: Some(mission_id),
            attachments: vec![],
        };
        self.send_and_collect(submit).await
    }

    /// Submit a turn to the daemon and collect all streamed events
    /// until `TurnComplete`. Returns the events and the outcome string.
    pub async fn submit_input(
        &mut self,
        text: String,
    ) -> Result<(Vec<StreamEventPayload>, String), DaemonError> {
        let submit = FrontendMessage::SubmitInput {
            session_id: self.session_id.clone(),
            text,
            mission_id: None,
            attachments: vec![],
        };
        self.send_and_collect(submit).await
    }

    /// Submit a turn with image attachments. Phase 45 multimodal path.
    pub async fn submit_input_with_attachments(
        &mut self,
        text: String,
        attachments: Vec<IpcAttachment>,
    ) -> Result<(Vec<StreamEventPayload>, String), DaemonError> {
        let submit = FrontendMessage::SubmitInput {
            session_id: self.session_id.clone(),
            text,
            mission_id: None,
            attachments,
        };
        self.send_and_collect(submit).await
    }

    async fn send_and_collect(
        &mut self,
        msg: FrontendMessage,
    ) -> Result<(Vec<StreamEventPayload>, String), DaemonError> {
        let frame = encode_frame(&msg)?;
        {
            let mut w = self.writer.lock().await;
            w.write_all(&frame).await?;
        }

        let mut events = Vec::new();
        loop {
            match decode_frame::<DaemonEnvelope>(&self.buf) {
                Ok((DaemonEnvelope::StreamEvent { event, .. }, consumed)) => {
                    self.buf.drain(..consumed);
                    events.push(event);
                }
                Ok((DaemonEnvelope::TurnComplete { outcome, .. }, consumed)) => {
                    self.buf.drain(..consumed);
                    return Ok((events, outcome));
                }
                Ok((DaemonEnvelope::Error { code, message }, _)) => {
                    return Err(DaemonError::Protocol(format!(
                        "daemon error ({code}): {message}"
                    )));
                }
                Ok((DaemonEnvelope::ShuttingDown { reason }, _)) => {
                    return Err(DaemonError::Protocol(format!(
                        "daemon shutting down: {reason}"
                    )));
                }
                Err(FrameError::IncompleteBuf) => {
                    read_more(&mut self.reader, &mut self.buf).await?;
                }
                Ok((other, consumed)) => {
                    self.buf.drain(..consumed);
                    return Err(DaemonError::Protocol(format!(
                        "unexpected message during turn: {other:?}"
                    )));
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Send `CancelTurn` to request cancellation of the in-flight turn.
    pub async fn cancel_turn(&mut self) -> Result<(), DaemonError> {
        let cancel = FrontendMessage::CancelTurn {
            session_id: self.session_id.clone(),
        };
        let frame = encode_frame(&cancel)?;
        let mut w = self.writer.lock().await;
        w.write_all(&frame).await?;
        Ok(())
    }

    /// Send `ResolveGate` and wait for `GateResolved` (or `Error`).
    pub async fn resolve_gate(
        &mut self,
        mission_id: String,
        gate_id: String,
        approved: bool,
    ) -> Result<(), DaemonError> {
        let msg = FrontendMessage::ResolveGate {
            mission_id,
            gate_id,
            approved,
        };
        let frame = encode_frame(&msg)?;
        {
            let mut w = self.writer.lock().await;
            w.write_all(&frame).await?;
        }

        loop {
            match decode_frame::<DaemonEnvelope>(&self.buf) {
                Ok((DaemonEnvelope::GateResolved { .. }, consumed)) => {
                    self.buf.drain(..consumed);
                    return Ok(());
                }
                Ok((DaemonEnvelope::Error { code, message }, _)) => {
                    return Err(DaemonError::Protocol(format!(
                        "gate resolve error ({code}): {message}"
                    )));
                }
                Ok((DaemonEnvelope::ShuttingDown { reason }, _)) => {
                    return Err(DaemonError::Protocol(format!(
                        "daemon shutting down: {reason}"
                    )));
                }
                Err(FrameError::IncompleteBuf) => {
                    read_more(&mut self.reader, &mut self.buf).await?;
                }
                Ok((other, consumed)) => {
                    self.buf.drain(..consumed);
                    return Err(DaemonError::Protocol(format!(
                        "unexpected message during ResolveGate: {other:?}"
                    )));
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Return a cloneable cancel handle for use from a signal handler.
    pub fn cancel_handle(&self) -> DaemonCancelHandle {
        DaemonCancelHandle {
            writer: Arc::clone(&self.writer),
            session_id: self.session_id.clone(),
        }
    }

    /// Send `Disconnect` and drop the connection cleanly.
    pub async fn disconnect(self) -> Result<(), DaemonError> {
        let frame = encode_frame(&FrontendMessage::Disconnect)?;
        let mut w = self.writer.lock().await;
        w.write_all(&frame).await?;
        Ok(())
    }
}

/// A cloneable handle for sending `CancelTurn` from a signal handler
/// without holding `&mut DaemonSession`. Created by
/// [`DaemonSession::cancel_handle`].
#[derive(Clone)]
pub struct DaemonCancelHandle {
    writer: Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>,
    session_id: String,
}

impl DaemonCancelHandle {
    /// Send `CancelTurn` to the daemon. Safe to call from any task.
    pub async fn cancel(&self) {
        let cancel = FrontendMessage::CancelTurn {
            session_id: self.session_id.clone(),
        };
        if let Ok(frame) = encode_frame(&cancel) {
            let mut w = self.writer.lock().await;
            let _ = w.write_all(&frame).await;
        }
    }
}

/// Check whether a daemon is listening at the given socket path.
/// Returns `true` if a connection succeeds, `false` otherwise.
pub async fn daemon_is_running(socket_path: &Path) -> bool {
    UnixStream::connect(socket_path).await.is_ok()
}

/// Result of a `daemon status` probe.
#[derive(Debug)]
pub struct DaemonStatusInfo {
    pub running: bool,
    pub version: Option<String>,
    pub pid: Option<u32>,
}

/// Read the PID from a daemon PID file, if it exists and contains a
/// valid u32. Returns `None` if the file is missing, empty, or
/// contains non-numeric data.
pub fn read_pid_file(pid_path: &Path) -> Option<u32> {
    std::fs::read_to_string(pid_path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

/// Probe a running daemon: connect, read `DaemonReady`, disconnect.
/// Returns status info without starting a session. The PID is read
/// from the sibling `.pid` file if it exists.
pub async fn daemon_status(socket_path: &Path) -> DaemonStatusInfo {
    let pid_path = socket_path.with_extension("pid");
    let pid = read_pid_file(&pid_path);

    let stream = match UnixStream::connect(socket_path).await {
        Ok(s) => s,
        Err(_) => return DaemonStatusInfo { running: false, version: None, pid },
    };
    let (mut reader, _writer) = stream.into_split();
    let mut buf = Vec::with_capacity(4096);
    if read_more(&mut reader, &mut buf).await.is_err() {
        return DaemonStatusInfo { running: true, version: None, pid };
    }
    match decode_frame::<DaemonEnvelope>(&buf) {
        Ok((DaemonEnvelope::DaemonReady { version }, _)) => {
            DaemonStatusInfo { running: true, version: Some(version), pid }
        }
        _ => DaemonStatusInfo { running: true, version: None, pid },
    }
}

/// Send `Shutdown` to a running daemon and wait for the `ShuttingDown`
/// lifecycle event. Returns the shutdown reason on success.
pub async fn daemon_stop(socket_path: &Path) -> Result<String, DaemonError> {
    let stream = UnixStream::connect(socket_path).await?;
    let (mut reader, mut writer) = stream.into_split();
    let mut buf = Vec::with_capacity(4096);

    // Read DaemonReady.
    read_more(&mut reader, &mut buf).await?;
    match decode_frame::<DaemonEnvelope>(&buf) {
        Ok((DaemonEnvelope::DaemonReady { .. }, consumed)) => {
            buf.drain(..consumed);
        }
        Ok((other, _)) => {
            return Err(DaemonError::Protocol(format!(
                "expected DaemonReady, got {other:?}"
            )));
        }
        Err(e) => return Err(e.into()),
    }

    // Send Shutdown.
    let frame = encode_frame(&FrontendMessage::Shutdown)?;
    writer.write_all(&frame).await?;

    // Wait for ShuttingDown.
    loop {
        match decode_frame::<DaemonEnvelope>(&buf) {
            Ok((DaemonEnvelope::ShuttingDown { reason }, _)) => {
                return Ok(reason);
            }
            Ok((other, consumed)) => {
                buf.drain(..consumed);
                return Err(DaemonError::Protocol(format!(
                    "expected ShuttingDown, got {other:?}"
                )));
            }
            Err(FrameError::IncompleteBuf) => {
                read_more(&mut reader, &mut buf).await?;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// Spawn a daemon process in the background and wait for its socket
/// to appear. Returns the socket path on success.
///
/// Uses `tokio::process::Command` to launch `aivyx daemon run` as a
/// detached child with stdout/stderr inherited (so the daemon's
/// startup banner appears on the operator's terminal).
pub async fn spawn_daemon_and_wait(
    socket_path: &Path,
    timeout: Duration,
) -> Result<PathBuf, DaemonError> {
    let exe = std::env::current_exe()?;

    let _child = tokio::process::Command::new(&exe)
        .args(["daemon", "run"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .spawn()
        .map_err(|e| DaemonError::Internal(format!("failed to spawn daemon: {e}")))?;

    // Poll for the socket to appear with exponential backoff.
    let start = tokio::time::Instant::now();
    let mut delay = Duration::from_millis(20);
    loop {
        if daemon_is_running(socket_path).await {
            return Ok(socket_path.to_path_buf());
        }
        if start.elapsed() > timeout {
            return Err(DaemonError::Internal(format!(
                "daemon did not start within {}ms — socket not found at {}",
                timeout.as_millis(),
                socket_path.display(),
            )));
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_millis(500));
    }
}

// -----------------------------------------------------------------------
// Phase 16 backward-compatible single-turn client
// -----------------------------------------------------------------------

/// Connect to the daemon, start a session, submit one input, and
/// collect all streamed events until `TurnComplete`.
pub async fn run_poc_client(
    socket_path: &Path,
    role: Option<String>,
    input_text: String,
) -> Result<DaemonTurnResult, DaemonError> {
    let mut session = DaemonSession::connect(socket_path, role, None).await?;
    let daemon_version = session.daemon_version.clone();
    let session_id = session.session_id.clone();

    let (events, outcome) = session.submit_input(input_text).await?;
    let _ = session.disconnect().await;

    Ok(DaemonTurnResult {
        session_id,
        events,
        outcome,
        daemon_version,
    })
}

async fn read_more(
    reader: &mut tokio::net::unix::OwnedReadHalf,
    buf: &mut Vec<u8>,
) -> Result<(), DaemonError> {
    let mut tmp = [0u8; 4096];
    let n = reader.read(&mut tmp).await?;
    if n == 0 {
        return Err(DaemonError::Protocol(
            "connection closed unexpectedly".into(),
        ));
    }
    buf.extend_from_slice(&tmp[..n]);
    Ok(())
}
