//! `calendar.list_events` — range-query a Google Calendar.
//!
//! Phase 128 Task 4. The first Calendar tool. Implements
//! [`aivyx_core::Tool`]; served by the lifted multi-tool
//! harness in `aivyx_tool::multi_harness`.
//!
//! ## API call
//!
//! Single GET to `/calendars/{calendarId}/events` with
//! `timeMin` / `timeMax` / `maxResults` / `singleEvents`
//! query parameters per the Google Calendar v3 events.list
//! reference. Recurring events are expanded into individual
//! instances (`singleEvents=true`) so the LLM sees one
//! event per occurrence rather than a recurrence template.
//!
//! ## Output shape
//!
//! Stable snake-case JSON; one entry per event with the
//! fields most LLM-useful for tool composition:
//!
//! ```json
//! {
//!   "events": [
//!     {
//!       "id": "abc123",
//!       "summary": "Team standup",
//!       "start": "2026-06-10T09:00:00-07:00",
//!       "end": "2026-06-10T09:30:00-07:00",
//!       "location": "Conf room A",
//!       "attendee_count": 5
//!     }
//!   ],
//!   "next_page_token": null
//! }
//! ```

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::calendar_client::SharedCalendarClient;

/// Hard cap on `max_results` per call. Google's API
/// supports up to 2500 per page; for a single tool call
/// we cap at 250 — keeps the response size predictable
/// for the LLM without preventing operator-scripted bulk
/// fetches.
const MAX_RESULTS_CAP: u64 = 250;
const DEFAULT_MAX_RESULTS: u64 = 50;
const DEFAULT_CALENDAR_ID: &str = "primary";

/// `calendar.list_events` tool. Holds a shared Calendar
/// client; the tool itself is stateless beyond the client
/// + a fixed input schema.
pub struct CalendarListEvents {
    id: ToolId,
    schema: Value,
    client: SharedCalendarClient,
}

impl CalendarListEvents {
    pub fn new(client: SharedCalendarClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for CalendarListEvents {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "calendar.list_events"
    }

    fn description(&self) -> &str {
        "List events on a Google Calendar within a time \
         range. Input is a JSON object with optional \
         `time_min` (RFC 3339 timestamp; lower bound, \
         exclusive), `time_max` (RFC 3339 timestamp; upper \
         bound, exclusive), `max_results` (default 50, \
         capped at 250), and `calendar_id` (default \
         `\"primary\"`). Returns a JSON object with an \
         `events` array of `{id, summary, start, end, \
         location, attendee_count}` entries and an \
         optional `next_page_token` for pagination. \
         Recurring events are expanded to individual \
         occurrences. Returns ONLY the visible-to-the- \
         operator subset of fields — pass each `id` to \
         `calendar.get_event` for the full event detail."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("calendar.read").expect(
            "calendar.read must parse — it is in KNOWN_BASES from Phase 128",
        )
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let parsed = match parse_input(&input) {
            Ok(p) => p,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("calendar.list_events: {reason}"),
                });
            }
        };

        let path = format!(
            "/calendars/{}/events",
            super::list_events_urlencode(&parsed.calendar_id)
        );
        let max_results_str = parsed.max_results.to_string();
        let mut query: Vec<(&str, String)> = vec![
            ("maxResults", max_results_str),
            ("singleEvents", "true".to_string()),
            ("orderBy", "startTime".to_string()),
        ];
        if let Some(ref t) = parsed.time_min {
            query.push(("timeMin", t.clone()));
        }
        if let Some(ref t) = parsed.time_max {
            query.push(("timeMax", t.clone()));
        }

        let body: Value = match self.client.get_json(&path, &query).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("calendar.list_events: API call failed: {e}"),
                });
            }
        };

        let events: Vec<Value> = body
            .get("items")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().map(super::event_summary).collect())
            .unwrap_or_default();
        let next_page_token = body
            .get("nextPageToken")
            .and_then(|v| v.as_str())
            .map(String::from);

        let mut output = json!({
            "events": events,
        });
        output["next_page_token"] = match next_page_token {
            Some(t) => Value::String(t),
            None => Value::Null,
        };

        ToolOutcome::Completed {
            output,
            // Read-only query — verification semantics are
            // "not meaningful" per TOOL_SDK.md.
            verified: Verification::NotApplicable,
        }
    }
}

// `event_summary` + `flatten_timestamp` lifted
// to `super` in Phase 141 so `calendar.upcoming`
// can reuse them. See `tools/mod.rs`.

#[derive(Debug)]
struct ParsedInput {
    time_min: Option<String>,
    time_max: Option<String>,
    max_results: u64,
    calendar_id: String,
}

