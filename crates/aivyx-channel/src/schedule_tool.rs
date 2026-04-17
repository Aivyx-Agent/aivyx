//! Schedule agent tools — Phase 26 Tasks 4–5.
//!
//! Four tools following the `OnceLock`-factory pattern from
//! `MissionCreateTool`:
//!
//! - `schedule.create` — create a new cron schedule
//! - `schedule.list` — list all schedules
//! - `schedule.delete` — delete a schedule by ID
//! - `schedule.update` — update fields on an existing schedule

use std::sync::OnceLock;

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};
use aivyx_storage::{DomainHandle, KeyDomain};

use crate::schedule::{self, ScheduleRecord};

// ---------------------------------------------------------------------------
// schedule.create
// ---------------------------------------------------------------------------

pub struct ScheduleCreateTool {
    id: ToolId,
    schema: Value,
    store: OnceLock<DomainHandle>,
}

impl std::fmt::Debug for ScheduleCreateTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScheduleCreateTool")
            .field("id", &self.id)
            .finish()
    }
}

impl Default for ScheduleCreateTool {
    fn default() -> Self {
        Self::new()
    }
}

impl ScheduleCreateTool {
    pub fn new() -> Self {
        ScheduleCreateTool {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "cron": {
                        "type": "string",
                        "description": "Cron expression (7-field: sec min hour dom month dow year). Example: \"0 0 9 * * * *\" for daily at 09:00 UTC."
                    },
                    "role": {
                        "type": "string",
                        "description": "Role name to run the scheduled turn under."
                    },
                    "prompt": {
                        "type": "string",
                        "description": "The prompt text submitted as a turn when the schedule fires."
                    }
                },
                "required": ["cron", "prompt"]
            }),
            store: OnceLock::new(),
        }
    }

    pub fn set_schedule_store(&self, handle: DomainHandle) -> Result<(), DomainHandle> {
        assert_eq!(handle.domain(), KeyDomain::Schedules);
        self.store.set(handle)
    }
}

#[async_trait]
impl Tool for ScheduleCreateTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "schedule.create"
    }

    fn description(&self) -> &str {
        "Create a new cron-triggered schedule. When the schedule fires, \
         the daemon submits the prompt as a turn under the specified role. \
         The cron expression uses 7 fields: sec min hour dom month dow year. \
         Returns the schedule_id for future reference."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("schedule.create").expect("known base")
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let Some(store) = self.store.get() else {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "schedule.create: no schedule store configured".to_string(),
            });
        };

        let cron = input
            .get("cron")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let prompt = input
            .get("prompt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let role = input
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .to_string();

        if cron.is_empty() {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "schedule.create requires a non-empty `cron` field".to_string(),
            });
        }
        if prompt.is_empty() {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "schedule.create requires a non-empty `prompt` field".to_string(),
            });
        }

        let schedule_id = format!("sched-{}", uuid::Uuid::new_v4().as_simple());
        let record = match ScheduleRecord::new(
            schedule_id.clone(),
            cron,
            role,
            prompt,
        ) {
            Ok(r) => r,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("schedule.create: {e}"),
                });
            }
        };

        if let Err(e) = schedule::create_schedule(store, &record).await {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("failed to persist schedule: {e}"),
            });
        }

        ToolOutcome::Completed {
            output: json!({ "schedule_id": schedule_id }),
            verified: Verification::NotApplicable,
        }
    }
}

// ---------------------------------------------------------------------------
// schedule.list
// ---------------------------------------------------------------------------

pub struct ScheduleListTool {
    id: ToolId,
    schema: Value,
    store: OnceLock<DomainHandle>,
}

impl std::fmt::Debug for ScheduleListTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScheduleListTool")
            .field("id", &self.id)
            .finish()
    }
}

impl Default for ScheduleListTool {
    fn default() -> Self {
        Self::new()
    }
}

impl ScheduleListTool {
    pub fn new() -> Self {
        ScheduleListTool {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
            store: OnceLock::new(),
        }
    }

    pub fn set_schedule_store(&self, handle: DomainHandle) -> Result<(), DomainHandle> {
        assert_eq!(handle.domain(), KeyDomain::Schedules);
        self.store.set(handle)
    }
}

