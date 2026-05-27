//! Transport seam between [`SlackChannel`](super::slack_channel::SlackChannel)
//! and the Slack APIs.
//!
//! Mirrors the Phase 8 `aivyx-telegram` and Phase 107
//! `aivyx-discord` transport-trait patterns: a private trait
//! with the minimum methods the channel actually calls, a thin
//! production wrapper around the SDK
//! (`slack-morphism` Socket Mode + REST), and a scripted
//! double for unit tests.
//!
//! ## Why an internal trait rather than a generic over slack-morphism's API
//!
//! Same rationale as Telegram + Discord
//! (`docs/ADAPTER_PATTERN.md`'s private-transport-trait
//! section): pin the surface to exactly what the channel
//! calls so the test double stays tiny, the SDK error shape
//! collapses to a single `TransportError::Platform(String)`,
//! and no slack-morphism type leaks into `SlackChannel`'s
//! method signatures.
//!
//! ## Callback-to-pull adapter
//!
//! Slack-morphism's Socket Mode is **callback-based** — the
//! SDK runs an event loop and invokes operator-supplied
//! callbacks for each message. Aivyx's
//! [`SlackTransport::next_message`] is async-pull-based to
//! match the Telegram and Discord precedents. The
//! [`SlackMorphismTransport`] production impl bridges the
//! two with a `tokio::sync::mpsc` channel: the SDK callback
//! sends each parsed message into the channel; `next_message`
//! receives from it.

use async_trait::async_trait;
use thiserror::Error;

// ---------------------------------------------------------------------------
// Message types
// ---------------------------------------------------------------------------

/// One outgoing Slack message the channel has already reduced
/// to a flat shape — `(channel_id, text)` — matching the Phase
/// 8 / Phase 107 `OutgoingMessage` pattern. Slack channel IDs
/// are string snowflakes (`C0123456789` for channels,
/// `D0123456789` for DMs), so the field is `String` rather
/// than `u64`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingMessage {
    pub channel_id: String,
    pub text: String,
}

/// One inbound Slack message the channel wants to route into
/// the turn loop. Only fields the channel actually consumes
/// are kept — `team_id`, `channel_id`, `user_id`, `text`, and
/// `message_ts` (the message timestamp used as Slack's
/// per-message identifier). Threads, Block Kit components,
/// attachments, reactions, mention-parsing — everything else
/// slack-morphism exposes is deliberately dropped at the
/// transport boundary.
///
/// `team_id` is the load-bearing addition vs. the Discord
/// shape: Phase 108 Q3a chose to stringify
/// `(team_id, channel_id)` as the partition key, which
/// confirms the three-data-point `Option<String>` partition
/// pattern at four data points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingMessage {
    pub team_id: String,
    pub channel_id: String,
    pub user_id: String,
    pub text: String,
    pub message_ts: String,
}

impl IncomingMessage {
    /// Stable per-channel partition key. Returns the Q3a-
    /// resolved `"{team_id}:{channel_id}"` form used by
    /// `SlackChannel::session_partition` and by the outer
    /// multiplexer's per-channel mailbox routing key.
    pub fn partition_key(&self) -> String {
        format!("{}:{}", self.team_id, self.channel_id)
    }
}

