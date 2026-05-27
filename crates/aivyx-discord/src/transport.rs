//! Transport seam between [`DiscordChannel`](super::discord_channel::DiscordChannel)
//! and the Discord Bot API.
//!
//! Mirrors the Phase 8 `aivyx-telegram` transport-trait pattern:
//! a private trait with the minimum methods the channel
//! actually calls, a thin production wrapper around the SDK
//! (`twilight-gateway` + `twilight-http`), and a scripted
//! double for unit tests.
//!
//! ## Why an internal trait rather than a generic over twilight's API
//!
//! Same rationale as Telegram (`docs/ADAPTER_PATTERN.md`'s
//! private-transport-trait section). twilight exposes a large
//! event-stream + REST surface; the channel needs exactly
//! two operations — "wait for the next inbound message" and
//! "send one outbound text message." Pinning the surface to
//! those two means:
//!
//! 1. **Surface narrowing.** A twilight API addition cannot
//!    accidentally break the test double. The double stays
//!    ~50 lines instead of ~500.
//! 2. **Error-shape collapse.** twilight's errors wrap WebSocket
//!    failures, gateway protocol errors, REST 4xx/5xx, rate
//!    limits, deserialization issues. The seam collapses all of
//!    that to a single [`TransportError::Platform`] string so
//!    the channel's error handling is uniform.
//! 3. **Dependency hygiene.** No twilight type ever appears in
//!    `DiscordChannel`'s method signatures; the public surface
//!    stays SDK-agnostic.

use async_trait::async_trait;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Message types
// ---------------------------------------------------------------------------

/// One outgoing Discord message the channel has already reduced
/// to a `(channel_id, text)` pair — matching the Phase 8
/// `OutgoingMessage` shape. The plain integer + string form
/// keeps the scripted transport's capture buffer directly
/// inspectable in tests without pulling `twilight_model` into
/// assertion code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingMessage {
    /// Discord channel snowflake (DM channel or guild text
    /// channel). Stored as `u64` because Discord IDs are
    /// 64-bit unsigned integers; `twilight_model::id::Id<…>`
    /// values trivially convert to `u64`.
    pub channel_id: u64,
    pub text: String,
}

/// One inbound Discord message the channel wants to route into
/// the turn loop. Only the fields the channel actually consumes
/// are kept — `message_id`, `channel_id`, `author_id`, and the
/// message text. Embeds, attachments, reactions, threads, guild
/// context — everything else twilight exposes is deliberately
/// dropped at the transport boundary so the channel's turn-loop
/// glue has one obvious shape to handle.
///
/// Image / attachment support (parallel to Phase 45's Telegram
/// photo extraction) is a deferred extension; the Phase 107
/// `Q2c` full-parity scope can choose to add it during Task 5
/// or punt to a follow-on phase based on operator pressure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingMessage {
    pub message_id: u64,
    pub channel_id: u64,
    pub author_id: u64,
    pub text: String,
}

