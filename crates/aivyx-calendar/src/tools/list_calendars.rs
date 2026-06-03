//! `calendar.list_calendars` — enumerate the
//! calendars the operator has access to.
//!
//! Phase 142. Calls
//! `GET /users/me/calendarList` and normalizes
//! the response to a stable snake-case shape:
//!
//! ```json
//! {
//!   "calendars": [
//!     {
//!       "id": "primary",
//!       "summary": "Personal",
//!       "is_primary": true,
//!       "access_role": "owner"
//!     },
//!     {
//!       "id": "work@example.com",
//!       "summary": "Work",
//!       "is_primary": false,
//!       "access_role": "writer"
//!     }
//!   ]
//! }
//! ```
//!
//! The agent typically calls this once at the
//! start of a conversation to learn which
//! calendars exist, then passes specific IDs to
//! `calendar.upcoming` (multi-calendar shape) or
//! `calendar.list_events`.

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::calendar_client::SharedCalendarClient;

/// `calendar.list_calendars` tool. Read-only
/// enumeration; held client reused across all
/// tools in this binary.
pub struct CalendarListCalendars {
    id: ToolId,
    schema: Value,
    client: SharedCalendarClient,
}

impl CalendarListCalendars {
    pub fn new(client: SharedCalendarClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for CalendarListCalendars {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "calendar.list_calendars"
    }

    fn description(&self) -> &str {
        "Enumerate the Google Calendars the \
         operator has access to. Takes no \
         arguments. Returns a JSON object with a \
         `calendars` array of \
         `{id, summary, is_primary, access_role, \
         can_read, can_write}` entries. \
         `access_role` is one of \"owner\", \
         \"writer\", \"reader\", or \
         \"freeBusyReader\" — passes through \
         from Google's API verbatim. Phase 151 \
         derived booleans: `can_read` is true for \
         owner/writer/reader (the calendars whose \
         event content is visible); `can_write` \
         is true for owner/writer. \
         `freeBusyReader` calendars are both \
         false (free/busy times are visible but \
         event details aren't). Use this once \
         per conversation to learn which \
         calendar IDs exist, then pass specific \
         IDs to `calendar.upcoming` (via \
         `calendar_ids`) or `calendar.list_events` \
         (via `calendar_id`). Without this, the \
         agent only knows the default \"primary\" \
         calendar."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("calendar.read").expect(
            "calendar.read must parse — it is in KNOWN_BASES from Phase 128",
        )
    }

    async fn execute(&self, _input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        // No paginated multi-page fetch in Phase
        // 142 — most operators have <50 calendars,
        // Google's default page size is 100, and
        // /users/me/calendarList returns a
        // single page for typical usage. If
        // operators with 100+ calendars surface,
        // Phase 143+ adds pagination.
        let body: Value = match self
            .client
            .get_json("/users/me/calendarList", &[])
            .await
        {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("calendar.list_calendars: API call failed: {e}"),
                });
            }
        };

        let calendars: Vec<Value> = body
            .get("items")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().map(calendar_summary).collect())
            .unwrap_or_default();

        ToolOutcome::Completed {
            output: json!({ "calendars": calendars }),
            verified: Verification::NotApplicable,
        }
    }
}

/// Pure-substrate mapper: one Google
/// `calendarList.items[]` entry → the LLM-
/// ergonomic shape. Designed to surface
/// exactly the fields the agent needs to make
/// follow-up tool calls (id), explain itself
/// (summary), and reason about permissions
/// (is_primary, access_role, can_read,
/// can_write).
///
/// Phase 151 adds `can_read` + `can_write`
/// booleans derived from `access_role`. The
/// raw `access_role` string still surfaces
/// alongside; the new fields are purely
/// additive so existing consumers don't
/// break.
pub(crate) fn calendar_summary(cal: &Value) -> Value {
    let id = cal.get("id").cloned().unwrap_or(Value::Null);
    let summary = cal.get("summary").cloned().unwrap_or(Value::Null);
    let is_primary = cal
        .get("primary")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let access_role = cal
        .get("accessRole")
        .cloned()
        .unwrap_or(Value::Null);
    let (can_read, can_write) = capability_from_access_role(
        access_role.as_str().unwrap_or(""),
    );
    json!({
        "id": id,
        "summary": summary,
        "is_primary": is_primary,
        "access_role": access_role,
        "can_read": can_read,
        "can_write": can_write,
    })
}

