//! Turn planners — the seam between the loop and whatever drives tool calls.
//!
//! Phase 1 uses a deterministic `VecPlanner` that walks a fixed list of
//! steps. Phase 2 will add an LLM-backed planner that streams tokens and
//! parses tool calls as it goes. The turn loop doesn't care which it has;
//! it just asks `next_step` and dispatches.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::{ChannelContext, Message, TokenUsage, Tool, ToolId, ToolOutcome, ToolOutcomeSummary};

/// What the loop should do next. Mirrors the shapes a real LLM step can
/// produce — a tool call, a final message, or a stop — but with none of
/// the streaming machinery.
#[derive(Debug, Clone)]
pub enum NextStep {
    /// Call this tool with this input. The loop will scope-check it and
    /// either execute or deny.
    ///
    /// Phase 120 — `auto_corrected_from` carries the verbatim name the
    /// LLM originally emitted when the planner's fuzzy-match recovery
    /// path landed on a different `tool_id` than the model said. `None`
    /// for the dominant case (model emitted a registered name verbatim).
    /// Threaded through to the agent's `AuditTag::ToolCall` emission so
    /// forensic walks see the correction.
    ToolCall {
        tool_id: ToolId,
        input: Value,
        #[doc(hidden)]
        auto_corrected_from: Option<String>,
    },

    /// Execute multiple tool calls concurrently. The loop dispatches all
    /// of them via `join_all`, observes every outcome, then asks the
    /// planner for the next step. Phase 40.
    ToolCalls(Vec<ToolCallRequest>),

    /// The planner has a final assistant message for the channel. Loop
    /// terminates with `TurnOutcome::Completed`.
    FinalMessage(String),

    /// No more steps — terminates with `TurnOutcome::Completed` and an
    /// empty final message. Used by planners that finish without a
    /// natural "final message" signal.
    Stop,
}

/// A single tool call within a [`NextStep::ToolCalls`] batch. Carries the
/// resolved `ToolId` (not the string name — resolution happens in the
/// planner before the batch reaches the turn loop).
#[derive(Debug, Clone)]
pub struct ToolCallRequest {
    pub tool_id: ToolId,
    pub input: Value,
    /// Phase 120 — verbatim name the LLM emitted before the planner's
    /// fuzzy-match recovery resolved to this `tool_id`. `None` in the
    /// dominant case. Threaded to the per-call `AuditTag::ToolCall`
    /// emission.
    pub auto_corrected_from: Option<String>,
}

/// What the planner observes after each executed step. Carries only the
/// outcome summary (not the full `ToolOutcome`) so the planner can't peek
/// at the underlying `serde_json::Value` payload and make decisions based
/// on secret-y data — audit stays authoritative.
#[derive(Debug, Clone)]
pub struct StepObservation {
    pub tool_id: ToolId,
    pub summary: ToolOutcomeSummary,
}

/// The seam between the loop and its step source.
///
/// Phase 2 added three methods to this trait — `begin_turn`,
/// `observe_tool_outcome`, and the `channel` parameter on `next_step` —
/// so the LLM-backed planner can see the user message, stream text to
/// the channel as tokens arrive, and read the full `ToolOutcome` after
/// each dispatched call. All three additions have defaults where
/// possible so pre-Phase-2 planners (like [`VecPlanner`]) require
/// minimal updates.
#[async_trait]
pub trait TurnPlanner: Send + Sync {
    /// Called once, at the start of a turn, with the triggering user
    /// message. Deterministic planners can ignore it; LLM-backed
    /// planners seed their conversation history here.
    async fn begin_turn(&mut self, _message: &Message) {}

    /// Return the next step given everything observed so far. The
    /// `channel` handle is available for planners that want to relay
    /// mid-step output (LLM token streaming); planners that don't
    /// stream just ignore it.
    async fn next_step(
        &mut self,
        observed: &[StepObservation],
        channel: &dyn ChannelContext,
    ) -> NextStep;

    /// Called by the turn loop immediately after a [`NextStep::ToolCall`]
    /// has been dispatched and its outcome is known, *before* the loop
    /// asks for the next step. LLM planners use this to append a
    /// `tool_result` message to their conversation history; other
    /// planners default to ignoring it.
    async fn observe_tool_outcome(
        &mut self,
        _tool_id: ToolId,
        _outcome: &ToolOutcome,
    ) {
    }

    /// Cumulative token usage across all LLM steps in this turn.
    /// The turn loop reads this after the step loop exits and passes
    /// it into `AuditTag::TurnEnded`. Deterministic planners return
    /// zero (the default).
    fn turn_usage(&self) -> TokenUsage {
        TokenUsage::default()
    }
}

/// Deterministic planner that walks a fixed script of steps. Used for
/// Phase 1 tests — every step is pre-recorded, no branching on
/// observations. Phase 2's LLM planner will be a different impl of the
/// same trait.
pub struct VecPlanner {
    steps: std::collections::VecDeque<NextStep>,
}

