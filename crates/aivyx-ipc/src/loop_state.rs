//! Autonomous-loop run state (Phase 173) — moved to `aivyx-ipc` in M.2b.
//!
//! The live state of one loop run, shared between the driver and the daemon's
//! `loop start|stop|status` IPC handlers, and carried on the wire by
//! `QueryResponsePayload::LoopStatus`. Pure data; the driver behavior
//! (`SharedLoopState`, `run_loop_driver`, …) stays in `aivyx-channel`.

use serde::{Deserialize, Serialize};

/// One run's live state. Shared between the driver and the daemon
/// IPC handlers (start / stop / status).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoopRunState {
    /// Whether a run is currently executing iterations.
    pub active: bool,
    /// How many iterations the current (or last) run has fired.
    pub iteration: u32,
    /// The per-run hard cap. A run stops once `iteration`
    /// reaches this.
    pub max_iterations: u32,
    /// Wall-clock (unix ms) the current run started, or `0`.
    pub started_at_unix_ms: u64,
    /// Why the last run ended (for `aivyx loop status`). `None`
    /// until a run has finished at least once.
    pub last_stop_reason: Option<String>,
    /// Phase 177 — the run-window token total at the last
    /// iteration boundary (the same window-sum the Phase 176
    /// budget uses). Surfaced by `aivyx loop status` so an
    /// operator can watch spend approach the cap. `0` until the
    /// first iteration of a run; reset on each `request_start`.
    pub tokens_used: u64,
    /// Chapter K — the run-window priced spend in **cents** (USD×100;
    /// `f64` is avoided so this state stays `Eq` + serde-clean). The same
    /// window the dollar cap uses; surfaced for `aivyx loop status`. `0`
    /// until the first iteration; reset on each `request_start`.
    #[serde(default)]
    pub spent_cents: u64,
    /// Chapter Circuit (CI.5) — how many *consecutive* iterations have
    /// made no progress (the live count behind the CI.1 stall breaker).
    /// Surfaced by `aivyx loop status` so a stalling run is legible
    /// before it trips. `0` when progressing; reset on each
    /// `request_start`.
    #[serde(default)]
    pub consecutive_idle: u32,
}