/// Transport-layer error. Matches the Telegram precedent: one
/// opaque `Platform(String)` variant for any failure originating
/// in the SDK or network layer. Callers treat the inner string
/// as diagnostic-only; the session driver does not match on it
/// beyond logging.
#[derive(Debug, Error)]
pub enum TransportError {
    #[error("discord transport platform error: {0}")]
    Platform(String),
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// The private transport seam. `pub(crate)` so the trait itself
/// stays an implementation detail; downstream code constructs
/// either a [`TwilightTransport`] or a `ScriptedTransport` (in
/// tests) and hands it to `DiscordChannel` via a
/// `Box<dyn DiscordTransport>`.
///
/// Two methods, matching `aivyx-telegram::TelegramTransport`'s
/// arity:
///
/// - [`Self::next_message`] returns the next inbound
///   `MessageCreate` event the bot sees. Production blocks
///   on the Gateway WebSocket until a real message arrives
///   (heartbeats / acks / presence updates / non-message
///   events drain internally and never surface). Scripted
///   pops from a queued `VecDeque` and `tokio::time::sleep`s
///   on drain so the outer loop doesn't hot-spin.
///
/// - [`Self::send_message`] issues one
///   `POST /channels/{id}/messages` REST call. Both impls
///   surface failures as `TransportError::Platform`.
#[async_trait]
pub trait DiscordTransport: Send + Sync {
    async fn next_message(&self) -> Result<IncomingMessage, TransportError>;
    async fn send_message(&self, msg: OutgoingMessage) -> Result<(), TransportError>;
}

// ---------------------------------------------------------------------------
// Production impl — twilight-gateway + twilight-http
// ---------------------------------------------------------------------------

/// Real transport: holds a `twilight_gateway::Shard` for inbound
/// Gateway events and a `twilight_http::Client` for outbound REST
/// calls. The Shard's state machine (identify, heartbeat,
/// sequence tracking, resume) lives inside twilight; we consume
/// it via the `Shard::next_event` polling loop.
///
/// Like `aivyx-telegram::ReqwestTransport`, **no Task 3 unit
/// test exercises this type** — its job is to be a real consumer
/// of the trait so the twilight dep is a legitimate compile
/// target. The scripted-double tests prove the trait shape; the
/// real-protocol smoke test runs at the Channel Activation
/// Milestone per `docs/ADAPTER_PATTERN.md` checklist item 7.
pub struct TwilightTransport {
    /// Gateway shard. `Mutex<…>` because `Shard::next_event`
    /// takes `&mut self` and the trait method takes `&self`;
    /// we hold the mutex across the await inside
    /// `next_message`, which is fine because there is exactly
    /// one Gateway stream and one reader per
    /// `TwilightTransport` instance.
    shard: tokio::sync::Mutex<twilight_gateway::Shard>,
    /// REST client. `Arc` for cheap clone; `Client` is already
    /// internally thread-safe so we don't need a mutex around
    /// it.
    http: std::sync::Arc<twilight_http::Client>,
}

impl TwilightTransport {
    /// Construct a production transport from a bot token. The
    /// Gateway shard is built but **not yet connected** — twilight
    /// connects lazily on the first `next_event` call, so the
    /// constructor is synchronous and infallible.
    ///
    /// `intents` is fixed to the union the channel actually needs:
    /// `GUILDS | GUILD_MESSAGES | DIRECT_MESSAGES |
    /// MESSAGE_CONTENT`. The `MESSAGE_CONTENT` intent is gated
    /// behind a developer-portal toggle for bots in 100+ servers;
    /// operators running a private bot get it without setup.
    pub fn new(token: impl Into<String>) -> Self {
        use twilight_gateway::{Intents, Shard, ShardId};

        let token = token.into();
        let intents = Intents::GUILDS
            | Intents::GUILD_MESSAGES
            | Intents::DIRECT_MESSAGES
            | Intents::MESSAGE_CONTENT;

        let config = twilight_gateway::Config::new(token.clone(), intents);
        let shard = Shard::with_config(ShardId::ONE, config);
        let http = std::sync::Arc::new(twilight_http::Client::new(token));

        TwilightTransport {
            shard: tokio::sync::Mutex::new(shard),
            http,
        }
    }
}

#[async_trait]
impl DiscordTransport for TwilightTransport {
    async fn next_message(&self) -> Result<IncomingMessage, TransportError> {
        // `next_event` is provided by the `StreamExt`
        // extension trait in twilight 0.16; bring it into
        // scope for the loop below.
        use twilight_gateway::{Event, StreamExt};

        let mut shard = self.shard.lock().await;
        loop {
            let event = match shard.next_event(twilight_gateway::EventTypeFlags::all()).await {
                Some(Ok(e)) => e,
                Some(Err(e)) => {
                    return Err(TransportError::Platform(format!(
                        "gateway: {e}"
                    )));
                }
                None => {
                    return Err(TransportError::Platform(
                        "gateway: stream ended (shard closed)".to_string(),
                    ));
                }
            };

            match event {
                Event::MessageCreate(msg) => {
                    // Drop bot-authored messages so the agent
                    // doesn't try to reply to its own output.
                    if msg.author.bot {
                        continue;
                    }
                    return Ok(IncomingMessage {
                        message_id: msg.id.get(),
                        channel_id: msg.channel_id.get(),
                        author_id: msg.author.id.get(),
                        text: msg.content.clone(),
                    });
                }
                // Every other event type (Ready, GuildCreate,
                // PresenceUpdate, TypingStart, ...) is drained
                // silently. twilight handles heartbeats and
                // sequence tracking internally.
                _ => continue,
            }
        }
    }

