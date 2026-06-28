//! Orchestration tools — the lead's mission interface (J.4.3): build + run a
//! DAG, weave results, and gate-check an output. All three require the
//! **`team.delegate`** scope — the same lead-only orchestration authority as
//! `delegate_task`, so a specialist (which never declares it) can neither
//! launch a mission nor convene a verification.
//!
//! - [`DecomposeTaskTool`] takes the lead's proposed plan, validates it into
//!   a [`MissionPlan`], and runs it on the [`TeamRuntime`] — concurrency and
//!   result collection happen inside the one tool call (the turn loop is
//!   sequential, so parallelism must live here).
//! - [`SynthesizeResultsTool`] assembles labeled outputs into one structured
//!   deliverable (deterministic; the lead's own prose synthesis is its turn).
//! - [`VerifyOutputTool`] runs a reviewer specialist as an ad-hoc quality
//!   gate, judged by the same rule as an in-DAG [`Gate`](crate::mission::StepKind::Gate).

use std::sync::Arc;

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};
use async_trait::async_trait;
use serde_json::{json, Map, Value};

use crate::mission::{GateMode, MissionPlan, Step, StepKind};
use crate::pool::SpecialistPool;
use crate::runtime::{gate_passed, MissionStatus, TeamRuntime};

const DELEGATE_SCOPE: &str = "team.delegate";

fn scope() -> Scope {
    Scope::parse(DELEGATE_SCOPE).expect("team.delegate must be a known base")
}

fn fail(id: ToolId, detail: impl Into<String>) -> ToolOutcome {
    ToolOutcome::Failed(AivyxError::Tool {
        tool: id,
        detail: detail.into(),
    })
}

// === decompose_task ========================================================

/// Parse one step object into a [`Step`] (a gate if it names a `reviewer`,
/// else a delegate).
fn parse_step(v: &Value) -> Result<Step, String> {
    let id = v.get("id").and_then(Value::as_str).ok_or("a step needs an `id` (string)")?;
    let deps: Vec<String> = match v.get("deps") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => a
            .iter()
            .map(|d| d.as_str().map(str::to_string).ok_or_else(|| format!("step {id:?}: each dep must be a string")))
            .collect::<Result<_, _>>()?,
        Some(_) => return Err(format!("step {id:?}: `deps` must be an array of step ids")),
    };

    let kind = if let Some(reviewer) = v.get("reviewer").and_then(Value::as_str) {
        let criteria = v.get("criteria").and_then(Value::as_str).unwrap_or_default();
        // Chapter L — a gate is human-approval when `"mode": "human"`; anything
        // else (including absent) is the default automatic reviewer gate.
        let mode = match v.get("mode").and_then(Value::as_str) {
            Some("human") => GateMode::Human,
            _ => GateMode::Auto,
        };
        StepKind::Gate {
            reviewer: reviewer.to_string(),
            criteria: criteria.to_string(),
            mode,
        }
    } else if let Some(specialist) = v.get("specialist").and_then(Value::as_str) {
        let prompt = v
            .get("prompt")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("delegate step {id:?} needs a `prompt` (string)"))?;
        StepKind::Delegate {
            specialist: specialist.to_string(),
            prompt: prompt.to_string(),
        }
    } else {
        return Err(format!(
            "step {id:?} must name either a `specialist` (delegate) or a `reviewer` (gate)"
        ));
    };

    Ok(Step {
        id: id.to_string(),
        kind,
        deps,
    })
}

/// Parse the `{ goal, steps }` input into a (still-unvalidated) plan.
fn parse_plan(input: &Value) -> Result<MissionPlan, String> {
    let goal = input.get("goal").and_then(Value::as_str).ok_or("`goal` (string) is required")?;
    let steps = input.get("steps").and_then(Value::as_array).ok_or("`steps` (array) is required")?;
    let steps = steps.iter().map(parse_step).collect::<Result<Vec<_>, _>>()?;
    Ok(MissionPlan::new(goal, steps))
}

