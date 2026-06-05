//! Phase 173 — the autonomous-loop driver (the Aivyx Ralph loop).
//!
//! A background task, sibling of
//! [`crate::reflection_scheduler::run_reflection_scheduler`], that
//! drives the autonomous loop: while a run is active it fires a
//! **fresh-context** `TriggerSource::Loop` agent turn per
//! iteration, re-arming until the backlog is empty or a hard cap
//! is hit. Each turn is one full pass of the Ralph cycle —
//! `loop.next` → implement → run gates (`shell`) → commit
//! (`git`) → `loop.complete` — carried by the canonical
//! [`LOOP_SYSTEM_PROMPT`].
//!
//! ## Control model
//!
//! The driver owns a shared [`LoopRunState`] the daemon IPC
//! handlers (Phase 173 Task 5) flip:
//!
//! - `aivyx loop start` → [`request_start`] sets `active` + the
//!   per-run `max_iterations` and wakes the driver via a
//!   `Notify`.
//! - `aivyx loop stop` → [`request_stop`] clears `active`; the
//!   driver checks between iterations and ends the run.
//! - `aivyx loop status` → [`snapshot`] reads the state.
//!
//! ## Termination (fully autonomous, capped)
//!
//! Per the Phase 173 entry decision, a run is fully autonomous —
//! no per-iteration operator gate — and ends only on one of the
//! [`LoopDecision`] stop conditions: the backlog drains, the
//! per-run `max_iterations` cap is reached, or an operator stop
//! is requested. The cap + capability gating + the audit chain
//! (every iteration is a `TriggerSource::Loop` entry) are the
//! guardrails. Token-budget + wall-clock caps and driver-side
//! gate verification are Phase 174.
//!
//! [`request_start`]: SharedLoopState::request_start
//! [`request_stop`]: SharedLoopState::request_stop
//! [`snapshot`]: SharedLoopState::snapshot

use std::sync::{Arc, RwLock};
use std::time::Duration;

use aivyx_core::CancellationToken;
use tokio::sync::Notify;

use crate::loop_backlog::PersistentLoopBacklog;
use crate::trigger::{TriggerDispatch, TriggerSource};

/// The canonical system-prompt-shaped instruction every loop
/// iteration carries. Deliberately Ralph-faithful: one story,
/// gates before completion, commit, then mark done — and stop
/// cleanly when the backlog is empty.
pub const LOOP_SYSTEM_PROMPT: &str = "\
You are one iteration of an autonomous task loop. A fresh context \
runs this same instruction each iteration; durable state lives in \
git, the backlog, and your memory — not in this conversation.

Do exactly this, then stop:
1. Call `loop.next` to get the highest-priority pending story. If \
   it reports the backlog is empty, stop immediately and report \
   that the backlog is complete — do not invent work.
2. Implement ONLY that one story. Keep the change small and \
   focused; do not start the next story.
3. Run the project's quality gates with `shell` (build + tests / \
   typecheck). If they do not pass, fix the issue or stop — do \
   NOT mark the story done on red.
4. Once the gates pass, commit the change with `git` (a focused \
   commit message naming the story).
5. Only after a green commit, call `loop.complete` with the \
   story's id.
6. Optionally write one short learning line to `memory` about \
   anything future iterations should know (a gotcha, a \
   convention, a path).

Be conservative: it is always correct to stop without completing \
a story if you are unsure or the gates are red. The loop will \
re-run and the next fresh context can try again.";

/// One run's live state. Shared between the driver and the daemon
/// IPC handlers (start / stop / status).
#[derive(
    Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize,
)]
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
}

/// What the driver should do at the top of an iteration. Pure
/// over the run state + the backlog's remaining count so the
/// termination logic is unit-testable without an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopDecision {
    /// Fire one more iteration.
    Continue,
    /// Stop — the backlog drained.
    StopBacklogEmpty,
    /// Stop — the per-run `max_iterations` cap was reached.
    StopMaxIterations,
    /// Stop — an operator `loop stop` cleared `active`.
    StopRequested,
    /// Phase 174 — stop, the wall-clock `max_run_secs` cap was
    /// reached.
    StopWallClock,
}

impl LoopDecision {
    /// The operator-readable reason recorded in
    /// `LoopRunState::last_stop_reason`. `Continue` has none.
    pub fn stop_reason(&self) -> Option<&'static str> {
        match self {
            LoopDecision::Continue => None,
            LoopDecision::StopBacklogEmpty => Some("backlog complete"),
            LoopDecision::StopMaxIterations => {
                Some("reached max_iterations cap")
            }
            LoopDecision::StopRequested => Some("operator stop"),
            LoopDecision::StopWallClock => {
                Some("reached max_run_secs wall-clock cap")
            }
        }
    }
}

