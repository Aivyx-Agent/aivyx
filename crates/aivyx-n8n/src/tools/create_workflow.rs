//! `n8n.create_workflow` — create a new workflow from a
//! definition payload.
//!
//! Phase 131 Task 10. Fourth write tool. Trusted-gated.
//! Q1c risky CRUD pick.
//!
//! ## API call
//!
//! `POST /api/v1/workflows` with body
//! `{name, nodes, connections, settings, staticData?}`.
//! The four required fields are passed through verbatim
//! — n8n's node + connection schemas are too rich to
//! wrap in a thin Rust shape, so we forward operator-
//! supplied JSON and surface n8n's validation errors as
//! API failures.
//!
//! ## Blast radius
//!
//! This is the **highest blast radius** tool in the
//! Chapter F surface. A created workflow can:
//! - call arbitrary external APIs the operator's n8n
//!   instance can reach,
//! - bind to credentials the operator has stored,
//! - schedule recurring execution (if activated),
//! - mutate state across every downstream service the
//!   operator integrates.
//!
//! Trusted-tier ceiling is mandatory by default; the
//! Phase 131 entry doc flags this as the surface
//! operators are most likely to attenuate further via
//! role `capability_scopes` (e.g.,
//! `n8n.write:wf-readonly-templates`).
//!
//! ## Inactive on create
//!
//! Newly created workflows are inactive by n8n's
//! default — the operator must explicitly call
//! `n8n.activate_workflow` to wire up triggers. This
//! is a deliberate safety property: even if the LLM
//! creates an unwanted workflow, it does not start
//! firing until activation, giving the operator a
//! second checkpoint.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::n8n_client::SharedN8nClient;

pub struct N8nCreateWorkflow {
    id: ToolId,
    schema: Value,
    client: SharedN8nClient,
}

impl N8nCreateWorkflow {
    pub fn new(client: SharedN8nClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for N8nCreateWorkflow {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "n8n.create_workflow"
    }

    fn description(&self) -> &str {
        "Create a new n8n workflow from a definition \
         payload. Input is a JSON object with required \
         `name` (string), `nodes` (array of n8n node \
         definitions), `connections` (n8n connection \
         object), `settings` (n8n settings object), and \
         optional `staticData` (object). Returns \
         `{id, name, active}` — the new workflow is \
         **inactive by default**; call \
         `n8n.activate_workflow` separately to wire up \
         triggers. Highest blast radius tool in the \
         surface (a workflow definition can call \
         arbitrary HTTP, bind credentials, and schedule \
         recurring execution); requires Trusted-tier \
         capability grant for `n8n.write` and operators \
         may further attenuate via role \
         `capability_scopes`."
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
        let body = match parse_input(&input) {
            Ok(b) => b,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("n8n.create_workflow: {reason}"),
                });
            }
        };
        let resp: Value = match self.client.post_json("/workflows", &body).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("n8n.create_workflow: API call failed: {e}"),
                });
            }
        };
        let output = json!({
            "id": resp.get("id").cloned().unwrap_or(Value::Null),
            "name": resp.get("name").cloned().unwrap_or(Value::Null),
            "active": resp.get("active").cloned().unwrap_or(Value::Bool(false)),
        });
        ToolOutcome::Completed {
            output,
            verified: Verification::Verified,
        }
    }
}

pub(crate) fn parse_input(input: &Value) -> Result<Value, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
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
    Ok(Value::Object(body))
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": {"type": "string"},
            "nodes": {"type": "array"},
            "connections": {"type": "object"},
            "settings": {"type": "object"},
            "staticData": {"type": ["object", "null"]}
        },
        "required": ["name", "nodes", "connections", "settings"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_input() -> Value {
        json!({
            "name": "Test Workflow",
            "nodes": [],
            "connections": {},
            "settings": {}
        })
    }

    #[test]
    fn parse_input_accepts_minimal_valid() {
        let body = parse_input(&minimal_input()).expect("parse");
        assert_eq!(body["name"], "Test Workflow");
        assert!(body["nodes"].is_array());
        assert!(body["connections"].is_object());
        assert!(body["settings"].is_object());
        assert!(body.get("staticData").is_none());
    }

    #[test]
    fn parse_input_passes_through_static_data_object() {
        let mut input = minimal_input();
        input["staticData"] = json!({"key": "val"});
        let body = parse_input(&input).expect("parse");
        assert_eq!(body["staticData"], json!({"key": "val"}));
    }

    #[test]
    fn parse_input_drops_null_static_data() {
        let mut input = minimal_input();
        input["staticData"] = Value::Null;
        let body = parse_input(&input).expect("parse");
        assert!(body.get("staticData").is_none());
    }

    #[test]
    fn parse_input_rejects_missing_name() {
        let mut input = minimal_input();
        input.as_object_mut().unwrap().remove("name");
        let e = parse_input(&input).expect_err("must error");
        assert!(e.contains("name"), "{e}");
    }

    #[test]
    fn parse_input_rejects_empty_name() {
        let mut input = minimal_input();
        input["name"] = json!("   ");
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
    fn parse_input_rejects_non_object_connections() {
        let mut input = minimal_input();
        input["connections"] = json!([]);
        let e = parse_input(&input).expect_err("must error");
        assert!(e.contains("connections"), "{e}");
    }

    #[test]
    fn parse_input_rejects_non_object_settings() {
        let mut input = minimal_input();
        input["settings"] = json!("nope");
        let e = parse_input(&input).expect_err("must error");
        assert!(e.contains("settings"), "{e}");
    }

    #[test]
    fn parse_input_rejects_non_object_static_data() {
        let mut input = minimal_input();
        input["staticData"] = json!([1, 2]);
        let e = parse_input(&input).expect_err("must error");
        assert!(e.contains("staticData"), "{e}");
    }

    #[test]
    fn input_schema_marks_four_required() {
        let schema = input_schema();
        let req = schema["required"].as_array().expect("required");
        assert_eq!(req.len(), 4);
        for f in ["name", "nodes", "connections", "settings"] {
            assert!(req.iter().any(|v| v == f), "missing {f}");
        }
    }

    fn make_tool() -> N8nCreateWorkflow {
        use crate::{N8nClient, N8nConfig};
        use std::sync::Arc;
        let client = Arc::new(N8nClient::new(
            reqwest::Client::new(),
            N8nConfig::new("https://n8n.example.com", "ntn_x"),
        ));
        N8nCreateWorkflow::new(client)
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
        assert_eq!(make_tool().name(), "n8n.create_workflow");
    }
}
