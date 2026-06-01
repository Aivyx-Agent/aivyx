//! `n8n.get_workflow` — fetch the full definition of a
//! single workflow by id.
//!
//! Phase 131 Task 4. Second read tool.
//!
//! ## API call
//!
//! `GET /api/v1/workflows/{id}`. Response is the full
//! workflow record — id, name, active, settings, nodes,
//! connections, staticData, tags, pinData, versionId,
//! triggerCount, createdAt, updatedAt.
//!
//! ## Output
//!
//! The full workflow object is passed through. Unlike
//! `n8n.list_workflows` (which projects down to a summary
//! for enumeration efficiency), get_workflow is the
//! "read everything" entry — operators and the LLM use
//! it to inspect node graphs, parameters, and credential
//! bindings before updating or executing.
//!
//! ## NotFound
//!
//! n8n returns HTTP 404 for missing workflow ids. The
//! N8nClient maps that to `N8nClientError::NotFound`,
//! which we surface as a structured error with a clear
//! message rather than letting it ride as a generic
//! API failure.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::n8n_client::{N8nClientError, SharedN8nClient};

pub struct N8nGetWorkflow {
    id: ToolId,
    schema: Value,
    client: SharedN8nClient,
}

impl N8nGetWorkflow {
    pub fn new(client: SharedN8nClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for N8nGetWorkflow {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "n8n.get_workflow"
    }

    fn description(&self) -> &str {
        "Fetch the full definition of an n8n workflow by \
         id. Input is a JSON object with a required `id` \
         (workflow id from `n8n.list_workflows`). Returns \
         the full workflow record: id, name, active, \
         settings, nodes (the node graph), connections, \
         staticData, tags, pinData, versionId, \
         triggerCount, createdAt, updatedAt. Use this \
         before `n8n.update_workflow` to read current \
         state, or before `n8n.execute_workflow` to \
         verify which inputs the trigger node expects. \
         Returns a structured error if the workflow id \
         does not exist."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("n8n.read").expect(
            "n8n.read must parse — it is in KNOWN_BASES from Phase 131",
        )
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let workflow_id = match parse_input(&input) {
            Ok(s) => s,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("n8n.get_workflow: {reason}"),
                });
            }
        };
        let path = format!("/workflows/{workflow_id}");
        let resp: Value = match self.client.get_json(&path, &[]).await {
            Ok(v) => v,
            Err(N8nClientError::NotFound) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!(
                        "n8n.get_workflow: no workflow with id `{workflow_id}` on this instance"
                    ),
                });
            }
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("n8n.get_workflow: API call failed: {e}"),
                });
            }
        };
        ToolOutcome::Completed {
            output: resp,
            verified: Verification::NotApplicable,
        }
    }
}

fn parse_input(input: &Value) -> Result<String, String> {
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
    fn parse_input_accepts_id() {
        let id = parse_input(&json!({"id": "wf-1"})).expect("parse");
        assert_eq!(id, "wf-1");
    }

    #[test]
    fn parse_input_trims_whitespace() {
        let id = parse_input(&json!({"id": "  wf-1  "})).expect("parse");
        assert_eq!(id, "wf-1");
    }

    #[test]
    fn parse_input_rejects_missing_id() {
        let e = parse_input(&json!({})).expect_err("must error");
        assert!(e.contains("id"), "{e}");
    }

    #[test]
    fn parse_input_rejects_empty_id() {
        let e = parse_input(&json!({"id": "   "})).expect_err("must error");
        assert!(e.contains("id"), "{e}");
    }

    #[test]
    fn parse_input_rejects_non_string_id() {
        let e = parse_input(&json!({"id": 42})).expect_err("must error");
        assert!(e.contains("id"), "{e}");
    }

    #[test]
    fn input_schema_marks_id_required() {
        let schema = input_schema();
        let req = schema["required"].as_array().expect("required");
        assert_eq!(req.len(), 1);
        assert_eq!(req[0], "id");
    }

    fn make_tool() -> N8nGetWorkflow {
        use crate::{N8nClient, N8nConfig};
        use std::sync::Arc;
        let client = Arc::new(N8nClient::new(
            reqwest::Client::new(),
            N8nConfig::new("https://n8n.example.com", "ntn_x"),
        ));
        N8nGetWorkflow::new(client)
    }

    #[test]
    fn required_scope_is_n8n_read() {
        assert_eq!(
            make_tool().required_scope(&json!({})).to_string(),
            "n8n.read"
        );
    }

    #[test]
    fn tool_name_is_canonical() {
        assert_eq!(make_tool().name(), "n8n.get_workflow");
    }
}
