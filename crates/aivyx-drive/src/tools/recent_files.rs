//! `drive.recent_files` — files the operator
//! owns that were modified in the last N days.
//!
//! Phase 145. LLM-ergonomic shape over the same
//! `/files` endpoint `drive.search` uses, with
//! defaults aimed at the cognitive shape "what
//! did I work on this week."
//!
//! ## API call
//!
//! Single GET to `/files` with a composed `q`:
//!
//! ```text
//! 'me' in owners and modifiedTime > '<now - window_days>'
//! ```
//!
//! `trashed = false` is prepended unless the
//! operator explicitly sets `include_trashed:
//! true` (the standard `build_q_string`
//! behaviour). Results are ordered by
//! `modifiedTime desc` so the most-recent file
//! appears first.
//!
//! ## Output shape
//!
//! Same `file_summary` shape `drive.search`
//! returns: `{id, name, mime_type, size,
//! modified_at, owner_email,
//! parent_folder_ids}`. The agent paraphrases
//! the list naturally; no relative-time
//! enrichment in Phase 145 (the timestamps
//! Drive returns are already operator-friendly
//! and including a `chrono::Utc::now()` per
//! call would just clutter without adding
//! information the agent can't compute).

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::drive_client::SharedDriveClient;
use crate::tools::search::{build_q_string, file_summary};

const MAX_RESULTS_CAP: u64 = 100;
const DEFAULT_MAX_RESULTS: u64 = 25;
const DEFAULT_WINDOW_DAYS: u64 = 7;
const MAX_WINDOW_DAYS: u64 = 365;

const FILES_LIST_FIELDS: &str =
    "files(id,name,mimeType,size,modifiedTime,owners(emailAddress),parents),nextPageToken";

pub struct DriveRecentFiles {
    id: ToolId,
    schema: Value,
    client: SharedDriveClient,
}

impl DriveRecentFiles {
    pub fn new(client: SharedDriveClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for DriveRecentFiles {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "drive.recent_files"
    }

    fn description(&self) -> &str {
        "List Google Drive files the operator owns \
         that were modified within the last N days. \
         Input is a JSON object with optional \
         `window_days` (default 7, capped at 365), \
         `max_results` (default 25, capped at 100), \
         `include_trashed` (default false), and \
         `parent_folder_id` (optional — when \
         supplied, scopes results to direct children \
         of that folder only; not recursive). \
         Returns `{files: [...], next_page_token, \
         window_days}` sorted by modifiedTime \
         descending — most recent first. The \
         cognitive shape is \"what did I work on \
         this week\"; for broader queries \
         (collaborator activity, shared-with-me \
         files), use `drive.recent_changes` or \
         `drive.search`."
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
                    detail: format!("drive.recent_files: {reason}"),
                });
            }
        };

        let now = Utc::now();
        // Phase 153 — compose the parent clause.
        // Single-folder: `'<id>' in parents`.
        // Recursive: walk the tree first, then
        // OR-join all folder IDs.
        let parent_clause: Option<String> = match (
            parsed.parent_folder_id.as_deref(),
            parsed.recursive,
        ) {
            (None, _) => None,
            (Some(pf), false) => {
                Some(super::compose_recursive_parent_clause(&[pf.to_string()]))
            }
            (Some(pf), true) => {
                let max_depth = parsed
                    .recursive_max_depth
                    .unwrap_or(super::RECURSIVE_MAX_DEPTH);
                let max_folders = parsed
                    .recursive_max_folders
                    .unwrap_or(super::RECURSIVE_MAX_FOLDERS);
                match super::walk_folder_tree(
                    &self.client,
                    pf,
                    max_depth,
                    max_folders,
                    parsed.drive_id.as_deref(),
                    parsed.walk_max_concurrent,
                )
                .await
                {
                    Ok(folder_ids) => {
                        if folder_ids.len() >= max_folders {
                            eprintln!(
                                "drive.recent_files: recursive folder walk hit \
                                 max_folders={} cap; results may miss \
                                 deeper subtrees",
                                max_folders
                            );
                        }
                        Some(super::compose_recursive_parent_clause(&folder_ids))
                    }
                    Err(e) => {
                        return ToolOutcome::Failed(AivyxError::Tool {
                            tool: self.id,
                            detail: format!(
                                "drive.recent_files: recursive walk from {pf:?} failed: {e}"
                            ),
                        });
                    }
                }
            }
        };
        let base_q = build_owned_recent_q(now, parsed.window_days, parent_clause.as_deref());
        let q_string = build_q_string(Some(&base_q), parsed.include_trashed);
        let max_results_str = parsed.max_results.to_string();
        let mut query: Vec<(&str, String)> = vec![
            ("pageSize", max_results_str),
            ("fields", FILES_LIST_FIELDS.to_string()),
            ("orderBy", "modifiedTime desc".to_string()),
        ];
        // Phase 153 — scope to a specific Shared
        // Drive when the operator provides
        // drive_id. The four parameters work
        // together per Google Drive's shared-
        // drives spec:
        // - corpora=drive limits the search to
        //   one corpus.
        // - driveId pins which corpus.
        // - includeItemsFromAllDrives +
        //   supportsAllDrives are required for
        //   the API to surface shared-drive
        //   content.
        if let Some(ref drive_id) = parsed.drive_id {
            query.push(("corpora", "drive".to_string()));
            query.push(("driveId", drive_id.clone()));
            query.push(("includeItemsFromAllDrives", "true".to_string()));
            query.push(("supportsAllDrives", "true".to_string()));
        }
        if let Some(ref q) = q_string {
            query.push(("q", q.clone()));
        }

        let body: Value = match self.client.get_json("/files", &query).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("drive.recent_files: API call failed: {e}"),
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
                "window_days": parsed.window_days,
            }),
            verified: Verification::NotApplicable,
        }
    }
}

