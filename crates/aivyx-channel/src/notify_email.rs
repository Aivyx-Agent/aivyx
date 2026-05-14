//! Phase 68 — SMTP email notification backend (Reach Phase 3).
//!
//! Third notify backend after Telegram (Phase 62) and webhook
//! (Phase 62). Closes the largest remaining adoption-shape gap
//! on the Reach axis: every operator has email; most don't run
//! a Telegram bot.
//!
//! ## Shape
//!
//! - `EmailSender` trait abstracts the SMTP send so unit tests
//!   can use a `ScriptedEmailSender` (no real network needed).
//!   Mirrors the Phase 62 `WebhookSender` pattern.
//! - `LettreEmailSender` is the production impl, wrapping a
//!   shared `lettre::AsyncSmtpTransport<Tokio1Executor>` built
//!   from `EmailConfig` once at daemon startup. Multiple email
//!   targets share the transport (and therefore the SMTP
//!   connection pool).
//! - `NotifyEmailBackend` holds an `Arc<dyn EmailSender>` + the
//!   `from` and per-target `to` addresses; implements
//!   `NotifyBackend` via `send_email`.
//!
//! ## TLS
//!
//! Per Q3(a) at sign-off: STARTTLS on port 587 by default
//! (modern submission standard). Implicit TLS (port 465) is
//! supported via `[email] tls_mode = "implicit"`. `tls_mode =
//! "none"` is rejected at config-load time because PLAIN/LOGIN
//! auth over cleartext would leak credentials.
//!
//! ## Auth
//!
//! Per Q4(a) at sign-off: PLAIN/LOGIN only in v1. XOAUTH2
//! deferred to a follow-up phase if pressure surfaces.
//! Operators using Gmail / Office 365 use app passwords.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use lettre::message::{header::ContentType, Mailbox};
use lettre::transport::smtp::authentication::{Credentials, Mechanism};
use lettre::transport::smtp::AsyncSmtpTransport;
use lettre::{AsyncTransport, Message, Tokio1Executor};
use secrecy::ExposeSecret;

use aivyx_config::{EmailConfig, TlsMode};

use crate::notify_dispatcher::{NotifyBackend, NotifyError};

/// Abstraction over the SMTP send so the dispatcher's email
/// targets can be unit-tested without spinning up a real SMTP
/// server. Production: [`LettreEmailSender`]. Tests:
/// `ScriptedEmailSender` (see the `#[cfg(test)]` module below).
#[async_trait]
pub trait EmailSender: Send + Sync {
    /// Send a single plain-text email. Returns `Ok(())` on
    /// successful submission to the SMTP server; backend errors
    /// surface as the appropriate [`NotifyError`] variant.
    async fn send_email(
        &self,
        from: &str,
        to: &str,
        subject: &str,
        body: &str,
    ) -> Result<(), NotifyError>;
}

/// Production [`EmailSender`] wrapping a shared
/// `AsyncSmtpTransport<Tokio1Executor>`. The transport is
/// built once from [`EmailConfig`] and Arc-cloned into every
/// email backend that uses the same SMTP account.
pub struct LettreEmailSender {
    transport: AsyncSmtpTransport<Tokio1Executor>,
}

impl LettreEmailSender {
    /// Build the transport from a parsed [`EmailConfig`].
    /// Returns an error on builder failure (typically an
    /// invalid hostname; TLS-mode validation already happened
    /// at config-load time).
    pub fn from_config(cfg: &EmailConfig) -> Result<Self, String> {
        let username = cfg.username.value.expose_secret().to_string();
        let password = cfg.password.value.expose_secret().to_string();
        let creds = Credentials::new(username, password);

        let mut builder = match cfg.tls_mode {
            TlsMode::Starttls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&cfg.host)
                    .map_err(|e| format!("smtp transport builder failed: {e}"))?
            }
            TlsMode::Implicit => {
                AsyncSmtpTransport::<Tokio1Executor>::relay(&cfg.host)
                    .map_err(|e| format!("smtp transport builder failed: {e}"))?
            }
            // tls_mode = "none" is rejected at config-load
            // time (Q4(a) sign-off); the loader never produces
            // this variant. Defense-in-depth: treat as a
            // builder error if it reaches here.
            TlsMode::None => {
                return Err(
                    "tls_mode = none is forbidden — config loader should have caught this"
                        .to_string(),
                );
            }
        };
        builder = builder
            .port(cfg.port)
            .credentials(creds)
            // PLAIN + LOGIN per Q4(a). lettre's default
            // mechanism set already covers these; we declare
            // explicitly so future lettre versions don't
            // silently enable XOAUTH2 without operator
            // sign-off.
            .authentication(vec![Mechanism::Plain, Mechanism::Login])
            .timeout(Some(Duration::from_secs(10)));

        Ok(Self {
            transport: builder.build(),
        })
    }
}

