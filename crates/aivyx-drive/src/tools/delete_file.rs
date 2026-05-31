//! `drive.delete_file` — delete a Drive file (or folder).
//!
//! Phase 129 Task 10. Seventh and final Drive tool; third
//! write tool (`drive.write` capability, Trusted-gated).
//! Completes the Q2b 7-tool surface.
//!
//! ## API call
//!
//! DELETE `/files/{id}`. Drive permanently deletes the
//! file (not move-to-trash) — operators wanting move-to-
//! trash semantics use `drive.update` with `trashed: true`
//! (out of Phase 129 scope; Phase 130+ candidate).
//!
//! ## Idempotency
//!
//! Drive returns 204 No Content for a successful delete
//! and 410 Gone for an already-deleted file. The lifted
//! `DriveClient::delete` helper treats both as success.
//! The tool surfaces the distinction via
//! `was_already_deleted` so the audit chain can tell
//! "first deletion" apart from "redundant deletion" (same
//! pattern as `calendar.delete_event`).

use async_trait::async_trait;
use reqwest::StatusCode;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::drive_client::SharedDriveClient;

pub struct DriveDeleteFile {
    id: ToolId,
    schema: Value,
    client: SharedDriveClient,
}

impl DriveDeleteFile {
    pub fn new(client: SharedDriveClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for DriveDeleteFile {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "drive.delete_file"
    }

    fn description(&self) -> &str {
        "Permanently delete a Google Drive file or folder \
         by ID. Input is a JSON object with a required \
         `file_id`. Idempotent: deleting an already-deleted \
         file succeeds with `was_already_deleted: true` \
         rather than failing. Returns `{file_id, \
         was_already_deleted}`. NOTE: Drive's DELETE is \
         permanent — for move-to-trash semantics use Drive's \
         UI directly (Phase 129 does not ship `drive.trash`). \
         Requires Trusted-tier capability grant for \
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
                    detail: format!("drive.delete_file: {reason}"),
                });
            }
        };

        let path = format!(
            "/files/{}",
            super::drive_urlencode(&parsed.file_id)
        );

        let status = match self.client.delete(&path).await {
            Ok(s) => s,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("drive.delete_file: API call failed: {e}"),
                });
            }
        };

        let was_already_deleted = status == StatusCode::GONE;

        let output = json!({
            "file_id": parsed.file_id,
            "was_already_deleted": was_already_deleted,
        });

        ToolOutcome::Completed {
            output,
            verified: Verification::Verified,
        }
    }
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
                "description": "Drive file or folder ID to delete. Required."
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
        let p = parse_input(&json!({"file_id": "  trimmed  "})).expect("parse");
        assert_eq!(p.file_id, "trimmed");
    }

    #[test]
    fn input_schema_declares_file_id_required() {
        let schema = input_schema();
        let req = schema["required"].as_array().expect("required");
        assert_eq!(req.len(), 1);
        assert_eq!(req[0], "file_id");
        assert_eq!(schema["additionalProperties"], false);
    }

    fn make_tool() -> DriveDeleteFile {
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
        DriveDeleteFile::new(client)
    }

    #[test]
    fn required_scope_is_drive_write() {
        assert_eq!(
            make_tool().required_scope(&json!({})).to_string(),
            "drive.write"
        );
    }

    #[test]
    fn tool_name_is_canonical() {
        assert_eq!(make_tool().name(), "drive.delete_file");
    }
}
