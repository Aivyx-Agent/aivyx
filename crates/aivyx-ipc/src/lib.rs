//! The daemon ↔ client **wire protocol** (Chapter M) — wasm-clean shared types.
//!
//! Carved out of `aivyx-channel` (the daemon, which can't target `wasm32`) so
//! the browser Mission-Control app (a Dioxus `wasm32` client) and the daemon
//! serialize from **one source of wire truth** — adding a variant updates both
//! sides at once, with no hand-maintained JSON mirror to drift.
//!
//! M.2a seeds the crate with the **team-mission** types (the headline of
//! Chapter M's Mission Control). The full IPC envelope (`FrontendMessage` /
//! `QueryPayload` / `QueryResponsePayload` …) and the other embedded data
//! types land in later M.2 slices, after each is made wasm-clean.
//!
//! Deps: `aivyx-team-types` (itself wasm-clean) + `serde` only.

pub mod team_mission;

pub use team_mission::{
    TeamMissionPhase, TeamMissionRecord, TeamMissionView, TeamStepState, TeamStepView,
};
