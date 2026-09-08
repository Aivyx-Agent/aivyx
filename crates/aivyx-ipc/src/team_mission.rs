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
    /// Chapter Mission Control — paused at a wave boundary by an explicit
    /// operator pause request (distinct from `AwaitingApproval`, which is
    /// a *plan-defined* human gate; this is an operator interrupting an
    /// otherwise-unattended run). **Non-terminal** — `is_terminal()`
    /// deliberately omits it. Completed-step outputs are preserved exactly
    /// like `Halted`'s are; resume continues the DAG walk from them. Never
    /// carries a `halt_reason` (that field's own contract is "set iff
    /// phase == Halted").
    Paused,
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
// Note: no `Eq` here (unlike this file's other wire structs) — `spend_usd`
// is an `f64`, which has no total-equality relation (NaN != NaN). Nothing
// in this codebase actually needs `TeamMissionRecord: Eq` (checked: no
// `HashSet`/`HashMap` key usage), only `PartialEq` for test assertions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Chapter Mission Control — cumulative metered spend (tokens) across
    /// this mission's whole lifetime, re-seeded into a fresh
    /// `MeteringAuditHook` on every `drive_registered` call (start, resume,
    /// gate-approval continuation, or Chapter Reprise retry) so a
    /// `[budget]` cap tracks real cumulative spend rather than resetting
    /// every time the mission is re-driven. Only meaningfully populated
    /// when the daemon's mission budget is bounded (unbounded missions
    /// never construct a meter at all — see `assemble_runtime`).
    /// `#[serde(default)]` keeps pre-existing records decoding.
    #[serde(default)]
    pub spend_tokens: u64,
    /// Chapter Mission Control — cumulative metered spend (USD), same
    /// rationale as `spend_tokens`.
    #[serde(default)]
    pub spend_usd: f64,
    /// Chapter Muster — the id of the `[[schedule]]` entry that started
    /// this mission, if any. `None` for every mission started any other
    /// way (manual `aivyx-pa team run`, the Studio, the autonomous loop's
    /// auto-delegation, ...). `#[serde(default)]` keeps pre-existing
    /// records decoding.
    #[serde(default)]
    pub triggered_by: Option<String>,
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
            spend_tokens: 0,
            spend_usd: 0.0,
            triggered_by: None,
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

    /// Chapter Muster — tag this mission with the schedule that started
    /// it. Mirrors `with_config`'s exact shape.
    pub fn with_triggered_by(mut self, source: impl Into<String>) -> Self {
        self.triggered_by = Some(source.into());
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
        self.to_view_with_running(&std::collections::HashSet::new())
    }

    /// Chapter Mission Control — like [`to_view`](Self::to_view), but
    /// overlays [`TeamStepState::Running`] onto every step id in
    /// `running_steps`, if any. `running_steps` should come from the
    /// daemon's own live in-memory tracking (never from anything persisted)
    /// — a wasm client projecting a bare `TeamMissionRecord` it already has
    /// has no way to supply a meaningful value here and should keep calling
    /// plain `to_view()`; only the daemon, building a `TeamMissionView` to
    /// broadcast, has the live signal in scope. Nonagon missions run every
    /// step in a DAG wave concurrently (`TeamRuntime::run_until_pause`'s
    /// `join_all`), so more than one step id can legitimately be in the set
    /// at once.
    pub fn to_view_with_running(
        &self,
        running_steps: &std::collections::HashSet<String>,
    ) -> TeamMissionView {
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
                    state: self.step_state(&step.id, running_steps),
                    step_id: step.id.clone(),
                    member: member.to_string(),
                    kind: kind.to_string(),
                    deps: step.deps.clone(),
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
            verify_attempts: self.verify_attempts,
            progress: ((done * 100) / total) as u16,
            steps,
        }
    }

    /// The operator-facing state of one step, derived from the checkpoint
    /// plus a live set of currently-running step ids. Precedence, highest
    /// first: a pending human gate is always `Awaiting` (even if
    /// `running_steps` stale-matches it — a step paused for operator input
    /// is never "running"); then membership in `running_steps` is
    /// `Running`; then the checkpoint: a rejected output is `Rejected`, any
    /// other output is `Done`, no output is `Pending`.
    fn step_state(
        &self,
        step_id: &str,
        running_steps: &std::collections::HashSet<String>,
    ) -> TeamStepState {
        if self.pending_gate.as_deref() == Some(step_id) {
            return TeamStepState::Awaiting;
        }
        if running_steps.contains(step_id) {
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
    /// AwaitingApproval` — what `aivyx-pa team approve|reject <id> <step>` /
    /// the approve/reject affordances target.
    pub pending_gate: Option<String>,
    /// Why the mission `Halted` (budget cap detail, or "aborted by operator"),
    /// when `phase == Halted`. Lets a client show *which* cause ended it.
    #[serde(default)]
    pub halt_reason: Option<String>,
    /// POLISH_WAVES.md sub-project 5, item B — the mission's current
    /// Chapter Reprise verification-retry attempt count: the number of
    /// FAILED verifications so far, so a repeated approval gate (the
    /// same `gate_review_brief` re-shown on each retry) can tell the
    /// operator which attempt this is. `0` means no retry has happened
    /// yet (the operator is on attempt 1); a client displays the
    /// human-facing attempt number as `verify_attempts + 1`.
    /// `MAX_MISSION_ATTEMPTS` (`aivyx-channel`'s `team_mission_driver.rs`)
    /// caps this at `1` in practice — a mission never retries twice.
    #[serde(default)]
    pub verify_attempts: u32,
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
    /// Chapter Mission Control — the step's own id (`MissionPlan`'s
    /// `Step::id`). A client that only had `label` before this had to
    /// parse a formatted display string to recover this — now it's a
    /// real field.
    pub step_id: String,
    /// Chapter Mission Control — the specialist (for a `delegate` step) or
    /// reviewer (for a `gate` step) this step runs on.
    pub member: String,
    /// Chapter Mission Control — `"delegate"` or `"gate"`, matching the
    /// two `StepKind` variants. A plain `String` rather than a new public
    /// enum, since these are the only two values `StepKind` has and the
    /// existing `label` formatting already treated them as a fixed pair.
    /// (Deliberately not `&'static str`, despite the two literal values
    /// this always holds: a `Deserialize`-deriving wire type can't carry a
    /// `&'static str` field — the derive requires `'de: 'static`, which is
    /// unsatisfiable for `TeamMissionView`'s own generic `Deserialize<'de>`
    /// impl since `TeamStepView` sits inside it via `Vec<TeamStepView>`;
    /// confirmed with a minimal repro before choosing `String` here.)
    pub kind: String,
    /// Chapter Mission Control — the ids of steps that must complete
    /// before this one is ready (`Step::deps`, unchanged from the engine
    /// type) — lets a client draw real dependency edges without touching
    /// `MissionPlan`/`StepKind` directly.
    #[serde(default)]
    pub deps: Vec<String>,
}

/// The checkpoint-derived state of a step for the feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TeamStepState {
    /// Not yet run.
    Pending,
    /// Chapter Mission Control — the driver has started this step and it
    /// hasn't finished yet. Set/cleared in-memory only, never persisted
    /// (see `SharedMissionState`'s `running_steps` field in
    /// `aivyx-channel`, which tracks a *set* of concurrently-running step
    /// ids per mission — Nonagon missions run every step in a DAG wave
    /// concurrently); a plain `to_view()` (no live signal available) never
    /// produces this variant.
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
    fn to_view_carries_verify_attempts() {
        let mut rec = sample("v3");
        rec.verify_attempts = 2;
        let view = rec.to_view();
        assert_eq!(view.verify_attempts, 2);
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
    fn record_with_spend_round_trips_through_serde_json() {
        // Chapter Mission Control (Fix A) — a record carrying non-zero
        // metered spend round-trips intact.
        let mut rec = sample("m2");
        rec.spend_tokens = 12_345;
        rec.spend_usd = 0.42;
        let json = serde_json::to_string(&rec).unwrap();
        let back: TeamMissionRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(back, rec);
        assert_eq!(back.spend_tokens, 12_345);
        assert!((back.spend_usd - 0.42).abs() < 1e-9);
    }

    #[test]
    fn a_pre_existing_record_missing_spend_fields_still_decodes() {
        // Chapter Mission Control (Fix A) — simulate a record persisted
        // before this change: no `spend_tokens`/`spend_usd` keys at all.
        // `#[serde(default)]` must default both to zero rather than
        // failing to decode.
        let json = r#"{
            "id": "old1",
            "goal": "legacy goal",
            "plan": {"goal": "legacy goal", "steps": []},
            "phase": "done",
            "started_at_unix_ms": 1,
            "updated_at_unix_ms": 1
        }"#;
        let rec: TeamMissionRecord = serde_json::from_str(json).unwrap();
        assert_eq!(rec.spend_tokens, 0);
        assert_eq!(rec.spend_usd, 0.0);
    }

    #[test]
    fn with_triggered_by_sets_the_field_and_leaves_everything_else_unchanged() {
        let plan = MissionPlan { goal: "g".to_string(), steps: vec![] };
        let record = TeamMissionRecord::new("m1", "goal", plan.clone())
            .with_triggered_by("cfg-nightly-boh-close");
        assert_eq!(record.triggered_by.as_deref(), Some("cfg-nightly-boh-close"));
        assert_eq!(record.id, "m1");
        assert_eq!(record.phase, TeamMissionPhase::Planning);
    }

    #[test]
    fn a_record_with_no_triggered_by_call_has_none() {
        let plan = MissionPlan { goal: "g".to_string(), steps: vec![] };
        let record = TeamMissionRecord::new("m1", "goal", plan);
        assert_eq!(record.triggered_by, None);
    }

    #[test]
    fn a_pre_existing_json_record_deserializes_with_no_triggered_by() {
        // Simulates a record persisted before this field existed -- no
        // "triggered_by" key at all. Use the real current field set (check
        // the struct definition for anything this literal is missing before
        // trusting it -- TeamMissionRecord has grown fields across several
        // chapters, most recently spend_tokens/spend_usd).
        let json = br#"{
            "id": "m1",
            "goal": "goal",
            "plan": {"goal": "goal", "steps": []},
            "outputs": {},
            "phase": "planning",
            "started_at_unix_ms": 1000,
            "updated_at_unix_ms": 1000
        }"#;
        let record: TeamMissionRecord = serde_json::from_slice(json).expect("deserialize old record");
        assert_eq!(record.triggered_by, None);
    }

    #[test]
    fn to_view_exposes_structured_step_fields_not_just_the_label() {
        let rec = sample("v1");
        let view = rec.to_view();
        // sample()'s fixture: count (delegate, inventory) -> approve
        // (human gate, manager) -> order (delegate, purchasing).
        assert_eq!(view.steps[0].step_id, "count");
        assert_eq!(view.steps[0].member, "inventory");
        assert_eq!(view.steps[0].kind, "delegate");
        assert_eq!(view.steps[0].deps, Vec::<String>::new(), "count has no deps");

        assert_eq!(view.steps[1].step_id, "approve");
        assert_eq!(view.steps[1].member, "manager");
        assert_eq!(view.steps[1].kind, "gate");
        assert_eq!(view.steps[1].deps, vec!["count".to_string()]);

        assert_eq!(view.steps[2].step_id, "order");
        assert_eq!(view.steps[2].member, "purchasing");
        assert_eq!(view.steps[2].kind, "delegate");
        assert_eq!(view.steps[2].deps, vec!["approve".to_string()]);

        // label is UNCHANGED -- existing aivyx-tui rendering still works.
        assert!(view.steps[1].label.contains("manager (gate)"));
    }

    #[test]
    fn to_view_with_running_marks_the_given_step_running() {
        let rec = sample("v1");
        let view = rec.to_view_with_running(&["order".to_string()].into_iter().collect());
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
        let view = rec.to_view_with_running(&["approve".to_string()].into_iter().collect());
        assert_eq!(view.steps[1].state, TeamStepState::Awaiting);
    }

    #[test]
    fn to_view_with_running_of_none_matches_plain_to_view() {
        let rec = sample("v1");
        assert_eq!(
            rec.to_view_with_running(&std::collections::HashSet::new()),
            rec.to_view()
        );
    }

    #[test]
    fn to_view_still_shows_no_running_step_at_all() {
        // Unchanged behavior for every existing caller: to_view() never
        // shows Running, since it never knows about the live signal.
        let rec = sample("v1");
        let view = rec.to_view();
        assert!(view.steps.iter().all(|s| s.state != TeamStepState::Running));
    }

    #[test]
    fn paused_is_not_terminal() {
        assert!(!TeamMissionPhase::Paused.is_terminal());
    }

    #[test]
    fn to_view_with_running_marks_multiple_concurrent_steps_running() {
        // Two independent delegate steps with no dependency between them --
        // a real concurrent-wave shape, not a sequential chain.
        let plan = MissionPlan::new(
            "parallel goal",
            vec![
                Step::delegate("a", "specialist-a", "do a"),
                Step::delegate("b", "specialist-b", "do b"),
            ],
        );
        let rec = TeamMissionRecord::new("v2", "parallel goal", plan);
        let running: std::collections::HashSet<String> =
            ["a".to_string(), "b".to_string()].into_iter().collect();
        let view = rec.to_view_with_running(&running);
        assert_eq!(view.steps[0].state, TeamStepState::Running, "a is running");
        assert_eq!(view.steps[1].state, TeamStepState::Running, "b is running too, not clobbered");
    }
}
