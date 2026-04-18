//! `ReflectionProposeTool` and `ReflectionApplyTool` — Phase 29
//! Agent Reflection Loop (PRODUCT.md G3/P8).
//!
//! The propose tool reads recent turn outcomes from the audit chain,
//! identifies patterns (repeated failures, timeouts, denied scopes),
//! and emits a structured proposal. It creates a mission with a gate
//! for operator approval.
//!
//! The apply tool takes a proposal ID (which is also the mission ID),
//! verifies the gate is approved, and executes the proposed memory
//! writes via the memory substrate.
//!
//! ## `reflection.propose` input
//!
//! ```json
//! { "lookback": 20 }
//! ```
//!
//! `lookback` is optional (default 20) — how many recent turns to
//! analyze.
//!
//! ## `reflection.propose` output
//!
//! ```json
//! {
//!   "proposal_id": "rp-...",
//!   "mission_id": "m-...",
//!   "observations": ["5 of last 10 turns timed out"],
//!   "memory_writes": [
//!     { "topic": "reflection:timeout-pattern", "content": "..." }
//!   ]
//! }
//! ```
//!
//! ## `reflection.apply` input
//!
//! ```json
//! { "proposal_id": "rp-..." }
//! ```
//!
//! ## `reflection.apply` output
//!
//! ```json
//! { "applied": true, "writes_executed": 2 }
//! ```

use std::sync::OnceLock;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use aivyx_audit::{AuditEvent, AuditLog};
use aivyx_capability::Scope;
use aivyx_core::{
    AivyxError, Tool, ToolContext, ToolId, ToolOutcome, TurnOutcomeSummary, Verification,
};
use aivyx_memory::Memory;
use aivyx_storage::{DomainHandle, KeyDomain};

use crate::mission::{
    self, GateState, MissionRecord, MissionState,
};

// ---------------------------------------------------------------------------
// Proposal record — serialized into the mission description
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProposalRecord {
    proposal_id: String,
    observations: Vec<String>,
    memory_writes: Vec<ProposedWrite>,
    /// Phase 30 — text to append to the system prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt_append: Option<String>,
    /// Phase 30 — tool allowlist additions and removals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    allowlist_changes: Option<AllowlistChanges>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProposedWrite {
    topic: String,
    content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AllowlistChanges {
    #[serde(default)]
    add: Vec<String>,
    #[serde(default)]
    remove: Vec<String>,
}

// ---------------------------------------------------------------------------
// reflection.propose
// ---------------------------------------------------------------------------

pub struct ReflectionProposeTool {
    id: ToolId,
    schema: Value,
    audit_log: OnceLock<std::sync::Arc<dyn AuditLog + Send + Sync>>,
    mission_store: OnceLock<DomainHandle>,
    role_name: OnceLock<String>,
}

impl std::fmt::Debug for ReflectionProposeTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReflectionProposeTool")
            .field("id", &self.id)
            .field("has_audit_log", &self.audit_log.get().is_some())
            .finish()
    }
}

impl Default for ReflectionProposeTool {
    fn default() -> Self {
        Self::new()
    }
}

impl ReflectionProposeTool {
    pub fn new() -> Self {
        ReflectionProposeTool {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "lookback": {
                        "type": "integer",
                        "description": "Number of recent turns to analyze (default 20)"
                    }
                },
                "required": []
            }),
            audit_log: OnceLock::new(),
            mission_store: OnceLock::new(),
            role_name: OnceLock::new(),
        }
    }

    pub fn set_audit_log(
        &self,
        log: std::sync::Arc<dyn AuditLog + Send + Sync>,
    ) -> Result<(), std::sync::Arc<dyn AuditLog + Send + Sync>> {
        self.audit_log.set(log)
    }

    pub fn set_mission_store(&self, handle: DomainHandle) -> Result<(), DomainHandle> {
        assert_eq!(handle.domain(), KeyDomain::Missions);
        self.mission_store.set(handle)
    }

    pub fn set_role_name(&self, name: String) -> Result<(), String> {
        self.role_name.set(name)
    }
}

