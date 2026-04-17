//! Minimal Server-Sent Events parser for the Anthropic Messages stream.
//!
//! We deliberately do not implement the full SSE spec — Anthropic's event
//! stream is narrow enough that "split on `\n\n`, look for `event:` and
//! `data:` prefixes" is sufficient. If we ever need a second SSE source
//! with richer features (id, retry, multiline data), this parser will
//! have to grow.
//!
//! The parser owns a `ByteStream` and a `Vec<u8>` tail buffer. Each call
//! to [`SseReader::next_event`] pulls more bytes until it can emit one
//! complete event or the stream ends.

use bytes::Bytes;
use futures_util::StreamExt;

use crate::LlmError;

use crate::transport::ByteStream;

/// One parsed SSE event. Anthropic always sends both the `event:` line
/// and a `data:` line, so both fields are required. `data` is the raw
/// JSON text — the caller (the provider's state machine) parses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SseEvent {
    pub event: String,
    pub data: String,
}

pub struct SseReader {
    stream: ByteStream,
    buf: Vec<u8>,
    exhausted: bool,
}

impl SseReader {
    pub fn new(stream: ByteStream) -> Self {
        SseReader {
            stream,
            buf: Vec::with_capacity(4096),
            exhausted: false,
        }
    }

    /// Pull the next complete SSE event from the stream. Returns `None`
    /// when the underlying byte stream has ended cleanly with no
    /// trailing partial event.
    pub async fn next_event(&mut self) -> Result<Option<SseEvent>, LlmError> {
        loop {
            if let Some(event) = try_parse_one(&mut self.buf)? {
                return Ok(Some(event));
            }
            if self.exhausted {
                // Stream ended. If there's trailing garbage it's a parse
                // error; an empty buffer is a clean end-of-stream.
                return if self.buf.is_empty() {
                    Ok(None)
                } else if self.buf.iter().all(|b| matches!(b, b'\r' | b'\n' | b' ')) {
                    self.buf.clear();
                    Ok(None)
                } else {
                    Err(LlmError::Parse(format!(
                        "SSE stream ended mid-event, {} trailing bytes",
                        self.buf.len()
                    )))
                };
            }
            match self.stream.next().await {
                Some(Ok(chunk)) => self.buf.extend_from_slice(&chunk),
                Some(Err(e)) => return Err(e),
                None => self.exhausted = true,
            }
        }
    }
}