#[async_trait]
impl Tool for ScheduleListTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "schedule.list"
    }

    fn description(&self) -> &str {
        "List all cron schedules. Returns an array of schedule objects \
         with id, cron expression, role, prompt, enabled status, and \
         next fire time."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("schedule.list").expect("known base")
    }

    async fn execute(&self, _input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let Some(store) = self.store.get() else {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "schedule.list: no schedule store configured".to_string(),
            });
        };

        match schedule::list_schedules(store).await {
            Ok(schedules) => {
                let entries: Vec<Value> = schedules
                    .iter()
                    .map(|s| {
                        let next = s.next_fire_time().map(|dt| dt.to_rfc3339());
                        json!({
                            "schedule_id": s.schedule_id,
                            "cron": s.cron_expr,
                            "role": s.role_name,
                            "prompt": s.prompt,
                            "enabled": s.enabled,
                            "next_fire": next,
                        })
                    })
                    .collect();
                ToolOutcome::Completed {
                    output: json!({ "schedules": entries }),
                    verified: Verification::NotApplicable,
                }
            }
            Err(e) => ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("failed to list schedules: {e}"),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// schedule.delete
// ---------------------------------------------------------------------------

pub struct ScheduleDeleteTool {
    id: ToolId,
    schema: Value,
    store: OnceLock<DomainHandle>,
}

impl std::fmt::Debug for ScheduleDeleteTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScheduleDeleteTool")
            .field("id", &self.id)
            .finish()
    }
}

impl Default for ScheduleDeleteTool {
    fn default() -> Self {
        Self::new()
    }
}

impl ScheduleDeleteTool {
    pub fn new() -> Self {
        ScheduleDeleteTool {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "schedule_id": {
                        "type": "string",
                        "description": "The ID of the schedule to delete."
                    }
                },
                "required": ["schedule_id"]
            }),
            store: OnceLock::new(),
        }
    }

    pub fn set_schedule_store(&self, handle: DomainHandle) -> Result<(), DomainHandle> {
        assert_eq!(handle.domain(), KeyDomain::Schedules);
        self.store.set(handle)
    }
}

#[async_trait]
impl Tool for ScheduleDeleteTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "schedule.delete"
    }

    fn description(&self) -> &str {
        "Delete a cron schedule by ID. The schedule will no longer fire. \
         Returns whether the schedule was found and deleted."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("schedule.delete").expect("known base")
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let Some(store) = self.store.get() else {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "schedule.delete: no schedule store configured".to_string(),
            });
        };

        let schedule_id = input
            .get("schedule_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if schedule_id.is_empty() {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "schedule.delete requires a non-empty `schedule_id` field".to_string(),
            });
        }

        let exists = match schedule::get_schedule(store, &schedule_id).await {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("failed to check schedule: {e}"),
                });
            }
        };

        if !exists {
            return ToolOutcome::Completed {
                output: json!({
                    "deleted": false,
                    "reason": format!("schedule {schedule_id} not found")
                }),
                verified: Verification::NotApplicable,
            };
        }

        if let Err(e) = schedule::delete_schedule(store, &schedule_id).await {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("failed to delete schedule: {e}"),
            });
        }

        ToolOutcome::Completed {
            output: json!({ "deleted": true, "schedule_id": schedule_id }),
            verified: Verification::NotApplicable,
        }
    }
}

// ---------------------------------------------------------------------------
// schedule.update
// ---------------------------------------------------------------------------

pub struct ScheduleUpdateTool {
    id: ToolId,
    schema: Value,
    store: OnceLock<DomainHandle>,
}

impl std::fmt::Debug for ScheduleUpdateTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScheduleUpdateTool")
            .field("id", &self.id)
            .finish()
    }
}

impl Default for ScheduleUpdateTool {
    fn default() -> Self {
        Self::new()
    }
}

impl ScheduleUpdateTool {
    pub fn new() -> Self {
        ScheduleUpdateTool {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "schedule_id": {
                        "type": "string",
                        "description": "The ID of the schedule to update."
                    },
                    "enabled": {
                        "type": "boolean",
                        "description": "Set to true to enable or false to disable the schedule."
                    },
                    "cron": {
                        "type": "string",
                        "description": "New cron expression (7-field: sec min hour dom month dow year)."
                    },
                    "prompt": {
                        "type": "string",
                        "description": "New prompt text for the scheduled turn."
                    },
                    "role": {
                        "type": "string",
                        "description": "New role name for the scheduled turn."
                    }
                },
                "required": ["schedule_id"]
            }),
            store: OnceLock::new(),
        }
    }

    pub fn set_schedule_store(&self, handle: DomainHandle) -> Result<(), DomainHandle> {
        assert_eq!(handle.domain(), KeyDomain::Schedules);
        self.store.set(handle)
    }
}

