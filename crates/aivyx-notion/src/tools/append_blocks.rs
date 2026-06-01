//! `notion.append_blocks` — append blocks to an existing
//! Notion page.
//!
//! Phase 130 Task 7. Fifth Notion tool; second write tool.
//!
//! ## API call
//!
//! PATCH `/blocks/{page_id}/children` with
//! `{"children": [...]}`. Same Notion block shapes as
//! `notion.create_page`'s `children` field.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::notion_client::SharedNotionClient;

pub struct NotionAppendBlocks {
    id: ToolId,
    schema: Value,
    client: SharedNotionClient,
}

impl NotionAppendBlocks {
    pub fn new(client: SharedNotionClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for NotionAppendBlocks {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "notion.append_blocks"
    }

    fn description(&self) -> &str {
        "Append blocks to an existing Notion page. Input is \
         a JSON object with required `page_id` and required \
         `blocks` (array of Notion block objects passed \
         verbatim — typical shape: \
         `{\"object\": \"block\", \"type\": \"paragraph\", \
         \"paragraph\": {\"rich_text\": [{\"text\": \
         {\"content\": \"...\"}}]}}`). Returns `{count, \
         block_ids}` of the appended blocks. Requires \
         Trusted-tier capability grant for `notion.write`."
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
                    detail: format!("notion.append_blocks: {reason}"),
                });
            }
        };

        let body = json!({"children": parsed.blocks.clone()});
        let path = format!("/blocks/{}/children", parsed.page_id);

        let resp: Value = match self.client.patch_json(&path, &body).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("notion.append_blocks: API call failed: {e}"),
                });
            }
        };

        let appended_ids: Vec<Value> = resp
            .get("results")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|b| b.get("id").cloned())
                    .collect()
            })
            .unwrap_or_default();
        let count = appended_ids.len();

        let output = json!({
            "count": count,
            "block_ids": appended_ids,
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
    pub(crate) blocks: Value,
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
    let blocks = obj
        .get("blocks")
        .ok_or_else(|| "input must include a `blocks` array".to_string())?
        .clone();
    let arr = blocks
        .as_array()
        .ok_or_else(|| "`blocks` must be an array".to_string())?;
    if arr.is_empty() {
        return Err("`blocks` must not be empty".to_string());
    }
    // Defensive: each block must be a JSON object (Notion's
    // block schema). Surface the index of the malformed
    // entry so operators can fix it.
    for (i, b) in arr.iter().enumerate() {
        if !b.is_object() {
            return Err(format!("`blocks[{i}]` must be an object"));
        }
    }
    Ok(ParsedInput {
        page_id: page_id.to_string(),
        blocks,
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "page_id": {
                "type": "string",
                "description": "Page ID to append blocks under. Required."
            },
            "blocks": {
                "type": "array",
                "items": {"type": "object"},
                "minItems": 1,
                "description": "Array of Notion block objects passed verbatim."
            }
        },
        "required": ["page_id", "blocks"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn good_blocks() -> Value {
        json!([
            {"object": "block", "type": "paragraph",
             "paragraph": {"rich_text": [{"text": {"content": "Hi"}}]}}
        ])
    }

    #[test]
    fn parse_input_accepts_minimal() {
        let p = parse_input(&json!({
            "page_id": "p1",
            "blocks": good_blocks()
        }))
        .expect("parse");
        assert_eq!(p.page_id, "p1");
        assert!(p.blocks.is_array());
    }

    #[test]
    fn parse_input_rejects_missing_page_id() {
        let e =
            parse_input(&json!({"blocks": good_blocks()})).expect_err("must error");
        assert!(e.contains("page_id"), "{e}");
    }

    #[test]
    fn parse_input_rejects_empty_page_id() {
        let e = parse_input(&json!({"page_id": "   ", "blocks": good_blocks()}))
            .expect_err("must error");
        assert!(e.contains("page_id"), "{e}");
    }

    #[test]
    fn parse_input_rejects_missing_blocks() {
        let e = parse_input(&json!({"page_id": "p1"})).expect_err("must error");
        assert!(e.contains("blocks"), "{e}");
    }

    #[test]
    fn parse_input_rejects_empty_blocks_array() {
        let e = parse_input(&json!({"page_id": "p1", "blocks": []}))
            .expect_err("must error");
        assert!(e.contains("must not be empty"), "{e}");
    }

    #[test]
    fn parse_input_rejects_non_array_blocks() {
        let e =
            parse_input(&json!({"page_id": "p1", "blocks": "x"})).expect_err("must error");
        assert!(e.contains("array"), "{e}");
    }

    #[test]
    fn parse_input_rejects_non_object_block_entry() {
        let e = parse_input(&json!({
            "page_id": "p1",
            "blocks": ["not-an-object"]
        }))
        .expect_err("must error");
        assert!(e.contains("blocks[0]"), "{e}");
    }

    #[test]
    fn input_schema_declares_required_fields() {
        let schema = input_schema();
        let req = schema["required"].as_array().expect("required");
        let req_strs: Vec<&str> = req.iter().filter_map(|v| v.as_str()).collect();
        assert!(req_strs.contains(&"page_id"));
        assert!(req_strs.contains(&"blocks"));
        assert_eq!(schema["additionalProperties"], false);
    }

    fn make_tool() -> NotionAppendBlocks {
        use crate::{NotionClient, NotionConfig};
        use std::sync::Arc;
        let client = Arc::new(NotionClient::new(
            reqwest::Client::new(),
            NotionConfig::new("ntn_x"),
        ));
        NotionAppendBlocks::new(client)
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
        assert_eq!(make_tool().name(), "notion.append_blocks");
    }
}
