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
use crate::relative_time::{format_relative_time, is_imminent, parse_event_time};

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
         the next N hours, optionally across multiple \
         calendars. Input is a JSON object with: \
         optional `window_hours` (default 24, capped \
         at 720 = 30 days); optional `calendar_ids` \
         (array of calendar IDs to query — Phase \
         142 multi-calendar shape) OR optional \
         `calendar_id` (single ID, Phase 141 \
         legacy shape — default `\"primary\"`); and \
         optional `max_results` (default 50, capped \
         at 250). When `calendar_ids` has multiple \
         entries, queries are sequential and the \
         merged event list is sorted by start \
         time before the max_results cap is \
         applied. Returns a JSON object with an \
         `events` array, a `now` timestamp, and \
         the `window_hours` actually used. Each \
         event has the same shape as \
         `calendar.list_events` plus three Phase \
         141/142 fields: `starts_in_human` \
         (e.g. \"in 15 minutes\", \"tomorrow\"), \
         `is_imminent` (true if the event starts \
         within 30 minutes), and `calendar_id` \
         (which calendar it came from). Pair with \
         `calendar.list_calendars` for the agent \
         to discover what calendar IDs exist."
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
        let mut parsed = match parse_input(&input) {
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
        let time_min_rfc = now.to_rfc3339();
        let time_max_rfc = time_max.to_rfc3339();

        // Phase 155 — writable_only filter. Pre-
        // fetch the calendar list, filter to
        // entries the operator can write to, and
        // intersect with parsed.calendar_ids.
        // When operator passed explicit ids
        // (calendar_id or calendar_ids), the
        // intersection drops read-only entries.
        // When operator didn't (calendar_ids
        // defaulted to ["primary"]), replace
        // with the full set of writable ids.
        //
        // Phase 158 — 5-minute session cache
        // sits behind `writable_calendar_ids` on
        // the client; subsequent calls in the
        // same session skip the round trip.
        if parsed.writable_only {
            let writable_ids = match self.client.writable_calendar_ids().await {
                Ok(v) => v,
                Err(e) => {
                    return ToolOutcome::Failed(AivyxError::Tool {
                        tool: self.id,
                        detail: format!(
                            "calendar.upcoming: writable_only list fetch failed: {e}"
                        ),
                    });
                }
            };
            let writable_set: std::collections::HashSet<String> =
                writable_ids.into_iter().collect();

            if parsed.calendar_ids_explicit {
                parsed.calendar_ids.retain(|id| writable_set.contains(id));
            } else {
                // Operator didn't pick — give
                // them all writable calendars.
                parsed.calendar_ids = writable_set.into_iter().collect();
            }

            if parsed.calendar_ids.is_empty() {
                // Nothing to query. Return empty
                // events cleanly rather than
                // failing.
                return ToolOutcome::Completed {
                    output: json!({
                        "events": [],
                        "now": now.to_rfc3339(),
                        "window_hours": parsed.window_hours,
                    }),
                    verified: Verification::NotApplicable,
                };
            }
        }

        // Phase 151 — parallel fan-out over the
        // requested calendars via
        // `futures_util::future::join_all`. Each
        // per-calendar future runs concurrently;
        // results merge into one Vec sorted by
        // start time. The cap applies post-merge
        // so cross-calendar event density is
        // preserved.
        //
        // Pre-Phase 151 this loop was sequential
        // — 5 calendars took ~5× single-calendar
        // latency. Now it's bounded by the
        // slowest single calendar's response.
        // Phase 155 — max_concurrent throttle.
        // When operator sets max_concurrent: N,
        // wrap each per-calendar future with a
        // semaphore-permit acquisition so at most
        // N requests run simultaneously. When
        // operator doesn't set it, we pass
        // calendar_ids.len() permits, which is
        // equivalent to unlimited (every future
        // can hold a permit at once).
        let permits = parsed
            .max_concurrent
            .map(|n| n as usize)
            .unwrap_or(parsed.calendar_ids.len().max(1));
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(permits));
        let per_calendar_futures = parsed.calendar_ids.iter().map(|calendar_id| {
            let path = format!(
                "/calendars/{}/events",
                super::list_events_urlencode(calendar_id)
            );
            let query: Vec<(&str, String)> = vec![
                ("maxResults", parsed.max_results.to_string()),
                ("singleEvents", "true".to_string()),
                ("orderBy", "startTime".to_string()),
                ("timeMin", time_min_rfc.clone()),
                ("timeMax", time_max_rfc.clone()),
            ];
            let client = self.client.clone();
            let cid = calendar_id.clone();
            let semaphore = std::sync::Arc::clone(&semaphore);
            async move {
                let _permit = semaphore
                    .acquire()
                    .await
                    .expect("semaphore not closed");
                let body: Value = client.get_json(&path, &query).await
                    .map_err(|e| (cid.clone(), e))?;
                Ok::<(String, Value), (String, _)>((cid, body))
            }
        });

        let results = futures_util::future::join_all(per_calendar_futures).await;

        let mut merged: Vec<Value> = Vec::new();
        for result in results {
            match result {
                Ok((calendar_id, body)) => {
                    if let Some(items) = body.get("items").and_then(|v| v.as_array()) {
                        for raw in items {
                            merged.push(enrich_event(raw, now, &calendar_id));
                        }
                    }
                }
                Err((calendar_id, e)) => {
                    return ToolOutcome::Failed(AivyxError::Tool {
                        tool: self.id,
                        detail: format!(
                            "calendar.upcoming: API call failed for \
                             calendar {calendar_id:?}: {e}"
                        ),
                    });
                }
            }
        }

        // Phase 151 — cross-calendar dedup
        // before the merge+sort+cap step. Events
        // that appear on multiple calendars (the
        // typical cross-invite case) collapse to
        // one entry keyed on (summary, start).
        // The first occurrence wins, which is
        // typically the operator's primary
        // calendar when calendar_ids is listed
        // primary-first.
        merged = dedup_events(merged, parsed.fuzzy_dedup);
        merge_sort_and_cap(&mut merged, parsed.max_results as usize);

        let output = json!({
            "events": merged,
            "now": now.to_rfc3339(),
            "window_hours": parsed.window_hours,
        });

        ToolOutcome::Completed {
            output,
            verified: Verification::NotApplicable,
        }
    }
}

