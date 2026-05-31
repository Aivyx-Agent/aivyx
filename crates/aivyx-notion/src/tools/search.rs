//! `notion.search` — global search across shared content.
//!
//! Phase 130 Task 3. First Notion tool. Implements
//! [`aivyx_core::Tool`]; served by the lifted multi-tool
//! harness.
//!
//! ## API call
//!
//! POST `/search` with a JSON body containing the
//! optional `query` string, `filter` (page or database),
//! `sort`, `page_size`, and `start_cursor` (cursor-based
//! pagination — different naming from Drive/Calendar's
//! `next_page_token`).
//!
//! ## Output flattening
//!
//! Notion's search returns a heterogeneous results array
//! mixing pages and databases. Each has different
//! property shapes; we flatten to a stable LLM-friendly
//! shape: `{id, type, title, last_edited_at, url,
//! parent_type}`. The original Notion object is NOT
//! returned (operators wanting the full payload pass the
//! id to `notion.get_page` Task 4).

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::notion_client::SharedNotionClient;

const MAX_PAGE_SIZE: u64 = 100;
const DEFAULT_PAGE_SIZE: u64 = 25;

pub struct NotionSearch {
    id: ToolId,
    schema: Value,
    client: SharedNotionClient,
}

impl NotionSearch {
    pub fn new(client: SharedNotionClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for NotionSearch {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "notion.search"
    }

    fn description(&self) -> &str {
        "Search Notion content shared with the integration. \
         Input is a JSON object with optional `q` (the \
         search query string — Notion uses substring \
         matching on page/database titles), optional \
         `filter` (`\"page\"` or `\"database\"` to restrict \
         to one object type), optional `max_results` \
         (default 25, capped at 100), and optional \
         `start_cursor` (for pagination). Returns a JSON \
         object with `results` array of `{id, type, title, \
         last_edited_at, url, parent_type}` entries and an \
         optional `next_cursor` for pagination. Empty \
         results may mean the operator hasn't shared the \
         relevant pages with the integration via Notion's \
         UI (Share → Invite → select the integration)."
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
                    detail: format!("notion.search: {reason}"),
                });
            }
        };

        let body = build_search_body(&parsed);

        let resp: Value = match self.client.post_json("/search", &body).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("notion.search: API call failed: {e}"),
                });
            }
        };

        let results: Vec<Value> = resp
            .get("results")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().map(flatten_result).collect())
            .unwrap_or_default();
        let next_cursor = resp
            .get("next_cursor")
            .cloned()
            .unwrap_or(Value::Null);

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

/// Flatten one search result (page OR database) to a
/// stable LLM-friendly shape. Title extraction differs
/// between pages (look in `properties` for the title-typed
/// property) and databases (look in the top-level `title`
/// array).
pub(crate) fn flatten_result(item: &Value) -> Value {
    let object_type = item
        .get("object")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let id = item.get("id").cloned().unwrap_or(Value::Null);
    let last_edited_at = item.get("last_edited_time").cloned().unwrap_or(Value::Null);
    let url = item.get("url").cloned().unwrap_or(Value::Null);
    let parent_type = item
        .get("parent")
        .and_then(|p| p.get("type"))
        .cloned()
        .unwrap_or(Value::Null);

    let title = extract_title(item, object_type);

    json!({
        "id": id,
        "type": object_type,
        "title": title,
        "last_edited_at": last_edited_at,
        "url": url,
        "parent_type": parent_type,
    })
}

/// Extract a plain-text title from a page or database
/// object. Returns null when no title is set.
fn extract_title(item: &Value, object_type: &str) -> Value {
    match object_type {
        "database" => {
            // Databases have a top-level `title` array of
            // rich-text objects.
            collapse_rich_text(item.get("title"))
        }
        "page" => {
            // Pages have a `properties` object; one of the
            // properties has `type: "title"` whose value
            // is a rich-text array. Walk properties to find
            // it.
            let Some(props) = item.get("properties").and_then(|v| v.as_object()) else {
                return Value::Null;
            };
            for (_, prop) in props {
                if prop.get("type").and_then(|v| v.as_str()) == Some("title") {
                    return collapse_rich_text(prop.get("title"));
                }
            }
            Value::Null
        }
        _ => Value::Null,
    }
}

