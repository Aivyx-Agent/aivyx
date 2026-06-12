//! The mission DAG types — `MissionPlan` / `Step` / `StepKind` / `GateMode`.
//!
//! Moved to the wasm-clean [`aivyx_team_types`] crate (Chapter M.1) so the
//! browser Mission-Control app can share them with the daemon. Re-exported
//! here so `aivyx-team`'s API and every `crate::mission::…` reference are
//! unchanged.
pub use aivyx_team_types::mission::*;
