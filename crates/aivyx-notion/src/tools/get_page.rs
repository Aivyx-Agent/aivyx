//! `notion.get_page` — fetch page properties + block
//! tree.
//!
//! Phase 130 Task 4. Second Notion tool.
//!
//! ## Two API calls
//!
//! 1. GET `/pages/{page_id}` — page properties + parent
//!    metadata.
//! 2. GET `/blocks/{page_id}/children?page_size=100` —
//!    top-level block tree (paragraphs, headings, list
//!    items, etc).
//!
//! Nested blocks (children of toggles, callouts, columns,
//! etc) are NOT recursively fetched — operators wanting
//! deep traversal call get_page recursively on each
//! `has_children: true` block. Documented in the tool
//! description.
//!
//! ## Block flattening
//!
//! Notion's block tree is heterogeneous (~30 block types
//! with type-specific shapes). For LLM-friendliness each
//! block flattens to:
//! `{id, type, plain_text, has_children}` where plain_text
//! is the collapsed rich-text content for text-bearing
//! blocks and null otherwise.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::notion_client::SharedNotionClient;

pub struct NotionGetPage {
    id: ToolId,
    schema: Value,
    client: SharedNotionClient,
}

impl NotionGetPage {
    pub fn new(client: SharedNotionClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for NotionGetPage {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "notion.get_page"
    }

    // Chapter Picket follow-up (Finding 3) — page content is
    // editable by any workspace collaborator; externally authored.
    fn output_is_untrusted(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Fetch a Notion page's properties + top-level block \
         tree. Input is a JSON object with a required \
         `page_id` (string; typically from `notion.search`). \
         Returns a JSON object with the page's `id`, `url`, \
         `title`, `properties` (flattened to a map of \
         property name → flat value), `created_at`, \
         `last_edited_at`, `parent_type`, `archived`, and \
         `blocks` array of `{id, type, plain_text, \
         has_children}`. Nested blocks (children of \
         toggles, callouts, etc) are NOT recursively \
         fetched — operators wanting deep traversal call \
         get_page on each `has_children: true` block. If \
         the response is 404 with `object_not_found`, the \
         integration likely doesn't have access — share \
         the page via Notion's UI."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("notion.read").expect(
            "notion.read must parse — it is in KNOWN_BASES from Phase 130",
        )
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let parsed = match parse_input(&input) {
            Ok(p) => p,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("notion.get_page: {reason}"),
                });
            }
        };

        let page_path = format!("/pages/{}", parsed.page_id);
        let page_body: Value = match self.client.get_json(&page_path, &[]).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("notion.get_page: page fetch failed: {e}"),
                });
            }
        };

        let blocks_path = format!("/blocks/{}/children", parsed.page_id);
        let blocks_resp: Value = match self
            .client
            .get_json(&blocks_path, &[("page_size", "100".to_string())])
            .await
        {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("notion.get_page: blocks fetch failed: {e}"),
                });
            }
        };

        let output = transform_page(&page_body, &blocks_resp);

        ToolOutcome::Completed {
            output,
            verified: Verification::NotApplicable,
        }
    }
}

pub(crate) fn transform_page(page: &Value, blocks_resp: &Value) -> Value {
    let id = page.get("id").cloned().unwrap_or(Value::Null);
    let url = page.get("url").cloned().unwrap_or(Value::Null);
    let created_at = page.get("created_time").cloned().unwrap_or(Value::Null);
    let last_edited_at = page.get("last_edited_time").cloned().unwrap_or(Value::Null);
    let parent_type = page
        .get("parent")
        .and_then(|p| p.get("type"))
        .cloned()
        .unwrap_or(Value::Null);
    let archived = page
        .get("archived")
        .cloned()
        .unwrap_or(Value::Bool(false));

    let title = super::search::extract_title_for_page(page);
    let properties = flatten_properties(page.get("properties"));

    let blocks: Vec<Value> = blocks_resp
        .get("results")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().map(flatten_block).collect())
        .unwrap_or_default();

    json!({
        "id": id,
        "url": url,
        "title": title,
        "properties": properties,
        "blocks": blocks,
        "created_at": created_at,
        "last_edited_at": last_edited_at,
        "parent_type": parent_type,
        "archived": archived,
    })
}

