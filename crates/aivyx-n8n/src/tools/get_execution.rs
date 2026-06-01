//! `n8n.get_execution` — fetch a single execution by id.
//!
//! Phase 131 Task 6. Fourth read tool — last of the read
//! surface before the lifecycle / CRUD writes begin.
//!
//! ## API call
//!
//! `GET /api/v1/executions/{id}?includeData={bool}`. When
//! `includeData=true`, the response carries the
//! per-node input + output payload of the run; when
//! false (default), the response is the execution
//! metadata only (status, mode, timing, error message
//! if any). We surface `include_data` as an explicit
//! input so the LLM has to opt into the larger payload.
//!
//! ## NotFound
//!
//! Same shape as `n8n.get_workflow` — 404 → structured
//! error.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::n8n_client::{N8nClientError, SharedN8nClient};

pub struct N8nGetExecution {
    id: ToolId,
    schema: Value,
    client: SharedN8nClient,
}

impl N8nGetExecution {
    pub fn new(client: SharedN8nClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for N8nGetExecution {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "n8n.get_execution"
    }

    fn description(&self) -> &str {
        "Fetch a single n8n execution by id. Input is a \
         JSON object with required `id` (execution id \
         from `n8n.list_executions`) and optional \
         `include_data` (boolean, default false). When \
         `include_data` is true, the response carries \
         the per-node input + output payload of the run \
         — large; the LLM should opt in only when \
         debugging a specific run. When false, the \
         response is metadata only: status, mode, \
         startedAt, stoppedAt, finished, retryOf, \
         workflowId, and (on failure) the error \
         message. Returns a structured error if the \
         execution id does not exist."
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
        let (execution_id, include_data) = match parse_input(&input) {
            Ok(p) => p,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("n8n.get_execution: {reason}"),
                });
            }
        };
        let path = format!("/executions/{execution_id}");
        let query: Vec<(&'static str, String)> = if include_data {
            vec![("includeData", "true".to_string())]
        } else {
            Vec::new()
        };
        let resp: Value = match self.client.get_json(&path, &query).await {
            Ok(v) => v,
            Err(N8nClientError::NotFound) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!(
                        "n8n.get_execution: no execution with id `{execution_id}` on this instance"
                    ),
                });
            }
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("n8n.get_execution: API call failed: {e}"),
                });
            }
        };
        ToolOutcome::Completed {
            output: resp,
            verified: Verification::NotApplicable,
        }
    }
}

fn parse_input(input: &Value) -> Result<(String, bool), String> {
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
    let include_data = match obj.get("include_data") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err("`include_data` must be a boolean".to_string()),
    };
    Ok((id.to_string(), include_data))
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": {
                "type": "string",
                "description": "Execution id from `n8n.list_executions`."
            },
            "include_data": {
                "type": ["boolean", "null"],
                "default": false,
                "description": "Set true to include the per-node input/output payload."
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
    fn parse_input_accepts_minimal() {
        let (id, inc) = parse_input(&json!({"id": "e1"})).expect("parse");
        assert_eq!(id, "e1");
        assert!(!inc);
    }

    #[test]
    fn parse_input_accepts_include_data_true() {
        let (_, inc) = parse_input(&json!({"id": "e1", "include_data": true})).unwrap();
        assert!(inc);
    }

    #[test]
    fn parse_input_rejects_non_boolean_include_data() {
        let e = parse_input(&json!({"id": "e1", "include_data": "yes"}))
            .expect_err("must error");
        assert!(e.contains("include_data"), "{e}");
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
    fn parse_input_treats_include_data_null_as_false() {
        let (_, inc) = parse_input(&json!({"id": "e1", "include_data": null})).unwrap();
        assert!(!inc);
    }

    #[test]
    fn input_schema_marks_only_id_required() {
        let schema = input_schema();
        let req = schema["required"].as_array().expect("required");
        assert_eq!(req.len(), 1);
        assert_eq!(req[0], "id");
    }

    fn make_tool() -> N8nGetExecution {
        use crate::{N8nClient, N8nConfig};
        use std::sync::Arc;
        let client = Arc::new(N8nClient::new(
            reqwest::Client::new(),
            N8nConfig::new("https://n8n.example.com", "ntn_x"),
        ));
        N8nGetExecution::new(client)
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
        assert_eq!(make_tool().name(), "n8n.get_execution");
    }
}
