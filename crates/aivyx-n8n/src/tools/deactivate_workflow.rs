//! `n8n.deactivate_workflow` — mark a workflow inactive.
//!
//! Phase 131 Task 9. Third write tool. Trusted-gated.
//!
//! ## API call
//!
//! `POST /api/v1/workflows/{id}/deactivate`. Mirror of
//! `n8n.activate_workflow` — same idempotent shape,
//! same was-already-X flag, same NotFound surface.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::n8n_client::{N8nClientError, SharedN8nClient};
use crate::tools::activate_workflow::parse_id;

pub struct N8nDeactivateWorkflow {
    id: ToolId,
    schema: Value,
    client: SharedN8nClient,
}

impl N8nDeactivateWorkflow {
    pub fn new(client: SharedN8nClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for N8nDeactivateWorkflow {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "n8n.deactivate_workflow"
    }

    fn description(&self) -> &str {
        "Mark an n8n workflow inactive. Input is a JSON \
         object with a required `id` (workflow id from \
         `n8n.list_workflows`). Inactive workflows stop \
         firing their triggers immediately on the n8n \
         instance — useful for pausing automation \
         without deleting the definition. Idempotent: \
         deactivating an already-inactive workflow \
         succeeds. Returns `{id, active: false, \
         was_already_inactive}`. Requires Trusted-tier \
         capability grant for `n8n.write`."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("n8n.write").expect(
            "n8n.write must parse — it is in KNOWN_BASES from Phase 131",
        )
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let workflow_id = match parse_id(&input) {
            Ok(s) => s,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("n8n.deactivate_workflow: {reason}"),
                });
            }
        };
        let was_already_inactive = match self
            .client
            .get_json::<Value>(&format!("/workflows/{workflow_id}"), &[])
            .await
        {
            Ok(v) => !v.get("active").and_then(|a| a.as_bool()).unwrap_or(false),
            Err(N8nClientError::NotFound) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!(
                        "n8n.deactivate_workflow: no workflow with id `{workflow_id}` on this instance"
                    ),
                });
            }
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!(
                        "n8n.deactivate_workflow: pre-flight GET failed: {e}"
                    ),
                });
            }
        };
        let path = format!("/workflows/{workflow_id}/deactivate");
        match self.client.post_json::<Value>(&path, &json!({})).await {
            Ok(_) => {}
            Err(N8nClientError::NotFound) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!(
                        "n8n.deactivate_workflow: no workflow with id `{workflow_id}` on this instance"
                    ),
                });
            }
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("n8n.deactivate_workflow: API call failed: {e}"),
                });
            }
        }
        ToolOutcome::Completed {
            output: json!({
                "id": workflow_id,
                "active": false,
                "was_already_inactive": was_already_inactive,
            }),
            verified: Verification::Verified,
        }
    }
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": {
                "type": "string",
                "description": "Workflow id from `n8n.list_workflows`."
            }
        },
        "required": ["id"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool() -> N8nDeactivateWorkflow {
        use crate::{N8nClient, N8nConfig};
        use std::sync::Arc;
        let client = Arc::new(N8nClient::new(
            reqwest::Client::new(),
            N8nConfig::new("https://n8n.example.com", "ntn_x"),
        ));
        N8nDeactivateWorkflow::new(client)
    }

    #[test]
    fn required_scope_is_n8n_write() {
        assert_eq!(
            make_tool().required_scope(&json!({})).to_string(),
            "n8n.write"
        );
    }

    #[test]
    fn tool_name_is_canonical() {
        assert_eq!(make_tool().name(), "n8n.deactivate_workflow");
    }

    #[test]
    fn input_schema_marks_id_required() {
        let schema = input_schema();
        let req = schema["required"].as_array().expect("required");
        assert_eq!(req.len(), 1);
        assert_eq!(req[0], "id");
    }
}
