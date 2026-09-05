//! `calendar.get_event` — single-event fetch.
//!
//! Phase 128 Task 5. Second Calendar tool. Reads one event
//! by ID; returns the full event detail (description,
//! conference data, recurrence rule if any, organizer,
//! all attendees with response status).
//!
//! Pair with `calendar.list_events`: range-query lists
//! summarize for LLM scan-friendliness, then drill into a
//! specific event via this tool when the LLM needs the
//! full payload (description / link / attendee responses).

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::calendar_client::SharedCalendarClient;

const DEFAULT_CALENDAR_ID: &str = "primary";

pub struct CalendarGetEvent {
    id: ToolId,
    schema: Value,
    client: SharedCalendarClient,
}

impl CalendarGetEvent {
    pub fn new(client: SharedCalendarClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for CalendarGetEvent {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "calendar.get_event"
    }

    // Chapter Picket follow-up (Finding 3) — an event's title,
    // description, and attendee-supplied fields are externally
    // authored (e.g. from an invite sent by someone else) and may
    // carry a prompt-injection payload.
    fn output_is_untrusted(&self) -> bool {
        true
    }

    fn description(&self) -> &str {
        "Fetch the full detail of one Google Calendar event \
         by ID. Input is a JSON object with a required \
         `event_id` (string; typically obtained from \
         `calendar.list_events`) and an optional \
         `calendar_id` (default `\"primary\"`). Returns \
         the event as a JSON object including \
         `summary`, `description`, `start`, `end`, \
         `location`, `organizer`, `attendees` (array with \
         response status per attendee), `recurrence` \
         (RRULE strings for recurring events), \
         `conference_data` (Meet link if any), and \
         `html_link` (the Calendar-UI URL). A 404 \
         response (event not found) surfaces as a tool \
         error so the LLM can re-list and retry rather \
         than confidently misreport."
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
                    detail: format!("calendar.get_event: {reason}"),
                });
            }
        };

        let path = format!(
            "/calendars/{}/events/{}",
            super::list_events_urlencode(&parsed.calendar_id),
            super::list_events_urlencode(&parsed.event_id),
        );

        let body: Value = match self.client.get_json(&path, &[]).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("calendar.get_event: API call failed: {e}"),
                });
            }
        };

        let output = transform_event(&body);

        ToolOutcome::Completed {
            output,
            verified: Verification::NotApplicable,
        }
    }
}

/// Transform Google's camelCase Calendar event into a
/// stable snake-case payload. Fields the LLM realistically
/// needs are pulled out top-level; the raw Google payload
/// is omitted (operators wanting raw can use `gmail.read`-
/// style snippet inspection on the audit chain).
fn transform_event(event: &Value) -> Value {
    let id = event.get("id").cloned().unwrap_or(Value::Null);
    let summary = event.get("summary").cloned().unwrap_or(Value::Null);
    let description = event.get("description").cloned().unwrap_or(Value::Null);
    let start = flatten_timestamp(event.get("start"));
    let end = flatten_timestamp(event.get("end"));
    let location = event.get("location").cloned().unwrap_or(Value::Null);
    let html_link = event.get("htmlLink").cloned().unwrap_or(Value::Null);
    let status = event.get("status").cloned().unwrap_or(Value::Null);

    let organizer = event
        .get("organizer")
        .map(|o| {
            json!({
                "email": o.get("email").cloned().unwrap_or(Value::Null),
                "display_name": o.get("displayName").cloned().unwrap_or(Value::Null),
                "is_self": o.get("self").cloned().unwrap_or(Value::Bool(false)),
            })
        })
        .unwrap_or(Value::Null);

    let attendees = event
        .get("attendees")
        .and_then(|v| v.as_array())
        .map(|arr| {
            let mapped: Vec<Value> = arr
                .iter()
                .map(|a| {
                    json!({
                        "email": a.get("email").cloned().unwrap_or(Value::Null),
                        "display_name": a.get("displayName").cloned().unwrap_or(Value::Null),
                        "response_status": a.get("responseStatus").cloned().unwrap_or(Value::Null),
                        "is_self": a.get("self").cloned().unwrap_or(Value::Bool(false)),
                        "is_organizer": a.get("organizer").cloned().unwrap_or(Value::Bool(false)),
                        "is_optional": a.get("optional").cloned().unwrap_or(Value::Bool(false)),
                    })
                })
                .collect();
            Value::Array(mapped)
        })
        .unwrap_or_else(|| Value::Array(Vec::new()));

    let recurrence = event
        .get("recurrence")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));

    let conference_data = event
        .get("conferenceData")
        .and_then(|c| {
            // Pull just the entry points (typically Meet URL).
            c.get("entryPoints").cloned()
        })
        .unwrap_or(Value::Null);

    json!({
        "id": id,
        "summary": summary,
        "description": description,
        "start": start,
        "end": end,
        "location": location,
        "html_link": html_link,
        "status": status,
        "organizer": organizer,
        "attendees": attendees,
        "recurrence": recurrence,
        "conference_data": conference_data,
    })
}