/// Transport-layer error. Matches the Telegram + Discord
/// precedent: one opaque `Platform(String)` variant for any
/// failure originating in the SDK or network layer.
#[derive(Debug, Error)]
pub enum TransportError {
    #[error("slack transport platform error: {0}")]
    Platform(String),
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// The private transport seam. `pub(crate)` so the trait
/// itself stays an implementation detail; downstream code
/// constructs either a [`SlackMorphismTransport`] or a
/// `ScriptedTransport` (in tests) and hands it to
/// `SlackChannel` via a `Box<dyn SlackTransport>`.
///
/// Two methods, matching `aivyx-discord::DiscordTransport`'s
/// arity:
///
/// - [`Self::next_message`] returns the next inbound message
///   from Socket Mode. Production blocks on the mpsc
///   adapter-channel that the SDK callbacks push into;
///   scripted pops from a queued `VecDeque` and
///   `tokio::time::sleep`s on drain.
///
/// - [`Self::send_message`] issues one
///   `chat.postMessage` REST call against Slack's Web API.
#[async_trait]
pub trait SlackTransport: Send + Sync {
    async fn next_message(&self) -> Result<IncomingMessage, TransportError>;
    async fn send_message(&self, msg: OutgoingMessage) -> Result<(), TransportError>;
}

// ---------------------------------------------------------------------------
// Production impl — slack-morphism Socket Mode + REST
// ---------------------------------------------------------------------------

/// Production transport stub. **Scoped down at Task 3 to a
/// compile-only target** because slack-morphism's Socket
/// Mode callback API is `fn`-pointer-shaped (callbacks
/// cannot capture mpsc senders directly; the SDK pushes
/// state through `SlackClientEventsUserState`). The right
/// API-discovery design is to route the mpsc sender through
/// a `UserState`-backed wrapper; that's a meaningful chunk
/// of slack-morphism-specific design work that does not
/// belong on the critical path of Task 3.
///
/// The pattern matches the Phase 107 deferral structure:
///
/// - Phase 107 Task 5 carved out the Discord daemon-frontend
///   variant for a focused follow-on.
/// - Phase 108 Task 3 carves out the `SlackMorphismTransport`
///   production wiring for the same follow-on bundle.
///
/// Both deferrals land together when an operator wants
/// live-bot smoke testing; the Channel Activation Milestone
/// is the natural home for the real-network exercises that
/// would catch any callback-state-passing bug.
///
/// At Task 3 the stub serves three roles:
/// 1. Keeps the slack-morphism dep a legitimate compile
///    target so the workspace catches version-conflict and
///    feature-flag mistakes.
/// 2. Pins the public surface (`connect`, `next_message`,
///    `send_message`) so the follow-on wiring is a fill-in,
///    not a refactor.
/// 3. Returns an explicit `TransportError::Platform` with a
///    "production transport not yet wired" message so an
///    operator who lands at the Slack dispatch arm before
///    the follow-on ships gets a precise error instead of
///    a panic.
pub struct SlackMorphismTransport {
    _bot_token: String,
    _app_token: String,
}

impl SlackMorphismTransport {
    /// Construct a production transport stub. Holds the
    /// supplied tokens so a future wiring pass has them
    /// ready, but does **not** open a Socket Mode
    /// connection yet — `next_message` and `send_message`
    /// return a deferral error.
    pub async fn connect(
        bot_token_raw: &str,
        app_token_raw: &str,
    ) -> Result<Self, TransportError> {
        Ok(SlackMorphismTransport {
            _bot_token: bot_token_raw.to_string(),
            _app_token: app_token_raw.to_string(),
        })
    }
}

#[async_trait]
impl SlackTransport for SlackMorphismTransport {
    async fn next_message(&self) -> Result<IncomingMessage, TransportError> {
        Err(TransportError::Platform(
            "production SlackMorphismTransport not yet wired — \
             callback-state-passing via SlackClientEventsUserState is the \
             Phase-108-internal deferral bundled with the Phase 107 \
             daemon-frontend follow-on. Use ScriptedTransport for tests."
                .to_string(),
        ))
    }

    async fn send_message(&self, _msg: OutgoingMessage) -> Result<(), TransportError> {
        Err(TransportError::Platform(
            "production SlackMorphismTransport not yet wired — see next_message".to_string(),
        ))
    }
}

// ---------------------------------------------------------------------------
// Scripted double — lives in this file rather than `tests.rs`
// for the same Rust-visibility reason aivyx-discord put it
// here.
// ---------------------------------------------------------------------------

/// Deterministic test double: pops queued inbound messages,
/// captures outbound sends, and sleeps on drain so the outer
/// loop doesn't hot-spin.
///
/// Test code constructs one with [`Self::with_queue`], hands
/// it to `SlackChannel` (or directly to the session driver),
/// drives the session, then inspects [`Self::sent`] to assert
/// the agent's outgoing message hit the wire.
pub struct ScriptedTransport {
    queue: tokio::sync::Mutex<std::collections::VecDeque<IncomingMessage>>,
    sent: tokio::sync::Mutex<Vec<OutgoingMessage>>,
    drain_sleep: std::time::Duration,
}

impl ScriptedTransport {
    /// New scripted transport pre-loaded with `queue` of
    /// inbound messages.
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

