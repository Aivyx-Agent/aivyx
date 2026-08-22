//! Team-mission wire types (Chapter L, moved to `aivyx-ipc` in M.2a).
//!
//! [`TeamMissionRecord`] is the durable + on-the-wire shape of a daemon-run
//! Nonagon mission: its plan, the checkpoint (completed-step outputs), the
//! lifecycle phase, and (when paused) the pending human gate. [`to_view`] is
//! the **driver seam** — it projects a record onto the TUI/GUI-agnostic
//! [`TeamMissionView`], deriving each step's state from the checkpoint, so a
//! client maps it to its own view-model without touching the engine types.
//!
//! The encrypted CRUD over `KeyDomain::TeamMissions` stays daemon-side in
//! `aivyx-channel`; only the data lives here.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use aivyx_team_types::{MissionPlan, StepKind, TeamConfig};

/// The lifecycle phase of a daemon-run team mission — the live state the
/// engine's terminal-only `MissionStatus` doesn't model. Maps onto the TUI's
/// `MissionPhase` (the daemon → IPC → TUI feed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamMissionPhase {
    /// Assembled but not yet executing (transient, pre-first-step).
    Planning,
    /// Walking the DAG.
    Executing,
    /// Paused at a human-approval gate, awaiting an operator decision.
    AwaitingApproval,
    /// Finished — every step ran and any gates passed.
    Done,
    /// Ended by a gate: an auto gate's FAIL verdict, or a human reject.
    Rejected,
    /// Chapter Ballast (Opp D) — halted at a wave boundary because a
    /// per-mission budget cap tripped. Terminal; completed-step outputs are
    /// preserved. Distinct from `Rejected` so the operator sees *why* it ended.
    Halted,
}

impl TeamMissionPhase {
    /// Whether the mission has reached a terminal phase (no further driving).
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TeamMissionPhase::Done
                | TeamMissionPhase::Rejected
                | TeamMissionPhase::Halted
        )
    }
}

/// One persisted team mission: its plan, the checkpoint (completed-step
/// outputs the runtime resumes from), the lifecycle phase, and — when
/// `AwaitingApproval` — the gate step pending an operator decision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamMissionRecord {
    /// Stable mission id (the storage key).
    pub id: String,
    /// The operator's goal / mission description.
    pub goal: String,
    /// The mission DAG.
    pub plan: MissionPlan,
    /// The checkpoint: completed step id → output. The runtime resumes from
    /// this (`run_until_pause(plan, outputs, ..)`).
    #[serde(default)]
    pub outputs: BTreeMap<String, String>,
    /// Current lifecycle phase.
    pub phase: TeamMissionPhase,
    /// The gate step id awaiting approval, set iff `phase == AwaitingApproval`.
    #[serde(default)]
    pub pending_gate: Option<String>,
    /// Why the mission `Halted` — e.g. a per-mission budget cap detail (Chapter
    /// Ballast) or "aborted by operator" (Chapter Belay). Set iff `phase ==
    /// Halted`; lets the operator see *which* cause ended it rather than guessing.
    /// `#[serde(default)]` keeps pre-existing records decoding.
    #[serde(default)]
    pub halt_reason: Option<String>,
    /// Chapter L — the team this mission runs (a vertical pack's `TeamConfig`).
    /// `None` ⇒ the daemon's default team (the Nonagon). Persisted so a resume
    /// after a restart re-assembles the *same* team the plan was built for.
    /// `#[serde(default)]` keeps pre-config records decoding.
    #[serde(default)]
    pub config: Option<TeamConfig>,
    /// Chapter Reprise — how many times the Keystone artifact verdict has
    /// rejected this mission's deliverable. PERSISTED (not a driver-local)
    /// because an approval gate pauses the mission by returning from
    /// `drive_registered`; a local counter reset on every gate resolution,
    /// so a gated mission retried forever (live rig 2026-07-05: five
    /// "attempt 2/2" retries, four gate prompts at the operator).
    /// `#[serde(default)]` keeps pre-existing records decoding.
    #[serde(default)]
    pub verify_attempts: u32,
    pub started_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
}

impl TeamMissionRecord {
    /// A freshly-created mission in the `Planning` phase, empty checkpoint, on
    /// the daemon's default team. Use [`with_config`](Self::with_config) to pin
    /// a vertical pack. (Stamps wall-clock time — daemon-side; a wasm client
    /// only ever *deserializes* records, never calls this.)
    pub fn new(id: impl Into<String>, goal: impl Into<String>, plan: MissionPlan) -> Self {
        let now = now_millis();
        TeamMissionRecord {
            id: id.into(),
            goal: goal.into(),
            plan,
            outputs: BTreeMap::new(),
            phase: TeamMissionPhase::Planning,
            pending_gate: None,
            halt_reason: None,
            config: None,
            verify_attempts: 0,
            started_at_unix_ms: now,
            updated_at_unix_ms: now,
        }
    }

