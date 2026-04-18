//! `MissionCreateTool` — Phase 21 mission creation primitive
//! (**PRODUCT.md P2**).
//!
//! Follows the same `OnceLock`-factory pattern as `RoleSwitchTool`
//! (Phase 14): the tool is registered in the `ToolRegistry` with
//! an empty storage slot, and the binary's startup path fills the
//! slot via `set_mission_store` after `RedbStorage::open`.
//!
//! ## Input shape
//!
//! ```json
//! { "description": "Track CI for regressions over the next week" }
//! ```
//!
//! - `description`: human-readable mission intent. Stored in the
//!   `MissionRecord` and surfaced in `daemon status` / mission
//!   listing.
//!
//! ## Output
//!
//! On success, returns the `mission_id` in a JSON object:
//! ```json
//! { "mission_id": "m-a1b2c3d4" }
//! ```

use std::sync::OnceLock;

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};
use aivyx_storage::{DomainHandle, KeyDomain};

use crate::mission::{self, MissionRecord};

pub struct MissionCreateTool {
    id: ToolId,
    schema: Value,
    store: OnceLock<DomainHandle>,
    role_name: OnceLock<String>,
}

impl std::fmt::Debug for MissionCreateTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MissionCreateTool")
            .field("id", &self.id)
            .field("has_store", &self.store.get().is_some())
            .finish()
    }
}

impl Default for MissionCreateTool {
    fn default() -> Self {
        Self::new()
    }
}

impl MissionCreateTool {
    pub fn new() -> Self {
        MissionCreateTool {
            id: ToolId::new(),
            schema: mission_create_input_schema(),
            store: OnceLock::new(),
            role_name: OnceLock::new(),
        }
    }

    pub fn set_mission_store(&self, handle: DomainHandle) -> Result<(), DomainHandle> {
        assert_eq!(handle.domain(), KeyDomain::Missions);
        self.store.set(handle)
    }

    pub fn set_role_name(&self, name: String) -> Result<(), String> {
        self.role_name.set(name)
    }
}

#[async_trait]
impl Tool for MissionCreateTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "mission.create"
    }

    fn description(&self) -> &str {
        "Create a new long-running mission. Input is a JSON object with a \
         `description` field describing the mission's intent. The mission \
         is persisted to storage and survives daemon restarts. Returns \
         the mission_id for future reference. The mission starts in \
         Created state and must be advanced to Running by the daemon."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("mission.create").expect("known base")
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let Some(store) = self.store.get() else {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "mission.create invoked without a mission store; \
                         the session layer must call \
                         MissionCreateTool::set_mission_store(...) after \
                         opening storage"
                    .to_string(),
            });
        };

        let description = input
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if description.is_empty() {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "mission.create requires a non-empty `description` field"
                    .to_string(),
            });
        }

        let role_name = self
            .role_name
            .get()
            .cloned()
            .unwrap_or_else(|| "default".to_string());

        let mission_id = format!("m-{}", uuid::Uuid::new_v4().as_simple());
        let record = MissionRecord::new(
            mission_id.clone(),
            role_name,
            description,
        );

        if let Err(e) = mission::create_mission(store, &record).await {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("failed to persist mission: {e}"),
            });
        }

        ToolOutcome::Completed {
            output: json!({ "mission_id": mission_id }),
            verified: Verification::NotApplicable,
        }
    }
}

fn mission_create_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "description": {
                "type": "string",
                "description": "Human-readable description of the mission's intent"
            }
        },
        "required": ["description"]
    })
}

// ---------------------------------------------------------------------------
// mission.list
// ---------------------------------------------------------------------------

pub struct MissionListTool {
    id: ToolId,
    schema: Value,
    store: OnceLock<DomainHandle>,
}

impl std::fmt::Debug for MissionListTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MissionListTool")
            .field("id", &self.id)
            .finish()
    }
}

impl Default for MissionListTool {
    fn default() -> Self {
        Self::new()
    }
}

impl MissionListTool {
    pub fn new() -> Self {
        MissionListTool {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
            store: OnceLock::new(),
        }
    }

    pub fn set_mission_store(&self, handle: DomainHandle) -> Result<(), DomainHandle> {
        assert_eq!(handle.domain(), KeyDomain::Missions);
        self.store.set(handle)
    }
}