#[async_trait]
impl Tool for ReflectionProposeTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "reflection.propose"
    }

    fn description(&self) -> &str {
        "Analyze recent turn outcomes and propose behavioral adjustments. \
         Reads the audit chain to identify patterns (repeated failures, \
         timeouts, denied scopes) and emits a structured proposal with \
         memory writes. Creates a mission with an approval gate — the \
         operator must approve before reflection.apply can execute."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("reflection.propose").expect("known base")
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let Some(log) = self.audit_log.get() else {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "reflection.propose: no audit log configured".to_string(),
            });
        };
        let Some(store) = self.mission_store.get() else {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "reflection.propose: no mission store configured".to_string(),
            });
        };

        let lookback = input
            .get("lookback")
            .and_then(|v| v.as_u64())
            .unwrap_or(20) as usize;

        // Collect recent turn outcomes by scanning the audit chain.
        let total = log.len();
        let mut turn_outcomes: Vec<(String, TurnOutcomeSummary, usize)> = Vec::new();
        let mut scope_denials: Vec<String> = Vec::new();

        let mut ended_map: std::collections::HashMap<
            String,
            (TurnOutcomeSummary, usize),
        > = std::collections::HashMap::new();

        let mut seq = total as u64;
        while seq > 0 && turn_outcomes.len() < lookback {
            seq -= 1;
            let Some(entry) = log.get(seq) else { break };

            match &entry.event {
                AuditEvent::TurnStarted { turn_id, .. } => {
                    let tid = format!("{turn_id:?}");
                    if let Some((outcome, tool_calls)) = ended_map.remove(&tid) {
                        turn_outcomes.push((tid, outcome, tool_calls));
                    }
                }
                AuditEvent::TurnEnded {
                    turn_id,
                    outcome,
                    tool_calls_made,
                    ..
                } => {
                    let tid = format!("{turn_id:?}");
                    ended_map.insert(tid, (outcome.clone(), *tool_calls_made));
                }
                AuditEvent::ScopeDenied {
                    scope_requested, ..
                } => {
                    scope_denials.push(scope_requested.as_str().to_string());
                }
                _ => {}
            }
        }

        // Analyze patterns.
        let mut observations: Vec<String> = Vec::new();
        let mut memory_writes: Vec<ProposedWrite> = Vec::new();

        // Count outcome types.
        let total_turns = turn_outcomes.len();
        if total_turns == 0 {
            return ToolOutcome::Completed {
                output: json!({
                    "proposal_id": null,
                    "observations": ["No recent turns found — nothing to reflect on."],
                    "memory_writes": []
                }),
                verified: Verification::NotApplicable,
            };
        }

        let failed_count = turn_outcomes
            .iter()
            .filter(|(_, o, _)| matches!(o, TurnOutcomeSummary::Failed))
            .count();
        let timed_out_count = turn_outcomes
            .iter()
            .filter(|(_, o, _)| matches!(o, TurnOutcomeSummary::TimedOut))
            .count();
        let escalated_count = turn_outcomes
            .iter()
            .filter(|(_, o, _)| matches!(o, TurnOutcomeSummary::Escalated))
            .count();
        let completed_count = turn_outcomes
            .iter()
            .filter(|(_, o, _)| matches!(o, TurnOutcomeSummary::Completed))
            .count();

        observations.push(format!(
            "Analyzed {total_turns} recent turns: {completed_count} completed, \
             {failed_count} failed, {timed_out_count} timed out, \
             {escalated_count} escalated."
        ));

        // Flag high failure/timeout rates.
        if total_turns >= 5 && failed_count * 2 > total_turns {
            let obs = format!(
                "High failure rate: {failed_count}/{total_turns} turns failed."
            );
            observations.push(obs.clone());
            memory_writes.push(ProposedWrite {
                topic: "reflection:failure-pattern".to_string(),
                content: format!(
                    "Reflection detected high failure rate ({failed_count}/{total_turns}). \
                     Consider investigating tool errors or adjusting approach."
                ),
            });
        }

        if total_turns >= 5 && timed_out_count * 2 > total_turns {
            let obs = format!(
                "High timeout rate: {timed_out_count}/{total_turns} turns timed out."
            );
            observations.push(obs.clone());
            memory_writes.push(ProposedWrite {
                topic: "reflection:timeout-pattern".to_string(),
                content: format!(
                    "Reflection detected high timeout rate ({timed_out_count}/{total_turns}). \
                     Consider reducing turn complexity or increasing timeout budget."
                ),
            });
        }

        // Flag repeated scope denials.
        if !scope_denials.is_empty() {
            let mut denial_counts: std::collections::HashMap<&str, usize> =
                std::collections::HashMap::new();
            for scope in &scope_denials {
                *denial_counts.entry(scope.as_str()).or_default() += 1;
            }
            let frequent: Vec<_> = denial_counts
                .iter()
                .filter(|(_, count)| **count >= 2)
                .collect();
            if !frequent.is_empty() {
                let scopes_str: Vec<String> = frequent
                    .iter()
                    .map(|(s, c)| format!("{s} ({c}x)"))
                    .collect();
                observations.push(format!(
                    "Repeated scope denials: {}",
                    scopes_str.join(", ")
                ));
                memory_writes.push(ProposedWrite {
                    topic: "reflection:scope-denials".to_string(),
                    content: format!(
                        "Reflection detected repeated scope denials: {}. \
                         These tools/scopes are being attempted but denied.",
                        scopes_str.join(", ")
                    ),
                });
            }
        }

        // If no actionable patterns, return without creating a mission.
        if memory_writes.is_empty() {
            return ToolOutcome::Completed {
                output: json!({
                    "proposal_id": null,
                    "observations": observations,
                    "memory_writes": []
                }),
                verified: Verification::NotApplicable,
            };
        }

        // Create a proposal and mission with gate.
        let proposal_id = format!("rp-{}", uuid::Uuid::new_v4().as_simple());
        let mission_id = format!("m-{}", uuid::Uuid::new_v4().as_simple());

        let proposal = ProposalRecord {
            proposal_id: proposal_id.clone(),
            observations: observations.clone(),
            memory_writes: memory_writes.clone(),
            prompt_append: None,
            allowlist_changes: None,
        };

        let proposal_json = match serde_json::to_string(&proposal) {
            Ok(j) => j,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("failed to serialize proposal: {e}"),
                });
            }
        };

        let role_name = self
            .role_name
            .get()
            .cloned()
            .unwrap_or_else(|| "default".to_string());

        let mut record = MissionRecord::new(
            mission_id.clone(),
            role_name,
            proposal_json,
        );

        // Transition to Running, then add a gate.
        if let Err(e) = mission::transition_to_running(&mut record) {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("failed to transition mission: {e}"),
            });
        }

        let gate_id = format!("g-{}", uuid::Uuid::new_v4().as_simple());
        let writes_summary: Vec<String> = memory_writes
            .iter()
            .map(|w| w.topic.clone())
            .collect();
        if let Err(e) = mission::add_gate(
            &mut record,
            gate_id,
            format!(
                "Reflection proposal {proposal_id}: approve writing to topics: {}",
                writes_summary.join(", ")
            ),
            Some("reflection.apply".to_string()),
        ) {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("failed to add gate: {e}"),
            });
        }

        if let Err(e) = mission::create_mission(store, &record).await {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("failed to persist mission: {e}"),
            });
        }

        let output_writes: Vec<Value> = memory_writes
            .iter()
            .map(|w| json!({ "topic": w.topic, "content": w.content }))
            .collect();

        ToolOutcome::Completed {
            output: json!({
                "proposal_id": proposal_id,
                "mission_id": mission_id,
                "observations": observations,
                "memory_writes": output_writes,
            }),
            verified: Verification::NotApplicable,
        }
    }
}

