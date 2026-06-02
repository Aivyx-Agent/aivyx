//! `calendar.upcoming` — surface imminent events
//! to the agent with relative-time enrichment.
//!
//! Phase 141. LLM-ergonomic shape over the same
//! Google Calendar `/calendars/{id}/events`
//! endpoint `calendar.list_events` uses. Where
//! `list_events` takes arbitrary `time_min` /
//! `time_max` bounds, `upcoming` takes a single
//! `window_hours` value and computes
//! `time_min = now`, `time_max = now + window`
//! itself. Each event in the output gets two
//! extra fields the agent can use directly:
//!
//! - `starts_in_human` — short phrase like
//!   "in 5 minutes" / "tomorrow" / "in 3 days".
//! - `is_imminent` — true if the event starts
//!   within 30 minutes of now (or has already
//!   started but not yet ended at +30 min).
//!
//! ## Output shape
//!
//! ```json
//! {
//!   "events": [
//!     {
//!       "id": "abc123",
//!       "summary": "Team standup",
//!       "start": "2026-06-03T12:15:00Z",
//!       "end": "2026-06-03T12:30:00Z",
//!       "location": "Conf room A",
//!       "attendee_count": 5,
//!       "starts_in_human": "in 15 minutes",
//!       "is_imminent": true
//!     }
//!   ],
//!   "now": "2026-06-03T12:00:00Z",
//!   "window_hours": 24
//! }
//! ```

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::calendar_client::SharedCalendarClient;
use crate::relative_time::{format_relative_time, is_imminent};

const MAX_RESULTS_CAP: u64 = 250;
const DEFAULT_MAX_RESULTS: u64 = 50;
const DEFAULT_CALENDAR_ID: &str = "primary";
const DEFAULT_WINDOW_HOURS: u64 = 24;
const MAX_WINDOW_HOURS: u64 = 24 * 30; // 30 days
const IMMINENT_THRESHOLD_SECS: i64 = 30 * 60;

/// `calendar.upcoming` tool. Holds a shared
/// Calendar client; tool is stateless beyond the
/// client + a fixed input schema.
pub struct CalendarUpcoming {
    id: ToolId,
    schema: Value,
    client: SharedCalendarClient,
}

impl CalendarUpcoming {
    pub fn new(client: SharedCalendarClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for CalendarUpcoming {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "calendar.upcoming"
    }

    fn description(&self) -> &str {
        "List Google Calendar events starting within \
         the next N hours. Input is a JSON object \
         with optional `window_hours` (default 24, \
         capped at 720 = 30 days), `calendar_id` \
         (default `\"primary\"`), and `max_results` \
         (default 50, capped at 250). Returns a JSON \
         object with an `events` array, a `now` \
         timestamp, and the `window_hours` actually \
         used. Each event has the same shape as \
         `calendar.list_events` plus two extra \
         fields: `starts_in_human` (e.g. \"in 15 \
         minutes\", \"tomorrow\") and `is_imminent` \
         (true if the event starts within 30 \
         minutes). Use this when the operator asks \
         \"what's coming up\" / \"do I have \
         anything today\" / similar; use \
         `calendar.list_events` for arbitrary time \
         ranges."
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
                    detail: format!("calendar.upcoming: {reason}"),
                });
            }
        };

        let now = Utc::now();
        let window = chrono::Duration::hours(parsed.window_hours as i64);
        let time_max = now + window;

        let path = format!(
            "/calendars/{}/events",
            super::list_events_urlencode(&parsed.calendar_id)
        );
        let query: Vec<(&str, String)> = vec![
            ("maxResults", parsed.max_results.to_string()),
            ("singleEvents", "true".to_string()),
            ("orderBy", "startTime".to_string()),
            ("timeMin", now.to_rfc3339()),
            ("timeMax", time_max.to_rfc3339()),
        ];

        let body: Value = match self.client.get_json(&path, &query).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("calendar.upcoming: API call failed: {e}"),
                });
            }
        };

        let events: Vec<Value> = body
            .get("items")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|e| enrich_event(e, now))
                    .collect()
            })
            .unwrap_or_default();

        let output = json!({
            "events": events,
            "now": now.to_rfc3339(),
            "window_hours": parsed.window_hours,
        });

        ToolOutcome::Completed {
            output,
            verified: Verification::NotApplicable,
        }
    }
}

