//! `TeamRuntime` — execute a [`MissionPlan`] DAG against the
//! [`SpecialistPool`] (J.4.2).
//!
//! This is where the future-proofing decision cashes out: the runtime walks
//! the DAG by repeatedly taking the **ready set** (steps whose deps are all
//! complete) and running **every ready step concurrently** (`join_all`).
//! Independent branches therefore make progress together — widening a linear
//! mission to a parallel one is a *flip* of the plan's shape, not a runtime
//! rewrite.
//!
//! Each step delegates to a specialist sub-turn via [`SpecialistPool::run`]
//! (attenuated + trust-floored, exactly as a single `delegate_task` call).
//! Downstream steps receive their upstream outputs as context. A
//! [`StepKind::Gate`] runs a reviewer over the upstream work; a failing
//! verdict aborts the mission so the gate's dependents never run — returned
//! as a [`MissionStatus::GateRejected`] report (not an error), so the lead
//! keeps the partial outputs and the verdict.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use aivyx_core::ChannelContext;

use crate::config::TeamError;
use crate::mission::{MissionPlan, Step, StepKind};
use crate::pool::SpecialistPool;

/// How a mission run ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissionStatus {
    /// Every step ran and any gates passed.
    Completed,
    /// A gate's reviewer rejected the upstream work; the gate's dependents
    /// were skipped. The partial outputs are still in the report.
    GateRejected { step: String, verdict: String },
    /// Chapter Ballast (Opp D) — the mission was halted at a wave boundary
    /// because an external budget cap tripped (the observer's
    /// [`should_halt`](MissionObserver::should_halt) returned a reason). The
    /// already-completed steps' outputs are preserved in the report; no further
    /// steps run. A graceful, terminal stop — not an error.
    Halted { reason: String },
}

/// The result of a mission run: the goal, every completed step's output
/// (`step_id → result`, ordered), and how it ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MissionReport {
    pub goal: String,
    pub outputs: BTreeMap<String, String>,
    pub status: MissionStatus,
}

impl MissionReport {
    /// Whether the mission completed without a gate rejection.
    pub fn succeeded(&self) -> bool {
        self.status == MissionStatus::Completed
    }
}

/// Observes a mission run as the runtime walks the DAG — the progress feed
/// Chapter L's daemon needs to surface live mission state (the J.7 TUI panel,
/// `aivyx team status`). All methods default to no-ops so an observer overrides
/// only the events it cares about; `()` is the null observer used by the plain
/// [`TeamRuntime::run`] path (no behavior change from the J.4 batch run).
///
/// Callbacks for steps in the same ready set fire **concurrently** (the runtime
/// runs them with `join_all`), so an observer must be `Send + Sync` and tolerate
/// interleaving. Per step the order is `on_step_started` → (`on_gate` for a
/// `Gate`, else `on_step_completed`); `on_mission_finished` fires once at the end.
pub trait MissionObserver: Send + Sync {
    /// A step's specialist sub-turn is about to run (`member` = specialist or
    /// reviewer). Maps to the TUI's `StepState::Running`.
    fn on_step_started(&self, _step_id: &str, _member: &str) {}
    /// A `Delegate` step finished with `output`. Maps to `StepState::Done`.
    fn on_step_completed(&self, _step_id: &str, _output: &str) {}
    /// A `Gate` step's reviewer returned a verdict. `passed` is `gate_passed`.
    /// Maps to `StepState::Gated` (passed) / `StepState::Failed` (rejected).
    fn on_gate(&self, _step_id: &str, _passed: bool, _verdict: &str) {}
    /// The mission ended (Completed, GateRejected, or Halted).
    fn on_mission_finished(&self, _report: &MissionReport) {}

    /// Chapter Ballast (Opp D) — checked at each wave boundary, **before** the
    /// next ready set is launched. Return `Some(reason)` to halt the mission
    /// gracefully (a terminal [`MissionStatus::Halted`] preserving completed
    /// outputs); `None` (the default) lets the run proceed unchanged. The
    /// daemon's mission driver overrides this to enforce a per-mission budget
    /// ([`aivyx_cost::MissionBudget`]); every other observer keeps the default
    /// so behavior is byte-identical.
    fn should_halt(&self) -> Option<String> {
        None
    }
}

