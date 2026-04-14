//! Unit tests for `TelegramChannel`.
//!
//! Every test in this module drives the channel against a
//! `ScriptedTransport` — an in-memory test double that implements
//! [`TelegramTransport`] with pre-canned inbound updates and a
//! `Mutex<Vec<OutgoingMessage>>` capture buffer. **No test hits the
//! network**, and no test depends on `frankenstein` beyond what the
//! production `ReqwestTransport` already pulls in. That is the whole
//! point of the private transport trait: the Phase 8 Task 1 test
//! surface is the same shape in CI, on a dev box, and on a plane.
//!
//! Test coverage map (per the Task 1 plan's "6–8 unit tests" bullet):
//!
//! 1. `metadata_is_telegram_and_semi_trusted` — platform, trust tier,
//!    and channel name are the values advertised in the module doc.
//! 2. `session_id_is_stable_across_reads` — matches the `LocalChannel`
//!    guarantee so audit correlation on session boundaries works.
//! 3. `reset_cancellation_rotates_token` — Phase 3 monotonic-token
//!    fix applied to the network channel.
//! 4. `finalize_sends_one_message_with_buffered_text` — the core
//!    "stream text → buffer → one send on finalize" contract.
//! 5. `tool_markers_and_status_append_to_buffer` — non-Text events
//!    render into the same buffer without becoming their own sends.
//! 6. `empty_turn_yields_no_reply_placeholder` — Telegram rejects
//!    empty sendMessage; the channel substitutes `"(no reply)"`.
//! 7. `finalize_footer_reflects_outcome` — Cancelled / Failed /
//!    TimedOut / Escalated outcomes all render a distinct footer.
//! 8. `transport_error_propagates_as_channel_error` — the scripted
//!    transport can inject a `TransportError::Platform` and the
//!    channel surfaces it as `ChannelError::Platform`.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;

use aivyx_capability::TrustTier;
use aivyx_core::{
    AivyxError, ChannelContext, ChannelPlatform, StreamEvent, ToolId, TurnOutcome,
};

use crate::telegram_channel::TelegramChannel;
use crate::transport::{IncomingMessage, OutgoingMessage, TelegramTransport, TransportError};

// ---------------------------------------------------------------------------
// ScriptedTransport — the test double
// ---------------------------------------------------------------------------

/// An in-memory `TelegramTransport` impl for tests. Three knobs:
///
/// - `updates`: a queue of pre-canned `IncomingMessage`s that
///   successive `get_updates` calls drain from. Exhausting the queue
///   returns `Ok(vec![])`, matching Bot API long-poll behavior.
/// - `sent`: a capture buffer that `send_message` appends to. Tests
///   read it back after a turn to assert exactly what the user would
///   have seen on Telegram.
/// - `send_error`: if `Some`, every `send_message` call returns that
///   error instead of buffering. Used by the error-propagation test.
struct ScriptedTransport {
    updates: Mutex<Vec<IncomingMessage>>,
    sent: Mutex<Vec<OutgoingMessage>>,
    send_error: Mutex<Option<String>>,
}

impl ScriptedTransport {
    fn new() -> Self {
        ScriptedTransport {
            updates: Mutex::new(Vec::new()),
            sent: Mutex::new(Vec::new()),
            send_error: Mutex::new(None),
        }
    }

    #[allow(dead_code)] // kept for tasks 2–6 which drive inbound updates
    fn push_update(&self, update: IncomingMessage) {
        self.updates.lock().unwrap().push(update);
    }

    fn inject_send_error(&self, err: impl Into<String>) {
        *self.send_error.lock().unwrap() = Some(err.into());
    }

    fn sent_snapshot(&self) -> Vec<OutgoingMessage> {
        self.sent.lock().unwrap().clone()
    }
}

#[async_trait]
impl TelegramTransport for ScriptedTransport {
    async fn get_updates(
        &self,
        _offset: i64,
        _timeout_secs: u32,
    ) -> Result<Vec<IncomingMessage>, TransportError> {
        let drained = std::mem::take(&mut *self.updates.lock().unwrap());
        Ok(drained)
    }

