//! Relative-time formatting for `calendar.upcoming`.
//!
//! Phase 141 — substrate behind the
//! `starts_in_human` and `is_imminent` fields the
//! tool adds to each event in its output. Given an
//! RFC 3339 / ISO 8601 timestamp string from the
//! Google Calendar API and a `now`, produce a
//! short human phrase ("in 5 minutes",
//! "in 3 hours", "tomorrow", "2 days ago") and a
//! boolean "imminent" flag.
//!
//! `now` is parameterized rather than read from
//! the clock so the tests can pin specific
//! "current times" and exercise every output
//! range deterministically.
//!
//! ## Phrase ranges
//!
//! Future events:
//! - Δ < 60s             → "now"
//! - Δ < 60min           → "in N minutes"
//! - Δ < 24h             → "in N hours"
//! - Δ < 48h             → "tomorrow"
//! - Δ ≥ 48h             → "in N days"
//!
//! Past events:
//! - Δ ≥ -60s            → "now"
//! - Δ ≥ -60min          → "N minutes ago"
//! - Δ ≥ -24h            → "N hours ago"
//! - Δ ≥ -48h            → "yesterday"
//! - Δ < -48h            → "N days ago"
//!
//! Past events surface only because some API
//! responses include very-recently-started events
//! that haven't ended; the agent saying "your
//! standup started 3 minutes ago" is more useful
//! than just dropping it.

use chrono::{DateTime, Utc};

/// Parse the event timestamp string Google returns
/// (RFC 3339 with timezone offset, or ISO 8601
/// date for all-day events) into a UTC `DateTime`.
/// Returns `None` if the string can't be parsed —
/// the caller falls back to omitting the relative-
/// time fields.
fn parse_event_time(s: &str) -> Option<DateTime<Utc>> {
    // Try RFC 3339 first (the dateTime case from
    // flatten_timestamp).
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    // All-day events come as just "2026-06-10" —
    // treat as midnight UTC on that day.
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        // and_hms_opt returns Option<NaiveDateTime>;
        // 00:00:00 always exists.
        let ndt = d.and_hms_opt(0, 0, 0)?;
        return Some(DateTime::<Utc>::from_naive_utc_and_offset(ndt, Utc));
    }
    None
}

/// Short human phrase describing when the event
/// starts relative to `now`. Returns an empty
/// string if the timestamp can't be parsed (the
/// caller decides whether to surface the field or
/// drop it).
pub fn format_relative_time(event_iso8601: &str, now: DateTime<Utc>) -> String {
    let Some(event_time) = parse_event_time(event_iso8601) else {
        return String::new();
    };
    let delta = event_time.signed_duration_since(now);
    let secs = delta.num_seconds();

    // Within a minute either direction reads as
    // "now" — the agent's "starting now" framing
    // matches operator expectation.
    if secs.abs() < 60 {
        return "now".to_string();
    }

    if secs > 0 {
        // Future event.
        let minutes = secs / 60;
        if minutes < 60 {
            return format!("in {minutes} minutes");
        }
        let hours = secs / 3600;
        if hours < 24 {
            return format!("in {hours} hours");
        }
        if hours < 48 {
            return "tomorrow".to_string();
        }
        let days = secs / 86_400;
        format!("in {days} days")
    } else {
        // Past event.
        let abs_secs = -secs;
        let minutes = abs_secs / 60;
        if minutes < 60 {
            return format!("{minutes} minutes ago");
        }
        let hours = abs_secs / 3600;
        if hours < 24 {
            return format!("{hours} hours ago");
        }
        if hours < 48 {
            return "yesterday".to_string();
        }
        let days = abs_secs / 86_400;
        format!("{days} days ago")
    }
}