/// Sort the merged event list by start time
/// (events without a parseable start go to the
/// end), then truncate to `cap`.
fn merge_sort_and_cap(events: &mut Vec<Value>, cap: usize) {
    events.sort_by(|a, b| {
        let a_key = sort_key(a);
        let b_key = sort_key(b);
        match (a_key, b_key) {
            (Some(a), Some(b)) => a.cmp(&b),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        }
    });
    if events.len() > cap {
        events.truncate(cap);
    }
}

fn sort_key(event: &Value) -> Option<DateTime<Utc>> {
    let start = event.get("start").and_then(|v| v.as_str())?;
    parse_event_time(start)
}

/// Phase 151 — cross-calendar event
/// deduplication. Removes events that share
/// `(summary, start)` with an earlier event in
/// the list. The first occurrence (by original
/// input order) wins — operators typically pass
/// calendar_ids with their primary calendar
/// first, so the retained copy is from the
/// most-authoritative source.
///
/// Pure substrate so the dedup can be tested
/// without touching the Drive client.
///
/// Phase 155 — `fuzzy` toggle. When true (the
/// post-155 default), summaries normalize via
/// `normalize_summary` (trim+lowercase) and
/// start times bucket to 5-minute boundaries.
///
/// Phase 158 — sliding-window adjacency merge
/// (only when `fuzzy=true`). Events sharing a
/// normalized summary are grouped, each group
/// sorted by start time, then swept: any pair
/// of consecutive starts within ±5 minutes
/// collapses into a cluster. Within a cluster,
/// the event with the smallest original input
/// index wins. Output is re-sorted by original
/// input index so cross-calendar ordering
/// matches Phase 151 semantics.
pub(crate) fn dedup_events(events: Vec<Value>, fuzzy: bool) -> Vec<Value> {
    if !fuzzy {
        // Phase 151 exact-key dedup, unchanged.
        let mut seen: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();
        let mut out: Vec<Value> = Vec::with_capacity(events.len());
        for event in events {
            let raw_summary = event
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let raw_start = event
                .get("start")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if seen.insert((raw_summary, raw_start)) {
                out.push(event);
            }
        }
        return out;
    }

    sliding_window_dedup(events)
}

