//! Google Calendar v3 REST API client.
//!
//! Phase 128 Task 3 — skeleton; per-tool tasks 4-8 fill in
//! the actual HTTP operations. This module exposes the
//! `CalendarClient` struct (token + HTTP client wrapper)
//! and the `CalendarClientError` shared error type so the
//! per-tool implementations have a single error surface.
//!
//! ## Why a thin wrapper instead of a generated SDK
//!
//! Same posture as `aivyx-gmail::gmail_client` — Google's
//! generated client crate is large and async-trait-heavy
//! for what amounts to half a dozen REST calls. A
//! hand-written reqwest client is simpler, audit-readable,
//! and matches the workspace's rustls-only TLS posture
//! without an OpenSSL pull-in.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use reqwest::{Client, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::Value;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::oauth::{
    refresh_access_token, save_tokens, ExchangeError, OAuthConfig, StorageError,
    TokenSet, GOOGLE_TOKEN_ENDPOINT,
};

/// Google Calendar API base URL. All endpoints in this
/// client are appended to this base.
pub const CALENDAR_API_BASE: &str = "https://www.googleapis.com/calendar/v3";

#[derive(Debug, Error)]
pub enum CalendarClientError {
    #[error("HTTP transport error: {0}")]
    Transport(String),
    #[error("Calendar API returned HTTP {status}: {message}")]
    Api { status: u16, message: String },
    #[error("response parse error: {0}")]
    Parse(String),
    #[error("token refresh failed: {0}")]
    TokenRefresh(#[from] ExchangeError),
    #[error("token storage failed during refresh persist: {0}")]
    TokenStorage(#[from] StorageError),
    #[error("no refresh_token available; re-run `aivyx-calendar auth init`")]
    NoRefreshToken,
    #[error("input validation: {0}")]
    InvalidInput(String),
}

/// Token-authenticated Calendar API client.
///
/// Holds the operator's OAuth config + a refreshable
/// `TokenSet` behind a Mutex so concurrent tool
/// invocations can share one refresh window without
/// duplicating the refresh round-trip. Refreshed tokens
/// are persisted to disk before the in-memory update so
/// a crash mid-refresh doesn't leave memory ahead of
/// disk (mirrors `aivyx_gmail::GmailClient`'s posture).
pub struct CalendarClient {
    http: Client,
    oauth_config: OAuthConfig,
    tokens: Arc<Mutex<TokenSet>>,
    token_path: PathBuf,
    token_endpoint: String,
    /// Phase 158 — session cache of writable
    /// calendar IDs for `calendar.upcoming`'s
    /// `writable_only: true` path. None until the
    /// first writable_only call; afterwards holds
    /// `(populated_at, ids)`. TTL is enforced at
    /// read time; the cache never grows beyond a
    /// single tuple (one operator per process).
    writable_calendars_cache: WritableCalendarsCache,
    /// Phase 171 — operator-tunable TTL for the
    /// writable_calendars cache. Defaults to
    /// [`WRITABLE_CALENDARS_CACHE_TTL`] (300s,
    /// matching Phase 158). Operators with
    /// rapidly-changing calendar lists
    /// (workspace admin scenarios) can shorten
    /// it via [`with_writable_calendars_cache_ttl`]
    /// or the env var
    /// `AIVYX_PA_CALENDAR_CACHE_TTL_SECS`.
    writable_calendars_cache_ttl: Duration,
}

/// Phase 158 — type alias for the writable-
/// calendars cache cell. Keeps the
/// `CalendarClient` field signature readable and
/// keeps clippy's `type_complexity` lint happy.
type WritableCalendarsCache = Arc<Mutex<Option<(Instant, Vec<String>)>>>;

/// Phase 158 — default TTL for the
/// writable_calendars cache. 5 minutes is long
/// enough that a multi-tool-call session
/// amortizes the round trip and short enough
/// that an operator who gains a new calendar
/// mid-session waits at most one TTL window
/// before it surfaces. Phase 171 promotes
/// this from the load-bearing TTL to the
/// named default; operators tune via the
/// builder or env var.
pub const WRITABLE_CALENDARS_CACHE_TTL: Duration = Duration::from_secs(300);

/// Phase 171 — read the env-var override for
/// the cache TTL or fall back to the default.
/// Pure substrate so the env-var resolution
/// can be tested. Same posture as Phase 166's
/// `pdf_page_cap_from_env_or_default` —
/// non-numeric and zero values fall back to
/// the default.
fn cache_ttl_from_env_or_default() -> Duration {
    match std::env::var("AIVYX_PA_CALENDAR_CACHE_TTL_SECS") {
        Ok(raw) => match raw.trim().parse::<u64>() {
            Ok(n) if n >= 1 => Duration::from_secs(n),
            _ => WRITABLE_CALENDARS_CACHE_TTL,
        },
        Err(_) => WRITABLE_CALENDARS_CACHE_TTL,
    }
}

impl CalendarClient {
    /// Build a `CalendarClient` from the operator-loaded
    /// OAuth config + a `TokenSet` (loaded via
    /// `oauth::load_tokens` at process startup) + the
    /// disk path where refreshed tokens should be
    /// persisted.
    pub fn new(
        http: Client,
        oauth_config: OAuthConfig,
        tokens: TokenSet,
        token_path: PathBuf,
    ) -> Self {
        Self {
            http,
            oauth_config,
            tokens: Arc::new(Mutex::new(tokens)),
            token_path,
            token_endpoint: GOOGLE_TOKEN_ENDPOINT.to_string(),
            writable_calendars_cache: Arc::new(Mutex::new(None)),
            writable_calendars_cache_ttl: cache_ttl_from_env_or_default(),
        }
    }

    /// Phase 171 — override the writable
    /// calendars cache TTL. Takes precedence
    /// over the env-var fallback applied in
    /// `new`.
    pub fn with_writable_calendars_cache_ttl(
        mut self,
        ttl: Duration,
    ) -> Self {
        self.writable_calendars_cache_ttl = ttl;
        self
    }

    /// Test-only constructor that lets tests point the
    /// refresh round-trip at a canned local HTTP server.
    #[cfg(test)]
    pub fn with_token_endpoint(
        http: Client,
        oauth_config: OAuthConfig,
        tokens: TokenSet,
        token_path: PathBuf,
        token_endpoint: impl Into<String>,
    ) -> Self {
        Self {
            http,
            oauth_config,
            tokens: Arc::new(Mutex::new(tokens)),
            token_path,
            token_endpoint: token_endpoint.into(),
            writable_calendars_cache: Arc::new(Mutex::new(None)),
            writable_calendars_cache_ttl: cache_ttl_from_env_or_default(),
        }
    }

    /// Refresh-check + refresh-if-needed. Returns the
    /// bearer access_token to use in the next request
    /// (cloned so the mutex is released before the HTTP
    /// call). Mirrors `aivyx_gmail::GmailClient::ensure_fresh_token`.
    async fn ensure_fresh_token(&self) -> Result<String, CalendarClientError> {
        let mut guard = self.tokens.lock().await;
        if guard.needs_refresh() {
            if !guard.can_refresh() {
                return Err(CalendarClientError::NoRefreshToken);
            }
            let refreshed = refresh_access_token(
                &self.http,
                &self.token_endpoint,
                &self.oauth_config,
                &guard,
            )
            .await?;
            // Persist to disk first; only update in-memory
            // if disk write succeeded so a crash mid-refresh
            // doesn't leave the in-memory state ahead of
            // disk.
            save_tokens(&self.token_path, &refreshed).await?;
            *guard = refreshed;
        }
        Ok(guard.access_token.clone())
    }

    /// Internal helper: GET `path` with Authorization
    /// header, parse JSON body. Per-task code uses this
    /// for read-side operations (list / get).
    pub async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T, CalendarClientError> {
        let token = self.ensure_fresh_token().await?;
        let url = format!("{}{}", CALENDAR_API_BASE, path);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .query(query)
            .send()
            .await
            .map_err(|e| CalendarClientError::Transport(e.to_string()))?;
        decode_response(resp).await
    }

    /// Internal helper: POST `path` with JSON body.
    /// Per-task code uses this for create operations.
    pub async fn post_json<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &Value,
    ) -> Result<T, CalendarClientError> {
        let token = self.ensure_fresh_token().await?;
        let url = format!("{}{}", CALENDAR_API_BASE, path);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .json(body)
            .send()
            .await
            .map_err(|e| CalendarClientError::Transport(e.to_string()))?;
        decode_response(resp).await
    }

    /// Internal helper: PATCH `path` with JSON body.
    /// Per-task code uses this for update operations
    /// (Google Calendar supports partial updates via
    /// PATCH).
    pub async fn patch_json<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &Value,
    ) -> Result<T, CalendarClientError> {
        let token = self.ensure_fresh_token().await?;
        let url = format!("{}{}", CALENDAR_API_BASE, path);
        let resp = self
            .http
            .patch(&url)
            .bearer_auth(&token)
            .json(body)
            .send()
            .await
            .map_err(|e| CalendarClientError::Transport(e.to_string()))?;
        decode_response(resp).await
    }