#[async_trait]
impl Tool for MissionListTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "mission.list"
    }

    fn description(&self) -> &str {
        "List all missions. Returns an array of mission objects with \
         id, role, description, state, gate count, and timestamps."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("mission.list").expect("known base")
    }

    async fn execute(&self, _input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let Some(store) = self.store.get() else {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "mission.list: no mission store configured".to_string(),
            });
        };

        match mission::list_missions(store).await {
            Ok(missions) => {
                let entries: Vec<Value> = missions
                    .iter()
                    .map(|m| {
                        json!({
                            "mission_id": m.mission_id,
                            "role": m.role_name,
                            "description": m.description,
                            "state": format!("{:?}", m.state),
                            "gates": m.gates.len(),
                            "created_at": m.created_at,
                            "updated_at": m.updated_at,
                        })
                    })
                    .collect();
                ToolOutcome::Completed {
                    output: json!({ "missions": entries }),
                    verified: Verification::NotApplicable,
                }
            }
            Err(e) => ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("failed to list missions: {e}"),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// mission.status
// ---------------------------------------------------------------------------

pub struct MissionStatusTool {
    id: ToolId,
    schema: Value,
    store: OnceLock<DomainHandle>,
}

impl std::fmt::Debug for MissionStatusTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MissionStatusTool")
            .field("id", &self.id)
            .finish()
    }
}

impl Default for MissionStatusTool {
    fn default() -> Self {
        Self::new()
    }
}

impl MissionStatusTool {
    pub fn new() -> Self {
        MissionStatusTool {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "mission_id": {
                        "type": "string",
                        "description": "The ID of the mission to inspect."
                    }
                },
                "required": ["mission_id"]
            }),
            store: OnceLock::new(),
        }
    }

    pub fn set_mission_store(&self, handle: DomainHandle) -> Result<(), DomainHandle> {
        assert_eq!(handle.domain(), KeyDomain::Missions);
        self.store.set(handle)
    }
}

#[async_trait]
impl Tool for MissionStatusTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "mission.status"
    }

    fn description(&self) -> &str {
        "Get the full status of a mission by ID. Returns the mission's \
         state, description, role, all gates with their resolution status, \
         and timestamps."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("mission.status").expect("known base")
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let Some(store) = self.store.get() else {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "mission.status: no mission store configured".to_string(),
            });
        };

        let mission_id = input
            .get("mission_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if mission_id.is_empty() {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "mission.status requires a non-empty `mission_id` field".to_string(),
            });
        }

        match mission::get_mission(store, &mission_id).await {
            Ok(Some(m)) => {
                let gates: Vec<Value> = m
                    .gates
                    .iter()
                    .map(|g| {
                        json!({
                            "gate_id": g.gate_id,
                            "reason": g.reason,
                            "scope": g.scope,
                            "state": format!("{:?}", g.state),
                            "created_at": g.created_at,
                            "resolved_at": g.resolved_at,
                        })
                    })
                    .collect();
                ToolOutcome::Completed {
                    output: json!({
                        "mission_id": m.mission_id,
                        "role": m.role_name,
                        "description": m.description,
                        "state": format!("{:?}", m.state),
                        "gates": gates,
                        "created_at": m.created_at,
                        "updated_at": m.updated_at,
                    }),
                    verified: Verification::NotApplicable,
                }
            }
            Ok(None) => ToolOutcome::Completed {
                output: json!({
                    "found": false,
                    "reason": format!("mission {mission_id} not found")
                }),
                verified: Verification::NotApplicable,
            },
            Err(e) => ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("failed to get mission: {e}"),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_scope_is_mission_create() {
        let tool = MissionCreateTool::new();
        let scope = tool.required_scope(&json!({"description": "test"}));
        assert_eq!(scope.as_str(), "mission.create");
    }

    #[test]
    fn name_and_schema() {
        let tool = MissionCreateTool::new();
        assert_eq!(tool.name(), "mission.create");
        let schema = tool.input_schema();
        assert_eq!(schema["required"][0], "description");
    }

    #[test]
    fn mission_list_scope() {
        let tool = MissionListTool::new();
        assert_eq!(
            tool.required_scope(&json!({})).as_str(),
            "mission.list"
        );
    }

    #[test]
    fn mission_list_name() {
        let tool = MissionListTool::new();
        assert_eq!(tool.name(), "mission.list");
    }

    #[test]
    fn mission_status_scope() {
        let tool = MissionStatusTool::new();
        assert_eq!(
            tool.required_scope(&json!({})).as_str(),
            "mission.status"
        );
    }

    #[test]
    fn mission_status_name_and_schema() {
        let tool = MissionStatusTool::new();
        assert_eq!(tool.name(), "mission.status");
        let schema = tool.input_schema();
        assert!(schema["required"].as_array().unwrap().contains(&json!("mission_id")));
    }
}