/// Phase 158 — sliding-window adjacency merge
/// for fuzzy dedup. Pure substrate so the merge
/// can be exhaustively tested without live API.
fn sliding_window_dedup(events: Vec<Value>) -> Vec<Value> {
    // Pair each event with original index and
    // pre-compute the merge keys.
    #[derive(Clone)]
    struct Tagged {
        index: usize,
        summary_key: String,
        // None means unparseable start —
        // event sits alone in its own cluster.
        start_dt: Option<chrono::DateTime<chrono::Utc>>,
        raw_start: String,
        event: Value,
    }

    let mut tagged: Vec<Tagged> = events
        .into_iter()
        .enumerate()
        .map(|(index, event)| {
            let raw_summary = event
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let raw_start = event
                .get("start")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let start_dt = parse_event_time(&raw_start);
            Tagged {
                index,
                summary_key: normalize_summary(&raw_summary),
                start_dt,
                raw_start,
                event,
            }
        })
        .collect();

    // Sort by (summary_key, start_dt). Unparseable
    // starts sort last within the summary group
    // and each becomes its own cluster.
    tagged.sort_by(|a, b| {
        a.summary_key
            .cmp(&b.summary_key)
            .then_with(|| match (a.start_dt, b.start_dt) {
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.raw_start.cmp(&b.raw_start),
            })
    });

    // Sweep: cluster boundary is summary change,
    // unparseable start, or consecutive gap > 5
    // minutes. Within each cluster, pick the
    // tagged with the smallest original index.
    let bucket_seconds: i64 = 300;
    let mut kept: Vec<Tagged> = Vec::with_capacity(tagged.len());
    let mut iter = tagged.into_iter().peekable();
    while let Some(first) = iter.next() {
        let mut cluster: Vec<Tagged> = vec![first];
        loop {
            let last = cluster.last().unwrap();
            let Some(prev_dt) = last.start_dt else {
                break; // unparseable: cluster closes after itself
            };
            let Some(next) = iter.peek() else { break };
            if next.summary_key != last.summary_key {
                break;
            }
            let Some(next_dt) = next.start_dt else {
                break;
            };
            let gap = next_dt.signed_duration_since(prev_dt).num_seconds().abs();
            if gap < bucket_seconds {
                cluster.push(iter.next().unwrap());
            } else {
                break;
            }
        }
        let winner = cluster
            .into_iter()
            .min_by_key(|t| t.index)
            .expect("cluster always has at least one element");
        kept.push(winner);
    }

    // Restore original input order.
    kept.sort_by_key(|t| t.index);
    kept.into_iter().map(|t| t.event).collect()
}

/// Phase 155 — fuzzy summary normalization:
/// trim + lowercase. Handles the most common
/// cross-calendar variants: "Standup" vs
/// "STANDUP" vs " Standup ". Doesn't collapse
/// suffixed variants ("Standup" vs "Standup —
/// Team A") — operators with intentional
/// suffixes keep their distinctions.
pub(crate) fn normalize_summary(summary: &str) -> String {
    summary.trim().to_lowercase()
}

