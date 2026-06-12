//! The team roster types — `TeamConfig` / `TeamMember` / `DialogueConfig` /
//! `TeamError` (+ `MAX_SPECIALISTS`).
//!
//! Moved to the wasm-clean [`aivyx_team_types`] crate (Chapter M.1) so the
//! browser Mission-Control app can share them with the daemon. Re-exported
//! here so `aivyx-team`'s API and every `crate::config::…` reference are
//! unchanged.
pub use aivyx_team_types::config::*;
