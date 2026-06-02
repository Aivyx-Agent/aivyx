//! `aivyx_core::Tool` implementations for the Calendar
//! tool process.
//!
//! Phase 128 Q3b operator-picked surface (5 tools):
//!
//! - [`list_events`] — `calendar.list_events` (Task 4;
//!   capability `calendar.read`)
//! - [`get_event`] — `calendar.get_event` (Task 5;
//!   capability `calendar.read`)
//! - [`create_event`] — `calendar.create_event` (Task 6;
//!   capability `calendar.write`, CEILING_TRUSTED)
//! - [`update_event`] — `calendar.update_event` (Task 7;
//!   capability `calendar.write`, CEILING_TRUSTED)
//! - [`delete_event`] — `calendar.delete_event` (Task 8;
//!   capability `calendar.write`, CEILING_TRUSTED)
//!
//! Per-tool modules land in tasks 4-8; this `mod.rs` is
//! the entry-point that `main.rs` reaches into to build
//! the harness's `Vec<Arc<dyn Tool>>`.
//!
//! ## Capability bases
//!
//! Phase 128 Task 3 registers `calendar.read` and
//! `calendar.write` in `aivyx-capability`. The read base
//! defaults to OPERATOR (visible to any role with
//! Operator-or-higher tier); the write base defaults to
//! CEILING_TRUSTED (write tools require an explicit grant
//! per role).

pub mod create_event;
pub mod delete_event;
pub mod get_event;
pub mod list_events;
pub mod update_event;
pub mod upcoming;

pub use create_event::CalendarCreateEvent;
pub use delete_event::CalendarDeleteEvent;
pub use get_event::CalendarGetEvent;
pub use list_events::CalendarListEvents;
pub use upcoming::CalendarUpcoming;
pub use update_event::CalendarUpdateEvent;

// ---------------------------------------------------------------------------
// Phase 141 — shared event-shape helpers.
//
// Lifted from list_events.rs so the new
// calendar.upcoming tool (which surfaces the same
// `{id, summary, start, end, location,
// attendee_count}` shape, then enriches it with
// relative-time fields) can reuse them. Read-side
// calendar tools that surface event lists should
// reuse these for output stability across tools.

use serde_json::{json, Value};

/// Pull a stable snake-case summary out of one
/// Google Calendar event JSON value. Designed for
/// LLM consumption: every field is either present
/// and typed, or null — never silently missing.
/// `start` and `end` flatten the Google
/// `{date, dateTime, timeZone}` shape via
/// [`flatten_timestamp`].
pub(crate) fn event_summary(event: &Value) -> Value {
    let id = event.get("id").cloned().unwrap_or(Value::Null);
    let summary = event.get("summary").cloned().unwrap_or(Value::Null);
    let start = flatten_timestamp(event.get("start"));
    let end = flatten_timestamp(event.get("end"));
    let location = event.get("location").cloned().unwrap_or(Value::Null);
    let attendee_count = event
        .get("attendees")
        .and_then(|v| v.as_array())
        .map(|a| a.len() as u64)
        .unwrap_or(0);
    json!({
        "id": id,
        "summary": summary,
        "start": start,
        "end": end,
        "location": location,
        "attendee_count": attendee_count,
    })
}

/// `{date, dateTime, timeZone}` → either the
/// dateTime string (preferred — full RFC 3339
/// with timezone offset), or the date string
/// (for all-day events), or null.
pub(crate) fn flatten_timestamp(slot: Option<&Value>) -> Value {
    let Some(obj) = slot else {
        return Value::Null;
    };
    if let Some(dt) = obj.get("dateTime").and_then(|v| v.as_str()) {
        return Value::String(dt.to_string());
    }
    if let Some(d) = obj.get("date").and_then(|v| v.as_str()) {
        return Value::String(d.to_string());
    }
    Value::Null
}

/// Minimal URL path-segment encoding shared across the
/// calendar tools. Calendar IDs (and event IDs) may
/// legitimately contain `@` (email-style group calendars
/// like `team@example.com`) or other characters Google's
/// API requires percent-encoded when in a path segment.
/// We don't pull a full `url` / `percent_encoding` crate —
/// just handle the chars that realistically appear in
/// calendar / event IDs.
pub(crate) fn list_events_urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(ch),
            '@' => out.push_str("%40"),
            '/' => out.push_str("%2F"),
            ':' => out.push_str("%3A"),
            other => {
                let mut buf = [0u8; 4];
                let encoded = other.encode_utf8(&mut buf);
                for byte in encoded.bytes() {
                    out.push_str(&format!("%{:02X}", byte));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod shared_tests {
    use super::list_events_urlencode;

    #[test]
    fn encodes_email_style() {
        assert_eq!(
            list_events_urlencode("user@example.com"),
            "user%40example.com"
        );
    }

    #[test]
    fn passes_safe_chars_through() {
        assert_eq!(list_events_urlencode("primary"), "primary");
        assert_eq!(list_events_urlencode("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn encodes_slash_and_colon() {
        assert_eq!(list_events_urlencode("a/b:c"), "a%2Fb%3Ac");
    }
}
