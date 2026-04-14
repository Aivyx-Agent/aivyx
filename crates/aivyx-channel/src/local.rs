//! `LocalChannel` — the first real `ChannelContext` implementation.
//!
//! A CLI channel that writes streamed text + tool markers + finalization
//! lines to a `std::io::Write` sink. The production binary wires it to
//! `io::stdout()`; unit tests wire it to a `Vec<u8>` behind an `Arc<Mutex>`
//! so assertions can inspect exactly what the user would have seen.
//!
//! ## Design notes
//!
//! - `ChannelContext` requires `Send + Sync`. The writer is held behind a
//!   `Mutex<W>` so that the `stream_event` / `finalize` async methods can
//!   take `&self` while still mutating the sink. The mutex is *never*
//!   held across an await — every critical section is a `write_all` call
//!   followed by a `flush`, both synchronous, so there's no deadlock
//!   risk from the cooperative scheduler.
//! - `W: Write + Send + 'static`. `Sync` is explicitly *not* required on
//!   the writer itself — `Mutex<W>` is `Sync` for any `Send` interior, so
//!   adding `Sync` would needlessly exclude common sinks like
//!   `io::Stdout` which is `Send` but technically `Sync` via its own
//!   lock.
//! - `flush()` runs after every text chunk. The whole point of a CLI
//!   channel is that tokens appear *as they arrive*; buffering defeats
//!   the streaming story. This makes each text chunk a syscall, which
//!   is fine at LLM token rates (~100/sec max) but would be a
//!   performance trap if the channel were ever reused for bulk I/O.
//!   Flagged, not fixed — a future batch channel is a different impl.
//! - `StreamEvent::Text` writes the chunk verbatim (no newline). The
//!   LLM is responsible for its own whitespace; adding one would
//!   double-space any model that already includes trailing newlines.
//! - `StreamEvent::ToolCallStarted` / `ToolCallFinished` get one-line
//!   markers prefixed with `  → ` / `  ← ` so they're visually distinct
//!   from the assistant text stream.
//! - `finalize()` prints a trailing newline + a one-line turn-outcome
//!   marker. This is the first phase where `finalize` actually writes
//!   anything — Phase 1/2 tests all no-op'd it — so the format is
//!   deliberately minimal and grep-able.

use std::io::Write;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use aivyx_core::{
    CancellationToken, ChannelContext, ChannelError, ChannelPlatform, SessionId, StreamEvent,
    TurnOutcome, TurnOutcomeSummary,
};
use aivyx_capability::TrustTier;

/// A CLI `ChannelContext` that streams turn output to a synchronous
/// writer.
///
/// Generic over `W: Write + Send` so the same impl covers `io::Stdout`
/// in production and `Vec<u8>` in tests.
pub struct LocalChannel<W: Write + Send + 'static> {
    name: String,
    session: SessionId,
    token: CancellationToken,
    writer: Arc<Mutex<W>>,
}

impl<W: Write + Send + 'static> LocalChannel<W> {
    /// Construct a `LocalChannel` with a caller-supplied writer. The
    /// writer is wrapped in an `Arc<Mutex>` internally; callers that
    /// need to inspect the sink after a turn can obtain a shared
    /// handle via [`writer_handle`](Self::writer_handle).
    pub fn new(name: impl Into<String>, writer: W) -> Self {
        LocalChannel {
            name: name.into(),
            session: SessionId::new(),
            token: CancellationToken::new(),
            writer: Arc::new(Mutex::new(writer)),
        }
    }

    /// Obtain a second handle to the underlying writer. Tests use this
    /// to read back what was written after a turn has completed.
    pub fn writer_handle(&self) -> Arc<Mutex<W>> {
        Arc::clone(&self.writer)
    }

    /// Access the channel's cancellation token without cloning. Used by
    /// the binary's signal handler to trigger a mid-turn cancel.
    pub fn cancel_handle(&self) -> CancellationToken {
        self.token.clone()
    }
}

