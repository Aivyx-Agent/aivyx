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
use crate::persona::{
    synthesize_delta_id, PersistentPersonaLog, PersonaDelta, ProposedPersonaDelta,
    SharedEffectivePersona,
};

// ---------------------------------------------------------------------------
// Proposal record — serialized into the mission description
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProposalRecord {
    pub(crate) proposal_id: String,
    pub(crate) observations: Vec<String>,
    pub(crate) memory_writes: Vec<ProposedWrite>,
    /// Phase 30 — text to append to the system prompt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) prompt_append: Option<String>,
    /// Phase 30 — tool allowlist additions and removals.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) allowlist_changes: Option<AllowlistChanges>,
    /// Phase 59 — Persona deltas the agent proposes for operator
    /// approval. Each is a single field-edit per Q1(a) on Phase 59
    /// sign-off; on gate approval, `reflection.apply` writes them to
    /// the Persona chain via `PersistentPersonaLog::append`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) persona_deltas: Vec<ProposedPersonaDelta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ProposedWrite {
    pub(crate) topic: String,
    pub(crate) content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AllowlistChanges {
    #[serde(default)]
    pub(crate) add: Vec<String>,
    #[serde(default)]
    pub(crate) remove: Vec<String>,
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
    /// Phase 70 — when set, every agent-supplied `ProposedPersonaDelta`
    /// is also appended to the persistent proposal chain as a
    /// `Pending` row. The mission/gate flow stays in place for
    /// memory writes (which still flow through reflection.apply);
    /// persona deltas land in both surfaces so the operator can
    /// review them via the new Web UI Proposals pane / `aivyx
    /// persona proposals` CLI rather than waiting on a mission
    /// gate.
    persona_proposal_log:
        OnceLock<std::sync::Arc<crate::persona_proposal::PersistentPersonaProposalLog>>,
}

impl std::fmt::Debug for ReflectionProposeTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReflectionProposeTool")
            .field("id", &self.id)
            .field("has_audit_log", &self.audit_log.get().is_some())
            .field(
                "has_proposal_log",
                &self.persona_proposal_log.get().is_some(),
            )
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
                    },
                    "persona_deltas": {
                        "type": "array",
                        "description": "Phase 59 — operator-approval-bound persona deltas. \
                                        Each item is a single field-edit on the assistant's identity \
                                        layer (PRODUCT.md P14). Requires the persona.propose capability \
                                        scope. The gate approval flow surfaces deltas to the operator \
                                        for explicit consent before reflection.apply writes them.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "category": {
                                    "type": "string",
                                    "enum": [
                                        "AssistantName",
                                        "OperatorProfile",
                                        "CommunicationStyle",
                                        "PrimaryUseCases",
                                        "BehavioralPreferences",
                                        "BehavioralConstraints",
                                        "LearnedContext",
                                        "CommunicationAdaptations",
                                        "CharacterTraits",
                                        "RelationshipMilestones"
                                    ]
                                },
                                "op": {
                                    "type": "object",
                                    "description": "One of: \
                                        { kind: 'SetScalar', value: <string|null> } for scalar categories \
                                        (AssistantName / OperatorProfile / CommunicationStyle), \
                                        { kind: 'AppendList', value: <string> } / \
                                        { kind: 'RemoveList', value: <string> } for list categories."
                                },
                                "reason": {
                                    "type": "string",
                                    "description": "Short rationale shown to the operator at the gate."
                                }
                            },
                            "required": ["category", "op"]
                        }
                    }
                },
                "required": []
            }),
            audit_log: OnceLock::new(),
            mission_store: OnceLock::new(),
            role_name: OnceLock::new(),
            persona_proposal_log: OnceLock::new(),
        }
    }

    /// Phase 70 — register the persistent proposal chain. When set,
    /// agent-supplied persona deltas are written to the chain as
    /// `Pending` rows in addition to the mission gate the existing
    /// flow creates. Returns the original Arc back if a setter
    /// collides (programming error — should only be set once at
    /// startup).
    pub fn set_persona_proposal_log(
        &self,
        log: std::sync::Arc<crate::persona_proposal::PersistentPersonaProposalLog>,
    ) -> Result<
        (),
        std::sync::Arc<crate::persona_proposal::PersistentPersonaProposalLog>,
    > {
        self.persona_proposal_log.set(log)
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

    fn required_scope(&self, input: &Value) -> Scope {
        // Phase 59 — when the call carries `persona_deltas`, the
        // stricter `persona.propose` scope is required (the agent
        // is extending the identity layer, not just memory). Roles
        // that have reflection.propose but not persona.propose can
        // still call this tool without deltas; the per-tool gate
        // rejects only the persona-bearing variant.
        let has_persona = input
            .get("persona_deltas")
            .and_then(|v| v.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false);
        if has_persona {
            Scope::parse("persona.propose").expect("known base")
        } else {
            Scope::parse("reflection.propose").expect("known base")
        }
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

        // Phase 59 — parse and validate the agent-supplied persona
        // deltas. Each delta must pass `ProposedPersonaDelta::validate`
        // (category/op pairing); a single bad delta fails the whole
        // call to avoid landing a partial proposal in the mission
        // record.
        let persona_deltas: Vec<ProposedPersonaDelta> = match input.get("persona_deltas") {
            Some(v) if !v.is_null() => match serde_json::from_value::<Vec<ProposedPersonaDelta>>(v.clone()) {
                Ok(list) => {
                    for (idx, d) in list.iter().enumerate() {
                        if let Err(reason) = d.validate() {
                            return ToolOutcome::Failed(AivyxError::Tool {
                                tool: self.id,
                                detail: format!(
                                    "reflection.propose: persona_deltas[{idx}] invalid: {reason}"
                                ),
                            });
                        }
                    }
                    list
                }
                Err(e) => {
                    return ToolOutcome::Failed(AivyxError::Tool {
                        tool: self.id,
                        detail: format!(
                            "reflection.propose: persona_deltas deserialization failed: {e}"
                        ),
                    });
                }
            },
            _ => Vec::new(),
        };

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
            // Phase 59 — even on "no recent turns", a non-empty
            // persona_deltas input still produces a mission. The
            // agent's reasoning may have come from inputs outside
            // the audit chain (operator transcript review, memory
            // walk, etc.) — we trust the agent's proposal and let
            // the operator gate it.
            if persona_deltas.is_empty() {
                return ToolOutcome::Completed {
                    output: json!({
                        "proposal_id": null,
                        "observations": ["No recent turns found — nothing to reflect on."],
                        "memory_writes": [],
                        "persona_deltas": [],
                    }),
                    verified: Verification::NotApplicable,
                };
            }
            observations.push(
                "No recent turn patterns to reflect on, but agent has \
                 supplied persona deltas for operator approval."
                    .to_string(),
            );
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

        // If no actionable patterns AND no agent-supplied persona
        // deltas, return without creating a mission. Phase 59 adds
        // the persona_deltas leg to the short-circuit condition.
        if memory_writes.is_empty() && persona_deltas.is_empty() {
            return ToolOutcome::Completed {
                output: json!({
                    "proposal_id": null,
                    "observations": observations,
                    "memory_writes": [],
                    "persona_deltas": [],
                }),
                verified: Verification::NotApplicable,
            };
        }

        // Create a proposal and mission with gate.
        let proposal_id = format!("rp-{}", uuid::Uuid::new_v4().as_simple());
        let mission_id = format!("m-{}", uuid::Uuid::new_v4().as_simple());

        // Phase 70 — also write each persona delta to the
        // persistent proposal chain as a `Pending` row so the
        // operator can review via the Web UI Proposals pane /
        // `aivyx persona proposals` CLI without waiting on the
        // mission gate. Best-effort: failures emit a tracing
        // diagnostic but don't fail the whole tool call, since
        // the mission/gate path still provides the legacy
        // approval surface.
        if let Some(proposal_log) = self.persona_proposal_log.get() {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let session_id = _ctx.session_id.to_string();
            for (idx, delta) in persona_deltas.iter().enumerate() {
                let pid = format!("pp-{}-{idx}", &proposal_id[3..]);
                if let Err(e) = proposal_log
                    .append_pending(
                        pid,
                        now_ms,
                        session_id.clone(),
                        delta.clone(),
                    )
                    .await
                {
                    eprintln!(
                        "reflection.propose: proposal log append failed: {e}"
                    );
                }
            }
        }

        let proposal = ProposalRecord {
            proposal_id: proposal_id.clone(),
            observations: observations.clone(),
            memory_writes: memory_writes.clone(),
            prompt_append: None,
            allowlist_changes: None,
            persona_deltas: persona_deltas.clone(),
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
        let mut gate_description = format!("Reflection proposal {proposal_id}: ");
        let mut parts: Vec<String> = Vec::new();
        if !writes_summary.is_empty() {
            parts.push(format!(
                "memory writes to topics [{}]",
                writes_summary.join(", ")
            ));
        }
        if !persona_deltas.is_empty() {
            // Phase 59 — surface the persona delta count in the gate
            // prompt so the operator sees what they're approving. The
            // full per-delta detail is in the mission record's
            // serialized ProposalRecord; the gate prompt is a
            // glance-summary.
            parts.push(format!(
                "{} persona delta(s) ({})",
                persona_deltas.len(),
                persona_deltas
                    .iter()
                    .map(|d| format!("{:?}", d.category))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        gate_description.push_str(&format!("approve {}", parts.join(" + ")));
        if let Err(e) = mission::add_gate(
            &mut record,
            gate_id,
            gate_description,
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
        let output_persona_deltas: Vec<Value> = persona_deltas
            .iter()
            .map(|d| {
                json!({
                    "category": d.category,
                    "op": d.op,
                    "reason": d.reason,
                })
            })
            .collect();

        ToolOutcome::Completed {
            output: json!({
                "proposal_id": proposal_id,
                "mission_id": mission_id,
                "observations": observations,
                "memory_writes": output_writes,
                "persona_deltas": output_persona_deltas,
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
    /// Phase 59 — persistent Persona chain. When configured, approved
    /// `persona_deltas` from the proposal are appended here on apply.
    /// `None` means the daemon was launched without Persona support
    /// (e.g. tests, in-process minimal paths) — persona deltas in a
    /// proposal are surfaced in the output but not written, leaving
    /// the operator a recoverable state.
    persona_log: OnceLock<std::sync::Arc<PersistentPersonaLog>>,
    /// Phase 59 — shared runtime state mutated under the write lock
    /// on apply. Planner factory reads from a clone of this `Arc`
    /// per-turn per Q5(a). `None` falls back to "write the chain
    /// but skip the runtime mutation" (operator restart picks up
    /// the new state).
    effective_persona: OnceLock<SharedEffectivePersona>,
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
            persona_log: OnceLock::new(),
            effective_persona: OnceLock::new(),
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

    /// Phase 59 — register the persistent Persona chain. When set,
    /// approved persona_deltas from a proposal are appended on apply.
    pub fn set_persona_log(
        &self,
        log: std::sync::Arc<PersistentPersonaLog>,
    ) -> Result<(), std::sync::Arc<PersistentPersonaLog>> {
        self.persona_log.set(log)
    }

    /// Phase 59 — register the shared runtime state. When set,
    /// approved persona deltas mutate the in-memory state alongside
    /// the chain append so the next turn's planner factory sees them.
    pub fn set_effective_persona(
        &self,
        shared: SharedEffectivePersona,
    ) -> Result<(), SharedEffectivePersona> {
        self.effective_persona.set(shared)
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

        // Phase 59 — apply approved persona deltas. The chain
        // append is authoritative; the shared runtime state update
        // is best-effort (a poisoned lock surfaces in the output
        // but does not block the chain commit). Each delta lands
        // at consecutive chain seqs; partial batches are allowed
        // so the caller can retry by re-applying the proposal.
        let mut persona_deltas_committed = 0usize;
        let mut persona_chain_error: Option<String> = None;
        let mut effective_persona_synced = true;
        if !proposal.persona_deltas.is_empty() {
            if let Some(log) = self.persona_log.get() {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                for (idx, proposed) in proposal.persona_deltas.iter().enumerate() {
                    let delta = PersonaDelta {
                        delta_id: synthesize_delta_id(
                            &proposal_id,
                            proposed.category,
                            &proposed.op,
                            idx as u32,
                        ),
                        proposed_at_unix_ms: now_ms,
                        approved_at_unix_ms: now_ms,
                        proposal_id: proposal_id.clone(),
                        category: proposed.category,
                        op: proposed.op.clone(),
                    };
                    match log.append(delta.clone()).await {
                        Ok(_seq) => {
                            persona_deltas_committed += 1;
                            // Phase 60: recompute the runtime state
                            // from the full chain. Per-delta apply is
                            // no longer sufficient since Revert ops
                            // need chain context to find their target.
                            // The recompute is cheap (chain is small)
                            // and gives Revert semantics for free.
                            if let Some(shared) = self.effective_persona.get() {
                                let entries = log.entries();
                                if !crate::persona::recompute_shared_from_entries(
                                    shared,
                                    &entries,
                                ) {
                                    effective_persona_synced = false;
                                }
                            }
                        }
                        Err(e) => {
                            persona_chain_error =
                                Some(format!("persona delta {idx} append failed: {e}"));
                            break;
                        }
                    }
                }
            } else {
                persona_chain_error = Some(
                    "persona deltas present in proposal but no persona log configured \
                     (in-process or test deployment); deltas were not written"
                        .to_string(),
                );
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
                "persona_deltas_committed": persona_deltas_committed,
                "persona_deltas_total": proposal.persona_deltas.len(),
                "persona_chain_error": persona_chain_error,
                "effective_persona_synced": effective_persona_synced,
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

    // -- Phase 59 Task 3 — persona_deltas integration tests --------

    #[test]
    fn propose_schema_advertises_persona_deltas_field() {
        let tool = ReflectionProposeTool::new();
        let schema = tool.input_schema();
        let persona_deltas = &schema["properties"]["persona_deltas"];
        assert!(persona_deltas.is_object());
        assert_eq!(persona_deltas["type"], "array");

        // The enum constraint advertises every Phase 59 category so
        // the model picks valid categories without trial-and-error.
        let categories = &persona_deltas["items"]["properties"]["category"]["enum"];
        let cats: Vec<&str> = categories
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(cats.contains(&"AssistantName"));
        assert!(cats.contains(&"BehavioralPreferences"));
        assert!(cats.contains(&"LearnedContext"));
        assert!(cats.contains(&"CommunicationAdaptations"));
        assert_eq!(cats.len(), 10);
    }

    #[test]
    fn propose_required_scope_escalates_to_persona_propose_when_deltas_present() {
        let tool = ReflectionProposeTool::new();

        // Empty deltas → reflection.propose suffices.
        let with_empty = json!({ "persona_deltas": [] });
        assert_eq!(
            tool.required_scope(&with_empty).as_str(),
            "reflection.propose"
        );

        // Absent field → reflection.propose suffices.
        let absent = json!({ "lookback": 10 });
        assert_eq!(
            tool.required_scope(&absent).as_str(),
            "reflection.propose"
        );

        // Non-empty deltas → persona.propose required.
        let with_deltas = json!({
            "persona_deltas": [{
                "category": "BehavioralPreferences",
                "op": { "kind": "AppendList", "value": "prefer terse" }
            }]
        });
        assert_eq!(
            tool.required_scope(&with_deltas).as_str(),
            "persona.propose"
        );
    }

    #[test]
    fn proposal_record_serializes_persona_deltas_through_mission_description() {
        // The ProposalRecord serde round-trip is what bridges
        // reflection.propose (writer) and reflection.apply (reader).
        // Task 4 will consume the serialized representation; Task 3
        // verifies the byte-level contract.
        let record = ProposalRecord {
            proposal_id: "rp-test".into(),
            observations: vec!["test obs".into()],
            memory_writes: vec![ProposedWrite {
                topic: "t".into(),
                content: "c".into(),
            }],
            prompt_append: None,
            allowlist_changes: None,
            persona_deltas: vec![ProposedPersonaDelta {
                category: crate::persona::PersonaDeltaCategory::BehavioralPreferences,
                op: crate::persona::PersonaDeltaOp::AppendList {
                    value: "prefer terse".into(),
                },
                reason: Some("operator revised 3 verbose answers".into()),
            }],
        };
        let json = serde_json::to_string(&record).unwrap();
        let parsed: ProposalRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.persona_deltas.len(), 1);
        assert_eq!(
            parsed.persona_deltas[0].category,
            crate::persona::PersonaDeltaCategory::BehavioralPreferences,
        );
    }

    #[test]
    fn proposal_record_persona_deltas_field_omitted_when_empty() {
        // Backwards compat: pre-Phase-59 proposal records had no
        // `persona_deltas` field. The `skip_serializing_if =
        // "Vec::is_empty"` attribute keeps the serialized output free
        // of an empty array for missions that don't touch persona.
        let record = ProposalRecord {
            proposal_id: "rp-test".into(),
            observations: vec![],
            memory_writes: vec![],
            prompt_append: None,
            allowlist_changes: None,
            persona_deltas: vec![],
        };
        let json = serde_json::to_string(&record).unwrap();
        assert!(!json.contains("persona_deltas"));

        // ...and the parser tolerates the missing field on the
        // read path.
        let parsed: ProposalRecord = serde_json::from_str(&json).unwrap();
        assert!(parsed.persona_deltas.is_empty());
    }
}
