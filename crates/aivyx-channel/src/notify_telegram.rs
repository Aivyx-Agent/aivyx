//! Phase 62 Task 5 — Telegram outbound notification backend.
//!
//! Wraps an `Arc<dyn TelegramTransport>` (typically a
//! `ReqwestTransport` constructed from the operator's
//! `[telegram] token`) with a per-target `chat_id` and implements
//! [`NotifyBackend`]. Multiple Telegram notification targets share
//! the same transport — Aivyx assumes one bot per deployment;
//! each `[[notify_target]] kind = "telegram"` entry just names a
//! different chat the same bot can reach.
//!
//! ## Subject rendering
//!
//! Per Q3(b) at Phase 62 sign-off, `subject` is optional. When
//! provided, it's prepended to the message body as bold Markdown
//! using Telegram's MarkdownV2 syntax:
//!
//! ```text
//! *Subject line*
//! body of message
//! ```
//!
//! Phase 8's existing `OutgoingMessage` shape is text-only with
//! no `parse_mode` field, so the Markdown is sent as plain text
//! today — Telegram clients render the asterisks literally. A
//! follow-up phase that adds `parse_mode` to the transport will
//! make the formatting render. The semantic content is correct
//! either way; it's a cosmetic-only deferral.

use std::sync::Arc;

use async_trait::async_trait;

use aivyx_telegram::transport::{OutgoingMessage, TelegramTransport, TransportError};

use crate::notify_dispatcher::{NotifyBackend, NotifyError};

/// Telegram-specific construction errors. The dispatcher only
/// hits these at startup; once a backend is registered, runtime
/// failures flow through [`NotifyError`].
#[derive(Debug, thiserror::Error)]
pub enum TelegramBackendError {
    /// The `chat_id` from `[[notify_target]] chat_id = "..."`
    /// wasn't a valid `i64`. The transport's `OutgoingMessage`
    /// uses `i64` per Bot API conventions; we surface this as a
    /// construction error rather than masking it until first
    /// send.
    #[error("invalid telegram chat_id `{value}`: {reason}")]
    InvalidChatId { value: String, reason: String },
}

/// Per-target Telegram backend. Phase 62 Task 5.
pub struct NotifyTelegramBackend {
    transport: Arc<dyn TelegramTransport>,
    chat_id: i64,
}

impl NotifyTelegramBackend {
    /// Construct from a shared transport and the target's
    /// declared chat_id string. The shared transport is
    /// expected to be a `ReqwestTransport` at runtime; the
    /// trait abstraction lets tests inject a scripted
    /// transport.
    pub fn new(
        transport: Arc<dyn TelegramTransport>,
        chat_id: &str,
    ) -> Result<Self, TelegramBackendError> {
        let chat_id_i64 = chat_id
            .parse::<i64>()
            .map_err(|e| TelegramBackendError::InvalidChatId {
                value: chat_id.to_string(),
                reason: e.to_string(),
            })?;
        Ok(Self {
            transport,
            chat_id: chat_id_i64,
        })
    }

    /// Render a notification with optional subject into a
    /// single message body. Public for testing.
    pub fn render_message(message: &str, subject: Option<&str>) -> String {
        match subject {
            Some(s) if !s.is_empty() => format!("*{}*\n{}", s, message),
            _ => message.to_string(),
        }
    }
}

#[async_trait]
impl NotifyBackend for NotifyTelegramBackend {
    async fn send(
        &self,
        message: &str,
        subject: Option<&str>,
    ) -> Result<(), NotifyError> {
        let text = Self::render_message(message, subject);
        let outgoing = OutgoingMessage {
            chat_id: self.chat_id,
            text,
        };
        self.transport
            .send_message(outgoing)
            .await
            .map_err(map_transport_error)
    }

    fn kind(&self) -> &'static str {
        "telegram"
    }
}

