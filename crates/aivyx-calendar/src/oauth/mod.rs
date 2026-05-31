//! OAuth 2.0 substrate for the Calendar tool process.
//!
//! **Phase 129 Task 2 lift:** the OAuth implementation lives
//! in [`aivyx_google_oauth`]. This module is a thin
//! re-export shim preserving aivyx-calendar's public OAuth
//! API (`aivyx_calendar::oauth::OAuthConfig` etc) so no
//! downstream consumer code changes. The only service-
//! specific bits that stay here:
//!
//! - [`DEFAULT_CALENDAR_SCOPES`] — Calendar's default OAuth
//!   scope set.
//! - [`storage::default_token_path`] — calendar-specific
//!   token file location resolution
//!   (`~/.aivyx/tool-processes/calendar/tokens.json`).
//!
//! See Phase 128 mod.rs preamble for the original inline-copy
//! tracking; that posture's N=3 trigger fired at Phase 129
//! with aivyx-drive.

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

/// Default Calendar OAuth scopes covering the Phase 128
/// Task 4-8 tool surface (5 tools — Q3b operator-picked).
///
/// Stays in this crate (NOT lifted) for the same reason
/// `DEFAULT_GMAIL_SCOPES` stays in `aivyx-gmail`: each
/// Google integration has its own service-specific scope
/// set. Per Phase 128 sign-off, the default is the BROAD
/// `auth/calendar` rather than narrower
/// `auth/calendar.events` — fewer "re-auth with new scope"
/// loops for operators.
pub const DEFAULT_CALENDAR_SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/calendar",
];