/// Parse the lead's friendly `{ goal, steps: [{ id, specialist|reviewer, … }] }`
/// spec into a (still-unvalidated) [`MissionPlan`] — the same shape the
/// `decompose_task` tool accepts. Chapter L (L.5) exposes it so the daemon-run
/// `aivyx team start --plan <file.json>` path can author a plan by hand without
/// the verbose serde-tagged `StepKind` wire form.
pub fn parse_plan_spec(input: &Value) -> Result<MissionPlan, String> {
    parse_plan(input)
}

/// `decompose_task` — build a mission DAG from the lead's plan and run it.
pub struct DecomposeTaskTool {
    id: ToolId,
    runtime: Arc<TeamRuntime>,
    schema: Value,
}

impl DecomposeTaskTool {
    pub fn new(runtime: Arc<TeamRuntime>) -> Self {
        DecomposeTaskTool {
            id: ToolId::new(),
            runtime,
            schema: json!({
                "type": "object",
                "properties": {
                    "goal": { "type": "string" },
                    "steps": {
                        "type": "array",
                        "description": "DAG steps. A delegate step has {specialist, prompt}; a gate step has {reviewer, criteria}. `deps` lists step ids that must finish first.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "specialist": { "type": "string" },
                                "prompt": { "type": "string" },
                                "reviewer": { "type": "string" },
                                "criteria": { "type": "string" },
                                "deps": { "type": "array", "items": { "type": "string" } }
                            },
                            "required": ["id"]
                        }
                    }
                },
                "required": ["goal", "steps"],
                "additionalProperties": false
            }),
        }
    }
}

#[async_trait]
impl Tool for DecomposeTaskTool {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "decompose_task"
    }
    fn description(&self) -> &str {
        "Decompose a mission into a DAG of steps and run it. Steps run concurrently where their \
         dependencies allow. Input: { \"goal\": string, \"steps\": [{ \"id\": string, \
         \"specialist\"+\"prompt\" (delegate) | \"reviewer\"+\"criteria\" (gate), \"deps\"?: [id] }] }. \
         Returns each step's output."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _: &Value) -> Scope {
        scope()
    }
    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> ToolOutcome {
        let plan = match parse_plan(&input) {
            Ok(plan) => plan,
            Err(e) => return fail(self.id, e),
        };
        match self.runtime.run(&plan, ctx.channel).await {
            Ok(report) => {
                let outputs: Map<String, Value> = report
                    .outputs
                    .into_iter()
                    .map(|(k, v)| (k, Value::String(v)))
                    .collect();
                let mut out = json!({ "goal": report.goal, "outputs": outputs });
                match report.status {
                    MissionStatus::Completed => out["status"] = json!("completed"),
                    MissionStatus::GateRejected { step, verdict } => {
                        out["status"] = json!("gate_rejected");
                        out["rejected_step"] = json!(step);
                        out["verdict"] = json!(verdict);
                    }
                    // This in-process path runs with the null observer, which
                    // never halts; the arm exists for exhaustiveness.
                    MissionStatus::Halted { reason } => {
                        out["status"] = json!("halted");
                        out["halt_reason"] = json!(reason);
                    }
                }
                ToolOutcome::Completed {
                    output: out,
                    verified: Verification::Unverified,
                }
            }
            Err(e) => fail(self.id, e.to_string()),
        }
    }
}

// === synthesize_results ====================================================

/// `synthesize_results` — assemble labeled outputs into one deliverable.
pub struct SynthesizeResultsTool {
    id: ToolId,
    schema: Value,
}

impl SynthesizeResultsTool {
    pub fn new() -> Self {
        SynthesizeResultsTool {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "directive": { "type": "string", "description": "Optional title/framing for the deliverable." },
                    "outputs": {
                        "type": "object",
                        "description": "label -> text. Pass decompose_task's `outputs` straight through.",
                        "additionalProperties": { "type": "string" }
                    }
                },
                "required": ["outputs"],
                "additionalProperties": false
            }),
        }
    }
}

