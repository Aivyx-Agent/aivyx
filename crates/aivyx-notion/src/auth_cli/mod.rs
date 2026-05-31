//! Minimal CLI surface for `aivyx-notion`.
//!
//! Phase 130 Task 2. Notion's Integration token auth is
//! much simpler than the OAuth-flavored flows in gmail /
//! calendar / drive — there's no token exchange, no
//! refresh dance, no callback flow. Operators paste the
//! token from Notion's Integrations dashboard directly
//! into config.toml and they're done.
//!
//! As a result this auth_cli is much smaller than the
//! gmail/calendar/drive ones — just three commands:
//!
//! - `aivyx-notion auth status` — does the token in
//!   config.toml work?
//! - `aivyx-notion auth check` — actively poll Notion's
//!   `/users/me` endpoint to confirm the token has API
//!   access.
//! - `aivyx-notion help` — usage.
//!
//! The slimness of this module is an empirical test
//! signal for the Phase 130 honest-scope-risk
//! "auth_cli lift posture" — if wrapping this in the
//! OAuth-shaped helpers (init / status / revoke) would
//! have been awkward, the lift to a shared substrate
//! becomes more justified. Phase 130 reports at exit.

pub mod cli;
pub mod config_file;
pub mod status;