/// Telegram's `TransportError` is a single `Platform(String)`
/// variant today — every failure mode collapses into one string.
/// Until the transport surface gains finer-grained errors, we
/// classify here heuristically by substring inspection. The
/// classification is best-effort: when in doubt, we return
/// `Transport`, which the agent should treat as retryable.
fn map_transport_error(e: TransportError) -> NotifyError {
    let TransportError::Platform(msg) = e;
    let lower = msg.to_lowercase();
    if lower.contains("401") || lower.contains("unauthorized") {
        NotifyError::Auth(msg)
    } else if lower.contains("timeout") || lower.contains("timed out") {
        NotifyError::Timeout
    } else if let Some(status) = extract_status_code(&msg) {
        // 4xx (other than 401) → Rejected. 5xx → Transport
        // (retryable). The Bot API typically returns 4xx for
        // bad chat_id (400), oversized message (400), or rate
        // limit (429); 5xx are server-side outages.
        if (400..500).contains(&status) {
            NotifyError::Rejected(status)
        } else {
            NotifyError::Transport(msg)
        }
    } else {
        NotifyError::Transport(msg)
    }
}

/// Best-effort extraction of a 3-digit HTTP status code from a
/// diagnostic string. Returns the first `100..=599` integer
/// found. Used by [`map_transport_error`] to classify
/// transport-layer errors when the underlying error type is
/// stringly-typed.
fn extract_status_code(msg: &str) -> Option<u16> {
    let bytes = msg.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i].is_ascii_digit()
            && bytes[i + 1].is_ascii_digit()
            && bytes[i + 2].is_ascii_digit()
        {
            // Boundary check: digit must not be part of a longer run.
            let starts_run = i == 0 || !bytes[i - 1].is_ascii_digit();
            let ends_run = i + 3 == bytes.len() || !bytes[i + 3].is_ascii_digit();
            if starts_run && ends_run {
                let s = std::str::from_utf8(&bytes[i..i + 3]).ok()?;
                let n: u16 = s.parse().ok()?;
                if (100..=599).contains(&n) {
                    return Some(n);
                }
            }
        }
        i += 1;
    }
    None
}