    /// Pin this mission to a specific team config (a vertical pack). `None`
    /// leaves it on the daemon default.
    pub fn with_config(mut self, config: Option<TeamConfig>) -> Self {
        self.config = config;
        self
    }

    /// Stamp `updated_at` to now — call after mutating a field before saving.
    pub fn touch(&mut self) {
        self.updated_at_unix_ms = now_millis();
    }

    /// Project this record onto the client-agnostic [`TeamMissionView`]
    /// (Chapter L.6), with no running-step overlay. Delegates to
    /// [`to_view_with_running`](Self::to_view_with_running) — see it for the
    /// full derivation. Every existing caller uses this; it never shows
    /// [`TeamStepState::Running`], since the checkpoint alone (a
    /// `TeamMissionRecord`'s own fields) has no notion of "executing right
    /// now" — that lives only in the daemon's in-memory driver state
    /// (`SharedMissionState::running_steps` in `aivyx-channel`), which this
    /// method has no access to.
    pub fn to_view(&self) -> TeamMissionView {
        self.to_view_with_running(None)
    }

    /// Chapter Mission Control — like [`to_view`](Self::to_view), but
    /// overlays [`TeamStepState::Running`] onto `running_step`'s step id, if
    /// given. `running_step` should come from the daemon's own live
    /// in-memory tracking (never from anything persisted) — a wasm client
    /// projecting a bare `TeamMissionRecord` it already has has no way to
    /// supply a meaningful value here and should keep calling plain
    /// `to_view()`; only the daemon, building a `TeamMissionView` to
    /// broadcast, has the live signal in scope.
    pub fn to_view_with_running(&self, running_step: Option<&str>) -> TeamMissionView {
        let steps: Vec<TeamStepView> = self
            .plan
            .steps
            .iter()
            .map(|step| {
                let (kind, member) = match &step.kind {
                    StepKind::Delegate { specialist, .. } => ("delegate", specialist.as_str()),
                    StepKind::Gate { reviewer, .. } => ("gate", reviewer.as_str()),
                };
                TeamStepView {
                    label: format!("{} — {member} ({kind})", step.id),
                    state: self.step_state(&step.id, running_step),
                }
            })
            .collect();
        let total = steps.len().max(1);
        let done = steps
            .iter()
            .filter(|s| matches!(s.state, TeamStepState::Done | TeamStepState::Rejected))
            .count();
        TeamMissionView {
            id: self.id.clone(),
            goal: self.goal.clone(),
            lead: self
                .config
                .as_ref()
                .map(|c| c.lead.clone())
                .unwrap_or_else(|| "coordinator".to_string()),
            phase: self.phase,
            pending_gate: self.pending_gate.clone(),
            halt_reason: self.halt_reason.clone(),
            progress: ((done * 100) / total) as u16,
            steps,
        }
    }

    /// The operator-facing state of one step, derived from the checkpoint
    /// plus an optional live running-step overlay. Precedence, highest
    /// first: a pending human gate is always `Awaiting` (even if
    /// `running_step` stale-matches it — a step paused for operator input is
    /// never "running"); then `running_step`'s own match is `Running`; then
    /// the checkpoint: a rejected output is `Rejected`, any other output is
    /// `Done`, no output is `Pending`.
    fn step_state(&self, step_id: &str, running_step: Option<&str>) -> TeamStepState {
        if self.pending_gate.as_deref() == Some(step_id) {
            return TeamStepState::Awaiting;
        }
        if running_step == Some(step_id) {
            return TeamStepState::Running;
        }
        match self.outputs.get(step_id) {
            Some(v) if v.starts_with("rejected") => TeamStepState::Rejected,
            Some(_) => TeamStepState::Done,
            None => TeamStepState::Pending,
        }
    }
}

/// A client-agnostic projection of a [`TeamMissionRecord`] for the Missions
/// feed (Chapter L.6). Carries only primitives + this module's own enums so a
/// client maps it to its view-model without depending on the engine types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamMissionView {
    pub id: String,
    pub goal: String,
    /// The team lead's name (the pack's lead, or `coordinator` for the
    /// default Nonagon) — shown in the feed.
    pub lead: String,
    pub phase: TeamMissionPhase,
    /// The step id awaiting an operator decision, when `phase ==
    /// AwaitingApproval` — what `aivyx team approve|reject <id> <step>` /
    /// the approve/reject affordances target.
    pub pending_gate: Option<String>,
    /// Why the mission `Halted` (budget cap detail, or "aborted by operator"),
    /// when `phase == Halted`. Lets a client show *which* cause ended it.
    #[serde(default)]
    pub halt_reason: Option<String>,
    /// Completion percent in `0..=100` (completed steps / total).
    pub progress: u16,
    pub steps: Vec<TeamStepView>,
}

