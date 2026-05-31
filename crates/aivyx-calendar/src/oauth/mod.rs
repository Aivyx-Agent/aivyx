//! OAuth 2.0 substrate for the Calendar tool process.
//!
//! **Phase 128 Q2a copy from `aivyx-gmail/src/oauth/`.**
//! Calendar uses the SAME Google OAuth flow as Gmail —
//! just a different scope (`auth/calendar` instead of
//! `auth/gmail.*`). Per Phase 128 Q2a Recommended, the
//! OAuth substrate is inline-copied rather than lifted to
//! a shared `aivyx-google-oauth` crate; the lift becomes
//! worthwhile at the third Google integration when N=3
//! triggers the substrate work (same threshold pattern as
//! the harness lift Phase 128 Task 2 just executed). Until
//! then, two in-tree copies (gmail + this one) are
//! honest tech debt documented in PHASE_128.md.
//!
//! Phase 123 Task 2 was the original implementation;
//! consult `aivyx-gmail/src/oauth/mod.rs` for the
//! historical sign-off (Q1a operator-provided OAuth app).
//! That sign-off applies here verbatim — Calendar uses
//! the same operator-provided Google Cloud project +
//! client_id + client_secret (typically the SAME GCP
//! project as the operator's Gmail OAuth client, just
//! with the Calendar API also enabled).
//!
//! ## Module layout
//!
//! - [`config`] — operator-provided [`OAuthConfig`]
//!   (client_id + client_secret + redirect_uri + scopes).
//! - [`tokens`] — [`TokenSet`] (access_token, refresh_token,
//!   expires_at, scope).
//! - [`storage`] — per-tool-process token file with 0600 perms
//!   and atomic write-then-rename. Path:
//!   `~/.aivyx/tool-processes/calendar/tokens.json`.
//! - [`exchange`] — auth-code → tokens; refresh-token → fresh
//!   access_token. HTTP against Google's token endpoint
//!   (`https://oauth2.googleapis.com/token`).

pub mod config;
pub mod exchange;
pub mod storage;
pub mod tokens;

pub use config::{OAuthConfig, DEFAULT_CALENDAR_SCOPES};
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
