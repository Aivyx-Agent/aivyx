//! `notion.create_page` — create a new Notion page.
//!
//! Phase 130 Task 6. Fourth Notion tool; first write tool
//! (`notion.write` capability, Trusted-gated).
//!
//! ## API call
//!
//! POST `/pages` with `{parent, properties, [children]}`.
//! The parent is either `{"page_id": "..."}` (create
//! under a page) or `{"database_id": "..."}` (create as a
//! database row); properties must match the parent's
//! schema. The shapes are operator-supplied verbatim per
//! Notion's API docs — same posture as
//! `notion.list_database`'s filter passthrough. The tool
//! validates that exactly one of page_id/database_id is
//! present in parent and that properties is an object.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::notion_client::SharedNotionClient;

pub struct NotionCreatePage {
    id: ToolId,
    schema: Value,
    client: SharedNotionClient,
}

impl NotionCreatePage {
    pub fn new(client: SharedNotionClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for NotionCreatePage {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "notion.create_page"
    }

    fn description(&self) -> &str {
        "Create a new Notion page. Input is a JSON object \
         with required `parent` (object with EITHER \
         `page_id` OR `database_id`, not both), required \
         `properties` (per Notion's property shape — for \
         database rows must match the database schema; for \
         a page parent typically just \
         `{\"title\": [{\"text\": {\"content\": \"...\"}}]}`), \
         and optional `children` (array of block objects). \
         All shapes are passed verbatim per Notion's API \
         docs. Returns `{id, url, title}` of the created \
         page. Requires Trusted-tier capability grant for \
         `notion.write`."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("notion.write").expect(
            "notion.write must parse — it is in KNOWN_BASES from Phase 130",
        )
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let parsed = match parse_input(&input) {
            Ok(p) => p,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("notion.create_page: {reason}"),
                });
            }
        };

        let body = build_create_body(&parsed);

        let resp: Value = match self.client.post_json("/pages", &body).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("notion.create_page: API call failed: {e}"),
                });
            }
        };

        let title = super::search::extract_title_for_page(&resp);
        let output = json!({
            "id": resp.get("id").cloned().unwrap_or(Value::Null),
            "url": resp.get("url").cloned().unwrap_or(Value::Null),
            "title": title,
        });

        ToolOutcome::Completed {
            output,
            verified: Verification::Verified,
        }
    }
}

pub(crate) fn build_create_body(parsed: &ParsedInput) -> Value {
    let mut body = serde_json::Map::new();
    body.insert("parent".to_string(), parsed.parent.clone());
    body.insert("properties".to_string(), parsed.properties.clone());
    if let Some(ref c) = parsed.children {
        body.insert("children".to_string(), c.clone());
    }
    Value::Object(body)
}

#[derive(Debug)]
pub(crate) struct ParsedInput {
    pub(crate) parent: Value,
    pub(crate) properties: Value,
    pub(crate) children: Option<Value>,
}

fn parse_input(input: &Value) -> Result<ParsedInput, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
    let parent = obj
        .get("parent")
        .ok_or_else(|| "input must include a `parent` object".to_string())?
        .clone();
    validate_parent(&parent)?;
    let properties = obj
        .get("properties")
        .ok_or_else(|| "input must include a `properties` object".to_string())?
        .clone();
    if !properties.is_object() {
        return Err("`properties` must be a JSON object".to_string());
    }
    let children = match obj.get("children") {
        None | Some(Value::Null) => None,
        Some(v) => {
            if !v.is_array() {
                return Err("`children` must be an array of block objects".to_string());
            }
            Some(v.clone())
        }
    };
    Ok(ParsedInput {
        parent,
        properties,
        children,
    })
}

