//! `n8n.update_workflow` — replace a workflow definition.
//!
//! Phase 131 Task 11. Fifth write tool. Trusted-gated.
//! Q1c risky CRUD pick.
//!
//! ## API call
//!
//! `PUT /api/v1/workflows/{id}` with a full replacement
//! body `{name, nodes, connections, settings,
//! staticData?}`. n8n treats update as a full
//! replacement (not a JSON Merge Patch) — callers
//! typically read the current definition via
//! `n8n.get_workflow`, mutate the relevant fields, then
//! send the whole thing back.
//!
//! ## Blast radius
//!
//! Same as `n8n.create_workflow` — a workflow
//! definition can call arbitrary HTTP, bind
//! credentials, schedule execution, mutate downstream.
//! On top of that, updating an **active** workflow
//! takes effect immediately on the next trigger
//! firing; the operator gets no second checkpoint the
//! way create does. Operators who care about staged
//! rollout can wrap calls in:
//! 1. `n8n.deactivate_workflow`
//! 2. `n8n.update_workflow`
//! 3. `n8n.activate_workflow`
//!
//! The tool description flags this pattern.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::n8n_client::{N8nClientError, SharedN8nClient};

pub struct N8nUpdateWorkflow {
    id: ToolId,
    schema: Value,
    client: SharedN8nClient,
}

impl N8nUpdateWorkflow {
    pub fn new(client: SharedN8nClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for N8nUpdateWorkflow {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "n8n.update_workflow"
    }

    fn description(&self) -> &str {
        "Replace an existing n8n workflow's definition. \
         Input is a JSON object with required `id` \
         (workflow id from `n8n.list_workflows`), \
         `name` (string), `nodes` (array), \
         `connections` (object), `settings` (object), \
         and optional `staticData` (object). Treated by \
         n8n as a **full replacement** — read the \
         current definition via `n8n.get_workflow`, \
         mutate the relevant fields, and send the whole \
         thing back. Updating an **active** workflow \
         takes effect immediately on the next trigger; \
         for staged rollout, deactivate → update → \
         activate. Requires Trusted-tier capability \
         grant for `n8n.write`."
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
                    detail: format!("n8n.update_workflow: {reason}"),
                });
            }
        };
        let path = format!("/workflows/{workflow_id}");
        let resp: Value = match self.client.put_json(&path, &body).await {
            Ok(v) => v,
            Err(N8nClientError::NotFound) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!(
                        "n8n.update_workflow: no workflow with id `{workflow_id}` on this instance"
                    ),
                });
            }
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("n8n.update_workflow: API call failed: {e}"),
                });
            }
        };
        let output = json!({
            "id": resp.get("id").cloned().unwrap_or(Value::String(workflow_id)),
            "name": resp.get("name").cloned().unwrap_or(Value::Null),
            "active": resp.get("active").cloned().unwrap_or(Value::Bool(false)),
            "updated_at": resp.get("updatedAt").cloned().unwrap_or(Value::Null),
        });
        ToolOutcome::Completed {
            output,
            verified: Verification::Verified,
        }
    }
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
    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| "input must include a `name` string field".to_string())?;
    if name.is_empty() {
        return Err("`name` must not be empty".to_string());
    }
    let nodes = obj
        .get("nodes")
        .ok_or_else(|| "input must include `nodes` (array)".to_string())?;
    if !nodes.is_array() {
        return Err("`nodes` must be an array".to_string());
    }
    let connections = obj
        .get("connections")
        .ok_or_else(|| "input must include `connections` (object)".to_string())?;
    if !connections.is_object() {
        return Err("`connections` must be an object".to_string());
    }
    let settings = obj
        .get("settings")
        .ok_or_else(|| "input must include `settings` (object)".to_string())?;
    if !settings.is_object() {
        return Err("`settings` must be an object".to_string());
    }
    let mut body = serde_json::Map::new();
    body.insert("name".to_string(), Value::String(name.to_string()));
    body.insert("nodes".to_string(), nodes.clone());
    body.insert("connections".to_string(), connections.clone());
    body.insert("settings".to_string(), settings.clone());
    if let Some(sd) = obj.get("staticData") {
        if !sd.is_object() && !sd.is_null() {
            return Err("`staticData`, if set, must be an object".to_string());
        }
        if !sd.is_null() {
            body.insert("staticData".to_string(), sd.clone());
        }
    }
    Ok((id.to_string(), Value::Object(body)))
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": {"type": "string"},
            "name": {"type": "string"},
            "nodes": {"type": "array"},
            "connections": {"type": "object"},
            "settings": {"type": "object"},
            "staticData": {"type": ["object", "null"]}
        },
        "required": ["id", "name", "nodes", "connections", "settings"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_input() -> Value {
        json!({
            "id": "wf-1",
            "name": "Updated",
            "nodes": [],
            "connections": {},
            "settings": {}
        })
    }

    #[test]
    fn parse_input_accepts_minimal() {
        let (id, body) = parse_input(&minimal_input()).expect("parse");
        assert_eq!(id, "wf-1");
        assert_eq!(body["name"], "Updated");
    }

    #[test]
    fn parse_input_rejects_missing_id() {
        let mut input = minimal_input();
        input.as_object_mut().unwrap().remove("id");
        let e = parse_input(&input).expect_err("must error");
        assert!(e.contains("id"), "{e}");
    }

    #[test]
    fn parse_input_rejects_empty_id() {
        let mut input = minimal_input();
        input["id"] = json!("   ");
        let e = parse_input(&input).expect_err("must error");
        assert!(e.contains("id"), "{e}");
    }

    #[test]
    fn parse_input_rejects_missing_name() {
        let mut input = minimal_input();
        input.as_object_mut().unwrap().remove("name");
        let e = parse_input(&input).expect_err("must error");
        assert!(e.contains("name"), "{e}");
    }

    #[test]
    fn parse_input_rejects_non_array_nodes() {
        let mut input = minimal_input();
        input["nodes"] = json!({});
        let e = parse_input(&input).expect_err("must error");
        assert!(e.contains("nodes"), "{e}");
    }

    #[test]
    fn parse_input_id_excluded_from_body() {
        let (_, body) = parse_input(&minimal_input()).unwrap();
        assert!(
            body.get("id").is_none(),
            "id is path-routed; must not be in the PUT body"
        );
    }

    #[test]
    fn parse_input_static_data_passthrough() {
        let mut input = minimal_input();
        input["staticData"] = json!({"a": 1});
        let (_, body) = parse_input(&input).unwrap();
        assert_eq!(body["staticData"], json!({"a": 1}));
    }

    #[test]
    fn input_schema_marks_five_required() {
        let schema = input_schema();
        let req = schema["required"].as_array().expect("required");
        assert_eq!(req.len(), 5);
    }

    fn make_tool() -> N8nUpdateWorkflow {
        use crate::{N8nClient, N8nConfig};
        use std::sync::Arc;
        let client = Arc::new(N8nClient::new(
            reqwest::Client::new(),
            N8nConfig::new("https://n8n.example.com", "ntn_x"),
        ));
        N8nUpdateWorkflow::new(client)
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
        assert_eq!(make_tool().name(), "n8n.update_workflow");
    }
}
