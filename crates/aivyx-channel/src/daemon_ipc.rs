//! Daemon IPC protocol types and framing.
//!
//! Implements the wire format specified in `docs/DAEMON_IPC.md`:
//! length-prefixed JSON frames over a Unix domain socket. Three
//! top-level message envelopes (`FrontendMessage`, `DaemonMessage`,
//! `DaemonLifecycleEvent`) are serde-serializable and round-trip
//! through the `encode_frame` / `decode_frame` helpers.
//!
//! Phase 16 Task 2 — this module is the parsing substrate the PoC
//! daemon (Task 3) builds on. It deliberately owns no I/O; the
//! async read/write loops live in the daemon and frontend dispatch
//! paths.

use serde::{Deserialize, Serialize};

/// 16 MiB — per `docs/DAEMON_IPC.md`. A frame whose length prefix
/// exceeds this is a protocol error.
pub const MAX_PAYLOAD_SIZE: u32 = 16 * 1024 * 1024;

/// Length of the frame header (4-byte big-endian payload length).
pub const FRAME_HEADER_LEN: usize = 4;

// ---------------------------------------------------------------------------
// Frontend → Daemon
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FrontendMessage {
    StartSession {
        role: Option<String>,
    },
    SubmitInput {
        session_id: String,
        text: String,
    },
    CancelTurn {
        session_id: String,
    },
    Disconnect,
}

// ---------------------------------------------------------------------------
// Daemon → Frontend (turn-loop traffic)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonMessage {
    SessionStarted {
        session_id: String,
    },
    StreamEvent {
        session_id: String,
        event: StreamEventPayload,
    },
    TurnComplete {
        session_id: String,
        outcome: String,
    },
    Error {
        code: String,
        message: String,
    },
}

// ---------------------------------------------------------------------------
// Daemon → Frontend (lifecycle, separate from DaemonMessage per Q4)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonLifecycleEvent {
    DaemonReady { version: String },
    ShuttingDown { reason: String },
}

// ---------------------------------------------------------------------------
// StreamEventPayload — owned, serializable mirror of core::StreamEvent<'a>
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum StreamEventPayload {
    Text {
        text: String,
    },
    Status {
        status: String,
    },
    ToolCallStarted {
        tool_id: String,
        tool_name: String,
        input: serde_json::Value,
    },
    ToolCallFinished {
        tool_id: String,
        tool_name: String,
        outcome_summary: String,
    },
    ToolOutput {
        tool_id: String,
        tool_name: String,
        chunk: String,
    },
}