    /// Phase 158 — fetch the operator's writable
    /// calendar IDs, using a 5-minute session
    /// cache. On a cache hit, returns the cached
    /// list with no API round trip. On a cache
    /// miss (or expiry), GETs
    /// `/users/me/calendarList`, filters to
    /// `owner` and `writer` access roles,
    /// caches, returns.
    ///
    /// The cache lives on the client so multiple
    /// `calendar.upcoming` calls in the same
    /// session share it. Other tools that
    /// internally need the writable set can call
    /// this helper too; the substrate stays
    /// consistent.
    pub async fn writable_calendar_ids(
        &self,
    ) -> Result<Vec<String>, CalendarClientError> {
        {
            let guard = self.writable_calendars_cache.lock().await;
            if let Some((populated_at, ref ids)) = *guard {
                if populated_at.elapsed() < self.writable_calendars_cache_ttl {
                    return Ok(ids.clone());
                }
            }
        }
        let body: Value = self
            .get_json(
                "/users/me/calendarList",
                &[("fields", "items(id,accessRole)".to_string())],
            )
            .await?;
        let ids: Vec<String> = body
            .get("items")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter(|cal| {
                        let role = cal
                            .get("accessRole")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        role == "owner" || role == "writer"
                    })
                    .filter_map(|cal| {
                        cal.get("id")
                            .and_then(|v| v.as_str())
                            .map(String::from)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut guard = self.writable_calendars_cache.lock().await;
        *guard = Some((Instant::now(), ids.clone()));
        Ok(ids)
    }

    /// Test-only: peek at the writable-calendars
    /// cache. Returns `(populated_at_elapsed,
    /// ids)` if populated, None if empty.
    #[cfg(test)]
    pub async fn writable_calendars_cache_peek(
        &self,
    ) -> Option<(Duration, Vec<String>)> {
        let guard = self.writable_calendars_cache.lock().await;
        guard.as_ref().map(|(at, ids)| (at.elapsed(), ids.clone()))
    }

    /// Test-only: pre-populate the writable-
    /// calendars cache so the
    /// `writable_calendar_ids` substrate logic can
    /// be tested without a live HTTP round trip.
    #[cfg(test)]
    pub async fn writable_calendars_cache_seed(
        &self,
        ids: Vec<String>,
    ) {
        let mut guard = self.writable_calendars_cache.lock().await;
        *guard = Some((Instant::now(), ids));
    }

    /// Test-only: pre-populate with a custom
    /// `populated_at` Instant so cache-expiry
    /// tests can run without sleeping.
    #[cfg(test)]
    pub async fn writable_calendars_cache_seed_at(
        &self,
        populated_at: Instant,
        ids: Vec<String>,
    ) {
        let mut guard = self.writable_calendars_cache.lock().await;
        *guard = Some((populated_at, ids));
    }

    /// Internal helper: DELETE `path`. Per-task code uses
    /// this for delete operations. Returns the raw status
    /// code so the caller can distinguish 204 (success)
    /// from 410 Gone (already-deleted, idempotent).
    pub async fn delete(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<StatusCode, CalendarClientError> {
        let token = self.ensure_fresh_token().await?;
        let url = format!("{}{}", CALENDAR_API_BASE, path);
        let resp = self
            .http
            .delete(&url)
            .bearer_auth(&token)
            .query(query)
            .send()
            .await
            .map_err(|e| CalendarClientError::Transport(e.to_string()))?;
        let status = resp.status();
        if status.is_success() || status == StatusCode::GONE {
            Ok(status)
        } else {
            let body = resp
                .text()
                .await
                .unwrap_or_else(|_| "<no body>".to_string());
            Err(CalendarClientError::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }
}

async fn decode_response<T: DeserializeOwned>(
    resp: reqwest::Response,
) -> Result<T, CalendarClientError> {
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| CalendarClientError::Transport(e.to_string()))?;
    if !status.is_success() {
        return Err(CalendarClientError::Api {
            status: status.as_u16(),
            message: body,
        });
    }
    serde_json::from_str(&body).map_err(|e| {
        CalendarClientError::Parse(format!(
            "JSON parse from Calendar API body: {e}"
        ))
    })
}

/// Shared `Arc<CalendarClient>` alias the tools hold
/// internally. Aliased so the tool impls don't have to
/// repeat the wrapping.
pub type SharedCalendarClient = Arc<CalendarClient>;

#[cfg(test)]
mod tests {
    use super::*;

    // Env vars are process-global. Serialize every test that mutates
    // the shared `AIVYX_PA_CALENDAR_CACHE_TTL_SECS` key so `cargo test`
    // parallelism can't make one test's `remove_var` race with
    // another's `set_var`. Same pattern as aivyx-channel::passphrase.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn api_base_is_calendar_v3_endpoint() {
        // Regression catch — accidentally swapping to a
        // gmail.googleapis.com root would silently break
        // every API call.
        assert!(CALENDAR_API_BASE.contains("calendar/v3"));
    }

    #[test]
    fn calendar_client_error_display_includes_status() {
        let e = CalendarClientError::Api {
            status: 404,
            message: "not found".into(),
        };
        let rendered = e.to_string();
        assert!(rendered.contains("404"));
    }

    // ---- Phase 158 — writable_calendars cache ----

    fn make_client() -> CalendarClient {
        CalendarClient::new(
            Client::new(),
            OAuthConfig::new("id", "secret", "http://127.0.0.1:0/cb"),
            TokenSet {
                access_token: "x".to_string(),
                refresh_token: None,
                expires_at_unix_secs: 0,
                granted_scope: "scope".to_string(),
                token_type: "Bearer".to_string(),
            },
            PathBuf::from("/tmp/unused"),
        )
    }

    #[test]
    fn writable_calendars_cache_ttl_pins_to_five_minutes() {
        // Regression pin — if a future phase
        // makes this operator-tunable, the
        // INSTALL.md row + open doc need to
        // track.
        assert_eq!(WRITABLE_CALENDARS_CACHE_TTL, Duration::from_secs(300));
    }

    #[tokio::test]
    async fn writable_calendars_cache_starts_empty() {
        let client = make_client();
        assert!(client.writable_calendars_cache_peek().await.is_none());
    }

    #[tokio::test]
    async fn writable_calendars_cache_seed_populates_peek() {
        let client = make_client();
        client
            .writable_calendars_cache_seed(vec![
                "primary".to_string(),
                "team@example.com".to_string(),
            ])
            .await;
        let peeked = client.writable_calendars_cache_peek().await.unwrap();
        assert_eq!(peeked.1, vec!["primary", "team@example.com"]);
        assert!(peeked.0 < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn writable_calendars_cache_hit_skips_round_trip() {
        // Seed with a known list, then call
        // writable_calendar_ids — it should hit
        // the cache and never reach the network
        // (which would fail because the test
        // token endpoint is bogus).
        let client = make_client();
        let canned =
            vec!["primary".to_string(), "team@example.com".to_string()];
        client
            .writable_calendars_cache_seed(canned.clone())
            .await;
        let ids = client.writable_calendar_ids().await.unwrap();
        assert_eq!(ids, canned);
    }

    #[tokio::test]
    async fn writable_calendars_cache_expiry_falls_through_to_fetch() {
        // Seed with a populated_at well in the
        // past so the TTL window has expired.
        // writable_calendar_ids will fall through
        // to the network fetch — which we expect
        // to fail because the test token endpoint
        // isn't running. The point is verifying
        // the cache-expiry branch is taken (vs.
        // returning the stale cached value).
        let client = make_client();
        let stale_at = Instant::now()
            .checked_sub(Duration::from_secs(600))
            .expect("Instant arithmetic should succeed in test");
        client
            .writable_calendars_cache_seed_at(
                stale_at,
                vec!["stale@example.com".to_string()],
            )
            .await;
        let result = client.writable_calendar_ids().await;
        // We don't assert success — the test
        // env can't reach the live endpoint. We
        // assert the cache was NOT returned: if
        // the stale entry had been served, we'd
        // see Ok(["stale@example.com"]). Any
        // other outcome (Err, or a different
        // Ok) proves the expiry branch fired.
        match result {
            Ok(ids) => assert_ne!(
                ids,
                vec!["stale@example.com".to_string()],
                "stale cache served past TTL"
            ),
            Err(_) => { /* expected: network unreachable */ }
        }
    }

    // ---- Phase 171 — cache TTL knob ----

    #[test]
    fn cache_ttl_default_matches_constant() {
        let client = make_client();
        // Field is private to the impl; pin
        // via the env-helper that new() uses.
        assert_eq!(
            cache_ttl_from_env_or_default(),
            WRITABLE_CALENDARS_CACHE_TTL
        );
        // Indirect pin: the client's field
        // would equal the default when no env
        // override is set. Verified via the
        // builder-override test below.
        drop(client);
    }

    #[test]
    fn cache_ttl_builder_override_takes_effect() {
        let client = make_client()
            .with_writable_calendars_cache_ttl(Duration::from_secs(60));
        // Indirect verification: peek at the
        // cache after seeding fresh and verify
        // a sufficiently-old populated_at
        // expires.
        let client = std::sync::Arc::new(client);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            // 90 seconds ago — past the
            // overridden 60s TTL.
            let stale_at = Instant::now()
                .checked_sub(Duration::from_secs(90))
                .expect("Instant arithmetic safe");
            client
                .writable_calendars_cache_seed_at(
                    stale_at,
                    vec!["stale@example.com".to_string()],
                )
                .await;
            // The builder-overridden TTL is
            // 60s; 90s elapsed should be
            // treated as expired by
            // writable_calendar_ids.
            let result = client.writable_calendar_ids().await;
            match result {
                Ok(ids) => assert_ne!(
                    ids,
                    vec!["stale@example.com".to_string()],
                    "60s-TTL override didn't expire 90s-old cache"
                ),
                Err(_) => { /* network unreachable in test */ }
            }
        });
    }

    #[test]
    fn cache_ttl_env_var_overrides_default() {
        let _lock = env_lock();
        let key = "AIVYX_PA_CALENDAR_CACHE_TTL_SECS";
        unsafe { std::env::set_var(key, "60") };
        let ttl = cache_ttl_from_env_or_default();
        unsafe { std::env::remove_var(key) };
        assert_eq!(ttl, Duration::from_secs(60));
    }

    #[test]
    fn cache_ttl_env_var_invalid_falls_back_to_default() {
        let _lock = env_lock();
        let key = "AIVYX_PA_CALENDAR_CACHE_TTL_SECS";
        unsafe { std::env::set_var(key, "not a number") };
        let ttl = cache_ttl_from_env_or_default();
        unsafe { std::env::remove_var(key) };
        assert_eq!(ttl, WRITABLE_CALENDARS_CACHE_TTL);
    }

    #[test]
    fn cache_ttl_env_var_zero_falls_back_to_default() {
        // Zero would effectively disable the
        // cache. Treating as invalid prevents
        // operator surprise; same posture as
        // Phase 166's PDF cap env-var.
        let _lock = env_lock();
        let key = "AIVYX_PA_CALENDAR_CACHE_TTL_SECS";
        unsafe { std::env::set_var(key, "0") };
        let ttl = cache_ttl_from_env_or_default();
        unsafe { std::env::remove_var(key) };
        assert_eq!(ttl, WRITABLE_CALENDARS_CACHE_TTL);
    }
}