/// Flatten Notion's `properties` map (keyed by property
/// name; values are objects with a `type` discriminator
/// plus type-keyed inner) to a flat `{name: value}` shape
/// the LLM can consume.
pub(crate) fn flatten_properties(props: Option<&Value>) -> Value {
    let Some(obj) = props.and_then(|v| v.as_object()) else {
        return Value::Object(serde_json::Map::new());
    };
    let mut out = serde_json::Map::new();
    for (name, prop) in obj {
        out.insert(name.clone(), flatten_property_value(prop));
    }
    Value::Object(out)
}

/// Flatten one property value. Handles the common Notion
/// property types: title, rich_text, select, multi_select,
/// number, date, checkbox, email, url, phone_number, status,
/// people. Other types fall back to a null with a `type` hint.
pub(crate) fn flatten_property_value(prop: &Value) -> Value {
    let Some(kind) = prop.get("type").and_then(|v| v.as_str()) else {
        return Value::Null;
    };
    match kind {
        "title" | "rich_text" => super::search::collapse_rich_text(prop.get(kind)),
        "select" => prop
            .get("select")
            .and_then(|s| s.get("name"))
            .cloned()
            .unwrap_or(Value::Null),
        "status" => prop
            .get("status")
            .and_then(|s| s.get("name"))
            .cloned()
            .unwrap_or(Value::Null),
        "multi_select" => prop
            .get("multi_select")
            .and_then(|v| v.as_array())
            .map(|arr| {
                Value::Array(
                    arr.iter()
                        .filter_map(|i| i.get("name").cloned())
                        .collect(),
                )
            })
            .unwrap_or_else(|| Value::Array(Vec::new())),
        "number" => prop.get("number").cloned().unwrap_or(Value::Null),
        "checkbox" => prop.get("checkbox").cloned().unwrap_or(Value::Bool(false)),
        "url" => prop.get("url").cloned().unwrap_or(Value::Null),
        "email" => prop.get("email").cloned().unwrap_or(Value::Null),
        "phone_number" => prop.get("phone_number").cloned().unwrap_or(Value::Null),
        "date" => prop
            .get("date")
            .and_then(|d| d.get("start"))
            .cloned()
            .unwrap_or(Value::Null),
        "people" => prop
            .get("people")
            .and_then(|v| v.as_array())
            .map(|arr| {
                Value::Array(
                    arr.iter()
                        .filter_map(|p| {
                            p.get("name")
                                .or_else(|| p.get("id"))
                                .cloned()
                        })
                        .collect(),
                )
            })
            .unwrap_or_else(|| Value::Array(Vec::new())),
        _ => Value::Null,
    }
}

/// Flatten one block to the LLM-friendly shape:
/// `{id, type, plain_text, has_children}`. For
/// text-bearing block types (paragraph, headings, list
/// items, etc) `plain_text` is the collapsed rich-text;
/// for non-text blocks (divider, image, table, etc) it's
/// null.
pub(crate) fn flatten_block(block: &Value) -> Value {
    let id = block.get("id").cloned().unwrap_or(Value::Null);
    let kind = block
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let has_children = block
        .get("has_children")
        .cloned()
        .unwrap_or(Value::Bool(false));
    let plain_text = extract_block_plain_text(block, &kind);
    json!({
        "id": id,
        "type": kind,
        "plain_text": plain_text,
        "has_children": has_children,
    })
}

fn extract_block_plain_text(block: &Value, kind: &str) -> Value {
    // Text-bearing block types put their rich-text array
    // at `{block}.<type>.rich_text`.
    let text_bearing = matches!(
        kind,
        "paragraph"
            | "heading_1"
            | "heading_2"
            | "heading_3"
            | "bulleted_list_item"
            | "numbered_list_item"
            | "to_do"
            | "toggle"
            | "quote"
            | "callout"
            | "code"
    );
    if !text_bearing {
        return Value::Null;
    }
    let Some(inner) = block.get(kind) else {
        return Value::Null;
    };
    super::search::collapse_rich_text(inner.get("rich_text"))
}

