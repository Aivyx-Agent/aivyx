//! OAuth 2.0 substrate for the Gmail tool process.
//!
//! Phase 123 Task 2. Operator-provided Google Cloud OAuth app per
//! Q1a (Recommended): the operator creates an OAuth client in
//! their own Google Cloud project and pastes
//! `client_id` + `client_secret` into the tool-process config
//! file. Aivyx ships no shared OAuth app.
//!
//! ## Module layout
//!
//! - [`config`] — operator-provided [`OAuthConfig`]
//!   (client_id + client_secret + redirect_uri + scopes).
//! - [`tokens`] — [`TokenSet`] (access_token, refresh_token,
//!   expires_at, scope).
//! - [`storage`] — per-tool-process token file with 0600 perms
//!   and atomic write-then-rename.
//! - [`exchange`] — auth-code → tokens; refresh-token → fresh
//!   access_token. HTTP against Google's token endpoint
//!   (`https://oauth2.googleapis.com/token`).
//!
//! ## What this module deliberately doesn't ship in Task 2
//!
//! - Browser-opening / local-loopback callback server — Task 3.
//! - Gmail API client (search, read, draft, send) — Tasks 4-7.

pub mod config;
pub mod exchange;
pub mod storage;
pub mod tokens;

pub use config::{OAuthConfig, DEFAULT_GMAIL_SCOPES};
pub use exchange::{
    exchange_code, refresh_access_token, ExchangeError, GOOGLE_AUTH_ENDPOINT,
    GOOGLE_TOKEN_ENDPOINT,
};
pub use storage::{load_tokens, save_tokens, StorageError};
pub use tokens::TokenSet;

use thiserror::Error;

/// Top-level OAuth error covering all sub-module failure modes.
/// Kept at the module root so callers can `?`-bubble a single
/// error type from any OAuth-touching path.
#[derive(Debug, Error)]
pub enum OAuthError {
    #[error("token exchange failed: {0}")]
    Exchange(#[from] ExchangeError),
    #[error("token storage failed: {0}")]
    Storage(#[from] StorageError),
}
