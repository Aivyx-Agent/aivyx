//! `drive.recent_changes` — files visible to the
//! operator that were modified in the last N
//! hours.
//!
//! Phase 145 — sibling to
//! [`super::recent_files`]. Where `recent_files`
//! filters to `'me' in owners` (the cognitive
//! shape "what did *I* work on"), this tool
//! drops the owner filter so collaborator edits,
//! shared docs, and incoming uploads all
//! surface. The cognitive shape is "what changed
//! in my Drive."
//!
//! ## API call
//!
//! Single GET to `/files` with a composed `q`:
//!
//! ```text
//! modifiedTime > '<now - window_hours>'
//! ```
//!
//! `trashed = false` is prepended unless the
//! operator explicitly sets `include_trashed:
//! true`. Results are ordered by `modifiedTime
//! desc` so the most-recent change appears
//! first.
//!
//! ## Output shape
//!
//! Same `file_summary` shape `drive.search` and
//! `drive.recent_files` return; the agent reads
//! `owner_email` to distinguish "this is mine"
//! from "this is from someone else" without
//! requiring a second `drive.get_metadata`
//! lookup.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::drive_client::SharedDriveClient;
use crate::tools::search::{build_q_string, file_summary};

const MAX_RESULTS_CAP: u64 = 100;
const DEFAULT_MAX_RESULTS: u64 = 25;
const DEFAULT_WINDOW_HOURS: u64 = 24;
const MAX_WINDOW_HOURS: u64 = 24 * 30; // 30 days

const FILES_LIST_FIELDS: &str =
    "files(id,name,mimeType,size,modifiedTime,owners(emailAddress),parents),nextPageToken";

pub struct DriveRecentChanges {
    id: ToolId,
    schema: Value,
    client: SharedDriveClient,
}

impl DriveRecentChanges {
    pub fn new(client: SharedDriveClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for DriveRecentChanges {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "drive.recent_changes"
    }

    fn description(&self) -> &str {
        "List Google Drive files visible to the \
         operator that were modified within the \
         last N hours, across every owner — \
         collaborator edits, shared docs, \
         incoming uploads. Input is a JSON \
         object with optional `window_hours` \
         (default 24, capped at 720 = 30 days), \
         `max_results` (default 25, capped at \
         100), and `include_trashed` (default \
         false). Returns `{files: [...], \
         next_page_token, window_hours}` sorted \
         by modifiedTime descending. Read \
         `owner_email` on each entry to \
         distinguish operator-owned from \
         collaborator-edited files. For \
         operator-owned files only, use \
         `drive.recent_files`."
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
                    detail: format!("drive.recent_changes: {reason}"),
                });
            }
        };

        let now = Utc::now();
        let base_q = build_recent_changes_q(now, parsed.window_hours);
        let q_string = build_q_string(Some(&base_q), parsed.include_trashed);
        let max_results_str = parsed.max_results.to_string();
        let mut query: Vec<(&str, String)> = vec![
            ("pageSize", max_results_str),
            ("fields", FILES_LIST_FIELDS.to_string()),
            ("orderBy", "modifiedTime desc".to_string()),
        ];
        if let Some(ref q) = q_string {
            query.push(("q", q.clone()));
        }

        let body: Value = match self.client.get_json("/files", &query).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("drive.recent_changes: API call failed: {e}"),
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

        ToolOutcome::Completed {
            output: json!({
                "files": files,
                "next_page_token": next_page_token,
                "window_hours": parsed.window_hours,
            }),
            verified: Verification::NotApplicable,
        }
    }
}

/// Compose the `q` clause for any-files-modified-
/// recently. Pure substrate — `now` is
/// parameterized so unit tests can pin a
/// deterministic timestamp and assert the exact
/// string.
pub(crate) fn build_recent_changes_q(now: DateTime<Utc>, window_hours: u64) -> String {
    let since = now - chrono::Duration::hours(window_hours as i64);
    format!("modifiedTime > '{}'", since.to_rfc3339())
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "window_hours": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_WINDOW_HOURS,
                "description": "Hours into the past to scan (default 24, max 720)"
            },
            "max_results": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_RESULTS_CAP,
                "description": "Cap on files returned (default 25, max 100)"
            },
            "include_trashed": {
                "type": "boolean",
                "description": "Include files in Trash (default false)"
            }
        },
        "additionalProperties": false
    })
}

