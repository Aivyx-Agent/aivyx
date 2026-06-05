//! `drive.recent_activity` — recent activity on
//! Google Drive items visible to the operator.
//!
//! Phase 159 — Drive Activity API tool. Where
//! `drive.recent_changes` answers "what *files*
//! moved" (a single record per file with the
//! latest modifiedTime), this tool answers
//! "*what happened* to files": who edited, who
//! shared, who renamed, who commented. Multiple
//! activities per file surface as multiple
//! records.
//!
//! ## API call
//!
//! Single POST to
//! `https://driveactivity.googleapis.com/v2/activity:query`
//! with body:
//!
//! ```json
//! {
//!   "consolidationStrategy": {"legacy": {}},
//!   "filter": "time >= \"<RFC3339>\"",
//!   "pageSize": <max_results>
//! }
//! ```
//!
//! Consolidation strategy is hardcoded to
//! `legacy` (matches Google Drive's "Activity"
//! UI feed). Phase 160+ candidate to make this
//! a knob.
//!
//! ## Output shape
//!
//! Each activity reduces to a flat envelope:
//! `{timestamp, action_type, target_title,
//! target_id, actor_email}`. Multi-action /
//! multi-actor / multi-target activities collapse
//! to their first-element representation — an
//! honest substrate loss for LLM ergonomics.

use async_trait::async_trait;
use chrono::{Duration, Utc};
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::drive_client::SharedDriveClient;

const MAX_RESULTS_CAP: u64 = 100;
const DEFAULT_MAX_RESULTS: u64 = 25;
const DEFAULT_WINDOW_HOURS: u64 = 24;
const MAX_WINDOW_HOURS: u64 = 24 * 30; // 30 days

pub struct DriveRecentActivity {
    id: ToolId,
    schema: Value,
    client: SharedDriveClient,
}

impl DriveRecentActivity {
    pub fn new(client: SharedDriveClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for DriveRecentActivity {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "drive.recent_activity"
    }

    fn description(&self) -> &str {
        "List recent activity (who did what to which file) \
         on Google Drive items visible to the operator. \
         Uses the Drive Activity API \
         (driveactivity.googleapis.com) — distinct from \
         `drive.recent_changes`, which only surfaces \
         modifiedTime per file. This tool surfaces \
         individual events: edits, creates, renames, \
         moves, deletes, comments, permission changes. \
         Input is a JSON object with optional \
         `window_hours` (default 24, capped at 720 = 30 \
         days) and `max_results` (default 25, capped at \
         100). Returns `{activities: [...], count}` \
         where each entry has `timestamp`, `action_type`, \
         `target_title`, `target_id`, `actor_email`. \
         Requires the `drive.activity.readonly` OAuth \
         scope — operators upgrading from a pre-Phase-159 \
         install must re-run `aivyx-drive auth init`."
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
                    detail: format!("drive.recent_activity: {reason}"),
                });
            }
        };

        let now = Utc::now();
        let window_start = now - Duration::hours(parsed.window_hours as i64);
        // Phase 167 — compose the filter with
        // optional action-type clauses.
        let filter =
            compose_filter(&window_start.to_rfc3339(), &parsed.action_type_filter);

        // Phase 167 — consolidation strategy
        // is `{<key>: {}}` shaped per the
        // Activity API contract.
        let consolidation_body = json!({ parsed.consolidation.as_api_key(): {} });
        let mut body = json!({
            "consolidationStrategy": consolidation_body,
            "filter": filter,
            "pageSize": parsed.max_results,
        });
        // Phase 167 — parent_folder_id maps to
        // the Activity API's `ancestorName`
        // request body field. Format: `items/
        // <folder_id>`.
        if let Some(ref pf) = parsed.parent_folder_id {
            body["ancestorName"] = json!(format!("items/{pf}"));
        }

        let response: Value =
            match self.client.post_json_activity("/activity:query", &body).await {
                Ok(v) => v,
                Err(e) => {
                    return ToolOutcome::Failed(AivyxError::Tool {
                        tool: self.id,
                        detail: format!(
                            "drive.recent_activity: Activity API query failed: {e}"
                        ),
                    });
                }
            };

        let raw_activities: Vec<Value> = response
            .get("activities")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let shaped: Vec<Value> = raw_activities
            .iter()
            .map(shape_activity)
            .collect();

        let output = json!({
            "activities": shaped,
            "count": raw_activities.len(),
            "window_hours": parsed.window_hours,
        });

        ToolOutcome::Completed {
            output,
            verified: Verification::NotApplicable,
        }
    }
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "window_hours": {
                "type": "integer",
                "minimum": 1,
                "maximum": 720,
                "description": "Time window in hours back from now. Default 24, capped at 720 (30 days)."
            },
            "max_results": {
                "type": "integer",
                "minimum": 1,
                "maximum": 100,
                "description": "Maximum activities to return. Default 25, capped at 100."
            },
            "action_type_filter": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Phase 167 — restrict results to one or more action types. Accepts case-insensitive variants of: edit, create, rename, delete, move, comment, permissionChange, restore, reference, settingsChange. Builds Activity API `detail.action_detail_case:CASE` filter clauses. Empty array or omitted = no action-type filter."
            },
            "consolidation": {
                "type": "string",
                "enum": ["legacy", "none"],
                "description": "Phase 167 — Activity API consolidation strategy. `legacy` (default) matches the Drive UI's activity feed. `none` returns un-consolidated events (a rename + edit on the same file surfaces as two activities instead of one). The internal `consolidated` strategy isn't exposed in the public API and is intentionally omitted."
            },
            "parent_folder_id": {
                "type": "string",
                "description": "Phase 167 — scope results to activities on items within the parent folder's subtree (recursive via the Activity API's `ancestorName` field). Composes with action_type_filter and the time window. Note: semantics differ from drive.recent_files / drive.recent_changes — those default to direct children with a separate `recursive: true` toggle; the Activity API is always recursive."
            }
        },
        "additionalProperties": false
    })
}

