//! Phase 62 Task 4 — `NotifyDispatcher` + `NotifyBackend` trait.
//!
//! The dispatcher is the runtime counterpart to the
//! `[[notify_target]]` config surface from
//! [`aivyx_config::NotifyTargetConfig`] (Task 3). At daemon
//! startup the binary constructs a [`NotifyDispatcher`] from the
//! loaded targets, registering one [`NotifyBackend`] per enabled
//! target keyed by `name`. The [`NotifySendTool`] (Task 7) holds
//! an `Arc<NotifyDispatcher>` and calls `dispatch` from its
//! `execute` path.
//!
//! ## Trait surface
//!
//! ```ignore
//! #[async_trait]
//! pub trait NotifyBackend: Send + Sync {
//!     async fn send(
//!         &self,
//!         message: &str,
//!         subject: Option<&str>,
//!     ) -> Result<(), NotifyError>;
//! }
//! ```
//!
//! `Send + Sync` because backends live behind an `Arc` shared
//! across daemon connection tasks. `async_trait` because the
//! `dyn`-compatible async method is what the dispatcher
//! requires.
//!
//! ## Failure shape
//!
//! [`NotifyError`] is intentionally small — four variants
//! capturing the distinctions that matter to the agent:
//!
//! - `Transport` — the dispatch itself failed (DNS, TLS, socket
//!   close, malformed response). Retryable.
//! - `Auth` — the backend rejected the credentials it was
//!   constructed with (Telegram 401, webhook 401/403). Not
//!   retryable without operator intervention.
//! - `Rejected(u16)` — the backend accepted the connection but
//!   refused the payload (HTTP 4xx other than 401/403, Telegram
//!   400). Often a malformed message body the agent can fix and
//!   retry.
//! - `Timeout` — the operation didn't complete inside the
//!   per-backend timeout. Retryable.
//! - `UnknownTarget` — `dispatch` was called with a target name
//!   that's not in the registry. Only the dispatcher itself
//!   raises this; backends never see it.
//!
//! The tool (Task 7) maps these to the `success=false` output
//! shape per Q4(a) at sign-off so the agent gets structured
//! retry guidance without `ToolOutcome::Failed` propagating up
//! the turn loop.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use aivyx_config::{NotifyTargetConfig, NotifyTargetKind};
use aivyx_telegram::transport::TelegramTransport;

use crate::notify_telegram::NotifyTelegramBackend;
use crate::notify_webhook::NotifyWebhookBackend;

/// Trait every notification backend implements. One impl per
/// kind: Telegram (Task 5), Webhook (Task 6), future kinds
/// (email SMTP, Web UI desktop, OS-level) land as additional
/// impls in their own files.
#[async_trait]
pub trait NotifyBackend: Send + Sync {
    /// Send a single message. `subject` is an optional short
    /// title — backends that don't support a title field (e.g.
    /// Telegram, which is just text messages) may either embed
    /// the subject inline (prepend bold) or ignore it.
    async fn send(
        &self,
        message: &str,
        subject: Option<&str>,
    ) -> Result<(), NotifyError>;

    /// Backend kind discriminator for diagnostics (used in
    /// error messages and the system-prompt enumeration). One
    /// of `"telegram"`, `"webhook"`, etc. — matches the TOML
    /// `kind` field.
    fn kind(&self) -> &'static str;
}

/// Failure modes for [`NotifyBackend::send`] and
/// [`NotifyDispatcher::dispatch`]. Phase 62 Task 4.
#[derive(Debug, Clone, thiserror::Error)]
pub enum NotifyError {
    /// Connection/protocol-level failure: DNS, TLS handshake,
    /// socket close mid-request, malformed server response.
    /// The agent should consider this retryable.
    #[error("notify transport error: {0}")]
    Transport(String),

    /// Backend rejected the credentials it was constructed
    /// with. Telegram 401, webhook 401/403. Not retryable
    /// without operator intervention (rotate the bot token,
    /// fix the URL, etc.).
    #[error("notify authentication error: {0}")]
    Auth(String),

    /// Backend accepted the connection but refused the
    /// payload. HTTP 4xx other than 401/403, Telegram 400 on a
    /// bad chat_id or oversized message. The agent may fix the
    /// message and retry.
    #[error("notify rejected (status {0})")]
    Rejected(u16),

    /// Operation didn't complete inside the per-backend
    /// timeout (5 seconds for Task 6's webhook impl). The
    /// agent should consider this retryable but should
    /// rate-limit retries to avoid wedging on a slow endpoint.
    #[error("notify timeout")]
    Timeout,

