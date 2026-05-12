//! Length-prefixed JSON framing for the tool process protocol.
//!
//! Wire format (per `docs/DAEMON_IPC.md`):
//!
//! ```text
//! +-----------------+-----------------------------------+
//! | 4 bytes         | N bytes                           |
//! | big-endian u32  | UTF-8 JSON payload                |
//! +-----------------+-----------------------------------+
//! ```
//!
//! Identical framing to `aivyx-channel::daemon_ipc`. Reimplemented
//! here so `aivyx-tool` does not depend on `aivyx-channel` (the
//! crate-graph edge would be the wrong direction — daemon side
//! pulls `aivyx-tool` in, not vice versa).

use serde::Serialize;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAX_PAYLOAD_SIZE: u32 = 16 * 1024 * 1024;
pub const FRAME_HEADER_LEN: usize = 4;

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("payload size {0} bytes exceeds 16 MiB cap")]
    PayloadTooLarge(usize),
    #[error("frame length prefix {0} exceeds 16 MiB cap")]
    HeaderTooLarge(u32),
    #[error("serialize failed: {0}")]
    Serialize(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid utf-8 in frame body: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("invalid json in frame body: {0}")]
    Json(#[from] serde_json::Error),
}

/// Encode `message` as a length-prefixed frame. Returns the bytes
/// to write to the wire.
pub fn encode_frame<T: Serialize>(message: &T) -> Result<Vec<u8>, FrameError> {
    let body =
        serde_json::to_vec(message).map_err(|e| FrameError::Serialize(e.to_string()))?;
    if body.len() > MAX_PAYLOAD_SIZE as usize {
        return Err(FrameError::PayloadTooLarge(body.len()));
    }
    let len = body.len() as u32;
    let mut out = Vec::with_capacity(FRAME_HEADER_LEN + body.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// Read one full frame from an async reader. Blocks until the
/// header + body are available. The returned bytes are the
/// decoded UTF-8 JSON body — the caller can `serde_json::from_str`
/// it into whatever shape they need.
///
/// Returns `Ok(None)` if the reader is at EOF before any header
/// bytes arrive (peer closed cleanly). Returns `Err` on a partial
/// frame.
pub async fn read_frame<R: AsyncReadExt + Unpin>(
    reader: &mut R,
) -> Result<Option<String>, FrameError> {
    let mut header = [0u8; FRAME_HEADER_LEN];
    let mut bytes_read = 0;
    while bytes_read < FRAME_HEADER_LEN {
        match reader.read(&mut header[bytes_read..]).await {
            Ok(0) => {
                if bytes_read == 0 {
                    return Ok(None); // clean EOF
                }
                return Err(FrameError::Io(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    format!(
                        "EOF in frame header after {bytes_read} of {FRAME_HEADER_LEN} bytes"
                    ),
                )));
            }
            Ok(n) => bytes_read += n,
            Err(e) => return Err(FrameError::Io(e)),
        }
    }
    let len = u32::from_be_bytes(header);
    if len > MAX_PAYLOAD_SIZE {
        return Err(FrameError::HeaderTooLarge(len));
    }
    let mut body = vec![0u8; len as usize];
    reader.read_exact(&mut body).await?;
    let text = String::from_utf8(body)?;
    Ok(Some(text))
}

/// Write one frame to an async writer. Convenience wrapper —
/// combines `encode_frame` and `write_all`.
pub async fn write_frame<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    message: &T,
) -> Result<(), FrameError> {
    let bytes = encode_frame(message)?;
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[derive(Debug, serde::Serialize, serde::Deserialize, PartialEq)]
    struct Sample {
        kind: String,
        value: i32,
    }

    #[tokio::test]
    async fn encode_then_decode_round_trips() {
        let (mut a, mut b) = duplex(8192);
        let sample = Sample {
            kind: "hello".into(),
            value: 42,
        };
        write_frame(&mut a, &sample).await.unwrap();
        let body = read_frame(&mut b).await.unwrap().expect("non-EOF");
        let back: Sample = serde_json::from_str(&body).unwrap();
        assert_eq!(back, sample);
    }

    #[tokio::test]
    async fn read_frame_returns_none_on_clean_eof() {
        let (a, mut b) = duplex(8192);
        drop(a); // close the write side cleanly
        let result = read_frame(&mut b).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn read_frame_errors_on_partial_header() {
        let (mut a, mut b) = duplex(8192);
        a.write_all(&[0u8, 1u8]).await.unwrap();
        drop(a);
        let result = read_frame(&mut b).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn header_too_large_is_rejected() {
        let (mut a, mut b) = duplex(8192);
        let bogus_len = (MAX_PAYLOAD_SIZE + 1).to_be_bytes();
        a.write_all(&bogus_len).await.unwrap();
        drop(a);
        let result = read_frame(&mut b).await;
        match result {
            Err(FrameError::HeaderTooLarge(_)) => {}
            other => panic!("expected HeaderTooLarge, got {other:?}"),
        }
    }
}