impl VecPlanner {
    pub fn new(steps: impl IntoIterator<Item = NextStep>) -> Self {
        VecPlanner {
            steps: steps.into_iter().collect(),
        }
    }
}

#[async_trait]
impl TurnPlanner for VecPlanner {
    async fn next_step(
        &mut self,
        _observed: &[StepObservation],
        _channel: &dyn ChannelContext,
    ) -> NextStep {
        self.steps.pop_front().unwrap_or(NextStep::Stop)
    }
}

/// A tool registry — how the loop looks up a `Tool` by its `ToolId`. The
/// simplest possible registry is a `Vec<Arc<dyn Tool>>` scanned linearly.
/// Real runtimes will hash by id; Phase 1 tests don't care.
pub struct ToolRegistry {
    tools: Vec<Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new(tools: Vec<Arc<dyn Tool>>) -> Self {
        ToolRegistry { tools }
    }

    pub fn get(&self, id: ToolId) -> Option<&Arc<dyn Tool>> {
        self.tools.iter().find(|t| t.id() == id)
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Iterate over every registered tool. Used by the LLM planner to
    /// build the `LlmToolDescriptor` list at construction time.
    pub fn iter_tools(&self) -> impl Iterator<Item = &Arc<dyn Tool>> {
        self.tools.iter()
    }

    /// Look up a tool by its human name. Linear scan — the registry
    /// holds at most a few dozen tools in realistic use, and the LLM
    /// planner only calls this once per LLM step.
    pub fn find_by_name(&self, name: &str) -> Option<ToolId> {
        self.tools.iter().find(|t| t.name() == name).map(|t| t.id())
    }
}

// ---------------------------------------------------------------------------
// Phase 4 task 1 — Tool trait surface audit
//
// The purpose of this test module is *not* to exercise `ToolRegistry`
// behavior — that's covered indirectly through `agent.rs`'s happy-path
// and scope-denial tests. It exists to prove, ahead of Phase 4 task 2's
// `FsReadTool`, that a concrete `Tool` impl with the shape a real
// filesystem tool needs:
//
//   - holds its own state (here, a scope prefix; there, a sandbox root)
//   - derives an input-specific `Scope` from the input JSON via R1
//   - is reachable through every registry lookup path (`get`,
//     `find_by_name`, `iter_tools`)
//   - surfaces a JSON input schema the LLM planner can stringify
//
// ...compiles and works against the existing Phase 0–3 trait surface
// with *no* amendment to the `Tool` trait, the `ToolContext` struct,
// the `ToolOutcome` enum, or the `ToolRegistry` API. If this test ever
// stops compiling without changes to the test itself, the contract has
// drifted and task 2's real filesystem tool is at risk.
//
// See `docs/PHASE_4.md` task 1 for the design decision.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tool_surface_audit {
    use super::*;

    use std::sync::OnceLock;

    use async_trait::async_trait;
    use serde_json::json;

    use aivyx_capability::{CapabilitySet, Scope, TrustTier};

    use crate::{Tool, ToolContext, ToolOutcome, Verification};

    /// A skeleton tool shaped like the upcoming `FsReadTool`: holds a
    /// scope-prefix string ("sandbox root"), derives a path-qualified
    /// scope from the input's `"path"` field, and returns a
    /// `Completed/NotApplicable` result. No real filesystem access —
    /// this is a type-level audit, not a behavioral test.
    struct FsReadSkeleton {
        id: ToolId,
        sandbox_root: String,
    }

    impl FsReadSkeleton {
        fn new(sandbox_root: impl Into<String>) -> Self {
            FsReadSkeleton {
                id: ToolId::new(),
                sandbox_root: sandbox_root.into(),
            }
        }
    }

    #[async_trait]
    impl Tool for FsReadSkeleton {
        fn id(&self) -> ToolId {
            self.id
        }
        fn name(&self) -> &str {
            "fs.read"
        }
        fn description(&self) -> &str {
            "Read a UTF-8 file under the agent's sandbox root."
        }
        fn input_schema(&self) -> &serde_json::Value {
            static SCHEMA: OnceLock<serde_json::Value> = OnceLock::new();
            SCHEMA.get_or_init(|| {
                json!({
                    "type": "object",
                    "properties": {
                        "path": {
                            "type": "string",
                            "description": "Relative path under the sandbox root."
                        }
                    },
                    "required": ["path"]
                })
            })
        }

        fn required_scope(&self, input: &serde_json::Value) -> Scope {
            // R1: the scope is derived from the *input*, not hard-coded.
            // Phase 4 task 2's real tool will canonicalize the path and
            // compare the canonicalized form against the sandbox prefix
            // (Q4 in PHASE_4.md). The skeleton here keeps the shape —
            // a qualified `fs.read:<path>` — without doing I/O.
            let path = input
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("/dev/null");
            Scope::parse(&format!("fs.read:{}/{path}", self.sandbox_root))
                .expect("sandbox path forms a legal scope qualifier")
        }

