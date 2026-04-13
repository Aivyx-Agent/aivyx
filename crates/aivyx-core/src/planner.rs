//! Turn planners — the seam between the loop and whatever drives tool calls.
//!
//! Phase 1 uses a deterministic `VecPlanner` that walks a fixed list of
//! steps. Phase 2 will add an LLM-backed planner that streams tokens and
//! parses tool calls as it goes. The turn loop doesn't care which it has;
//! it just asks `next_step` and dispatches.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::{Tool, ToolId, ToolOutcomeSummary};

/// What the loop should do next. Mirrors the shapes a real LLM step can
/// produce — a tool call, a final message, or a stop — but with none of
/// the streaming machinery.
#[derive(Debug, Clone)]
pub enum NextStep {
    /// Call this tool with this input. The loop will scope-check it and
    /// either execute or deny.
    ToolCall { tool_id: ToolId, input: Value },

    /// The planner has a final assistant message for the channel. Loop
    /// terminates with `TurnOutcome::Completed`.
    FinalMessage(String),

    /// No more steps — terminates with `TurnOutcome::Completed` and an
    /// empty final message. Used by planners that finish without a
    /// natural "final message" signal.
    Stop,
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
#[async_trait]
pub trait TurnPlanner: Send + Sync {
    /// Return the next step given everything observed so far. May return
    /// `Stop` at any time to terminate.
    async fn next_step(&mut self, observed: &[StepObservation]) -> NextStep;
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
    async fn next_step(&mut self, _observed: &[StepObservation]) -> NextStep {
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
}
