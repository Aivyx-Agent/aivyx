//! # aivyx-google-oauth
//!
//! Shared Google OAuth substrate for Aivyx tool processes.
//! Phase 129 Task 2 lift from `aivyx-gmail::oauth::*`.
//!
//! ## Lift history
//!
//! Phase 123 (Gmail) shipped the OAuth substrate inline in
//! `aivyx-gmail/src/oauth/`. Phase 128 (Calendar) Q2a
//! Recommended copied the substrate inline a second time
//! into `aivyx-calendar/src/oauth/` — documented as
//! N=2 tech debt at the time, with the explicit posture
//! that "the OAuth lift becomes worthwhile at three copies
//! (gmail + calendar + future Google integration)." Phase
//! 129 (Drive) is that moment: Drive is the third Google
//! integration; lifting the substrate to this shared crate
//! collapses three would-be in-tree copies (gmail +
//! calendar + drive) to one before the third lands.
//!
//! ## What's lifted
//!
//! - [`OAuthConfig`] — operator-supplied client_id +
//!   client_secret + redirect_uri + scopes.
//! - [`TokenSet`] — access_token + optional refresh_token +
//!   expires_at + granted_scope + token_type. Includes
//!   refresh-readiness helpers
//!   ([`TokenSet::needs_refresh`],
//!   [`TokenSet::can_refresh`]).
//! - [`exchange_code`] / [`refresh_access_token`] — HTTP
//!   round-trips against Google's token endpoint.
//! - [`save_tokens`] / [`load_tokens`] — token file
//!   storage with `0600` perms + atomic
//!   write-then-rename. Path is a parameter so each
//!   consumer's `~/.aivyx-pa/tool-processes/{service}/tokens.json`
//!   works without service-specific code here.
//! - [`GOOGLE_AUTH_ENDPOINT`] / [`GOOGLE_TOKEN_ENDPOINT`] —
//!   the standard Google OAuth 2.0 endpoints.
//! - [`OAuthError`] — top-level enum covering exchange +
//!   storage failures so callers can `?`-bubble.
//!
//! ## What's NOT lifted (intentionally)
//!
//! - **Service-specific default scope sets.** Each consumer
//!   crate provides its own
//!   `DEFAULT_X_SCOPES: &[&str]` constant (gmail keeps
//!   `DEFAULT_GMAIL_SCOPES`; calendar keeps
//!   `DEFAULT_CALENDAR_SCOPES`; drive ships
//!   `DEFAULT_DRIVE_SCOPES`). The substrate has no opinion
//!   on which scopes are appropriate for which Google API.
//!
//! - **Per-binary CLI dispatchers (`aivyx-gmail auth
//!   init` argv parsing etc).** Each binary's `auth_cli/`
//!   module still owns its CLI surface. The OAuth flow
//!   bodies (consent URL building, callback-loopback
//!   server, browser-open) lift here as
//!   service-name-parameterized helpers; the binaries
//!   wrap them with their CLI-arg parsing.
//!
//! - **Per-binary default config / token file paths.**
//!   Each consumer's `default_config_path()` /
//!   `default_token_path()` stays in its own crate
//!   because the service-name segment of the path is
//!   per-service. The path-handling logic itself is
//!   trivial enough not to warrant a parameterized
//!   helper here.
//!
//! ## Post-lift consumer shape
//!
//! `aivyx-gmail/src/oauth/*` and
//! `aivyx-calendar/src/oauth/*` become thin re-export
//! shims preserving each crate's public API
//! (`aivyx_gmail::OAuthConfig`,
//! `aivyx_calendar::TokenSet`, etc) so no downstream
//! consumer code needs to change. `aivyx-drive` consumes
//! the lifted crate directly from the start.

pub mod config;
pub mod exchange;
pub mod storage;
pub mod tokens;

pub use config::OAuthConfig;
pub use exchange::{
    exchange_code, refresh_access_token, ExchangeError, GOOGLE_AUTH_ENDPOINT,
    GOOGLE_TOKEN_ENDPOINT,
};
pub use storage::{load_tokens, save_tokens, StorageError};
pub use tokens::TokenSet;

use thiserror::Error;

/// Top-level OAuth error covering all sub-module failure
/// modes. Kept at the crate root so callers can `?`-bubble
/// a single error type from any OAuth-touching path.
#[derive(Debug, Error)]
pub enum OAuthError {
    #[error("token exchange failed: {0}")]
    Exchange(#[from] ExchangeError),
    #[error("token storage failed: {0}")]
    Storage(#[from] StorageError),
}