fn flatten_timestamp(slot: Option<&Value>) -> Value {
    let Some(obj) = slot else { return Value::Null };
    if let Some(dt) = obj.get("dateTime").and_then(|v| v.as_str()) {
        return Value::String(dt.to_string());
    }
    if let Some(d) = obj.get("date").and_then(|v| v.as_str()) {
        return Value::String(d.to_string());
    }
    Value::Null
}

#[derive(Debug)]
struct ParsedInput {
    event_id: String,
    calendar_id: String,
}

fn parse_input(input: &Value) -> Result<ParsedInput, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
    let event_id = obj
        .get("event_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| "input must include an `event_id` string field".to_string())?;
    if event_id.is_empty() {
        return Err("`event_id` must not be empty".to_string());
    }
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
        event_id: event_id.to_string(),
        calendar_id,
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "event_id": {
                "type": "string",
                "description": "Event ID — typically from `calendar.list_events` output. Required."
            },
            "calendar_id": {
                "type": "string",
                "default": DEFAULT_CALENDAR_ID,
                "description": "Google Calendar ID. `\"primary\"` for the authenticated user's main calendar."
            }
        },
        "required": ["event_id"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendar_get_event_output_is_untrusted_for_bulwark() {
        assert!(make_tool().output_is_untrusted());
    }

    #[test]
    fn parse_input_accepts_minimal_required() {
        let p = parse_input(&json!({"event_id": "abc123"})).expect("parse");
        assert_eq!(p.event_id, "abc123");
        assert_eq!(p.calendar_id, DEFAULT_CALENDAR_ID);
    }

    #[test]
    fn parse_input_accepts_explicit_calendar_id() {
        let p = parse_input(&json!({
            "event_id": "abc",
            "calendar_id": "team@example.com"
        }))
        .expect("parse");
        assert_eq!(p.calendar_id, "team@example.com");
    }

    #[test]
    fn parse_input_rejects_missing_event_id() {
        let e = parse_input(&json!({"calendar_id": "primary"})).expect_err("must error");
        assert!(e.contains("event_id"), "{e}");
    }

    #[test]
    fn parse_input_rejects_empty_event_id() {
        let e = parse_input(&json!({"event_id": "   "})).expect_err("must error");
        assert!(e.contains("event_id"), "{e}");
    }

    #[test]
    fn parse_input_rejects_non_string_event_id() {
        let e = parse_input(&json!({"event_id": 42})).expect_err("must error");
        assert!(e.contains("event_id"), "{e}");
    }

    #[test]
    fn parse_input_rejects_empty_calendar_id() {
        let e = parse_input(&json!({"event_id": "x", "calendar_id": ""}))
            .expect_err("must error");
        assert!(e.contains("calendar_id"), "{e}");
    }

    #[test]
    fn parse_input_trims_whitespace() {
        let p = parse_input(&json!({
            "event_id": "  trimmed-id  ",
        }))
        .expect("parse");
        assert_eq!(p.event_id, "trimmed-id");
    }

    #[test]
    fn transform_event_flattens_dotted_timestamp_shape() {
        let event = json!({
            "id": "evt1",
            "summary": "Strategy",
            "description": "Quarterly check-in",
            "start": {"dateTime": "2026-06-10T14:00:00-07:00"},
            "end": {"dateTime": "2026-06-10T15:00:00-07:00"},
            "location": "HQ",
            "htmlLink": "https://calendar.google.com/event?eid=xxx",
            "status": "confirmed",
        });
        let out = transform_event(&event);
        assert_eq!(out["id"], "evt1");
        assert_eq!(out["summary"], "Strategy");
        assert_eq!(out["description"], "Quarterly check-in");
        assert_eq!(out["start"], "2026-06-10T14:00:00-07:00");
        assert_eq!(out["end"], "2026-06-10T15:00:00-07:00");
        assert_eq!(out["location"], "HQ");
        assert_eq!(out["html_link"], "https://calendar.google.com/event?eid=xxx");
        assert_eq!(out["status"], "confirmed");
    }

    #[test]
    fn transform_event_pulls_organizer_fields() {
        let event = json!({
            "id": "e",
            "organizer": {"email": "boss@example.com", "displayName": "Boss", "self": false}
        });
        let out = transform_event(&event);
        let org = &out["organizer"];
        assert_eq!(org["email"], "boss@example.com");
        assert_eq!(org["display_name"], "Boss");
        assert_eq!(org["is_self"], false);
    }

    #[test]
    fn transform_event_maps_attendee_response_status() {
        let event = json!({
            "id": "e",
            "attendees": [
                {"email": "a@x", "responseStatus": "accepted", "self": true},
                {"email": "b@x", "responseStatus": "needsAction"},
                {"email": "c@x", "responseStatus": "declined", "optional": true}
            ]
        });
        let out = transform_event(&event);
        let atts = out["attendees"].as_array().expect("attendees array");
        assert_eq!(atts.len(), 3);
        assert_eq!(atts[0]["response_status"], "accepted");
        assert_eq!(atts[0]["is_self"], true);
        assert_eq!(atts[1]["response_status"], "needsAction");
        assert_eq!(atts[2]["is_optional"], true);
    }

    #[test]
    fn transform_event_renders_recurrence_as_array() {
        let event = json!({
            "id": "e",
            "recurrence": ["RRULE:FREQ=WEEKLY;BYDAY=MO"]
        });
        let out = transform_event(&event);
        assert_eq!(
            out["recurrence"][0],
            "RRULE:FREQ=WEEKLY;BYDAY=MO"
        );
    }

    #[test]
    fn transform_event_pulls_conference_entry_points() {
        let event = json!({
            "id": "e",
            "conferenceData": {
                "entryPoints": [{"entryPointType": "video", "uri": "https://meet.google.com/abc"}]
            }
        });
        let out = transform_event(&event);
        let cd = &out["conference_data"];
        assert_eq!(cd[0]["uri"], "https://meet.google.com/abc");
    }

    #[test]
    fn transform_event_handles_all_day_form() {
        let event = json!({
            "id": "holiday",
            "summary": "Independence Day",
            "start": {"date": "2026-07-04"},
            "end": {"date": "2026-07-05"},
        });
        let out = transform_event(&event);
        assert_eq!(out["start"], "2026-07-04");
        assert_eq!(out["end"], "2026-07-05");
    }

    #[test]
    fn transform_event_handles_minimal_event() {
        let event = json!({"id": "minimal"});
        let out = transform_event(&event);
        // All optional fields render null / [] rather than
        // missing — keeps the LLM-side consumer simple.
        assert!(out["summary"].is_null());
        assert!(out["organizer"].is_null());
        assert!(out["attendees"].is_array());
        assert_eq!(out["attendees"].as_array().unwrap().len(), 0);
        assert!(out["recurrence"].is_array());
        assert_eq!(out["recurrence"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn input_schema_declares_event_id_required() {
        let schema = input_schema();
        let req = schema["required"].as_array().expect("required");
        assert!(req.iter().any(|v| v.as_str() == Some("event_id")));
        assert_eq!(schema["additionalProperties"], false);
    }

    fn make_tool() -> CalendarGetEvent {
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
        CalendarGetEvent::new(client)
    }

    #[test]
    fn required_scope_is_calendar_read() {
        let tool = make_tool();
        assert_eq!(tool.required_scope(&json!({})).to_string(), "calendar.read");
    }

    #[test]
    fn tool_name_is_canonical() {
        assert_eq!(make_tool().name(), "calendar.get_event");
    }
}
