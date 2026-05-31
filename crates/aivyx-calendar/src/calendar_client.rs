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
//
// dead_code allowance: Task 3 ships the skeleton; the
// per-tool tasks 4-8 consume every field + helper method
// here. Removed once a single tool wires up.
#[allow(dead_code)]
pub struct CalendarClient {
    http: Client,
    oauth_config: OAuthConfig,
    tokens: Arc<Mutex<TokenSet>>,
    token_path: PathBuf,
    token_endpoint: String,
}

// dead_code allowance: same posture as the struct above —
// per-tool tasks 4-8 wire every helper.
#[allow(dead_code)]
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
        }
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
    pub(crate) async fn get_json<T: DeserializeOwned>(
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
    pub(crate) async fn post_json<T: DeserializeOwned>(
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
    pub(crate) async fn patch_json<T: DeserializeOwned>(
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

    /// Internal helper: DELETE `path`. Per-task code uses
    /// this for delete operations. Returns the raw status
    /// code so the caller can distinguish 204 (success)
    /// from 410 Gone (already-deleted, idempotent).
    pub(crate) async fn delete(
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

#[allow(dead_code)]
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
