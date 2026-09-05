//! `drive.list_folder` — list children of a Drive folder.
//!
//! Phase 129 Task 6. Third Drive tool; second read tool.
//! Specializes `drive.search` for the common operator-ask
//! "what's in this folder?" — internally a `files.list`
//! with `'{folder_id}' in parents` query.
//!
//! ## Why a separate tool instead of an operator-built
//! `drive.search` query?
//!
//! `'<folder_id>' in parents` is the load-bearing Drive
//! query DSL fragment for folder enumeration, and the
//! string-quoting + escape semantics are awkward enough
//! for LLM-direct construction (single quotes around the
//! folder_id; folder_id mustn't contain unescaped `'`).
//! A dedicated tool with `folder_id` as a parameter
//! sidesteps that and yields cleaner audit-chain inputs
//! (operators querying "what tool calls listed folder X"
//! see the folder_id in `input.folder_id`, not buried in a
//! free-form query string).

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::drive_client::SharedDriveClient;

const MAX_RESULTS_CAP: u64 = 100;
const DEFAULT_MAX_RESULTS: u64 = 25;
const DEFAULT_FOLDER_ID: &str = "root";
const FILES_LIST_FIELDS: &str =
    "files(id,name,mimeType,size,modifiedTime,owners(emailAddress),parents),nextPageToken";

pub struct DriveListFolder {
    id: ToolId,
    schema: Value,
    client: SharedDriveClient,
}

impl DriveListFolder {
    pub fn new(client: SharedDriveClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for DriveListFolder {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "drive.list_folder"
    }

    // Chapter Picket follow-up (Finding 3) — file/folder names in
    // Drive are externally authored and may carry a prompt-injection
    // payload.
    fn output_is_untrusted(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "List the children of a Google Drive folder. Input \
         is a JSON object with optional `folder_id` (default \
         `\"root\"` — the authenticated user's My Drive top \
         level), `max_results` (default 25, capped at 100), \
         and `include_trashed` (default false). Returns a \
         JSON object with a `files` array of `{id, name, \
         mime_type, size, modified_at, owner_email, \
         parent_folder_ids}` entries (subfolders appear \
         with mime_type `application/vnd.google-apps.folder`) \
         and an optional `next_page_token` for pagination."
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
                    detail: format!("drive.list_folder: {reason}"),
                });
            }
        };

        let q_string = build_folder_q(&parsed.folder_id, parsed.include_trashed);
        let max_results_str = parsed.max_results.to_string();
        let query: Vec<(&str, String)> = vec![
            ("pageSize", max_results_str),
            ("fields", FILES_LIST_FIELDS.to_string()),
            ("q", q_string),
        ];

        let body: Value = match self.client.get_json("/files", &query).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("drive.list_folder: API call failed: {e}"),
                });
            }
        };

        // Reuse search.rs's file_summary so the LLM sees an
        // identical shape across list/search results.
        let files: Vec<Value> = body
            .get("files")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().map(super::search::file_summary).collect())
            .unwrap_or_default();
        let next_page_token = body
            .get("nextPageToken")
            .and_then(|v| v.as_str())
            .map(String::from);

        let output = json!({
            "files": files,
            "next_page_token": next_page_token,
        });

        ToolOutcome::Completed {
            output,
            verified: Verification::NotApplicable,
        }
    }
}

/// Build the Drive `q` string for folder enumeration.
/// `'{folder_id}' in parents` is the canonical filter; add
/// the trashed clause depending on include_trashed.
pub(crate) fn build_folder_q(folder_id: &str, include_trashed: bool) -> String {
    // Escape any single-quotes in folder_id; backslash is
    // Drive's escape character inside single-quoted strings.
    let escaped = folder_id.replace('\\', "\\\\").replace('\'', "\\'");
    let parents_clause = format!("'{escaped}' in parents");
    if include_trashed {
        parents_clause
    } else {
        format!("trashed = false and {parents_clause}")
    }
}

#[derive(Debug)]
struct ParsedInput {
    folder_id: String,
    max_results: u64,
    include_trashed: bool,
}