#[async_trait]
impl EmailSender for LettreEmailSender {
    async fn send_email(
        &self,
        from: &str,
        to: &str,
        subject: &str,
        body: &str,
    ) -> Result<(), NotifyError> {
        let from_mb: Mailbox = from
            .parse()
            .map_err(|e: lettre::address::AddressError| {
                NotifyError::Transport(format!("invalid `from` address `{from}`: {e}"))
            })?;
        let to_mb: Mailbox = to
            .parse()
            .map_err(|e: lettre::address::AddressError| {
                NotifyError::Transport(format!("invalid `to` address `{to}`: {e}"))
            })?;
        let message = Message::builder()
            .from(from_mb)
            .to(to_mb)
            .subject(subject)
            .header(ContentType::TEXT_PLAIN)
            .body(body.to_string())
            .map_err(|e| {
                NotifyError::Transport(format!("compose message failed: {e}"))
            })?;
        self.transport
            .send(message)
            .await
            .map(|_| ())
            .map_err(map_lettre_error)
    }
}

/// Map a lettre `Error` to the right [`NotifyError`] variant.
/// Heuristic — lettre's error surface is a couple dozen kinds;
/// we collapse them into the five-variant taxonomy the
/// `notify.send` tool already uses.
pub fn map_lettre_error(e: lettre::transport::smtp::Error) -> NotifyError {
    let message = e.to_string();
    let lower = message.to_lowercase();

    if e.is_timeout() {
        return NotifyError::Timeout;
    }
    if lower.contains("authentication")
        || lower.contains("auth failed")
        || lower.contains("login failed")
        || lower.contains("535")
    {
        return NotifyError::Auth(message);
    }
    // Status code extraction — SMTP servers return 5xx
    // permanent / 4xx transient. lettre exposes status codes
    // on `Error` via its category enums in newer versions, but
    // the most portable path is to scan the message for a
    // 3-digit reply code.
    if let Some(status) = extract_smtp_status(&message) {
        if (500..600).contains(&status) {
            return NotifyError::Rejected(status);
        }
        if (400..500).contains(&status) {
            return NotifyError::Transport(message);
        }
    }
    NotifyError::Transport(message)
}

/// Best-effort extraction of an SMTP reply code from a lettre
/// error message. Standalone 3-digit number, boundary-aware
/// (mirrors the Phase 62 `extract_status_code` from
/// `notify_telegram.rs` — same algorithm).
fn extract_smtp_status(msg: &str) -> Option<u16> {
    let bytes = msg.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i].is_ascii_digit()
            && bytes[i + 1].is_ascii_digit()
            && bytes[i + 2].is_ascii_digit()
        {
            let starts_run = i == 0 || !bytes[i - 1].is_ascii_digit();
            let ends_run = i + 3 == bytes.len() || !bytes[i + 3].is_ascii_digit();
            if starts_run && ends_run {
                let s = std::str::from_utf8(&bytes[i..i + 3]).ok()?;
                let n: u16 = s.parse().ok()?;
                if (200..=599).contains(&n) {
                    return Some(n);
                }
            }
        }
        i += 1;
    }
    None
}

/// Per-target email backend. Phase 68.
pub struct NotifyEmailBackend {
    sender: Arc<dyn EmailSender>,
    from: String,
    to: String,
}

impl NotifyEmailBackend {
    /// Construct with a shared sender + the per-target
    /// recipient. The `from` address comes from `[email] from`
    /// (one sender per deployment).
    pub fn new(sender: Arc<dyn EmailSender>, from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            sender,
            from: from.into(),
            to: to.into(),
        }
    }

    /// Compose the subject line for a notification. Public for
    /// testing. When `subject` is present, that's the subject
    /// verbatim; when absent, "Aivyx notification" is the
    /// default so the email doesn't ship with an empty Subject:
    /// header (which some servers downgrade).
    pub fn compose_subject(subject: Option<&str>) -> String {
        match subject {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => "Aivyx notification".to_string(),
        }
    }
}