// ---------------------------------------------------------------------------
// Framing: encode / decode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum FrameError {
    PayloadTooLarge(u32),
    IncompleteBuf,
    Utf8(String),
    Json(String),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::PayloadTooLarge(n) => {
                write!(f, "payload size {n} exceeds max {MAX_PAYLOAD_SIZE}")
            }
            FrameError::IncompleteBuf => write!(f, "buffer too short for a complete frame"),
            FrameError::Utf8(e) => write!(f, "payload is not valid UTF-8: {e}"),
            FrameError::Json(e) => write!(f, "JSON parse error: {e}"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Encode a serializable message into a length-prefixed frame.
pub fn encode_frame<T: Serialize>(msg: &T) -> Result<Vec<u8>, FrameError> {
    let json = serde_json::to_vec(msg).map_err(|e| FrameError::Json(e.to_string()))?;
    let len: u32 = json
        .len()
        .try_into()
        .map_err(|_| FrameError::PayloadTooLarge(u32::MAX))?;
    if len > MAX_PAYLOAD_SIZE {
        return Err(FrameError::PayloadTooLarge(len));
    }
    let mut buf = Vec::with_capacity(FRAME_HEADER_LEN + json.len());
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(&json);
    Ok(buf)
}

/// Try to decode one frame from the front of `buf`. On success returns
/// the deserialized message and the number of bytes consumed (header +
/// payload). Returns `Err(IncompleteBuf)` if `buf` does not yet contain
/// a full frame — the caller should read more bytes and retry.
pub fn decode_frame<T: for<'de> Deserialize<'de>>(buf: &[u8]) -> Result<(T, usize), FrameError> {
    if buf.len() < FRAME_HEADER_LEN {
        return Err(FrameError::IncompleteBuf);
    }
    let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
    if len > MAX_PAYLOAD_SIZE {
        return Err(FrameError::PayloadTooLarge(len));
    }
    let total = FRAME_HEADER_LEN + len as usize;
    if buf.len() < total {
        return Err(FrameError::IncompleteBuf);
    }
    let payload = &buf[FRAME_HEADER_LEN..total];
    let text = std::str::from_utf8(payload).map_err(|e| FrameError::Utf8(e.to_string()))?;
    let msg: T = serde_json::from_str(text).map_err(|e| FrameError::Json(e.to_string()))?;
    Ok((msg, total))
}

/// Convenience: decode a frame where the message type is one of the
/// three IPC envelopes. Wraps `decode_frame` with the union type.
///
/// The daemon's receive loop calls `decode_frame::<FrontendMessage>`.
/// The frontend's receive loop needs to demux `DaemonMessage` vs.
/// `DaemonLifecycleEvent` — this enum carries both.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonEnvelope {
    // DaemonMessage variants (flattened for serde tag dispatch)
    SessionStarted {
        session_id: String,
    },
    StreamEvent {
        session_id: String,
        event: StreamEventPayload,
    },
    TurnComplete {
        session_id: String,
        outcome: String,
    },
    Error {
        code: String,
        message: String,
    },
    // DaemonLifecycleEvent variants
    DaemonReady {
        version: String,
    },
    ShuttingDown {
        reason: String,
    },
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- FrontendMessage round-trip ----

    #[test]
    fn frontend_message_round_trips() {
        let cases = vec![
            FrontendMessage::StartSession {
                role: Some("coder".into()),
            },
            FrontendMessage::StartSession { role: None },
            FrontendMessage::SubmitInput {
                session_id: "abc-123".into(),
                text: "hello world".into(),
            },
            FrontendMessage::CancelTurn {
                session_id: "abc-123".into(),
            },
            FrontendMessage::Disconnect,
        ];
        for msg in cases {
            let frame = encode_frame(&msg).expect("encode");
            let (decoded, consumed): (FrontendMessage, _) =
                decode_frame(&frame).expect("decode");
            assert_eq!(decoded, msg);
            assert_eq!(consumed, frame.len());
        }
    }

    // ---- DaemonMessage round-trip ----

    #[test]
    fn daemon_message_round_trips() {
        let cases = vec![
            DaemonMessage::SessionStarted {
                session_id: "s1".into(),
            },
            DaemonMessage::StreamEvent {
                session_id: "s1".into(),
                event: StreamEventPayload::Text {
                    text: "hello".into(),
                },
            },
            DaemonMessage::StreamEvent {
                session_id: "s1".into(),
                event: StreamEventPayload::ToolCallStarted {
                    tool_id: "t1".into(),
                    tool_name: "fs.read".into(),
                    input: serde_json::json!({"path": "/tmp/test"}),
                },
            },
            DaemonMessage::TurnComplete {
                session_id: "s1".into(),
                outcome: "completed".into(),
            },
            DaemonMessage::Error {
                code: "internal".into(),
                message: "something broke".into(),
            },
        ];
        for msg in cases {
            let frame = encode_frame(&msg).expect("encode");
            let (decoded, consumed): (DaemonMessage, _) =
                decode_frame(&frame).expect("decode");
            assert_eq!(decoded, msg);
            assert_eq!(consumed, frame.len());
        }
    }

    // ---- DaemonLifecycleEvent round-trip ----

    #[test]
    fn daemon_lifecycle_event_round_trips() {
        let cases = vec![
            DaemonLifecycleEvent::DaemonReady {
                version: "0.1".into(),
            },
            DaemonLifecycleEvent::ShuttingDown {
                reason: "operator requested".into(),
            },
        ];
        for msg in cases {
            let frame = encode_frame(&msg).expect("encode");
            let (decoded, consumed): (DaemonLifecycleEvent, _) =
                decode_frame(&frame).expect("decode");
            assert_eq!(decoded, msg);
            assert_eq!(consumed, frame.len());
        }
    }

    // ---- Max payload size boundary ----

    #[test]
    fn encode_rejects_oversized_payload() {
        let huge = "x".repeat(MAX_PAYLOAD_SIZE as usize + 1);
        let msg = FrontendMessage::SubmitInput {
            session_id: "s".into(),
            text: huge,
        };
        let err = encode_frame(&msg).unwrap_err();
        assert!(matches!(err, FrameError::PayloadTooLarge(_)));
    }

    #[test]
    fn decode_rejects_oversized_length_prefix() {
        let mut buf = vec![0u8; 8];
        let bad_len: u32 = MAX_PAYLOAD_SIZE + 1;
        buf[0..4].copy_from_slice(&bad_len.to_be_bytes());
        let err = decode_frame::<FrontendMessage>(&buf).unwrap_err();
        assert!(matches!(err, FrameError::PayloadTooLarge(_)));
    }

    // ---- Incomplete buffer ----

    #[test]
    fn decode_returns_incomplete_for_short_buffer() {
        assert!(matches!(
            decode_frame::<FrontendMessage>(&[0, 0]),
            Err(FrameError::IncompleteBuf)
        ));
        // Header says 10 bytes but only 2 payload bytes present
        let buf = [0, 0, 0, 10, b'h', b'i'];
        assert!(matches!(
            decode_frame::<FrontendMessage>(&buf),
            Err(FrameError::IncompleteBuf)
        ));
    }

    // ---- DaemonEnvelope demux ----

    #[test]
    fn daemon_envelope_demuxes_all_variants() {
        let lifecycle = DaemonLifecycleEvent::DaemonReady {
            version: "0.1".into(),
        };
        let frame = encode_frame(&lifecycle).expect("encode lifecycle");
        let (envelope, _): (DaemonEnvelope, _) = decode_frame(&frame).expect("decode");
        assert!(matches!(envelope, DaemonEnvelope::DaemonReady { .. }));

        let turn = DaemonMessage::TurnComplete {
            session_id: "s1".into(),
            outcome: "done".into(),
        };
        let frame = encode_frame(&turn).expect("encode turn");
        let (envelope, _): (DaemonEnvelope, _) = decode_frame(&frame).expect("decode");
        assert!(matches!(envelope, DaemonEnvelope::TurnComplete { .. }));
    }

    // ---- StreamEventPayload covers all variants ----

    #[test]
    fn stream_event_payload_all_variants_round_trip() {
        let cases = vec![
            StreamEventPayload::Text {
                text: "hello".into(),
            },
            StreamEventPayload::Status {
                status: "thinking...".into(),
            },
            StreamEventPayload::ToolCallStarted {
                tool_id: "id-1".into(),
                tool_name: "memory.read".into(),
                input: serde_json::json!({"topic": "notes"}),
            },
            StreamEventPayload::ToolCallFinished {
                tool_id: "id-1".into(),
                tool_name: "memory.read".into(),
                outcome_summary: "3 entries".into(),
            },
            StreamEventPayload::ToolOutput {
                tool_id: "id-1".into(),
                tool_name: "web.fetch".into(),
                chunk: "<html>...".into(),
            },
        ];
        for payload in cases {
            let json = serde_json::to_string(&payload).expect("serialize");
            let back: StreamEventPayload = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, payload);
        }
    }
}
