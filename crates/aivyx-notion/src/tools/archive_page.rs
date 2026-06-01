//! `notion.archive_page` — archive (Notion's "delete") a
//! page.
//!
//! Phase 130 Task 9. Seventh and final Notion tool;
//! fourth write tool. Completes the Q1a 7-tool Notion
//! surface.
//!
//! ## API call
//!
//! PATCH `/pages/{page_id}` with `{"archived": true}`.
//! Notion doesn't have a DELETE endpoint — archive is the
//! equivalent operation. Archived pages are recoverable
//! via Notion's UI (Trash menu) for ~30 days.
//!
//! ## Idempotency
//!
//! Re-archiving an already-archived page succeeds; the
//! tool surfaces `was_already_archived: true` so the
//! audit chain can tell "first archive" apart from
//! "redundant archive" (same pattern as
//! `calendar.delete_event` and `drive.delete_file`).

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::notion_client::SharedNotionClient;

pub struct NotionArchivePage {
    id: ToolId,
    schema: Value,
    client: SharedNotionClient,
}

impl NotionArchivePage {
    pub fn new(client: SharedNotionClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for NotionArchivePage {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "notion.archive_page"
    }

    fn description(&self) -> &str {
        "Archive (Notion's \"delete\") a page. Input is a \
         JSON object with required `page_id`. Notion \
         doesn't have a hard-delete endpoint — archive is \
         the equivalent operation. Archived pages are \
         recoverable via Notion's UI (Trash menu) for ~30 \
         days. Returns `{page_id, was_already_archived}`. \
         Requires Trusted-tier capability grant for \
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
                    detail: format!("notion.archive_page: {reason}"),
                });
            }
        };

        // First read the page to determine the pre-state
        // (was it already archived?). Notion's PATCH
        // returns the page object with the final state
        // but doesn't echo the pre-state.
        let page_path = format!("/pages/{}", parsed.page_id);
        let pre_state: Value = match self.client.get_json(&page_path, &[]).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("notion.archive_page: pre-state fetch failed: {e}"),
                });
            }
        };
        let was_already_archived = pre_state
            .get("archived")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let body = json!({"archived": true});
        let resp: Result<Value, _> = self.client.patch_json(&page_path, &body).await;
        if let Err(e) = resp {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("notion.archive_page: PATCH failed: {e}"),
            });
        }

        let output = json!({
            "page_id": parsed.page_id,
            "was_already_archived": was_already_archived,
        });

        ToolOutcome::Completed {
            output,
            verified: Verification::Verified,
        }
    }
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
                "description": "Page ID to archive. Required."
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
    fn parse_input_accepts_page_id() {
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
    fn input_schema_declares_page_id_required() {
        let schema = input_schema();
        let req = schema["required"].as_array().expect("required");
        assert_eq!(req.len(), 1);
        assert_eq!(req[0], "page_id");
    }

    fn make_tool() -> NotionArchivePage {
        use crate::{NotionClient, NotionConfig};
        use std::sync::Arc;
        let client = Arc::new(NotionClient::new(
            reqwest::Client::new(),
            NotionConfig::new("ntn_x"),
        ));
        NotionArchivePage::new(client)
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
        assert_eq!(make_tool().name(), "notion.archive_page");
    }
}