    /// `dispatch` was called with a target name not present in
    /// the registry. Only [`NotifyDispatcher::dispatch`] raises
    /// this; backends never see it. The tool (Task 7) maps
    /// this to a distinct `success=false` shape so the agent
    /// can correct the target name on retry.
    #[error("unknown notification target `{0}`")]
    UnknownTarget(String),
}

/// Phase 62 Task 4 — runtime registry of notification backends.
///
/// Constructed at daemon startup with one entry per enabled
/// `[[notify_target]]` config block. The mapping from
/// `NotifyTargetKind` to a concrete backend is wired in Tasks 5
/// (Telegram) and 6 (webhook); Task 4 ships the trait, the
/// registry shape, and the dispatch logic.
pub struct NotifyDispatcher {
    /// Target-name keyed registry. `Arc<dyn NotifyBackend>`
    /// lets the dispatcher hand out cheap clones if a future
    /// API surfaces individual backends to callers.
    backends: HashMap<String, Arc<dyn NotifyBackend>>,
}

impl std::fmt::Debug for NotifyDispatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `dyn NotifyBackend` isn't Debug, so we surface what's
        // observable: target names + kinds. Useful in tests and
        // for the daemon's startup log.
        let mut targets: Vec<(&str, &'static str)> = self.list_targets();
        targets.sort();
        f.debug_struct("NotifyDispatcher")
            .field("targets", &targets)
            .finish()
    }
}

impl NotifyDispatcher {
    /// New empty dispatcher. Callers register backends with
    /// [`register`](Self::register) per loaded target. The
    /// daemon's startup path does this in one pass after
    /// constructing the per-kind backend instances.
    pub fn new() -> Self {
        Self {
            backends: HashMap::new(),
        }
    }

    /// Register a backend under `target_name`. Returns the
    /// previous entry if a collision occurs — the daemon
    /// should treat this as a programming error since
    /// `aivyx-config` already rejects duplicate names at load
    /// time (`notify_target_duplicate_names_are_error`). A
    /// duplicate here means the loader and the registrar
    /// disagreed.
    pub fn register(
        &mut self,
        target_name: impl Into<String>,
        backend: Arc<dyn NotifyBackend>,
    ) -> Option<Arc<dyn NotifyBackend>> {
        self.backends.insert(target_name.into(), backend)
    }

    /// Send `message` to the named target. Returns
    /// `NotifyError::UnknownTarget` if no backend is registered
    /// under that name; otherwise propagates the backend's
    /// `send` result.
    pub async fn dispatch(
        &self,
        target_name: &str,
        message: &str,
        subject: Option<&str>,
    ) -> Result<(), NotifyError> {
        let backend = self
            .backends
            .get(target_name)
            .ok_or_else(|| NotifyError::UnknownTarget(target_name.to_string()))?;
        backend.send(message, subject).await
    }

    /// Enumerate registered targets as `(name, kind)` pairs.
    /// Used by Task 8's system-prompt surfacing to list
    /// reachable targets in the assistant's prompt section.
    /// Order is iteration order of the underlying HashMap;
    /// callers that need stable ordering should sort.
    pub fn list_targets(&self) -> Vec<(&str, &'static str)> {
        self.backends
            .iter()
            .map(|(name, backend)| (name.as_str(), backend.kind()))
            .collect()
    }

    /// Number of registered backends. Useful for the daemon's
    /// startup log line and for tests.
    pub fn len(&self) -> usize {
        self.backends.len()
    }

    /// True when no backends are registered. Equivalent to
    /// `len() == 0`.
    pub fn is_empty(&self) -> bool {
        self.backends.is_empty()
    }
}

impl Default for NotifyDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