fn parse_input(input: &Value) -> Result<ParsedInput, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
    let folder_id = match obj.get("folder_id") {
        None => DEFAULT_FOLDER_ID.to_string(),
        Some(v) => v
            .as_str()
            .ok_or_else(|| "`folder_id` must be a string".to_string())?
            .trim()
            .to_string(),
    };
    if folder_id.is_empty() {
        return Err("`folder_id` must not be empty".to_string());
    }
    let max_results = match obj.get("max_results") {
        None => DEFAULT_MAX_RESULTS,
        Some(v) => v
            .as_u64()
            .ok_or_else(|| "`max_results` must be a non-negative integer".to_string())?,
    };
    if max_results == 0 {
        return Err("`max_results` must be >= 1".to_string());
    }
    let max_results = max_results.min(MAX_RESULTS_CAP);
    let include_trashed = match obj.get("include_trashed") {
        None => false,
        Some(v) => v
            .as_bool()
            .ok_or_else(|| "`include_trashed` must be a boolean".to_string())?,
    };
    Ok(ParsedInput {
        folder_id,
        max_results,
        include_trashed,
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "folder_id": {
                "type": "string",
                "default": DEFAULT_FOLDER_ID,
                "description": "Drive folder ID. Default `\"root\"` (the authenticated user's My Drive top level)."
            },
            "max_results": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_RESULTS_CAP,
                "default": DEFAULT_MAX_RESULTS,
                "description": "Maximum entries to return. Capped at 100."
            },
            "include_trashed": {
                "type": "boolean",
                "default": false,
                "description": "Include trashed children. Default false."
            }
        },
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_list_folder_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }

    #[test]
    fn parse_input_defaults_when_empty() {
        let p = parse_input(&json!({})).expect("parse");
        assert_eq!(p.folder_id, DEFAULT_FOLDER_ID);
        assert_eq!(p.max_results, DEFAULT_MAX_RESULTS);
        assert!(!p.include_trashed);
    }

    #[test]
    fn parse_input_accepts_folder_id() {
        let p = parse_input(&json!({"folder_id": "1XyZ"})).expect("parse");
        assert_eq!(p.folder_id, "1XyZ");
    }

    #[test]
    fn parse_input_rejects_empty_folder_id() {
        let e = parse_input(&json!({"folder_id": "   "})).expect_err("must error");
        assert!(e.contains("folder_id"), "{e}");
    }

    #[test]
    fn parse_input_rejects_non_string_folder_id() {
        let e = parse_input(&json!({"folder_id": 42})).expect_err("must error");
        assert!(e.contains("folder_id"), "{e}");
    }

    #[test]
    fn parse_input_caps_max_results() {
        let p = parse_input(&json!({"max_results": 9999})).expect("parse");
        assert_eq!(p.max_results, MAX_RESULTS_CAP);
    }

    #[test]
    fn parse_input_rejects_zero_max_results() {
        let e = parse_input(&json!({"max_results": 0})).expect_err("must error");
        assert!(e.contains(">= 1"), "{e}");
    }

    #[test]
    fn build_folder_q_default_excludes_trashed() {
        let q = build_folder_q("root", false);
        assert_eq!(q, "trashed = false and 'root' in parents");
    }

    #[test]
    fn build_folder_q_include_trashed_drops_trashed_clause() {
        let q = build_folder_q("abc", true);
        assert_eq!(q, "'abc' in parents");
    }

    #[test]
    fn build_folder_q_escapes_single_quote_in_folder_id() {
        // Drive folder IDs in practice don't include `'`,
        // but defensive escaping prevents query-DSL
        // injection if a malformed ID ever leaks through.
        let q = build_folder_q("ab'cd", false);
        assert!(q.contains("'ab\\'cd'"));
    }

    #[test]
    fn build_folder_q_escapes_backslash_in_folder_id() {
        let q = build_folder_q("ab\\cd", false);
        assert!(q.contains("'ab\\\\cd'"));
    }

    #[test]
    fn input_schema_declares_no_required_fields() {
        let schema = input_schema();
        let req = schema.get("required");
        assert!(req.is_none() || req.unwrap().as_array().unwrap().is_empty());
        assert_eq!(schema["additionalProperties"], false);
    }

    fn make_tool() -> DriveListFolder {
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
        DriveListFolder::new(client)
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
        assert_eq!(make_tool().name(), "drive.list_folder");
    }
}
