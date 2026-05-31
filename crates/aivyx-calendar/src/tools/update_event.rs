//! `calendar.update_event` — partial-patch a Google
//! Calendar event.
//!
//! Phase 128 Task 7. Fourth Calendar tool; second write
//! tool (`calendar.write` capability, Trusted-gated).
//!
//! ## API call
//!
//! PATCH `/calendars/{calendarId}/events/{eventId}` with a
//! body containing ONLY the fields the operator (or LLM)
//! wants to change. Google Calendar's PATCH semantics are
//! partial-update — fields absent from the request body
//! are preserved.
//!
//! ## Operator framing
//!
//! Same LLM-friendly input shape as `calendar.create_event`
//! except `summary` / `start` / `end` are all OPTIONAL
//! (every field is an optional patch). `event_id` is the
//! one required input.

use async_trait::async_trait;
use serde_json::{json, Map, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::calendar_client::SharedCalendarClient;

const DEFAULT_CALENDAR_ID: &str = "primary";

pub struct CalendarUpdateEvent {
    id: ToolId,
    schema: Value,
    client: SharedCalendarClient,
}

impl CalendarUpdateEvent {
    pub fn new(client: SharedCalendarClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for CalendarUpdateEvent {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "calendar.update_event"
    }

    fn description(&self) -> &str {
        "Partial-update a Google Calendar event by ID. Input \
         is a JSON object with required `event_id` and any \
         subset of `summary`, `start` (RFC 3339), `end` (RFC \
         3339), `description`, `location`, `time_zone`, \
         `attendees` (array of email strings — REPLACES the \
         existing attendee list), and `calendar_id` \
         (default `\"primary\"`). Plus optional \
         `send_notifications` (boolean; default true). \
         Returns the updated event's `{id, summary, start, \
         end, html_link}`. Fields not present in the \
         request body are preserved. Requires Trusted-tier \
         capability grant for `calendar.write`."
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
                    detail: format!("calendar.update_event: {reason}"),
                });
            }
        };

        if parsed.is_empty_patch() {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "calendar.update_event: no patch fields supplied — provide at least one of summary / start / end / description / location / time_zone / attendees".to_string(),
            });
        }

        let body = build_patch_body(&parsed);
        let send_updates = if parsed.send_notifications {
            "all"
        } else {
            "none"
        };
        let path = format!(
            "/calendars/{}/events/{}?sendUpdates={}",
            super::list_events_urlencode(&parsed.calendar_id),
            super::list_events_urlencode(&parsed.event_id),
            send_updates
        );

        let body_json: Value = match self.client.patch_json(&path, &body).await {
            Ok(v) => v,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("calendar.update_event: API call failed: {e}"),
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
            verified: Verification::Verified,
        }
    }
}

fn build_patch_body(parsed: &ParsedPatch) -> Value {
    let mut body = Map::new();
    if let Some(ref s) = parsed.summary {
        body.insert("summary".to_string(), Value::String(s.clone()));
    }
    if let Some(ref d) = parsed.description {
        body.insert("description".to_string(), Value::String(d.clone()));
    }
    if let Some(ref l) = parsed.location {
        body.insert("location".to_string(), Value::String(l.clone()));
    }
    if let Some(ref s) = parsed.start {
        body.insert("start".to_string(), build_timestamp(s, parsed.time_zone.as_deref()));
    }
    if let Some(ref e) = parsed.end {
        body.insert("end".to_string(), build_timestamp(e, parsed.time_zone.as_deref()));
    }
    if let Some(ref atts) = parsed.attendees {
        let arr: Vec<Value> = atts.iter().map(|email| json!({"email": email})).collect();
        body.insert("attendees".to_string(), Value::Array(arr));
    }
    Value::Object(body)
}

