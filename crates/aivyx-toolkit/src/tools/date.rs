//! `date.diff` + `date.add` — date arithmetic beyond `time.now`.
//!
//! Chapter Abacus (AB.3). The date group of the pure-compute
//! utilities pack. Both tools are gated by a single capability base,
//! **`date.compute`**, at the **SemiTrusted** ceiling — like
//! `calc.eval` (AB.1) and the convert group (AB.2), these compute
//! over their inputs and touch no network, no filesystem, and no
//! operator data (see `docs/ABACUS.md` §2).
//!
//! ## Why tools at all
//!
//! "How many days until 2026-12-25?" and "what's 90 days after
//! today?" are exactly the date arithmetic a language model
//! mis-counts (leap years, month lengths). A calendar-correct
//! computation makes the answer exact.
//!
//! ## Tool surface
//!
//! - `date.diff` — `{from: string, to: string}` →
//!   `{from, to, days, seconds}`. The signed span between two
//!   instants (`to - from`).
//! - `date.add` — `{date?: string, days?, hours?, minutes?,
//!   seconds?, weeks?}` → `{base, result}`. Add a (possibly
//!   negative) duration to a date.
//!
//! ## The one non-pure edge (AB.3)
//!
//! Both tools default a missing date to **now** (`Utc::now()`), so
//! `date.diff {to: "2026-12-25"}` answers "from now" and
//! `date.add {days: 90}` means "90 days from now". That default is
//! the *only* clock dependence in the whole utilities pack; supply an
//! explicit `from`/`date` for a fully deterministic call.

use async_trait::async_trait;
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{
    AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification,
};

/// The capability base gating the whole date group (`date.diff` +
/// `date.add`).
const DATE_SCOPE: &str = "date.compute";

// =====================================================================
// date.diff
// =====================================================================

pub struct DateDiff {
    id: ToolId,
    schema: Value,
}

impl Default for DateDiff {
    fn default() -> Self {
        Self::new()
    }
}

impl DateDiff {
    pub fn new() -> Self {
        Self {
            id: ToolId::new(),
            schema: diff_schema(),
        }
    }
}

#[async_trait]
impl Tool for DateDiff {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "date.diff"
    }
    fn description(&self) -> &str {
        "Compute the signed span between two instants (`to - \
         from`). Input: `{from: string (optional, default now), to: \
         string (optional, default now)}`. Dates accept \
         `2026-12-25`, `2026-12-25T09:00`, or RFC 3339 with an \
         offset (a bare date is midnight UTC). Returns `{from, to, \
         days, seconds}` where `days` is whole days (truncated) and \
         `seconds` is the exact signed total. Scope: `date.compute`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse(DATE_SCOPE)
            .expect("date.compute must parse — it is in KNOWN_BASES from Chapter Abacus")
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let from = match optional_datetime(&input, "from") {
            Ok(dt) => dt.unwrap_or_else(Utc::now),
            Err(e) => return failed(self.id, format!("date.diff: {e}")),
        };
        let to = match optional_datetime(&input, "to") {
            Ok(dt) => dt.unwrap_or_else(Utc::now),
            Err(e) => return failed(self.id, format!("date.diff: {e}")),
        };
        let delta = to - from;
        ToolOutcome::Completed {
            output: json!({
                "from": from.to_rfc3339(),
                "to": to.to_rfc3339(),
                "days": delta.num_days(),
                "seconds": delta.num_seconds(),
            }),
            verified: Verification::NotApplicable,
        }
    }
}

fn diff_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "from": { "type": "string", "description": "Start instant (optional, default now). E.g. `2026-12-25` or `2026-12-25T09:00`." },
            "to": { "type": "string", "description": "End instant (optional, default now)." }
        },
        "additionalProperties": false
    })
}

// =====================================================================
// date.add
// =====================================================================

pub struct DateAdd {
    id: ToolId,
    schema: Value,
}

impl Default for DateAdd {
    fn default() -> Self {
        Self::new()
    }
}

impl DateAdd {
    pub fn new() -> Self {
        Self {
            id: ToolId::new(),
            schema: add_schema(),
        }
    }
}