pub(crate) fn validate_parent(parent: &Value) -> Result<(), String> {
    let obj = parent
        .as_object()
        .ok_or_else(|| "`parent` must be an object".to_string())?;
    let has_page_id = obj
        .get("page_id")
        .map(|v| v.is_string() && !v.as_str().unwrap_or("").trim().is_empty())
        .unwrap_or(false);
    let has_db_id = obj
        .get("database_id")
        .map(|v| v.is_string() && !v.as_str().unwrap_or("").trim().is_empty())
        .unwrap_or(false);
    match (has_page_id, has_db_id) {
        (true, true) => Err(
            "`parent` must contain EITHER `page_id` OR `database_id`, not both"
                .to_string(),
        ),
        (false, false) => Err(
            "`parent` must contain `page_id` or `database_id` (non-empty string)"
                .to_string(),
        ),
        _ => Ok(()),
    }
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "parent": {
                "type": "object",
                "description": "Object with EXACTLY ONE of `page_id` or `database_id` (string)."
            },
            "properties": {
                "type": "object",
                "description": "Notion property shape (must match parent's schema for database parents). Passed verbatim per Notion's API."
            },
            "children": {
                "type": ["array", "null"],
                "description": "Optional initial block children. Notion block objects passed verbatim."
            }
        },
        "required": ["parent", "properties"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good_input() -> Value {
        json!({
            "parent": {"database_id": "db-xyz"},
            "properties": {
                "Name": {"title": [{"text": {"content": "Hi"}}]}
            }
        })
    }

    #[test]
    fn parse_input_accepts_database_parent() {
        let p = parse_input(&good_input()).expect("parse");
        assert_eq!(p.parent["database_id"], "db-xyz");
    }

    #[test]
    fn parse_input_accepts_page_parent() {
        let p = parse_input(&json!({
            "parent": {"page_id": "page-abc"},
            "properties": {"title": [{"text": {"content": "x"}}]}
        }))
        .expect("parse");
        assert_eq!(p.parent["page_id"], "page-abc");
    }

    #[test]
    fn parse_input_accepts_children_blocks() {
        let p = parse_input(&json!({
            "parent": {"page_id": "page"},
            "properties": {"title": [{"text": {"content": "x"}}]},
            "children": [
                {"object": "block", "type": "paragraph",
                 "paragraph": {"rich_text": [{"text": {"content": "Hi"}}]}}
            ]
        }))
        .expect("parse");
        assert!(p.children.is_some());
    }

    #[test]
    fn parse_input_rejects_both_parent_kinds() {
        let e = parse_input(&json!({
            "parent": {"page_id": "p", "database_id": "d"},
            "properties": {}
        }))
        .expect_err("must error");
        assert!(e.contains("EITHER"), "{e}");
    }

    #[test]
    fn parse_input_rejects_empty_parent() {
        let e = parse_input(&json!({"parent": {}, "properties": {}}))
            .expect_err("must error");
        assert!(e.contains("page_id"), "{e}");
        assert!(e.contains("database_id"), "{e}");
    }

    #[test]
    fn parse_input_rejects_non_string_parent_id() {
        let e = parse_input(&json!({
            "parent": {"page_id": 42},
            "properties": {}
        }))
        .expect_err("must error");
        assert!(e.contains("page_id") || e.contains("database_id"), "{e}");
    }

    #[test]
    fn parse_input_rejects_missing_parent() {
        let e = parse_input(&json!({"properties": {}})).expect_err("must error");
        assert!(e.contains("parent"), "{e}");
    }

    #[test]
    fn parse_input_rejects_missing_properties() {
        let e = parse_input(&json!({"parent": {"page_id": "p"}}))
            .expect_err("must error");
        assert!(e.contains("properties"), "{e}");
    }

    #[test]
    fn parse_input_rejects_non_object_properties() {
        let e = parse_input(&json!({
            "parent": {"page_id": "p"},
            "properties": "not-an-object"
        }))
        .expect_err("must error");
        assert!(e.contains("properties"), "{e}");
    }

    #[test]
    fn parse_input_rejects_non_array_children() {
        let e = parse_input(&json!({
            "parent": {"page_id": "p"},
            "properties": {"title": []},
            "children": {"not": "array"}
        }))
        .expect_err("must error");
        assert!(e.contains("children"), "{e}");
    }

    #[test]
    fn build_create_body_emits_parent_and_properties() {
        let p = parse_input(&good_input()).unwrap();
        let body = build_create_body(&p);
        assert_eq!(body["parent"]["database_id"], "db-xyz");
        assert!(body["properties"].is_object());
        assert!(body.get("children").is_none());
    }

    #[test]
    fn build_create_body_includes_children_when_present() {
        let p = parse_input(&json!({
            "parent": {"page_id": "p"},
            "properties": {"title": []},
            "children": [{"object": "block", "type": "paragraph",
                          "paragraph": {"rich_text": []}}]
        }))
        .unwrap();
        let body = build_create_body(&p);
        assert!(body["children"].is_array());
    }

    #[test]
    fn input_schema_declares_parent_and_properties_required() {
        let schema = input_schema();
        let req = schema["required"].as_array().expect("required");
        let req_strs: Vec<&str> = req.iter().filter_map(|v| v.as_str()).collect();
        assert!(req_strs.contains(&"parent"));
        assert!(req_strs.contains(&"properties"));
        assert_eq!(schema["additionalProperties"], false);
    }

    fn make_tool() -> NotionCreatePage {
        use crate::{NotionClient, NotionConfig};
        use std::sync::Arc;
        let client = Arc::new(NotionClient::new(
            reqwest::Client::new(),
            NotionConfig::new("ntn_x"),
        ));
        NotionCreatePage::new(client)
    }

    #[test]
    fn required_scope_is_notion_write() {
        assert_eq!(
            make_tool().required_scope(&json!({})).to_string(),
            "notion.write"
        );
    }

    #[test]
    fn tool_name_is_canonical() {
        assert_eq!(make_tool().name(), "notion.create_page");
    }
}