fn build_timestamp(ts: &str, time_zone: Option<&str>) -> Value {
    let mut obj = Map::new();
    obj.insert("dateTime".to_string(), Value::String(ts.to_string()));
    if let Some(tz) = time_zone {
        obj.insert("timeZone".to_string(), Value::String(tz.to_string()));
    }
    Value::Object(obj)
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
struct ParsedPatch {
    event_id: String,
    calendar_id: String,
    summary: Option<String>,
    description: Option<String>,
    location: Option<String>,
    start: Option<String>,
    end: Option<String>,
    time_zone: Option<String>,
    attendees: Option<Vec<String>>,
    send_notifications: bool,
}

impl ParsedPatch {
    fn is_empty_patch(&self) -> bool {
        self.summary.is_none()
            && self.description.is_none()
            && self.location.is_none()
            && self.start.is_none()
            && self.end.is_none()
            && self.time_zone.is_none()
            && self.attendees.is_none()
    }
}

fn parse_input(input: &Value) -> Result<ParsedPatch, String> {
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
    let summary = optional_nonempty_string(obj.get("summary"), "summary")?;
    let description = optional_nonempty_string(obj.get("description"), "description")?;
    let location = optional_nonempty_string(obj.get("location"), "location")?;
    let time_zone = optional_nonempty_string(obj.get("time_zone"), "time_zone")?;
    let start = optional_rfc3339(obj.get("start"), "start")?;
    let end = optional_rfc3339(obj.get("end"), "end")?;
    let attendees = parse_optional_attendees(obj.get("attendees"))?;
    let send_notifications = match obj.get("send_notifications") {
        None => true,
        Some(v) => v
            .as_bool()
            .ok_or_else(|| "`send_notifications` must be a boolean".to_string())?,
    };
    Ok(ParsedPatch {
        event_id: event_id.to_string(),
        calendar_id,
        summary,
        description,
        location,
        start,
        end,
        time_zone,
        attendees,
        send_notifications,
    })
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

fn optional_rfc3339(value: Option<&Value>, field: &str) -> Result<Option<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Err(format!("`{field}` must not be empty when present"));
            }
            if !trimmed.chars().any(|c| c.is_ascii_digit()) {
                return Err(format!(
                    "`{field}` must look like an RFC 3339 timestamp (e.g. 2026-06-10T09:00:00-07:00)"
                ));
            }
            Ok(Some(trimmed.to_string()))
        }
        Some(_) => Err(format!("`{field}` must be a string (RFC 3339 timestamp)")),
    }
}