/// The null observer — `TeamRuntime::run` walks the DAG with this, preserving
/// the J.4 batch behavior exactly.
impl MissionObserver for () {}

/// The outcome of one [`run_until_pause`](TeamRuntime::run_until_pause) leg
/// (Chapter L). Either the mission reached a terminal state, or it paused at a
/// human-approval gate with a durable checkpoint the daemon persists and later
/// resumes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunYield {
    /// The mission ran to a terminal state — `Completed`, or an **auto** gate
    /// rejected it (`GateRejected`). The report carries the partial/full
    /// outputs either way.
    Done(MissionReport),
    /// A human-approval gate (`GateMode::Human`) became ready and the run
    /// paused. `outputs` is the checkpoint (every step completed so far, **not**
    /// the pending gate); the daemon persists it, and on approval resumes by
    /// inserting the gate's verdict into `outputs` and calling
    /// `run_until_pause` again.
    AwaitingHuman {
        step: String,
        outputs: BTreeMap<String, String>,
    },
}

/// Runs mission DAGs against a team's [`SpecialistPool`].
pub struct TeamRuntime {
    pool: Arc<SpecialistPool>,
}

impl TeamRuntime {
    pub fn new(pool: Arc<SpecialistPool>) -> Self {
        TeamRuntime { pool }
    }

    /// Execute `plan` against the pool, concurrently within each ready set,
    /// over the lead's live channel. Returns a [`MissionReport`]; a
    /// specialist error (or an invalid plan) is an `Err`, while a gate
    /// rejection is an `Ok` report with [`MissionStatus::GateRejected`].
    ///
    /// The observer-less entry point: walks the DAG with the null observer,
    /// preserving the J.4 batch behavior byte-for-byte.
    pub async fn run(
        &self,
        plan: &MissionPlan,
        lead_channel: &dyn ChannelContext,
    ) -> Result<MissionReport, TeamError> {
        self.run_observed(plan, lead_channel, &()).await
    }

    /// Like [`run`](Self::run), but reports progress to `observer` as the DAG
    /// is walked (Chapter L's live feed). Behavior is otherwise identical —
    /// the observer only watches; it never changes the run.
    ///
    /// This is the **non-interactive** entry point: a plan with a human-approval
    /// gate cannot be auto-resolved here, so it returns an `Err` — drive such a
    /// plan through [`run_until_pause`](Self::run_until_pause) (the daemon path).
    pub async fn run_observed(
        &self,
        plan: &MissionPlan,
        lead_channel: &dyn ChannelContext,
        observer: &dyn MissionObserver,
    ) -> Result<MissionReport, TeamError> {
        match self
            .run_until_pause(plan, BTreeMap::new(), lead_channel, observer)
            .await?
        {
            RunYield::Done(report) => Ok(report),
            RunYield::AwaitingHuman { step, .. } => Err(TeamError::Config(format!(
                "mission {:?} has a human-approval gate {step:?}; run it through the \
                 daemon (run_until_pause), not the one-shot run path",
                plan.goal
            ))),
        }
    }

