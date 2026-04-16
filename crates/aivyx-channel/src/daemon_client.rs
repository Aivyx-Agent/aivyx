//! Minimal PoC daemon client — Phase 16 Task 3.
//!
//! Connects to a running daemon over the Unix domain socket, sends
//! `StartSession` + `SubmitInput`, reads streamed `DaemonMessage`
//! frames, and returns the collected events. This is the frontend
//! side of the Phase 16 IPC proof-of-concept.
//!
//! **Not production-ready.** No reconnection, no multi-turn, no
//! auto-spawn. All of those are Phase 17+ concerns.

use std::path::Path;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::daemon_ipc::{
    decode_frame, encode_frame, DaemonEnvelope, FrameError, FrontendMessage, StreamEventPayload,
};

/// Result of a single PoC daemon turn.
#[derive(Debug)]
pub struct DaemonTurnResult {
    pub session_id: String,
    pub events: Vec<StreamEventPayload>,
    pub outcome: String,
    pub daemon_version: Option<String>,
}

/// Connect to the daemon, start a session, submit one input, and
/// collect all streamed events until `TurnComplete`.
pub async fn run_poc_client(
    socket_path: &Path,
    role: Option<String>,
    input_text: String,
) -> Result<DaemonTurnResult, String> {
    let stream = UnixStream::connect(socket_path)
        .await
        .map_err(|e| format!("failed to connect to daemon at {}: {e}", socket_path.display()))?;
    let (mut reader, mut writer) = stream.into_split();

    let mut buf = Vec::with_capacity(4096);
    let mut events: Vec<StreamEventPayload> = Vec::new();

    // Read DaemonReady.
    read_more(&mut reader, &mut buf).await?;
    let daemon_version: Option<String> = match decode_frame::<DaemonEnvelope>(&buf) {
        Ok((DaemonEnvelope::DaemonReady { version }, consumed)) => {
            buf.drain(..consumed);
            Some(version)
        }
        Ok((other, _)) => {
            return Err(format!("expected DaemonReady, got {other:?}"));
        }
        Err(e) => {
            return Err(format!("failed to read DaemonReady: {e}"));
        }
    };

    // Send StartSession.
    let start = FrontendMessage::StartSession { role };
    let frame = encode_frame(&start).map_err(|e| format!("encode StartSession: {e}"))?;
    writer
        .write_all(&frame)
        .await
        .map_err(|e| format!("write StartSession: {e}"))?;

    // Read SessionStarted.
    let sid: String = loop {
        match decode_frame::<DaemonEnvelope>(&buf) {
            Ok((DaemonEnvelope::SessionStarted { session_id: sid }, consumed)) => {
                buf.drain(..consumed);
                break sid;
            }
            Ok((DaemonEnvelope::Error { code, message }, _)) => {
                return Err(format!("daemon error ({code}): {message}"));
            }
            Err(FrameError::IncompleteBuf) => {
                read_more(&mut reader, &mut buf).await?;
            }
            Ok((other, _)) => {
                return Err(format!("expected SessionStarted, got {other:?}"));
            }
            Err(e) => {
                return Err(format!("failed to read SessionStarted: {e}"));
            }
        }
    };

    // Send SubmitInput.
    let submit = FrontendMessage::SubmitInput {
        session_id: sid.clone(),
        text: input_text,
    };
    let frame = encode_frame(&submit).map_err(|e| format!("encode SubmitInput: {e}"))?;
    writer
        .write_all(&frame)
        .await
        .map_err(|e| format!("write SubmitInput: {e}"))?;

    // Read streaming events until TurnComplete.
    loop {
        match decode_frame::<DaemonEnvelope>(&buf) {
            Ok((DaemonEnvelope::StreamEvent { event, .. }, consumed)) => {
                buf.drain(..consumed);
                events.push(event);
            }
            Ok((DaemonEnvelope::TurnComplete { outcome, .. }, consumed)) => {
                buf.drain(..consumed);
                return Ok(DaemonTurnResult {
                    session_id: sid,
                    events,
                    outcome,
                    daemon_version,
                });
            }
            Ok((DaemonEnvelope::Error { code, message }, _)) => {
                return Err(format!("daemon error ({code}): {message}"));
            }
            Err(FrameError::IncompleteBuf) => {
                read_more(&mut reader, &mut buf).await?;
            }
            Ok((_other, consumed)) => {
                buf.drain(..consumed);
            }
            Err(e) => {
                return Err(format!("frame decode error: {e}"));
            }
        }
    }
}

async fn read_more(
    reader: &mut tokio::net::unix::OwnedReadHalf,
    buf: &mut Vec<u8>,
) -> Result<(), String> {
    let mut tmp = [0u8; 4096];
    let n = reader
        .read(&mut tmp)
        .await
        .map_err(|e| format!("read error: {e}"))?;
    if n == 0 {
        return Err("connection closed unexpectedly".to_string());
    }
    buf.extend_from_slice(&tmp[..n]);
    Ok(())
}