/// Phase 151 — map Google Calendar's
/// `accessRole` string to the operator-facing
/// `(can_read, can_write)` boolean pair the
/// agent can reason about directly.
///
/// Mapping:
/// - `owner` / `writer` → can_write = true.
/// - `owner` / `writer` / `reader` →
///   can_read = true (read event content).
/// - `freeBusyReader` → both false (can read
///   busy times but not event content; the
///   semantic of "read events" is the latter
///   for the agent's purposes).
/// - Anything else / null → both false
///   (conservative; better to under-promise).
pub(crate) fn capability_from_access_role(role: &str) -> (bool, bool) {
    match role {
        "owner" | "writer" => (true, true),
        "reader" => (true, false),
        // freeBusyReader and unknowns: both false.
        _ => (false, false),
    }
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

    #[test]
    fn calendar_summary_maps_primary_owner_calendar() {
        let raw = json!({
            "id": "primary",
            "summary": "Personal",
            "primary": true,
            "accessRole": "owner",
            "backgroundColor": "#9fe1e7",
        });
        let got = calendar_summary(&raw);
        assert_eq!(got["id"], json!("primary"));
        assert_eq!(got["summary"], json!("Personal"));
        assert_eq!(got["is_primary"], json!(true));
        assert_eq!(got["access_role"], json!("owner"));
        // Phase 151 — owner gets full caps.
        assert_eq!(got["can_read"], json!(true));
        assert_eq!(got["can_write"], json!(true));
        // Fields not in the output shape don't
        // appear.
        assert!(got.get("backgroundColor").is_none());
    }

    #[test]
    fn calendar_summary_maps_secondary_writer_calendar() {
        // Google omits `primary` for non-primary
        // calendars — verify the bool defaults to
        // false rather than null.
        let raw = json!({
            "id": "work@example.com",
            "summary": "Work",
            "accessRole": "writer",
        });
        let got = calendar_summary(&raw);
        assert_eq!(got["id"], json!("work@example.com"));
        assert_eq!(got["summary"], json!("Work"));
        assert_eq!(got["is_primary"], json!(false));
        assert_eq!(got["access_role"], json!("writer"));
    }

    #[test]
    fn calendar_summary_handles_freebusy_reader() {
        // Calendars the operator has only
        // free/busy access to (shared
        // pseudo-calendars) come back with
        // accessRole=freeBusyReader. The agent
        // sees the role verbatim.
        let raw = json!({
            "id": "shared@example.com",
            "summary": null,
            "accessRole": "freeBusyReader",
        });
        let got = calendar_summary(&raw);
        assert_eq!(got["access_role"], json!("freeBusyReader"));
        assert_eq!(got["summary"], Value::Null);
    }

    #[test]
    fn calendar_summary_handles_empty_input() {
        // Pathological: empty object.
        // Defensively returns nulls + false rather
        // than panicking.
        let got = calendar_summary(&json!({}));
        assert_eq!(got["id"], Value::Null);
        assert_eq!(got["summary"], Value::Null);
        assert_eq!(got["is_primary"], json!(false));
        assert_eq!(got["access_role"], Value::Null);
        // Phase 151 — null access_role conserves
        // to both-false caps.
        assert_eq!(got["can_read"], json!(false));
        assert_eq!(got["can_write"], json!(false));
    }

    // ---- Phase 151 — capability_from_access_role ----

    #[test]
    fn capability_owner_grants_read_and_write() {
        let (r, w) = capability_from_access_role("owner");
        assert!(r);
        assert!(w);
    }

    #[test]
    fn capability_writer_grants_read_and_write() {
        let (r, w) = capability_from_access_role("writer");
        assert!(r);
        assert!(w);
    }

    #[test]
    fn capability_reader_grants_read_only() {
        let (r, w) = capability_from_access_role("reader");
        assert!(r);
        assert!(!w);
    }

    #[test]
    fn capability_free_busy_reader_grants_neither() {
        // Read free/busy times only; can't see
        // event content. Phase 151 semantic
        // choice: "can_read = false" because
        // "read events" semantically means
        // "read event content" not "read busy
        // times."
        let (r, w) = capability_from_access_role("freeBusyReader");
        assert!(!r);
        assert!(!w);
    }

    #[test]
    fn capability_unknown_role_grants_neither() {
        // Conservative posture for forward-compat
        // — better to under-promise than over-
        // promise on an unrecognized role.
        let (r, w) = capability_from_access_role("supersecret-future-role");
        assert!(!r);
        assert!(!w);
    }

    #[test]
    fn capability_empty_string_grants_neither() {
        let (r, w) = capability_from_access_role("");
        assert!(!r);
        assert!(!w);
    }

    #[test]
    fn calendar_summary_writer_is_full_caps() {
        // Phase 151 — exercise the writer path
        // end-to-end through calendar_summary
        // (the test above covers owner; this
        // one covers writer for completeness).
        let raw = json!({
            "id": "team@example.com",
            "summary": "Team Calendar",
            "accessRole": "writer",
        });
        let got = calendar_summary(&raw);
        assert_eq!(got["access_role"], json!("writer"));
        assert_eq!(got["can_read"], json!(true));
        assert_eq!(got["can_write"], json!(true));
    }

    #[test]
    fn calendar_summary_reader_is_read_only_caps() {
        let raw = json!({
            "id": "shared@example.com",
            "summary": "Shared Read-Only",
            "accessRole": "reader",
        });
        let got = calendar_summary(&raw);
        assert_eq!(got["access_role"], json!("reader"));
        assert_eq!(got["can_read"], json!(true));
        assert_eq!(got["can_write"], json!(false));
    }
}