/// Compose the `q` clause for owned-files-
/// modified-recently. Pure substrate — `now` is
/// parameterized so unit tests can pin a
/// deterministic timestamp and assert the exact
/// string.
///
/// Phase 148 — optional `parent_clause` appends
/// scope filtering (direct children).
/// Phase 153 — the clause is now a pre-composed
/// string (`'<id>' in parents` for single-folder,
/// or `'<id>' in parents or '<id>' in parents`
/// for recursive). The execute() builds it via
/// [`super::compose_recursive_parent_clause`] so
/// this helper stays pure / single-purpose.
pub(crate) fn build_owned_recent_q(
    now: DateTime<Utc>,
    window_days: u64,
    parent_clause: Option<&str>,
) -> String {
    let since = now - chrono::Duration::days(window_days as i64);
    let mut q = format!(
        "'me' in owners and modifiedTime > '{}'",
        since.to_rfc3339()
    );
    if let Some(clause) = parent_clause {
        q.push_str(&format!(" and ({})", clause));
    }
    q
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "window_days": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_WINDOW_DAYS,
                "description": "Days into the past to scan (default 7, max 365)"
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
            },
            "parent_folder_id": {
                "type": "string",
                "description": "Scope results to direct children of this folder (not recursive unless `recursive: true`)"
            },
            "recursive": {
                "type": "boolean",
                "description": "Phase 153 — when true and parent_folder_id is set, walks the folder tree (max_depth 5, max_folders 100) and matches files anywhere under the root. Default false (direct children only)."
            },
            "drive_id": {
                "type": "string",
                "description": "Phase 153 — scope results to a specific Shared Drive (Team Drive). Pair with `drive.list_drives` to discover IDs. Composable with parent_folder_id and recursive."
            },
            "recursive_max_depth": {
                "type": "integer",
                "minimum": 1,
                "maximum": 20,
                "description": "Phase 157 — override the default recursive walk depth cap. Default 5 (RECURSIVE_MAX_DEPTH); upper bound 20."
            },
            "recursive_max_folders": {
                "type": "integer",
                "minimum": 1,
                "maximum": 1000,
                "description": "Phase 157 — override the default recursive walk folder cap. Default 100 (RECURSIVE_MAX_FOLDERS); upper bound 1000."
            },
            "walk_max_concurrent": {
                "type": "integer",
                "minimum": 1,
                "maximum": 32,
                "description": "Phase 160 — throttle the recursive walk's parallel fan-out. At most N per-folder children-queries run simultaneously per depth level. Default unlimited (level width). Upper bound 32. Useful for rate-limited operators hitting 429s on large folder trees."
            }
        },
        "additionalProperties": false
    })
}

#[derive(Debug)]
struct ParsedInput {
    window_days: u64,
    max_results: u64,
    include_trashed: bool,
    parent_folder_id: Option<String>,
    recursive: bool,
    drive_id: Option<String>,
    /// Phase 157 — operator override for the
    /// walk_folder_tree max_depth. When None,
    /// `RECURSIVE_MAX_DEPTH` (5) is used.
    recursive_max_depth: Option<usize>,
    /// Phase 157 — operator override for the
    /// walk_folder_tree max_folders. When None,
    /// `RECURSIVE_MAX_FOLDERS` (100) is used.
    recursive_max_folders: Option<usize>,
    /// Phase 160 — operator throttle for the
    /// walk's parallel fan-out. When None, the
    /// walk fires unlimited per-folder queries
    /// per depth level (pre-Phase-160 default).
    walk_max_concurrent: Option<usize>,
}

