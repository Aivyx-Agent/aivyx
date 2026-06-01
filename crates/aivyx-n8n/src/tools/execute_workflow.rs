//! `n8n.execute_workflow` — trigger a one-off run of a
//! workflow.
//!
//! Phase 131 Task 7. First write tool. Trusted-gated.
//!
//! ## API call
//!
//! `POST /api/v1/workflows/{id}/execute` with an optional
//! body carrying trigger input data. n8n returns the
//! resulting execution record so the LLM can immediately
//! inspect status / id / mode without a follow-up
//! `n8n.get_execution` call.
//!
//! ## Why Trusted
//!
//! Executing a workflow runs the operator-authored
//! node graph — which may call arbitrary external APIs,
//! mutate downstream services, or send notifications.
//! Same trust-boundary rationale as `notify.send` /
//! `email.send` / `calendar.write`: SemiTrusted
//! channels (e.g., a remote Telegram operator) must not
//! be able to coerce the agent into firing operator
//! automation. Operators who want narrower delegation
//! grant `n8n.write:<workflow-id>` to a role via
//! `capability_scopes`.
//!
//! ## Verification
//!
//! `Verified` — n8n returns the execution id, which is
//! a positive confirmation that the request landed. We
//! don't wait for completion; long-running workflows
//! finish asynchronously and the LLM uses
//! `n8n.get_execution` to poll if it cares.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::n8n_client::{N8nClientError, SharedN8nClient};

pub struct N8nExecuteWorkflow {
    id: ToolId,
    schema: Value,
    client: SharedN8nClient,
}

impl N8nExecuteWorkflow {
    pub fn new(client: SharedN8nClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for N8nExecuteWorkflow {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "n8n.execute_workflow"
    }

    fn description(&self) -> &str {
        "Trigger a one-off run of an n8n workflow. \
         Input is a JSON object with a required `id` \
         (workflow id from `n8n.list_workflows`) and \
         optional `input_data` (JSON object passed to \
         the workflow's trigger node). Returns the \
         execution record — `{id, status, mode, \
         finished, started_at}` — so the call site can \
         immediately read the execution id; use \
         `n8n.get_execution` to poll for completion of \
         long-running workflows. Requires Trusted-tier \
         capability grant for `n8n.write` (workflow \
         runs invoke operator-authored node graphs that \
         may call external APIs or mutate downstream \
         services)."
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
        let (workflow_id, body) = match parse_input(&input) {
            Ok(p) => p,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("n8n.execute_workflow: {reason}"),
                });
            }
        };
        let path = format!("/workflows/{workflow_id}/execute");
        let resp: Value = match self.client.post_json(&path, &body).await {
            Ok(v) => v,
            Err(N8nClientError::NotFound) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!(
                        "n8n.execute_workflow: no workflow with id `{workflow_id}` on this instance"
                    ),
                });
            }
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("n8n.execute_workflow: API call failed: {e}"),
                });
            }
        };
        let output = summarize_execution(&resp);
        ToolOutcome::Completed {
            output,
            verified: Verification::Verified,
        }
    }
}

pub(crate) fn summarize_execution(resp: &Value) -> Value {
    let id = resp.get("id").cloned().unwrap_or(Value::Null);
    let status = resp.get("status").cloned().unwrap_or(Value::Null);
    let mode = resp.get("mode").cloned().unwrap_or(Value::Null);
    let finished = resp.get("finished").cloned().unwrap_or(Value::Null);
    let started_at = resp.get("startedAt").cloned().unwrap_or(Value::Null);
    json!({
        "id": id,
        "status": status,
        "mode": mode,
        "finished": finished,
        "started_at": started_at,
    })
}

fn parse_input(input: &Value) -> Result<(String, Value), String> {
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
    let body = match obj.get("input_data") {
        None | Some(Value::Null) => json!({}),
        Some(v) => {
            if !v.is_object() {
                return Err(
                    "`input_data` must be a JSON object (it is forwarded to the trigger node)"
                        .to_string(),
                );
            }
            v.clone()
        }
    };
    Ok((id.to_string(), body))
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": {
                "type": "string",
                "description": "Workflow id from `n8n.list_workflows`."
            },
            "input_data": {
                "type": ["object", "null"],
                "description": "Optional JSON object forwarded to the workflow's trigger node."
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
    fn parse_input_accepts_id_only() {
        let (id, body) = parse_input(&json!({"id": "wf-1"})).expect("parse");
        assert_eq!(id, "wf-1");
        assert_eq!(body, json!({}));
    }

    #[test]
    fn parse_input_accepts_input_data() {
        let (_, body) =
            parse_input(&json!({"id": "wf-1", "input_data": {"x": 1}})).expect("parse");
        assert_eq!(body, json!({"x": 1}));
    }

    #[test]
    fn parse_input_rejects_non_object_input_data() {
        let e = parse_input(&json!({"id": "wf-1", "input_data": [1, 2]}))
            .expect_err("must error");
        assert!(e.contains("input_data"), "{e}");
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
    fn summarize_execution_projects_canonical_fields() {
        let resp = json!({
            "id": "e1",
            "status": "running",
            "mode": "manual",
            "finished": false,
            "startedAt": "2026-06-01T00:00:00Z",
            "data": {"large": "payload"}
        });
        let out = summarize_execution(&resp);
        assert_eq!(out["id"], "e1");
        assert_eq!(out["status"], "running");
        assert_eq!(out["mode"], "manual");
        assert_eq!(out["finished"], false);
        assert_eq!(out["started_at"], "2026-06-01T00:00:00Z");
        assert!(out.get("data").is_none());
    }

    #[test]
    fn input_schema_marks_id_required() {
        let schema = input_schema();
        let req = schema["required"].as_array().expect("required");
        assert_eq!(req.len(), 1);
        assert_eq!(req[0], "id");
    }

    fn make_tool() -> N8nExecuteWorkflow {
        use crate::{N8nClient, N8nConfig};
        use std::sync::Arc;
        let client = Arc::new(N8nClient::new(
            reqwest::Client::new(),
            N8nConfig::new("https://n8n.example.com", "ntn_x"),
        ));
        N8nExecuteWorkflow::new(client)
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
        assert_eq!(make_tool().name(), "n8n.execute_workflow");
    }
}
