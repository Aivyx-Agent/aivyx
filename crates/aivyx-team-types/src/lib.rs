//! Pure, wasm-clean mission + team **data** types (Chapter M.1).
//!
//! These are the shapes that ride on the daemon's wire protocol: the
//! [`MissionPlan`] DAG (`TeamRun { plan }`) and the [`TeamConfig`] roster
//! (`TeamRun { config }`). They were carved out of `aivyx-team` so the browser
//! Mission-Control app (a `wasm32` Dioxus client, Chapter M) can share them
//! with the daemon and never drift from the wire format.
//!
//! The split is **data vs. behavior**: the structs/enums, their serde derives,
//! and their pure helper methods live here; the runtime engine that *executes*
//! a plan (specialist pools, channels, async) stays in `aivyx-team`, which
//! re-exports everything below so its public API is unchanged.
//!
//! No `async` / storage / provider / network deps — only `aivyx-capability`
//! (itself wasm-clean), `serde`, `toml`, `thiserror`.

pub mod config;
pub mod mission;

pub use config::{DialogueConfig, TeamConfig, TeamError, TeamMember, MAX_SPECIALISTS};
pub use mission::{GateMode, MissionPlan, Step, StepKind};
// Re-exported so consumers (the Teams screen, IPC round-trip tests) can name
// `TeamMember::trust_ceiling`'s type without depending on `aivyx-capability`.
pub use aivyx_capability::TrustTier;
