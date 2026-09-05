//! `n8n.list_workflows` — enumerate workflows on the
//! operator's n8n instance.
//!
//! Phase 131 Task 3. First read tool.
//!
//! ## API call
//!
//! `GET /api/v1/workflows` with optional query
//! parameters: `active` (bool), `name` (string),
//! `tags` (string), `limit` (1-250), `cursor` (opaque
//! pagination token). Response shape:
//! `{"data": [...], "nextCursor": "..."}`.
//!
//! ## Output
//!
//! We project each workflow down to the fields the LLM
//! cares about — id, name, active, createdAt, updatedAt,
//! and tags. The full n8n workflow payload includes
//! nodes + connections + settings + pinData and is
//! large; for enumeration the projected shape is the
//! right altitude. Operators who need the full payload
//! call `n8n.get_workflow` against a specific id.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::n8n_client::SharedN8nClient;

const MAX_PAGE_SIZE: u64 = 250;
const DEFAULT_PAGE_SIZE: u64 = 25;

pub struct N8nListWorkflows {
    id: ToolId,
    schema: Value,
    client: SharedN8nClient,
}

impl N8nListWorkflows {
    pub fn new(client: SharedN8nClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for N8nListWorkflows {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "n8n.list_workflows"
    }

    // Chapter Picket follow-up (Finding 3) — same rationale as
    // n8n.get_workflow: externally authored workflow definitions.
    fn output_is_untrusted(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "List workflows on the operator's n8n instance. \
         Input is a JSON object with optional `active` \
         (boolean filter: only return active or only \
         inactive workflows), `name` (substring filter \
         applied server-side), `tags` (comma-separated \
         tag names — n8n's API treats this as an OR \
         match), `max_results` (default 25, capped at \
         250), and `cursor` for pagination. Returns \
         `{results, next_cursor}` where each result is \
         `{id, name, active, created_at, updated_at, \
         tags}`. To fetch the full workflow definition \
         (nodes, connections, settings), call \
         `n8n.get_workflow` against a specific id."
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
                    detail: format!("n8n.list_workflows: {reason}"),
                });
            }
        };
        let query = build_query(&parsed);
        let resp: Value = match self.client.get_json("/workflows", &query).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("n8n.list_workflows: API call failed: {e}"),
                });
            }
        };
        let results: Vec<Value> = resp
            .get("data")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().map(project_workflow).collect())
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

pub(crate) fn project_workflow(wf: &Value) -> Value {
    let id = wf.get("id").cloned().unwrap_or(Value::Null);
    let name = wf.get("name").cloned().unwrap_or(Value::Null);
    let active = wf.get("active").cloned().unwrap_or(Value::Null);
    let created_at = wf.get("createdAt").cloned().unwrap_or(Value::Null);
    let updated_at = wf.get("updatedAt").cloned().unwrap_or(Value::Null);
    let tags = wf
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.get("name").and_then(|n| n.as_str()))
                .map(|s| Value::String(s.to_string()))
                .collect::<Vec<_>>()
        })
        .map(Value::Array)
        .unwrap_or(Value::Array(Vec::new()));
    json!({
        "id": id,
        "name": name,
        "active": active,
        "created_at": created_at,
        "updated_at": updated_at,
        "tags": tags,
    })
}

#[derive(Debug)]
pub(crate) struct ParsedInput {
    pub(crate) active: Option<bool>,
    pub(crate) name: Option<String>,
    pub(crate) tags: Option<String>,
    pub(crate) max_results: u64,
    pub(crate) cursor: Option<String>,
}

fn parse_input(input: &Value) -> Result<ParsedInput, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
    let active = match obj.get("active") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => return Err("`active` must be a boolean".to_string()),
    };
    let name = match obj.get("name") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        Some(_) => return Err("`name` must be a string".to_string()),
    };
    let tags = match obj.get("tags") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        Some(_) => return Err("`tags` must be a string".to_string()),
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
        active,
        name,
        tags,
        max_results,
        cursor,
    })
}

