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
}