// ---------------------------------------------------------------------------
// reflection.apply
// ---------------------------------------------------------------------------

pub struct ReflectionApplyTool {
    id: ToolId,
    schema: Value,
    mission_store: OnceLock<DomainHandle>,
    memory: OnceLock<std::sync::Arc<dyn Memory>>,
    role_overrides: OnceLock<crate::role_overrides::SharedRoleOverrides>,
}

impl std::fmt::Debug for ReflectionApplyTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReflectionApplyTool")
            .field("id", &self.id)
            .finish()
    }
}

impl Default for ReflectionApplyTool {
    fn default() -> Self {
        Self::new()
    }
}

impl ReflectionApplyTool {
    pub fn new() -> Self {
        ReflectionApplyTool {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "proposal_id": {
                        "type": "string",
                        "description": "The proposal ID returned by reflection.propose"
                    }
                },
                "required": ["proposal_id"]
            }),
            mission_store: OnceLock::new(),
            memory: OnceLock::new(),
            role_overrides: OnceLock::new(),
        }
    }

    pub fn set_mission_store(&self, handle: DomainHandle) -> Result<(), DomainHandle> {
        assert_eq!(handle.domain(), KeyDomain::Missions);
        self.mission_store.set(handle)
    }

    pub fn set_memory(&self, mem: std::sync::Arc<dyn Memory>) -> Result<(), std::sync::Arc<dyn Memory>> {
        self.memory.set(mem)
    }

    pub fn set_role_overrides(
        &self,
        overrides: crate::role_overrides::SharedRoleOverrides,
    ) -> Result<(), crate::role_overrides::SharedRoleOverrides> {
        self.role_overrides.set(overrides)
    }
}

