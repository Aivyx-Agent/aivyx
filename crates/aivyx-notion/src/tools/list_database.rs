//! `notion.list_database` — query a Notion database.
//!
//! Phase 130 Task 5. Third Notion tool.
//!
//! ## API call
//!
//! POST `/databases/{database_id}/query` with optional
//! `filter`, `sorts`, `page_size`, `start_cursor`. The
//! filter and sorts shapes are operator-supplied verbatim
//! per Notion's API docs — Notion's filter DSL is too
//! rich to wrap in a thin schema, so we pass through and
//! surface Notion's error if the operator's filter is
//! malformed.
//!
//! ## Output
//!
//! Each result is a page; we flatten its properties via
//! `notion.get_page::flatten_properties` so the LLM sees
//! the same property-flattening shape across get_page and
//! list_database.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::notion_client::SharedNotionClient;

const MAX_PAGE_SIZE: u64 = 100;
const DEFAULT_PAGE_SIZE: u64 = 25;

pub struct NotionListDatabase {
    id: ToolId,
    schema: Value,
    client: SharedNotionClient,
}

impl NotionListDatabase {
    pub fn new(client: SharedNotionClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for NotionListDatabase {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "notion.list_database"
    }

    // Chapter Picket follow-up (Finding 3) — database entries are
    // editable by any workspace collaborator; externally authored.
    fn output_is_untrusted(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Query a Notion database for pages. Input is a JSON \
         object with a required `database_id`, optional \
         `filter` (Notion filter object passed verbatim — \
         see Notion API docs for the DSL), optional `sorts` \
         (Notion sort array), `max_results` (default 25, \
         capped at 100), and `start_cursor` for pagination. \
         Returns `{results, next_cursor}` where each result \
         is `{id, url, title, properties, last_edited_at}` \
         with `properties` flattened to a `{name: value}` \
         map matching `notion.get_page`'s property shape."
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
                    detail: format!("notion.list_database: {reason}"),
                });
            }
        };

        let body = build_query_body(&parsed);
        let path = format!("/databases/{}/query", parsed.database_id);

        let resp: Value = match self.client.post_json(&path, &body).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("notion.list_database: API call failed: {e}"),
                });
            }
        };

        let results: Vec<Value> = resp
            .get("results")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().map(flatten_db_result).collect())
            .unwrap_or_default();
        let next_cursor = resp.get("next_cursor").cloned().unwrap_or(Value::Null);

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

pub(crate) fn flatten_db_result(page: &Value) -> Value {
    let id = page.get("id").cloned().unwrap_or(Value::Null);
    let url = page.get("url").cloned().unwrap_or(Value::Null);
    let last_edited_at = page.get("last_edited_time").cloned().unwrap_or(Value::Null);
    let title = super::search::extract_title_for_page(page);
    let properties = super::get_page::flatten_properties(page.get("properties"));
    json!({
        "id": id,
        "url": url,
        "title": title,
        "properties": properties,
        "last_edited_at": last_edited_at,
    })
}

pub(crate) fn build_query_body(parsed: &ParsedInput) -> Value {
    let mut body = serde_json::Map::new();
    if let Some(ref f) = parsed.filter {
        body.insert("filter".to_string(), f.clone());
    }
    if let Some(ref s) = parsed.sorts {
        body.insert("sorts".to_string(), s.clone());
    }
    body.insert(
        "page_size".to_string(),
        Value::Number(parsed.max_results.into()),
    );
    if let Some(ref c) = parsed.start_cursor {
        body.insert("start_cursor".to_string(), Value::String(c.clone()));
    }
    Value::Object(body)
}

#[derive(Debug)]
pub(crate) struct ParsedInput {
    pub(crate) database_id: String,
    pub(crate) filter: Option<Value>,
    pub(crate) sorts: Option<Value>,
    pub(crate) max_results: u64,
    pub(crate) start_cursor: Option<String>,
}

