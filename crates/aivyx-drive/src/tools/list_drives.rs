//! `drive.list_drives` — enumerate shared drives
//! (Google's "Team Drives") the operator has
//! access to.
//!
//! Phase 148. Mirrors Phase 142's
//! `calendar.list_calendars` pattern: the agent
//! calls this once to discover which shared
//! drives exist, then uses the IDs as
//! `drive.search` `corpora=drive` arguments or
//! future Phase 149+ `drive_id`-scoped
//! recent_* queries.
//!
//! ## API call
//!
//! Single GET to `/drives`. No `corpora` or `q`
//! parameter — we want the full enumeration of
//! shared drives the operator is a member of.
//! Field selection trimmed to keep the response
//! compact for LLM consumption.
//!
//! ## Output shape
//!
//! ```json
//! {
//!   "drives": [
//!     {
//!       "id": "0AAABbCcc...",
//!       "name": "Aivyx Working Group",
//!       "created_at": "2026-01-15T08:00:00Z"
//!     }
//!   ]
//! }
//! ```
//!
//! No `next_page_token` surfaced in Phase 148 —
//! most operators are members of <100 shared
//! drives (Google's default page size) and the
//! pagination story for "list every Team Drive
//! I'm in" is Phase 149+ if 100+-drive operators
//! surface.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::drive_client::SharedDriveClient;

const DRIVES_LIST_FIELDS: &str = "drives(id,name,createdTime),nextPageToken";

pub struct DriveListDrives {
    id: ToolId,
    schema: Value,
    client: SharedDriveClient,
}

impl DriveListDrives {
    pub fn new(client: SharedDriveClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for DriveListDrives {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "drive.list_drives"
    }

    // Chapter Picket follow-up (Finding 3) — shared-drive names can
    // be set by any collaborator; externally authored metadata.
    fn output_is_untrusted(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Enumerate the Google Shared Drives \
         (formerly Team Drives) the operator is \
         a member of. Takes no arguments. Returns \
         `{drives: [{id, name, created_at}]}`. \
         Use this once per conversation so the \
         agent knows which shared drive IDs exist; \
         then pass specific IDs to `drive.search` \
         via its query DSL (`'<drive_id>' in \
         parents` plus the `corpora=drive` query \
         param, which `drive.search` exposes via \
         hand-written `q`). Files in the \
         operator's My Drive are NOT listed here — \
         use `drive.recent_files` / \
         `drive.recent_changes` for those."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("drive.read").expect(
            "drive.read must parse — it is in KNOWN_BASES from Phase 129",
        )
    }

    async fn execute(&self, _input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let query: Vec<(&str, String)> =
            vec![("fields", DRIVES_LIST_FIELDS.to_string())];

        let body: Value = match self.client.get_json("/drives", &query).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("drive.list_drives: API call failed: {e}"),
                });
            }
        };

        let drives: Vec<Value> = body
            .get("drives")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().map(drive_summary).collect())
            .unwrap_or_default();

        ToolOutcome::Completed {
            output: json!({ "drives": drives }),
            verified: Verification::NotApplicable,
        }
    }
}

/// Pure-substrate mapper: one Google
/// `drives.items[]` entry → the LLM-ergonomic
/// shape. Renames `createdTime` to `created_at`
/// (snake_case canonical) and surfaces only the
/// fields the agent needs.
pub(crate) fn drive_summary(drive: &Value) -> Value {
    let id = drive.get("id").cloned().unwrap_or(Value::Null);
    let name = drive.get("name").cloned().unwrap_or(Value::Null);
    let created_at = drive
        .get("createdTime")
        .cloned()
        .unwrap_or(Value::Null);
    json!({
        "id": id,
        "name": name,
        "created_at": created_at,
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool() -> DriveListDrives {
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
        DriveListDrives::new(client)
    }

    #[test]
    fn drive_list_drives_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }

    #[test]
    fn drive_summary_maps_full_entry() {
        let raw = json!({
            "id": "0AABbCcc-team-drive-id",
            "name": "Aivyx Working Group",
            "createdTime": "2026-01-15T08:00:00Z",
            // Google may return extra fields the
            // agent doesn't need. They must NOT
            // appear in the normalized output.
            "kind": "drive#drive",
            "colorRgb": "#000000",
        });
        let got = drive_summary(&raw);
        assert_eq!(got["id"], json!("0AABbCcc-team-drive-id"));
        assert_eq!(got["name"], json!("Aivyx Working Group"));
        assert_eq!(got["created_at"], json!("2026-01-15T08:00:00Z"));
        // Extra fields dropped.
        assert!(got.get("kind").is_none());
        assert!(got.get("colorRgb").is_none());
    }

    #[test]
    fn drive_summary_handles_missing_optional_fields() {
        let raw = json!({
            "id": "0AABbCcc-no-time",
            "name": "Sparse Drive",
            // createdTime omitted — Google's
            // response sometimes drops fields the
            // viewer can't access. We defensively
            // surface null.
        });
        let got = drive_summary(&raw);
        assert_eq!(got["id"], json!("0AABbCcc-no-time"));
        assert_eq!(got["name"], json!("Sparse Drive"));
        assert_eq!(got["created_at"], Value::Null);
    }

    #[test]
    fn drive_summary_handles_empty_input() {
        // Pathological: completely empty entry.
        // Defensively returns nulls rather than
        // panicking.
        let got = drive_summary(&json!({}));
        assert_eq!(got["id"], Value::Null);
        assert_eq!(got["name"], Value::Null);
        assert_eq!(got["created_at"], Value::Null);
    }

    #[test]
    fn input_schema_has_no_required_arguments() {
        let s = input_schema();
        assert_eq!(s["type"], "object");
        // No `required` key at all — the schema
        // is permissive on absent input.
        assert!(s.get("required").is_none());
    }
}