#[async_trait]
impl<W: Write + Send + 'static> ChannelContext for LocalChannel<W> {
    fn channel_name(&self) -> &str {
        &self.name
    }

    fn platform(&self) -> ChannelPlatform {
        ChannelPlatform::Local
    }

    fn trust_tier(&self) -> TrustTier {
        // The local CLI is the single most trusted channel on the box —
        // it runs as the user, in the user's shell. Remote channels
        // will drop to `SemiTrusted` or `Untrusted` as they're added.
        TrustTier::Trusted
    }

    fn session_id(&self) -> SessionId {
        self.session
    }

    async fn stream_event(&self, event: StreamEvent<'_>) -> Result<(), ChannelError> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|e| ChannelError::Send(format!("LocalChannel writer poisoned: {e}")))?;

        let write_result = match event {
            StreamEvent::Text(chunk) => writer.write_all(chunk.as_bytes()),
            StreamEvent::Status(msg) => writeln!(writer, "\n  · {msg}"),
            StreamEvent::ToolCallStarted { tool, input } => {
                writeln!(writer, "\n  → tool[{tool:?}] {input}")
            }
            StreamEvent::ToolCallFinished {
                tool,
                outcome_summary,
            } => writeln!(writer, "  ← tool[{tool:?}] {outcome_summary}"),
            StreamEvent::Attachment { kind, data, filename } => {
                let name = filename.unwrap_or("<unnamed>");
                writeln!(
                    writer,
                    "\n  ⎘ attachment[{kind:?}] {name} ({} bytes)",
                    data.len()
                )
            }
        };

        write_result.map_err(|e| ChannelError::Send(format!("LocalChannel write failed: {e}")))?;
        writer
            .flush()
            .map_err(|e| ChannelError::Send(format!("LocalChannel flush failed: {e}")))?;

        Ok(())
    }

    async fn finalize(&self, outcome: &TurnOutcome) -> Result<(), ChannelError> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|e| ChannelError::Send(format!("LocalChannel writer poisoned: {e}")))?;

        let summary = TurnOutcomeSummary::from(outcome);
        let marker = match summary {
            TurnOutcomeSummary::Completed => "completed",
            TurnOutcomeSummary::Escalated => "escalated",
            TurnOutcomeSummary::TimedOut => "timed out",
            TurnOutcomeSummary::Cancelled => "cancelled",
            TurnOutcomeSummary::Failed => "failed",
        };

        writeln!(writer, "\n[turn {marker}]")
            .map_err(|e| ChannelError::Send(format!("LocalChannel finalize failed: {e}")))?;
        writer
            .flush()
            .map_err(|e| ChannelError::Send(format!("LocalChannel flush failed: {e}")))?;

        Ok(())
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.token.clone()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_core::{AivyxError, ToolId};
    use serde_json::json;

    /// Build a `LocalChannel` wrapped around an in-memory byte sink,
    /// plus the shared handle the test uses to read back what was
    /// written.
    fn mem_channel() -> (LocalChannel<Vec<u8>>, Arc<Mutex<Vec<u8>>>) {
        let channel = LocalChannel::new("test", Vec::<u8>::new());
        let handle = channel.writer_handle();
        (channel, handle)
    }

    fn read_output(handle: &Arc<Mutex<Vec<u8>>>) -> String {
        String::from_utf8(handle.lock().unwrap().clone()).expect("output is utf-8")
    }

    #[test]
    fn platform_and_trust_tier_are_local_and_trusted() {
        let (channel, _) = mem_channel();
        assert_eq!(channel.platform(), ChannelPlatform::Local);
        assert_eq!(channel.trust_tier(), TrustTier::Trusted);
        assert_eq!(channel.channel_name(), "test");
    }

    #[test]
    fn session_id_is_stable_across_reads() {
        let (channel, _) = mem_channel();
        let s1 = channel.session_id();
        let s2 = channel.session_id();
        assert_eq!(s1, s2, "session_id must be stable across calls");
    }

    #[tokio::test]
    async fn text_chunks_are_written_verbatim_in_order() {
        let (channel, handle) = mem_channel();

        channel
            .stream_event(StreamEvent::Text("Hello, "))
            .await
            .unwrap();
        channel
            .stream_event(StreamEvent::Text("world!"))
            .await
            .unwrap();

        assert_eq!(
            read_output(&handle),
            "Hello, world!",
            "text chunks should be concatenated verbatim — no added whitespace"
        );
    }

    #[tokio::test]
    async fn tool_call_markers_are_distinct_from_text() {
        let (channel, handle) = mem_channel();
        let tool = ToolId::new();

        channel
            .stream_event(StreamEvent::Text("thinking…"))
            .await
            .unwrap();
        channel
            .stream_event(StreamEvent::ToolCallStarted {
                tool,
                input: &json!({"topic": "todos"}),
            })
            .await
            .unwrap();
        channel
            .stream_event(StreamEvent::ToolCallFinished {
                tool,
                outcome_summary: "completed (2 items)",
            })
            .await
            .unwrap();
        channel
            .stream_event(StreamEvent::Text("here's what I found."))
            .await
            .unwrap();

        let out = read_output(&handle);
        assert!(out.starts_with("thinking…"), "text prefix preserved");
        assert!(
            out.contains("→ tool["),
            "tool-start marker should appear: {out:?}"
        );
        assert!(
            out.contains("← tool["),
            "tool-end marker should appear: {out:?}"
        );
        assert!(
            out.contains("completed (2 items)"),
            "outcome summary should be rendered: {out:?}"
        );
        assert!(out.ends_with("here's what I found."), "trailing text preserved");
    }

    #[tokio::test]
    async fn status_events_render_with_prefix() {
        let (channel, handle) = mem_channel();
        channel
            .stream_event(StreamEvent::Status("retrying"))
            .await
            .unwrap();
        assert!(read_output(&handle).contains("· retrying"));
    }

    #[tokio::test]
    async fn finalize_completed_writes_marker() {
        let (channel, handle) = mem_channel();
        let outcome = TurnOutcome::Completed {
            final_message: "done".to_string(),
            tool_calls_made: 0,
            duration: std::time::Duration::from_millis(1),
        };
        channel.finalize(&outcome).await.unwrap();
        let out = read_output(&handle);
        assert!(
            out.ends_with("[turn completed]\n"),
            "finalize should end with the completion marker, got {out:?}"
        );
    }

    #[tokio::test]
    async fn finalize_cancelled_and_failed_write_distinct_markers() {
        let (channel_a, handle_a) = mem_channel();
        channel_a
            .finalize(&TurnOutcome::Cancelled {
                tool_calls_made: 0,
            })
            .await
            .unwrap();
        assert!(read_output(&handle_a).contains("[turn cancelled]"));

        let (channel_b, handle_b) = mem_channel();
        channel_b
            .finalize(&TurnOutcome::Failed(AivyxError::Internal("boom".into())))
            .await
            .unwrap();
        assert!(read_output(&handle_b).contains("[turn failed]"));
    }

    #[tokio::test]
    async fn cancellation_token_is_shared_between_channel_and_handle() {
        let (channel, _) = mem_channel();
        let cancel = channel.cancel_handle();
        assert!(!channel.cancellation_token().is_cancelled());
        cancel.cancel();
        assert!(
            channel.cancellation_token().is_cancelled(),
            "cancelling via cancel_handle should be visible through cancellation_token()"
        );
    }

    #[tokio::test]
    async fn each_text_chunk_is_flushed_so_streaming_is_visible() {
        // Proxy for "we called flush after every write": wrap the sink
        // in a type that counts flushes, and assert one flush per write.
        struct CountingSink {
            buf: Vec<u8>,
            flushes: usize,
        }
        impl Write for CountingSink {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                self.buf.extend_from_slice(data);
                Ok(data.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                self.flushes += 1;
                Ok(())
            }
        }

        let sink = CountingSink {
            buf: Vec::new(),
            flushes: 0,
        };
        let channel = LocalChannel::new("counting", sink);
        let handle = channel.writer_handle();

        channel.stream_event(StreamEvent::Text("a")).await.unwrap();
        channel.stream_event(StreamEvent::Text("b")).await.unwrap();
        channel.stream_event(StreamEvent::Text("c")).await.unwrap();

        let guard = handle.lock().unwrap();
        assert_eq!(guard.buf, b"abc");
        assert_eq!(
            guard.flushes, 3,
            "each text chunk should trigger one flush so the user sees tokens as they arrive"
        );
    }
}