fn parse_input(input: &Value) -> Result<ParsedInput, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
    let window_days = match obj.get("window_days") {
        None => DEFAULT_WINDOW_DAYS,
        Some(v) => v
            .as_u64()
            .ok_or_else(|| "`window_days` must be a positive integer".to_string())?,
    };
    if window_days == 0 {
        return Err("`window_days` must be >= 1".to_string());
    }
    let window_days = window_days.min(MAX_WINDOW_DAYS);

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

    let parent_folder_id = match obj.get("parent_folder_id") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let s = v
                .as_str()
                .ok_or_else(|| "`parent_folder_id` must be a string".to_string())?
                .trim()
                .to_string();
            if s.is_empty() {
                None
            } else if s.contains('\'') {
                // Drive's q DSL uses single
                // quotes to delimit values;
                // embedded quotes would break
                // the composed query. Reject
                // at parse time so the error is
                // clean.
                return Err(
                    "`parent_folder_id` must not contain single quotes".to_string(),
                );
            } else {
                Some(s)
            }
        }
    };

    let recursive = match obj.get("recursive") {
        None => false,
        Some(v) => v
            .as_bool()
            .ok_or_else(|| "`recursive` must be a boolean".to_string())?,
    };

    let drive_id = match obj.get("drive_id") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let s = v
                .as_str()
                .ok_or_else(|| "`drive_id` must be a string".to_string())?
                .trim()
                .to_string();
            if s.is_empty() {
                None
            } else {
                Some(s)
            }
        }
    };

    let recursive_max_depth = parse_recursive_cap(
        obj.get("recursive_max_depth"),
        "recursive_max_depth",
        20,
    )?;
    let recursive_max_folders = parse_recursive_cap(
        obj.get("recursive_max_folders"),
        "recursive_max_folders",
        1000,
    )?;
    let walk_max_concurrent = parse_recursive_cap(
        obj.get("walk_max_concurrent"),
        "walk_max_concurrent",
        32,
    )?;

    Ok(ParsedInput {
        window_days,
        max_results,
        include_trashed,
        parent_folder_id,
        recursive,
        drive_id,
        recursive_max_depth,
        recursive_max_folders,
        walk_max_concurrent,
    })
}

