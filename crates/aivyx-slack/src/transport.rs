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

/// Production transport. **Live wiring closed the Phase 108
/// Task 3 carve-out** at Phase 111 Task 4. The
/// callback-state-passing design that the Phase 108 doc
/// named (route the mpsc sender through
/// `SlackClientEventsUserState`) lives in
/// [`Self::connect`] below.
///
/// ## How the callback bridge works
///
/// slack-morphism's Socket Mode callbacks are `fn`-pointer-
/// shaped — they can't capture closures with state. The SDK
/// solves this with a typed `UserState` registry: at
/// listener construction, the caller registers state values
/// keyed by their type; callbacks retrieve them via the
/// `_states` parameter.
///
/// The Phase 111 live wiring constructs an mpsc channel,
/// registers the sender as a typed user-state value
/// (`SlackSenderState`), spawns the listener's background
/// task, and exposes the receiver through `next_message`.
/// The callback retrieves `SlackSenderState` from the
/// registry on each event and pushes parsed
/// `IncomingMessage`s into the channel.
///
/// `next_message` blocks on `recv().await`; `send_message`
/// uses the live `twilight_http`-equivalent
/// `slack-morphism` Web API session.
pub struct SlackMorphismTransport {
    /// REST client owned for `chat.postMessage` calls. `Arc`
    /// so the Socket Mode background task and `send_message`
    /// share one client.
    client: std::sync::Arc<
        slack_morphism::SlackClient<
            slack_morphism::hyper_tokio::SlackClientHyperHttpsConnector,
        >,
    >,
    /// Bot token used for REST calls (`chat.postMessage`,
    /// any other Web API surfaces a future phase wires).
    bot_token: slack_morphism::SlackApiToken,
    /// Inbound message receiver. The Socket Mode callback
    /// pushes parsed `IncomingMessage`s into this channel
    /// via the `SlackSenderState` user-state; `next_message`
    /// receives from it. `Mutex<...>` because
    /// `Receiver::recv` takes `&mut self` and the trait
    /// method takes `&self`; one consumer per transport
    /// instance, so contention is trivial.
    rx: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<IncomingMessage>>,
}

/// Wrapper type registered as a slack-morphism user-state
/// value. Carries the mpsc sender the callback pushes into.
/// A distinct newtype (not just `mpsc::Sender<_>` directly)
/// makes the user-state retrieval `states.read::<SlackSenderState>()`
/// unambiguous.
#[derive(Clone)]
struct SlackSenderState {
    tx: tokio::sync::mpsc::Sender<IncomingMessage>,
}

impl SlackMorphismTransport {
    /// Construct a production transport. Opens the Socket
    /// Mode connection, spawns the SDK's listener background
    /// task, and returns a handle ready to answer
    /// `next_message` and `send_message`.
    ///
    /// `bot_token_raw` is the `xoxb-...` token used for REST
    /// calls. `app_token_raw` is the `xapp-...` app-level
    /// token used to authenticate the Socket Mode WebSocket
    /// connection.
    pub async fn connect(
        bot_token_raw: &str,
        app_token_raw: &str,
    ) -> Result<Self, TransportError> {
        use slack_morphism::prelude::*;

        let connector = SlackClientHyperConnector::new().map_err(|e| {
            TransportError::Platform(format!("hyper connector build: {e}"))
        })?;
        let client = std::sync::Arc::new(SlackClient::new(connector));

        let bot_token: SlackApiToken =
            SlackApiToken::new(SlackApiTokenValue(bot_token_raw.to_string()));
        let app_token: SlackApiToken =
            SlackApiToken::new(SlackApiTokenValue(app_token_raw.to_string()));

        // mpsc adapter: callback pushes; next_message pops.
        // Capacity matches aivyx-discord's outer multiplexer
        // mailbox bound.
        let (tx, rx) = tokio::sync::mpsc::channel::<IncomingMessage>(64);
        let sender_state = SlackSenderState { tx };

        // Build the Socket Mode listener environment with the
        // mpsc sender registered as user state. The callback
        // retrieves the sender via
        // `states.read::<SlackSenderState>().await`.
        let listener_environment = std::sync::Arc::new(
            SlackClientEventsListenerEnvironment::new(client.clone())
                .with_error_handler(|err, _client, _states| {
                    eprintln!("aivyx-slack: socket-mode error: {err:?}");
                    http::StatusCode::BAD_REQUEST
                })
                .with_user_state(sender_state),
        );

        let callbacks = SlackSocketModeListenerCallbacks::new()
            .with_push_events(push_event_callback);

        let socket_mode_listener = SlackClientSocketModeListener::new(
            &SlackClientSocketModeConfig::new(),
            listener_environment,
            callbacks,
        );

        // Start the Socket Mode connection. This resolves
        // once the connect handshake completes; the actual
        // event loop runs in a background task spawned
        // below.
        socket_mode_listener
            .listen_for(&app_token)
            .await
            .map_err(|e| {
                TransportError::Platform(format!("socket-mode listen_for: {e}"))
            })?;

        tokio::spawn(async move {
            socket_mode_listener.serve().await;
        });

        Ok(SlackMorphismTransport {
            client,
            bot_token,
            rx: tokio::sync::Mutex::new(rx),
        })
    }
}