    /// Walk the DAG from a checkpoint until the mission finishes **or** reaches
    /// a human-approval gate, returning a [`RunYield`] (Chapter L). This is the
    /// resumable core that [`run`] / [`run_observed`] wrap.
    ///
    /// `starting_outputs` is the checkpoint — every already-completed step's
    /// output (empty for a fresh run). Steps in it are treated as done and
    /// never re-run (resume idempotence): driving a plan straight through equals
    /// running it pause-by-pause. The walk runs each ready set's non-human
    /// steps concurrently exactly as the batch run did; when the **only**
    /// remaining ready steps are human gates, it pauses at the first
    /// ([`RunYield::AwaitingHuman`]) with the checkpoint. To resume after an
    /// approval, insert the gate id → verdict into the returned `outputs` and
    /// call this again; to reject, the caller builds a `GateRejected` report.
    pub async fn run_until_pause(
        &self,
        plan: &MissionPlan,
        starting_outputs: BTreeMap<String, String>,
        lead_channel: &dyn ChannelContext,
        observer: &dyn MissionObserver,
    ) -> Result<RunYield, TeamError> {
        plan.validate()?;

        let mut outputs = starting_outputs;
        let mut completed: HashSet<String> = outputs.keys().cloned().collect();

        while completed.len() < plan.steps.len() {
            // Chapter Ballast — budget check at the wave boundary, before any
            // more specialist sub-turns are launched. A tripped cap halts the
            // mission gracefully: completed outputs are preserved, no new steps
            // run. Bounded overspend = the in-flight wave that already ran.
            if let Some(reason) = observer.should_halt() {
                let report = MissionReport {
                    goal: plan.goal.clone(),
                    outputs,
                    status: MissionStatus::Halted { reason },
                };
                observer.on_mission_finished(&report);
                return Ok(RunYield::Done(report));
            }

            // The ready set — sorted for deterministic scheduling/reporting.
            let mut ready = plan.ready(&completed);
            ready.sort_by(|a, b| a.id.cmp(&b.id));
            if ready.is_empty() {
                // A validated DAG always frees a step until all are done;
                // empty-while-incomplete would be a runtime bug, not input.
                return Err(TeamError::Config(
                    "mission stalled: no ready steps but the plan is incomplete".into(),
                ));
            }

            // Human-approval gates pause the run; everything else runs now. We
            // pause only once nothing else is runnable, so all work not blocked
            // by the gate makes progress first (the gate's dependents are, by
            // definition, not in this ready set).
            let runnable: Vec<&Step> =
                ready.iter().copied().filter(|s| !s.is_human_gate()).collect();
            if runnable.is_empty() {
                // Every ready step is a human gate — pause at the first.
                return Ok(RunYield::AwaitingHuman {
                    step: ready[0].id.clone(),
                    outputs,
                });
            }

            // Run the runnable set concurrently. Each future captures only owned
            // strings + shared refs (self.pool, lead_channel, observer), so the
            // outputs map is free to mutate once join_all has collected.
            let futures = runnable.iter().map(|step| {
                let id = step.id.clone();
                let member = step.kind.member().to_string();
                let input = self.build_input(step, &outputs);
                async move {
                    observer.on_step_started(&id, &member);
                    let res = self.pool.run(&member, &input, lead_channel).await;
                    (id, res)
                }
            });
            let results = futures_util::future::join_all(futures).await;

            let mut rejection: Option<(String, String)> = None;
            for (id, res) in results {
                let output = res?; // a specialist error aborts the whole mission
                // Only AUTO gates run here (human gates were filtered out above).
                let is_gate = matches!(
                    plan.step(&id).map(|s| &s.kind),
                    Some(StepKind::Gate { .. })
                );
                if is_gate {
                    let passed = gate_passed(&output);
                    observer.on_gate(&id, passed, &output);
                    if !passed && rejection.is_none() {
                        rejection = Some((id.clone(), output.clone()));
                    }
                } else {
                    observer.on_step_completed(&id, &output);
                }
                outputs.insert(id.clone(), output);
                completed.insert(id);
            }

            if let Some((step, verdict)) = rejection {
                let report = MissionReport {
                    goal: plan.goal.clone(),
                    outputs,
                    status: MissionStatus::GateRejected { step, verdict },
                };
                observer.on_mission_finished(&report);
                return Ok(RunYield::Done(report));
            }
        }

        let report = MissionReport {
            goal: plan.goal.clone(),
            outputs,
            status: MissionStatus::Completed,
        };
        observer.on_mission_finished(&report);
        Ok(RunYield::Done(report))
    }