/// One step in a [`TeamMissionView`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamStepView {
    /// e.g. `"approve — reviewer (gate)"`.
    pub label: String,
    pub state: TeamStepState,
}

/// The checkpoint-derived state of a step for the feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TeamStepState {
    /// Not yet run.
    Pending,
    /// Chapter Mission Control — the driver has started this step and it
    /// hasn't finished yet. Set/cleared in-memory only, never persisted
    /// (see `SharedMissionState`'s `running_steps` field in
    /// `aivyx-channel`); a plain `to_view()` (no live signal available)
    /// never produces this variant.
    Running,
    /// Completed (its output is in the checkpoint).
    Done,
    /// The human gate awaiting an operator decision.
    Awaiting,
    /// A gate the operator rejected.
    Rejected,
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_team_types::Step;

    fn sample(id: &str) -> TeamMissionRecord {
        let plan = MissionPlan::new(
            "overnight close",
            vec![
                Step::delegate("count", "inventory", "count stock"),
                Step::human_gate("approve", "manager", "ok to order?").after(["count"]),
                Step::delegate("order", "purchasing", "place PO").after(["approve"]),
            ],
        );
        let mut r = TeamMissionRecord::new(id, "overnight close", plan);
        r.outputs.insert("count".into(), "12 low items".into());
        r.phase = TeamMissionPhase::AwaitingApproval;
        r.pending_gate = Some("approve".into());
        r
    }

    #[test]
    fn to_view_derives_step_states_and_progress() {
        // count → [human gate approve] → order, paused at the gate.
        let rec = sample("v1");
        let view = rec.to_view();
        assert_eq!(view.id, "v1");
        assert_eq!(view.phase, TeamMissionPhase::AwaitingApproval);
        assert_eq!(view.pending_gate.as_deref(), Some("approve"));
        // count is done, approve is awaiting, order is pending → 1/3 = 33%.
        assert_eq!(view.steps.len(), 3);
        assert_eq!(view.steps[0].state, TeamStepState::Done);
        assert_eq!(view.steps[1].state, TeamStepState::Awaiting);
        assert_eq!(view.steps[2].state, TeamStepState::Pending);
        assert!(view.steps[1].label.contains("manager (gate)"));
        assert_eq!(view.progress, 33);
    }

    #[test]
    fn to_view_marks_a_rejected_gate() {
        let mut rec = sample("v2");
        rec.phase = TeamMissionPhase::Rejected;
        rec.pending_gate = None;
        rec.outputs.insert("approve".into(), "rejected by operator".into());
        let view = rec.to_view();
        assert_eq!(view.steps[1].state, TeamStepState::Rejected);
        assert_eq!(view.steps[2].state, TeamStepState::Pending, "dependent never ran");
    }

    #[test]
    fn record_round_trips_through_serde_json() {
        // The wire-truth check: a record serializes + deserializes intact.
        let rec = sample("m1");
        let json = serde_json::to_string(&rec).unwrap();
        let back: TeamMissionRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back, rec);
        assert!(back.plan.step("approve").unwrap().is_human_gate());
    }

    #[test]
    fn to_view_with_running_marks_the_given_step_running() {
        let rec = sample("v1");
        let view = rec.to_view_with_running(Some("order"));
        assert_eq!(view.steps[0].state, TeamStepState::Done, "count unaffected");
        assert_eq!(view.steps[1].state, TeamStepState::Awaiting, "approve unaffected");
        assert_eq!(view.steps[2].state, TeamStepState::Running, "order is now running");
    }

    #[test]
    fn to_view_with_running_never_overrides_a_pending_gate() {
        // "approve" is the mission's pending_gate. A stale running-step
        // marker for it (e.g. left over from on_step_started firing before
        // the human-gate pause) must NOT surface as Running -- Awaiting
        // always wins, matching step_state()'s existing precedence.
        let rec = sample("v1");
        let view = rec.to_view_with_running(Some("approve"));
        assert_eq!(view.steps[1].state, TeamStepState::Awaiting);
    }

    #[test]
    fn to_view_with_running_of_none_matches_plain_to_view() {
        let rec = sample("v1");
        assert_eq!(rec.to_view_with_running(None), rec.to_view());
    }

    #[test]
    fn to_view_still_shows_no_running_step_at_all() {
        // Unchanged behavior for every existing caller: to_view() never
        // shows Running, since it never knows about the live signal.
        let rec = sample("v1");
        let view = rec.to_view();
        assert!(view.steps.iter().all(|s| s.state != TeamStepState::Running));
    }
}