/// Phase 157 — parse one of the recursive cap
/// inputs (max_depth or max_folders). Pure
/// substrate so both recent_files and
/// recent_changes can share the validation.
pub(crate) fn parse_recursive_cap(
    v: Option<&Value>,
    field_name: &str,
    upper_bound: u64,
) -> Result<Option<usize>, String> {
    let raw = match v {
        None | Some(Value::Null) => return Ok(None),
        Some(v) => v
            .as_u64()
            .ok_or_else(|| format!("`{field_name}` must be a positive integer"))?,
    };
    if raw == 0 {
        return Err(format!("`{field_name}` must be >= 1"));
    }
    if raw > upper_bound {
        return Err(format!(
            "`{field_name}` must be <= {upper_bound}; got {raw}"
        ));
    }
    Ok(Some(raw as usize))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now_fixed() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 3, 12, 0, 0).unwrap()
    }

    #[test]
    fn parse_defaults_to_seven_days_25_results() {
        let p = parse_input(&json!({})).unwrap();
        assert_eq!(p.window_days, 7);
        assert_eq!(p.max_results, 25);
        assert!(!p.include_trashed);
    }

    #[test]
    fn parse_honors_explicit_window_and_results() {
        let p = parse_input(&json!({
            "window_days": 30,
            "max_results": 50,
        }))
        .unwrap();
        assert_eq!(p.window_days, 30);
        assert_eq!(p.max_results, 50);
    }

    #[test]
    fn parse_clamps_window_to_one_year() {
        let p = parse_input(&json!({"window_days": 9999})).unwrap();
        assert_eq!(p.window_days, MAX_WINDOW_DAYS);
    }

    #[test]
    fn parse_clamps_max_results_to_one_hundred() {
        let p = parse_input(&json!({"max_results": 9999})).unwrap();
        assert_eq!(p.max_results, MAX_RESULTS_CAP);
    }

    #[test]
    fn parse_zero_window_rejected() {
        let err = parse_input(&json!({"window_days": 0})).unwrap_err();
        assert!(err.contains(">= 1"), "{err}");
    }

    #[test]
    fn parse_include_trashed_honored() {
        let p = parse_input(&json!({"include_trashed": true})).unwrap();
        assert!(p.include_trashed);
    }

    #[test]
    fn build_owned_recent_q_includes_me_clause_and_rfc3339_window() {
        // 7 days before 2026-06-03 noon UTC = 2026-05-27 noon UTC.
        let q = build_owned_recent_q(now_fixed(), 7, None);
        assert!(q.contains("'me' in owners"), "{q}");
        assert!(q.contains("modifiedTime > '2026-05-27T12:00:00"), "{q}");
    }

    #[test]
    fn build_owned_recent_q_one_day_window() {
        let q = build_owned_recent_q(now_fixed(), 1, None);
        assert!(q.contains("modifiedTime > '2026-06-02T12:00:00"), "{q}");
    }

    #[test]
    fn build_owned_recent_q_year_window() {
        let q = build_owned_recent_q(now_fixed(), 365, None);
        assert!(q.contains("modifiedTime > '2025-06-03T12:00:00"), "{q}");
    }

    #[test]
    fn build_q_string_wraps_with_trashed_clause_when_excluded() {
        let base = build_owned_recent_q(now_fixed(), 7, None);
        let wrapped = build_q_string(Some(&base), false).expect("some");
        // standard `build_q_string` prepends
        // `trashed = false and (...)` for the
        // exclude-trash case.
        assert!(wrapped.starts_with("trashed = false and ("), "{wrapped}");
        assert!(wrapped.contains("'me' in owners"), "{wrapped}");
    }

    #[test]
    fn build_q_string_passes_through_when_trashed_included() {
        let base = build_owned_recent_q(now_fixed(), 7, None);
        let q = build_q_string(Some(&base), true).expect("some");
        assert!(!q.contains("trashed = false"), "{q}");
        assert!(q.contains("'me' in owners"), "{q}");
    }

    // ---- Phase 148 — parent_folder_id filter ----

    #[test]
    fn build_owned_recent_q_appends_folder_clause_when_provided() {
        // Phase 153 — helper now takes a pre-
        // composed clause. Single-folder test
        // passes the same shape compose_recursive_parent_clause
        // would produce for one folder.
        let q = build_owned_recent_q(
            now_fixed(),
            7,
            Some("'0AAfolder123' in parents"),
        );
        assert!(q.contains("'me' in owners"), "{q}");
        assert!(q.contains("modifiedTime > '"), "{q}");
        assert!(q.contains("'0AAfolder123' in parents"), "{q}");
    }

    #[test]
    fn build_owned_recent_q_omits_folder_clause_when_none() {
        let q = build_owned_recent_q(now_fixed(), 7, None);
        assert!(!q.contains("in parents"), "{q}");
    }

    #[test]
    fn parse_input_extracts_parent_folder_id() {
        let p = parse_input(&json!({"parent_folder_id": "0AAfolder123"})).unwrap();
        assert_eq!(p.parent_folder_id.as_deref(), Some("0AAfolder123"));
    }

    #[test]
    fn parse_input_null_parent_folder_id_is_none() {
        let p = parse_input(&json!({"parent_folder_id": null})).unwrap();
        assert!(p.parent_folder_id.is_none());
    }

    #[test]
    fn parse_input_empty_string_parent_folder_id_is_none() {
        let p = parse_input(&json!({"parent_folder_id": "   "})).unwrap();
        assert!(p.parent_folder_id.is_none());
    }

    #[test]
    fn parse_input_rejects_parent_folder_id_with_quote() {
        let err = parse_input(&json!({"parent_folder_id": "0AA'inject"})).unwrap_err();
        assert!(err.contains("single quotes"), "{err}");
    }

    // ---- Phase 153 — drive_id + recursive ----

    #[test]
    fn parse_default_recursive_is_false_and_drive_id_none() {
        let p = parse_input(&json!({})).unwrap();
        assert!(!p.recursive);
        assert!(p.drive_id.is_none());
    }

    #[test]
    fn parse_input_extracts_drive_id() {
        let p = parse_input(&json!({"drive_id": "0AAteamdrive"})).unwrap();
        assert_eq!(p.drive_id.as_deref(), Some("0AAteamdrive"));
    }

    #[test]
    fn parse_input_null_drive_id_is_none() {
        let p = parse_input(&json!({"drive_id": null})).unwrap();
        assert!(p.drive_id.is_none());
    }

    #[test]
    fn parse_input_empty_drive_id_is_none() {
        let p = parse_input(&json!({"drive_id": "   "})).unwrap();
        assert!(p.drive_id.is_none());
    }

    #[test]
    fn parse_input_recursive_flag_honored() {
        let p = parse_input(&json!({"recursive": true})).unwrap();
        assert!(p.recursive);
    }

    #[test]
    fn parse_input_drive_id_and_parent_folder_id_compose() {
        let p = parse_input(&json!({
            "drive_id": "0AAteamdrive",
            "parent_folder_id": "0AAfolder123",
            "recursive": true,
        }))
        .unwrap();
        assert_eq!(p.drive_id.as_deref(), Some("0AAteamdrive"));
        assert_eq!(p.parent_folder_id.as_deref(), Some("0AAfolder123"));
        assert!(p.recursive);
    }

    // ---- Phase 157 — recursive cap parsing ----

    #[test]
    fn parse_recursive_cap_absent_yields_none() {
        let out = parse_recursive_cap(None, "x", 20).unwrap();
        assert_eq!(out, None);
    }

    #[test]
    fn parse_recursive_cap_null_yields_none() {
        let v = Value::Null;
        let out = parse_recursive_cap(Some(&v), "x", 20).unwrap();
        assert_eq!(out, None);
    }

    #[test]
    fn parse_recursive_cap_zero_rejected() {
        let v = json!(0);
        let err = parse_recursive_cap(Some(&v), "rmd", 20).unwrap_err();
        assert!(err.contains("`rmd` must be >= 1"));
    }

    #[test]
    fn parse_recursive_cap_over_bound_rejected() {
        let v = json!(99);
        let err = parse_recursive_cap(Some(&v), "rmd", 20).unwrap_err();
        assert!(err.contains("must be <= 20"));
        assert!(err.contains("got 99"));
    }

    #[test]
    fn parse_recursive_cap_negative_rejected() {
        let v = json!(-1);
        let err = parse_recursive_cap(Some(&v), "rmd", 20).unwrap_err();
        assert!(err.contains("positive integer"));
    }

    #[test]
    fn parse_recursive_cap_within_bound_passes() {
        let v = json!(10);
        let out = parse_recursive_cap(Some(&v), "rmd", 20).unwrap();
        assert_eq!(out, Some(10));
    }

    #[test]
    fn parse_input_recursive_max_depth_honored() {
        let p = parse_input(&json!({"recursive_max_depth": 7})).unwrap();
        assert_eq!(p.recursive_max_depth, Some(7));
        assert_eq!(p.recursive_max_folders, None);
    }

    #[test]
    fn parse_input_recursive_max_folders_honored() {
        let p = parse_input(&json!({"recursive_max_folders": 500})).unwrap();
        assert_eq!(p.recursive_max_folders, Some(500));
        assert_eq!(p.recursive_max_depth, None);
    }

    #[test]
    fn parse_input_recursive_max_depth_over_cap_rejected() {
        let err =
            parse_input(&json!({"recursive_max_depth": 99})).unwrap_err();
        assert!(err.contains("recursive_max_depth"));
        assert!(err.contains("<= 20"));
    }

    #[test]
    fn parse_input_recursive_max_folders_over_cap_rejected() {
        let err =
            parse_input(&json!({"recursive_max_folders": 9999})).unwrap_err();
        assert!(err.contains("recursive_max_folders"));
        assert!(err.contains("<= 1000"));
    }

    // ---- Phase 160 — walk_max_concurrent ----

    #[test]
    fn parse_input_walk_max_concurrent_default_none() {
        let p = parse_input(&json!({})).unwrap();
        assert_eq!(p.walk_max_concurrent, None);
    }

    #[test]
    fn parse_input_walk_max_concurrent_honored() {
        let p = parse_input(&json!({"walk_max_concurrent": 8})).unwrap();
        assert_eq!(p.walk_max_concurrent, Some(8));
    }

    #[test]
    fn parse_input_walk_max_concurrent_over_cap_rejected() {
        let err =
            parse_input(&json!({"walk_max_concurrent": 99})).unwrap_err();
        assert!(err.contains("walk_max_concurrent"));
        assert!(err.contains("<= 32"));
    }

    #[test]
    fn parse_input_walk_max_concurrent_zero_rejected() {
        let err =
            parse_input(&json!({"walk_max_concurrent": 0})).unwrap_err();
        assert!(err.contains("walk_max_concurrent"));
        assert!(err.contains(">= 1"));
    }
}