fn parse_input(input: &Value) -> Result<ParsedInput, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
    let database_id = obj
        .get("database_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| "input must include a `database_id` string field".to_string())?;
    if database_id.is_empty() {
        return Err("`database_id` must not be empty".to_string());
    }
    let filter = match obj.get("filter") {
        None | Some(Value::Null) => None,
        Some(v) => {
            if !v.is_object() {
                return Err("`filter` must be a Notion filter object".to_string());
            }
            Some(v.clone())
        }
    };
    let sorts = match obj.get("sorts") {
        None | Some(Value::Null) => None,
        Some(v) => {
            if !v.is_array() {
                return Err("`sorts` must be an array".to_string());
            }
            Some(v.clone())
        }
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
    let start_cursor = match obj.get("start_cursor") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Some(_) => return Err("`start_cursor` must be a string".to_string()),
    };
    Ok(ParsedInput {
        database_id: database_id.to_string(),
        filter,
        sorts,
        max_results,
        start_cursor,
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "database_id": {
                "type": "string",
                "description": "Database ID. Required."
            },
            "filter": {
                "type": ["object", "null"],
                "description": "Notion filter object (passed verbatim — see Notion API docs). Optional."
            },
            "sorts": {
                "type": ["array", "null"],
                "description": "Notion sort array (passed verbatim). Optional."
            },
            "max_results": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_PAGE_SIZE,
                "default": DEFAULT_PAGE_SIZE
            },
            "start_cursor": {
                "type": ["string", "null"]
            }
        },
        "required": ["database_id"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notion_list_database_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }

    #[test]
    fn parse_input_accepts_minimal() {
        let p = parse_input(&json!({"database_id": "db1"})).expect("parse");
        assert_eq!(p.database_id, "db1");
        assert!(p.filter.is_none());
        assert!(p.sorts.is_none());
        assert_eq!(p.max_results, DEFAULT_PAGE_SIZE);
    }

    #[test]
    fn parse_input_rejects_missing_database_id() {
        let e = parse_input(&json!({})).expect_err("must error");
        assert!(e.contains("database_id"), "{e}");
    }

    #[test]
    fn parse_input_rejects_empty_database_id() {
        let e = parse_input(&json!({"database_id": "   "})).expect_err("must error");
        assert!(e.contains("database_id"), "{e}");
    }

    #[test]
    fn parse_input_accepts_filter_object() {
        let p = parse_input(&json!({
            "database_id": "db1",
            "filter": {"property": "Status", "select": {"equals": "Done"}}
        }))
        .expect("parse");
        assert!(p.filter.is_some());
    }

    #[test]
    fn parse_input_rejects_non_object_filter() {
        let e = parse_input(&json!({"database_id": "db1", "filter": "not-an-object"}))
            .expect_err("must error");
        assert!(e.contains("filter"), "{e}");
    }

    #[test]
    fn parse_input_accepts_sorts_array() {
        let p = parse_input(&json!({
            "database_id": "db1",
            "sorts": [{"property": "Name", "direction": "ascending"}]
        }))
        .expect("parse");
        assert!(p.sorts.is_some());
    }

    #[test]
    fn parse_input_rejects_non_array_sorts() {
        let e = parse_input(&json!({"database_id": "db1", "sorts": "asc"}))
            .expect_err("must error");
        assert!(e.contains("sorts"), "{e}");
    }

    #[test]
    fn parse_input_caps_max_results() {
        let p = parse_input(&json!({"database_id": "db1", "max_results": 9999}))
            .expect("parse");
        assert_eq!(p.max_results, MAX_PAGE_SIZE);
    }

    #[test]
    fn build_query_body_emits_filter_and_sorts_passthrough() {
        let p = parse_input(&json!({
            "database_id": "db1",
            "filter": {"property": "x", "select": {"equals": "y"}},
            "sorts": [{"property": "Name", "direction": "asc"}]
        }))
        .unwrap();
        let body = build_query_body(&p);
        assert!(body["filter"].is_object());
        assert!(body["sorts"].is_array());
        assert_eq!(body["page_size"], 25);
    }

    #[test]
    fn build_query_body_omits_optional_fields_when_absent() {
        let p = parse_input(&json!({"database_id": "db1"})).unwrap();
        let body = build_query_body(&p);
        assert!(body.get("filter").is_none());
        assert!(body.get("sorts").is_none());
        assert!(body.get("start_cursor").is_none());
    }

    #[test]
    fn flatten_db_result_shape_matches_get_page_property_flatten() {
        let page = json!({
            "id": "p1",
            "url": "https://www.notion.so/p1",
            "last_edited_time": "2026-06-01T00:00:00.000Z",
            "properties": {
                "Name": {
                    "type": "title",
                    "title": [{"plain_text": "Item A"}]
                },
                "Done": {
                    "type": "checkbox",
                    "checkbox": true
                }
            }
        });
        let out = flatten_db_result(&page);
        assert_eq!(out["id"], "p1");
        assert_eq!(out["title"], "Item A");
        assert_eq!(out["properties"]["Name"], "Item A");
        assert_eq!(out["properties"]["Done"], true);
    }

    #[test]
    fn input_schema_declares_database_id_required() {
        let schema = input_schema();
        let req = schema["required"].as_array().expect("required");
        assert_eq!(req.len(), 1);
        assert_eq!(req[0], "database_id");
    }

    fn make_tool() -> NotionListDatabase {
        use crate::{NotionClient, NotionConfig};
        use std::sync::Arc;
        let client = Arc::new(NotionClient::new(
            reqwest::Client::new(),
            NotionConfig::new("ntn_x"),
        ));
        NotionListDatabase::new(client)
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
        assert_eq!(make_tool().name(), "notion.list_database");
    }
}