fn parse_input(input: &Value) -> Result<ParsedInput, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;

    let time_min = parse_optional_rfc3339(obj.get("time_min"), "time_min")?;
    let time_max = parse_optional_rfc3339(obj.get("time_max"), "time_max")?;

    let max_results = match obj.get("max_results") {
        None => DEFAULT_MAX_RESULTS,
        Some(v) => v.as_u64().ok_or_else(|| {
            "`max_results` must be a non-negative integer".to_string()
        })?,
    };
    if max_results == 0 {
        return Err("`max_results` must be >= 1".to_string());
    }
    let max_results = max_results.min(MAX_RESULTS_CAP);

    let calendar_id = match obj.get("calendar_id") {
        None => DEFAULT_CALENDAR_ID.to_string(),
        Some(v) => v
            .as_str()
            .ok_or_else(|| "`calendar_id` must be a string".to_string())?
            .trim()
            .to_string(),
    };
    if calendar_id.is_empty() {
        return Err("`calendar_id` must not be empty".to_string());
    }

    Ok(ParsedInput {
        time_min,
        time_max,
        max_results,
        calendar_id,
    })
}

/// Validate that a JSON field is either absent, null, or
/// a non-empty string that looks like RFC 3339. The check
/// is intentionally lightweight — Google's API rejects
/// malformed timestamps with a clear error, so a
/// pre-flight format-verifier would be redundant. We just
/// catch the obvious mistakes (boolean, integer, empty
/// string).
fn parse_optional_rfc3339(value: Option<&Value>, field: &str) -> Result<Option<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Err(format!("`{field}` must not be empty"));
            }
            // Cheap structural check: an RFC 3339 timestamp
            // must contain at least one digit, one `T` or
            // `-` separator. Don't go deeper — Google
            // rejects malformed input with a clear error.
            if !trimmed.chars().any(|c| c.is_ascii_digit()) {
                return Err(format!(
                    "`{field}` must look like an RFC 3339 timestamp (e.g. 2026-06-10T00:00:00Z)"
                ));
            }
            Ok(Some(trimmed.to_string()))
        }
        Some(_) => Err(format!("`{field}` must be a string (RFC 3339 timestamp)")),
    }
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "time_min": {
                "type": ["string", "null"],
                "description": "Lower bound (exclusive) for the event's end time as an RFC 3339 timestamp (e.g. `2026-06-10T00:00:00Z`). Optional — if omitted, no lower bound is applied."
            },
            "time_max": {
                "type": ["string", "null"],
                "description": "Upper bound (exclusive) for the event's start time as an RFC 3339 timestamp. Optional."
            },
            "max_results": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_RESULTS_CAP,
                "default": DEFAULT_MAX_RESULTS,
                "description": "Maximum events to return in this call. Capped at 250."
            },
            "calendar_id": {
                "type": "string",
                "default": DEFAULT_CALENDAR_ID,
                "description": "Google Calendar ID. `\"primary\"` for the authenticated user's main calendar; otherwise the calendar's address (e.g. `team@example.com`)."
            }
        },
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::super::event_summary;
    use super::*;

    #[test]
    fn parse_input_defaults_when_all_optional_omitted() {
        let p = parse_input(&json!({})).expect("parse");
        assert!(p.time_min.is_none());
        assert!(p.time_max.is_none());
        assert_eq!(p.max_results, DEFAULT_MAX_RESULTS);
        assert_eq!(p.calendar_id, DEFAULT_CALENDAR_ID);
    }

    #[test]
    fn parse_input_accepts_rfc3339_timestamps() {
        let input = json!({
            "time_min": "2026-06-10T00:00:00Z",
            "time_max": "2026-06-11T00:00:00Z",
        });
        let p = parse_input(&input).expect("parse");
        assert_eq!(p.time_min.as_deref(), Some("2026-06-10T00:00:00Z"));
        assert_eq!(p.time_max.as_deref(), Some("2026-06-11T00:00:00Z"));
    }

    #[test]
    fn parse_input_caps_max_results_at_cap() {
        let p = parse_input(&json!({"max_results": 9999})).expect("parse");
        assert_eq!(p.max_results, MAX_RESULTS_CAP);
    }

    #[test]
    fn parse_input_rejects_zero_max_results() {
        let e = parse_input(&json!({"max_results": 0})).expect_err("must error");
        assert!(e.contains(">= 1"), "{e}");
    }

    #[test]
    fn parse_input_rejects_non_integer_max_results() {
        let e = parse_input(&json!({"max_results": "lots"})).expect_err("must error");
        assert!(e.contains("non-negative integer"), "{e}");
    }

    #[test]
    fn parse_input_rejects_empty_time_min() {
        let e = parse_input(&json!({"time_min": ""})).expect_err("must error");
        assert!(e.contains("time_min"), "{e}");
    }

    #[test]
    fn parse_input_rejects_non_string_time_min() {
        let e = parse_input(&json!({"time_min": 12345})).expect_err("must error");
        assert!(e.contains("string"), "{e}");
    }

    #[test]
    fn parse_input_rejects_time_min_with_no_digits() {
        // "tomorrow" is operator shorthand the model might
        // try; we reject so the API call doesn't 400 with a
        // less-actionable Google error.
        let e = parse_input(&json!({"time_min": "tomorrow"})).expect_err("must error");
        assert!(e.contains("RFC 3339"), "{e}");
    }

    #[test]
    fn parse_input_rejects_non_string_calendar_id() {
        let e = parse_input(&json!({"calendar_id": true})).expect_err("must error");
        assert!(e.contains("calendar_id"), "{e}");
    }

    #[test]
    fn parse_input_rejects_empty_calendar_id() {
        let e = parse_input(&json!({"calendar_id": "   "})).expect_err("must error");
        assert!(e.contains("calendar_id"), "{e}");
    }

    #[test]
    fn parse_input_accepts_email_style_calendar_id() {
        let p =
            parse_input(&json!({"calendar_id": "team@example.com"})).expect("parse");
        assert_eq!(p.calendar_id, "team@example.com");
    }

    #[test]
    fn event_summary_flattens_date_time_form() {
        let event = json!({
            "id": "abc",
            "summary": "standup",
            "start": {"dateTime": "2026-06-10T09:00:00-07:00", "timeZone": "America/Los_Angeles"},
            "end": {"dateTime": "2026-06-10T09:30:00-07:00", "timeZone": "America/Los_Angeles"},
            "location": "Conf A",
            "attendees": [{"email": "a@x"}, {"email": "b@x"}, {"email": "c@x"}],
        });
        let s = event_summary(&event);
        assert_eq!(s["id"], "abc");
        assert_eq!(s["summary"], "standup");
        assert_eq!(s["start"], "2026-06-10T09:00:00-07:00");
        assert_eq!(s["end"], "2026-06-10T09:30:00-07:00");
        assert_eq!(s["location"], "Conf A");
        assert_eq!(s["attendee_count"], 3);
    }

    #[test]
    fn event_summary_flattens_all_day_date_form() {
        let event = json!({
            "id": "all-day",
            "summary": "Holiday",
            "start": {"date": "2026-12-25"},
            "end": {"date": "2026-12-26"},
        });
        let s = event_summary(&event);
        assert_eq!(s["start"], "2026-12-25");
        assert_eq!(s["end"], "2026-12-26");
        assert_eq!(s["attendee_count"], 0);
        assert!(s["location"].is_null());
    }

    #[test]
    fn event_summary_handles_missing_optional_fields() {
        let event = json!({"id": "minimal"});
        let s = event_summary(&event);
        assert_eq!(s["id"], "minimal");
        assert!(s["summary"].is_null());
        assert!(s["start"].is_null());
        assert!(s["end"].is_null());
        assert!(s["location"].is_null());
        assert_eq!(s["attendee_count"], 0);
    }

    #[test]
    fn input_schema_declares_no_required_fields() {
        let schema = input_schema();
        // All fields optional — operators can call with
        // `{}` to list events from the primary calendar
        // with no time bounds.
        let req = schema.get("required");
        assert!(req.is_none() || req.unwrap().as_array().unwrap().is_empty());
        assert_eq!(schema["additionalProperties"], false);
    }

    #[test]
    fn input_schema_bounds_max_results() {
        let schema = input_schema();
        assert_eq!(schema["properties"]["max_results"]["maximum"], MAX_RESULTS_CAP);
        assert_eq!(schema["properties"]["max_results"]["minimum"], 1);
    }

    fn make_tool() -> CalendarListEvents {
        use crate::oauth::TokenSet;
        use crate::{CalendarClient, OAuthConfig};
        use std::sync::Arc;
        let client = Arc::new(CalendarClient::new(
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
        CalendarListEvents::new(client)
    }

    #[test]
    fn required_scope_is_calendar_read() {
        let tool = make_tool();
        let scope = tool.required_scope(&json!({}));
        assert_eq!(scope.to_string(), "calendar.read");
    }

    #[test]
    fn tool_name_is_canonical() {
        assert_eq!(make_tool().name(), "calendar.list_events");
    }
}
