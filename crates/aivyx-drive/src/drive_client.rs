//! Google Drive v3 REST API client.
//!
//! Phase 129 Task 3 — skeleton; per-tool tasks 4-10 fill
//! in the actual HTTP operations. Mirrors `aivyx-calendar`'s
//! `CalendarClient` shape (token-authenticated reqwest
//! wrapper with refresh-on-expiry posture) but with two
//! distinct API base URLs because Drive splits its REST
//! surface across two:
//!
//! - **Metadata + querying:** `https://www.googleapis.com/drive/v3`
//!   for `files.list`, `files.get`, `files.create`
//!   (metadata-only), `files.update`, `files.delete`.
//! - **Media + upload:** `https://www.googleapis.com/upload/drive/v3`
//!   for `files.create` (with content body) and
//!   `files.get?alt=media` for content downloads.

use std::path::PathBuf;
use std::sync::Arc;

use reqwest::{Client, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::Value;
use thiserror::Error;
use tokio::sync::Mutex;

use aivyx_google_oauth::{
    refresh_access_token, save_tokens, ExchangeError, OAuthConfig, StorageError,
    TokenSet, GOOGLE_TOKEN_ENDPOINT,
};

pub const DRIVE_API_BASE: &str = "https://www.googleapis.com/drive/v3";
pub const DRIVE_UPLOAD_BASE: &str = "https://www.googleapis.com/upload/drive/v3";

#[derive(Debug, Error)]
pub enum DriveClientError {
    #[error("HTTP transport error: {0}")]
    Transport(String),
    #[error("Drive API returned HTTP {status}: {message}")]
    Api { status: u16, message: String },
    #[error("response parse error: {0}")]
    Parse(String),
    #[error("token refresh failed: {0}")]
    TokenRefresh(#[from] ExchangeError),
    #[error("token storage failed during refresh persist: {0}")]
    TokenStorage(#[from] StorageError),
    #[error("no refresh_token available; re-run `aivyx-drive auth init`")]
    NoRefreshToken,
    #[error("input validation: {0}")]
    InvalidInput(String),
    #[error("file content exceeds inline cap: {size} bytes > {cap} bytes")]
    ContentTooLarge { size: usize, cap: usize },
}

pub struct DriveClient {
    http: Client,
    oauth_config: OAuthConfig,
    tokens: Arc<Mutex<TokenSet>>,
    token_path: PathBuf,
    token_endpoint: String,
}

impl DriveClient {
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

    /// Same refresh-on-expiry posture as `CalendarClient` /
    /// `GmailClient`. Persists to disk before updating
    /// in-memory state.
    async fn ensure_fresh_token(&self) -> Result<String, DriveClientError> {
        let mut guard = self.tokens.lock().await;
        if guard.needs_refresh() {
            if !guard.can_refresh() {
                return Err(DriveClientError::NoRefreshToken);
            }
            let refreshed = refresh_access_token(
                &self.http,
                &self.token_endpoint,
                &self.oauth_config,
                &guard,
            )
            .await?;
            save_tokens(&self.token_path, &refreshed).await?;
            *guard = refreshed;
        }
        Ok(guard.access_token.clone())
    }

    /// GET `{DRIVE_API_BASE}{path}` with query params; parse
    /// JSON body.
    pub async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T, DriveClientError> {
        let token = self.ensure_fresh_token().await?;
        let url = format!("{}{}", DRIVE_API_BASE, path);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .query(query)
            .send()
            .await
            .map_err(|e| DriveClientError::Transport(e.to_string()))?;
        decode_json(resp).await
    }

    /// POST `{DRIVE_API_BASE}{path}` with JSON body; parse
    /// JSON response. Used for metadata-only file creation
    /// (e.g. folders, where no media body is needed).
    pub async fn post_json<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &Value,
    ) -> Result<T, DriveClientError> {
        let token = self.ensure_fresh_token().await?;
        let url = format!("{}{}", DRIVE_API_BASE, path);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .json(body)
            .send()
            .await
            .map_err(|e| DriveClientError::Transport(e.to_string()))?;
        decode_json(resp).await
    }

    /// GET `{DRIVE_API_BASE}{path}?alt=media` returning the
    /// raw response bytes. Used by `drive.download_file`.
    pub async fn get_media(
        &self,
        path: &str,
        extra_query: &[(&str, String)],
    ) -> Result<Vec<u8>, DriveClientError> {
        let token = self.ensure_fresh_token().await?;
        let url = format!("{}{}", DRIVE_API_BASE, path);
        let mut query: Vec<(&str, String)> =
            vec![("alt", "media".to_string())];
        query.extend(extra_query.iter().map(|(k, v)| (*k, v.clone())));
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .query(&query)
            .send()
            .await
            .map_err(|e| DriveClientError::Transport(e.to_string()))?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp
                .text()
                .await
                .unwrap_or_else(|_| "<no body>".to_string());
            return Err(DriveClientError::Api {
                status: status.as_u16(),
                message: body,
            });
        }
        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| DriveClientError::Transport(e.to_string()))
    }

    /// Multipart POST to `{DRIVE_UPLOAD_BASE}{path}` with a
    /// JSON metadata part + a content part. Used by
    /// `drive.upload_file`. Caller provides serialized
    /// metadata + raw content bytes + a content mime type.
    pub async fn post_multipart<T: DeserializeOwned>(
        &self,
        path: &str,
        metadata_json: Value,
        content_bytes: Vec<u8>,
        content_mime: &str,
    ) -> Result<T, DriveClientError> {
        let token = self.ensure_fresh_token().await?;
        let url = format!("{}{}", DRIVE_UPLOAD_BASE, path);
        let form = reqwest::multipart::Form::new()
            .part(
                "metadata",
                reqwest::multipart::Part::text(metadata_json.to_string())
                    .mime_str("application/json")
                    .map_err(|e| DriveClientError::Transport(e.to_string()))?,
            )
            .part(
                "media",
                reqwest::multipart::Part::bytes(content_bytes)
                    .mime_str(content_mime)
                    .map_err(|e| DriveClientError::Transport(e.to_string()))?,
            );
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .query(&[("uploadType", "multipart")])
            .multipart(form)
            .send()
            .await
            .map_err(|e| DriveClientError::Transport(e.to_string()))?;
        decode_json(resp).await
    }

    /// DELETE `{DRIVE_API_BASE}{path}`. Returns the raw
    /// status so the caller can distinguish 204 from 410
    /// Gone (idempotent delete).
    pub async fn delete(&self, path: &str) -> Result<StatusCode, DriveClientError> {
        let token = self.ensure_fresh_token().await?;
        let url = format!("{}{}", DRIVE_API_BASE, path);
        let resp = self
            .http
            .delete(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| DriveClientError::Transport(e.to_string()))?;
        let status = resp.status();
        if status.is_success() || status == StatusCode::GONE {
            Ok(status)
        } else {
            let body = resp
                .text()
                .await
                .unwrap_or_else(|_| "<no body>".to_string());
            Err(DriveClientError::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }
}

async fn decode_json<T: DeserializeOwned>(
    resp: reqwest::Response,
) -> Result<T, DriveClientError> {
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| DriveClientError::Transport(e.to_string()))?;
    if !status.is_success() {
        return Err(DriveClientError::Api {
            status: status.as_u16(),
            message: body,
        });
    }
    serde_json::from_str(&body)
        .map_err(|e| DriveClientError::Parse(format!("Drive API body: {e}")))
}

pub type SharedDriveClient = Arc<DriveClient>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_base_is_drive_v3() {
        assert!(DRIVE_API_BASE.contains("drive/v3"));
        assert!(!DRIVE_API_BASE.contains("upload"));
    }

    #[test]
    fn upload_base_is_distinct_from_api_base() {
        assert!(DRIVE_UPLOAD_BASE.contains("upload/drive/v3"));
        assert_ne!(DRIVE_API_BASE, DRIVE_UPLOAD_BASE);
    }

    #[test]
    fn drive_client_error_display_includes_status() {
        let e = DriveClientError::Api {
            status: 404,
            message: "not found".into(),
        };
        assert!(e.to_string().contains("404"));
    }

    #[test]
    fn content_too_large_error_reports_both_size_and_cap() {
        let e = DriveClientError::ContentTooLarge {
            size: 20_000_000,
            cap: 10_000_000,
        };
        let s = e.to_string();
        assert!(s.contains("20000000"));
        assert!(s.contains("10000000"));
    }
}