// ---------------------------------------------------------------------------
// Tests — scripted-transport based, no real network.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_telegram::transport::IncomingMessage;
    use std::sync::Mutex;

    /// Scripted Telegram transport: records every `send_message`
    /// call and returns a caller-configurable outcome.
    struct ScriptedTransport {
        sent: Mutex<Vec<OutgoingMessage>>,
        outcome: Mutex<Result<(), TransportError>>,
    }

    impl ScriptedTransport {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                sent: Mutex::new(Vec::new()),
                outcome: Mutex::new(Ok(())),
            })
        }

        fn with_outcome(outcome: Result<(), TransportError>) -> Arc<Self> {
            Arc::new(Self {
                sent: Mutex::new(Vec::new()),
                outcome: Mutex::new(outcome),
            })
        }
    }

    #[async_trait]
    impl TelegramTransport for ScriptedTransport {
        async fn get_updates(
            &self,
            _offset: i64,
            _timeout_secs: u32,
        ) -> Result<Vec<IncomingMessage>, TransportError> {
            Ok(Vec::new())
        }

        async fn send_message(&self, msg: OutgoingMessage) -> Result<(), TransportError> {
            self.sent.lock().unwrap().push(msg);
            // Clone the outcome — `TransportError` doesn't impl
            // Clone, so we manually rebuild Platform variants.
            match &*self.outcome.lock().unwrap() {
                Ok(()) => Ok(()),
                Err(TransportError::Platform(s)) => Err(TransportError::Platform(s.clone())),
            }
        }
    }

    #[test]
    fn render_message_without_subject_returns_message_verbatim() {
        let out = NotifyTelegramBackend::render_message("hello", None);
        assert_eq!(out, "hello");
    }

    #[test]
    fn render_message_with_subject_prepends_bold_block() {
        let out = NotifyTelegramBackend::render_message("body line", Some("Alert"));
        assert_eq!(out, "*Alert*\nbody line");
    }

    #[test]
    fn render_message_with_empty_subject_treats_as_absent() {
        // An empty subject string should not produce a `**\n…`
        // leading block. Defensive — the tool's input schema
        // makes `subject` optional, but a model could send an
        // empty string.
        let out = NotifyTelegramBackend::render_message("body", Some(""));
        assert_eq!(out, "body");
    }

    #[test]
    fn invalid_chat_id_returns_construction_error() {
        let t = ScriptedTransport::new();
        let err = NotifyTelegramBackend::new(t, "not-a-number")
            .err()
            .expect("must error");
        match err {
            TelegramBackendError::InvalidChatId { value, .. } => {
                assert_eq!(value, "not-a-number");
            }
        }
    }

    #[tokio::test]
    async fn send_dispatches_outgoing_message_to_transport() {
        let t = ScriptedTransport::new();
        let backend =
            NotifyTelegramBackend::new(t.clone(), "123456789").expect("valid chat_id");
        backend.send("hello", None).await.expect("send ok");

        let sent = t.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].chat_id, 123456789);
        assert_eq!(sent[0].text, "hello");
    }

    #[tokio::test]
    async fn send_renders_subject_into_message_body() {
        let t = ScriptedTransport::new();
        let backend =
            NotifyTelegramBackend::new(t.clone(), "111").expect("valid chat_id");
        backend.send("body", Some("Subject")).await.expect("send ok");

        let sent = t.sent.lock().unwrap();
        assert_eq!(sent[0].text, "*Subject*\nbody");
    }

    #[tokio::test]
    async fn send_negative_chat_id_works_for_group_chats() {
        // Telegram group/channel chat_ids are negative i64.
        let t = ScriptedTransport::new();
        let backend = NotifyTelegramBackend::new(t.clone(), "-1001234567890")
            .expect("negative group chat_id parses");
        backend.send("group msg", None).await.expect("send ok");

        let sent = t.sent.lock().unwrap();
        assert_eq!(sent[0].chat_id, -1001234567890);
    }

    #[tokio::test]
    async fn send_maps_401_transport_error_to_auth() {
        let t = ScriptedTransport::with_outcome(Err(TransportError::Platform(
            "send_message: 401 Unauthorized".into(),
        )));
        let backend = NotifyTelegramBackend::new(t, "1").expect("chat_id ok");
        let r = backend.send("hi", None).await;
        match r {
            Err(NotifyError::Auth(_)) => {}
            other => panic!("expected Auth, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn send_maps_400_transport_error_to_rejected() {
        let t = ScriptedTransport::with_outcome(Err(TransportError::Platform(
            "send_message: 400 Bad Request".into(),
        )));
        let backend = NotifyTelegramBackend::new(t, "1").expect("chat_id ok");
        let r = backend.send("hi", None).await;
        match r {
            Err(NotifyError::Rejected(400)) => {}
            other => panic!("expected Rejected(400), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn send_maps_timeout_transport_error_to_timeout_variant() {
        let t = ScriptedTransport::with_outcome(Err(TransportError::Platform(
            "send_message: request timed out".into(),
        )));
        let backend = NotifyTelegramBackend::new(t, "1").expect("chat_id ok");
        let r = backend.send("hi", None).await;
        assert!(matches!(r, Err(NotifyError::Timeout)));
    }

    #[tokio::test]
    async fn send_unclassified_error_falls_through_to_transport() {
        let t = ScriptedTransport::with_outcome(Err(TransportError::Platform(
            "send_message: unexpected EOF".into(),
        )));
        let backend = NotifyTelegramBackend::new(t, "1").expect("chat_id ok");
        let r = backend.send("hi", None).await;
        match r {
            Err(NotifyError::Transport(_)) => {}
            other => panic!("expected Transport, got {other:?}"),
        }
    }

    #[test]
    fn extract_status_code_finds_standalone_3digit_4xx() {
        assert_eq!(extract_status_code("got 404 from api"), Some(404));
        assert_eq!(extract_status_code("status: 429 too many"), Some(429));
    }

    #[test]
    fn extract_status_code_ignores_embedded_digit_runs() {
        // A 4-digit run is not a status code — extract should
        // skip the embedded 3-digit window because the boundary
        // check fails.
        assert_eq!(extract_status_code("file 12345 bad"), None);
    }

    #[test]
    fn extract_status_code_returns_none_when_absent() {
        assert_eq!(extract_status_code("network unreachable"), None);
    }

    #[test]
    fn telegram_backend_kind_is_telegram() {
        let t = ScriptedTransport::new();
        let backend = NotifyTelegramBackend::new(t, "1").unwrap();
        assert_eq!(backend.kind(), "telegram");
    }
}
