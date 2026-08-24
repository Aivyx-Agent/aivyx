//! `RoleUpdateTool` — Phase 30 Runtime Role Mutation.
//!
//! Writes to the shared `RoleOverrides` handle. Every mutation
//! emits an `AuditTag::ToolCall` via the normal tool execution path
//! (no special audit wiring needed). Scoped to `role.update`.
//!
//! ## Input
//!
//! ```json
//! {
//!   "prompt_append": "When using shell.exec, prefer smaller commands.",
//!   "allowlist_add": ["tool_name"],
//!   "allowlist_remove": ["tool_name"]
//! }
//! ```
//!
//! All fields are optional. At least one must be provided.
//!
//! ## Output
//!
//! ```json
//! {
//!   "updated": true,
//!   "prompt_appendix_length": 47,
//!   "allowlist_additions": 1,
//!   "allowlist_removals": 0
//! }
//! ```

use std::sync::OnceLock;

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::role_overrides::SharedRoleOverrides;

pub struct RoleUpdateTool {
    id: ToolId,
    schema: Value,
    overrides: OnceLock<SharedRoleOverrides>,
}

impl std::fmt::Debug for RoleUpdateTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RoleUpdateTool")
            .field("id", &self.id)
            .field("has_overrides", &self.overrides.get().is_some())
            .finish()
    }
}

impl Default for RoleUpdateTool {
    fn default() -> Self {
        Self::new()
    }
}

impl RoleUpdateTool {
    pub fn new() -> Self {
        RoleUpdateTool {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "prompt_append": {
                        "type": "string",
                        "description": "Text to append to the system prompt. \
                            Replaces any previous appendix."
                    },
                    "allowlist_add": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Tool names to add to the active allowlist."
                    },
                    "allowlist_remove": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Tool names to remove from the active allowlist."
                    }
                }
            }),
            overrides: OnceLock::new(),
        }
    }

    pub fn set_overrides(
        &self,
        handle: SharedRoleOverrides,
    ) -> Result<(), SharedRoleOverrides> {
        self.overrides.set(handle)
    }
}

#[async_trait]
impl Tool for RoleUpdateTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "role.update"
    }

    fn description(&self) -> &str {
        "Modify the agent's runtime role configuration. Can append to \
         the system prompt and add/remove tools from the active allowlist. \
         Changes take effect on the next turn. The original system prompt \
         is always preserved — only an appendix is added."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    // role.update is never auto-granted -- self-escalation scopes stay
    // out of the default floor (P8 no-self-escalation).
    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("role.update").expect("known base")
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let Some(shared) = self.overrides.get() else {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "role.update: no overrides handle configured".to_string(),
            });
        };

        let prompt_append = input.get("prompt_append").and_then(|v| v.as_str());
        let allowlist_add: Vec<String> = input
            .get("allowlist_add")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let allowlist_remove: Vec<String> = input
            .get("allowlist_remove")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        if prompt_append.is_none() && allowlist_add.is_empty() && allowlist_remove.is_empty() {
            return ToolOutcome::Completed {
                output: json!({
                    "updated": false,
                    "reason": "no fields provided — at least one of prompt_append, \
                               allowlist_add, or allowlist_remove is required"
                }),
                verified: Verification::NotApplicable,
            };
        }

        let mut w = match shared.write() {
            Ok(w) => w,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("role.update: lock poisoned: {e}"),
                });
            }
        };

        let additions_count = allowlist_add.len();
        let removals_count = allowlist_remove.len();

        if let Some(text) = prompt_append {
            w.prompt_appendix = Some(text.to_string());
        }

        for name in allowlist_add {
            if !w.allowlist_additions.contains(&name) {
                w.allowlist_additions.push(name);
            }
        }

        for name in allowlist_remove {
            // Remove from additions if present (cancel out).
            w.allowlist_additions.retain(|n| n != &name);
            if !w.allowlist_removals.contains(&name) {
                w.allowlist_removals.push(name);
            }
        }

        let appendix_len = w
            .prompt_appendix
            .as_ref()
            .map(|s| s.len())
            .unwrap_or(0);

        ToolOutcome::Completed {
            output: json!({
                "updated": true,
                "prompt_appendix_length": appendix_len,
                "allowlist_additions": additions_count,
                "allowlist_removals": removals_count,
            }),
            verified: Verification::Verified,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_and_scope() {
        let tool = RoleUpdateTool::new();
        assert_eq!(tool.name(), "role.update");
        assert_eq!(
            tool.required_scope(&json!({})).as_str(),
            "role.update"
        );
    }

    #[test]
    fn schema_has_expected_properties() {
        let tool = RoleUpdateTool::new();
        let schema = tool.input_schema();
        let props = schema["properties"].as_object().unwrap();
        assert!(props.contains_key("prompt_append"));
        assert!(props.contains_key("allowlist_add"));
        assert!(props.contains_key("allowlist_remove"));
    }
}