#[async_trait]
impl NotifyBackend for NotifyEmailBackend {
    async fn send(
        &self,
        message: &str,
        subject: Option<&str>,
    ) -> Result<(), NotifyError> {
        let subject_line = Self::compose_subject(subject);
        self.sender
            .send_email(&self.from, &self.to, &subject_line, message)
            .await
    }

    fn kind(&self) -> &'static str {
        "email"
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Test-only sender that records every call and returns a
    /// caller-configurable outcome. Lets the backend tests
    /// verify dispatch without any I/O.
    struct ScriptedEmailSender {
        outcome: Mutex<Result<(), NotifyError>>,
        calls: Mutex<Vec<(String, String, String, String)>>,
    }

    impl ScriptedEmailSender {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                outcome: Mutex::new(Ok(())),
                calls: Mutex::new(Vec::new()),
            })
        }

        fn with_outcome(outcome: Result<(), NotifyError>) -> Arc<Self> {
            Arc::new(Self {
                outcome: Mutex::new(outcome),
                calls: Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl EmailSender for ScriptedEmailSender {
        async fn send_email(
            &self,
            from: &str,
            to: &str,
            subject: &str,
            body: &str,
        ) -> Result<(), NotifyError> {
            self.calls.lock().unwrap().push((
                from.to_string(),
                to.to_string(),
                subject.to_string(),
                body.to_string(),
            ));
            self.outcome.lock().unwrap().clone()
        }
    }

    #[test]
    fn compose_subject_uses_supplied_value() {
        assert_eq!(
            NotifyEmailBackend::compose_subject(Some("Daily summary")),
            "Daily summary",
        );
    }

    #[test]
    fn compose_subject_default_when_none() {
        assert_eq!(
            NotifyEmailBackend::compose_subject(None),
            "Aivyx notification",
        );
    }

    #[test]
    fn compose_subject_default_when_empty() {
        assert_eq!(
            NotifyEmailBackend::compose_subject(Some("")),
            "Aivyx notification",
        );
    }

    #[tokio::test]
    async fn send_dispatches_to_underlying_sender_with_from_and_to() {
        let sender = ScriptedEmailSender::new();
        let backend = NotifyEmailBackend::new(
            sender.clone(),
            "aivyx@example.com",
            "alice@example.com",
        );
        backend.send("hello world", Some("Test")).await.expect("send ok");

        let calls = sender.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let (from, to, subject, body) = &calls[0];
        assert_eq!(from, "aivyx@example.com");
        assert_eq!(to, "alice@example.com");
        assert_eq!(subject, "Test");
        assert_eq!(body, "hello world");
    }

    #[tokio::test]
    async fn send_uses_default_subject_when_omitted() {
        let sender = ScriptedEmailSender::new();
        let backend = NotifyEmailBackend::new(
            sender.clone(),
            "aivyx@example.com",
            "alice@example.com",
        );
        backend.send("body", None).await.expect("send ok");

        let calls = sender.calls.lock().unwrap();
        assert_eq!(calls[0].2, "Aivyx notification");
    }

    #[tokio::test]
    async fn send_propagates_auth_error() {
        let sender = ScriptedEmailSender::with_outcome(Err(NotifyError::Auth(
            "535 5.7.8".into(),
        )));
        let backend = NotifyEmailBackend::new(sender, "a@example.com", "b@example.com");
        let r = backend.send("x", None).await;
        assert!(matches!(r, Err(NotifyError::Auth(_))));
    }

    #[tokio::test]
    async fn send_propagates_timeout() {
        let sender = ScriptedEmailSender::with_outcome(Err(NotifyError::Timeout));
        let backend = NotifyEmailBackend::new(sender, "a@example.com", "b@example.com");
        let r = backend.send("x", None).await;
        assert!(matches!(r, Err(NotifyError::Timeout)));
    }

    #[test]
    fn email_backend_kind_is_email() {
        let sender = ScriptedEmailSender::new();
        let backend = NotifyEmailBackend::new(sender, "a@example.com", "b@example.com");
        assert_eq!(backend.kind(), "email");
    }

    #[test]
    fn extract_smtp_status_finds_3xx_4xx_5xx() {
        assert_eq!(extract_smtp_status("550 mailbox unavailable"), Some(550));
        assert_eq!(extract_smtp_status("got 421 try later"), Some(421));
        assert_eq!(extract_smtp_status("no code here"), None);
    }

    #[test]
    fn extract_smtp_status_skips_embedded_long_numbers() {
        assert_eq!(extract_smtp_status("entry 12345 found"), None);
    }
}
