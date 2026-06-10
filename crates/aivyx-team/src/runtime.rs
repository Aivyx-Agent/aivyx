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

use aivyx_core::ChannelContext;

use crate::config::TeamError;
use crate::mission::{MissionPlan, StepKind};
use crate::pool::SpecialistPool;

/// How a mission run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MissionStatus {
    /// Every step ran and any gates passed.
    Completed,
    /// A gate's reviewer rejected the upstream work; the gate's dependents
    /// were skipped. The partial outputs are still in the report.
    GateRejected { step: String, verdict: String },
}

/// The result of a mission run: the goal, every completed step's output
/// (`step_id → result`, ordered), and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    pub async fn run(
        &self,
        plan: &MissionPlan,
        lead_channel: &dyn ChannelContext,
    ) -> Result<MissionReport, TeamError> {
        plan.validate()?;

        let mut completed: HashSet<String> = HashSet::new();
        let mut outputs: BTreeMap<String, String> = BTreeMap::new();

        while completed.len() < plan.steps.len() {
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

            // Run every ready step concurrently. Each future captures only
            // owned strings + shared refs (self.pool, lead_channel), so the
            // outputs map is free to mutate once join_all has collected.
            let futures = ready.iter().map(|step| {
                let id = step.id.clone();
                let member = step.kind.member().to_string();
                let input = self.build_input(step, &outputs);
                async move {
                    let res = self.pool.run(&member, &input, lead_channel).await;
                    (id, res)
                }
            });
            let results = futures_util::future::join_all(futures).await;

            let mut rejection: Option<(String, String)> = None;
            for (id, res) in results {
                let output = res?; // a specialist error aborts the whole mission
                let is_gate = matches!(
                    plan.step(&id).map(|s| &s.kind),
                    Some(StepKind::Gate { .. })
                );
                if is_gate && !gate_passed(&output) && rejection.is_none() {
                    rejection = Some((id.clone(), output.clone()));
                }
                outputs.insert(id.clone(), output);
                completed.insert(id);
            }

            if let Some((step, verdict)) = rejection {
                return Ok(MissionReport {
                    goal: plan.goal.clone(),
                    outputs,
                    status: MissionStatus::GateRejected { step, verdict },
                });
            }
        }

        Ok(MissionReport {
            goal: plan.goal.clone(),
            outputs,
            status: MissionStatus::Completed,
        })
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
fn gate_passed(verdict: &str) -> bool {
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