/// Build the shared event summary, then attach
/// the Phase 141 relative-time fields and the
/// Phase 142 `calendar_id` traceability field.
/// Pure substrate (apart from the relative_time
/// bridge); unit-tested via the existing
/// relative_time tests + the per-event tests
/// below.
fn enrich_event(event: &Value, now: DateTime<Utc>, calendar_id: &str) -> Value {
    let mut summary = super::event_summary(event);
    let start_str = summary
        .get("start")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let starts_in_human = format_relative_time(start_str, now);
    let imminent = is_imminent(start_str, now, IMMINENT_THRESHOLD_SECS);
    summary["starts_in_human"] = Value::String(starts_in_human);
    summary["is_imminent"] = Value::Bool(imminent);
    summary["calendar_id"] = Value::String(calendar_id.to_string());
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
                "description": "Single calendar identifier (legacy Phase 141 shape; default \"primary\"). Mutually exclusive with calendar_ids."
            },
            "calendar_ids": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Multiple calendar identifiers to fan-out and merge (Phase 142). Mutually exclusive with calendar_id."
            },
            "max_results": {
                "type": "integer",
                "minimum": 1,
                "maximum": MAX_RESULTS_CAP,
                "description": "Cap on events returned post-merge (default 50, max 250)"
            },
            "fuzzy_dedup": {
                "type": "boolean",
                "description": "Phase 155 — normalize summary (lowercase + trim) and bucket start time to 5-minute boundaries when matching cross-calendar duplicates. Default true. Set false for the Phase 151 exact-match behavior."
            },
            "writable_only": {
                "type": "boolean",
                "description": "Phase 155 — when true, pre-fetches the calendar list and filters to entries with access_role in [owner, writer]. Useful for 'what's coming up that I can edit'. Adds one extra API round-trip."
            },
            "max_concurrent": {
                "type": "integer",
                "minimum": 1,
                "description": "Phase 155 — throttle the parallel fan-out so at most N per-calendar requests run simultaneously. Default unlimited (every requested calendar's request fires at once). Useful for rate-limited operators hitting 429s."
            }
        },
        "additionalProperties": false
    })
}

#[derive(Debug)]
struct ParsedInput {
    window_hours: u64,
    /// Phase 142 — every input shape normalizes
    /// to this Vec. Single-calendar callers get
    /// a length-1 Vec; multi-calendar callers get
    /// whatever they passed in.
    calendar_ids: Vec<String>,
    /// Phase 155 — true when the operator
    /// supplied `calendar_id` or `calendar_ids`
    /// explicitly. False when the default
    /// `["primary"]` was used. Affects how
    /// `writable_only` interprets the
    /// calendar set.
    calendar_ids_explicit: bool,
    max_results: u64,
    /// Phase 155 — when true, dedup uses
    /// normalized summary + 5-min time bucket.
    /// When false, falls back to Phase 151 exact
    /// match.
    fuzzy_dedup: bool,
    /// Phase 155 — when true, only writable
    /// calendars (owner / writer) are queried.
    writable_only: bool,
    /// Phase 155 — concurrent-request cap for
    /// the parallel fan-out. None = unlimited.
    max_concurrent: Option<u32>,
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