/// Phase 167 — Activity API's action-type
/// enumeration. Hand-maintained; new types
/// added after Phase 167 fail-closed until
/// added here. Each entry maps the operator-
/// facing variant (case-folded) to the
/// upper-snake-case form the API expects in
/// the filter DSL.
pub(crate) const SUPPORTED_ACTION_TYPES: &[(&str, &str)] = &[
    ("edit", "EDIT"),
    ("create", "CREATE"),
    ("rename", "RENAME"),
    ("delete", "DELETE"),
    ("move", "MOVE"),
    ("comment", "COMMENT"),
    ("permissionchange", "PERMISSION_CHANGE"),
    ("restore", "RESTORE"),
    ("reference", "REFERENCE"),
    ("settingschange", "SETTINGS_CHANGE"),
];

/// Phase 167 — Activity API consolidation
/// strategies exposed in the public API.
/// `consolidated` exists internally but isn't
/// publicly addressable; intentionally omitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Consolidation {
    Legacy,
    None,
}

impl Consolidation {
    pub(crate) fn as_api_key(self) -> &'static str {
        match self {
            Consolidation::Legacy => "legacy",
            Consolidation::None => "none",
        }
    }
}

#[derive(Debug)]
struct ParsedInput {
    window_hours: u64,
    max_results: u64,
    /// Phase 167 — upper-snake-case action
    /// names already validated against
    /// `SUPPORTED_ACTION_TYPES`. Empty Vec =
    /// no action-type filter.
    action_type_filter: Vec<String>,
    /// Phase 167 — consolidation strategy.
    /// Default `Legacy` matches Phase 159
    /// behavior.
    consolidation: Consolidation,
    /// Phase 167 — when present, scope to
    /// activities on items within this folder
    /// subtree via the Activity API's
    /// `ancestorName` field. Stored as the
    /// raw Drive folder ID (no `items/`
    /// prefix); execute() composes the
    /// full ancestorName.
    parent_folder_id: Option<String>,
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