#[async_trait]
impl Tool for DateAdd {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "date.add"
    }
    fn description(&self) -> &str {
        "Add a (possibly negative) duration to a date. Input: \
         `{date: string (optional, default now), weeks?: number, \
         days?: number, hours?: number, minutes?: number, \
         seconds?: number}`. At least one duration field is \
         required; fields combine (and may be negative to \
         subtract). Returns `{base, result}` as RFC 3339. Scope: \
         `date.compute`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse(DATE_SCOPE)
            .expect("date.compute must parse — it is in KNOWN_BASES from Chapter Abacus")
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let base = match optional_datetime(&input, "date") {
            Ok(dt) => dt.unwrap_or_else(Utc::now),
            Err(e) => return failed(self.id, format!("date.add: {e}")),
        };
        let duration = match duration_from_input(&input) {
            Ok(d) => d,
            Err(e) => return failed(self.id, format!("date.add: {e}")),
        };
        let result = match base.checked_add_signed(duration) {
            Some(r) => r,
            None => return failed(self.id, "date.add: result is out of range".to_string()),
        };
        ToolOutcome::Completed {
            output: json!({
                "base": base.to_rfc3339(),
                "result": result.to_rfc3339(),
            }),
            verified: Verification::NotApplicable,
        }
    }
}

fn add_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "date": { "type": "string", "description": "Base instant (optional, default now)." },
            "weeks": { "type": "number", "description": "Weeks to add (may be negative)." },
            "days": { "type": "number", "description": "Days to add (may be negative)." },
            "hours": { "type": "number", "description": "Hours to add (may be negative)." },
            "minutes": { "type": "number", "description": "Minutes to add (may be negative)." },
            "seconds": { "type": "number", "description": "Seconds to add (may be negative)." }
        },
        "additionalProperties": false
    })
}

// =====================================================================
// Pure parsing / duration logic
// =====================================================================

/// Parse a date string accepting a bare date, a naive datetime, or an
/// RFC 3339 instant with offset. A bare date / naive datetime is
/// interpreted as **UTC** (the pack has no operator timezone).
pub fn parse_datetime(s: &str) -> Result<DateTime<Utc>, String> {
    let t = s.trim();
    // RFC 3339 with an explicit offset (e.g. `2026-12-25T09:00:00-05:00`).
    if let Ok(dt) = DateTime::parse_from_rfc3339(t) {
        return Ok(dt.with_timezone(&Utc));
    }
    // Naive datetime → assume UTC.
    const DT_FORMATS: &[&str] = &[
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%dT%H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
    ];
    for fmt in DT_FORMATS {
        if let Ok(ndt) = NaiveDateTime::parse_from_str(t, fmt) {
            return Ok(Utc.from_utc_datetime(&ndt));
        }
    }
    // Bare date → midnight UTC.
    if let Ok(d) = NaiveDate::parse_from_str(t, "%Y-%m-%d") {
        let ndt = d
            .and_hms_opt(0, 0, 0)
            .expect("midnight is always valid for a valid date");
        return Ok(Utc.from_utc_datetime(&ndt));
    }
    Err(format!(
        "could not parse `{s}` — expected `2026-12-25`, `2026-12-25T09:00`, or an RFC 3339 instant"
    ))
}

/// Read an optional datetime field. Absent / null → `None` (the
/// caller defaults to now); present-but-unparseable → error.
fn optional_datetime(input: &Value, field: &str) -> Result<Option<DateTime<Utc>>, String> {
    match input.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::String(s)) => parse_datetime(s).map(Some),
        Some(_) => Err(format!("`{field}` must be a date string if present")),
    }
}

/// Build a [`Duration`] from the numeric fields, requiring at least
/// one. Fractional values are rounded to whole seconds (the finest
/// unit a `Duration` carries here).
fn duration_from_input(input: &Value) -> Result<Duration, String> {
    const FIELDS: &[(&str, f64)] = &[
        ("weeks", 7.0 * 86_400.0),
        ("days", 86_400.0),
        ("hours", 3_600.0),
        ("minutes", 60.0),
        ("seconds", 1.0),
    ];
    let mut total_secs = 0.0_f64;
    let mut any = false;
    for (field, scale) in FIELDS {
        match input.get(*field) {
            None | Some(Value::Null) => {}
            Some(v) => {
                let n = v
                    .as_f64()
                    .ok_or_else(|| format!("`{field}` must be a number"))?;
                if !n.is_finite() {
                    return Err(format!("`{field}` must be a finite number"));
                }
                total_secs += n * scale;
                any = true;
            }
        }
    }
    if !any {
        return Err(
            "at least one of weeks/days/hours/minutes/seconds is required".to_string(),
        );
    }
    Ok(Duration::seconds(total_secs.round() as i64))
}