impl Default for SynthesizeResultsTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SynthesizeResultsTool {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "synthesize_results"
    }
    fn description(&self) -> &str {
        "Weave specialist outputs into one structured deliverable. Input: \
         { \"directive\"?: string, \"outputs\": { label: text, ... } }."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _: &Value) -> Scope {
        scope()
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let Some(outputs) = input.get("outputs").and_then(Value::as_object) else {
            return fail(self.id, "`outputs` (object of label -> text) is required");
        };
        if outputs.is_empty() {
            return fail(self.id, "`outputs` is empty — nothing to synthesize");
        }
        let title = input
            .get("directive")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or("Synthesis");

        // Deterministic: labels in sorted order (serde_json::Map iterates
        // insertion-ordered, so sort explicitly).
        let mut labels: Vec<&String> = outputs.keys().collect();
        labels.sort();
        let mut deliverable = format!("# {title}\n");
        for label in labels {
            let text = outputs[label].as_str().unwrap_or_default();
            deliverable.push_str(&format!("\n## {label}\n{text}\n"));
        }

        ToolOutcome::Completed {
            output: json!({ "deliverable": deliverable }),
            verified: Verification::Unverified,
        }
    }
}

// === verify_output =========================================================

/// `verify_output` — an ad-hoc quality gate: a reviewer specialist judges an
/// output against criteria, with the same PASS/FAIL rule as an in-DAG gate.
pub struct VerifyOutputTool {
    id: ToolId,
    pool: Arc<SpecialistPool>,
    schema: Value,
}

impl VerifyOutputTool {
    pub fn new(pool: Arc<SpecialistPool>) -> Self {
        VerifyOutputTool {
            id: ToolId::new(),
            pool,
            schema: json!({
                "type": "object",
                "properties": {
                    "reviewer": { "type": "string", "description": "The specialist to review." },
                    "output": { "type": "string", "description": "The work to check." },
                    "criteria": { "type": "string" }
                },
                "required": ["reviewer", "output", "criteria"],
                "additionalProperties": false
            }),
        }
    }
}