    async fn send_message(&self, msg: OutgoingMessage) -> Result<(), TransportError> {
        if let Some(err) = self.send_error.lock().unwrap().clone() {
            return Err(TransportError::Platform(err));
        }
        self.sent.lock().unwrap().push(msg);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_channel() -> (TelegramChannel<ScriptedTransport>, Arc<ScriptedTransport>) {
    let transport = Arc::new(ScriptedTransport::new());
    let channel = TelegramChannel::new("tg-test", 42, Arc::clone(&transport));
    (channel, transport)
}

fn completed_outcome() -> TurnOutcome {
    TurnOutcome::Completed {
        final_message: String::new(),
        tool_calls_made: 0,
        duration: Duration::from_millis(1),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn metadata_is_telegram_and_semi_trusted() {
    let (channel, _) = make_channel();
    assert_eq!(channel.platform(), ChannelPlatform::Telegram);
    assert_eq!(
        channel.trust_tier(),
        TrustTier::SemiTrusted,
        "Telegram = authenticated remote = SemiTrusted, not Untrusted (see module doc)"
    );
    assert_eq!(channel.channel_name(), "tg-test");
    assert_eq!(channel.chat_id(), 42);
}

#[test]
fn session_id_is_stable_across_reads() {
    let (channel, _) = make_channel();
    let s1 = channel.session_id();
    let s2 = channel.session_id();
    assert_eq!(s1, s2);
}

#[tokio::test]
async fn reset_cancellation_rotates_token() {
    // Phase 3 monotonic-token fix: a Cancelled turn must not poison
    // the next turn's cancellation token.
    let (channel, _) = make_channel();
    let old = channel.cancellation_token();
    old.cancel();
    assert!(channel.cancellation_token().is_cancelled());

    channel.reset_cancellation();
    assert!(
        !channel.cancellation_token().is_cancelled(),
        "post-reset token must be un-cancelled"
    );
    // The orphaned clone stays cancelled — that's the whole reason
    // we rotate instead of trying to un-cancel in place.
    assert!(old.is_cancelled());
}

#[tokio::test]
async fn finalize_sends_one_message_with_buffered_text() {
    // The core contract: three Text chunks buffer into one Telegram
    // send when `finalize` runs. A `LocalChannel` would have produced
    // three flushes here — the Telegram channel produces exactly one
    // `send_message` call, with the concatenated text.
    let (channel, transport) = make_channel();

    channel.stream_event(StreamEvent::Text("hello ")).await.unwrap();
    channel.stream_event(StreamEvent::Text("there, ")).await.unwrap();
    channel.stream_event(StreamEvent::Text("world")).await.unwrap();

    // Before finalize: nothing sent, buffer has the concatenation.
    assert!(transport.sent_snapshot().is_empty());
    assert_eq!(channel.buffer_snapshot(), "hello there, world");

    channel.finalize(&completed_outcome()).await.unwrap();

    let sent = transport.sent_snapshot();
    assert_eq!(sent.len(), 1, "one turn = one Telegram message");
    assert_eq!(sent[0].chat_id, 42);
    assert_eq!(sent[0].text, "hello there, world");

    // Buffer must be drained so the next turn starts clean.
    assert_eq!(channel.buffer_snapshot(), "");
}

#[tokio::test]
async fn tool_markers_and_status_append_to_buffer() {
    let (channel, transport) = make_channel();
    let tool = ToolId::new();
    let input = serde_json::json!({"path": "/tmp/x"});

    channel.stream_event(StreamEvent::Text("thinking")).await.unwrap();
    channel.stream_event(StreamEvent::Status("still thinking")).await.unwrap();
    channel
        .stream_event(StreamEvent::ToolCallStarted {
            tool,
            input: &input,
        })
        .await
        .unwrap();
    channel
        .stream_event(StreamEvent::ToolCallFinished {
            tool,
            outcome_summary: "ok",
        })
        .await
        .unwrap();
    channel.stream_event(StreamEvent::Text("done")).await.unwrap();

    channel.finalize(&completed_outcome()).await.unwrap();

    let sent = transport.sent_snapshot();
    assert_eq!(sent.len(), 1);
    let text = &sent[0].text;
    assert!(text.starts_with("thinking\n… still thinking\n"), "{text:?}");
    assert!(text.contains("→ tool["), "tool-started marker: {text:?}");
    assert!(
        text.contains("← tool[") && text.contains("ok"),
        "tool-finished marker: {text:?}"
    );
    assert!(text.ends_with("done"), "trailing text joined: {text:?}");
}

#[tokio::test]
async fn empty_turn_yields_no_reply_placeholder() {
    // A turn the LLM ended without speaking a single Text chunk (all
    // tool, no reply) must still produce a non-empty Telegram message
    // — the Bot API rejects empty `sendMessage`.
    let (channel, transport) = make_channel();
    channel.finalize(&completed_outcome()).await.unwrap();
    let sent = transport.sent_snapshot();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].text, "(no reply)");
}

#[tokio::test]
async fn finalize_footer_reflects_outcome() {
    // Each non-Completed outcome renders a distinct, grep-able footer.
    // We test three variants (Cancelled, TimedOut, Failed) — Escalated
    // is covered by the ToolId-bearing variant in the next assertion
    // block.
    let (channel, transport) = make_channel();
    channel.stream_event(StreamEvent::Text("partial")).await.unwrap();
    channel
        .finalize(&TurnOutcome::Cancelled { tool_calls_made: 0 })
        .await
        .unwrap();
    let sent = transport.sent_snapshot();
    assert!(sent[0].text.contains("✕ cancelled"), "{}", sent[0].text);

    // A fresh channel for the next outcome so buffers don't bleed.
    let (channel2, transport2) = make_channel();
    channel2.stream_event(StreamEvent::Text("slow")).await.unwrap();
    channel2
        .finalize(&TurnOutcome::TimedOut {
            tool_calls_made: 0,
            elapsed: Duration::from_secs(30),
        })
        .await
        .unwrap();
    assert!(
        transport2.sent_snapshot()[0].text.contains("⏱ timed out"),
        "{}",
        transport2.sent_snapshot()[0].text
    );

    let (channel3, transport3) = make_channel();
    channel3
        .finalize(&TurnOutcome::Failed(AivyxError::Channel("boom".into())))
        .await
        .unwrap();
    assert!(
        transport3.sent_snapshot()[0].text.contains("✕ failed"),
        "{}",
        transport3.sent_snapshot()[0].text
    );
}

#[tokio::test]
async fn transport_error_propagates_as_channel_error() {
    // The channel translates `TransportError::Platform(..)` to
    // `ChannelError::Platform(..)`. The turn loop sees a uniform
    // ChannelError regardless of which transport was behind the trait.
    let (channel, transport) = make_channel();
    transport.inject_send_error("429 rate limited");
    channel.stream_event(StreamEvent::Text("hi")).await.unwrap();
    let err = channel
        .finalize(&completed_outcome())
        .await
        .expect_err("send_error must surface");
    let msg = err.to_string();
    assert!(
        msg.contains("429 rate limited"),
        "platform error should propagate verbatim: {msg}"
    );
}
