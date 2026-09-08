//! OAuth 2.0 substrate for the Gmail tool process.
//!
//! **Phase 129 Task 2 lift:** the OAuth implementation lives
//! in [`aivyx_google_oauth`]. This module is a thin
//! re-export shim preserving aivyx-gmail's public OAuth API
//! (`aivyx_gmail::oauth::OAuthConfig` etc) so no downstream
//! consumer code changes. The only service-specific bits
//! that stay here:
//!
//! - [`DEFAULT_GMAIL_SCOPES`] — Gmail's default OAuth scope
//!   set. Each consumer crate (`aivyx-gmail`,
//!   `aivyx-calendar`, `aivyx-drive`, …) owns its own
//!   default scopes constant; the lifted substrate has no
//!   opinion.
//! - [`storage::default_token_path`] — gmail-specific token
//!   file location resolution
//!   (`~/.aivyx-pa/tool-processes/gmail/tokens.json`).
//!
//! See the Phase 129 entry doc for the lift rationale (Q2a
//! Recommended; collapses three would-be in-tree copies of
//! the OAuth substrate to one shared crate).

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

pub use aivyx_google_oauth::OAuthError;

/// Default Gmail OAuth scopes covering the Phase 123
/// Task 4-7 tool surface.
///
/// - `gmail.readonly` — `gmail.search` + `gmail.read`.
/// - `gmail.compose` — `gmail.draft` (covers draft create
///   AND sending per Google's scope hierarchy; we use
///   `gmail.send` separately for the trust-tier
///   distinction).
/// - `gmail.send` — `gmail.send` (Trusted-gated).
///
/// Stays in this crate (NOT lifted to
/// `aivyx_google_oauth`) because each Google integration
/// has its own service-specific default scope set —
/// `aivyx-calendar` has `DEFAULT_CALENDAR_SCOPES`,
/// `aivyx-drive` has `DEFAULT_DRIVE_SCOPES`, etc. The
/// lifted substrate has no opinion on which scopes are
/// appropriate for which Google API.
pub const DEFAULT_GMAIL_SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/gmail.readonly",
    "https://www.googleapis.com/auth/gmail.compose",
    "https://www.googleapis.com/auth/gmail.send",
];