    // Phase 142 — three input shapes:
    // 1. calendar_ids array → use as-is.
    // 2. calendar_id string → wrap to single-
    //    item Vec (Phase 141 legacy path).
    // 3. neither → default ["primary"].
    // 4. both → reject as ambiguous.
    let has_ids = obj.contains_key("calendar_ids");
    let has_id = obj.contains_key("calendar_id");
    let calendar_ids: Vec<String> = match (has_ids, has_id) {
        (true, true) => {
            return Err(
                "specify either `calendar_id` (single, legacy) or \
                 `calendar_ids` (array, multi), not both"
                    .to_string(),
            );
        }
        (true, false) => {
            let arr = obj
                .get("calendar_ids")
                .and_then(|v| v.as_array())
                .ok_or_else(|| "`calendar_ids` must be an array of strings".to_string())?;
            let mut out: Vec<String> = Vec::with_capacity(arr.len());
            for item in arr {
                let s = item
                    .as_str()
                    .ok_or_else(|| {
                        "`calendar_ids[]` entries must be strings".to_string()
                    })?
                    .trim()
                    .to_string();
                if s.is_empty() {
                    return Err(
                        "`calendar_ids[]` entries must not be empty".to_string()
                    );
                }
                out.push(s);
            }
            if out.is_empty() {
                return Err("`calendar_ids` must not be empty".to_string());
            }
            out
        }
        (false, true) => {
            let s = obj
                .get("calendar_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "`calendar_id` must be a string".to_string())?
                .trim()
                .to_string();
            if s.is_empty() {
                return Err("`calendar_id` must not be empty".to_string());
            }
            vec![s]
        }
        (false, false) => vec![DEFAULT_CALENDAR_ID.to_string()],
    };

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

    let fuzzy_dedup = match obj.get("fuzzy_dedup") {
        None => true,
        Some(v) => v
            .as_bool()
            .ok_or_else(|| "`fuzzy_dedup` must be a boolean".to_string())?,
    };

    let writable_only = match obj.get("writable_only") {
        None => false,
        Some(v) => v
            .as_bool()
            .ok_or_else(|| "`writable_only` must be a boolean".to_string())?,
    };

    let max_concurrent = match obj.get("max_concurrent") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let n = v
                .as_u64()
                .ok_or_else(|| "`max_concurrent` must be a positive integer".to_string())?;
            if n == 0 {
                return Err("`max_concurrent` must be >= 1".to_string());
            }
            Some(n as u32)
        }
    };

    // Phase 155 — true iff operator supplied an
    // explicit calendar_id or calendar_ids.
    let calendar_ids_explicit = has_ids || has_id;

    Ok(ParsedInput {
        window_hours,
        calendar_ids,
        calendar_ids_explicit,
        max_results,
        fuzzy_dedup,
        writable_only,
        max_concurrent,
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
        assert_eq!(parsed.calendar_ids, vec!["primary".to_string()]);
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

    // ---- Phase 142 — multi-calendar parsing ----

    #[test]
    fn parse_legacy_calendar_id_normalizes_to_singleton_vec() {
        let parsed = parse_input(&json!({ "calendar_id": "work@x.com" })).unwrap();
        assert_eq!(parsed.calendar_ids, vec!["work@x.com".to_string()]);
    }

    #[test]
    fn parse_calendar_ids_array_used_as_is() {
        let parsed = parse_input(&json!({
            "calendar_ids": ["primary", "work@x.com", "shared@y.com"],
        }))
        .unwrap();
        assert_eq!(
            parsed.calendar_ids,
            vec![
                "primary".to_string(),
                "work@x.com".to_string(),
                "shared@y.com".to_string()
            ],
        );
    }

    #[test]
    fn parse_both_calendar_id_and_calendar_ids_rejected() {
        let err = parse_input(&json!({
            "calendar_id": "primary",
            "calendar_ids": ["primary"],
        }))
        .unwrap_err();
        assert!(err.contains("not both"), "{err}");
    }

    #[test]
    fn parse_empty_calendar_ids_array_rejected() {
        let err = parse_input(&json!({ "calendar_ids": [] })).unwrap_err();
        assert!(err.contains("must not be empty"), "{err}");
    }

    #[test]
    fn parse_blank_entry_in_calendar_ids_rejected() {
        let err = parse_input(&json!({ "calendar_ids": ["primary", "  "] })).unwrap_err();
        assert!(err.contains("must not be empty"), "{err}");
    }

    // ---- Phase 142 — merge + sort + cap ----

    #[test]
    fn merge_sort_orders_by_start_time_across_calendars() {
        // Three events: middle one is earliest,
        // last one is latest. After sort the
        // order must be (earliest, middle,
        // latest).
        let mut events = vec![
            json!({
                "id": "b",
                "start": at_offset(20 * 60),
                "calendar_id": "personal",
            }),
            json!({
                "id": "a",
                "start": at_offset(5 * 60),
                "calendar_id": "work",
            }),
            json!({
                "id": "c",
                "start": at_offset(60 * 60),
                "calendar_id": "shared",
            }),
        ];
        merge_sort_and_cap(&mut events, 10);
        assert_eq!(events[0]["id"], json!("a"));
        assert_eq!(events[1]["id"], json!("b"));
        assert_eq!(events[2]["id"], json!("c"));
    }

    #[test]
    fn merge_sort_truncates_to_cap() {
        let mut events: Vec<Value> = (0..10)
            .map(|i| {
                json!({
                    "id": format!("evt-{i}"),
                    "start": at_offset((i as i64) * 600),
                })
            })
            .collect();
        merge_sort_and_cap(&mut events, 3);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0]["id"], json!("evt-0"));
        assert_eq!(events[1]["id"], json!("evt-1"));
        assert_eq!(events[2]["id"], json!("evt-2"));
    }

    #[test]
    fn merge_sort_handles_unparseable_start_by_pushing_to_end() {
        let mut events = vec![
            json!({"id": "a", "start": "garbage"}),
            json!({"id": "b", "start": at_offset(60)}),
            json!({"id": "c", "start": null}),
        ];
        merge_sort_and_cap(&mut events, 10);
        // Parseable event comes first; the two
        // unparseable ones fall to the end in
        // stable relative order.
        assert_eq!(events[0]["id"], json!("b"));
    }

    // ---- Phase 151 — cross-calendar dedup ----

    #[test]
    fn dedup_no_duplicates_returns_unchanged() {
        let events = vec![
            json!({"id": "a", "summary": "Standup", "start": at_offset(60)}),
            json!({"id": "b", "summary": "Review", "start": at_offset(120)}),
        ];
        let got = dedup_events(events, false);
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn dedup_exact_duplicate_keeps_first() {
        // Same event surfaces on both primary
        // and work calendars (cross-invite case).
        let start = at_offset(60);
        let events = vec![
            json!({
                "id": "personal-copy",
                "summary": "Team standup",
                "start": start,
                "calendar_id": "primary",
            }),
            json!({
                "id": "work-copy",
                "summary": "Team standup",
                "start": start,
                "calendar_id": "work@example.com",
            }),
        ];
        let got = dedup_events(events, false);
        assert_eq!(got.len(), 1);
        // First occurrence wins → the primary
        // calendar's copy survives, work's
        // copy drops.
        assert_eq!(got[0]["id"], json!("personal-copy"));
        assert_eq!(got[0]["calendar_id"], json!("primary"));
    }

    #[test]
    fn dedup_differ_by_summary_kept_both() {
        let start = at_offset(60);
        let events = vec![
            json!({"id": "a", "summary": "Standup", "start": start}),
            json!({"id": "b", "summary": "Review", "start": start}),
        ];
        let got = dedup_events(events, false);
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn dedup_differ_by_start_kept_both() {
        let events = vec![
            json!({"id": "a", "summary": "Standup", "start": at_offset(60)}),
            json!({"id": "b", "summary": "Standup", "start": at_offset(120)}),
        ];
        let got = dedup_events(events, false);
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn dedup_null_summary_is_handled_defensively() {
        // Two events with null summaries +
        // identical starts collapse to one
        // (their key is ("", "...")). This is
        // the documented honest behavior;
        // operators should never see this
        // in practice since Google always
        // surfaces a summary, but we don't
        // panic on malformed inputs.
        let start = at_offset(60);
        let events = vec![
            json!({"id": "a", "summary": null, "start": start}),
            json!({"id": "b", "summary": null, "start": start}),
        ];
        let got = dedup_events(events, false);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["id"], json!("a"));
    }

    #[test]
    fn dedup_empty_input_returns_empty() {
        let got = dedup_events(Vec::new(), false);
        assert!(got.is_empty());
        let got = dedup_events(Vec::new(), true);
        assert!(got.is_empty());
    }

    // ---- Phase 155 — fuzzy dedup ----

    #[test]
    fn normalize_summary_lowercases_and_trims() {
        assert_eq!(normalize_summary("Standup"), "standup");
        assert_eq!(normalize_summary("STANDUP"), "standup");
        assert_eq!(normalize_summary("  Standup  "), "standup");
        assert_eq!(normalize_summary("Team Standup"), "team standup");
    }

    #[test]
    fn dedup_fuzzy_identical_normalized_titles_dedupe() {
        let start = at_offset(60);
        let events = vec![
            json!({"id": "a", "summary": "Standup", "start": start.clone()}),
            json!({"id": "b", "summary": "  STANDUP  ", "start": start}),
        ];
        let got_phase_151 = dedup_events(events.clone(), false);
        assert_eq!(got_phase_151.len(), 2, "Phase 151 kept both");
        let got_phase_155 = dedup_events(events, true);
        assert_eq!(got_phase_155.len(), 1, "Phase 155 fuzzy collapses");
        assert_eq!(got_phase_155[0]["id"], json!("a"));
    }

    #[test]
    fn dedup_fuzzy_within_5min_bucket_dedupes() {
        let events = vec![
            json!({"id": "a", "summary": "Standup", "start": "2026-06-04T10:00:00+00:00"}),
            json!({"id": "b", "summary": "Standup", "start": "2026-06-04T10:01:30+00:00"}),
        ];
        let got = dedup_events(events, true);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["id"], json!("a"));
    }

    #[test]
    fn dedup_fuzzy_outside_5min_bucket_kept_both() {
        let events = vec![
            json!({"id": "a", "summary": "Standup", "start": "2026-06-04T10:00:00+00:00"}),
            json!({"id": "b", "summary": "Standup", "start": "2026-06-04T10:05:00+00:00"}),
        ];
        let got = dedup_events(events, true);
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn dedup_fuzzy_differ_by_suffix_kept_both() {
        // "Standup" vs "Standup — Team A" — the
        // suffix carries intentional operator
        // meaning; fuzzy normalization should NOT
        // collapse these.
        let start = at_offset(60);
        let events = vec![
            json!({"id": "a", "summary": "Standup", "start": start.clone()}),
            json!({"id": "b", "summary": "Standup — Team A", "start": start}),
        ];
        let got = dedup_events(events, true);
        assert_eq!(got.len(), 2);
    }

    // ---- Phase 158 — sliding-window dedup ----

    #[test]
    fn dedup_fuzzy_sliding_window_merges_adjacent_buckets() {
        // 10:04 + 10:06 — under Phase 155 bucket
        // flooring these fell in 10:00 and 10:05
        // buckets (kept separate). Under Phase
        // 158 sliding-window they're 2 min apart
        // — adjacency merges.
        let events = vec![
            json!({"id": "a", "summary": "Standup", "start": "2026-06-04T10:04:00+00:00"}),
            json!({"id": "b", "summary": "Standup", "start": "2026-06-04T10:06:00+00:00"}),
        ];
        let got = dedup_events(events, true);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["id"], json!("a"));
    }

    #[test]
    fn dedup_fuzzy_sliding_window_chain_merges_three_events() {
        // 10:00, 10:04, 10:08 — consecutive gaps
        // are 4 min and 4 min. Chain folds all
        // three into one cluster.
        let events = vec![
            json!({"id": "a", "summary": "Standup", "start": "2026-06-04T10:00:00+00:00"}),
            json!({"id": "b", "summary": "Standup", "start": "2026-06-04T10:04:00+00:00"}),
            json!({"id": "c", "summary": "Standup", "start": "2026-06-04T10:08:00+00:00"}),
        ];
        let got = dedup_events(events, true);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["id"], json!("a"));
    }

    #[test]
    fn dedup_fuzzy_sliding_window_chain_breaks_at_5min_gap() {
        // 10:00, 10:04 (merge), then 10:10 — gap
        // 6 min from 10:04 → new cluster.
        let events = vec![
            json!({"id": "a", "summary": "Standup", "start": "2026-06-04T10:00:00+00:00"}),
            json!({"id": "b", "summary": "Standup", "start": "2026-06-04T10:04:00+00:00"}),
            json!({"id": "c", "summary": "Standup", "start": "2026-06-04T10:10:00+00:00"}),
        ];
        let got = dedup_events(events, true);
        assert_eq!(got.len(), 2);
        let ids: Vec<&str> = got
            .iter()
            .map(|e| e["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["a", "c"]);
    }

    #[test]
    fn dedup_fuzzy_sliding_window_lowest_input_index_wins_regardless_of_time() {
        // Primary calendar (index 0) lists event
        // at 10:06; secondary (index 1) lists at
        // 10:04. Sliding-window merges them; the
        // primary's entry wins on lower input
        // index even though it's later in time.
        let events = vec![
            json!({"id": "primary", "summary": "Standup", "start": "2026-06-04T10:06:00+00:00"}),
            json!({"id": "secondary", "summary": "Standup", "start": "2026-06-04T10:04:00+00:00"}),
        ];
        let got = dedup_events(events, true);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["id"], json!("primary"));
    }

    #[test]
    fn dedup_fuzzy_sliding_window_preserves_input_order() {
        // Two separate events in reverse time
        // order get sorted internally for the
        // sweep, but output preserves input
        // order (matches Phase 151 semantics).
        let events = vec![
            json!({"id": "a", "summary": "Late", "start": "2026-06-04T15:00:00+00:00"}),
            json!({"id": "b", "summary": "Early", "start": "2026-06-04T09:00:00+00:00"}),
        ];
        let got = dedup_events(events, true);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0]["id"], json!("a"));
        assert_eq!(got[1]["id"], json!("b"));
    }

    #[test]
    fn dedup_fuzzy_sliding_window_unparseable_start_stays_alone() {
        // Unparseable start can't participate in
        // adjacency merge — stays in its own
        // cluster regardless of summary match.
        let events = vec![
            json!({"id": "a", "summary": "Standup", "start": "2026-06-04T10:00:00+00:00"}),
            json!({"id": "b", "summary": "Standup", "start": "not-a-time"}),
        ];
        let got = dedup_events(events, true);
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn fuzzy_dedup_default_true_in_parse_input() {
        let parsed = parse_input(&json!({})).unwrap();
        assert!(parsed.fuzzy_dedup);
    }

    #[test]
    fn fuzzy_dedup_false_honored_in_parse_input() {
        let parsed = parse_input(&json!({"fuzzy_dedup": false})).unwrap();
        assert!(!parsed.fuzzy_dedup);
    }

    // ---- Phase 155 — writable_only + max_concurrent ----

    #[test]
    fn writable_only_default_false() {
        let parsed = parse_input(&json!({})).unwrap();
        assert!(!parsed.writable_only);
    }

    #[test]
    fn writable_only_honored() {
        let parsed = parse_input(&json!({"writable_only": true})).unwrap();
        assert!(parsed.writable_only);
    }

    #[test]
    fn max_concurrent_default_none() {
        let parsed = parse_input(&json!({})).unwrap();
        assert!(parsed.max_concurrent.is_none());
    }

    #[test]
    fn max_concurrent_honored() {
        let parsed = parse_input(&json!({"max_concurrent": 3})).unwrap();
        assert_eq!(parsed.max_concurrent, Some(3));
    }

    #[test]
    fn max_concurrent_zero_rejected() {
        let err = parse_input(&json!({"max_concurrent": 0})).unwrap_err();
        assert!(err.contains(">= 1"), "{err}");
    }

    #[test]
    fn calendar_ids_explicit_tracks_operator_choice() {
        // Default — no calendar_id / calendar_ids
        // supplied.
        let parsed = parse_input(&json!({})).unwrap();
        assert!(!parsed.calendar_ids_explicit);
        // Single calendar_id explicit.
        let parsed =
            parse_input(&json!({"calendar_id": "primary"})).unwrap();
        assert!(parsed.calendar_ids_explicit);
        // Multi calendar_ids explicit.
        let parsed =
            parse_input(&json!({"calendar_ids": ["primary", "work@x"]})).unwrap();
        assert!(parsed.calendar_ids_explicit);
    }

    // ---- enrichment with calendar_id tag ----

    #[test]
    fn enrich_attaches_relative_time_imminent_and_calendar_id() {
        let event = json!({
            "id": "evt-1",
            "summary": "Standup",
            "start": { "dateTime": at_offset(15 * 60) },
            "end":   { "dateTime": at_offset(30 * 60) },
            "location": "Conf A",
            "attendees": [{"email": "a@x"}, {"email": "b@x"}],
        });
        let enriched = enrich_event(&event, now_fixed(), "work@x.com");
        assert_eq!(enriched["id"], json!("evt-1"));
        assert_eq!(enriched["summary"], json!("Standup"));
        assert_eq!(enriched["location"], json!("Conf A"));
        assert_eq!(enriched["attendee_count"], json!(2));
        assert_eq!(enriched["starts_in_human"], json!("in 15 minutes"));
        assert_eq!(enriched["is_imminent"], json!(true));
        // Phase 142 — calendar_id traceability.
        assert_eq!(enriched["calendar_id"], json!("work@x.com"));
    }

    #[test]
    fn enrich_marks_far_future_event_not_imminent() {
        let event = json!({
            "id": "evt-2",
            "summary": "Quarterly review",
            "start": { "dateTime": at_offset(72 * 3600) },
            "end":   { "dateTime": at_offset(73 * 3600) },
        });
        let enriched = enrich_event(&event, now_fixed(), "primary");
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
        let enriched = enrich_event(&event, now_fixed(), "primary");
        assert_eq!(enriched["starts_in_human"], json!("in 6 days"));
        assert_eq!(enriched["is_imminent"], json!(false));
    }
}
