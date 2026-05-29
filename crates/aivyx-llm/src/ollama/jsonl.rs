//! Phase 121 Task 3 — Newline-delimited JSON (JSONL) line reader
//! over a [`ByteStream`].
//!
//! Ollama's `/api/chat` streams as newline-delimited JSON: one
//! complete JSON object per `\n` boundary, no `data:` framing,
//! no `[DONE]` sentinel. The transport layer's `post_sse`
//! returns the raw byte stream; this reader buffers bytes,
//! splits on `\n`, and emits one trimmed UTF-8 line at a time
//! to the caller. The caller is responsible for deserializing
//! each line into the Ollama-specific chunk shape (Task 4).
//!
//! ## Contract
//!
//! - Lines exclude the trailing `\n`.
//! - Empty lines (`\n\n`) and lines that are pure whitespace are
//!   skipped — Ollama's protocol doesn't use them, but defending
//!   against trailing blank lines from buffering edge cases is
//!   cheap and operator-conservative.
//! - A trailing partial line at EOF (no closing `\n`) is emitted
//!   as the final line. Ollama's stream always ends with a
//!   `done: true` chunk followed by `\n`, but the spec doesn't
//!   prohibit a final non-terminated line, so we emit it.
//! - Non-UTF8 bytes cause [`LlmError::Parse`]. Ollama's
//!   protocol is JSON-only and the JSON spec mandates UTF-8.
//!
//! ## Why a dedicated module
//!
//! Keeps the streaming substrate testable without spinning up
//! the rest of the Ollama provider. Task 4's stream state
//! machine reuses this reader; Task 5's `chat_stream` integration
//! wires it onto a real `ByteStream` from `HttpTransport::post_sse`.

use futures_util::StreamExt;

use crate::transport::ByteStream;
use crate::LlmError;

/// State-machine reader that emits one JSONL line per call. Holds
/// a buffer of pending bytes from the underlying stream so a line
/// spanning multiple `Bytes` chunks reassembles correctly.
pub struct JsonlReader {
    stream: ByteStream,
    buf: Vec<u8>,
    exhausted: bool,
}

impl JsonlReader {
    pub fn new(stream: ByteStream) -> Self {
        JsonlReader {
            stream,
            buf: Vec::with_capacity(4096),
            exhausted: false,
        }
    }

    /// Return the next JSONL line, excluding the trailing `\n`.
    /// Returns `Ok(None)` when the stream has been fully consumed
    /// (no more bytes AND the buffer has no trailing partial
    /// line). See module docs for the EOF, blank-line, and
    /// UTF-8 contracts.
    pub async fn next_line(&mut self) -> Result<Option<String>, LlmError> {
        loop {
            // Try to extract a `\n`-terminated line from the
            // current buffer. Skip blank-whitespace lines (they
            // don't appear in Ollama's protocol but defending
            // against them is cheap).
            if let Some(line) = try_extract_line(&mut self.buf)? {
                if line.trim().is_empty() {
                    continue;
                }
                return Ok(Some(line));
            }

            // No complete line in buffer. If the stream is
            // exhausted, emit any trailing partial line (also
            // skipping blank-whitespace) and return None
            // thereafter.
            if self.exhausted {
                if !self.buf.is_empty() {
                    let trailing =
                        std::str::from_utf8(&self.buf).map_err(|e| {
                            LlmError::Parse(format!(
                                "ollama JSONL non-UTF8 trailing bytes: {e}"
                            ))
                        })?;
                    let trimmed = trailing.trim();
                    if trimmed.is_empty() {
                        self.buf.clear();
                        return Ok(None);
                    }
                    let out = trimmed.to_string();
                    self.buf.clear();
                    return Ok(Some(out));
                }
                return Ok(None);
            }

            // Pull more bytes from the underlying stream.
            match self.stream.next().await {
                Some(Ok(chunk)) => self.buf.extend_from_slice(&chunk),
                Some(Err(e)) => return Err(e),
                None => self.exhausted = true,
            }
        }
    }
}

