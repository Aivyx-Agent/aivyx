//! `drive.get_metadata` — single-file metadata fetch.
//!
//! Phase 129 Task 5. Second Drive tool. Reads one file by
//! ID and returns the full metadata. Pair with
//! `drive.search` (range search) and `drive.list_folder`
//! (folder enumeration): use those to find a file, then
//! drill in here for full per-file detail.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::drive_client::SharedDriveClient;

/// Field list for the metadata fetch. Wider than
/// `drive.search`'s subset — pulls description, version,
/// `webViewLink`, and `appProperties` in addition to the
/// search-shape fields.
const FILE_GET_FIELDS: &str =
    "id,name,mimeType,size,modifiedTime,createdTime,owners(emailAddress,displayName),parents,description,version,webViewLink,iconLink,trashed,starred,appProperties";

pub struct DriveGetMetadata {
    id: ToolId,
    schema: Value,
    client: SharedDriveClient,
}

impl DriveGetMetadata {
    pub fn new(client: SharedDriveClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for DriveGetMetadata {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "drive.get_metadata"
    }

    fn description(&self) -> &str {
        "Fetch full metadata for one Google Drive file by \
         ID. Input is a JSON object with a required \
         `file_id` (string; typically from `drive.search` \
         or `drive.list_folder`). Returns a JSON object \
         with the file's `id`, `name`, `mime_type`, `size`, \
         `modified_at`, `created_at`, `owner_email`, \
         `owner_display_name`, `description`, `version`, \
         `parent_folder_ids`, `web_view_link` (the Drive UI \
         URL), `trashed`, `starred`, and any \
         `app_properties` (custom metadata key/value pairs)."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("drive.read").expect(
            "drive.read must parse — it is in KNOWN_BASES from Phase 129",
        )
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let parsed = match parse_input(&input) {
            Ok(p) => p,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("drive.get_metadata: {reason}"),
                });
            }
        };

        let path = format!(
            "/files/{}",
            super::drive_urlencode(&parsed.file_id)
        );
        let query: Vec<(&str, String)> = vec![("fields", FILE_GET_FIELDS.to_string())];

        let body: Value = match self.client.get_json(&path, &query).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("drive.get_metadata: API call failed: {e}"),
                });
            }
        };

        let output = transform(&body);

        ToolOutcome::Completed {
            output,
            verified: Verification::NotApplicable,
        }
    }
}

fn transform(file: &Value) -> Value {
    let id = file.get("id").cloned().unwrap_or(Value::Null);
    let name = file.get("name").cloned().unwrap_or(Value::Null);
    let mime_type = file.get("mimeType").cloned().unwrap_or(Value::Null);
    let size = file
        .get("size")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u64>().ok())
        .map(|n| Value::Number(n.into()))
        .unwrap_or(Value::Null);
    let modified_at = file.get("modifiedTime").cloned().unwrap_or(Value::Null);
    let created_at = file.get("createdTime").cloned().unwrap_or(Value::Null);
    let owner = file
        .get("owners")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first());
    let owner_email = owner
        .and_then(|o| o.get("emailAddress"))
        .cloned()
        .unwrap_or(Value::Null);
    let owner_display_name = owner
        .and_then(|o| o.get("displayName"))
        .cloned()
        .unwrap_or(Value::Null);
    let parent_folder_ids = file
        .get("parents")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let description = file.get("description").cloned().unwrap_or(Value::Null);
    let version = file
        .get("version")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u64>().ok())
        .map(|n| Value::Number(n.into()))
        .unwrap_or(Value::Null);
    let web_view_link = file.get("webViewLink").cloned().unwrap_or(Value::Null);
    let trashed = file
        .get("trashed")
        .cloned()
        .unwrap_or(Value::Bool(false));
    let starred = file
        .get("starred")
        .cloned()
        .unwrap_or(Value::Bool(false));
    let app_properties = file
        .get("appProperties")
        .cloned()
        .unwrap_or_else(|| Value::Object(serde_json::Map::new()));

    json!({
        "id": id,
        "name": name,
        "mime_type": mime_type,
        "size": size,
        "modified_at": modified_at,
        "created_at": created_at,
        "owner_email": owner_email,
        "owner_display_name": owner_display_name,
        "parent_folder_ids": parent_folder_ids,
        "description": description,
        "version": version,
        "web_view_link": web_view_link,
        "trashed": trashed,
        "starred": starred,
        "app_properties": app_properties,
    })
}

#[derive(Debug)]
struct ParsedInput {
    file_id: String,
}