fn parse_optional_attendees(value: Option<&Value>) -> Result<Option<Vec<String>>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(arr)) => {
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
            Ok(Some(emails))
        }
        Some(_) => Err("`attendees` must be an array of email strings".to_string()),
    }
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "event_id": {
                "type": "string",
                "description": "Event ID to patch. Required."
            },
            "calendar_id": {
                "type": "string",
                "default": DEFAULT_CALENDAR_ID,
                "description": "Calendar containing the event. Default `\"primary\"`."
            },
            "summary": {"type": ["string", "null"]},
            "description": {"type": ["string", "null"]},
            "location": {"type": ["string", "null"]},
            "start": {"type": ["string", "null"], "description": "RFC 3339 timestamp"},
            "end": {"type": ["string", "null"], "description": "RFC 3339 timestamp"},
            "time_zone": {"type": ["string", "null"]},
            "attendees": {
                "type": ["array", "null"],
                "items": {"type": "string"},
                "description": "REPLACES the existing attendee list."
            },
            "send_notifications": {"type": "boolean", "default": true}
        },
        "required": ["event_id"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_input_accepts_event_id_only() {
        let p = parse_input(&json!({"event_id": "abc"})).expect("parse");
        assert_eq!(p.event_id, "abc");
        assert_eq!(p.calendar_id, DEFAULT_CALENDAR_ID);
        assert!(p.is_empty_patch(), "no fields = empty patch");
    }

    #[test]
    fn parse_input_rejects_missing_event_id() {
        let e = parse_input(&json!({"summary": "x"})).expect_err("must error");
        assert!(e.contains("event_id"), "{e}");
    }

    #[test]
    fn parse_input_accepts_summary_only_patch() {
        let p =
            parse_input(&json!({"event_id": "abc", "summary": "renamed"})).expect("parse");
        assert_eq!(p.summary.as_deref(), Some("renamed"));
        assert!(!p.is_empty_patch());
    }

    #[test]
    fn parse_input_accepts_attendees_replacement() {
        let p = parse_input(&json!({
            "event_id": "abc",
            "attendees": ["new@example.com"]
        }))
        .expect("parse");
        let atts = p.attendees.as_ref().expect("attendees present");
        assert_eq!(atts.len(), 1);
        assert_eq!(atts[0], "new@example.com");
    }

    #[test]
    fn parse_input_accepts_empty_attendees_array_as_clear() {
        // [] is meaningfully "clear all attendees"; we
        // pass it through as Some(vec![]) — the build_patch
        // body will emit `attendees: []` to Google which
        // is the canonical "remove all" PATCH.
        let p = parse_input(&json!({"event_id": "abc", "attendees": []})).expect("parse");
        let atts = p.attendees.expect("attendees present");
        assert!(atts.is_empty());
    }

    #[test]
    fn parse_input_rejects_attendee_without_at() {
        let e = parse_input(&json!({
            "event_id": "abc",
            "attendees": ["nope"]
        }))
        .expect_err("must error");
        assert!(e.contains("@"), "{e}");
    }

    #[test]
    fn parse_input_rejects_non_rfc3339_start() {
        let e = parse_input(&json!({
            "event_id": "abc",
            "start": "tomorrow"
        }))
        .expect_err("must error");
        assert!(e.contains("RFC 3339"), "{e}");
    }

    #[test]
    fn parse_input_rejects_empty_summary_when_present() {
        let e = parse_input(&json!({
            "event_id": "abc",
            "summary": "   "
        }))
        .expect_err("must error");
        assert!(e.contains("summary"), "{e}");
    }

    #[test]
    fn parse_input_send_notifications_defaults_true() {
        let p = parse_input(&json!({"event_id": "abc"})).expect("parse");
        assert!(p.send_notifications);
    }

    #[test]
    fn parse_input_send_notifications_false_honored() {
        let p = parse_input(&json!({
            "event_id": "abc",
            "send_notifications": false
        }))
        .expect("parse");
        assert!(!p.send_notifications);
    }

    #[test]
    fn build_patch_body_only_emits_present_fields() {
        // Loadbearing: PATCH semantics mean we MUST NOT
        // send fields the operator didn't supply, otherwise
        // we'd clear them.
        let p = parse_input(&json!({
            "event_id": "abc",
            "summary": "renamed"
        }))
        .unwrap();
        let body = build_patch_body(&p);
        let obj = body.as_object().expect("object body");
        assert!(obj.contains_key("summary"));
        assert!(!obj.contains_key("description"));
        assert!(!obj.contains_key("location"));
        assert!(!obj.contains_key("start"));
        assert!(!obj.contains_key("end"));
        assert!(!obj.contains_key("attendees"));
    }

    #[test]
    fn build_patch_body_emits_time_zone_with_start_end() {
        let p = parse_input(&json!({
            "event_id": "abc",
            "start": "2026-06-10T09:00:00",
            "end": "2026-06-10T10:00:00",
            "time_zone": "America/New_York"
        }))
        .unwrap();
        let body = build_patch_body(&p);
        assert_eq!(body["start"]["timeZone"], "America/New_York");
        assert_eq!(body["end"]["timeZone"], "America/New_York");
    }

    #[test]
    fn build_patch_body_attendee_clear_emits_empty_array() {
        // Empty attendees array round-trips so Google can
        // interpret as "remove all attendees".
        let p = parse_input(&json!({
            "event_id": "abc",
            "attendees": []
        }))
        .unwrap();
        let body = build_patch_body(&p);
        assert!(body["attendees"].is_array());
        assert_eq!(body["attendees"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn input_schema_declares_only_event_id_required() {
        let schema = input_schema();
        let req = schema["required"].as_array().expect("required");
        assert_eq!(req.len(), 1);
        assert_eq!(req[0], "event_id");
        assert_eq!(schema["additionalProperties"], false);
    }

    fn make_tool() -> CalendarUpdateEvent {
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
        CalendarUpdateEvent::new(client)
    }

    #[test]
    fn required_scope_is_calendar_write() {
        let tool = make_tool();
        assert_eq!(
            tool.required_scope(&json!({})).to_string(),
            "calendar.write"
        );
    }

    #[test]
    fn tool_name_is_canonical() {
        assert_eq!(make_tool().name(), "calendar.update_event");
    }
}