#[derive(Debug)]
struct ParsedInput {
    page_id: String,
}

fn parse_input(input: &Value) -> Result<ParsedInput, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
    let page_id = obj
        .get("page_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| "input must include a `page_id` string field".to_string())?;
    if page_id.is_empty() {
        return Err("`page_id` must not be empty".to_string());
    }
    Ok(ParsedInput {
        page_id: page_id.to_string(),
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "page_id": {
                "type": "string",
                "description": "Notion page ID. Required."
            }
        },
        "required": ["page_id"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notion_get_page_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }

    #[test]
    fn parse_input_accepts_minimal() {
        let p = parse_input(&json!({"page_id": "abc"})).expect("parse");
        assert_eq!(p.page_id, "abc");
    }

    #[test]
    fn parse_input_rejects_missing_page_id() {
        let e = parse_input(&json!({})).expect_err("must error");
        assert!(e.contains("page_id"), "{e}");
    }

    #[test]
    fn parse_input_rejects_empty_page_id() {
        let e = parse_input(&json!({"page_id": "   "})).expect_err("must error");
        assert!(e.contains("page_id"), "{e}");
    }

    #[test]
    fn flatten_property_value_handles_title() {
        let prop = json!({
            "type": "title",
            "title": [{"plain_text": "My Page"}]
        });
        assert_eq!(flatten_property_value(&prop), "My Page");
    }

    #[test]
    fn flatten_property_value_handles_rich_text() {
        let prop = json!({
            "type": "rich_text",
            "rich_text": [
                {"plain_text": "Hello "},
                {"plain_text": "world"}
            ]
        });
        assert_eq!(flatten_property_value(&prop), "Hello world");
    }

    #[test]
    fn flatten_property_value_handles_select() {
        let prop = json!({"type": "select", "select": {"name": "in-progress"}});
        assert_eq!(flatten_property_value(&prop), "in-progress");
    }

    #[test]
    fn flatten_property_value_handles_status() {
        let prop = json!({"type": "status", "status": {"name": "Done"}});
        assert_eq!(flatten_property_value(&prop), "Done");
    }

    #[test]
    fn flatten_property_value_handles_multi_select() {
        let prop = json!({
            "type": "multi_select",
            "multi_select": [{"name": "a"}, {"name": "b"}]
        });
        assert_eq!(flatten_property_value(&prop), json!(["a", "b"]));
    }

    #[test]
    fn flatten_property_value_handles_number() {
        let prop = json!({"type": "number", "number": 42.5});
        assert_eq!(flatten_property_value(&prop), 42.5);
    }

    #[test]
    fn flatten_property_value_handles_checkbox() {
        let prop = json!({"type": "checkbox", "checkbox": true});
        assert_eq!(flatten_property_value(&prop), true);
    }

    #[test]
    fn flatten_property_value_handles_date_with_start() {
        let prop = json!({
            "type": "date",
            "date": {"start": "2026-06-15", "end": null}
        });
        assert_eq!(flatten_property_value(&prop), "2026-06-15");
    }

    #[test]
    fn flatten_property_value_handles_url() {
        let prop = json!({"type": "url", "url": "https://example.com"});
        assert_eq!(flatten_property_value(&prop), "https://example.com");
    }

    #[test]
    fn flatten_property_value_returns_null_for_unknown_type() {
        let prop = json!({"type": "formula", "formula": {"value": "complex"}});
        assert!(flatten_property_value(&prop).is_null());
    }

    #[test]
    fn flatten_block_extracts_paragraph_text() {
        let block = json!({
            "id": "b1",
            "type": "paragraph",
            "paragraph": {
                "rich_text": [{"plain_text": "Hello world"}]
            },
            "has_children": false
        });
        let out = flatten_block(&block);
        assert_eq!(out["id"], "b1");
        assert_eq!(out["type"], "paragraph");
        assert_eq!(out["plain_text"], "Hello world");
        assert_eq!(out["has_children"], false);
    }

    #[test]
    fn flatten_block_extracts_heading_text() {
        let block = json!({
            "id": "h1",
            "type": "heading_1",
            "heading_1": {
                "rich_text": [{"plain_text": "Section Title"}]
            },
            "has_children": false
        });
        assert_eq!(flatten_block(&block)["plain_text"], "Section Title");
    }

    #[test]
    fn flatten_block_extracts_to_do_text() {
        let block = json!({
            "id": "t1",
            "type": "to_do",
            "to_do": {
                "rich_text": [{"plain_text": "Buy milk"}],
                "checked": false
            },
            "has_children": false
        });
        assert_eq!(flatten_block(&block)["plain_text"], "Buy milk");
    }

    #[test]
    fn flatten_block_null_text_for_divider() {
        let block = json!({
            "id": "d1",
            "type": "divider",
            "divider": {},
            "has_children": false
        });
        assert!(flatten_block(&block)["plain_text"].is_null());
    }

    #[test]
    fn flatten_block_carries_has_children_flag() {
        let block = json!({
            "id": "tg1",
            "type": "toggle",
            "toggle": {"rich_text": [{"plain_text": "Click me"}]},
            "has_children": true
        });
        let out = flatten_block(&block);
        assert_eq!(out["has_children"], true);
        assert_eq!(out["plain_text"], "Click me");
    }

    #[test]
    fn transform_page_combines_metadata_and_blocks() {
        let page = json!({
            "id": "p1",
            "url": "https://www.notion.so/p1",
            "created_time": "2026-01-01T00:00:00.000Z",
            "last_edited_time": "2026-06-01T12:00:00.000Z",
            "parent": {"type": "workspace"},
            "archived": false,
            "properties": {
                "Name": {
                    "type": "title",
                    "title": [{"plain_text": "Project Alpha"}]
                },
                "Status": {
                    "type": "status",
                    "status": {"name": "Active"}
                }
            }
        });
        let blocks = json!({
            "results": [
                {
                    "id": "b1",
                    "type": "paragraph",
                    "paragraph": {"rich_text": [{"plain_text": "Intro paragraph."}]},
                    "has_children": false
                }
            ]
        });
        let out = transform_page(&page, &blocks);
        assert_eq!(out["id"], "p1");
        assert_eq!(out["title"], "Project Alpha");
        assert_eq!(out["properties"]["Status"], "Active");
        assert_eq!(out["blocks"][0]["plain_text"], "Intro paragraph.");
        assert_eq!(out["archived"], false);
    }

    #[test]
    fn transform_page_empty_blocks_yields_empty_array() {
        let page = json!({"id": "p", "properties": {}});
        let blocks = json!({"results": []});
        let out = transform_page(&page, &blocks);
        assert!(out["blocks"].as_array().unwrap().is_empty());
    }

    #[test]
    fn input_schema_declares_page_id_required() {
        let schema = input_schema();
        let req = schema["required"].as_array().expect("required");
        assert_eq!(req.len(), 1);
        assert_eq!(req[0], "page_id");
        assert_eq!(schema["additionalProperties"], false);
    }

    fn make_tool() -> NotionGetPage {
        use crate::{NotionClient, NotionConfig};
        use std::sync::Arc;
        let client = Arc::new(NotionClient::new(
            reqwest::Client::new(),
            NotionConfig::new("ntn_x"),
        ));
        NotionGetPage::new(client)
    }

    #[test]
    fn required_scope_is_notion_read() {
        assert_eq!(
            make_tool().required_scope(&json!({})).to_string(),
            "notion.read"
        );
    }

    #[test]
    fn tool_name_is_canonical() {
        assert_eq!(make_tool().name(), "notion.get_page");
    }
}
