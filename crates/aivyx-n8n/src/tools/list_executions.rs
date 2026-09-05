//! `n8n.list_executions` — enumerate workflow executions.
//!
//! Phase 131 Task 5. Third read tool.
//!
//! ## API call
//!
//! `GET /api/v1/executions` with optional query
//! parameters: `workflowId` (string, filter to a single
//! workflow), `status` (`success` / `error` / `waiting`),
//! `limit` (1-250), `cursor`. We deliberately do **not**
//! expose `includeData` — execution payloads can contain
//! the full input + output of every node and quickly
//! exceed the LLM's context window. Operators who need
//! the full payload call `n8n.get_execution` against a
//! specific id.
//!
//! ## Output
//!
//! Each execution is projected to summary fields: id,
//! workflow_id, status, mode, started_at, stopped_at,
//! finished, retry_of. Same `{results, next_cursor}`
//! pagination shape as `n8n.list_workflows`.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::n8n_client::SharedN8nClient;

const MAX_PAGE_SIZE: u64 = 250;
const DEFAULT_PAGE_SIZE: u64 = 25;

pub struct N8nListExecutions {
    id: ToolId,
    schema: Value,
    client: SharedN8nClient,
}

impl N8nListExecutions {
    pub fn new(client: SharedN8nClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for N8nListExecutions {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "n8n.list_executions"
    }

    // Chapter Picket follow-up (Finding 3) — this tool deliberately
    // projects away the execution payload (see its own description),
    // but flagged as a matter of defense-in-depth: if that projection
    // ever widens to include node inputs/outputs, coverage is already
    // in place rather than needing to be added at the same time.
    fn output_is_untrusted(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "List workflow executions on the operator's n8n \
         instance. Input is a JSON object with optional \
         `workflow_id` (filter to a single workflow), \
         `status` (one of `success`, `error`, `waiting`), \
         `max_results` (default 25, capped at 250), and \
         `cursor` for pagination. Returns \
         `{results, next_cursor}` where each result is \
         `{id, workflow_id, status, mode, started_at, \
         stopped_at, finished, retry_of}`. The full \
         execution data payload (node inputs + outputs) \
         is deliberately not included — call \
         `n8n.get_execution` against a specific id to \
         fetch the payload."
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
        let parsed = match parse_input(&input) {
            Ok(p) => p,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("n8n.list_executions: {reason}"),
                });
            }
        };
        let query = build_query(&parsed);
        let resp: Value = match self.client.get_json("/executions", &query).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("n8n.list_executions: API call failed: {e}"),
                });
            }
        };
        let results: Vec<Value> = resp
            .get("data")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().map(project_execution).collect())
            .unwrap_or_default();
        let next_cursor = resp.get("nextCursor").cloned().unwrap_or(Value::Null);
        let output = json!({
            "results": results,
            "next_cursor": next_cursor,
        });
        ToolOutcome::Completed {
            output,
            verified: Verification::NotApplicable,
        }
    }
}

pub(crate) fn project_execution(exec: &Value) -> Value {
    let id = exec.get("id").cloned().unwrap_or(Value::Null);
    let workflow_id = exec.get("workflowId").cloned().unwrap_or(Value::Null);
    let status = exec.get("status").cloned().unwrap_or(Value::Null);
    let mode = exec.get("mode").cloned().unwrap_or(Value::Null);
    let started_at = exec.get("startedAt").cloned().unwrap_or(Value::Null);
    let stopped_at = exec.get("stoppedAt").cloned().unwrap_or(Value::Null);
    let finished = exec.get("finished").cloned().unwrap_or(Value::Null);
    let retry_of = exec.get("retryOf").cloned().unwrap_or(Value::Null);
    json!({
        "id": id,
        "workflow_id": workflow_id,
        "status": status,
        "mode": mode,
        "started_at": started_at,
        "stopped_at": stopped_at,
        "finished": finished,
        "retry_of": retry_of,
    })
}

#[derive(Debug)]
pub(crate) struct ParsedInput {
    pub(crate) workflow_id: Option<String>,
    pub(crate) status: Option<String>,
    pub(crate) max_results: u64,
    pub(crate) cursor: Option<String>,
}

const ALLOWED_STATUS: &[&str] = &["success", "error", "waiting"];

fn parse_input(input: &Value) -> Result<ParsedInput, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
    let workflow_id = match obj.get("workflow_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        Some(_) => return Err("`workflow_id` must be a string".to_string()),
    };
    let status = match obj.get("status") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else if !ALLOWED_STATUS.contains(&t) {
                return Err(format!(
                    "`status` must be one of {ALLOWED_STATUS:?} — got `{t}`"
                ));
            } else {
                Some(t.to_string())
            }
        }
        Some(_) => return Err("`status` must be a string".to_string()),
    };
    let max_results = match obj.get("max_results") {
        None => DEFAULT_PAGE_SIZE,
        Some(v) => v
            .as_u64()
            .ok_or_else(|| "`max_results` must be a non-negative integer".to_string())?,
    };
    if max_results == 0 {
        return Err("`max_results` must be >= 1".to_string());
    }
    let max_results = max_results.min(MAX_PAGE_SIZE);
    let cursor = match obj.get("cursor") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        Some(_) => return Err("`cursor` must be a string".to_string()),
    };
    Ok(ParsedInput {
        workflow_id,
        status,
        max_results,
        cursor,
    })
}

