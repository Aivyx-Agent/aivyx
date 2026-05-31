//! `calendar.create_event` — create a new Google Calendar
//! event.
//!
//! Phase 128 Task 6. Third Calendar tool; first write tool
//! (`calendar.write` capability, Trusted-tier-only at the
//! ceiling level per Phase 128 Task 3 capability registration).
//!
//! ## API call
//!
//! POST `/calendars/{calendarId}/events` with the event
//! body. Google returns the created event including its
//! generated `id` and `htmlLink`.
//!
//! ## Operator framing
//!
//! Inputs use LLM-friendly shapes:
//! - `attendees` is an array of email strings (NOT the
//!   Google `[{email: ...}]` shape). The tool translates.
//! - `start` / `end` accept RFC 3339 strings directly
//!   (passed to Google as `{dateTime, timeZone}` shape;
//!   timezone optional — Google infers from the timestamp's
//!   offset).

use async_trait::async_trait;
use serde_json::{json, Map, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::calendar_client::SharedCalendarClient;

const DEFAULT_CALENDAR_ID: &str = "primary";

pub struct CalendarCreateEvent {
    id: ToolId,
    schema: Value,
    client: SharedCalendarClient,
}

impl CalendarCreateEvent {
    pub fn new(client: SharedCalendarClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for CalendarCreateEvent {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "calendar.create_event"
    }

    fn description(&self) -> &str {
        "Create a new Google Calendar event. Input is a JSON \
         object with required `summary` (event title), \
         `start` (RFC 3339 timestamp), `end` (RFC 3339 \
         timestamp), and optional `description`, \
         `location`, `attendees` (array of email \
         strings), `calendar_id` (default `\"primary\"`), \
         `time_zone` (IANA name like `\"America/Los_Angeles\"`; \
         optional — Google infers from offset), and \
         `send_notifications` (boolean; controls whether \
         attendees get an invite email; default true). \
         Returns `{id, summary, start, end, html_link}` of \
         the created event. Requires Trusted-tier capability \
         grant for `calendar.write`."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("calendar.write").expect(
            "calendar.write must parse — it is in KNOWN_BASES from Phase 128",
        )
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let parsed = match parse_input(&input) {
            Ok(p) => p,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("calendar.create_event: {reason}"),
                });
            }
        };

        let body = build_event_body(&parsed);
        let send_updates = if parsed.send_notifications {
            "all"
        } else {
            "none"
        };
        let path = format!(
            "/calendars/{}/events?sendUpdates={}",
            super::list_events_urlencode(&parsed.calendar_id),
            send_updates
        );

        let body_json: Value = match self.client.post_json(&path, &body).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("calendar.create_event: API call failed: {e}"),
                });
            }
        };

        let output = json!({
            "id": body_json.get("id").cloned().unwrap_or(Value::Null),
            "summary": body_json.get("summary").cloned().unwrap_or(Value::Null),
            "start": flatten_timestamp(body_json.get("start")),
            "end": flatten_timestamp(body_json.get("end")),
            "html_link": body_json.get("htmlLink").cloned().unwrap_or(Value::Null),
        });

        ToolOutcome::Completed {
            output,
            // Write succeeded → API returned a populated
            // event object with an `id`. Verification
            // semantics: the write landed; subsequent
            // calendar.get_event with the returned id would
            // confirm. We don't make the extra round-trip.
            verified: Verification::Verified,
        }
    }
}

fn build_event_body(parsed: &ParsedInput) -> Value {
    let mut event = Map::new();
    event.insert("summary".to_string(), Value::String(parsed.summary.clone()));
    if let Some(ref d) = parsed.description {
        event.insert("description".to_string(), Value::String(d.clone()));
    }
    if let Some(ref l) = parsed.location {
        event.insert("location".to_string(), Value::String(l.clone()));
    }
    event.insert("start".to_string(), build_timestamp(&parsed.start, parsed.time_zone.as_deref()));
    event.insert("end".to_string(), build_timestamp(&parsed.end, parsed.time_zone.as_deref()));
    if !parsed.attendees.is_empty() {
        let arr: Vec<Value> = parsed
            .attendees
            .iter()
            .map(|email| json!({"email": email}))
            .collect();
        event.insert("attendees".to_string(), Value::Array(arr));
    }
    Value::Object(event)
}

