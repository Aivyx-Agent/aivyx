//! # aivyx-calendar
//!
//! Google Calendar third-party tool process for Aivyx.
//! Chapter F #2 — Phase 128. Ships as a separate binary the
//! operator installs and wires into `aivyx.toml` via
//! `[[tool_process]]`. Per PRODUCT.md P10 (substrate is closed
//! at thirteen tools forever; calendar is third-party
//! territory).
//!
//! ## Layout (Phase 128)
//!
//! - [`oauth`] — OAuth 2.0 substrate. **Inline copy from
//!   `aivyx-gmail` per Phase 128 Q2a Recommended;** the lift
//!   to a shared `aivyx-google-oauth` crate becomes
//!   worthwhile at the third Google integration (N=3
//!   threshold). Same operator-provided Google Cloud
//!   OAuth app pattern; auth-code exchange; refresh-token
//!   flow; per-tool-process token file storage at
//!   `~/.aivyx/tool-processes/calendar/tokens.json` (0600
//!   perms).
//! - [`auth_cli`] — `aivyx-calendar auth init / status /
//!   revoke` operator-facing CLI subcommand surface.
//! - [`calendar_client`] — Google Calendar v3 REST API
//!   client (token-authenticated reqwest wrapper around
//!   `https://www.googleapis.com/calendar/v3/...`).
//! - [`tools`] — five `aivyx_core::Tool` impls per Phase 128
//!   Q3b (operator-picked richer surface):
//!   - `calendar.list_events` (Task 4; `calendar.read`)
//!   - `calendar.get_event` (Task 5; `calendar.read`)
//!   - `calendar.create_event` (Task 6; `calendar.write`,
//!     CEILING_TRUSTED)
//!   - `calendar.update_event` (Task 7; `calendar.write`,
//!     CEILING_TRUSTED)
//!   - `calendar.delete_event` (Task 8; `calendar.write`,
//!     CEILING_TRUSTED)
//!
//! Multi-tool harness consumed from
//! `aivyx_tool::multi_harness` (Phase 128 Task 2 lift). No
//! in-tree harness copy — Calendar was the third would-be
//! consumer; the lift collapsed all three (gmail + toolkit
//! + this crate's would-be copy) to one shared substrate.
//!
//! ## Auth model
//!
//! Operator-provided OAuth app — same posture as Gmail
//! (Phase 123 Q1a Recommended), reused here per Phase 128
//! Q2a. The operator typically uses the SAME GCP project +
//! client_id + client_secret for both Gmail and Calendar
//! tool processes; they just enable the Calendar API
//! alongside Gmail in the project, register both
//! `https://www.googleapis.com/auth/gmail.*` and
//! `https://www.googleapis.com/auth/calendar` scopes on
//! the OAuth consent screen, and run `aivyx-calendar auth
//! init` to grant the Calendar scope independently.

pub mod auth_cli;
pub mod calendar_client;
pub mod oauth;
pub mod relative_time;
pub mod tools;

pub use calendar_client::{CalendarClient, CalendarClientError};
pub use oauth::{
    OAuthConfig, OAuthError, TokenSet, DEFAULT_CALENDAR_SCOPES,
    GOOGLE_AUTH_ENDPOINT, GOOGLE_TOKEN_ENDPOINT,
};

// Re-export the lifted multi-tool harness so consumers
// (main.rs + downstream) can use the same import surface
// as aivyx-gmail / aivyx-toolkit.
pub use aivyx_tool::multi_harness::{run_multi_tool_subprocess, HarnessError};