/// `with_push_events` callback. Static `fn` pointer (no
/// captures); state passes through the `_states` registry.
/// Retrieves the [`SlackSenderState`] registered at listener
/// construction, parses the `MessageCreate`-equivalent
/// event, and pushes an [`IncomingMessage`] into the mpsc
/// adapter.
// slack-morphism's `with_push_events` takes a fn-pointer with
// the exact signature below. The boxed-future return type is
// the SDK's own — type_complexity here is unavoidable.
#[allow(clippy::type_complexity)]
fn push_event_callback(
    event: slack_morphism::prelude::SlackPushEventCallback,
    _client: std::sync::Arc<slack_morphism::hyper_tokio::SlackHyperClient>,
    states: slack_morphism::prelude::SlackClientEventsUserState,
) -> std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = Result<
                    (),
                    Box<dyn std::error::Error + Send + Sync + 'static>,
                >,
            > + Send
            + 'static,
    >,
> {
    use slack_morphism::prelude::*;

    Box::pin(async move {
        // Only the Message event variant is interesting for
        // the agent turn loop. Everything else (app mentions
        // we already see as messages, presence updates, team
        // joins, etc.) is drained silently.
        let SlackEventCallbackBody::Message(msg_event) = event.event else {
            return Ok(());
        };

        // Skip bot-authored messages so the agent doesn't
        // reply to its own output.
        if msg_event.sender.bot_id.is_some() {
            return Ok(());
        }

        // Pull the five fields IncomingMessage carries. Slack's
        // message envelope is richer than this — origin
        // carries thread_ts, the content has blocks, etc. —
        // but Phase 108's foundation scope (Q4a) deliberately
        // narrowed to plain text + IDs.
        let team_id = event.team_id.0;
        let channel_id = match msg_event.origin.channel {
            Some(c) => c.0,
            None => return Ok(()),
        };
        let user_id = match msg_event.sender.user {
            Some(u) => u.0,
            None => return Ok(()),
        };
        let text = match msg_event.content.and_then(|c| c.text) {
            Some(t) => t,
            None => return Ok(()),
        };
        if text.is_empty() {
            return Ok(());
        }
        let message_ts = msg_event.origin.ts.0;

        // Retrieve the sender from the user-state registry
        // the listener was constructed with.
        let states_read = states.read().await;
        let sender_state = match states_read.get_user_state::<SlackSenderState>() {
            Some(s) => s.clone(),
            None => {
                eprintln!(
                    "aivyx-slack: SlackSenderState missing from user-state — \
                     listener-environment construction must register it"
                );
                return Ok(());
            }
        };
        drop(states_read);

        // Push best-effort. If the receiver has been dropped
        // (transport going down), silently discard rather
        // than failing the callback — the SDK error handler
        // logs at the listener layer.
        let _ = sender_state
            .tx
            .send(IncomingMessage {
                team_id,
                channel_id,
                user_id,
                text,
                message_ts,
            })
            .await;

        Ok(())
    })
}

#[async_trait]
impl SlackTransport for SlackMorphismTransport {
    async fn next_message(&self) -> Result<IncomingMessage, TransportError> {
        let mut rx = self.rx.lock().await;
        rx.recv().await.ok_or_else(|| {
            TransportError::Platform(
                "socket-mode sender dropped; SDK listener task exited".to_string(),
            )
        })
    }

    async fn send_message(&self, msg: OutgoingMessage) -> Result<(), TransportError> {
        use slack_morphism::prelude::*;

        let session = self.client.open_session(&self.bot_token);
        let request = SlackApiChatPostMessageRequest::new(
            SlackChannelId(msg.channel_id),
            SlackMessageContent::new().with_text(msg.text),
        );
        session
            .chat_post_message(&request)
            .await
            .map_err(|e| TransportError::Platform(format!("chat.postMessage: {e}")))?;
        Ok(())
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
