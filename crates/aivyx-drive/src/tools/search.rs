//! `drive.search` — Drive query DSL file search.
//!
//! Phase 129 Task 4. The first Drive tool. Implements
//! [`aivyx_core::Tool`]; served by the lifted multi-tool
//! harness in `aivyx_tool::multi_harness`.
//!
//! ## API call
//!
//! Single GET to `files.list` with the operator-supplied
//! `q` (Google Drive query DSL — e.g.
//! `name contains 'budget'`,
//! `mimeType = 'application/vnd.google-apps.spreadsheet'`,
//! `'<folder_id>' in parents`). Returns metadata-only
//! summaries; enrichment via `drive.get_metadata` (Task 5)
//! for the full per-file detail or `drive.download_file`
//! (Task 8) for content.
//!
//! ## include_trashed semantics
//!
//! Drive doesn't have a dedicated "exclude trashed"
//! parameter; trashed-filtering is part of the `q` DSL.
//! When `include_trashed` is false (the default), we
//! prepend `trashed = false and ` to the operator's query
//! string (or pass `trashed = false` alone if the operator
//! supplied no query). When true, the operator's `q`
//! passes through verbatim. The operator can override by
//! explicitly including `trashed = true` in their query
//! when `include_trashed` is true.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::drive_client::SharedDriveClient;

const MAX_RESULTS_CAP: u64 = 100;
const DEFAULT_MAX_RESULTS: u64 = 25;

/// Fields requested from Google Drive's `files.list`. Keep
/// scoped to what we serialize back to the LLM — extra
/// fields would inflate the response without adding value.
const FILES_LIST_FIELDS: &str =
    "files(id,name,mimeType,size,modifiedTime,owners(emailAddress),parents),nextPageToken";

pub struct DriveSearch {
    id: ToolId,
    schema: Value,
    client: SharedDriveClient,
}

impl DriveSearch {
    pub fn new(client: SharedDriveClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for DriveSearch {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "drive.search"
    }

    fn description(&self) -> &str {
        "Search Google Drive files using Drive's query DSL. \
         Input is a JSON object with an optional `q` field \
         (Drive query DSL — e.g. `name contains 'budget'`, \
         `mimeType = 'application/vnd.google-apps.spreadsheet'`, \
         `'<folder_id>' in parents`, \
         `modifiedTime > '2026-01-01T00:00:00'`), an optional \
         `max_results` (default 25, capped at 100), and an \
         optional `include_trashed` (default false). Returns a \
         JSON object with a `files` array of `{id, name, \
         mime_type, size, modified_at, owner_email, \
         parent_folder_ids}` entries and an optional \
         `next_page_token` for pagination. To filter to a \
         specific folder use `'<folder_id>' in parents`; for \
         all-folder search omit `q` entirely."
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
                    detail: format!("drive.search: {reason}"),
                });
            }
        };

        let q_string = build_q_string(parsed.q.as_deref(), parsed.include_trashed);
        let max_results_str = parsed.max_results.to_string();
        let mut query: Vec<(&str, String)> = vec![
            ("pageSize", max_results_str),
            ("fields", FILES_LIST_FIELDS.to_string()),
        ];
        if let Some(ref q) = q_string {
            query.push(("q", q.clone()));
        }

        let body: Value = match self.client.get_json("/files", &query).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("drive.search: API call failed: {e}"),
                });
            }
        };

        let files: Vec<Value> = body
            .get("files")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().map(file_summary).collect())
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

/// Pull a stable snake-case summary out of one Google
/// Drive file JSON value. Same posture as
/// `calendar.list_events::event_summary` — every field
/// present-and-typed or null, never silently missing.
pub(crate) fn file_summary(file: &Value) -> Value {
    let id = file.get("id").cloned().unwrap_or(Value::Null);
    let name = file.get("name").cloned().unwrap_or(Value::Null);
    let mime_type = file.get("mimeType").cloned().unwrap_or(Value::Null);
    // Size is a string in Drive's API (because file sizes
    // exceed JSON safe-int range). Parse to u64 when
    // possible; pass through as null if absent or
    // unparseable. Google-native types (Docs / Sheets /
    // Slides) have no `size` field.
    let size = file
        .get("size")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<u64>().ok())
        .map(|n| Value::Number(n.into()))
        .unwrap_or(Value::Null);
    let modified_at = file
        .get("modifiedTime")
        .cloned()
        .unwrap_or(Value::Null);
    let owner_email = file
        .get("owners")
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|o| o.get("emailAddress"))
        .cloned()
        .unwrap_or(Value::Null);
    let parent_folder_ids = file
        .get("parents")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    json!({
        "id": id,
        "name": name,
        "mime_type": mime_type,
        "size": size,
        "modified_at": modified_at,
        "owner_email": owner_email,
        "parent_folder_ids": parent_folder_ids,
    })
}

/// Build the final `q` query param. Returns `None` when
/// nothing should be sent (operator supplied no query AND
/// requested include_trashed=true).
pub(crate) fn build_q_string(user_q: Option<&str>, include_trashed: bool) -> Option<String> {
    let user_q = user_q.map(str::trim).filter(|s| !s.is_empty());
    match (user_q, include_trashed) {
        (Some(q), true) => Some(q.to_string()),
        (Some(q), false) => Some(format!("trashed = false and ({q})")),
        (None, true) => None,
        (None, false) => Some("trashed = false".to_string()),
    }
}