/// Collapse Notion's rich-text array (a list of objects
/// each with a `plain_text` field) to a single string.
/// Returns null for absent / empty arrays.
pub(crate) fn collapse_rich_text(rt: Option<&Value>) -> Value {
    let Some(arr) = rt.and_then(|v| v.as_array()) else {
        return Value::Null;
    };
    if arr.is_empty() {
        return Value::Null;
    }
    let combined: String = arr
        .iter()
        .filter_map(|item| item.get("plain_text").and_then(|v| v.as_str()))
        .collect::<Vec<_>>()
        .join("");
    if combined.is_empty() {
        Value::Null
    } else {
        Value::String(combined)
    }
}

pub(crate) fn build_search_body(parsed: &ParsedInput) -> Value {
    let mut body = serde_json::Map::new();
    if let Some(ref q) = parsed.q {
        body.insert("query".to_string(), Value::String(q.clone()));
    }
    if let Some(ref filter_kind) = parsed.filter {
        body.insert(
            "filter".to_string(),
            json!({"value": filter_kind, "property": "object"}),
        );
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
    pub(crate) q: Option<String>,
    pub(crate) filter: Option<String>,
    pub(crate) max_results: u64,
    pub(crate) start_cursor: Option<String>,
}

fn parse_input(input: &Value) -> Result<ParsedInput, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
    let q = match obj.get("q") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        Some(_) => return Err("`q` must be a string".to_string()),
    };
    let filter = match obj.get("filter") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let v = s.trim();
            if v.is_empty() {
                None
            } else if v == "page" || v == "database" {
                Some(v.to_string())
            } else {
                return Err(format!(
                    "`filter` must be `\"page\"` or `\"database\"`; got `{v}`"
                ));
            }
        }
        Some(_) => return Err("`filter` must be a string".to_string()),
    };
    let max_results = match obj.get("max_results") {
        None => DEFAULT_PAGE_SIZE,
        Some(v) => v.as_u64().ok_or_else(|| {
            "`max_results` must be a non-negative integer".to_string()
        })?,
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
        q,
        filter,
        max_results,
        start_cursor,
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "q": {
                "type": ["string", "null"],
                "description": "Search query (substring match on page/database titles). Optional."
            },
            "filter": {
                "type": ["string", "null"],
                "enum": ["page", "database", null],
                "description": "Restrict results to one object type. Optional."
            },
            "max_results": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_PAGE_SIZE,
                "default": DEFAULT_PAGE_SIZE,
                "description": "Maximum results to return. Capped at 100."
            },
            "start_cursor": {
                "type": ["string", "null"],
                "description": "Cursor from a previous response's `next_cursor` for pagination. Optional."
            }
        },
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_input_defaults_when_empty() {
        let p = parse_input(&json!({})).expect("parse");
        assert!(p.q.is_none());
        assert!(p.filter.is_none());
        assert_eq!(p.max_results, DEFAULT_PAGE_SIZE);
        assert!(p.start_cursor.is_none());
    }

    #[test]
    fn parse_input_accepts_query() {
        let p = parse_input(&json!({"q": "budget"})).expect("parse");
        assert_eq!(p.q.as_deref(), Some("budget"));
    }

    #[test]
    fn parse_input_rejects_unknown_filter() {
        let e = parse_input(&json!({"filter": "comment"})).expect_err("must error");
        assert!(e.contains("filter"), "{e}");
    }

    #[test]
    fn parse_input_accepts_page_and_database_filter() {
        assert_eq!(
            parse_input(&json!({"filter": "page"}))
                .unwrap()
                .filter
                .as_deref(),
            Some("page")
        );
        assert_eq!(
            parse_input(&json!({"filter": "database"}))
                .unwrap()
                .filter
                .as_deref(),
            Some("database")
        );
    }

    #[test]
    fn parse_input_caps_max_results() {
        let p = parse_input(&json!({"max_results": 9999})).expect("parse");
        assert_eq!(p.max_results, MAX_PAGE_SIZE);
    }

    #[test]
    fn parse_input_rejects_zero_max_results() {
        let e = parse_input(&json!({"max_results": 0})).expect_err("must error");
        assert!(e.contains(">= 1"), "{e}");
    }

    #[test]
    fn build_search_body_omits_query_when_absent() {
        let p = parse_input(&json!({})).unwrap();
        let body = build_search_body(&p);
        assert!(body.get("query").is_none());
    }

    #[test]
    fn build_search_body_emits_query_when_present() {
        let p = parse_input(&json!({"q": "Q4"})).unwrap();
        let body = build_search_body(&p);
        assert_eq!(body["query"], "Q4");
    }

    #[test]
    fn build_search_body_emits_filter_with_object_property() {
        let p = parse_input(&json!({"filter": "page"})).unwrap();
        let body = build_search_body(&p);
        assert_eq!(body["filter"]["value"], "page");
        assert_eq!(body["filter"]["property"], "object");
    }

    #[test]
    fn build_search_body_emits_page_size() {
        let p = parse_input(&json!({"max_results": 50})).unwrap();
        let body = build_search_body(&p);
        assert_eq!(body["page_size"], 50);
    }

    #[test]
    fn build_search_body_includes_start_cursor_when_present() {
        let p = parse_input(&json!({"start_cursor": "abc123"})).unwrap();
        let body = build_search_body(&p);
        assert_eq!(body["start_cursor"], "abc123");
    }

    #[test]
    fn collapse_rich_text_concatenates_plain_text_segments() {
        let rt = json!([
            {"plain_text": "Hello "},
            {"plain_text": "world"},
        ]);
        assert_eq!(collapse_rich_text(Some(&rt)), "Hello world");
    }

    #[test]
    fn collapse_rich_text_returns_null_for_empty_array() {
        let rt = json!([]);
        assert!(collapse_rich_text(Some(&rt)).is_null());
    }

    #[test]
    fn collapse_rich_text_returns_null_for_absent_field() {
        assert!(collapse_rich_text(None).is_null());
    }

    #[test]
    fn flatten_result_extracts_page_title_from_properties() {
        let page = json!({
            "object": "page",
            "id": "page-abc",
            "url": "https://www.notion.so/page-abc",
            "last_edited_time": "2026-06-01T12:00:00.000Z",
            "parent": {"type": "workspace"},
            "properties": {
                "Name": {
                    "type": "title",
                    "title": [{"plain_text": "Quarterly Budget"}]
                },
                "Status": {
                    "type": "select",
                    "select": {"name": "in-progress"}
                }
            }
        });
        let out = flatten_result(&page);
        assert_eq!(out["id"], "page-abc");
        assert_eq!(out["type"], "page");
        assert_eq!(out["title"], "Quarterly Budget");
        assert_eq!(out["parent_type"], "workspace");
        assert_eq!(out["url"], "https://www.notion.so/page-abc");
    }

    #[test]
    fn flatten_result_extracts_database_title_from_top_level() {
        let db = json!({
            "object": "database",
            "id": "db-xyz",
            "url": "https://www.notion.so/db-xyz",
            "last_edited_time": "2026-06-01T12:00:00.000Z",
            "parent": {"type": "page_id"},
            "title": [{"plain_text": "Projects"}]
        });
        let out = flatten_result(&db);
        assert_eq!(out["type"], "database");
        assert_eq!(out["title"], "Projects");
    }

    #[test]
    fn flatten_result_handles_page_with_no_title_property() {
        let page = json!({
            "object": "page",
            "id": "p",
            "properties": {
                "Status": {
                    "type": "select",
                    "select": {"name": "x"}
                }
            }
        });
        let out = flatten_result(&page);
        assert!(out["title"].is_null());
    }

    #[test]
    fn input_schema_declares_no_required_fields() {
        let schema = input_schema();
        let req = schema.get("required");
        assert!(req.is_none() || req.unwrap().as_array().unwrap().is_empty());
        assert_eq!(schema["additionalProperties"], false);
    }

    fn make_tool() -> NotionSearch {
        use crate::{NotionClient, NotionConfig};
        use std::sync::Arc;
        let client = Arc::new(NotionClient::new(
            reqwest::Client::new(),
            NotionConfig::new("ntn_x"),
        ));
        NotionSearch::new(client)
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
        assert_eq!(make_tool().name(), "notion.search");
    }
}
