//! `calendar.delete_event` — delete a Google Calendar
//! event.
//!
//! Phase 128 Task 8. Fifth and final Calendar tool; third
//! write tool (`calendar.write` capability, Trusted-gated).
//!
//! ## API call
//!
//! DELETE `/calendars/{calendarId}/events/{eventId}` with
//! optional `sendUpdates=all|none` query param controlling
//! whether attendees receive a cancellation email.
//!
//! ## Idempotency
//!
//! Google returns 410 Gone for events that were already
//! deleted. We treat 410 as success (the event is gone,
//! which is what the caller wanted) and report it
//! explicitly in the output so the audit chain can tell
//! "first deletion" apart from "redundant deletion".

use async_trait::async_trait;
use reqwest::StatusCode;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::calendar_client::SharedCalendarClient;

const DEFAULT_CALENDAR_ID: &str = "primary";

pub struct CalendarDeleteEvent {
    id: ToolId,
    schema: Value,
    client: SharedCalendarClient,
}

impl CalendarDeleteEvent {
    pub fn new(client: SharedCalendarClient) -> Self {
        Self {
            id: ToolId::new(),
            schema: input_schema(),
            client,
        }
    }
}

#[async_trait]
impl Tool for CalendarDeleteEvent {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "calendar.delete_event"
    }

    fn description(&self) -> &str {
        "Delete a Google Calendar event by ID. Input is a \
         JSON object with required `event_id`, optional \
         `calendar_id` (default `\"primary\"`), and \
         optional `notify_attendees` (boolean, default \
         false; when true Google sends a cancellation \
         email to attendees). Idempotent: deleting an \
         already-deleted event succeeds with \
         `was_already_deleted: true` rather than failing. \
         Returns `{event_id, calendar_id, was_already_deleted}`. \
         Requires Trusted-tier capability grant for \
         `calendar.write`."
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
                    detail: format!("calendar.delete_event: {reason}"),
                });
            }
        };

        let path = format!(
            "/calendars/{}/events/{}",
            super::list_events_urlencode(&parsed.calendar_id),
            super::list_events_urlencode(&parsed.event_id),
        );
        let send_updates = if parsed.notify_attendees { "all" } else { "none" };
        let query = vec![("sendUpdates", send_updates.to_string())];

        let status = match self.client.delete(&path, &query).await {
            Ok(s) => s,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("calendar.delete_event: API call failed: {e}"),
                });
            }
        };

        let was_already_deleted = status == StatusCode::GONE;

        let output = json!({
            "event_id": parsed.event_id,
            "calendar_id": parsed.calendar_id,
            "was_already_deleted": was_already_deleted,
        });

        ToolOutcome::Completed {
            output,
            verified: Verification::Verified,
        }
    }
}

#[derive(Debug)]
struct ParsedInput {
    event_id: String,
    calendar_id: String,
    notify_attendees: bool,
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
    let notify_attendees = match obj.get("notify_attendees") {
        None => false,
        Some(v) => v
            .as_bool()
            .ok_or_else(|| "`notify_attendees` must be a boolean".to_string())?,
    };
    Ok(ParsedInput {
        event_id: event_id.to_string(),
        calendar_id,
        notify_attendees,
    })
}

fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "event_id": {
                "type": "string",
                "description": "Event ID to delete. Required."
            },
            "calendar_id": {
                "type": "string",
                "default": DEFAULT_CALENDAR_ID,
                "description": "Calendar containing the event. Default `\"primary\"`."
            },
            "notify_attendees": {
                "type": "boolean",
                "default": false,
                "description": "When true, send cancellation emails to attendees. Default false."
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
    fn parse_input_accepts_event_id_only() {
        let p = parse_input(&json!({"event_id": "abc"})).expect("parse");
        assert_eq!(p.event_id, "abc");
        assert_eq!(p.calendar_id, DEFAULT_CALENDAR_ID);
        assert!(!p.notify_attendees, "default false");
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
    fn parse_input_notify_attendees_true_honored() {
        let p = parse_input(&json!({
            "event_id": "x",
            "notify_attendees": true
        }))
        .expect("parse");
        assert!(p.notify_attendees);
    }

    #[test]
    fn parse_input_rejects_non_boolean_notify_attendees() {
        let e = parse_input(&json!({
            "event_id": "x",
            "notify_attendees": "yes"
        }))
        .expect_err("must error");
        assert!(e.contains("notify_attendees"), "{e}");
    }

    #[test]
    fn input_schema_declares_event_id_required() {
        let schema = input_schema();
        let req = schema["required"].as_array().expect("required");
        assert_eq!(req.len(), 1);
        assert_eq!(req[0], "event_id");
        assert_eq!(schema["additionalProperties"], false);
    }

    #[test]
    fn input_schema_notify_attendees_defaults_false() {
        let schema = input_schema();
        assert_eq!(schema["properties"]["notify_attendees"]["default"], false);
    }

    fn make_tool() -> CalendarDeleteEvent {
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
        CalendarDeleteEvent::new(client)
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
        assert_eq!(make_tool().name(), "calendar.delete_event");
    }
}