/// Pure termination decision. Checked at the top of every
/// iteration. Order matters: an operator stop wins, then the
/// wall-clock cap, then the iteration cap, then backlog drain
/// (so a stop mid-run is honoured even if the backlog still has
/// work).
///
/// Phase 174 — `elapsed_secs` + `max_run_secs` add the
/// wall-clock cap; `max_run_secs = None` disables it (the cap
/// never fires).
pub fn decide(
    active: bool,
    iteration: u32,
    max_iterations: u32,
    remaining_stories: usize,
    elapsed_secs: u64,
    max_run_secs: Option<u64>,
) -> LoopDecision {
    if !active {
        return LoopDecision::StopRequested;
    }
    if let Some(cap) = max_run_secs {
        if elapsed_secs >= cap {
            return LoopDecision::StopWallClock;
        }
    }
    if iteration >= max_iterations {
        return LoopDecision::StopMaxIterations;
    }
    if remaining_stories == 0 {
        return LoopDecision::StopBacklogEmpty;
    }
    LoopDecision::Continue
}

/// Phase 174 — format the stop reason for a red gate after
/// iteration N (pre-flight uses iteration `0`). Pure so the
/// label is testable without running the driver.
pub fn gate_stop_reason(
    outcome: &crate::loop_gate::GateOutcome,
    iteration: u32,
) -> String {
    if iteration == 0 {
        format!("pre-flight {}", outcome.label())
    } else {
        format!("{} after iteration {iteration}", outcome.label())
    }
}

/// Shared run-state handle plus the start-notify. Cloned into the
/// driver task and every IPC handler.
#[derive(Clone)]
pub struct SharedLoopState {
    state: Arc<RwLock<LoopRunState>>,
    /// Wakes the idle driver when a run is requested.
    notify: Arc<Notify>,
}

impl Default for SharedLoopState {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedLoopState {
    pub fn new() -> Self {
        SharedLoopState {
            state: Arc::new(RwLock::new(LoopRunState::default())),
            notify: Arc::new(Notify::new()),
        }
    }

    /// Read-only snapshot for `aivyx loop status`.
    pub fn snapshot(&self) -> LoopRunState {
        self.state.read().expect("loop state lock").clone()
    }

    /// Request a run start with the given cap. No-op (returns
    /// `false`) if a run is already active. Wakes the driver.
    pub fn request_start(
        &self,
        max_iterations: u32,
        now_unix_ms: u64,
    ) -> bool {
        {
            let mut s = self.state.write().expect("loop state lock");
            if s.active {
                return false;
            }
            s.active = true;
            s.iteration = 0;
            s.max_iterations = max_iterations;
            s.started_at_unix_ms = now_unix_ms;
            s.last_stop_reason = None;
        }
        self.notify.notify_one();
        true
    }

    /// Request the active run to stop. Returns `false` if no run
    /// is active. The driver ends the run between iterations.
    pub fn request_stop(&self) -> bool {
        let mut s = self.state.write().expect("loop state lock");
        if !s.active {
            return false;
        }
        s.active = false;
        true
    }

    fn is_active(&self) -> bool {
        self.state.read().expect("loop state lock").active
    }

    fn iteration(&self) -> u32 {
        self.state.read().expect("loop state lock").iteration
    }

    fn max_iterations(&self) -> u32 {
        self.state.read().expect("loop state lock").max_iterations
    }

    fn started_at_unix_ms(&self) -> u64 {
        self.state.read().expect("loop state lock").started_at_unix_ms
    }

    fn record_iteration(&self) {
        let mut s = self.state.write().expect("loop state lock");
        s.iteration = s.iteration.saturating_add(1);
    }