    /// The prompt handed to a step's specialist: its own instruction, plus
    /// every upstream output as context (so a pipeline's later steps see
    /// what their dependencies produced).
    fn build_input(&self, step: &crate::mission::Step, outputs: &BTreeMap<String, String>) -> String {
        let base = match &step.kind {
            StepKind::Delegate { prompt, .. } => prompt.clone(),
            StepKind::Gate { criteria, .. } => format!(
                "Review the upstream work against these criteria. Begin your reply with PASS \
                 or FAIL, then explain.\n\nCriteria: {criteria}"
            ),
        };
        if step.deps.is_empty() {
            return base;
        }
        let mut ctx = String::from("\n\n--- Context from upstream steps ---");
        for dep in &step.deps {
            if let Some(out) = outputs.get(dep) {
                ctx.push_str(&format!("\n[{dep}]: {out}"));
            }
        }
        format!("{base}{ctx}")
    }
}

/// A gate passes unless its verdict begins with `FAIL` (case-insensitive).
/// Shared with `verify_output` (J.4.3) so in-DAG gates and the ad-hoc verify
/// tool judge a verdict the same way.
pub(crate) fn gate_passed(verdict: &str) -> bool {
    !verdict.trim_start().to_ascii_uppercase().starts_with("FAIL")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mission::Step;
    use crate::testutil::{member, team_pool, FakeLeadChannel, FakeProvider};
    use aivyx_capability::TrustTier;
    use aivyx_core::{
        CancellationToken, ChannelContext, ChannelError, ChannelPlatform, SessionId, StreamEvent,
        TurnOutcome,
    };
    use aivyx_llm::{LlmError, LlmProvider, LlmRequest, LlmStepEnd, LlmStream, LlmStreamEvent};
    use async_trait::async_trait;
    use std::time::Duration;
    use tokio::sync::Barrier;

    fn runtime(provider: Arc<dyn LlmProvider>, members: &[&str]) -> TeamRuntime {
        let mut roster = vec![member("lead", &[], TrustTier::Trusted)];
        for m in members {
            roster.push(member(m, &[], TrustTier::Trusted));
        }
        TeamRuntime::new(Arc::new(team_pool(provider, roster, "lead", &[])))
    }

    #[test]
    fn gate_pass_fail_detection() {
        assert!(gate_passed("PASS looks good"));
        assert!(gate_passed("LGTM"));
        assert!(!gate_passed("FAIL: missing tests"));
        assert!(!gate_passed("  fail, try again"), "case + leading space insensitive");
    }

    /// Records every observer callback as an ordered string, so a test can
    /// assert the runtime fired the live feed in the expected sequence.
    #[derive(Default)]
    struct RecordingObserver {
        events: std::sync::Mutex<Vec<String>>,
    }
    impl RecordingObserver {
        fn snapshot(&self) -> Vec<String> {
            self.events.lock().unwrap().clone()
        }
    }
    impl MissionObserver for RecordingObserver {
        fn on_step_started(&self, step_id: &str, member: &str) {
            self.events.lock().unwrap().push(format!("start:{step_id}:{member}"));
        }
        fn on_step_completed(&self, step_id: &str, _output: &str) {
            self.events.lock().unwrap().push(format!("done:{step_id}"));
        }
        fn on_gate(&self, step_id: &str, passed: bool, _verdict: &str) {
            self.events.lock().unwrap().push(format!("gate:{step_id}:{passed}"));
        }
        fn on_mission_finished(&self, report: &MissionReport) {
            self.events
                .lock()
                .unwrap()
                .push(format!("finished:{}", report.succeeded()));
        }
    }

    #[tokio::test]
    async fn run_observed_reports_progress_in_order() {
        // a (delegate) → g (gate, passes) → after_g (delegate).
        let rt = runtime(FakeProvider::always("PASS ok"), &["worker", "reviewer"]);
        let plan = MissionPlan::new(
            "observed",
            vec![
                Step::delegate("a", "worker", "do work"),
                Step::gate("g", "reviewer", "good?").after(["a"]),
                Step::delegate("after_g", "worker", "ship").after(["g"]),
            ],
        );
        let lead = FakeLeadChannel::at(TrustTier::Trusted);
        let obs = RecordingObserver::default();
        let report = rt.run_observed(&plan, &lead, &obs).await.unwrap();
        assert!(report.succeeded());
        assert_eq!(
            obs.snapshot(),
            vec![
                "start:a:worker",
                "done:a",
                "start:g:reviewer",
                "gate:g:true",
                "start:after_g:worker",
                "done:after_g",
                "finished:true",
            ],
            "delegate steps report done, the gate reports its verdict, finished fires once"
        );
    }

    #[tokio::test]
    async fn run_observed_reports_a_gate_rejection() {
        let rt = runtime(FakeProvider::always("FAIL: nope"), &["worker", "reviewer"]);
        let plan = MissionPlan::new(
            "observed-reject",
            vec![
                Step::delegate("a", "worker", "do work"),
                Step::gate("g", "reviewer", "good?").after(["a"]),
                Step::delegate("after_g", "worker", "ship").after(["g"]),
            ],
        );
        let lead = FakeLeadChannel::at(TrustTier::Trusted);
        let obs = RecordingObserver::default();
        let report = rt.run_observed(&plan, &lead, &obs).await.unwrap();
        assert!(matches!(report.status, MissionStatus::GateRejected { .. }));
        let events = obs.snapshot();
        assert!(events.contains(&"gate:g:false".to_string()), "rejection observed: {events:?}");
        assert!(events.contains(&"finished:false".to_string()));
        assert!(
            !events.iter().any(|e| e.starts_with("start:after_g")),
            "the rejected gate's dependent never started: {events:?}"
        );
    }

    // ---- Chapter Ballast: wave-boundary budget halt ----

    /// Returns `None` for the first `allow` wave-boundary checks, then halts —
    /// modelling a per-mission budget that trips after some work has run.
    struct HaltAfterWaves {
        allow: usize,
        seen: std::sync::atomic::AtomicUsize,
    }
    impl MissionObserver for HaltAfterWaves {
        fn should_halt(&self) -> Option<String> {
            let n = self.seen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if n >= self.allow {
                Some(format!("test budget cap (wave {n})"))
            } else {
                None
            }
        }
    }

    #[tokio::test]
    async fn budget_halt_stops_at_wave_boundary_preserving_outputs() {
        // a (wave 1) → b (wave 2). The observer permits one wave boundary, so
        // wave 1 runs and the mission halts before wave 2 launches.
        let rt = runtime(FakeProvider::always("ok"), &["worker"]);
        let plan = MissionPlan::new(
            "halting",
            vec![
                Step::delegate("a", "worker", "step one"),
                Step::delegate("b", "worker", "step two").after(["a"]),
            ],
        );
        let lead = FakeLeadChannel::at(TrustTier::Trusted);
        let obs = HaltAfterWaves {
            allow: 1,
            seen: std::sync::atomic::AtomicUsize::new(0),
        };
        let yielded = rt
            .run_until_pause(&plan, BTreeMap::new(), &lead, &obs)
            .await
            .unwrap();
        let report = match yielded {
            RunYield::Done(r) => r,
            other => panic!("expected a terminal halt, got {other:?}"),
        };
        match &report.status {
            MissionStatus::Halted { reason } => {
                assert!(reason.contains("test budget cap"), "reason: {reason}");
            }
            other => panic!("expected Halted, got {other:?}"),
        }
        // Wave 1's output is preserved; wave 2 never ran.
        assert!(report.outputs.contains_key("a"), "a completed: {:?}", report.outputs);
        assert!(!report.outputs.contains_key("b"), "b never ran: {:?}", report.outputs);
    }

    #[tokio::test]
    async fn no_halt_by_default_runs_to_completion() {
        // The default observer never halts — byte-identical to pre-Ballast.
        let rt = runtime(FakeProvider::always("ok"), &["worker"]);
        let plan = MissionPlan::new(
            "no-halt",
            vec![
                Step::delegate("a", "worker", "step one"),
                Step::delegate("b", "worker", "step two").after(["a"]),
            ],
        );
        let lead = FakeLeadChannel::at(TrustTier::Trusted);
        let report = rt.run(&plan, &lead).await.unwrap();
        assert_eq!(report.status, MissionStatus::Completed);
        assert!(report.outputs.contains_key("b"));
    }

    // ---- L.2: checkpoint/resume + human-approval gates ----

    #[tokio::test]
    async fn human_gate_pauses_then_resumes_to_completion() {
        let rt = runtime(FakeProvider::always("done"), &["worker", "reviewer"]);
        let plan = MissionPlan::new(
            "approval",
            vec![
                Step::delegate("a", "worker", "do work"),
                Step::human_gate("g", "reviewer", "approve?").after(["a"]),
                Step::delegate("ship", "worker", "ship it").after(["g"]),
            ],
        );
        let lead = FakeLeadChannel::at(TrustTier::Trusted);

        // Leg 1: runs `a`, then pauses at the human gate `g`.
        let mut checkpoint = match rt
            .run_until_pause(&plan, Default::default(), &lead, &())
            .await
            .unwrap()
        {
            RunYield::AwaitingHuman { step, outputs } => {
                assert_eq!(step, "g");
                assert!(outputs.contains_key("a"), "upstream ran");
                assert!(!outputs.contains_key("g"), "gate not yet decided");
                assert!(!outputs.contains_key("ship"), "downstream blocked");
                outputs
            }
            other => panic!("expected AwaitingHuman, got {other:?}"),
        };

        // Operator approves: record the gate verdict, resume from the checkpoint.
        checkpoint.insert("g".to_string(), "APPROVED".to_string());
        match rt.run_until_pause(&plan, checkpoint, &lead, &()).await.unwrap() {
            RunYield::Done(report) => {
                assert!(report.succeeded());
                assert!(report.outputs.contains_key("ship"), "downstream ran after approval");
            }
            other => panic!("expected Done after approval, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn resume_path_equals_straight_run_when_no_human_gates() {
        let rt = runtime(FakeProvider::always("x"), &["a", "b"]);
        let plan = MissionPlan::new(
            "linear",
            vec![
                Step::delegate("a", "a", "one"),
                Step::delegate("b", "b", "two").after(["a"]),
            ],
        );
        let lead = FakeLeadChannel::at(TrustTier::Trusted);
        let straight = rt.run(&plan, &lead).await.unwrap();
        let via_pause = match rt
            .run_until_pause(&plan, Default::default(), &lead, &())
            .await
            .unwrap()
        {
            RunYield::Done(r) => r,
            other => panic!("no human gate → Done, got {other:?}"),
        };
        assert_eq!(straight, via_pause, "run_until_pause with no human gates == run");
    }

    #[tokio::test]
    async fn independent_work_completes_before_a_human_gate_pause() {
        // `indep` is unrelated to the gate, so it must run before we pause.
        let rt = runtime(FakeProvider::always("done"), &["worker", "reviewer", "other"]);
        let plan = MissionPlan::new(
            "mixed",
            vec![
                Step::delegate("a", "worker", "work"),
                Step::delegate("indep", "other", "independent"),
                Step::human_gate("g", "reviewer", "ok?").after(["a"]),
            ],
        );
        let lead = FakeLeadChannel::at(TrustTier::Trusted);
        match rt
            .run_until_pause(&plan, Default::default(), &lead, &())
            .await
            .unwrap()
        {
            RunYield::AwaitingHuman { step, outputs } => {
                assert_eq!(step, "g");
                assert!(outputs.contains_key("a"));
                assert!(outputs.contains_key("indep"), "independent work ran before the pause");
            }
            other => panic!("expected AwaitingHuman, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_errors_on_a_human_gate_plan() {
        // The one-shot `run` can't resolve a human gate — it must error, not hang.
        let rt = runtime(FakeProvider::always("done"), &["worker", "reviewer"]);
        let plan = MissionPlan::new(
            "approval",
            vec![
                Step::delegate("a", "worker", "work"),
                Step::human_gate("g", "reviewer", "ok?").after(["a"]),
            ],
        );
        let lead = FakeLeadChannel::at(TrustTier::Trusted);
        let err = rt.run(&plan, &lead).await.unwrap_err();
        assert!(
            format!("{err}").contains("human-approval gate"),
            "expected a human-gate error, got {err}"
        );
    }

    #[tokio::test]
    async fn runs_a_linear_chain_and_collects_outputs() {
        let rt = runtime(FakeProvider::always("done"), &["researcher", "writer"]);
        let plan = MissionPlan::new(
            "write a brief",
            vec![
                Step::delegate("research", "researcher", "gather facts"),
                Step::delegate("write", "writer", "draft it").after(["research"]),
            ],
        );
        let lead = FakeLeadChannel::at(TrustTier::Trusted);
        let report = rt.run(&plan, &lead).await.unwrap();
        assert!(report.succeeded());
        assert_eq!(report.outputs.len(), 2);
        assert_eq!(report.outputs["research"], "done");
        assert_eq!(report.outputs["write"], "done");
    }

    #[tokio::test]
    async fn collects_every_branch_of_a_diamond() {
        let rt = runtime(
            FakeProvider::always("ok"),
            &["a", "b", "c", "reviewer"],
        );
        let plan = MissionPlan::new(
            "diamond",
            vec![
                Step::delegate("a", "a", "root"),
                Step::delegate("b", "b", "left").after(["a"]),
                Step::delegate("c", "c", "right").after(["a"]),
                Step::gate("d", "reviewer", "both branches consistent?").after(["b", "c"]),
            ],
        );
        let lead = FakeLeadChannel::at(TrustTier::Trusted);
        let report = rt.run(&plan, &lead).await.unwrap();
        assert!(report.succeeded(), "the gate's PASS-less 'ok' verdict passes");
        let keys: Vec<&String> = report.outputs.keys().collect();
        assert_eq!(keys, ["a", "b", "c", "d"], "every step recorded");
    }

    #[tokio::test]
    async fn a_failing_gate_aborts_and_skips_dependents() {
        // Every turn returns a FAIL verdict; the gate rejects, so `after_g`
        // (its dependent) must never run.
        let rt = runtime(FakeProvider::always("FAIL: not good enough"), &["worker", "reviewer"]);
        let plan = MissionPlan::new(
            "gated",
            vec![
                Step::delegate("a", "worker", "do work"),
                Step::gate("g", "reviewer", "good enough?").after(["a"]),
                Step::delegate("after_g", "worker", "ship it").after(["g"]),
            ],
        );
        let lead = FakeLeadChannel::at(TrustTier::Trusted);
        let report = rt.run(&plan, &lead).await.unwrap();
        match &report.status {
            MissionStatus::GateRejected { step, verdict } => {
                assert_eq!(step, "g");
                assert!(verdict.contains("FAIL"));
            }
            other => panic!("expected GateRejected, got {other:?}"),
        }
        assert!(report.outputs.contains_key("a"), "upstream output kept");
        assert!(report.outputs.contains_key("g"), "gate verdict kept");
        assert!(!report.outputs.contains_key("after_g"), "dependent skipped");
    }

    #[tokio::test]
    async fn a_passing_gate_lets_dependents_run() {
        let rt = runtime(FakeProvider::always("PASS great work"), &["worker", "reviewer"]);
        let plan = MissionPlan::new(
            "gated",
            vec![
                Step::delegate("a", "worker", "do work"),
                Step::gate("g", "reviewer", "good?").after(["a"]),
                Step::delegate("after_g", "worker", "ship").after(["g"]),
            ],
        );
        let lead = FakeLeadChannel::at(TrustTier::Trusted);
        let report = rt.run(&plan, &lead).await.unwrap();
        assert!(report.succeeded());
        assert!(report.outputs.contains_key("after_g"), "gate passed → dependent ran");
    }

    #[tokio::test]
    async fn an_unknown_specialist_aborts_the_mission() {
        let rt = runtime(FakeProvider::always("x"), &["known"]);
        let plan = MissionPlan::new("m", vec![Step::delegate("a", "ghost", "p")]);
        let lead = FakeLeadChannel::at(TrustTier::Trusted);
        assert!(rt.run(&plan, &lead).await.is_err(), "no such specialist");
    }

    #[tokio::test]
    async fn an_invalid_plan_is_rejected_before_running() {
        let rt = runtime(FakeProvider::always("x"), &["worker"]);
        let cyclic = MissionPlan::new(
            "m",
            vec![
                Step::delegate("a", "worker", "p").after(["b"]),
                Step::delegate("b", "worker", "q").after(["a"]),
            ],
        );
        let lead = FakeLeadChannel::at(TrustTier::Trusted);
        assert!(matches!(rt.run(&cyclic, &lead).await, Err(TeamError::Config(m)) if m.contains("cycle")));
    }

    // --- concurrency proof -------------------------------------------------

    /// A provider whose every turn rendezvouses on a shared barrier before
    /// returning. Two steps run concurrently iff both reach the barrier — if
    /// the runtime ran them sequentially, the first would block forever.
    struct BarrierProvider {
        barrier: Arc<Barrier>,
    }
    #[async_trait]
    impl LlmProvider for BarrierProvider {
        async fn chat_stream(
            &self,
            _: LlmRequest<'_>,
            _: &CancellationToken,
        ) -> Result<Box<dyn LlmStream>, LlmError> {
            self.barrier.wait().await;
            Ok(Box::new(BarrierStream {
                emitted: false,
            }))
        }
    }
    struct BarrierStream {
        emitted: bool,
    }
    #[async_trait]
    impl LlmStream for BarrierStream {
        async fn next_event(&mut self) -> Result<Option<LlmStreamEvent>, LlmError> {
            if self.emitted {
                return Ok(None);
            }
            self.emitted = true;
            Ok(Some(LlmStreamEvent::TextChunk("done".into())))
        }
        async fn finish(self: Box<Self>) -> Result<LlmStepEnd, LlmError> {
            Ok(LlmStepEnd::FinalMessage {
                text: "done".into(),
                usage: aivyx_llm::LlmUsage::default(),
            })
        }
    }

    // A fresh lead channel that doesn't depend on testutil internals.
    struct Lead(SessionId, CancellationToken);
    #[async_trait]
    impl ChannelContext for Lead {
        fn channel_name(&self) -> &str {
            "lead"
        }
        fn platform(&self) -> ChannelPlatform {
            ChannelPlatform::Local
        }
        fn trust_tier(&self) -> TrustTier {
            TrustTier::Trusted
        }
        fn session_id(&self) -> SessionId {
            self.0
        }
        async fn stream_event(&self, _: StreamEvent<'_>) -> Result<(), ChannelError> {
            Ok(())
        }
        async fn finalize(&self, _: &TurnOutcome) -> Result<(), ChannelError> {
            Ok(())
        }
        fn cancellation_token(&self) -> CancellationToken {
            self.1.clone()
        }
    }

    #[tokio::test]
    async fn independent_steps_run_concurrently() {
        // Barrier(2): the two root steps must BOTH reach the provider for
        // either to proceed. A sequential runtime would deadlock; the 2s
        // timeout turns that regression into a clean failure.
        let provider = Arc::new(BarrierProvider {
            barrier: Arc::new(Barrier::new(2)),
        });
        let rt = runtime(provider, &["a", "b"]);
        let plan = MissionPlan::new(
            "parallel",
            vec![Step::delegate("a", "a", "p"), Step::delegate("b", "b", "q")],
        );
        let lead = Lead(SessionId::new(), CancellationToken::new());
        let report = tokio::time::timeout(Duration::from_secs(2), rt.run(&plan, &lead))
            .await
            .expect("did not deadlock → the two steps ran concurrently")
            .expect("mission ok");
        assert_eq!(report.outputs.len(), 2);
    }
}
