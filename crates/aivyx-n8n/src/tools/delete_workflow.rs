//! `n8n.delete_workflow` — permanently delete a workflow.
//!
//! Phase 131 Task 12. Sixth and final write tool.
//! Trusted-gated.
//!
//! ## API call
//!
//! `DELETE /api/v1/workflows/{id}`. Idempotent on
//! already-missing via the N8nClient's
//! 404-treated-as-success delete pattern (matches the
//! `obsidian.delete_note` / Phase 130 idempotent-delete
//! convention).
//!
//! ## Permanence
//!
//! This is a **permanent delete** — n8n does not move
//! the workflow to a trash bin. Execution history
//! associated with the workflow is also removed.
//! Operators who want recovery should
//! `n8n.get_workflow` first and save the definition
//! locally.

use async_trait::async_trait;
use reqwest::StatusCode;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::n8n_client::SharedN8nClient;

pub struct N8nDeleteWorkflow {
    id: ToolId,
    schema: Value,
    client: SharedN8nClient,
}

impl N8nDeleteWorkflow {
    pub fn new(client: SharedN8nClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for N8nDeleteWorkflow {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "n8n.delete_workflow"
    }

    fn description(&self) -> &str {
        "Permanently delete an n8n workflow. Input is a \
         JSON object with a required `id` (workflow id \
         from `n8n.list_workflows`). Idempotent: \
         deleting a missing workflow succeeds with \
         `was_already_missing: true`. NOTE: this is a \
         **permanent delete** — n8n has no trash. \
         Execution history for the workflow is also \
         removed. To preserve the definition for \
         recovery, call `n8n.get_workflow` first and \
         save the result. Requires Trusted-tier \
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
                    detail: format!("n8n.delete_workflow: {reason}"),
                });
            }
        };
        let path = format!("/workflows/{workflow_id}");
        let status = match self.client.delete(&path).await {
            Ok(s) => s,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("n8n.delete_workflow: API call failed: {e}"),
                });
            }
        };
        let was_already_missing = status == StatusCode::NOT_FOUND;
        ToolOutcome::Completed {
            output: json!({
                "id": workflow_id,
                "was_already_missing": was_already_missing,
            }),
            verified: Verification::Verified,
        }
    }
}

fn parse_id(input: &Value) -> Result<String, String> {
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
    fn parse_id_rejects_missing() {
        let e = parse_id(&json!({})).expect_err("must error");
        assert!(e.contains("id"), "{e}");
    }

    #[test]
    fn parse_id_rejects_empty() {
        let e = parse_id(&json!({"id": "  "})).expect_err("must error");
        assert!(e.contains("id"), "{e}");
    }

    #[test]
    fn input_schema_marks_id_required() {
        let schema = input_schema();
        let req = schema["required"].as_array().expect("required");
        assert_eq!(req.len(), 1);
    }

    fn make_tool() -> N8nDeleteWorkflow {
        use crate::{N8nClient, N8nConfig};
        use std::sync::Arc;
        let client = Arc::new(N8nClient::new(
            reqwest::Client::new(),
            N8nConfig::new("https://n8n.example.com", "ntn_x"),
        ));
        N8nDeleteWorkflow::new(client)
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
        assert_eq!(make_tool().name(), "n8n.delete_workflow");
    }
}