    let action_type_filter = parse_action_type_filter(obj.get("action_type_filter"))?;
    let consolidation = parse_consolidation(obj.get("consolidation"))?;
    let parent_folder_id = parse_parent_folder_id(obj.get("parent_folder_id"))?;

    Ok(ParsedInput {
        window_hours,
        max_results,
        action_type_filter,
        consolidation,
        parent_folder_id,
    })
}

/// Phase 167 — parse the `parent_folder_id`
/// input. Returns the raw folder ID (no
/// `items/` prefix) or None when absent. Trims
/// surrounding whitespace; rejects empty
/// strings; rejects IDs containing single
/// quotes (same posture as recent_files /
/// recent_changes — Drive folder IDs are
/// opaque alphanumeric tokens, single quotes
/// would only appear if the input were a
/// query-DSL injection attempt).
pub(crate) fn parse_parent_folder_id(
    v: Option<&Value>,
) -> Result<Option<String>, String> {
    let s = match v {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(s)) => s.trim().to_string(),
        Some(_) => {
            return Err("`parent_folder_id` must be a string".to_string());
        }
    };
    if s.is_empty() {
        return Ok(None);
    }
    if s.contains('\'') {
        return Err(
            "`parent_folder_id` must not contain single quotes".to_string(),
        );
    }
    Ok(Some(s))
}

/// Phase 167 — parse the `consolidation` input.
/// Defaults to `Consolidation::Legacy` when
/// absent or null. Rejects unknown strings
/// (including the public-API-omitted
/// `consolidated`) with a clear error.
pub(crate) fn parse_consolidation(
    v: Option<&Value>,
) -> Result<Consolidation, String> {
    let raw = match v {
        None | Some(Value::Null) => return Ok(Consolidation::Legacy),
        Some(Value::String(s)) => s.trim().to_ascii_lowercase(),
        Some(_) => {
            return Err("`consolidation` must be a string".to_string());
        }
    };
    match raw.as_str() {
        "legacy" => Ok(Consolidation::Legacy),
        "none" => Ok(Consolidation::None),
        "consolidated" => Err(
            "`consolidation: \"consolidated\"` is not exposed in the public \
             Drive Activity API; supported: legacy, none"
                .to_string(),
        ),
        other => Err(format!(
            "unsupported consolidation {other:?}; supported: legacy, none"
        )),
    }
}

/// Phase 167 — parse and normalize the
/// operator's `action_type_filter` input.
/// Returns a Vec of upper-snake-case API names,
/// or an empty Vec when the input is absent /
/// null / empty array. Rejects non-array
/// inputs and unknown type names with a clear
/// error message listing the supported types.
pub(crate) fn parse_action_type_filter(
    v: Option<&Value>,
) -> Result<Vec<String>, String> {
    let arr = match v {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(a)) => a,
        Some(_) => {
            return Err(
                "`action_type_filter` must be an array of strings".to_string(),
            );
        }
    };
    if arr.is_empty() {
        return Ok(Vec::new());
    }
    let mut out: Vec<String> = Vec::with_capacity(arr.len());
    for item in arr {
        let raw = item
            .as_str()
            .ok_or_else(|| {
                "`action_type_filter` entries must be strings".to_string()
            })?
            .trim()
            .to_ascii_lowercase();
        let resolved = SUPPORTED_ACTION_TYPES
            .iter()
            .find(|(folded, _)| *folded == raw.as_str())
            .map(|(_, api)| api.to_string());
        match resolved {
            Some(api) => {
                if !out.contains(&api) {
                    out.push(api);
                }
            }
            None => {
                let supported: Vec<&str> = SUPPORTED_ACTION_TYPES
                    .iter()
                    .map(|(folded, _)| *folded)
                    .collect();
                return Err(format!(
                    "unsupported action_type {raw:?}; supported: {}",
                    supported.join(", ")
                ));
            }
        }
    }
    Ok(out)
}