pub(crate) fn build_query(parsed: &ParsedInput) -> Vec<(&'static str, String)> {
    let mut q: Vec<(&'static str, String)> = Vec::new();
    if let Some(a) = parsed.active {
        q.push(("active", a.to_string()));
    }
    if let Some(ref n) = parsed.name {
        q.push(("name", n.clone()));
    }
    if let Some(ref t) = parsed.tags {
        q.push(("tags", t.clone()));
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
            "active": {
                "type": ["boolean", "null"],
                "description": "Filter: true returns only active workflows, false only inactive. Omit for both."
            },
            "name": {
                "type": ["string", "null"],
                "description": "Substring filter on workflow name (server-side)."
            },
            "tags": {
                "type": ["string", "null"],
                "description": "Comma-separated tag names (n8n OR-matches)."
            },
            "max_results": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_PAGE_SIZE,
                "default": DEFAULT_PAGE_SIZE
            },
            "cursor": {
                "type": ["string", "null"],
                "description": "Pagination cursor from a previous response's `next_cursor`."
            }
        },
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn n8n_list_workflows_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }

    #[test]
    fn parse_input_accepts_empty() {
        let p = parse_input(&json!({})).expect("parse");
        assert!(p.active.is_none());
        assert!(p.name.is_none());
        assert!(p.tags.is_none());
        assert_eq!(p.max_results, DEFAULT_PAGE_SIZE);
        assert!(p.cursor.is_none());
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
    fn parse_input_rejects_non_boolean_active() {
        let e = parse_input(&json!({"active": "yes"})).expect_err("must error");
        assert!(e.contains("active"), "{e}");
    }

    #[test]
    fn parse_input_accepts_active_filter() {
        let p = parse_input(&json!({"active": true})).expect("parse");
        assert_eq!(p.active, Some(true));
        let p = parse_input(&json!({"active": false})).expect("parse");
        assert_eq!(p.active, Some(false));
    }

    #[test]
    fn parse_input_trims_strings_and_treats_empty_as_none() {
        let p = parse_input(&json!({"name": "  ", "tags": "", "cursor": "   "})).expect("parse");
        assert!(p.name.is_none());
        assert!(p.tags.is_none());
        assert!(p.cursor.is_none());
    }

    #[test]
    fn build_query_omits_unset_fields() {
        let p = parse_input(&json!({})).unwrap();
        let q = build_query(&p);
        assert!(q.iter().any(|(k, v)| *k == "limit" && v == "25"));
        assert!(!q.iter().any(|(k, _)| *k == "active"));
        assert!(!q.iter().any(|(k, _)| *k == "name"));
    }

    #[test]
    fn build_query_emits_all_set_fields() {
        let p = parse_input(&json!({
            "active": true,
            "name": "deploy",
            "tags": "ci,prod",
            "max_results": 50,
            "cursor": "abc"
        }))
        .unwrap();
        let q = build_query(&p);
        assert!(q.contains(&("active", "true".to_string())));
        assert!(q.contains(&("name", "deploy".to_string())));
        assert!(q.contains(&("tags", "ci,prod".to_string())));
        assert!(q.contains(&("limit", "50".to_string())));
        assert!(q.contains(&("cursor", "abc".to_string())));
    }

    #[test]
    fn project_workflow_extracts_canonical_fields() {
        let wf = json!({
            "id": "1",
            "name": "Daily Report",
            "active": true,
            "createdAt": "2026-01-01T00:00:00.000Z",
            "updatedAt": "2026-06-01T00:00:00.000Z",
            "tags": [{"name": "report"}, {"name": "daily"}],
            "nodes": ["large", "array", "discarded"]
        });
        let out = project_workflow(&wf);
        assert_eq!(out["id"], "1");
        assert_eq!(out["name"], "Daily Report");
        assert_eq!(out["active"], true);
        assert_eq!(out["tags"], json!(["report", "daily"]));
        assert!(out.get("nodes").is_none(), "nodes must be projected away");
    }

    #[test]
    fn project_workflow_handles_missing_tags() {
        let wf = json!({"id": "1", "name": "x", "active": false});
        let out = project_workflow(&wf);
        assert_eq!(out["tags"], json!([]));
    }

    #[test]
    fn input_schema_has_no_required_fields() {
        let schema = input_schema();
        assert!(schema.get("required").is_none());
    }

    fn make_tool() -> N8nListWorkflows {
        use crate::{N8nClient, N8nConfig};
        use std::sync::Arc;
        let client = Arc::new(N8nClient::new(
            reqwest::Client::new(),
            N8nConfig::new("https://n8n.example.com", "ntn_x"),
        ));
        N8nListWorkflows::new(client)
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
        assert_eq!(make_tool().name(), "n8n.list_workflows");
    }
}