/// Wrap an RFC 3339 timestamp in Google's
/// `{dateTime, [timeZone]}` shape. Omits timeZone when
/// not provided — Google infers from the timestamp's
/// offset suffix.
fn build_timestamp(ts: &str, time_zone: Option<&str>) -> Value {
    let mut obj = Map::new();
    obj.insert("dateTime".to_string(), Value::String(ts.to_string()));
    if let Some(tz) = time_zone {
        obj.insert("timeZone".to_string(), Value::String(tz.to_string()));
    }
    Value::Object(obj)
}

/// Mirror of list_events / get_event's timestamp flattener
/// — converts Google's response shape back to a flat
/// string for the LLM-side output.
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
    summary: String,
    description: Option<String>,
    location: Option<String>,
    start: String,
    end: String,
    time_zone: Option<String>,
    attendees: Vec<String>,
    calendar_id: String,
    send_notifications: bool,
}

fn parse_input(input: &Value) -> Result<ParsedInput, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
    let summary = obj
        .get("summary")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| "input must include a `summary` string field".to_string())?;
    if summary.is_empty() {
        return Err("`summary` must not be empty".to_string());
    }
    let start = require_rfc3339(obj.get("start"), "start")?;
    let end = require_rfc3339(obj.get("end"), "end")?;
    let description = optional_nonempty_string(obj.get("description"), "description")?;
    let location = optional_nonempty_string(obj.get("location"), "location")?;
    let time_zone = optional_nonempty_string(obj.get("time_zone"), "time_zone")?;
    let attendees = parse_attendees(obj.get("attendees"))?;
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
    let send_notifications = match obj.get("send_notifications") {
        None => true,
        Some(v) => v
            .as_bool()
            .ok_or_else(|| "`send_notifications` must be a boolean".to_string())?,
    };
    Ok(ParsedInput {
        summary: summary.to_string(),
        description,
        location,
        start,
        end,
        time_zone,
        attendees,
        calendar_id,
        send_notifications,
    })
}

fn require_rfc3339(value: Option<&Value>, field: &str) -> Result<String, String> {
    let s = value
        .and_then(|v| v.as_str())
        .map(str::trim)
        .ok_or_else(|| format!("`{field}` is required as an RFC 3339 timestamp string"))?;
    if s.is_empty() {
        return Err(format!("`{field}` must not be empty"));
    }
    if !s.chars().any(|c| c.is_ascii_digit()) {
        return Err(format!(
            "`{field}` must look like an RFC 3339 timestamp (e.g. 2026-06-10T09:00:00-07:00)"
        ));
    }
    Ok(s.to_string())
}

fn optional_nonempty_string(value: Option<&Value>, field: &str) -> Result<Option<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                Err(format!("`{field}` must not be empty when present"))
            } else {
                Ok(Some(trimmed.to_string()))
            }
        }
        Some(_) => Err(format!("`{field}` must be a string")),
    }
}