/// Phase 62 Task 8 — build a dispatcher from the operator's
/// configured `[[notify_target]]` entries.
///
/// Constructs one backend per target:
///
/// - `NotifyTargetKind::Telegram { chat_id }` → [`NotifyTelegramBackend`]
///   wrapping the supplied `telegram_transport`. If no transport
///   is supplied (the operator didn't configure `[telegram]
///   token`), returns an error naming the offending target.
/// - `NotifyTargetKind::Webhook { url }` → [`NotifyWebhookBackend`]
///   with a default reqwest sender (5s timeout).
/// - `NotifyTargetKind::Email { to }` → [`NotifyEmailBackend`]
///   (Phase 68) wrapping the supplied `email_sender`. Built
///   from `[email]` config once and shared across every email
///   target.
///
/// Returns an empty dispatcher if `targets` is empty — the
/// `notify.send` tool then surfaces `UnknownTarget` for every
/// call, which is the operator-correct behavior (the agent will
/// learn from the system prompt or its tool description that no
/// targets are configured).
pub fn build_notify_dispatcher(
    targets: &[NotifyTargetConfig],
    telegram_transport: Option<Arc<dyn TelegramTransport>>,
    email_context: Option<EmailDispatchContext>,
    web_ui_broadcaster: Option<Arc<crate::notify_webui::WebUiBroadcaster>>,
) -> Result<Arc<NotifyDispatcher>, String> {
    let mut d = NotifyDispatcher::new();
    for target in targets {
        let backend: Arc<dyn NotifyBackend> = match &target.kind {
            NotifyTargetKind::Telegram { chat_id } => {
                let transport = telegram_transport.clone().ok_or_else(|| {
                    format!(
                        "notify_target `{}` (kind = telegram) requires a configured \
                         [telegram] token; either remove the target or add \
                         `[telegram] token = \"...\"` to aivyx.toml",
                        target.name
                    )
                })?;
                let backend = NotifyTelegramBackend::new(transport, chat_id)
                    .map_err(|e| format!("notify_target `{}`: {e}", target.name))?;
                Arc::new(backend)
            }
            NotifyTargetKind::Webhook { url } => Arc::new(NotifyWebhookBackend::new(
                target.name.clone(),
                url.clone(),
            )),
            NotifyTargetKind::Email { to } => {
                let ctx = email_context.as_ref().ok_or_else(|| {
                    format!(
                        "notify_target `{}` (kind = email) requires a configured \
                         [email] section; the config loader should have caught \
                         this — defense-in-depth check at dispatcher build time",
                        target.name,
                    )
                })?;
                Arc::new(crate::notify_email::NotifyEmailBackend::new(
                    Arc::clone(&ctx.sender),
                    ctx.from.clone(),
                    to.clone(),
                ))
            }
            // Phase 69 — Web UI desktop notification. Multiple
            // `kind = "web-ui"` targets all funnel into the same
            // broadcaster (one Web UI server per daemon); the
            // dispatcher registers a distinct backend per target
            // name so the agent's `notify.send` tool addresses
            // them individually for audit purposes.
            NotifyTargetKind::WebUi => {
                let bc = web_ui_broadcaster.as_ref().ok_or_else(|| {
                    format!(
                        "notify_target `{}` (kind = web-ui) requires the \
                         Web UI server to be enabled; either remove the \
                         target or enable the Web UI in aivyx.toml",
                        target.name,
                    )
                })?;
                Arc::new(crate::notify_webui::NotifyWebUiBackend::new(
                    Arc::clone(bc),
                ))
            }
        };
        d.register(&target.name, backend);
    }
    Ok(Arc::new(d))
}

/// Phase 68 — context for building email backends in
/// `build_notify_dispatcher`. Bundles the shared
/// `Arc<dyn EmailSender>` (one `LettreEmailSender` per
/// deployment) with the `from` address pulled from
/// `[email] from`. The binary's startup path constructs this
/// once if any email targets exist; the dispatcher Arc-clones
/// the sender into each per-target backend.
pub struct EmailDispatchContext {
    pub sender: Arc<dyn crate::notify_email::EmailSender>,
    pub from: String,
}