/// Phase 167 — compose the `filter` field
/// for the Activity API query body. Joins the
/// time-window clause with any
/// `detail.action_detail_case:CASE` clauses
/// using space-separated AND syntax.
pub(crate) fn compose_filter(
    window_start_rfc3339: &str,
    action_types: &[String],
) -> String {
    let mut parts: Vec<String> =
        vec![format!("time >= \"{window_start_rfc3339}\"")];
    for case in action_types {
        parts.push(format!("detail.action_detail_case:{case}"));
    }
    parts.join(" ")
}

/// Phase 159 — shape a raw Activity API entry
/// into the flat agent-ergonomic envelope. Pure
/// substrate so the projection can be tested
/// without live HTTP.
///
/// Strategy: pick the first action / actor /
/// target. Multi-element activities lose detail
/// — operator-visible loss documented in the
/// open doc's honest scope risks.
pub(crate) fn shape_activity(raw: &Value) -> Value {
    let timestamp = extract_timestamp(raw);
    let action_type = extract_action_type(raw);
    let (target_title, target_id) = extract_target(raw);
    let actor_email = extract_actor_email(raw);

    json!({
        "timestamp": timestamp,
        "action_type": action_type,
        "target_title": target_title,
        "target_id": target_id,
        "actor_email": actor_email,
    })
}

fn extract_timestamp(raw: &Value) -> String {
    // Single-point activities use top-level
    // `timestamp`. Multi-point use `timeRange`
    // with `startTime` / `endTime`; we pick
    // endTime as the "this latest happened" time
    // for UI sanity.
    if let Some(ts) = raw.get("timestamp").and_then(|v| v.as_str()) {
        return ts.to_string();
    }
    if let Some(end) = raw
        .get("timeRange")
        .and_then(|v| v.get("endTime"))
        .and_then(|v| v.as_str())
    {
        return end.to_string();
    }
    String::new()
}

fn extract_action_type(raw: &Value) -> String {
    // `primaryActionDetail` is a oneof with
    // single-key variants:
    //   {"edit": {...}}
    //   {"create": {"new": {...}}}
    //   {"rename": {"oldTitle": ..., "newTitle": ...}}
    //   {"delete": {"type": "..."}}
    //   {"move": {"addedParents": ..., "removedParents": ...}}
    //   {"comment": {"post": {...}}}
    //   {"permissionChange": {...}}
    //   {"restore": {...}}
    //   {"reference": {...}}
    // We pick the first key as the type label.
    let Some(detail) = raw.get("primaryActionDetail").and_then(|v| v.as_object())
    else {
        return "unknown".to_string();
    };
    detail
        .keys()
        .next()
        .cloned()
        .unwrap_or_else(|| "unknown".to_string())
}

fn extract_target(raw: &Value) -> (String, String) {
    let Some(targets) = raw.get("targets").and_then(|v| v.as_array()) else {
        return (String::new(), String::new());
    };
    let Some(first) = targets.first() else {
        return (String::new(), String::new());
    };
    let Some(item) = first.get("driveItem") else {
        return (String::new(), String::new());
    };
    let title = item
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let id = item
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    (title, id)
}