#[derive(Debug)]
struct ParsedInput {
    q: Option<String>,
    max_results: u64,
    include_trashed: bool,
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
        Some(_) => return Err("`q` must be a string (Drive query DSL)".to_string()),
    };
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
        q,
        max_results,
        include_trashed,
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "q": {
                "type": ["string", "null"],
                "description": "Drive query DSL — e.g. `name contains 'foo'`, `mimeType = 'application/pdf'`, `'<folder_id>' in parents`. Optional — when omitted, lists all non-trashed files visible to the operator."
            },
            "max_results": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_RESULTS_CAP,
                "default": DEFAULT_MAX_RESULTS,
                "description": "Maximum files to return. Capped at 100."
            },
            "include_trashed": {
                "type": "boolean",
                "default": false,
                "description": "Include files in Trash. Default false."
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
        assert_eq!(p.max_results, DEFAULT_MAX_RESULTS);
        assert!(!p.include_trashed);
    }

    #[test]
    fn parse_input_accepts_query_string() {
        let p = parse_input(&json!({"q": "name contains 'budget'"})).expect("parse");
        assert_eq!(p.q.as_deref(), Some("name contains 'budget'"));
    }

    #[test]
    fn parse_input_treats_empty_q_as_none() {
        let p = parse_input(&json!({"q": "   "})).expect("parse");
        assert!(p.q.is_none(), "whitespace-only q normalizes to None");
    }

    #[test]
    fn parse_input_rejects_non_string_q() {
        let e = parse_input(&json!({"q": 42})).expect_err("must error");
        assert!(e.contains("`q`"), "{e}");
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
    fn parse_input_rejects_non_boolean_include_trashed() {
        let e = parse_input(&json!({"include_trashed": "yes"})).expect_err("must error");
        assert!(e.contains("include_trashed"), "{e}");
    }

    #[test]
    fn build_q_string_default_excludes_trashed() {
        let q = build_q_string(None, false).expect("present");
        assert_eq!(q, "trashed = false");
    }

    #[test]
    fn build_q_string_with_user_q_default_wraps() {
        let q = build_q_string(Some("name contains 'x'"), false).expect("present");
        assert_eq!(q, "trashed = false and (name contains 'x')");
    }

    #[test]
    fn build_q_string_include_trashed_with_user_q_passes_through() {
        let q = build_q_string(Some("trashed = true"), true).expect("present");
        assert_eq!(q, "trashed = true");
    }

    #[test]
    fn build_q_string_include_trashed_no_user_q_yields_none() {
        // Operator wants "everything including trashed" with no
        // additional filter → don't send a `q` at all.
        assert!(build_q_string(None, true).is_none());
    }

    #[test]
    fn build_q_string_trims_whitespace_only_user_q() {
        let q = build_q_string(Some("   "), false).expect("present");
        assert_eq!(q, "trashed = false");
    }

    #[test]
    fn file_summary_extracts_canonical_fields() {
        let file = json!({
            "id": "abc",
            "name": "Q4 Budget",
            "mimeType": "application/vnd.google-apps.spreadsheet",
            "size": "12345",
            "modifiedTime": "2026-05-30T14:23:00Z",
            "owners": [{"emailAddress": "alice@example.com"}],
            "parents": ["folder-id-1", "folder-id-2"],
        });
        let s = file_summary(&file);
        assert_eq!(s["id"], "abc");
        assert_eq!(s["name"], "Q4 Budget");
        assert_eq!(s["mime_type"], "application/vnd.google-apps.spreadsheet");
        assert_eq!(s["size"], 12345);
        assert_eq!(s["modified_at"], "2026-05-30T14:23:00Z");
        assert_eq!(s["owner_email"], "alice@example.com");
        assert_eq!(s["parent_folder_ids"][0], "folder-id-1");
    }

    #[test]
    fn file_summary_null_when_size_absent_or_unparseable() {
        // Google-native types (Docs/Sheets/Slides) have no
        // `size` field — must come through as null, not 0.
        let file = json!({"id": "x", "name": "Doc", "mimeType": "application/vnd.google-apps.document"});
        let s = file_summary(&file);
        assert!(s["size"].is_null());

        // Unparseable size string → null (defensive).
        let file = json!({"id": "x", "size": "garbage"});
        let s = file_summary(&file);
        assert!(s["size"].is_null());
    }

    #[test]
    fn file_summary_null_when_owners_absent() {
        let file = json!({"id": "x", "name": "Untitled"});
        let s = file_summary(&file);
        assert!(s["owner_email"].is_null());
    }

    #[test]
    fn file_summary_empty_array_when_parents_absent() {
        let file = json!({"id": "x"});
        let s = file_summary(&file);
        assert!(s["parent_folder_ids"].is_array());
        assert_eq!(s["parent_folder_ids"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn input_schema_declares_no_required_fields() {
        let schema = input_schema();
        let req = schema.get("required");
        assert!(req.is_none() || req.unwrap().as_array().unwrap().is_empty());
        assert_eq!(schema["additionalProperties"], false);
    }

    fn make_tool() -> DriveSearch {
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
        DriveSearch::new(client)
    }

    #[test]
    fn required_scope_is_drive_read() {
        assert_eq!(make_tool().required_scope(&json!({})).to_string(), "drive.read");
    }

    #[test]
    fn tool_name_is_canonical() {
        assert_eq!(make_tool().name(), "drive.search");
    }
}