/// Try to extract a single `\n`-terminated line from `buf`. On
/// success, the matched bytes (including the `\n`) are drained
/// from the buffer and the returned `String` has the `\n`
/// stripped. Returns `Ok(None)` when no `\n` exists yet.
///
/// Pure function on the buffer; no stream interaction. Extracted
/// so the line-splitting logic is testable without async
/// scaffolding.
fn try_extract_line(buf: &mut Vec<u8>) -> Result<Option<String>, LlmError> {
    let Some(nl_idx) = buf.iter().position(|&b| b == b'\n') else {
        return Ok(None);
    };
    let line_bytes: Vec<u8> = buf.drain(..=nl_idx).collect();
    // line_bytes ends with `\n`; trim it.
    let without_nl = &line_bytes[..line_bytes.len() - 1];
    // Handle CRLF (defensive — Ollama uses LF but proxies might
    // rewrite).
    let without_cr = if without_nl.last() == Some(&b'\r') {
        &without_nl[..without_nl.len() - 1]
    } else {
        without_nl
    };
    let line = std::str::from_utf8(without_cr).map_err(|e| {
        LlmError::Parse(format!("ollama JSONL non-UTF8 line: {e}"))
    })?;
    Ok(Some(line.to_string()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;
    use futures_util::stream;
    use std::pin::Pin;

    fn make_stream(chunks: Vec<&[u8]>) -> ByteStream {
        let items: Vec<Result<Bytes, LlmError>> = chunks
            .into_iter()
            .map(|c| Ok::<_, LlmError>(Bytes::copy_from_slice(c)))
            .collect();
        Pin::from(Box::new(stream::iter(items)))
            as Pin<Box<dyn futures_util::Stream<Item = _> + Send>>
    }

    // ----- try_extract_line (pure helper) -----

    #[test]
    fn try_extract_line_returns_none_for_no_newline() {
        let mut buf = b"partial".to_vec();
        let line = try_extract_line(&mut buf).unwrap();
        assert!(line.is_none());
        // Buffer untouched on no-match.
        assert_eq!(buf, b"partial");
    }

    #[test]
    fn try_extract_line_drains_one_line_excluding_newline() {
        let mut buf = b"{\"a\":1}\nrest".to_vec();
        let line = try_extract_line(&mut buf).unwrap();
        assert_eq!(line.as_deref(), Some("{\"a\":1}"));
        assert_eq!(buf, b"rest");
    }

    #[test]
    fn try_extract_line_handles_crlf() {
        // Defensive against proxy-rewritten line endings.
        let mut buf = b"line\r\ntail".to_vec();
        let line = try_extract_line(&mut buf).unwrap();
        assert_eq!(line.as_deref(), Some("line"));
        assert_eq!(buf, b"tail");
    }

    #[test]
    fn try_extract_line_returns_empty_string_for_bare_newline() {
        let mut buf = b"\ntail".to_vec();
        let line = try_extract_line(&mut buf).unwrap();
        assert_eq!(line.as_deref(), Some(""));
        assert_eq!(buf, b"tail");
    }

    #[test]
    fn try_extract_line_rejects_non_utf8() {
        let mut buf = vec![0xFF, b'\n'];
        let err = try_extract_line(&mut buf).unwrap_err();
        assert!(matches!(err, LlmError::Parse(_)));
    }

    // ----- JsonlReader (async stream-driven) -----

    #[tokio::test]
    async fn reader_emits_single_line_complete_in_one_chunk() {
        let stream = make_stream(vec![b"{\"a\":1}\n"]);
        let mut reader = JsonlReader::new(stream);
        let line = reader.next_line().await.unwrap();
        assert_eq!(line.as_deref(), Some("{\"a\":1}"));
        // Next call returns None on EOF.
        assert!(reader.next_line().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn reader_emits_multiple_lines_in_one_chunk() {
        let stream = make_stream(vec![b"{\"a\":1}\n{\"b\":2}\n{\"c\":3}\n"]);
        let mut reader = JsonlReader::new(stream);
        assert_eq!(
            reader.next_line().await.unwrap().as_deref(),
            Some("{\"a\":1}")
        );
        assert_eq!(
            reader.next_line().await.unwrap().as_deref(),
            Some("{\"b\":2}")
        );
        assert_eq!(
            reader.next_line().await.unwrap().as_deref(),
            Some("{\"c\":3}")
        );
        assert!(reader.next_line().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn reader_reassembles_line_spanning_two_chunks() {
        // The load-bearing case for the buffer state machine: a
        // single Ollama chunk arrives split across two
        // network reads. The buffer must hold the partial line
        // until the `\n` arrives.
        let stream =
            make_stream(vec![b"{\"a\":1", b",\"b\":2}\n"]);
        let mut reader = JsonlReader::new(stream);
        let line = reader.next_line().await.unwrap();
        assert_eq!(line.as_deref(), Some("{\"a\":1,\"b\":2}"));
        assert!(reader.next_line().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn reader_emits_trailing_partial_line_at_eof() {
        // Defensive: Ollama always emits a final `\n` after
        // `done: true`, but the spec doesn't prohibit a final
        // non-terminated line. We emit it.
        let stream = make_stream(vec![b"{\"a\":1}\n{\"b\":2}"]);
        let mut reader = JsonlReader::new(stream);
        assert_eq!(
            reader.next_line().await.unwrap().as_deref(),
            Some("{\"a\":1}")
        );
        // Trailing line emits even without `\n`.
        assert_eq!(
            reader.next_line().await.unwrap().as_deref(),
            Some("{\"b\":2}")
        );
        assert!(reader.next_line().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn reader_skips_blank_lines() {
        // Defensive against buffering edge cases; Ollama doesn't
        // emit blank lines but we tolerate them.
        let stream = make_stream(vec![b"{\"a\":1}\n\n{\"b\":2}\n"]);
        let mut reader = JsonlReader::new(stream);
        assert_eq!(
            reader.next_line().await.unwrap().as_deref(),
            Some("{\"a\":1}")
        );
        // The blank line is skipped; next non-blank line returned.
        assert_eq!(
            reader.next_line().await.unwrap().as_deref(),
            Some("{\"b\":2}")
        );
        assert!(reader.next_line().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn reader_handles_empty_stream() {
        let stream = make_stream(vec![]);
        let mut reader = JsonlReader::new(stream);
        assert!(reader.next_line().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn reader_handles_only_newline_chunks() {
        // Pure whitespace → no lines emitted; treated as EOF
        // for the caller's purposes.
        let stream = make_stream(vec![b"\n\n\n"]);
        let mut reader = JsonlReader::new(stream);
        assert!(reader.next_line().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn reader_emits_multiple_chunks_with_split_lines() {
        // Combined stress test: 4 chunks, 3 lines split
        // across boundaries unevenly.
        let stream = make_stream(vec![
            b"{\"a\":1}\n{\"b\":",
            b"2}\n{\"c\":",
            b"3,\"d\":",
            b"4}\n",
        ]);
        let mut reader = JsonlReader::new(stream);
        assert_eq!(
            reader.next_line().await.unwrap().as_deref(),
            Some("{\"a\":1}")
        );
        assert_eq!(
            reader.next_line().await.unwrap().as_deref(),
            Some("{\"b\":2}")
        );
        assert_eq!(
            reader.next_line().await.unwrap().as_deref(),
            Some("{\"c\":3,\"d\":4}")
        );
        assert!(reader.next_line().await.unwrap().is_none());
    }

    #[tokio::test]
    async fn reader_surfaces_underlying_stream_error() {
        let items: Vec<Result<Bytes, LlmError>> = vec![
            Ok(Bytes::from_static(b"{\"a\":1}\n")),
            Err(LlmError::Transport("network died".into())),
        ];
        let stream: ByteStream = Pin::from(Box::new(stream::iter(items)))
            as Pin<Box<dyn futures_util::Stream<Item = _> + Send>>;
        let mut reader = JsonlReader::new(stream);
        // First line succeeds.
        assert_eq!(
            reader.next_line().await.unwrap().as_deref(),
            Some("{\"a\":1}")
        );
        // Next call surfaces the transport error.
        let err = reader.next_line().await.unwrap_err();
        assert!(matches!(err, LlmError::Transport(_)));
    }
}