fn parse_attendees(value: Option<&Value>) -> Result<Vec<String>, String> {
    let Some(v) = value else { return Ok(Vec::new()) };
    if v.is_null() {
        return Ok(Vec::new());
    }
    let arr = v
        .as_array()
        .ok_or_else(|| "`attendees` must be an array of email strings".to_string())?;
    let mut emails = Vec::with_capacity(arr.len());
    for (i, item) in arr.iter().enumerate() {
        let email = item.as_str().ok_or_else(|| {
            format!("`attendees[{i}]` must be a string email address")
        })?;
        let trimmed = email.trim();
        if trimmed.is_empty() {
            return Err(format!("`attendees[{i}]` must not be empty"));
        }
        if !trimmed.contains('@') {
            return Err(format!(
                "`attendees[{i}]` must look like an email address (contains `@`)"
            ));
        }
        emails.push(trimmed.to_string());
    }
    Ok(emails)
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "summary": {
                "type": "string",
                "description": "Event title. Required."
            },
            "start": {
                "type": "string",
                "description": "Event start as RFC 3339 timestamp (e.g. `2026-06-10T09:00:00-07:00`). Required."
            },
            "end": {
                "type": "string",
                "description": "Event end as RFC 3339 timestamp. Required."
            },
            "description": {
                "type": ["string", "null"],
                "description": "Long-form event description / notes. Optional."
            },
            "location": {
                "type": ["string", "null"],
                "description": "Location string (free text). Optional."
            },
            "time_zone": {
                "type": ["string", "null"],
                "description": "IANA time zone (e.g. `America/Los_Angeles`). Optional — Google infers from the timestamp's offset when omitted."
            },
            "attendees": {
                "type": "array",
                "items": {"type": "string"},
                "description": "Array of attendee email addresses. Optional."
            },
            "calendar_id": {
                "type": "string",
                "default": DEFAULT_CALENDAR_ID,
                "description": "Calendar to create the event on. Default `\"primary\"`."
            },
            "send_notifications": {
                "type": "boolean",
                "default": true,
                "description": "Whether attendees receive an invite email. Default true."
            }
        },
        "required": ["summary", "start", "end"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_input_accepts_minimal_required() {
        let p = parse_input(&json!({
            "summary": "standup",
            "start": "2026-06-10T09:00:00-07:00",
            "end": "2026-06-10T09:30:00-07:00",
        }))
        .expect("parse");
        assert_eq!(p.summary, "standup");
        assert_eq!(p.start, "2026-06-10T09:00:00-07:00");
        assert_eq!(p.end, "2026-06-10T09:30:00-07:00");
        assert_eq!(p.calendar_id, DEFAULT_CALENDAR_ID);
        assert!(p.attendees.is_empty());
        assert!(p.send_notifications, "default is true");
    }

    #[test]
    fn parse_input_rejects_missing_summary() {
        let e = parse_input(&json!({
            "start": "2026-06-10T09:00:00Z",
            "end": "2026-06-10T10:00:00Z",
        }))
        .expect_err("must error");
        assert!(e.contains("summary"), "{e}");
    }

    #[test]
    fn parse_input_rejects_missing_start_or_end() {
        let e = parse_input(&json!({
            "summary": "x",
            "end": "2026-06-10T10:00:00Z",
        }))
        .expect_err("missing start");
        assert!(e.contains("start"), "{e}");
        let e = parse_input(&json!({
            "summary": "x",
            "start": "2026-06-10T09:00:00Z",
        }))
        .expect_err("missing end");
        assert!(e.contains("end"), "{e}");
    }

    #[test]
    fn parse_input_rejects_non_rfc3339_start() {
        let e = parse_input(&json!({
            "summary": "x",
            "start": "tomorrow",
            "end": "2026-06-10T10:00:00Z",
        }))
        .expect_err("must error");
        assert!(e.contains("RFC 3339"), "{e}");
    }

    #[test]
    fn parse_input_accepts_optional_description_location_time_zone() {
        let p = parse_input(&json!({
            "summary": "review",
            "start": "2026-06-10T09:00:00Z",
            "end": "2026-06-10T10:00:00Z",
            "description": "Quarterly check-in",
            "location": "HQ",
            "time_zone": "America/Los_Angeles",
        }))
        .expect("parse");
        assert_eq!(p.description.as_deref(), Some("Quarterly check-in"));
        assert_eq!(p.location.as_deref(), Some("HQ"));
        assert_eq!(p.time_zone.as_deref(), Some("America/Los_Angeles"));
    }

    #[test]
    fn parse_input_rejects_empty_optional_string() {
        let e = parse_input(&json!({
            "summary": "x",
            "start": "2026-06-10T09:00:00Z",
            "end": "2026-06-10T10:00:00Z",
            "description": "   ",
        }))
        .expect_err("must error");
        assert!(e.contains("description"), "{e}");
    }

    #[test]
    fn parse_input_accepts_attendees_as_email_array() {
        let p = parse_input(&json!({
            "summary": "x",
            "start": "2026-06-10T09:00:00Z",
            "end": "2026-06-10T10:00:00Z",
            "attendees": ["alice@example.com", "bob@example.com"],
        }))
        .expect("parse");
        assert_eq!(p.attendees, vec!["alice@example.com", "bob@example.com"]);
    }

    #[test]
    fn parse_input_rejects_attendee_without_at_sign() {
        let e = parse_input(&json!({
            "summary": "x",
            "start": "2026-06-10T09:00:00Z",
            "end": "2026-06-10T10:00:00Z",
            "attendees": ["not-an-email"],
        }))
        .expect_err("must error");
        assert!(e.contains("@"), "{e}");
    }

    #[test]
    fn parse_input_rejects_non_array_attendees() {
        let e = parse_input(&json!({
            "summary": "x",
            "start": "2026-06-10T09:00:00Z",
            "end": "2026-06-10T10:00:00Z",
            "attendees": "alice@example.com",
        }))
        .expect_err("must error");
        assert!(e.contains("array"), "{e}");
    }

    #[test]
    fn parse_input_honors_send_notifications_false() {
        let p = parse_input(&json!({
            "summary": "x",
            "start": "2026-06-10T09:00:00Z",
            "end": "2026-06-10T10:00:00Z",
            "send_notifications": false,
        }))
        .expect("parse");
        assert!(!p.send_notifications);
    }

    #[test]
    fn build_event_body_emits_summary_start_end() {
        let p = parse_input(&json!({
            "summary": "team sync",
            "start": "2026-06-10T09:00:00-07:00",
            "end": "2026-06-10T10:00:00-07:00",
        }))
        .unwrap();
        let body = build_event_body(&p);
        assert_eq!(body["summary"], "team sync");
        assert_eq!(body["start"]["dateTime"], "2026-06-10T09:00:00-07:00");
        assert_eq!(body["end"]["dateTime"], "2026-06-10T10:00:00-07:00");
        // No timezone supplied → not in the body.
        assert!(body["start"].get("timeZone").is_none());
    }

    #[test]
    fn build_event_body_includes_time_zone_when_supplied() {
        let p = parse_input(&json!({
            "summary": "x",
            "start": "2026-06-10T09:00:00Z",
            "end": "2026-06-10T10:00:00Z",
            "time_zone": "America/New_York",
        }))
        .unwrap();
        let body = build_event_body(&p);
        assert_eq!(body["start"]["timeZone"], "America/New_York");
        assert_eq!(body["end"]["timeZone"], "America/New_York");
    }

    #[test]
    fn build_event_body_translates_email_attendees_to_google_shape() {
        let p = parse_input(&json!({
            "summary": "x",
            "start": "2026-06-10T09:00:00Z",
            "end": "2026-06-10T10:00:00Z",
            "attendees": ["a@x.com", "b@x.com"],
        }))
        .unwrap();
        let body = build_event_body(&p);
        let arr = body["attendees"].as_array().expect("attendees array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["email"], "a@x.com");
        assert_eq!(arr[1]["email"], "b@x.com");
    }

    #[test]
    fn build_event_body_omits_attendees_when_empty() {
        let p = parse_input(&json!({
            "summary": "x",
            "start": "2026-06-10T09:00:00Z",
            "end": "2026-06-10T10:00:00Z",
        }))
        .unwrap();
        let body = build_event_body(&p);
        assert!(body.get("attendees").is_none());
    }

    #[test]
    fn input_schema_declares_three_required_fields() {
        let schema = input_schema();
        let req = schema["required"].as_array().expect("required");
        let req_strs: Vec<&str> = req.iter().filter_map(|v| v.as_str()).collect();
        assert!(req_strs.contains(&"summary"));
        assert!(req_strs.contains(&"start"));
        assert!(req_strs.contains(&"end"));
        assert_eq!(schema["additionalProperties"], false);
    }

    fn make_tool() -> CalendarCreateEvent {
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
        CalendarCreateEvent::new(client)
    }

    #[test]
    fn required_scope_is_calendar_write() {
        let tool = make_tool();
        assert_eq!(
            tool.required_scope(&json!({})).to_string(),
            "calendar.write",
            "create_event MUST use calendar.write (Trusted-tier) per Phase 128 Task 3"
        );
    }

    #[test]
    fn tool_name_is_canonical() {
        assert_eq!(make_tool().name(), "calendar.create_event");
    }
}