fn extract_actor_email(raw: &Value) -> String {
    // `actors` is an array of:
    //   {"user": {"knownUser": {"personName": ..., "isCurrentUser": ...}}}
    //   {"user": {"deletedUser": {}}}
    //   {"user": {"unknownUser": {}}}
    //   {"anonymous": {}}
    //   {"impersonation": {...}}
    //   {"system": {...}}
    //   {"administrator": {...}}
    // The personName field is a People API
    // resource name (e.g. "people/123"); the
    // Activity API doesn't surface the raw email
    // directly. We fall back to descriptive
    // labels for the non-knownUser variants.
    let Some(actors) = raw.get("actors").and_then(|v| v.as_array()) else {
        return "unknown".to_string();
    };
    let Some(first) = actors.first() else {
        return "unknown".to_string();
    };
    if let Some(user) = first.get("user") {
        if let Some(known) = user.get("knownUser") {
            if let Some(name) =
                known.get("personName").and_then(|v| v.as_str())
            {
                return name.to_string();
            }
            return "known user".to_string();
        }
        if user.get("deletedUser").is_some() {
            return "deleted user".to_string();
        }
        if user.get("unknownUser").is_some() {
            return "unknown user".to_string();
        }
    }
    if first.get("anonymous").is_some() {
        return "anonymous".to_string();
    }
    if first.get("system").is_some() {
        return "system".to_string();
    }
    if first.get("administrator").is_some() {
        return "administrator".to_string();
    }
    if first.get("impersonation").is_some() {
        return "impersonation".to_string();
    }
    "unknown".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_name_is_canonical() {
        // Construct a tool via a make-believe
        // client so we can read the name without
        // touching the network.
        let client = make_test_client();
        let tool = DriveRecentActivity::new(client);
        assert_eq!(tool.name(), "drive.recent_activity");
    }

    fn make_test_client() -> SharedDriveClient {
        use crate::drive_client::DriveClient;
        use aivyx_google_oauth::{OAuthConfig, TokenSet};
        use std::sync::Arc;
        Arc::new(DriveClient::new(
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
        ))
    }

    // ---- input parsing ----

    #[test]
    fn parse_defaults_to_24_hours_25_results() {
        let p = parse_input(&json!({})).unwrap();
        assert_eq!(p.window_hours, 24);
        assert_eq!(p.max_results, 25);
    }

    #[test]
    fn parse_honors_explicit_window_and_results() {
        let p = parse_input(&json!({
            "window_hours": 72,
            "max_results": 50,
        }))
        .unwrap();
        assert_eq!(p.window_hours, 72);
        assert_eq!(p.max_results, 50);
    }

    #[test]
    fn parse_clamps_window_to_thirty_days() {
        let p = parse_input(&json!({"window_hours": 9999})).unwrap();
        assert_eq!(p.window_hours, 24 * 30);
    }

    #[test]
    fn parse_clamps_max_results_to_one_hundred() {
        let p = parse_input(&json!({"max_results": 9999})).unwrap();
        assert_eq!(p.max_results, 100);
    }

    #[test]
    fn parse_zero_window_rejected() {
        let err = parse_input(&json!({"window_hours": 0})).unwrap_err();
        assert!(err.contains(">= 1"), "{err}");
    }

    #[test]
    fn parse_zero_max_results_rejected() {
        let err = parse_input(&json!({"max_results": 0})).unwrap_err();
        assert!(err.contains(">= 1"), "{err}");
    }

    // ---- shape_activity substrate ----

    #[test]
    fn shape_edit_activity_with_known_user_actor() {
        let raw = json!({
            "primaryActionDetail": {"edit": {}},
            "timestamp": "2026-06-04T10:00:00Z",
            "actors": [{"user": {"knownUser": {"personName": "people/123"}}}],
            "targets": [{"driveItem": {"name": "items/abc", "title": "Q2 plan"}}],
        });
        let shaped = shape_activity(&raw);
        assert_eq!(shaped["timestamp"], json!("2026-06-04T10:00:00Z"));
        assert_eq!(shaped["action_type"], json!("edit"));
        assert_eq!(shaped["target_title"], json!("Q2 plan"));
        assert_eq!(shaped["target_id"], json!("items/abc"));
        assert_eq!(shaped["actor_email"], json!("people/123"));
    }

    #[test]
    fn shape_create_activity_with_anonymous_actor() {
        let raw = json!({
            "primaryActionDetail": {"create": {"new": {}}},
            "timestamp": "2026-06-04T11:00:00Z",
            "actors": [{"anonymous": {}}],
            "targets": [{"driveItem": {"name": "items/new1", "title": "Untitled"}}],
        });
        let shaped = shape_activity(&raw);
        assert_eq!(shaped["action_type"], json!("create"));
        assert_eq!(shaped["actor_email"], json!("anonymous"));
    }

    #[test]
    fn shape_rename_activity_picks_first_action_type() {
        let raw = json!({
            "primaryActionDetail": {"rename": {"oldTitle": "Old", "newTitle": "New"}},
            "timestamp": "2026-06-04T12:00:00Z",
            "actors": [{"user": {"knownUser": {"personName": "people/xyz"}}}],
            "targets": [{"driveItem": {"name": "items/r", "title": "New"}}],
        });
        let shaped = shape_activity(&raw);
        assert_eq!(shaped["action_type"], json!("rename"));
        assert_eq!(shaped["target_title"], json!("New"));
    }

    #[test]
    fn shape_activity_falls_back_to_time_range_end() {
        let raw = json!({
            "primaryActionDetail": {"edit": {}},
            "timeRange": {
                "startTime": "2026-06-04T09:00:00Z",
                "endTime": "2026-06-04T10:30:00Z",
            },
            "actors": [{"user": {"knownUser": {"personName": "people/a"}}}],
            "targets": [{"driveItem": {"name": "items/x", "title": "T"}}],
        });
        let shaped = shape_activity(&raw);
        assert_eq!(shaped["timestamp"], json!("2026-06-04T10:30:00Z"));
    }

    #[test]
    fn shape_activity_with_deleted_user_actor() {
        let raw = json!({
            "primaryActionDetail": {"delete": {"type": "TRASH"}},
            "timestamp": "2026-06-04T13:00:00Z",
            "actors": [{"user": {"deletedUser": {}}}],
            "targets": [{"driveItem": {"name": "items/d", "title": "Old doc"}}],
        });
        let shaped = shape_activity(&raw);
        assert_eq!(shaped["action_type"], json!("delete"));
        assert_eq!(shaped["actor_email"], json!("deleted user"));
    }

    #[test]
    fn shape_activity_missing_targets_yields_empty_target() {
        let raw = json!({
            "primaryActionDetail": {"edit": {}},
            "timestamp": "2026-06-04T14:00:00Z",
            "actors": [{"user": {"knownUser": {"personName": "people/a"}}}],
            "targets": [],
        });
        let shaped = shape_activity(&raw);
        assert_eq!(shaped["target_title"], json!(""));
        assert_eq!(shaped["target_id"], json!(""));
    }

    #[test]
    fn shape_activity_missing_action_detail_yields_unknown() {
        let raw = json!({
            "timestamp": "2026-06-04T15:00:00Z",
            "actors": [{"user": {"knownUser": {"personName": "people/a"}}}],
            "targets": [{"driveItem": {"name": "items/x", "title": "X"}}],
        });
        let shaped = shape_activity(&raw);
        assert_eq!(shaped["action_type"], json!("unknown"));
    }

    #[test]
    fn shape_activity_system_actor() {
        let raw = json!({
            "primaryActionDetail": {"permissionChange": {}},
            "timestamp": "2026-06-04T16:00:00Z",
            "actors": [{"system": {}}],
            "targets": [{"driveItem": {"name": "items/p", "title": "Doc"}}],
        });
        let shaped = shape_activity(&raw);
        assert_eq!(shaped["action_type"], json!("permissionChange"));
        assert_eq!(shaped["actor_email"], json!("system"));
    }

    #[test]
    fn shape_multi_target_activity_collapses_to_first() {
        let raw = json!({
            "primaryActionDetail": {"move": {}},
            "timestamp": "2026-06-04T17:00:00Z",
            "actors": [{"user": {"knownUser": {"personName": "people/a"}}}],
            "targets": [
                {"driveItem": {"name": "items/first", "title": "First"}},
                {"driveItem": {"name": "items/second", "title": "Second"}},
            ],
        });
        let shaped = shape_activity(&raw);
        assert_eq!(shaped["target_title"], json!("First"));
        assert_eq!(shaped["target_id"], json!("items/first"));
    }

    // ---- Phase 167 — action_type_filter ----

    #[test]
    fn parse_action_type_filter_absent_yields_empty() {
        let p = parse_input(&json!({})).unwrap();
        assert!(p.action_type_filter.is_empty());
    }

    #[test]
    fn parse_action_type_filter_null_yields_empty() {
        let p = parse_input(&json!({"action_type_filter": null})).unwrap();
        assert!(p.action_type_filter.is_empty());
    }

    #[test]
    fn parse_action_type_filter_empty_array_yields_empty() {
        let p = parse_input(&json!({"action_type_filter": []})).unwrap();
        assert!(p.action_type_filter.is_empty());
    }

    #[test]
    fn parse_action_type_filter_normalizes_case_to_api_form() {
        let p = parse_input(&json!({
            "action_type_filter": ["edit", "CREATE", "Rename"]
        }))
        .unwrap();
        assert_eq!(p.action_type_filter, vec!["EDIT", "CREATE", "RENAME"]);
    }

    #[test]
    fn parse_action_type_filter_normalizes_camel_case_to_snake() {
        // permissionChange (camel) → PERMISSION_CHANGE (API).
        let p = parse_input(&json!({
            "action_type_filter": ["permissionChange", "settingsChange"]
        }))
        .unwrap();
        assert_eq!(
            p.action_type_filter,
            vec!["PERMISSION_CHANGE", "SETTINGS_CHANGE"]
        );
    }

    #[test]
    fn parse_action_type_filter_dedupes() {
        let p = parse_input(&json!({
            "action_type_filter": ["edit", "EDIT", "Edit", "create"]
        }))
        .unwrap();
        assert_eq!(p.action_type_filter, vec!["EDIT", "CREATE"]);
    }

    #[test]
    fn parse_action_type_filter_rejects_unknown_type() {
        let err = parse_input(&json!({
            "action_type_filter": ["nuke"]
        }))
        .unwrap_err();
        assert!(err.contains("unsupported"));
        assert!(err.contains("nuke"));
        assert!(err.contains("edit"));
    }

    #[test]
    fn parse_action_type_filter_rejects_non_string_entry() {
        let err = parse_input(&json!({
            "action_type_filter": ["edit", 42]
        }))
        .unwrap_err();
        assert!(err.contains("must be strings"));
    }

    #[test]
    fn parse_action_type_filter_rejects_non_array_input() {
        let err = parse_input(&json!({
            "action_type_filter": "edit"
        }))
        .unwrap_err();
        assert!(err.contains("must be an array"));
    }

    // ---- Phase 167 — compose_filter ----

    #[test]
    fn compose_filter_window_only() {
        let s =
            compose_filter("2026-06-05T10:00:00+00:00", &[]);
        assert_eq!(s, "time >= \"2026-06-05T10:00:00+00:00\"");
    }

    #[test]
    fn compose_filter_window_plus_one_action_type() {
        let s = compose_filter(
            "2026-06-05T10:00:00+00:00",
            &["EDIT".to_string()],
        );
        assert_eq!(
            s,
            "time >= \"2026-06-05T10:00:00+00:00\" detail.action_detail_case:EDIT"
        );
    }

    #[test]
    fn compose_filter_window_plus_multiple_action_types() {
        let s = compose_filter(
            "2026-06-05T10:00:00+00:00",
            &["EDIT".to_string(), "CREATE".to_string()],
        );
        assert_eq!(
            s,
            "time >= \"2026-06-05T10:00:00+00:00\" \
             detail.action_detail_case:EDIT \
             detail.action_detail_case:CREATE"
        );
    }

    #[test]
    fn supported_action_types_pinned_to_phase_167_list() {
        // Regression pin so a future widening
        // is caught at exit-doc time.
        let folded: Vec<&str> =
            SUPPORTED_ACTION_TYPES.iter().map(|(f, _)| *f).collect();
        assert_eq!(folded.len(), 10);
        assert!(folded.contains(&"edit"));
        assert!(folded.contains(&"permissionchange"));
        assert!(folded.contains(&"settingschange"));
    }

    // ---- Phase 167 — consolidation strategy ----

    #[test]
    fn parse_consolidation_defaults_to_legacy() {
        let p = parse_input(&json!({})).unwrap();
        assert_eq!(p.consolidation, Consolidation::Legacy);
    }

    #[test]
    fn parse_consolidation_null_defaults_to_legacy() {
        let p = parse_input(&json!({"consolidation": null})).unwrap();
        assert_eq!(p.consolidation, Consolidation::Legacy);
    }

    #[test]
    fn parse_consolidation_legacy_honored() {
        let p =
            parse_input(&json!({"consolidation": "legacy"})).unwrap();
        assert_eq!(p.consolidation, Consolidation::Legacy);
    }

    #[test]
    fn parse_consolidation_none_honored() {
        let p = parse_input(&json!({"consolidation": "none"})).unwrap();
        assert_eq!(p.consolidation, Consolidation::None);
    }

    #[test]
    fn parse_consolidation_case_insensitive() {
        let p =
            parse_input(&json!({"consolidation": "LEGACY"})).unwrap();
        assert_eq!(p.consolidation, Consolidation::Legacy);
        let p =
            parse_input(&json!({"consolidation": "  None  "})).unwrap();
        assert_eq!(p.consolidation, Consolidation::None);
    }

    #[test]
    fn parse_consolidation_consolidated_rejected_with_explanation() {
        // The `consolidated` strategy isn't
        // exposed in the public Drive Activity
        // API; we surface this honestly
        // instead of silently sending it.
        let err =
            parse_input(&json!({"consolidation": "consolidated"}))
                .unwrap_err();
        assert!(err.contains("not exposed in the public"));
        assert!(err.contains("supported: legacy, none"));
    }

    #[test]
    fn parse_consolidation_unknown_rejected() {
        let err =
            parse_input(&json!({"consolidation": "magic"})).unwrap_err();
        assert!(err.contains("magic"));
        assert!(err.contains("supported: legacy, none"));
    }

    #[test]
    fn parse_consolidation_non_string_rejected() {
        let err =
            parse_input(&json!({"consolidation": 7})).unwrap_err();
        assert!(err.contains("must be a string"));
    }

    #[test]
    fn consolidation_as_api_key_matches_request_body_shape() {
        // The body builds `{<as_api_key>: {}}`;
        // pin the strings so a future rename
        // doesn't silently break the API
        // contract.
        assert_eq!(Consolidation::Legacy.as_api_key(), "legacy");
        assert_eq!(Consolidation::None.as_api_key(), "none");
    }

    // ---- Phase 167 — parent_folder_id ----

    #[test]
    fn parse_parent_folder_id_absent_yields_none() {
        let p = parse_input(&json!({})).unwrap();
        assert_eq!(p.parent_folder_id, None);
    }

    #[test]
    fn parse_parent_folder_id_null_yields_none() {
        let p =
            parse_input(&json!({"parent_folder_id": null})).unwrap();
        assert_eq!(p.parent_folder_id, None);
    }

    #[test]
    fn parse_parent_folder_id_empty_string_yields_none() {
        let p = parse_input(&json!({"parent_folder_id": ""})).unwrap();
        assert_eq!(p.parent_folder_id, None);
    }

    #[test]
    fn parse_parent_folder_id_extracts_string_value() {
        let p = parse_input(&json!({
            "parent_folder_id": "0AAfolder123"
        }))
        .unwrap();
        assert_eq!(p.parent_folder_id, Some("0AAfolder123".to_string()));
    }

    #[test]
    fn parse_parent_folder_id_trims_whitespace() {
        let p = parse_input(&json!({
            "parent_folder_id": "  abc-123  "
        }))
        .unwrap();
        assert_eq!(p.parent_folder_id, Some("abc-123".to_string()));
    }

    #[test]
    fn parse_parent_folder_id_rejects_single_quote() {
        // Same posture as recent_files /
        // recent_changes — guards against
        // query-DSL injection attempts.
        let err = parse_input(&json!({
            "parent_folder_id": "0AA'inject"
        }))
        .unwrap_err();
        assert!(err.contains("single quotes"));
    }

    #[test]
    fn parse_parent_folder_id_rejects_non_string() {
        let err = parse_input(&json!({
            "parent_folder_id": 42
        }))
        .unwrap_err();
        assert!(err.contains("must be a string"));
    }
}