fn parse_input(input: &Value) -> Result<ParsedInput, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
    let file_id = obj
        .get("file_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| "input must include a `file_id` string field".to_string())?;
    if file_id.is_empty() {
        return Err("`file_id` must not be empty".to_string());
    }
    Ok(ParsedInput {
        file_id: file_id.to_string(),
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "file_id": {
                "type": "string",
                "description": "Drive file ID. Required."
            }
        },
        "required": ["file_id"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_input_accepts_minimal() {
        let p = parse_input(&json!({"file_id": "abc"})).expect("parse");
        assert_eq!(p.file_id, "abc");
    }

    #[test]
    fn parse_input_rejects_missing_file_id() {
        let e = parse_input(&json!({})).expect_err("must error");
        assert!(e.contains("file_id"), "{e}");
    }

    #[test]
    fn parse_input_rejects_empty_file_id() {
        let e = parse_input(&json!({"file_id": "   "})).expect_err("must error");
        assert!(e.contains("file_id"), "{e}");
    }

    #[test]
    fn parse_input_rejects_non_string_file_id() {
        let e = parse_input(&json!({"file_id": 42})).expect_err("must error");
        assert!(e.contains("file_id"), "{e}");
    }

    #[test]
    fn parse_input_trims_whitespace() {
        let p = parse_input(&json!({"file_id": "  trimmed-id  "})).expect("parse");
        assert_eq!(p.file_id, "trimmed-id");
    }

    #[test]
    fn transform_extracts_full_metadata() {
        let file = json!({
            "id": "1AbCdE",
            "name": "Q4 Budget",
            "mimeType": "application/vnd.google-apps.spreadsheet",
            "size": "98765",
            "modifiedTime": "2026-05-30T14:23:00Z",
            "createdTime": "2026-01-15T09:00:00Z",
            "owners": [{"emailAddress": "alice@example.com", "displayName": "Alice"}],
            "parents": ["folder-x"],
            "description": "Quarterly budget tracking",
            "version": "42",
            "webViewLink": "https://docs.google.com/spreadsheets/d/1AbCdE/edit",
            "trashed": false,
            "starred": true,
            "appProperties": {"client_id": "aivyx-1"},
        });
        let out = transform(&file);
        assert_eq!(out["id"], "1AbCdE");
        assert_eq!(out["name"], "Q4 Budget");
        assert_eq!(out["mime_type"], "application/vnd.google-apps.spreadsheet");
        assert_eq!(out["size"], 98765);
        assert_eq!(out["created_at"], "2026-01-15T09:00:00Z");
        assert_eq!(out["owner_email"], "alice@example.com");
        assert_eq!(out["owner_display_name"], "Alice");
        assert_eq!(out["parent_folder_ids"][0], "folder-x");
        assert_eq!(out["description"], "Quarterly budget tracking");
        assert_eq!(out["version"], 42);
        assert!(out["web_view_link"].as_str().unwrap().starts_with("https://"));
        assert_eq!(out["trashed"], false);
        assert_eq!(out["starred"], true);
        assert_eq!(out["app_properties"]["client_id"], "aivyx-1");
    }

    #[test]
    fn transform_minimal_event_fills_nulls_and_empty_defaults() {
        let file = json!({"id": "x"});
        let out = transform(&file);
        assert!(out["name"].is_null());
        assert!(out["size"].is_null());
        assert!(out["owner_email"].is_null());
        assert!(out["owner_display_name"].is_null());
        assert!(out["parent_folder_ids"].is_array());
        assert_eq!(out["parent_folder_ids"].as_array().unwrap().len(), 0);
        // Defensive defaults for the boolean fields.
        assert_eq!(out["trashed"], false);
        assert_eq!(out["starred"], false);
        // app_properties defaults to {} not null.
        assert!(out["app_properties"].is_object());
    }

    #[test]
    fn transform_handles_google_native_no_size() {
        // Google Docs / Sheets / Slides have no `size`
        // field — must come through as null.
        let file = json!({
            "id": "doc1",
            "name": "Untitled doc",
            "mimeType": "application/vnd.google-apps.document",
        });
        let out = transform(&file);
        assert!(out["size"].is_null());
    }

    #[test]
    fn input_schema_declares_file_id_required() {
        let schema = input_schema();
        let req = schema["required"].as_array().expect("required");
        assert_eq!(req.len(), 1);
        assert_eq!(req[0], "file_id");
        assert_eq!(schema["additionalProperties"], false);
    }

    fn make_tool() -> DriveGetMetadata {
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
        DriveGetMetadata::new(client)
    }

    #[test]
    fn required_scope_is_drive_read() {
        assert_eq!(
            make_tool().required_scope(&json!({})).to_string(),
            "drive.read"
        );
    }

    #[test]
    fn tool_name_is_canonical() {
        assert_eq!(make_tool().name(), "drive.get_metadata");
    }
}
