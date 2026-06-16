//! Google People API v1 client.
//!
//! Token-authenticated reqwest wrapper around
//! `https://people.googleapis.com/v1/...`, mirroring
//! `aivyx-drive`'s `DriveClient` refresh-on-expiry posture.
//! Unlike Drive, the People API lives on a single host, so
//! there is one base URL — exposed as a field with a
//! [`ContactsClient::with_api_base_url`] test seam so the CT.3 /
//! CT.4 tool tests can point it at an in-process mock server
//! (the same pattern Gmail's `with_api_base_url` established).
//!
//! Verbs the six-tool surface needs:
//! - `get_json`   — `contacts.search`, `contacts.list`,
//!   `contacts.get`.
//! - `post_json`  — `contacts.create` (`people:createContact`).
//! - `patch_json` — `contacts.update`
//!   (`people/*:updateContact`).
//! - `delete`     — `contacts.delete`
//!   (`people/*:deleteContact`).

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

/// Google People API v1 base URL.
pub const PEOPLE_API_BASE: &str = "https://people.googleapis.com/v1";

#[derive(Debug, Error)]
pub enum ContactsClientError {
    #[error("HTTP transport error: {0}")]
    Transport(String),
    #[error("People API returned HTTP {status}: {message}")]
    Api { status: u16, message: String },
    #[error("response parse error: {0}")]
    Parse(String),
    #[error("token refresh failed: {0}")]
    TokenRefresh(#[from] ExchangeError),
    #[error("token storage failed during refresh persist: {0}")]
    TokenStorage(#[from] StorageError),
    #[error("no refresh_token available; re-run `aivyx-contacts auth init`")]
    NoRefreshToken,
    #[error("input validation: {0}")]
    InvalidInput(String),
}

pub struct ContactsClient {
    http: Client,
    oauth_config: OAuthConfig,
    tokens: Arc<Mutex<TokenSet>>,
    token_path: PathBuf,
    token_endpoint: String,
    api_base: String,
}

impl ContactsClient {
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
            api_base: PEOPLE_API_BASE.to_string(),
        }
    }

    /// Test seam — override the People API base URL to point at
    /// an in-process mock server. Same pattern as Gmail's
    /// `with_api_base_url`. CT.3 / CT.4 tool tests use this.
    pub fn with_api_base_url(mut self, base: impl Into<String>) -> Self {
        self.api_base = base.into();
        self
    }

    /// Same refresh-on-expiry posture as `DriveClient` /
    /// `CalendarClient`. Persists to disk before updating
    /// in-memory state.
    async fn ensure_fresh_token(&self) -> Result<String, ContactsClientError> {
        let mut guard = self.tokens.lock().await;
        if guard.needs_refresh() {
            if !guard.can_refresh() {
                return Err(ContactsClientError::NoRefreshToken);
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

    /// GET `{api_base}{path}` with query params; parse JSON body.
    pub async fn get_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
    ) -> Result<T, ContactsClientError> {
        let token = self.ensure_fresh_token().await?;
        let url = format!("{}{}", self.api_base, path);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&token)
            .query(query)
            .send()
            .await
            .map_err(|e| ContactsClientError::Transport(e.to_string()))?;
        decode_json(resp).await
    }

    /// POST `{api_base}{path}` with JSON body + query params;
    /// parse JSON response. Used by `contacts.create`.
    pub async fn post_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
        body: &Value,
    ) -> Result<T, ContactsClientError> {
        let token = self.ensure_fresh_token().await?;
        let url = format!("{}{}", self.api_base, path);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .query(query)
            .json(body)
            .send()
            .await
            .map_err(|e| ContactsClientError::Transport(e.to_string()))?;
        decode_json(resp).await
    }

    /// PATCH `{api_base}{path}` with JSON body + query params;
    /// parse JSON response. Used by `contacts.update`
    /// (`updatePersonFields` is passed as a query param).
    pub async fn patch_json<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(&str, String)],
        body: &Value,
    ) -> Result<T, ContactsClientError> {
        let token = self.ensure_fresh_token().await?;
        let url = format!("{}{}", self.api_base, path);
        let resp = self
            .http
            .patch(&url)
            .bearer_auth(&token)
            .query(query)
            .json(body)
            .send()
            .await
            .map_err(|e| ContactsClientError::Transport(e.to_string()))?;
        decode_json(resp).await
    }

    /// DELETE `{api_base}{path}`. Returns the raw status so the
    /// caller can treat 404/410 as already-gone (idempotent
    /// delete). Used by `contacts.delete`.
    pub async fn delete(
        &self,
        path: &str,
    ) -> Result<StatusCode, ContactsClientError> {
        let token = self.ensure_fresh_token().await?;
        let url = format!("{}{}", self.api_base, path);
        let resp = self
            .http
            .delete(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| ContactsClientError::Transport(e.to_string()))?;
        let status = resp.status();
        if status.is_success()
            || status == StatusCode::GONE
            || status == StatusCode::NOT_FOUND
        {
            Ok(status)
        } else {
            let body = resp
                .text()
                .await
                .unwrap_or_else(|_| "<no body>".to_string());
            Err(ContactsClientError::Api {
                status: status.as_u16(),
                message: body,
            })
        }
    }
}

async fn decode_json<T: DeserializeOwned>(
    resp: reqwest::Response,
) -> Result<T, ContactsClientError> {
    let status = resp.status();
    let body = resp
        .text()
        .await
        .map_err(|e| ContactsClientError::Transport(e.to_string()))?;
    if !status.is_success() {
        return Err(ContactsClientError::Api {
            status: status.as_u16(),
            message: body,
        });
    }
    serde_json::from_str(&body)
        .map_err(|e| ContactsClientError::Parse(format!("People API body: {e}")))
}

pub type SharedContactsClient = Arc<ContactsClient>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_base_targets_people_v1() {
        assert!(PEOPLE_API_BASE.contains("people.googleapis.com"));
        assert!(PEOPLE_API_BASE.ends_with("/v1"));
    }

    #[test]
    fn contacts_client_error_display_includes_status() {
        let e = ContactsClientError::Api {
            status: 404,
            message: "not found".into(),
        };
        assert!(e.to_string().contains("404"));
    }
}