#[derive(Debug)]
struct ParsedInput {
    window_hours: u64,
    max_results: u64,
    include_trashed: bool,
}

fn parse_input(input: &Value) -> Result<ParsedInput, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
    let window_hours = match obj.get("window_hours") {
        None => DEFAULT_WINDOW_HOURS,
        Some(v) => v
            .as_u64()
            .ok_or_else(|| "`window_hours` must be a positive integer".to_string())?,
    };
    if window_hours == 0 {
        return Err("`window_hours` must be >= 1".to_string());
    }
    let window_hours = window_hours.min(MAX_WINDOW_HOURS);

    let max_results = match obj.get("max_results") {
        None => DEFAULT_MAX_RESULTS,
        Some(v) => v
            .as_u64()
            .ok_or_else(|| "`max_results` must be a positive integer".to_string())?,
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
        window_hours,
        max_results,
        include_trashed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now_fixed() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 3, 12, 0, 0).unwrap()
    }

    #[test]
    fn parse_defaults_to_24_hours_25_results() {
        let p = parse_input(&json!({})).unwrap();
        assert_eq!(p.window_hours, 24);
        assert_eq!(p.max_results, 25);
        assert!(!p.include_trashed);
    }

    #[test]
    fn parse_honors_explicit_window_and_results() {
        let p = parse_input(&json!({
            "window_hours": 6,
            "max_results": 50,
        }))
        .unwrap();
        assert_eq!(p.window_hours, 6);
        assert_eq!(p.max_results, 50);
    }

    #[test]
    fn parse_clamps_window_to_thirty_days() {
        let p = parse_input(&json!({"window_hours": 9999})).unwrap();
        assert_eq!(p.window_hours, MAX_WINDOW_HOURS);
    }

    #[test]
    fn parse_clamps_max_results_to_one_hundred() {
        let p = parse_input(&json!({"max_results": 9999})).unwrap();
        assert_eq!(p.max_results, MAX_RESULTS_CAP);
    }

    #[test]
    fn parse_zero_window_rejected() {
        let err = parse_input(&json!({"window_hours": 0})).unwrap_err();
        assert!(err.contains(">= 1"), "{err}");
    }

    #[test]
    fn build_recent_changes_q_does_not_include_owner_filter() {
        let q = build_recent_changes_q(now_fixed(), 24);
        assert!(
            !q.contains("owners"),
            "recent_changes must NOT filter by owner: {q}"
        );
        assert!(q.contains("modifiedTime > '"), "{q}");
    }

    #[test]
    fn build_recent_changes_q_24_hour_window() {
        // 24 hours before 2026-06-03 noon UTC =
        // 2026-06-02 noon UTC.
        let q = build_recent_changes_q(now_fixed(), 24);
        assert!(q.contains("modifiedTime > '2026-06-02T12:00:00"), "{q}");
    }

    #[test]
    fn build_recent_changes_q_one_hour_window() {
        let q = build_recent_changes_q(now_fixed(), 1);
        assert!(q.contains("modifiedTime > '2026-06-03T11:00:00"), "{q}");
    }

    #[test]
    fn build_recent_changes_q_thirty_day_window() {
        let q = build_recent_changes_q(now_fixed(), 24 * 30);
        // 720 hours before 2026-06-03 = 2026-05-04 noon UTC.
        assert!(q.contains("modifiedTime > '2026-05-04T12:00:00"), "{q}");
    }

    #[test]
    fn build_q_string_wraps_trashed_clause_when_excluded() {
        let base = build_recent_changes_q(now_fixed(), 24);
        let wrapped = build_q_string(Some(&base), false).expect("some");
        assert!(wrapped.starts_with("trashed = false and ("), "{wrapped}");
        assert!(wrapped.contains("modifiedTime > '"), "{wrapped}");
        assert!(!wrapped.contains("owners"), "{wrapped}");
    }
}