#[async_trait]
impl Tool for ScheduleUpdateTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "schedule.update"
    }

    fn description(&self) -> &str {
        "Update an existing schedule. Provide the schedule_id and any \
         fields to change: enabled (true/false), cron expression, prompt, \
         or role. Unspecified fields are left unchanged. Returns the \
         updated schedule."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("schedule.update").expect("known base")
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let Some(store) = self.store.get() else {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "schedule.update: no schedule store configured".to_string(),
            });
        };

        let schedule_id = input
            .get("schedule_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if schedule_id.is_empty() {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "schedule.update requires a non-empty `schedule_id` field".to_string(),
            });
        }

        let mut record = match schedule::get_schedule(store, &schedule_id).await {
            Ok(Some(r)) => r,
            Ok(None) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("schedule {schedule_id} not found"),
                });
            }
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("failed to read schedule: {e}"),
                });
            }
        };

        if let Some(enabled) = input.get("enabled").and_then(|v| v.as_bool()) {
            record.enabled = enabled;
        }

        if let Some(cron) = input.get("cron").and_then(|v| v.as_str()) {
            if let Err(e) = schedule::validate_cron(cron) {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("schedule.update: {e}"),
                });
            }
            record.cron_expr = cron.to_string();
        }

        if let Some(prompt) = input.get("prompt").and_then(|v| v.as_str()) {
            if prompt.is_empty() {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: "schedule.update: prompt cannot be empty".to_string(),
                });
            }
            record.prompt = prompt.to_string();
        }

        if let Some(role) = input.get("role").and_then(|v| v.as_str()) {
            record.role_name = role.to_string();
        }

        if let Err(e) = schedule::update_schedule(store, &record).await {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("failed to persist schedule update: {e}"),
            });
        }

        let next = record.next_fire_time().map(|dt| dt.to_rfc3339());
        ToolOutcome::Completed {
            output: json!({
                "schedule_id": record.schedule_id,
                "cron": record.cron_expr,
                "role": record.role_name,
                "prompt": record.prompt,
                "enabled": record.enabled,
                "next_fire": next,
            }),
            verified: Verification::NotApplicable,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_create_scope() {
        let tool = ScheduleCreateTool::new();
        assert_eq!(
            tool.required_scope(&json!({})).as_str(),
            "schedule.create"
        );
    }

    #[test]
    fn schedule_create_name_and_schema() {
        let tool = ScheduleCreateTool::new();
        assert_eq!(tool.name(), "schedule.create");
        let schema = tool.input_schema();
        assert!(schema["required"].as_array().unwrap().contains(&json!("cron")));
        assert!(schema["required"].as_array().unwrap().contains(&json!("prompt")));
    }

    #[test]
    fn schedule_list_scope() {
        let tool = ScheduleListTool::new();
        assert_eq!(
            tool.required_scope(&json!({})).as_str(),
            "schedule.list"
        );
    }

    #[test]
    fn schedule_list_name() {
        let tool = ScheduleListTool::new();
        assert_eq!(tool.name(), "schedule.list");
    }

    #[test]
    fn schedule_delete_scope() {
        let tool = ScheduleDeleteTool::new();
        assert_eq!(
            tool.required_scope(&json!({})).as_str(),
            "schedule.delete"
        );
    }

    #[test]
    fn schedule_delete_name_and_schema() {
        let tool = ScheduleDeleteTool::new();
        assert_eq!(tool.name(), "schedule.delete");
        let schema = tool.input_schema();
        assert!(schema["required"].as_array().unwrap().contains(&json!("schedule_id")));
    }

    #[test]
    fn schedule_update_scope() {
        let tool = ScheduleUpdateTool::new();
        assert_eq!(
            tool.required_scope(&json!({})).as_str(),
            "schedule.update"
        );
    }

    #[test]
    fn schedule_update_name_and_schema() {
        let tool = ScheduleUpdateTool::new();
        assert_eq!(tool.name(), "schedule.update");
        let schema = tool.input_schema();
        assert!(schema["required"].as_array().unwrap().contains(&json!("schedule_id")));
        assert!(schema["properties"]["enabled"].is_object());
        assert!(schema["properties"]["cron"].is_object());
        assert!(schema["properties"]["prompt"].is_object());
        assert!(schema["properties"]["role"].is_object());
    }
}