fn failed(id: ToolId, detail: String) -> ToolOutcome {
    ToolOutcome::Failed(AivyxError::Tool { tool: id, detail })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dt(s: &str) -> DateTime<Utc> {
        parse_datetime(s).unwrap_or_else(|e| panic!("parse({s:?}): {e}"))
    }

    // ---- parse_datetime -----------------------------------------

    #[test]
    fn parses_bare_date_as_midnight_utc() {
        let d = dt("2026-12-25");
        assert_eq!(d.to_rfc3339(), "2026-12-25T00:00:00+00:00");
    }

    #[test]
    fn parses_naive_datetime_as_utc() {
        assert_eq!(dt("2026-12-25T09:30").to_rfc3339(), "2026-12-25T09:30:00+00:00");
        assert_eq!(dt("2026-12-25 09:30:15").to_rfc3339(), "2026-12-25T09:30:15+00:00");
    }

    #[test]
    fn parses_rfc3339_with_offset() {
        // 09:00 at -05:00 is 14:00 UTC.
        assert_eq!(dt("2026-12-25T09:00:00-05:00").to_rfc3339(), "2026-12-25T14:00:00+00:00");
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse_datetime("not a date").is_err());
        assert!(parse_datetime("2026-13-40").is_err());
    }

    // ---- diff (via the Duration arithmetic) ---------------------

    #[test]
    fn diff_whole_days() {
        let delta = dt("2026-12-25") - dt("2026-12-01");
        assert_eq!(delta.num_days(), 24);
    }

    #[test]
    fn diff_crosses_leap_day() {
        // 2028 is a leap year: Feb has 29 days.
        let delta = dt("2028-03-01") - dt("2028-02-01");
        assert_eq!(delta.num_days(), 29);
    }

    #[test]
    fn diff_is_signed() {
        let delta = dt("2026-01-01") - dt("2026-01-08");
        assert_eq!(delta.num_days(), -7);
        assert_eq!(delta.num_seconds(), -7 * 86_400);
    }

    // ---- duration_from_input ------------------------------------

    #[test]
    fn duration_combines_fields() {
        let d = duration_from_input(&json!({"days": 1, "hours": 2, "minutes": 30})).unwrap();
        assert_eq!(d.num_seconds(), 86_400 + 2 * 3_600 + 30 * 60);
    }

    #[test]
    fn duration_weeks_and_negative() {
        assert_eq!(duration_from_input(&json!({"weeks": 2})).unwrap().num_days(), 14);
        assert_eq!(duration_from_input(&json!({"days": -3})).unwrap().num_days(), -3);
    }

    #[test]
    fn duration_requires_a_field() {
        assert!(duration_from_input(&json!({})).unwrap_err().contains("at least one"));
    }

    #[test]
    fn duration_rejects_non_number() {
        assert!(duration_from_input(&json!({"days": "lots"})).unwrap_err().contains("must be a number"));
    }

    #[test]
    fn add_ninety_days_calendar_correct() {
        // 2026-01-01 + 90 days = 2026-04-01 (Jan31 + Feb28 + Mar31 = 90).
        let base = dt("2026-01-01");
        let result = base + duration_from_input(&json!({"days": 90})).unwrap();
        assert_eq!(result.to_rfc3339(), "2026-04-01T00:00:00+00:00");
    }

    // ---- optional_datetime --------------------------------------

    #[test]
    fn optional_datetime_absent_is_none() {
        assert!(optional_datetime(&json!({}), "from").unwrap().is_none());
        assert!(optional_datetime(&json!({"from": ""}), "from").unwrap().is_none());
    }

    #[test]
    fn optional_datetime_present_parses() {
        let got = optional_datetime(&json!({"from": "2026-12-25"}), "from").unwrap();
        assert_eq!(got.unwrap().to_rfc3339(), "2026-12-25T00:00:00+00:00");
    }

    // ---- tool wiring --------------------------------------------

    #[test]
    fn tools_metadata_is_sound() {
        let d = DateDiff::new();
        assert_eq!(d.name(), "date.diff");
        assert!(!d.description().is_empty());
        assert_eq!(d.input_schema()["type"], "object");
        assert_eq!(d.required_scope(&json!({})), Scope::parse("date.compute").unwrap());

        let a = DateAdd::new();
        assert_eq!(a.name(), "date.add");
        assert!(!a.description().is_empty());
        // Both date tools share the date.compute group base.
        assert_eq!(a.required_scope(&json!({})), Scope::parse("date.compute").unwrap());
    }
}