#[async_trait]
impl Tool for VerifyOutputTool {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "verify_output"
    }
    fn description(&self) -> &str {
        "Have a reviewer specialist quality-check an output against criteria. Input: \
         { \"reviewer\": string, \"output\": string, \"criteria\": string }. Returns \
         { verdict, passed }."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _: &Value) -> Scope {
        scope()
    }
    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> ToolOutcome {
        let reviewer = input.get("reviewer").and_then(Value::as_str);
        let output = input.get("output").and_then(Value::as_str);
        let criteria = input.get("criteria").and_then(Value::as_str);
        let (Some(reviewer), Some(output), Some(criteria)) = (reviewer, output, criteria) else {
            return fail(self.id, "`reviewer`, `output`, and `criteria` (strings) are required");
        };
        let prompt = format!(
            "Review the work below against the criteria. Begin your reply with PASS or FAIL, \
             then explain.\n\nCriteria: {criteria}\n\nWork:\n{output}"
        );
        match self.pool.run(reviewer, &prompt, ctx.channel).await {
            Ok(verdict) => {
                let passed = gate_passed(&verdict);
                ToolOutcome::Completed {
                    output: json!({ "verdict": verdict, "passed": passed }),
                    verified: Verification::Unverified,
                }
            }
            Err(e) => fail(self.id, e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{member, team_pool, FakeLeadChannel, FakeProvider};
    use aivyx_capability::TrustTier;
    use aivyx_core::{AgentId, CancellationToken, ChannelContext, NullAuditHook, TurnId};
    use aivyx_llm::LlmProvider;

    macro_rules! ctx {
        ($ch:expr, $audit:expr, $tok:expr) => {
            ToolContext {
                agent_id: AgentId::new(),
                session_id: $ch.session_id(),
                turn_id: TurnId::new(),
                channel: &$ch,
                audit: &$audit,
                cancellation: &$tok,
            }
        };
    }

    fn pool(provider: Arc<dyn LlmProvider>, members: &[&str]) -> Arc<SpecialistPool> {
        let mut roster = vec![member("lead", &[], TrustTier::Trusted)];
        for m in members {
            roster.push(member(m, &[], TrustTier::Trusted));
        }
        Arc::new(team_pool(provider, roster, "lead", &[]))
    }
    fn runtime(provider: Arc<dyn LlmProvider>, members: &[&str]) -> Arc<TeamRuntime> {
        Arc::new(TeamRuntime::new(pool(provider, members)))
    }

    // --- decompose_task ----------------------------------------------------

    #[test]
    fn decompose_surface_and_scope() {
        let tool = DecomposeTaskTool::new(runtime(FakeProvider::always("x"), &[]));
        assert_eq!(tool.name(), "decompose_task");
        assert_eq!(tool.required_scope(&Value::Null).base(), "team.delegate");
    }

    #[test]
    fn parse_step_distinguishes_delegate_and_gate() {
        let d = parse_step(&json!({ "id": "a", "specialist": "coder", "prompt": "build" })).unwrap();
        assert!(matches!(d.kind, StepKind::Delegate { .. }));
        let g = parse_step(&json!({ "id": "g", "reviewer": "rev", "criteria": "ok?", "deps": ["a"] })).unwrap();
        assert!(matches!(g.kind, StepKind::Gate { .. }));
        assert_eq!(g.deps, ["a"]);
        // A delegate step missing its prompt, and a step naming neither role.
        assert!(parse_step(&json!({ "id": "a", "specialist": "coder" })).is_err());
        assert!(parse_step(&json!({ "id": "a" })).is_err());
    }

    #[tokio::test]
    async fn decompose_runs_a_plan_and_returns_outputs() {
        let tool = DecomposeTaskTool::new(runtime(FakeProvider::always("done"), &["researcher", "writer"]));
        let ch = FakeLeadChannel::at(TrustTier::Trusted);
        let audit = NullAuditHook;
        let tok = CancellationToken::new();
        let input = json!({
            "goal": "brief",
            "steps": [
                { "id": "r", "specialist": "researcher", "prompt": "gather" },
                { "id": "w", "specialist": "writer", "prompt": "draft", "deps": ["r"] }
            ]
        });
        match tool.execute(input, &ctx!(ch, audit, tok)).await {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["status"], json!("completed"));
                assert_eq!(output["outputs"]["r"], json!("done"));
                assert_eq!(output["outputs"]["w"], json!("done"));
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn decompose_reports_a_gate_rejection() {
        let tool = DecomposeTaskTool::new(runtime(FakeProvider::always("FAIL: nope"), &["worker", "reviewer"]));
        let ch = FakeLeadChannel::at(TrustTier::Trusted);
        let audit = NullAuditHook;
        let tok = CancellationToken::new();
        let input = json!({
            "goal": "gated",
            "steps": [
                { "id": "a", "specialist": "worker", "prompt": "work" },
                { "id": "g", "reviewer": "reviewer", "criteria": "ok?", "deps": ["a"] },
                { "id": "z", "specialist": "worker", "prompt": "ship", "deps": ["g"] }
            ]
        });
        match tool.execute(input, &ctx!(ch, audit, tok)).await {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["status"], json!("gate_rejected"));
                assert_eq!(output["rejected_step"], json!("g"));
                assert!(output["outputs"].get("z").is_none(), "dependent skipped");
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn decompose_rejects_a_cyclic_plan() {
        let tool = DecomposeTaskTool::new(runtime(FakeProvider::always("x"), &["w"]));
        let ch = FakeLeadChannel::at(TrustTier::Trusted);
        let audit = NullAuditHook;
        let tok = CancellationToken::new();
        let input = json!({
            "goal": "g",
            "steps": [
                { "id": "a", "specialist": "w", "prompt": "p", "deps": ["b"] },
                { "id": "b", "specialist": "w", "prompt": "q", "deps": ["a"] }
            ]
        });
        assert!(matches!(tool.execute(input, &ctx!(ch, audit, tok)).await, ToolOutcome::Failed(_)));
    }

    #[tokio::test]
    async fn decompose_rejects_a_malformed_step() {
        let tool = DecomposeTaskTool::new(runtime(FakeProvider::always("x"), &["w"]));
        let ch = FakeLeadChannel::at(TrustTier::Trusted);
        let audit = NullAuditHook;
        let tok = CancellationToken::new();
        // Step names neither a specialist nor a reviewer.
        let input = json!({ "goal": "g", "steps": [{ "id": "a" }] });
        assert!(matches!(tool.execute(input, &ctx!(ch, audit, tok)).await, ToolOutcome::Failed(_)));
    }

    // --- synthesize_results ------------------------------------------------

    #[tokio::test]
    async fn synthesize_assembles_outputs_deterministically() {
        let tool = SynthesizeResultsTool::new();
        let ch = FakeLeadChannel::at(TrustTier::Trusted);
        let audit = NullAuditHook;
        let tok = CancellationToken::new();
        let input = json!({
            "directive": "Close-down report",
            "outputs": { "stocktake": "all counted", "haccp": "fridge ok" }
        });
        match tool.execute(input, &ctx!(ch, audit, tok)).await {
            ToolOutcome::Completed { output, .. } => {
                let d = output["deliverable"].as_str().unwrap();
                assert!(d.starts_with("# Close-down report"));
                // Sorted: haccp before stocktake regardless of input order.
                assert!(d.find("## haccp").unwrap() < d.find("## stocktake").unwrap());
                assert!(d.contains("fridge ok") && d.contains("all counted"));
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn synthesize_rejects_empty_outputs() {
        let tool = SynthesizeResultsTool::new();
        let ch = FakeLeadChannel::at(TrustTier::Trusted);
        let audit = NullAuditHook;
        let tok = CancellationToken::new();
        let out = tool.execute(json!({ "outputs": {} }), &ctx!(ch, audit, tok)).await;
        assert!(matches!(out, ToolOutcome::Failed(_)));
        assert_eq!(tool.required_scope(&Value::Null).base(), "team.delegate");
    }

    // --- verify_output -----------------------------------------------------

    #[tokio::test]
    async fn verify_passes_on_a_pass_verdict() {
        let tool = VerifyOutputTool::new(pool(FakeProvider::always("PASS, all good"), &["reviewer"]));
        let ch = FakeLeadChannel::at(TrustTier::Trusted);
        let audit = NullAuditHook;
        let tok = CancellationToken::new();
        let input = json!({ "reviewer": "reviewer", "output": "the work", "criteria": "complete?" });
        match tool.execute(input, &ctx!(ch, audit, tok)).await {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["passed"], json!(true));
                assert!(output["verdict"].as_str().unwrap().contains("PASS"));
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn verify_fails_on_a_fail_verdict() {
        let tool = VerifyOutputTool::new(pool(FakeProvider::always("FAIL: incomplete"), &["reviewer"]));
        let ch = FakeLeadChannel::at(TrustTier::Trusted);
        let audit = NullAuditHook;
        let tok = CancellationToken::new();
        let input = json!({ "reviewer": "reviewer", "output": "the work", "criteria": "complete?" });
        match tool.execute(input, &ctx!(ch, audit, tok)).await {
            ToolOutcome::Completed { output, .. } => assert_eq!(output["passed"], json!(false)),
            other => panic!("expected Completed, got {other:?}"),
        }
        assert_eq!(tool.name(), "verify_output");
        assert_eq!(tool.required_scope(&Value::Null).base(), "team.delegate");
    }
}