    fn finish_run(&self, reason: &str) {
        let mut s = self.state.write().expect("loop state lock");
        s.active = false;
        s.last_stop_reason = Some(reason.to_string());
    }
}

/// How long the idle driver waits for a start signal before
/// re-checking the shutdown token. Bounded so daemon shutdown is
/// responsive even when no run is active.
const IDLE_POLL: Duration = Duration::from_secs(30);

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Run the autonomous-loop driver. Never returns normally — runs
/// until `shutdown` is cancelled. Idle (no CPU) until a run is
/// requested via [`SharedLoopState::request_start`]; then fires
/// `TriggerSource::Loop` iterations until [`decide`] says stop.
pub async fn run_loop_driver(
    dispatch: TriggerDispatch,
    backlog: Arc<PersistentLoopBacklog>,
    shared: SharedLoopState,
    gate: Option<Arc<dyn crate::loop_gate::GateRunner>>,
    max_run_secs: Option<u64>,
    shutdown: CancellationToken,
) {
    loop {
        if shutdown.is_cancelled() {
            return;
        }
        // Idle until a run is requested (or shutdown / poll).
        if !shared.is_active() {
            tokio::select! {
                _ = shared.notify.notified() => {}
                _ = tokio::time::sleep(IDLE_POLL) => { continue; }
                _ = shutdown.cancelled() => return,
            }
        }

        // A run is active — drive iterations.
        eprintln!(
            "aivyx loop: run started (max_iterations={}, gate={}, \
             max_run_secs={:?})",
            shared.max_iterations(),
            if gate.is_some() { "on" } else { "off" },
            max_run_secs,
        );

        // Phase 174 — pre-flight gate: refuse to start on a red
        // tree. (iteration 0 → "pre-flight" in the reason.)
        if let Some(g) = &gate {
            let outcome = g.run().await;
            if !outcome.is_green() {
                let reason = gate_stop_reason(&outcome, 0);
                shared.finish_run(&reason);
                eprintln!("aivyx loop: run not started — {reason}");
                continue;
            }
        }

        loop {
            if shutdown.is_cancelled() {
                // Leave the run flagged active so it can resume on
                // restart-via-start; just stop driving.
                return;
            }
            let remaining = backlog.remaining_count();
            let elapsed_secs = now_unix_ms()
                .saturating_sub(shared.started_at_unix_ms())
                / 1000;
            let decision = decide(
                shared.is_active(),
                shared.iteration(),
                shared.max_iterations(),
                remaining,
                elapsed_secs,
                max_run_secs,
            );
            if let Some(reason) = decision.stop_reason() {
                shared.finish_run(reason);
                eprintln!(
                    "aivyx loop: run ended — {reason} (after {} \
                     iteration(s))",
                    shared.iteration(),
                );
                break;
            }

            let iter = shared.iteration() + 1;
            let trigger_id = format!("loop-iter-{iter}");
            eprintln!(
                "aivyx loop: iteration {iter} firing ({remaining} \
                 stor{} remaining)",
                if remaining == 1 { "y" } else { "ies" },
            );
            // Fire a fresh-context loop turn. `wrap_mission =
            // false`: the loop's own backlog is the work tracker,
            // not a per-iteration mission. No notify target.
            let _ = dispatch
                .fire(
                    TriggerSource::Loop,
                    &trigger_id,
                    LOOP_SYSTEM_PROMPT,
                    false,
                    &[],
                    aivyx_config::NotifyWhen::Always,
                )
                .await;
            shared.record_iteration();

            // Phase 174 — post-iteration gate: verify the tree is
            // still green. A red gate stops the run immediately
            // (non-destructive — the commit is preserved for the
            // operator; the loop does not pile more changes onto
            // a broken tree).
            if let Some(g) = &gate {
                let outcome = g.run().await;
                if !outcome.is_green() {
                    let reason = gate_stop_reason(&outcome, iter);
                    shared.finish_run(&reason);
                    eprintln!("aivyx loop: run ended — {reason}");
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decide_continue_when_active_under_cap_with_work() {
        assert_eq!(
            decide(true, 0, 5, 3, 0, None),
            LoopDecision::Continue
        );
        assert_eq!(
            decide(true, 4, 5, 1, 10, Some(3600)),
            LoopDecision::Continue
        );
    }

    #[test]
    fn decide_stops_at_cap() {
        assert_eq!(
            decide(true, 5, 5, 3, 0, None),
            LoopDecision::StopMaxIterations
        );
        assert_eq!(
            decide(true, 6, 5, 3, 0, None),
            LoopDecision::StopMaxIterations
        );
    }

    #[test]
    fn decide_stops_on_empty_backlog() {
        assert_eq!(
            decide(true, 1, 5, 0, 0, None),
            LoopDecision::StopBacklogEmpty
        );
    }

    #[test]
    fn decide_stop_request_wins_over_remaining_work() {
        // Inactive (operator stopped) beats everything else.
        assert_eq!(
            decide(false, 1, 5, 3, 0, None),
            LoopDecision::StopRequested
        );
    }

    #[test]
    fn decide_cap_wins_over_backlog_empty() {
        // At the cap with an empty backlog, the cap reason is
        // reported (checked first) — both are valid stops.
        assert_eq!(
            decide(true, 5, 5, 0, 0, None),
            LoopDecision::StopMaxIterations
        );
    }

    #[test]
    fn decide_wall_clock_cap() {
        // None → never fires, even at huge elapsed.
        assert_eq!(
            decide(true, 1, 5, 3, 1_000_000, None),
            LoopDecision::Continue
        );
        // Under the cap → continue.
        assert_eq!(
            decide(true, 1, 5, 3, 59, Some(60)),
            LoopDecision::Continue
        );
        // At/over the cap → stop.
        assert_eq!(
            decide(true, 1, 5, 3, 60, Some(60)),
            LoopDecision::StopWallClock
        );
        assert_eq!(
            decide(true, 1, 5, 3, 61, Some(60)),
            LoopDecision::StopWallClock
        );
    }

    #[test]
    fn decide_wall_clock_beats_iteration_cap_and_backlog() {
        // Wall-clock is checked before the iteration cap + drain.
        assert_eq!(
            decide(true, 99, 5, 0, 100, Some(60)),
            LoopDecision::StopWallClock
        );
    }

    #[test]
    fn decide_stop_request_beats_wall_clock() {
        assert_eq!(
            decide(false, 1, 5, 3, 100, Some(60)),
            LoopDecision::StopRequested
        );
    }

    #[test]
    fn gate_stop_reason_preflight_vs_iteration() {
        use crate::loop_gate::GateOutcome;
        let failed = GateOutcome::Failed { code: Some(1) };
        assert!(gate_stop_reason(&failed, 0).starts_with("pre-flight"));
        let r = gate_stop_reason(&failed, 3);
        assert!(r.contains("after iteration 3"));
        assert!(r.contains("exit 1"));
    }

    #[test]
    fn stop_reasons_are_labeled() {
        assert!(LoopDecision::Continue.stop_reason().is_none());
        assert_eq!(
            LoopDecision::StopBacklogEmpty.stop_reason(),
            Some("backlog complete")
        );
        assert_eq!(
            LoopDecision::StopMaxIterations.stop_reason(),
            Some("reached max_iterations cap")
        );
        assert_eq!(
            LoopDecision::StopRequested.stop_reason(),
            Some("operator stop")
        );
        assert_eq!(
            LoopDecision::StopWallClock.stop_reason(),
            Some("reached max_run_secs wall-clock cap")
        );
    }

    #[test]
    fn request_start_sets_state_and_is_idempotent() {
        let s = SharedLoopState::new();
        assert!(!s.snapshot().active);
        assert!(s.request_start(10, 1_000));
        let snap = s.snapshot();
        assert!(snap.active);
        assert_eq!(snap.max_iterations, 10);
        assert_eq!(snap.iteration, 0);
        assert_eq!(snap.started_at_unix_ms, 1_000);
        // Second start while active is a no-op.
        assert!(!s.request_start(99, 2_000));
        assert_eq!(s.snapshot().max_iterations, 10);
    }

    #[test]
    fn request_stop_only_when_active() {
        let s = SharedLoopState::new();
        assert!(!s.request_stop()); // not active
        s.request_start(5, 0);
        assert!(s.request_stop());
        assert!(!s.snapshot().active);
    }

    #[test]
    fn record_iteration_and_finish_run() {
        let s = SharedLoopState::new();
        s.request_start(5, 0);
        s.record_iteration();
        s.record_iteration();
        assert_eq!(s.snapshot().iteration, 2);
        s.finish_run("backlog complete");
        let snap = s.snapshot();
        assert!(!snap.active);
        assert_eq!(
            snap.last_stop_reason.as_deref(),
            Some("backlog complete")
        );
    }

    #[test]
    fn restart_after_finish_resets_counters() {
        let s = SharedLoopState::new();
        s.request_start(3, 0);
        s.record_iteration();
        s.finish_run("operator stop");
        // A fresh run zeroes iteration + clears the stop reason.
        assert!(s.request_start(7, 5_000));
        let snap = s.snapshot();
        assert_eq!(snap.iteration, 0);
        assert_eq!(snap.max_iterations, 7);
        assert!(snap.last_stop_reason.is_none());
    }

    #[test]
    fn prompt_mentions_the_core_steps() {
        // Guard the canonical prompt's load-bearing instructions.
        assert!(LOOP_SYSTEM_PROMPT.contains("loop.next"));
        assert!(LOOP_SYSTEM_PROMPT.contains("loop.complete"));
        assert!(LOOP_SYSTEM_PROMPT.contains("gates"));
        assert!(LOOP_SYSTEM_PROMPT.contains("commit"));
        assert!(LOOP_SYSTEM_PROMPT.contains("empty"));
    }
}
