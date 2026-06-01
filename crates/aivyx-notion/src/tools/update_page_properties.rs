//! `notion.update_page_properties` — patch property values
//! on an existing page.
//!
//! Phase 130 Task 8. Sixth Notion tool; third write tool.
//!
//! ## API call
//!
//! PATCH `/pages/{page_id}` with `{"properties": {...}}`.
//! Only the keys present in the request body are updated;
//! unmentioned properties keep their current value (Notion's
//! property-patch semantics). Property shapes are passed
//! verbatim per Notion's API docs.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::notion_client::SharedNotionClient;

pub struct NotionUpdatePageProperties {
    id: ToolId,
    schema: Value,
    client: SharedNotionClient,
}

impl NotionUpdatePageProperties {
    pub fn new(client: SharedNotionClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for NotionUpdatePageProperties {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "notion.update_page_properties"
    }

    fn description(&self) -> &str {
        "Update properties on an existing Notion page. Input \
         is a JSON object with required `page_id` and \
         required `properties` (object mapping property name \
         → Notion property value shape per Notion's API). \
         Only the keys present in the body are updated; \
         unmentioned properties keep their current value. \
         Returns `{id, last_edited_at, title}` of the \
         updated page. Requires Trusted-tier capability \
         grant for `notion.write`."
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
                    detail: format!("notion.update_page_properties: {reason}"),
                });
            }
        };

        let body = json!({"properties": parsed.properties.clone()});
        let path = format!("/pages/{}", parsed.page_id);

        let resp: Value = match self.client.patch_json(&path, &body).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("notion.update_page_properties: API call failed: {e}"),
                });
            }
        };

        let title = super::search::extract_title_for_page(&resp);
        let output = json!({
            "id": resp.get("id").cloned().unwrap_or(Value::Null),
            "last_edited_at": resp.get("last_edited_time").cloned().unwrap_or(Value::Null),
            "title": title,
        });

        ToolOutcome::Completed {
            output,
            verified: Verification::Verified,
        }
    }
}

#[derive(Debug)]
pub(crate) struct ParsedInput {
    pub(crate) page_id: String,
    pub(crate) properties: Value,
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
    let properties = obj
        .get("properties")
        .ok_or_else(|| "input must include a `properties` object".to_string())?
        .clone();
    let props_obj = properties
        .as_object()
        .ok_or_else(|| "`properties` must be a JSON object".to_string())?;
    if props_obj.is_empty() {
        return Err(
            "`properties` must not be empty — supply at least one property to update"
                .to_string(),
        );
    }
    Ok(ParsedInput {
        page_id: page_id.to_string(),
        properties,
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "page_id": {
                "type": "string",
                "description": "Page ID. Required."
            },
            "properties": {
                "type": "object",
                "description": "Property patches keyed by property name. Each value is a Notion property shape (passed verbatim per Notion's API).",
                "minProperties": 1
            }
        },
        "required": ["page_id", "properties"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good_input() -> Value {
        json!({
            "page_id": "p1",
            "properties": {
                "Status": {"status": {"name": "Done"}}
            }
        })
    }

    #[test]
    fn parse_input_accepts_minimal() {
        let p = parse_input(&good_input()).expect("parse");
        assert_eq!(p.page_id, "p1");
        assert!(p.properties.is_object());
    }

    #[test]
    fn parse_input_rejects_missing_page_id() {
        let e = parse_input(&json!({"properties": {"x": {}}})).expect_err("must error");
        assert!(e.contains("page_id"), "{e}");
    }

    #[test]
    fn parse_input_rejects_empty_page_id() {
        let e = parse_input(&json!({"page_id": "   ", "properties": {"x": {}}}))
            .expect_err("must error");
        assert!(e.contains("page_id"), "{e}");
    }

    #[test]
    fn parse_input_rejects_missing_properties() {
        let e = parse_input(&json!({"page_id": "p1"})).expect_err("must error");
        assert!(e.contains("properties"), "{e}");
    }

    #[test]
    fn parse_input_rejects_non_object_properties() {
        let e = parse_input(&json!({"page_id": "p1", "properties": []}))
            .expect_err("must error");
        assert!(e.contains("properties"), "{e}");
    }

    #[test]
    fn parse_input_rejects_empty_properties_object() {
        let e = parse_input(&json!({"page_id": "p1", "properties": {}}))
            .expect_err("must error");
        assert!(e.contains("must not be empty"), "{e}");
    }

    #[test]
    fn input_schema_declares_required_fields() {
        let schema = input_schema();
        let req = schema["required"].as_array().expect("required");
        let req_strs: Vec<&str> = req.iter().filter_map(|v| v.as_str()).collect();
        assert!(req_strs.contains(&"page_id"));
        assert!(req_strs.contains(&"properties"));
    }

    fn make_tool() -> NotionUpdatePageProperties {
        use crate::{NotionClient, NotionConfig};
        use std::sync::Arc;
        let client = Arc::new(NotionClient::new(
            reqwest::Client::new(),
            NotionConfig::new("ntn_x"),
        ));
        NotionUpdatePageProperties::new(client)
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
        assert_eq!(make_tool().name(), "notion.update_page_properties");
    }
}