    /// Push another inbound message into the queue at
    /// runtime — used by tests that script a follow-up
    /// message after the first turn completes.
    pub async fn push_inbound(&self, msg: IncomingMessage) {
        self.queue.lock().await.push_back(msg);
    }
}

#[async_trait]
impl SlackTransport for ScriptedTransport {
    async fn next_message(&self) -> Result<IncomingMessage, TransportError> {
        loop {
            {
                let mut q = self.queue.lock().await;
                if let Some(msg) = q.pop_front() {
                    return Ok(msg);
                }
            }
            tokio::time::sleep(self.drain_sleep).await;
        }
    }

    async fn send_message(&self, msg: OutgoingMessage) -> Result<(), TransportError> {
        self.sent.lock().await.push(msg);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests — pure shape + scripted-double behavior. End-to-end
// coverage that drives a `SlackChannel` against this scripted
// impl lands at Task 6.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod transport_tests {
    use super::*;

    fn sample_inbound(
        team_id: &str,
        channel_id: &str,
        text: &str,
    ) -> IncomingMessage {
        IncomingMessage {
            team_id: team_id.to_string(),
            channel_id: channel_id.to_string(),
            user_id: "U001".to_string(),
            text: text.to_string(),
            message_ts: "1700000000.000100".to_string(),
        }
    }

    #[test]
    fn partition_key_joins_team_and_channel_with_colon() {
        let m = sample_inbound("T01", "C42", "hi");
        assert_eq!(m.partition_key(), "T01:C42");
    }

    #[test]
    fn partition_key_handles_empty_team_id() {
        // Defense: an SDK regression that drops team_id
        // should produce a partition key that still parses
        // (the colon separator stays present so downstream
        // log/audit consumers can detect the empty half).
        let m = sample_inbound("", "C42", "hi");
        assert_eq!(m.partition_key(), ":C42");
    }

    #[tokio::test]
    async fn scripted_next_message_pops_queue_in_order() {
        let t = ScriptedTransport::with_queue(vec![
            sample_inbound("T01", "C1", "first"),
            sample_inbound("T01", "C1", "second"),
        ]);
        let first = t.next_message().await.unwrap();
        assert_eq!(first.text, "first");
        let second = t.next_message().await.unwrap();
        assert_eq!(second.text, "second");
    }

    #[tokio::test]
    async fn scripted_send_message_captures_outbound() {
        let t = ScriptedTransport::with_queue(vec![]);
        t.send_message(OutgoingMessage {
            channel_id: "C42".to_string(),
            text: "hello".to_string(),
        })
        .await
        .unwrap();
        let captured = t.sent().await;
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].channel_id, "C42");
        assert_eq!(captured[0].text, "hello");
    }

    #[tokio::test]
    async fn scripted_push_inbound_adds_to_queue_runtime() {
        let t = ScriptedTransport::with_queue(vec![sample_inbound(
            "T01", "C1", "first",
        )]);
        let first = t.next_message().await.unwrap();
        assert_eq!(first.text, "first");

        t.push_inbound(sample_inbound("T01", "C1", "second")).await;
        let second = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            t.next_message(),
        )
        .await
        .expect("push_inbound should make next_message return promptly")
        .unwrap();
        assert_eq!(second.text, "second");
    }

    #[tokio::test]
    async fn scripted_empty_queue_sleeps_then_retries() {
        let t = ScriptedTransport::with_queue(vec![]);
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(60),
            t.next_message(),
        )
        .await;
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