/// Build the shared event summary, then attach the
/// two Phase 141 fields. Pure substrate (apart
/// from the relative_time bridge); unit-tested
/// via the existing relative_time tests + the
/// per-event tests below.
fn enrich_event(event: &Value, now: DateTime<Utc>) -> Value {
    let mut summary = super::event_summary(event);
    let start_str = summary
        .get("start")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let starts_in_human = format_relative_time(start_str, now);
    let imminent = is_imminent(start_str, now, IMMINENT_THRESHOLD_SECS);
    summary["starts_in_human"] = Value::String(starts_in_human);
    summary["is_imminent"] = Value::Bool(imminent);
    summary
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "window_hours": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_WINDOW_HOURS,
                "description": "Hours into the future to scan (default 24, max 720)"
            },
            "calendar_id": {
                "type": "string",
                "description": "Calendar identifier (default \"primary\")"
            },
            "max_results": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_RESULTS_CAP,
                "description": "Cap on events returned (default 50, max 250)"
            }
        },
        "additionalProperties": false
    })
}

#[derive(Debug)]
struct ParsedInput {
    window_hours: u64,
    calendar_id: String,
    max_results: u64,
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

    Ok(ParsedInput {
        window_hours,
        calendar_id,
        max_results,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now_fixed() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 3, 12, 0, 0).unwrap()
    }

    fn at_offset(seconds: i64) -> String {
        let dt = now_fixed() + chrono::Duration::seconds(seconds);
        dt.to_rfc3339()
    }

    #[test]
    fn parse_default_input_uses_defaults() {
        let parsed = parse_input(&json!({})).unwrap();
        assert_eq!(parsed.window_hours, 24);
        assert_eq!(parsed.calendar_id, "primary");
        assert_eq!(parsed.max_results, 50);
    }

    #[test]
    fn parse_explicit_window_honored() {
        let parsed =
            parse_input(&json!({ "window_hours": 48, "max_results": 10 })).unwrap();
        assert_eq!(parsed.window_hours, 48);
        assert_eq!(parsed.max_results, 10);
    }

    #[test]
    fn parse_window_cap_clamps_at_30_days() {
        let parsed = parse_input(&json!({ "window_hours": 10_000 })).unwrap();
        assert_eq!(parsed.window_hours, MAX_WINDOW_HOURS);
    }

    #[test]
    fn parse_max_results_cap_clamps_at_250() {
        let parsed = parse_input(&json!({ "max_results": 999 })).unwrap();
        assert_eq!(parsed.max_results, MAX_RESULTS_CAP);
    }

    #[test]
    fn parse_zero_window_rejected() {
        let err = parse_input(&json!({ "window_hours": 0 })).unwrap_err();
        assert!(err.contains(">= 1"), "{err}");
    }

    #[test]
    fn parse_empty_calendar_id_rejected() {
        let err = parse_input(&json!({ "calendar_id": "  " })).unwrap_err();
        assert!(err.contains("must not be empty"), "{err}");
    }

    #[test]
    fn enrich_attaches_relative_time_and_imminent_flag() {
        // Event 15 minutes ahead → "in 15 minutes",
        // imminent=true.
        let event = json!({
            "id": "evt-1",
            "summary": "Standup",
            "start": { "dateTime": at_offset(15 * 60) },
            "end":   { "dateTime": at_offset(30 * 60) },
            "location": "Conf A",
            "attendees": [{"email": "a@x"}, {"email": "b@x"}],
        });
        let enriched = enrich_event(&event, now_fixed());
        assert_eq!(enriched["id"], json!("evt-1"));
        assert_eq!(enriched["summary"], json!("Standup"));
        assert_eq!(enriched["location"], json!("Conf A"));
        assert_eq!(enriched["attendee_count"], json!(2));
        assert_eq!(enriched["starts_in_human"], json!("in 15 minutes"));
        assert_eq!(enriched["is_imminent"], json!(true));
    }

    #[test]
    fn enrich_marks_far_future_event_not_imminent() {
        let event = json!({
            "id": "evt-2",
            "summary": "Quarterly review",
            "start": { "dateTime": at_offset(72 * 3600) },
            "end":   { "dateTime": at_offset(73 * 3600) },
        });
        let enriched = enrich_event(&event, now_fixed());
        assert_eq!(enriched["starts_in_human"], json!("in 3 days"));
        assert_eq!(enriched["is_imminent"], json!(false));
    }

    #[test]
    fn enrich_handles_all_day_event() {
        let event = json!({
            "id": "evt-allday",
            "summary": "Holiday",
            "start": { "date": "2026-06-10" },
            "end":   { "date": "2026-06-11" },
        });
        let enriched = enrich_event(&event, now_fixed());
        // 6.5 days out → integer-day-truncates to
        // "in 6 days".
        assert_eq!(enriched["starts_in_human"], json!("in 6 days"));
        assert_eq!(enriched["is_imminent"], json!(false));
    }
}
