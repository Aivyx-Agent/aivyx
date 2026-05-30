//! Shared Gmail HTTP client with automatic token refresh.
//!
//! Phase 123 Task 4 — used by all Gmail tools (`gmail.search`,
//! `gmail.read`, `gmail.draft`, `gmail.send`) for their API
//! calls.
//!
//! ## Concurrency model
//!
//! Each tool invocation may run in parallel with others
//! (Amendment A6 — agent dispatches tool calls in parallel).
//! The client holds the current [`TokenSet`] behind a
//! [`tokio::sync::Mutex`]; each API call:
//!
//! 1. Locks the mutex briefly.
//! 2. Checks `tokens.needs_refresh()`.
//! 3. If yes: refreshes via the OAuth substrate, saves to
//!    disk, releases lock.
//! 4. Clones the access_token + token_type out.
//! 5. Releases the lock.
//! 6. Performs the HTTP request outside the lock with the
//!    cloned bearer header.
//!
//! Steps 1-5 are O(microseconds); the HTTP request is
//! O(network latency). Holding the lock only for steps 1-5
//! means concurrent API calls only serialize on the
//! refresh-check, not on the HTTP traffic itself.
//!
//! ## Test seam
//!
//! The Gmail API base URL is configurable per-instance so unit
//! tests can point at an in-process mock server, same pattern
//! as the OAuth substrate.

use std::path::PathBuf;
use std::sync::Arc;

use reqwest::{Client, Response};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::oauth::{
    refresh_access_token, save_tokens, ExchangeError, OAuthConfig, StorageError,
    TokenSet, GOOGLE_TOKEN_ENDPOINT,
};

/// Default Gmail API v1 base URL. Tools combine this with
/// per-endpoint paths (`/users/me/messages`, etc).
pub const GMAIL_API_BASE_URL: &str = "https://gmail.googleapis.com/gmail/v1";