#[async_trait]
impl Tool for ReflectionApplyTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "reflection.apply"
    }

    fn description(&self) -> &str {
        "Apply an approved reflection proposal. Takes the proposal_id \
         from reflection.propose, verifies the associated mission gate \
         is approved, then executes the proposed memory writes. Fails \
         if the gate is still pending or was rejected."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("reflection.apply").expect("known base")
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let Some(store) = self.mission_store.get() else {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "reflection.apply: no mission store configured".to_string(),
            });
        };
        let Some(memory) = self.memory.get() else {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "reflection.apply: no memory configured".to_string(),
            });
        };

        let proposal_id = input
            .get("proposal_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if proposal_id.is_empty() {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "reflection.apply requires a non-empty `proposal_id`".to_string(),
            });
        }

        // Find the mission containing this proposal by scanning missions.
        let missions = match mission::list_missions(store).await {
            Ok(m) => m,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("failed to list missions: {e}"),
                });
            }
        };

        let target = missions.iter().find(|m| {
            serde_json::from_str::<ProposalRecord>(&m.description)
                .map(|p| p.proposal_id == proposal_id)
                .unwrap_or(false)
        });

        let Some(record) = target else {
            return ToolOutcome::Completed {
                output: json!({
                    "applied": false,
                    "reason": format!("no mission found for proposal {proposal_id}")
                }),
                verified: Verification::NotApplicable,
            };
        };

        // Check gate status — all gates must be approved.
        let has_pending = record.gates.iter().any(|g| g.state == GateState::Pending);
        let has_rejected = record
            .gates
            .iter()
            .any(|g| g.state == GateState::Rejected);

        if has_rejected {
            return ToolOutcome::Completed {
                output: json!({
                    "applied": false,
                    "reason": "proposal was rejected by the operator"
                }),
                verified: Verification::NotApplicable,
            };
        }

        if has_pending {
            return ToolOutcome::Completed {
                output: json!({
                    "applied": false,
                    "reason": "proposal gate is still pending operator approval"
                }),
                verified: Verification::NotApplicable,
            };
        }

        // Parse the proposal from the mission description.
        let proposal: ProposalRecord = match serde_json::from_str(&record.description) {
            Ok(p) => p,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("failed to parse proposal from mission: {e}"),
                });
            }
        };

        // Execute memory writes.
        let mut writes_executed = 0usize;
        for write in &proposal.memory_writes {
            match memory.put(&write.topic, &write.content).await {
                Ok(_) => writes_executed += 1,
                Err(e) => {
                    return ToolOutcome::Failed(AivyxError::Tool {
                        tool: self.id,
                        detail: format!(
                            "failed to write to memory topic '{}': {e}",
                            write.topic
                        ),
                    });
                }
            }
        }

        // Phase 30 — apply role overrides if present in the proposal.
        let mut role_updated = false;
        if proposal.prompt_append.is_some() || proposal.allowlist_changes.is_some() {
            if let Some(shared) = self.role_overrides.get() {
                if let Ok(mut w) = shared.write() {
                    if let Some(ref text) = proposal.prompt_append {
                        w.prompt_appendix = Some(text.clone());
                    }
                    if let Some(ref changes) = proposal.allowlist_changes {
                        for name in &changes.add {
                            if !w.allowlist_additions.contains(name) {
                                w.allowlist_additions.push(name.clone());
                            }
                        }
                        for name in &changes.remove {
                            w.allowlist_additions.retain(|n| n != name);
                            if !w.allowlist_removals.contains(name) {
                                w.allowlist_removals.push(name.clone());
                            }
                        }
                    }
                    role_updated = true;
                }
            }
        }

        // Complete the mission.
        let mut updated = record.clone();
        if updated.state == MissionState::Running {
            let _ = mission::complete_mission(&mut updated);
            let _ = mission::update_mission(store, &updated).await;
        }

        ToolOutcome::Completed {
            output: json!({
                "applied": true,
                "writes_executed": writes_executed,
                "role_updated": role_updated,
                "proposal_id": proposal_id,
            }),
            verified: Verification::Verified,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn propose_scope_is_reflection_propose() {
        let tool = ReflectionProposeTool::new();
        assert_eq!(
            tool.required_scope(&json!({})).as_str(),
            "reflection.propose"
        );
    }

    #[test]
    fn propose_name_and_schema() {
        let tool = ReflectionProposeTool::new();
        assert_eq!(tool.name(), "reflection.propose");
        let schema = tool.input_schema();
        assert!(schema["properties"]["lookback"].is_object());
    }

    #[test]
    fn apply_scope_is_reflection_apply() {
        let tool = ReflectionApplyTool::new();
        assert_eq!(
            tool.required_scope(&json!({})).as_str(),
            "reflection.apply"
        );
    }

    #[test]
    fn apply_name_and_schema() {
        let tool = ReflectionApplyTool::new();
        assert_eq!(tool.name(), "reflection.apply");
        let schema = tool.input_schema();
        assert!(schema["required"].as_array().unwrap().contains(&json!("proposal_id")));
    }
}
