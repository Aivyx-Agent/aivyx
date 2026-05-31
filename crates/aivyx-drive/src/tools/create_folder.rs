//! `drive.create_folder` — create a Drive folder.
//!
//! Phase 129 Task 7. Fourth Drive tool; first write tool
//! (`drive.write` capability, Trusted-tier-only at the
//! ceiling level per Phase 129 Task 3 capability
//! registration).
//!
//! ## API call
//!
//! POST `/files` with a JSON body containing
//! `mimeType = "application/vnd.google-apps.folder"`,
//! the operator-supplied `name`, and the `parents` array
//! holding the parent folder ID. No multipart upload —
//! folder creation is metadata-only.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::drive_client::SharedDriveClient;

const DEFAULT_PARENT: &str = "root";
const FOLDER_MIME: &str = "application/vnd.google-apps.folder";

pub struct DriveCreateFolder {
    id: ToolId,
    schema: Value,
    client: SharedDriveClient,
}

impl DriveCreateFolder {
    pub fn new(client: SharedDriveClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for DriveCreateFolder {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "drive.create_folder"
    }

    fn description(&self) -> &str {
        "Create a new Google Drive folder. Input is a JSON \
         object with required `name` (folder display name) \
         and optional `parent_folder_id` (default \
         `\"root\"`). Returns `{id, name, \
         parent_folder_id, web_view_link}` of the created \
         folder. Requires Trusted-tier capability grant for \
         `drive.write`."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("drive.write").expect(
            "drive.write must parse — it is in KNOWN_BASES from Phase 129",
        )
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let parsed = match parse_input(&input) {
            Ok(p) => p,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("drive.create_folder: {reason}"),
                });
            }
        };

        let body = build_create_body(&parsed);
        // Restrict response fields so the result payload is
        // predictable + LLM-friendly.
        let path = "/files?fields=id,name,parents,webViewLink";

        let resp: Value = match self.client.post_json(path, &body).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("drive.create_folder: API call failed: {e}"),
                });
            }
        };

        let parent_folder_id = resp
            .get("parents")
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .cloned()
            .unwrap_or(Value::Null);

        let output = json!({
            "id": resp.get("id").cloned().unwrap_or(Value::Null),
            "name": resp.get("name").cloned().unwrap_or(Value::Null),
            "parent_folder_id": parent_folder_id,
            "web_view_link": resp.get("webViewLink").cloned().unwrap_or(Value::Null),
        });

        ToolOutcome::Completed {
            output,
            verified: Verification::Verified,
        }
    }
}

pub(crate) fn build_create_body(parsed: &ParsedInput) -> Value {
    json!({
        "name": parsed.name,
        "mimeType": FOLDER_MIME,
        "parents": [parsed.parent_folder_id.clone()],
    })
}

#[derive(Debug)]
pub(crate) struct ParsedInput {
    pub(crate) name: String,
    pub(crate) parent_folder_id: String,
}

fn parse_input(input: &Value) -> Result<ParsedInput, String> {
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
    let parent_folder_id = match obj.get("parent_folder_id") {
        None => DEFAULT_PARENT.to_string(),
        Some(v) => v
            .as_str()
            .ok_or_else(|| "`parent_folder_id` must be a string".to_string())?
            .trim()
            .to_string(),
    };
    if parent_folder_id.is_empty() {
        return Err("`parent_folder_id` must not be empty".to_string());
    }
    Ok(ParsedInput {
        name: name.to_string(),
        parent_folder_id,
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "Folder display name. Required."
            },
            "parent_folder_id": {
                "type": "string",
                "default": DEFAULT_PARENT,
                "description": "Parent folder ID. Default `\"root\"` (the user's My Drive top level)."
            }
        },
        "required": ["name"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_input_accepts_name_only() {
        let p = parse_input(&json!({"name": "Projects"})).expect("parse");
        assert_eq!(p.name, "Projects");
        assert_eq!(p.parent_folder_id, DEFAULT_PARENT);
    }

    #[test]
    fn parse_input_accepts_parent_folder_id() {
        let p = parse_input(&json!({
            "name": "Q3",
            "parent_folder_id": "1XyZ"
        }))
        .expect("parse");
        assert_eq!(p.parent_folder_id, "1XyZ");
    }

    #[test]
    fn parse_input_rejects_missing_name() {
        let e = parse_input(&json!({})).expect_err("must error");
        assert!(e.contains("name"), "{e}");
    }

    #[test]
    fn parse_input_rejects_empty_name() {
        let e = parse_input(&json!({"name": "   "})).expect_err("must error");
        assert!(e.contains("name"), "{e}");
    }

    #[test]
    fn parse_input_rejects_non_string_name() {
        let e = parse_input(&json!({"name": 42})).expect_err("must error");
        assert!(e.contains("name"), "{e}");
    }

    #[test]
    fn parse_input_rejects_empty_parent_folder_id() {
        let e = parse_input(&json!({"name": "x", "parent_folder_id": ""}))
            .expect_err("must error");
        assert!(e.contains("parent_folder_id"), "{e}");
    }

    #[test]
    fn build_create_body_emits_folder_mime_type() {
        let p = parse_input(&json!({"name": "Quarterly"})).unwrap();
        let body = build_create_body(&p);
        assert_eq!(body["name"], "Quarterly");
        assert_eq!(
            body["mimeType"],
            "application/vnd.google-apps.folder",
            "folder mime type is load-bearing — creates a folder rather than a regular file"
        );
        assert_eq!(body["parents"][0], "root");
    }

    #[test]
    fn build_create_body_uses_explicit_parent() {
        let p = parse_input(&json!({
            "name": "x",
            "parent_folder_id": "1XyZ"
        }))
        .unwrap();
        let body = build_create_body(&p);
        assert_eq!(body["parents"][0], "1XyZ");
    }

    #[test]
    fn input_schema_declares_name_required() {
        let schema = input_schema();
        let req = schema["required"].as_array().expect("required");
        assert_eq!(req.len(), 1);
        assert_eq!(req[0], "name");
        assert_eq!(schema["additionalProperties"], false);
    }

    fn make_tool() -> DriveCreateFolder {
        use crate::{DriveClient, OAuthConfig, TokenSet};
        use std::sync::Arc;
        let client = Arc::new(DriveClient::new(
            reqwest::Client::new(),
            OAuthConfig::new("id", "secret", "http://127.0.0.1:0/cb"),
            TokenSet {
                access_token: "x".to_string(),
                refresh_token: None,
                expires_at_unix_secs: 0,
                granted_scope: "scope".to_string(),
                token_type: "Bearer".to_string(),
            },
            std::path::PathBuf::from("/tmp/unused"),
        ));
        DriveCreateFolder::new(client)
    }

    #[test]
    fn required_scope_is_drive_write() {
        assert_eq!(
            make_tool().required_scope(&json!({})).to_string(),
            "drive.write",
            "create_folder MUST use drive.write (Trusted-tier)"
        );
    }

    #[test]
    fn tool_name_is_canonical() {
        assert_eq!(make_tool().name(), "drive.create_folder");
    }
}