        async fn execute(
            &self,
            _input: serde_json::Value,
            _ctx: &ToolContext<'_>,
        ) -> ToolOutcome {
            // The task-1 skeleton never actually runs — the audit is
            // structural, not behavioral. Task 2 provides the real impl.
            ToolOutcome::Completed {
                output: json!({"bytes": 0}),
                verified: Verification::NotApplicable,
            }
        }
    }

    #[test]
    fn concrete_tool_is_reachable_through_every_registry_lookup_path() {
        let tool = Arc::new(FsReadSkeleton::new("/home/user/aivyx-sandbox"));
        let id = tool.id();
        let registry = ToolRegistry::new(vec![tool as Arc<dyn Tool>]);

        assert!(!registry.is_empty());

        // Path 1: LLM planner round-trips `name → id → Tool` at tool-call
        // dispatch time. Both halves must succeed for a real tool call
        // to reach execute.
        let found_id = registry
            .find_by_name("fs.read")
            .expect("fs.read must be findable by name");
        assert_eq!(
            found_id, id,
            "name lookup must return the same id the tool reports"
        );

        let found_tool = registry
            .get(found_id)
            .expect("id lookup must return the tool");
        assert_eq!(found_tool.name(), "fs.read");

        // Path 2: LLM planner walks `iter_tools` once at construction to
        // build the descriptor list it sends to the model.
        let names: Vec<&str> = registry.iter_tools().map(|t| t.name()).collect();
        assert_eq!(names, vec!["fs.read"]);

        // Path 3: `input_schema()` returns a stable `&serde_json::Value`
        // — the descriptor-building path at the planner needs this to
        // serialize into the Anthropic `tool_input_schema` field.
        let schema = found_tool.input_schema();
        assert_eq!(schema["type"], json!("object"));
        assert_eq!(schema["required"], json!(["path"]));
    }

    #[test]
    fn r1_scope_derivation_uses_the_input_path() {
        let tool = FsReadSkeleton::new("/home/user/aivyx-sandbox");

        // A read under the sandbox derives a path-qualified scope.
        let scope = tool.required_scope(&json!({"path": "notes/today.md"}));
        assert_eq!(scope.base(), "fs.read");
        assert_eq!(
            scope.qualifier(),
            Some("/home/user/aivyx-sandbox/notes/today.md")
        );

        // A missing-path input falls back to /dev/null. This is
        // deliberately a *legal* scope — task 2's real tool will instead
        // fail the scope-derivation step by returning an error path, and
        // the loop's scope check at `agent.rs:314` will deny the call.
        // Here we just prove the trait method is *pure* (no panic on
        // missing field, no I/O).
        let fallback = tool.required_scope(&json!({}));
        assert_eq!(fallback.base(), "fs.read");
    }

    #[test]
    fn broad_capability_grants_derived_narrow_scope() {
        // Reprises the `r1_narrow_scope_is_granted_by_broad_capability`
        // check from lib.rs, but against the filesystem shape. An agent
        // with `fs.read:/home/user/aivyx-sandbox/**` must be able to
        // cover a tool call whose R1 derives
        // `fs.read:/home/user/aivyx-sandbox/notes/today.md`. This is the
        // scope system's core promise and the reason Phase 4 picked a
        // filesystem tool to stress-test it.
        let held = CapabilitySet::from_scopes([
            Scope::parse("fs.read:/home/user/aivyx-sandbox/**").unwrap(),
        ]);
        let effective = held.intersect(TrustTier::Trusted.default_ceiling());

        let tool = FsReadSkeleton::new("/home/user/aivyx-sandbox");
        let needed = tool.required_scope(&json!({"path": "notes/today.md"}));

        assert!(
            effective.grants(&needed),
            "broad sandbox scope must cover a path under the sandbox root"
        );
    }

    #[test]
    fn out_of_sandbox_scope_is_not_granted_by_sandbox_capability() {
        // The negative — the *whole point* of prefix-attenuated scopes.
        // An agent holding `fs.read:/home/user/aivyx-sandbox/**` must
        // NOT cover `fs.read:/etc/passwd`. Task 2's real tool will reach
        // this by canonicalizing an evil input like `"../etc/passwd"`;
        // the skeleton here simulates by passing an absolute path
        // through the template directly.
        let held = CapabilitySet::from_scopes([
            Scope::parse("fs.read:/home/user/aivyx-sandbox/**").unwrap(),
        ]);
        let effective = held.intersect(TrustTier::Trusted.default_ceiling());

        let attacker = Scope::parse("fs.read:/etc/passwd").unwrap();
        assert!(
            !effective.grants(&attacker),
            "sandbox scope must not grant reads outside the sandbox prefix"
        );
    }
}
