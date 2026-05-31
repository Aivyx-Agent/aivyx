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
pub mod get_event;
pub mod list_events;
pub mod update_event;

pub use create_event::CalendarCreateEvent;
pub use get_event::CalendarGetEvent;
pub use list_events::CalendarListEvents;
pub use update_event::CalendarUpdateEvent;

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