// ---------------------------------------------------------------------------
// Tests — exercise the dispatcher's dispatch routing using a
// MockBackend. Tasks 5 and 6 add real backend impls; their tests
// hit the actual Telegram / webhook APIs in isolation.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Test-only backend that records every `send` call in
    /// memory and returns a caller-configurable outcome. Lets
    /// the dispatcher tests verify routing without any I/O.
    struct MockBackend {
        kind_tag: &'static str,
        outcome: Mutex<Result<(), NotifyError>>,
        calls: Mutex<Vec<(String, Option<String>)>>,
    }

    impl MockBackend {
        fn new(kind_tag: &'static str) -> Arc<Self> {
            Arc::new(Self {
                kind_tag,
                outcome: Mutex::new(Ok(())),
                calls: Mutex::new(Vec::new()),
            })
        }

        fn with_outcome(kind_tag: &'static str, outcome: Result<(), NotifyError>) -> Arc<Self> {
            Arc::new(Self {
                kind_tag,
                outcome: Mutex::new(outcome),
                calls: Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl NotifyBackend for MockBackend {
        async fn send(
            &self,
            message: &str,
            subject: Option<&str>,
        ) -> Result<(), NotifyError> {
            self.calls
                .lock()
                .unwrap()
                .push((message.to_string(), subject.map(|s| s.to_string())));
            self.outcome.lock().unwrap().clone()
        }

        fn kind(&self) -> &'static str {
            self.kind_tag
        }
    }

    #[tokio::test]
    async fn dispatch_routes_to_named_backend_and_records_call() {
        let backend = MockBackend::new("mock");
        let mut d = NotifyDispatcher::new();
        d.register("phone", backend.clone());

        let r = d.dispatch("phone", "hello", Some("subject")).await;
        assert!(r.is_ok());

        let calls = backend.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "hello");
        assert_eq!(calls[0].1, Some("subject".to_string()));
    }

    #[tokio::test]
    async fn dispatch_unknown_target_returns_unknown_target_error() {
        let d = NotifyDispatcher::new();
        let r = d.dispatch("nope", "hi", None).await;
        match r {
            Err(NotifyError::UnknownTarget(name)) => assert_eq!(name, "nope"),
            other => panic!("expected UnknownTarget, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn dispatch_propagates_backend_error() {
        let backend = MockBackend::with_outcome(
            "mock",
            Err(NotifyError::Rejected(429)),
        );
        let mut d = NotifyDispatcher::new();
        d.register("phone", backend);

        let r = d.dispatch("phone", "hi", None).await;
        assert!(matches!(r, Err(NotifyError::Rejected(429))));
    }

    #[tokio::test]
    async fn dispatch_routes_to_correct_backend_among_many() {
        let phone = MockBackend::new("telegram");
        let alerts = MockBackend::new("webhook");
        let mut d = NotifyDispatcher::new();
        d.register("phone", phone.clone());
        d.register("alerts", alerts.clone());

        d.dispatch("phone", "to phone", None).await.unwrap();
        d.dispatch("alerts", "to alerts", None).await.unwrap();

        assert_eq!(phone.calls.lock().unwrap().len(), 1);
        assert_eq!(phone.calls.lock().unwrap()[0].0, "to phone");
        assert_eq!(alerts.calls.lock().unwrap().len(), 1);
        assert_eq!(alerts.calls.lock().unwrap()[0].0, "to alerts");
    }

    #[test]
    fn register_returns_previous_on_collision() {
        let a = MockBackend::new("mock");
        let b = MockBackend::new("mock");
        let mut d = NotifyDispatcher::new();
        assert!(d.register("phone", a).is_none());
        assert!(
            d.register("phone", b).is_some(),
            "duplicate registration must surface the previous backend"
        );
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn list_targets_enumerates_name_kind_pairs() {
        let phone = MockBackend::new("telegram");
        let alerts = MockBackend::new("webhook");
        let mut d = NotifyDispatcher::new();
        d.register("phone", phone);
        d.register("alerts", alerts);

        let mut pairs: Vec<(&str, &str)> = d.list_targets();
        pairs.sort();
        assert_eq!(pairs, vec![("alerts", "webhook"), ("phone", "telegram")]);
    }

    #[test]
    fn empty_dispatcher_reports_zero_len() {
        let d = NotifyDispatcher::new();
        assert_eq!(d.len(), 0);
        assert!(d.is_empty());
    }

    // ---- build_notify_dispatcher (Phase 62 Task 8) -----------

    use aivyx_telegram::transport::{IncomingMessage, OutgoingMessage, TransportError};

    struct NoopTransport;

    #[async_trait]
    impl TelegramTransport for NoopTransport {
        async fn get_updates(
            &self,
            _offset: i64,
            _timeout_secs: u32,
        ) -> Result<Vec<IncomingMessage>, TransportError> {
            Ok(Vec::new())
        }
        async fn send_message(&self, _msg: OutgoingMessage) -> Result<(), TransportError> {
            Ok(())
        }
    }

    #[test]
    fn build_empty_targets_returns_empty_dispatcher() {
        let d = build_notify_dispatcher(&[], None, None, None).expect("ok");
        assert_eq!(d.len(), 0);
    }

    #[test]
    fn build_webhook_only_does_not_require_telegram_transport() {
        let targets = vec![NotifyTargetConfig {
            name: "alerts".into(),
            kind: NotifyTargetKind::Webhook {
                url: "https://example.com/x".into(),
            },
            enabled: true,
            is_default: false,
        }];
        let d = build_notify_dispatcher(&targets, None, None, None).expect("ok");
        assert_eq!(d.len(), 1);
        let pairs = d.list_targets();
        assert_eq!(pairs[0], ("alerts", "webhook"));
    }

    #[test]
    fn build_telegram_without_transport_returns_descriptive_error() {
        let targets = vec![NotifyTargetConfig {
            name: "phone".into(),
            kind: NotifyTargetKind::Telegram {
                chat_id: "123".into(),
            },
            enabled: true,
            is_default: false,
        }];
        let err = build_notify_dispatcher(&targets, None, None, None).expect_err("must error");
        assert!(err.contains("`phone`"), "error: {err}");
        assert!(err.contains("[telegram] token"), "error: {err}");
    }

    #[test]
    fn build_telegram_with_transport_succeeds() {
        let targets = vec![NotifyTargetConfig {
            name: "phone".into(),
            kind: NotifyTargetKind::Telegram {
                chat_id: "123".into(),
            },
            enabled: true,
            is_default: false,
        }];
        let transport: Arc<dyn TelegramTransport> = Arc::new(NoopTransport);
        let d = build_notify_dispatcher(&targets, Some(transport), None, None).expect("ok");
        assert_eq!(d.len(), 1);
        let pairs = d.list_targets();
        assert_eq!(pairs[0], ("phone", "telegram"));
    }

    #[test]
    fn build_telegram_with_invalid_chat_id_returns_construction_error() {
        let targets = vec![NotifyTargetConfig {
            name: "phone".into(),
            kind: NotifyTargetKind::Telegram {
                chat_id: "not-a-number".into(),
            },
            enabled: true,
            is_default: false,
        }];
        let transport: Arc<dyn TelegramTransport> = Arc::new(NoopTransport);
        let err =
            build_notify_dispatcher(&targets, Some(transport), None, None).expect_err("must error");
        assert!(err.contains("phone"), "error: {err}");
        assert!(err.contains("invalid telegram chat_id"), "error: {err}");
    }

    #[test]
    fn build_mixed_kinds_registers_each() {
        let targets = vec![
            NotifyTargetConfig {
                name: "phone".into(),
                kind: NotifyTargetKind::Telegram {
                    chat_id: "1".into(),
                },
                enabled: true,
            is_default: false,
            },
            NotifyTargetConfig {
                name: "alerts".into(),
                kind: NotifyTargetKind::Webhook {
                    url: "https://example.com/".into(),
                },
                enabled: true,
            is_default: false,
            },
        ];
        let transport: Arc<dyn TelegramTransport> = Arc::new(NoopTransport);
        let d = build_notify_dispatcher(&targets, Some(transport), None, None).expect("ok");
        assert_eq!(d.len(), 2);
        let mut pairs = d.list_targets();
        pairs.sort();
        assert_eq!(pairs, vec![("alerts", "webhook"), ("phone", "telegram")]);
    }

    // ---- Phase 69 — Web UI dispatcher arm -----------------

    #[test]
    fn build_web_ui_without_broadcaster_returns_descriptive_error() {
        let targets = vec![NotifyTargetConfig {
            name: "desktop".into(),
            kind: NotifyTargetKind::WebUi,
            enabled: true,
            is_default: false,
        }];
        let err =
            build_notify_dispatcher(&targets, None, None, None).expect_err("must error");
        assert!(err.contains("`desktop`"), "error: {err}");
        assert!(err.contains("Web UI"), "error: {err}");
    }

    #[test]
    fn build_web_ui_with_broadcaster_registers_backend() {
        let targets = vec![NotifyTargetConfig {
            name: "desktop".into(),
            kind: NotifyTargetKind::WebUi,
            enabled: true,
            is_default: false,
        }];
        let bc = Arc::new(crate::notify_webui::WebUiBroadcaster::new());
        let d = build_notify_dispatcher(&targets, None, None, Some(bc)).expect("ok");
        assert_eq!(d.len(), 1);
        let pairs = d.list_targets();
        assert_eq!(pairs[0], ("desktop", "web-ui"));
    }

    #[tokio::test]
    async fn dispatch_routes_web_ui_target_to_broadcaster() {
        let targets = vec![NotifyTargetConfig {
            name: "desktop".into(),
            kind: NotifyTargetKind::WebUi,
            enabled: true,
            is_default: false,
        }];
        let bc = Arc::new(crate::notify_webui::WebUiBroadcaster::new());
        let mut rx = bc.subscribe();
        let d = build_notify_dispatcher(&targets, None, None, Some(Arc::clone(&bc)))
            .expect("ok");
        d.dispatch("desktop", "build done", Some("Aivyx"))
            .await
            .expect("dispatch");
        let frame = rx.recv().await.expect("recv");
        assert_eq!(frame.title, "Aivyx");
        assert_eq!(frame.body, "build done");
    }
}
