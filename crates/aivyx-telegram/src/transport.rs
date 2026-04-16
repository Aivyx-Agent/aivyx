//! Transport seam between [`TelegramChannel`](super::TelegramChannel)
//! and the Bot API.
//!
//! The real implementation ([`ReqwestTransport`]) is a thin wrapper
//! around `frankenstein::client_reqwest::Bot`. The point of the
//! [`TelegramTransport`] trait is to let unit tests swap in a scripted
//! double without touching the network — see `tests::ScriptedTransport`.
//!
//! ## Why an internal trait rather than a generic over `AsyncTelegramApi`
//!
//! `frankenstein` already defines an `AsyncTelegramApi` trait that the
//! reqwest client implements. We could generic the channel over that
//! directly and skip this wrapper. We *don't*, for two reasons:
//!
//! 1. **Surface narrowing.** `AsyncTelegramApi` has ~90 methods. The
//!    channel needs exactly two (get_updates, send_message). Defining
//!    our own trait with just those two pins the surface, makes the
//!    test double trivially small, and means a `frankenstein` bump
//!    that adds a method can't accidentally break our test fakes.
//! 2. **Error-shape collapse.** `frankenstein`'s errors wrap both
//!    HTTP failures and Telegram API errors in one enum; we collapse
//!    that to a single [`TransportError::Platform`] string at the
//!    seam so the channel's error handling is uniform. Callers inside
//!    `TelegramChannel::listen` never need to match on a
//!    frankenstein-specific variant.

use async_trait::async_trait;
use thiserror::Error;

/// One outgoing Telegram update the channel has already reduced to
/// its post-`SendMessage`-params shape. Keeping this as a plain
/// `(chat_id, text)` pair rather than a `frankenstein::SendMessageParams`
/// makes the scripted transport's capture buffer directly inspectable
/// in tests without pulling `frankenstein` types into assertion code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingMessage {
    pub chat_id: i64,
    pub text: String,
}

/// One inbound update the channel wants to route into the turn loop.
/// Only fields the channel actually consumes are kept — `message_id`,
/// `chat_id`, `user_id`, and the user's text. Everything else
/// (entities, attachments, forwarded_from, edit_date, ...) is
/// deliberately dropped at the transport boundary so the channel's
/// turn-loop glue has one obvious shape to handle.
///
/// Task 1 only constructs this from the production `ReqwestTransport`
/// (not exercised by tests) and from `ScriptedTransport::push_update`
/// (also `#[allow(dead_code)]` until task 2). The #[allow] is the
/// minimum scoped annotation so clippy's -D warnings still catches
/// anything newly orphaned elsewhere in the module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingMessage {
    pub update_id: i64,
    pub chat_id: i64,
    pub user_id: i64,
    pub text: String,
}

#[derive(Debug, Error)]
pub enum TransportError {
    /// A Telegram API call failed — network error, 4xx from Bot API,
    /// deserialization mismatch, or a platform-side rate-limit. The
    /// string is diagnostic only; callers treat it as opaque.
    #[error("telegram transport platform error: {0}")]
    Platform(String),
}

#[async_trait]
pub trait TelegramTransport: Send + Sync {
    /// Long-poll the Bot API for new updates. The `offset` is the
    /// "all updates with id >= offset" cursor — callers pass the
    /// last-seen `update_id + 1` to acknowledge everything before it.
    /// `timeout_secs` is forwarded to Bot API `getUpdates?timeout=…`;
    /// the server holds the request open up to that many seconds
    /// before returning an empty list.
    ///
    /// Task 1 has no inbound-routing caller yet — task 4's `listen()`
    /// loop is where this becomes load-bearing. The allow is on the
    /// trait method because the test `ScriptedTransport::get_updates`
    /// impl is also currently unused at runtime.
    async fn get_updates(
        &self,
        offset: i64,
        timeout_secs: u32,
    ) -> Result<Vec<IncomingMessage>, TransportError>;

    /// Send one text message to a chat. Text-only by design — Phase 8
    /// explicitly defers rich media (photos, files, inline keyboards)
    /// per PHASE_8.md's non-goals list.
    async fn send_message(&self, msg: OutgoingMessage) -> Result<(), TransportError>;
}

// ---------------------------------------------------------------------------
// Production impl — frankenstein + reqwest
// ---------------------------------------------------------------------------

/// Real transport: wraps a `frankenstein::client_reqwest::Bot` and
/// adapts its types to our narrow `IncomingMessage` / `OutgoingMessage`
/// shape.
///
/// Phase 8 Task 1 ships the *wiring* but **no Task 1 unit test
/// exercises this type**. It exists so the trait has a real consumer
/// and the `frankenstein` dep is a legitimate compile target — that
/// catches "we're importing a crate we don't actually use" at build
/// time rather than at phase exit. A later Task 7 smoke test against
/// a real bot token is what actually covers this code path.
pub struct ReqwestTransport {
    bot: frankenstein::client_reqwest::Bot,
}

impl ReqwestTransport {
    pub fn new(token: &str) -> Self {
        ReqwestTransport {
            bot: frankenstein::client_reqwest::Bot::new(token),
        }
    }
}

#[async_trait]
impl TelegramTransport for ReqwestTransport {
    async fn get_updates(
        &self,
        offset: i64,
        timeout_secs: u32,
    ) -> Result<Vec<IncomingMessage>, TransportError> {
        use frankenstein::AsyncTelegramApi;
        use frankenstein::methods::GetUpdatesParams;

        let params = GetUpdatesParams::builder()
            .offset(offset)
            .timeout(timeout_secs)
            .build();

        let response = self
            .bot
            .get_updates(&params)
            .await
            .map_err(|e| TransportError::Platform(format!("get_updates: {e}")))?;

        Ok(response
            .result
            .into_iter()
            .filter_map(|u| {
                // Only route `message` updates with text content. All
                // other update kinds (callback_query, edited_message,
                // channel_post, inline_query, ...) are silently skipped
                // at the transport boundary — they're out of scope for
                // Phase 8's "text turn in, text turn out" shape.
                //
                // `frankenstein 0.49` flattens update variants into
                // an `UpdateContent` enum; we pattern-match the
                // `Message` arm and drop the others.
                let update_id = u.update_id as i64;
                let msg = match u.content {
                    frankenstein::updates::UpdateContent::Message(m) => *m,
                    _ => return None,
                };
                let text = msg.text?;
                let from = msg.from?;
                Some(IncomingMessage {
                    update_id,
                    chat_id: msg.chat.id,
                    user_id: from.id as i64,
                    text,
                })
            })
            .collect())
    }

    async fn send_message(&self, msg: OutgoingMessage) -> Result<(), TransportError> {
        use frankenstein::AsyncTelegramApi;
        use frankenstein::methods::SendMessageParams;

        let params = SendMessageParams::builder()
            .chat_id(msg.chat_id)
            .text(msg.text)
            .build();

        self.bot
            .send_message(&params)
            .await
            .map_err(|e| TransportError::Platform(format!("send_message: {e}")))?;
        Ok(())
    }
}