/// True when the event starts within
/// `threshold_secs` of `now` (or has already
/// started but not yet ended at +threshold_secs).
/// The tool uses this to flag events the agent
/// should treat as "very soon."
pub fn is_imminent(event_iso8601: &str, now: DateTime<Utc>, threshold_secs: i64) -> bool {
    let Some(event_time) = parse_event_time(event_iso8601) else {
        return false;
    };
    let delta_secs = event_time.signed_duration_since(now).num_seconds();
    // Event in the future within threshold_secs:
    // 0 <= delta <= threshold.
    // Event recently started (negative delta) but
    // within threshold of now: -threshold <= delta < 0.
    delta_secs >= -threshold_secs && delta_secs <= threshold_secs
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn now_fixed() -> DateTime<Utc> {
        // Deterministic "current time" for every
        // test that doesn't need a specific value.
        Utc.with_ymd_and_hms(2026, 6, 3, 12, 0, 0).unwrap()
    }

    fn at_offset(seconds: i64) -> String {
        let dt = now_fixed() + chrono::Duration::seconds(seconds);
        dt.to_rfc3339()
    }

    #[test]
    fn relative_unparseable_input_returns_empty() {
        let got = format_relative_time("not-a-timestamp", now_fixed());
        assert_eq!(got, "");
    }

    #[test]
    fn relative_within_a_minute_reads_as_now() {
        // 30 seconds in the future.
        assert_eq!(format_relative_time(&at_offset(30), now_fixed()), "now");
        // 30 seconds in the past.
        assert_eq!(format_relative_time(&at_offset(-30), now_fixed()), "now");
        // Boundary: exactly 60s reads as "in 1 minutes".
        assert_eq!(
            format_relative_time(&at_offset(60), now_fixed()),
            "in 1 minutes"
        );
    }

    #[test]
    fn relative_future_minutes_phrase() {
        // 5 minutes ahead.
        assert_eq!(
            format_relative_time(&at_offset(5 * 60), now_fixed()),
            "in 5 minutes"
        );
        // 59 minutes ahead — still minutes.
        assert_eq!(
            format_relative_time(&at_offset(59 * 60), now_fixed()),
            "in 59 minutes"
        );
    }

    #[test]
    fn relative_future_hours_phrase() {
        // 2 hours ahead.
        assert_eq!(
            format_relative_time(&at_offset(2 * 3600), now_fixed()),
            "in 2 hours"
        );
        // 23 hours ahead — still hours.
        assert_eq!(
            format_relative_time(&at_offset(23 * 3600), now_fixed()),
            "in 23 hours"
        );
    }

    #[test]
    fn relative_future_tomorrow_window() {
        // 25 hours ahead reads as "tomorrow".
        assert_eq!(
            format_relative_time(&at_offset(25 * 3600), now_fixed()),
            "tomorrow"
        );
        // 47 hours ahead — still "tomorrow".
        assert_eq!(
            format_relative_time(&at_offset(47 * 3600), now_fixed()),
            "tomorrow"
        );
        // 48 hours ahead — flips to "in N days".
        assert_eq!(
            format_relative_time(&at_offset(48 * 3600), now_fixed()),
            "in 2 days"
        );
    }

    #[test]
    fn relative_future_days_phrase() {
        // 3 days ahead.
        assert_eq!(
            format_relative_time(&at_offset(3 * 86_400), now_fixed()),
            "in 3 days"
        );
        // 7 days ahead.
        assert_eq!(
            format_relative_time(&at_offset(7 * 86_400), now_fixed()),
            "in 7 days"
        );
    }

    #[test]
    fn relative_past_phrases_mirror_future() {
        // 5 minutes ago.
        assert_eq!(
            format_relative_time(&at_offset(-5 * 60), now_fixed()),
            "5 minutes ago"
        );
        // 2 hours ago.
        assert_eq!(
            format_relative_time(&at_offset(-2 * 3600), now_fixed()),
            "2 hours ago"
        );
        // 25 hours ago → "yesterday".
        assert_eq!(
            format_relative_time(&at_offset(-25 * 3600), now_fixed()),
            "yesterday"
        );
        // 3 days ago.
        assert_eq!(
            format_relative_time(&at_offset(-3 * 86_400), now_fixed()),
            "3 days ago"
        );
    }

    #[test]
    fn relative_parses_all_day_date_format() {
        // Google returns "2026-06-10" for all-day
        // events (the `date` field, not
        // `dateTime`). We treat as midnight UTC.
        // From `now` = 2026-06-03 noon UTC, that's
        // 6.5 days out. Integer-day truncation
        // → "in 6 days". The rough-grain phrasing
        // is good enough for the agent's natural-
        // language summary; tomorrow's all-day vs
        // today's all-day still split cleanly.
        let got = format_relative_time("2026-06-10", now_fixed());
        assert_eq!(got, "in 6 days");
    }

    #[test]
    fn imminent_within_threshold_returns_true() {
        // 20 minutes ahead with 30-minute threshold.
        assert!(is_imminent(&at_offset(20 * 60), now_fixed(), 30 * 60));
        // 30 minutes ahead — boundary inclusive.
        assert!(is_imminent(&at_offset(30 * 60), now_fixed(), 30 * 60));
        // 5 minutes ago — recently started, still
        // within window.
        assert!(is_imminent(&at_offset(-5 * 60), now_fixed(), 30 * 60));
    }

    #[test]
    fn imminent_outside_threshold_returns_false() {
        // 31 minutes ahead — just outside.
        assert!(!is_imminent(&at_offset(31 * 60), now_fixed(), 30 * 60));
        // 31 minutes ago — also outside.
        assert!(!is_imminent(&at_offset(-31 * 60), now_fixed(), 30 * 60));
    }

    #[test]
    fn imminent_unparseable_returns_false() {
        assert!(!is_imminent("garbage", now_fixed(), 30 * 60));
    }
}