    async fn send_message(&self, msg: OutgoingMessage) -> Result<(), TransportError> {
        use twilight_model::id::{Id, marker::ChannelMarker};

        let channel_id: Id<ChannelMarker> = Id::new(msg.channel_id);
        self.http
            .create_message(channel_id)
            .content(&msg.text)
            .await
            .map_err(|e| TransportError::Platform(format!("create_message: {e}")))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Scripted double — lives in this file rather than `tests.rs`
// because the channel's unit tests in `discord_channel.rs` need
// it, and Rust visibility rules don't let a `mod tests;`
// sibling-of-channel re-export `pub(crate)` items.
// ---------------------------------------------------------------------------

/// Deterministic test double: pops queued inbound messages,
/// captures outbound sends, and sleeps a short interval on
/// drain so the outer loop doesn't hot-spin.
///
/// Test code constructs one with [`Self::with_queue`], hands it
/// to `DiscordChannel`, drives the session, then inspects
/// [`Self::sent`] to assert the agent's outgoing message hit
/// the wire.
pub struct ScriptedTransport {
    /// Queued inbound messages, popped front-to-back on each
    /// `next_message` call.
    queue: tokio::sync::Mutex<std::collections::VecDeque<IncomingMessage>>,
    /// Capture buffer for outbound messages.
    sent: tokio::sync::Mutex<Vec<OutgoingMessage>>,
    /// Sleep duration when the queue is empty. Short enough
    /// not to make tests slow; long enough that a tight
    /// `tokio::select!` consumer doesn't burn CPU.
    drain_sleep: std::time::Duration,
}

impl ScriptedTransport {
    /// New scripted transport pre-loaded with `queue` of inbound
    /// messages.
    pub fn with_queue(queue: Vec<IncomingMessage>) -> Self {
        ScriptedTransport {
            queue: tokio::sync::Mutex::new(queue.into()),
            sent: tokio::sync::Mutex::new(Vec::new()),
            drain_sleep: std::time::Duration::from_millis(25),
        }
    }

    /// Snapshot of every outbound message captured so far.
    pub async fn sent(&self) -> Vec<OutgoingMessage> {
        self.sent.lock().await.clone()
    }

    /// Push another inbound message into the queue at runtime —
    /// used by the gate-resolve test which scripts a follow-up
    /// `/approve` after the first turn completes.
    pub async fn push_inbound(&self, msg: IncomingMessage) {
        self.queue.lock().await.push_back(msg);
    }
}

#[async_trait]
impl DiscordTransport for ScriptedTransport {
    async fn next_message(&self) -> Result<IncomingMessage, TransportError> {
        loop {
            {
                let mut q = self.queue.lock().await;
                if let Some(msg) = q.pop_front() {
                    return Ok(msg);
                }
            }
            // Queue empty — sleep briefly and retry. A test that
            // wants a clean shutdown wraps the channel's
            // `listen` loop in `tokio::time::timeout` rather
            // than relying on the scripted transport to return
            // an "end of stream" error.
            tokio::time::sleep(self.drain_sleep).await;
        }
    }

    async fn send_message(&self, msg: OutgoingMessage) -> Result<(), TransportError> {
        self.sent.lock().await.push(msg);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests — pure shape coverage. The trait + scripted-double
// behavior. End-to-end coverage that drives a `DiscordChannel`
// against this scripted impl lands at Task 6.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod transport_tests {
    use super::*;

    fn sample_inbound(id: u64, text: &str) -> IncomingMessage {
        IncomingMessage {
            message_id: id,
            channel_id: 100,
            author_id: 200,
            text: text.to_string(),
        }
    }

    #[tokio::test]
    async fn scripted_next_message_pops_queue_in_order() {
        let t = ScriptedTransport::with_queue(vec![
            sample_inbound(1, "first"),
            sample_inbound(2, "second"),
        ]);
        let first = t.next_message().await.unwrap();
        assert_eq!(first.message_id, 1);
        assert_eq!(first.text, "first");
        let second = t.next_message().await.unwrap();
        assert_eq!(second.message_id, 2);
        assert_eq!(second.text, "second");
    }

    #[tokio::test]
    async fn scripted_send_message_captures_outbound() {
        let t = ScriptedTransport::with_queue(vec![]);
        t.send_message(OutgoingMessage {
            channel_id: 100,
            text: "hello".to_string(),
        })
        .await
        .unwrap();
        let captured = t.sent().await;
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].channel_id, 100);
        assert_eq!(captured[0].text, "hello");
    }

    #[tokio::test]
    async fn scripted_push_inbound_adds_to_queue_runtime() {
        let t = ScriptedTransport::with_queue(vec![sample_inbound(1, "first")]);
        let first = t.next_message().await.unwrap();
        assert_eq!(first.message_id, 1);

        // Push a runtime-arrival message; next_message picks it
        // up after a short drain interval.
        t.push_inbound(sample_inbound(2, "second")).await;
        let second = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            t.next_message(),
        )
        .await
        .expect("push_inbound should make next_message return promptly")
        .unwrap();
        assert_eq!(second.message_id, 2);
    }

    #[tokio::test]
    async fn scripted_empty_queue_sleeps_then_retries() {
        // Verify the drain-sleep behavior by timing a bounded
        // next_message call against an empty queue. The call
        // should not return within the drain interval.
        let t = ScriptedTransport::with_queue(vec![]);
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(60),
            t.next_message(),
        )
        .await;
        // Expect timeout (not Ok) — empty queue keeps looping.
        assert!(
            result.is_err(),
            "empty queue should keep looping; got {result:?}",
        );
    }

    #[test]
    fn transport_error_display_includes_inner_string() {
        let err = TransportError::Platform("bad token".to_string());
        let s = format!("{err}");
        assert!(s.contains("bad token"));
    }
}
