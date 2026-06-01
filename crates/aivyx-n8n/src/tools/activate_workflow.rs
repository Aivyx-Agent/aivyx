//! `n8n.activate_workflow` — mark a workflow active.
//!
//! Phase 131 Task 8. Second write tool. Trusted-gated.
//!
//! ## API call
//!
//! `POST /api/v1/workflows/{id}/activate`. n8n returns
//! the updated workflow record. Active workflows have
//! their trigger nodes wired up (cron, webhook, file
//! watcher, etc.) and start firing immediately.
//!
//! ## Idempotent
//!
//! Activating an already-active workflow is a no-op on
//! n8n's side and returns the workflow record
//! unchanged. We surface `was_already_active: true`
//! when we can detect it from the response so callers
//! can avoid spurious follow-up logging.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::n8n_client::{N8nClientError, SharedN8nClient};

pub struct N8nActivateWorkflow {
    id: ToolId,
    schema: Value,
    client: SharedN8nClient,
}

impl N8nActivateWorkflow {
    pub fn new(client: SharedN8nClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for N8nActivateWorkflow {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "n8n.activate_workflow"
    }

    fn description(&self) -> &str {
        "Mark an n8n workflow active. Input is a JSON \
         object with a required `id` (workflow id from \
         `n8n.list_workflows`). Active workflows wire \
         up their trigger nodes and start firing \
         immediately on the n8n instance. Idempotent: \
         activating an already-active workflow \
         succeeds. Returns `{id, active: true}` plus a \
         `was_already_active` flag for visibility. \
         Requires Trusted-tier capability grant for \
         `n8n.write`."
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
                    detail: format!("n8n.activate_workflow: {reason}"),
                });
            }
        };
        let was_already_active = match self
            .client
            .get_json::<Value>(&format!("/workflows/{workflow_id}"), &[])
            .await
        {
            Ok(v) => v.get("active").and_then(|a| a.as_bool()).unwrap_or(false),
            Err(N8nClientError::NotFound) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!(
                        "n8n.activate_workflow: no workflow with id `{workflow_id}` on this instance"
                    ),
                });
            }
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!(
                        "n8n.activate_workflow: pre-flight GET failed: {e}"
                    ),
                });
            }
        };
        let path = format!("/workflows/{workflow_id}/activate");
        match self.client.post_json::<Value>(&path, &json!({})).await {
            Ok(_) => {}
            Err(N8nClientError::NotFound) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!(
                        "n8n.activate_workflow: no workflow with id `{workflow_id}` on this instance"
                    ),
                });
            }
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("n8n.activate_workflow: API call failed: {e}"),
                });
            }
        }
        ToolOutcome::Completed {
            output: json!({
                "id": workflow_id,
                "active": true,
                "was_already_active": was_already_active,
            }),
            verified: Verification::Verified,
        }
    }
}

pub(crate) fn parse_id(input: &Value) -> Result<String, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
    let id = obj
        .get("id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| "input must include an `id` string field".to_string())?;
    if id.is_empty() {
        return Err("`id` must not be empty".to_string());
    }
    Ok(id.to_string())
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

    #[test]
    fn parse_id_accepts_id() {
        assert_eq!(parse_id(&json!({"id": "wf-1"})).unwrap(), "wf-1");
    }

    #[test]
    fn parse_id_trims_whitespace() {
        assert_eq!(parse_id(&json!({"id": "  wf-1  "})).unwrap(), "wf-1");
    }

    #[test]
    fn parse_id_rejects_missing() {
        let e = parse_id(&json!({})).expect_err("must error");
        assert!(e.contains("id"), "{e}");
    }

    #[test]
    fn parse_id_rejects_empty() {
        let e = parse_id(&json!({"id": "   "})).expect_err("must error");
        assert!(e.contains("id"), "{e}");
    }

    fn make_tool() -> N8nActivateWorkflow {
        use crate::{N8nClient, N8nConfig};
        use std::sync::Arc;
        let client = Arc::new(N8nClient::new(
            reqwest::Client::new(),
            N8nConfig::new("https://n8n.example.com", "ntn_x"),
        ));
        N8nActivateWorkflow::new(client)
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
        assert_eq!(make_tool().name(), "n8n.activate_workflow");
    }
}