pub(crate) fn build_query(parsed: &ParsedInput) -> Vec<(&'static str, String)> {
    let mut q: Vec<(&'static str, String)> = Vec::new();
    if let Some(ref w) = parsed.workflow_id {
        q.push(("workflowId", w.clone()));
    }
    if let Some(ref s) = parsed.status {
        q.push(("status", s.clone()));
    }
    q.push(("limit", parsed.max_results.to_string()));
    if let Some(ref c) = parsed.cursor {
        q.push(("cursor", c.clone()));
    }
    q
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "workflow_id": {
                "type": ["string", "null"],
                "description": "Filter executions to a single workflow id."
            },
            "status": {
                "type": ["string", "null"],
                "enum": ["success", "error", "waiting", null],
                "description": "Filter by execution outcome."
            },
            "max_results": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_PAGE_SIZE,
                "default": DEFAULT_PAGE_SIZE
            },
            "cursor": {
                "type": ["string", "null"],
                "description": "Pagination cursor from a previous response."
            }
        },
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn n8n_list_executions_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }

    #[test]
    fn parse_input_accepts_empty() {
        let p = parse_input(&json!({})).expect("parse");
        assert!(p.workflow_id.is_none());
        assert!(p.status.is_none());
        assert_eq!(p.max_results, DEFAULT_PAGE_SIZE);
        assert!(p.cursor.is_none());
    }

    #[test]
    fn parse_input_accepts_all_filters() {
        let p = parse_input(&json!({
            "workflow_id": "wf-1",
            "status": "success",
            "max_results": 100,
            "cursor": "abc"
        }))
        .expect("parse");
        assert_eq!(p.workflow_id.as_deref(), Some("wf-1"));
        assert_eq!(p.status.as_deref(), Some("success"));
        assert_eq!(p.max_results, 100);
        assert_eq!(p.cursor.as_deref(), Some("abc"));
    }

    #[test]
    fn parse_input_rejects_invalid_status() {
        let e = parse_input(&json!({"status": "running"})).expect_err("must error");
        assert!(e.contains("status"), "{e}");
    }

    #[test]
    fn parse_input_accepts_all_valid_statuses() {
        for s in ALLOWED_STATUS {
            parse_input(&json!({"status": s})).expect("parse");
        }
    }

    #[test]
    fn parse_input_caps_max_results() {
        let p = parse_input(&json!({"max_results": 9999})).expect("parse");
        assert_eq!(p.max_results, MAX_PAGE_SIZE);
    }

    #[test]
    fn parse_input_rejects_zero_max_results() {
        let e = parse_input(&json!({"max_results": 0})).expect_err("must error");
        assert!(e.contains("max_results"), "{e}");
    }

    #[test]
    fn build_query_emits_workflow_filter_with_camelcase_key() {
        let p = parse_input(&json!({"workflow_id": "wf-7"})).unwrap();
        let q = build_query(&p);
        assert!(q.contains(&("workflowId", "wf-7".to_string())));
        assert!(!q.iter().any(|(k, _)| *k == "workflow_id"));
    }

    #[test]
    fn build_query_omits_unset_fields() {
        let p = parse_input(&json!({})).unwrap();
        let q = build_query(&p);
        assert!(q.iter().any(|(k, v)| *k == "limit" && v == "25"));
        assert!(!q.iter().any(|(k, _)| *k == "workflowId"));
        assert!(!q.iter().any(|(k, _)| *k == "status"));
    }

    #[test]
    fn project_execution_extracts_summary_fields() {
        let exec = json!({
            "id": "e1",
            "workflowId": "wf-1",
            "status": "success",
            "mode": "trigger",
            "startedAt": "2026-06-01T00:00:00Z",
            "stoppedAt": "2026-06-01T00:00:05Z",
            "finished": true,
            "retryOf": null,
            "data": {"large": "payload"}
        });
        let out = project_execution(&exec);
        assert_eq!(out["id"], "e1");
        assert_eq!(out["workflow_id"], "wf-1");
        assert_eq!(out["status"], "success");
        assert_eq!(out["finished"], true);
        assert!(out.get("data").is_none(), "data payload must be projected away");
    }

    fn make_tool() -> N8nListExecutions {
        use crate::{N8nClient, N8nConfig};
        use std::sync::Arc;
        let client = Arc::new(N8nClient::new(
            reqwest::Client::new(),
            N8nConfig::new("https://n8n.example.com", "ntn_x"),
        ));
        N8nListExecutions::new(client)
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
        assert_eq!(make_tool().name(), "n8n.list_executions");
    }
}