/// Scan `buf` for a complete event (delimited by `\n\n`). If one is
/// found, drain it from the front of `buf` and return it. Otherwise
/// return `Ok(None)` to signal "need more bytes."
fn try_parse_one(buf: &mut Vec<u8>) -> Result<Option<SseEvent>, LlmError> {
    // Look for a frame terminator: `\n\n` or `\r\n\r\n`.
    let Some((end, term_len)) = find_double_newline(buf) else {
        return Ok(None);
    };

    // Extract the frame (bytes up to the terminator).
    let frame_bytes: Bytes = Bytes::copy_from_slice(&buf[..end]);
    buf.drain(..end + term_len);

    let frame_text = std::str::from_utf8(&frame_bytes)
        .map_err(|e| LlmError::Parse(format!("SSE frame is not UTF-8: {e}")))?;

    let mut event_name: Option<String> = None;
    let mut data_line: Option<String> = None;
    for raw_line in frame_text.split('\n') {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with(':') {
            // Empty line or comment — ignore.
            continue;
        }
        if let Some(value) = line.strip_prefix("event:") {
            event_name = Some(value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("data:") {
            // Strip one leading space per the SSE convention, but only one.
            let trimmed = value.strip_prefix(' ').unwrap_or(value);
            data_line = Some(trimmed.to_string());
        }
        // Other fields (id:, retry:) are ignored — Anthropic doesn't send them.
    }

    match (event_name, data_line) {
        (Some(event), Some(data)) => Ok(Some(SseEvent { event, data })),
        (None, Some(_)) => Err(LlmError::Parse(
            "SSE frame had data: but no event: line".to_string(),
        )),
        (Some(_), None) => Err(LlmError::Parse(
            "SSE frame had event: but no data: line".to_string(),
        )),
        (None, None) => {
            // An all-empty frame — silently skip it and ask for more.
            Ok(None)
        }
    }
}

/// Look for a frame terminator: either `\n\n` (2 bytes) or `\r\n\r\n`
/// (4 bytes). Returns `(offset, terminator_len)` — the offset is the
/// start of the terminator in `buf`, the length is how many bytes to
/// skip past it.
fn find_double_newline(buf: &[u8]) -> Option<(usize, usize)> {
    // Prefer the longer terminator first so LF-only streams still work
    // when the buffer happens to contain both.
    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
        return Some((pos, 4));
    }
    buf.windows(2)
        .position(|w| w == b"\n\n")
        .map(|pos| (pos, 2))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;

    fn stream_from_chunks(chunks: Vec<&'static str>) -> ByteStream {
        let iter = chunks
            .into_iter()
            .map(|s| Ok::<Bytes, LlmError>(Bytes::from_static(s.as_bytes())));
        Box::pin(stream::iter(iter))
    }

    #[tokio::test]
    async fn parses_one_complete_event_in_one_chunk() {
        let s = stream_from_chunks(vec!["event: message_start\ndata: {\"x\":1}\n\n"]);
        let mut r = SseReader::new(s);
        let ev = r.next_event().await.unwrap().unwrap();
        assert_eq!(ev.event, "message_start");
        assert_eq!(ev.data, "{\"x\":1}");
        assert!(r.next_event().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn parses_event_split_across_chunks() {
        // The boundary falls in the middle of `data:`.
        let s = stream_from_chunks(vec![
            "event: content_block_delta\nda",
            "ta: {\"delta\":\"hi\"}\n\n",
        ]);
        let mut r = SseReader::new(s);
        let ev = r.next_event().await.unwrap().unwrap();
        assert_eq!(ev.event, "content_block_delta");
        assert_eq!(ev.data, "{\"delta\":\"hi\"}");
    }

    #[tokio::test]
    async fn parses_multiple_events_in_one_chunk() {
        let s = stream_from_chunks(vec![
            "event: a\ndata: 1\n\nevent: b\ndata: 2\n\nevent: c\ndata: 3\n\n",
        ]);
        let mut r = SseReader::new(s);
        let a = r.next_event().await.unwrap().unwrap();
        let b = r.next_event().await.unwrap().unwrap();
        let c = r.next_event().await.unwrap().unwrap();
        assert_eq!((a.event.as_str(), a.data.as_str()), ("a", "1"));
        assert_eq!((b.event.as_str(), b.data.as_str()), ("b", "2"));
        assert_eq!((c.event.as_str(), c.data.as_str()), ("c", "3"));
        assert!(r.next_event().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn tolerates_crlf_line_endings() {
        // Some proxies rewrite LF to CRLF. The parser accepts both the
        // LFLF and CRLFCRLF frame terminators.
        let s = stream_from_chunks(vec!["event: ping\r\ndata: {}\r\n\r\n"]);
        let mut r = SseReader::new(s);
        let ev = r.next_event().await.unwrap().unwrap();
        assert_eq!(ev.event, "ping");
        assert_eq!(ev.data, "{}");
    }

    #[tokio::test]
    async fn ignores_comment_lines() {
        let s = stream_from_chunks(vec![
            ": this is a heartbeat comment\nevent: ping\ndata: {}\n\n",
        ]);
        let mut r = SseReader::new(s);
        let ev = r.next_event().await.unwrap().unwrap();
        assert_eq!(ev.event, "ping");
    }

    #[tokio::test]
    async fn errors_on_trailing_partial_frame() {
        let s = stream_from_chunks(vec!["event: incomplete\ndata: {\"x\":"]);
        let mut r = SseReader::new(s);
        let err = r.next_event().await.unwrap_err();
        assert!(matches!(err, LlmError::Parse(_)));
    }
}