#[derive(Debug, Error)]
pub enum GmailClientError {
    #[error("OAuth refresh failed: {0}")]
    Refresh(#[from] ExchangeError),
    #[error("token persistence failed: {0}")]
    Storage(#[from] StorageError),
    #[error("Gmail API HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Gmail API returned status {status}; body: {body}")]
    NonSuccessStatus { status: u16, body: String },
    #[error("Gmail API response parse failed: {0}")]
    Parse(String),
    #[error("client has no refresh token; operator must re-run `aivyx-gmail auth init`")]
    NoRefreshToken,
}

/// Shared Gmail API client. Construct one per tool process;
/// hand `Arc<GmailClient>` to each tool's `execute()`.
pub struct GmailClient {
    http: Client,
    api_base_url: String,
    token_endpoint: String,
    oauth_config: OAuthConfig,
    tokens: Mutex<TokenSet>,
    token_path: PathBuf,
}

impl GmailClient {
    /// Build a new client wrapping an already-loaded
    /// [`TokenSet`] + [`OAuthConfig`]. The tool process loads
    /// both at startup (the [`crate::auth_cli::config_file`]
    /// + [`crate::oauth::storage`] modules) and hands them in.
    pub fn new(
        http: Client,
        oauth_config: OAuthConfig,
        tokens: TokenSet,
        token_path: PathBuf,
    ) -> Self {
        Self {
            http,
            api_base_url: GMAIL_API_BASE_URL.to_string(),
            token_endpoint: GOOGLE_TOKEN_ENDPOINT.to_string(),
            oauth_config,
            tokens: Mutex::new(tokens),
            token_path,
        }
    }

    /// Override the API base URL — test seam. The default is
    /// [`GMAIL_API_BASE_URL`]; tests point at a mock server.
    pub fn with_api_base_url(mut self, url: impl Into<String>) -> Self {
        self.api_base_url = url.into();
        self
    }

    /// Override the token-refresh endpoint — test seam paired
    /// with [`Self::with_api_base_url`] for refresh-path tests.
    pub fn with_token_endpoint(mut self, url: impl Into<String>) -> Self {
        self.token_endpoint = url.into();
        self
    }

    /// Read-only access to the configured API base. Tools
    /// build per-endpoint URLs by string concatenation.
    pub fn api_base_url(&self) -> &str {
        &self.api_base_url
    }

    /// GET a Gmail API path. `path` is appended to
    /// [`Self::api_base_url`]; `query` is appended as a query
    /// string. Returns the raw [`Response`] so the caller can
    /// decode whatever response shape it expects.
    pub async fn get(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<Response, GmailClientError> {
        let bearer = self.ensure_fresh_token().await?;
        let url = format!("{}{}", self.api_base_url, path);
        let response = self
            .http
            .get(&url)
            .query(query)
            .bearer_auth(bearer)
            .send()
            .await
            .map_err(GmailClientError::Http)?;
        Ok(response)
    }

    /// POST a Gmail API path with a JSON body. Same shape as
    /// [`Self::get`].
    pub async fn post_json(
        &self,
        path: &str,
        body: &Value,
    ) -> Result<Response, GmailClientError> {
        let bearer = self.ensure_fresh_token().await?;
        let url = format!("{}{}", self.api_base_url, path);
        let response = self
            .http
            .post(&url)
            .bearer_auth(bearer)
            .json(body)
            .send()
            .await
            .map_err(GmailClientError::Http)?;
        Ok(response)
    }

    /// Decode a Gmail API response as JSON, surfacing the
    /// status + body in the error case so operators see the
    /// underlying Google error envelope (typically
    /// `{error: {code, message, errors: [...]}}`).
    pub async fn decode_json(&self, response: Response) -> Result<Value, GmailClientError> {
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(GmailClientError::Http)?;
        if !status.is_success() {
            return Err(GmailClientError::NonSuccessStatus {
                status: status.as_u16(),
                body,
            });
        }
        serde_json::from_str(&body).map_err(|e| GmailClientError::Parse(e.to_string()))
    }

    /// Refresh-check + refresh-if-needed. Returns the bearer
    /// access_token to use in the next request (cloned so the
    /// mutex is released before the HTTP call).
    async fn ensure_fresh_token(&self) -> Result<String, GmailClientError> {
        let mut guard = self.tokens.lock().await;
        if guard.needs_refresh() {
            if !guard.can_refresh() {
                return Err(GmailClientError::NoRefreshToken);
            }
            let refreshed = refresh_access_token(
                &self.http,
                &self.token_endpoint,
                &self.oauth_config,
                &guard,
            )
            .await?;
            // Persist to disk first; only update in-memory if
            // disk write succeeded so a crash mid-refresh
            // doesn't leave the in-memory state ahead of disk.
            save_tokens(&self.token_path, &refreshed).await?;
            *guard = refreshed;
        }
        Ok(guard.access_token.clone())
    }
}

/// Shared `Arc<GmailClient>` alias the tools hold internally.
/// Aliased so the tool impls don't have to repeat the wrapping.
pub type SharedGmailClient = Arc<GmailClient>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::TokenSet;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc as StdArc, Mutex as StdMutex};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    fn now_unix_secs() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    fn scratch_dir() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let tmp = std::env::var("TMPDIR")
            .or_else(|_| std::env::var("TEMP"))
            .unwrap_or_else(|_| "/tmp".to_string());
        let dir = PathBuf::from(tmp).join(format!(
            "aivyx-gmail-client-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_config() -> OAuthConfig {
        OAuthConfig::new("id.x", "secret-x", "http://127.0.0.1:0/cb")
    }

    fn fresh_tokens() -> TokenSet {
        TokenSet {
            access_token: "ya29.fresh".to_string(),
            refresh_token: Some("1//refresh".to_string()),
            expires_at_unix_secs: now_unix_secs() + 3600,
            granted_scope: "scope".to_string(),
            token_type: "Bearer".to_string(),
        }
    }

    fn expired_tokens() -> TokenSet {
        TokenSet {
            expires_at_unix_secs: now_unix_secs() - 10,
            ..fresh_tokens()
        }
    }

    /// In-process Gmail-API mock. Captures Authorization
    /// header + path + query and responds with canned JSON.
    /// Returns `(base_url, captured)`. `captured` becomes a
    /// `(auth_header, path_and_query)` pair after one request.
    async fn spawn_mock_api(
        canned_status: u16,
        canned_body: &'static str,
    ) -> (String, StdArc<StdMutex<(String, String)>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let captured: StdArc<StdMutex<(String, String)>> =
            StdArc::new(StdMutex::new(("".into(), "".into())));
        let captured_clone = StdArc::clone(&captured);
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let (read_half, mut write_half) = sock.split();
            let mut reader = BufReader::new(read_half);
            let mut request_line = String::new();
            reader.read_line(&mut request_line).await.unwrap();
            let path_and_query = request_line
                .split_whitespace()
                .nth(1)
                .unwrap_or("")
                .to_string();
            let mut auth_header = String::new();
            let mut content_length: usize = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap() == 0 {
                    break;
                }
                if line == "\r\n" {
                    break;
                }
                // Case-insensitive header-name match; preserve
                // the original VALUE (which is case-sensitive
                // for the bearer header).
                if let Some(idx) = line.find(':') {
                    let name_lower = line[..idx].to_ascii_lowercase();
                    let value = &line[idx + 1..];
                    if name_lower == "authorization" {
                        auth_header = value.trim().to_string();
                    } else if name_lower == "content-length" {
                        content_length = value.trim().parse().unwrap_or(0);
                    }
                }
            }
            if content_length > 0 {
                let mut body = vec![0u8; content_length];
                let _ = reader.read_exact(&mut body).await;
            }
            *captured_clone.lock().unwrap() = (auth_header, path_and_query);
            let phrase = if canned_status == 200 { "OK" } else { "Err" };
            let response = format!(
                "HTTP/1.1 {canned_status} {phrase}\r\n\
                 Content-Type: application/json\r\n\
                 Content-Length: {}\r\n\
                 Connection: close\r\n\
                 \r\n\
                 {}",
                canned_body.len(),
                canned_body,
            );
            write_half.write_all(response.as_bytes()).await.unwrap();
            write_half.flush().await.unwrap();
        });
        (format!("http://127.0.0.1:{port}"), captured)
    }

    #[tokio::test]
    async fn get_attaches_bearer_header_from_fresh_token() {
        let dir = scratch_dir();
        let path = dir.join("tokens.json");
        save_tokens(&path, &fresh_tokens()).await.unwrap();
        let (base, captured) = spawn_mock_api(200, r#"{"ok":true}"#).await;
        let client = GmailClient::new(Client::new(), sample_config(), fresh_tokens(), path)
            .with_api_base_url(base);
        let response = client.get("/users/me/messages", &[("q", "in:inbox")])
            .await
            .expect("get");
        let body = client.decode_json(response).await.expect("decode");
        assert_eq!(body["ok"], true);
        let (auth, path_q) = captured.lock().unwrap().clone();
        assert_eq!(auth, "Bearer ya29.fresh", "{auth}");
        assert!(path_q.contains("q=in%3Ainbox"), "{path_q}");
        assert!(path_q.contains("/users/me/messages"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn decode_json_surfaces_non_success_status_with_body() {
        let dir = scratch_dir();
        let path = dir.join("tokens.json");
        save_tokens(&path, &fresh_tokens()).await.unwrap();
        let canned = r#"{"error":{"code":401,"message":"Invalid credentials"}}"#;
        let (base, _) = spawn_mock_api(401, canned).await;
        let client = GmailClient::new(Client::new(), sample_config(), fresh_tokens(), path)
            .with_api_base_url(base);
        let response = client.get("/users/me/messages", &[]).await.expect("get");
        match client.decode_json(response).await {
            Err(GmailClientError::NonSuccessStatus { status, body }) => {
                assert_eq!(status, 401);
                assert!(body.contains("Invalid credentials"));
            }
            other => panic!("expected NonSuccessStatus; got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn ensure_fresh_token_refreshes_when_expired_and_persists() {
        let dir = scratch_dir();
        let path = dir.join("tokens.json");
        save_tokens(&path, &expired_tokens()).await.unwrap();
        // Mock token endpoint: returns fresh access token, no
        // new refresh_token (Google's typical refresh response).
        let canned = r#"{"access_token":"ya29.refreshed","expires_in":3600,"token_type":"Bearer"}"#;
        let (token_base, _) = spawn_mock_api(200, canned).await;
        // Mock API endpoint that just echoes 200; we only care
        // about whether the bearer is the refreshed one.
        let (api_base, captured_api) =
            spawn_mock_api(200, r#"{"ok":true}"#).await;
        let client = GmailClient::new(
            Client::new(),
            sample_config(),
            expired_tokens(),
            path.clone(),
        )
        .with_api_base_url(api_base)
        // The mock token endpoint accepts POST to root; we
        // serve only one request so any path is fine.
        .with_token_endpoint(token_base);
        let response = client.get("/users/me/messages", &[]).await.expect("get");
        client.decode_json(response).await.expect("decode");
        // Verify the API saw the refreshed bearer (not the
        // expired one).
        let (auth, _) = captured_api.lock().unwrap().clone();
        assert_eq!(auth, "Bearer ya29.refreshed");
        // Verify disk got the new tokens persisted.
        let on_disk = crate::oauth::load_tokens(&path)
            .await
            .expect("load")
            .expect("present");
        assert_eq!(on_disk.access_token, "ya29.refreshed");
        // Refresh-token preserved.
        assert_eq!(on_disk.refresh_token, Some("1//refresh".to_string()));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn ensure_fresh_token_errors_when_no_refresh_token_available() {
        let dir = scratch_dir();
        let path = dir.join("tokens.json");
        let mut tokens = expired_tokens();
        tokens.refresh_token = None;
        save_tokens(&path, &tokens).await.unwrap();
        let client = GmailClient::new(
            Client::new(),
            sample_config(),
            tokens,
            path,
        );
        match client.get("/users/me/messages", &[]).await {
            Err(GmailClientError::NoRefreshToken) => {}
            other => panic!("expected NoRefreshToken; got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn post_json_attaches_bearer_and_sends_body() {
        let dir = scratch_dir();
        let path = dir.join("tokens.json");
        save_tokens(&path, &fresh_tokens()).await.unwrap();
        let (base, captured) = spawn_mock_api(200, r#"{"id":"draft-x"}"#).await;
        let client = GmailClient::new(
            Client::new(),
            sample_config(),
            fresh_tokens(),
            path,
        )
        .with_api_base_url(base);
        let body = serde_json::json!({"message": {"raw": "encoded-mime"}});
        let response =
            client.post_json("/users/me/drafts", &body).await.expect("post");
        let decoded = client.decode_json(response).await.expect("decode");
        assert_eq!(decoded["id"], "draft-x");
        let (auth, _) = captured.lock().unwrap().clone();
        assert_eq!(auth, "Bearer ya29.fresh");
        std::fs::remove_dir_all(&dir).ok();
    }
}
