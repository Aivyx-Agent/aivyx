//! `budget.*` — lightweight personal spending
//! tracker.
//!
//! Phase 143 — Chapter G #2. Two tools sharing
//! the [`crate::budget_store::BudgetStore`]
//! handle through `Arc`. Two capability scopes:
//! `budget.read` for `budget.summary`;
//! `budget.write` for `budget.record`.
//!
//! Data lives at
//! `~/.aivyx/tool-processes/toolkit/budget.json`
//! (0600 perms, atomic write — see
//! [`crate::budget_store`] for the storage
//! substrate).
//!
//! ## Tool surface
//!
//! - `budget.record` — `{amount, category, note?}`
//!   → `{id, amount, category, note, recorded_at}`.
//! - `budget.summary` — `{period?, since?, until?}`
//!   → `{period, since, until, total, entry_count,
//!   by_category: [{category, total, count}]}`.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{
    AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification,
};

use crate::budget_store::BudgetStore;
use crate::budget_store::BudgetStoreError;

// =====================================================================
// budget.record
// =====================================================================

pub struct BudgetRecord {
    id: ToolId,
    schema: Value,
    store: Arc<BudgetStore>,
}

impl BudgetRecord {
    pub fn new(store: Arc<BudgetStore>) -> Self {
        Self {
            id: ToolId::new(),
            schema: record_schema(),
            store,
        }
    }
}

#[async_trait]
impl Tool for BudgetRecord {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "budget.record"
    }
    fn description(&self) -> &str {
        "Record a budget entry. Input: \
         `{amount: number (required; positive = \
         expense, negative = income/refund), \
         category: string (required; free-text — \
         \"food\", \"transport\", \"rent\"), \
         note: string (optional)}`. Returns \
         `{id, amount, category, note, \
         recorded_at}`. Scope: `budget.write`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("budget.write").expect(
            "budget.write must parse — it is in KNOWN_BASES from Phase 143",
        )
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let parsed = match parse_record_input(&input) {
            Ok(p) => p,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("budget.record: {reason}"),
                });
            }
        };
        let entry = match self
            .store
            .record(parsed.amount, parsed.category, parsed.note)
            .await
        {
            Ok(e) => e,
            Err(e) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("budget.record: {e}"),
                });
            }
        };
        ToolOutcome::Completed {
            output: json!({
                "id": entry.id,
                "amount": entry.amount,
                "category": entry.category,
                "note": entry.note,
                "recorded_at": entry.recorded_at.to_rfc3339(),
            }),
            verified: Verification::Verified,
        }
    }
}

fn record_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "amount": { "type": "number", "description": "Positive = expense, negative = income/refund" },
            "category": { "type": "string", "description": "Free-text category (food, transport, etc.)" },
            "note": { "type": "string", "description": "Optional context" }
        },
        "required": ["amount", "category"],
        "additionalProperties": false
    })
}

#[derive(Debug)]
struct RecordInput {
    amount: f64,
    category: String,
    note: Option<String>,
}

fn parse_record_input(input: &Value) -> Result<RecordInput, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
    let amount = obj
        .get("amount")
        .ok_or_else(|| "`amount` is required".to_string())?
        .as_f64()
        .ok_or_else(|| "`amount` must be a number".to_string())?;
    let category = obj
        .get("category")
        .ok_or_else(|| "`category` is required".to_string())?
        .as_str()
        .ok_or_else(|| "`category` must be a string".to_string())?
        .to_string();
    let note = match obj.get("note") {
        None => None,
        Some(Value::Null) => None,
        Some(v) => Some(
            v.as_str()
                .ok_or_else(|| "`note` must be a string".to_string())?
                .to_string(),
        ),
    };
    Ok(RecordInput {
        amount,
        category,
        note,
    })
}

// =====================================================================
// budget.summary
// =====================================================================

pub struct BudgetSummaryTool {
    id: ToolId,
    schema: Value,
    store: Arc<BudgetStore>,
}

impl BudgetSummaryTool {
    pub fn new(store: Arc<BudgetStore>) -> Self {
        Self {
            id: ToolId::new(),
            schema: summary_schema(),
            store,
        }
    }
}

#[async_trait]
impl Tool for BudgetSummaryTool {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "budget.summary"
    }
    fn description(&self) -> &str {
        "Aggregate budget entries over a period. \
         Input: `{period: \"today\" | \"this_week\" | \
         \"this_month\" | \"this_year\" | \
         \"all_time\" (default \"this_week\"), \
         since: string (optional, RFC 3339 — \
         overrides period lower bound), until: \
         string (optional, RFC 3339 — overrides \
         period upper bound)}`. Period definitions \
         are calendar-based (this_week = current \
         ISO week Mon-Sun; this_month = current \
         calendar month). For rolling windows \
         (\"last 7 days\"), pass explicit since/ \
         until. Returns `{period, since, until, \
         total, entry_count, by_category: \
         [{category, total, count}]}` sorted \
         descending by total. Scope: `budget.read`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("budget.read")
            .expect("budget.read must parse — it is in KNOWN_BASES from Phase 143")
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let parsed = match parse_summary_input(&input, Utc::now()) {
            Ok(p) => p,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("budget.summary: {reason}"),
                });
            }
        };
        let summary = self.store.summary(parsed.since, parsed.until).await;
        let by_category: Vec<Value> = summary
            .by_category
            .iter()
            .map(|c| {
                json!({
                    "category": c.category,
                    "total": c.total,
                    "count": c.count,
                })
            })
            .collect();
        ToolOutcome::Completed {
            output: json!({
                "period": parsed.period_label,
                "since": parsed.since.to_rfc3339(),
                "until": parsed.until.to_rfc3339(),
                "total": summary.total,
                "entry_count": summary.entry_count,
                "by_category": by_category,
            }),
            verified: Verification::NotApplicable,
        }
    }
}

fn summary_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "period": {
                "type": "string",
                "enum": ["today", "this_week", "this_month", "this_year", "all_time"],
                "description": "Calendar-based window (default this_week)"
            },
            "since": { "type": "string", "description": "RFC 3339 lower bound (inclusive) — overrides period" },
            "until": { "type": "string", "description": "RFC 3339 upper bound (exclusive) — overrides period" }
        },
        "additionalProperties": false
    })
}

#[derive(Debug)]
struct SummaryInput {
    period_label: String,
    since: DateTime<Utc>,
    until: DateTime<Utc>,
}

/// Parameterized `now` so tests can pin deterministic
/// windows.
fn parse_summary_input(input: &Value, now: DateTime<Utc>) -> Result<SummaryInput, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
    let period = match obj.get("period") {
        None => "this_week".to_string(),
        Some(v) => v
            .as_str()
            .ok_or_else(|| "`period` must be a string".to_string())?
            .to_string(),
    };
    let (mut since, mut until) = period_window(&period, now)?;

    if let Some(v) = obj.get("since") {
        let s = v
            .as_str()
            .ok_or_else(|| "`since` must be an RFC 3339 string".to_string())?;
        since = DateTime::parse_from_rfc3339(s)
            .map_err(|e| format!("`since` parse: {e}"))?
            .with_timezone(&Utc);
    }
    if let Some(v) = obj.get("until") {
        let s = v
            .as_str()
            .ok_or_else(|| "`until` must be an RFC 3339 string".to_string())?;
        until = DateTime::parse_from_rfc3339(s)
            .map_err(|e| format!("`until` parse: {e}"))?
            .with_timezone(&Utc);
    }
    if since >= until {
        return Err("`since` must be earlier than `until`".to_string());
    }
    Ok(SummaryInput {
        period_label: period,
        since,
        until,
    })
}

/// Map a period label to a (since, until) window
/// anchored on `now`. Pure substrate.
fn period_window(period: &str, now: DateTime<Utc>) -> Result<(DateTime<Utc>, DateTime<Utc>), String> {
    match period {
        "today" => {
            let start = day_start(now);
            let end = start + chrono::Duration::days(1);
            Ok((start, end))
        }
        "this_week" => {
            // ISO week starts Monday.
            let today_start = day_start(now);
            let weekday_from_mon = now.weekday().num_days_from_monday() as i64;
            let monday = today_start - chrono::Duration::days(weekday_from_mon);
            let next_monday = monday + chrono::Duration::days(7);
            Ok((monday, next_monday))
        }
        "this_month" => {
            let first_of_month = Utc
                .with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
                .single()
                .ok_or_else(|| "month boundary failed".to_string())?;
            let (next_year, next_month) = if now.month() == 12 {
                (now.year() + 1, 1)
            } else {
                (now.year(), now.month() + 1)
            };
            let first_of_next = Utc
                .with_ymd_and_hms(next_year, next_month, 1, 0, 0, 0)
                .single()
                .ok_or_else(|| "next month boundary failed".to_string())?;
            Ok((first_of_month, first_of_next))
        }
        "this_year" => {
            let first_of_year = Utc
                .with_ymd_and_hms(now.year(), 1, 1, 0, 0, 0)
                .single()
                .ok_or_else(|| "year boundary failed".to_string())?;
            let first_of_next_year = Utc
                .with_ymd_and_hms(now.year() + 1, 1, 1, 0, 0, 0)
                .single()
                .ok_or_else(|| "next year boundary failed".to_string())?;
            Ok((first_of_year, first_of_next_year))
        }
        "all_time" => {
            let zero = Utc
                .with_ymd_and_hms(1970, 1, 1, 0, 0, 0)
                .single()
                .ok_or_else(|| "epoch boundary failed".to_string())?;
            let far = Utc
                .with_ymd_and_hms(2999, 12, 31, 23, 59, 59)
                .single()
                .ok_or_else(|| "far-future boundary failed".to_string())?;
            Ok((zero, far))
        }
        other => Err(format!("unknown period {other:?}")),
    }
}

fn day_start(t: DateTime<Utc>) -> DateTime<Utc> {
    let d: NaiveDate = t.date_naive();
    Utc.from_utc_datetime(&d.and_hms_opt(0, 0, 0).expect("00:00 valid"))
}

// =====================================================================
// budget.update — Phase 144
// =====================================================================

pub struct BudgetUpdate {
    id: ToolId,
    schema: Value,
    store: Arc<BudgetStore>,
}

impl BudgetUpdate {
    pub fn new(store: Arc<BudgetStore>) -> Self {
        Self {
            id: ToolId::new(),
            schema: update_schema(),
            store,
        }
    }
}

#[async_trait]
impl Tool for BudgetUpdate {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "budget.update"
    }
    fn description(&self) -> &str {
        "Update an existing budget entry by id. \
         Partial update: only the fields you \
         supply change. Input: `{id: string \
         (required), amount?: number, category?: \
         string, note?: string|null}`. For \
         `note`, JSON `null` explicitly clears \
         the note; omitting the key leaves it \
         unchanged (standard JSON-PATCH \
         semantics). Returns the updated entry. \
         Errors with NotFound if no entry has the \
         given id. Scope: `budget.write`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("budget.write").expect(
            "budget.write must parse — it is in KNOWN_BASES from Phase 143",
        )
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let parsed = match parse_update_input(&input) {
            Ok(p) => p,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("budget.update: {reason}"),
                });
            }
        };
        let result = self
            .store
            .update(&parsed.id, parsed.amount, parsed.category, parsed.note)
            .await;
        match result {
            Ok(entry) => ToolOutcome::Completed {
                output: json!({
                    "id": entry.id,
                    "amount": entry.amount,
                    "category": entry.category,
                    "note": entry.note,
                    "recorded_at": entry.recorded_at.to_rfc3339(),
                }),
                verified: Verification::Verified,
            },
            Err(BudgetStoreError::NotFound(id)) => ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("budget.update: no entry with id {id:?}"),
            }),
            Err(e) => ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("budget.update: {e}"),
            }),
        }
    }
}

fn update_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "string" },
            "amount": { "type": "number" },
            "category": { "type": "string" },
            "note": { "type": ["string", "null"], "description": "Null explicitly clears; absent key leaves unchanged" }
        },
        "required": ["id"],
        "additionalProperties": false
    })
}

#[derive(Debug)]
struct UpdateInput {
    id: String,
    amount: Option<f64>,
    category: Option<String>,
    note: Option<Option<String>>,
}

fn parse_update_input(input: &Value) -> Result<UpdateInput, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
    let id = obj
        .get("id")
        .ok_or_else(|| "`id` is required".to_string())?
        .as_str()
        .ok_or_else(|| "`id` must be a string".to_string())?
        .to_string();
    if id.is_empty() {
        return Err("`id` must not be empty".to_string());
    }
    let amount = match obj.get("amount") {
        None => None,
        Some(v) => Some(
            v.as_f64()
                .ok_or_else(|| "`amount` must be a number".to_string())?,
        ),
    };
    let category = match obj.get("category") {
        None => None,
        Some(v) => Some(
            v.as_str()
                .ok_or_else(|| "`category` must be a string".to_string())?
                .to_string(),
        ),
    };
    // Phase 144 — JSON null vs absent-key
    // distinguished here. Absent → leave alone
    // (Option::None on the outer); null →
    // explicit clear (Some(None) on the outer);
    // string → Some(Some(s)) on the outer.
    let note = if obj.contains_key("note") {
        let v = obj.get("note").expect("contains_key just succeeded");
        if v.is_null() {
            Some(None)
        } else {
            Some(Some(
                v.as_str()
                    .ok_or_else(|| {
                        "`note` must be a string or null".to_string()
                    })?
                    .to_string(),
            ))
        }
    } else {
        None
    };
    Ok(UpdateInput {
        id,
        amount,
        category,
        note,
    })
}

// =====================================================================
// budget.delete — Phase 144
// =====================================================================

pub struct BudgetDelete {
    id: ToolId,
    schema: Value,
    store: Arc<BudgetStore>,
}

impl BudgetDelete {
    pub fn new(store: Arc<BudgetStore>) -> Self {
        Self {
            id: ToolId::new(),
            schema: delete_schema(),
            store,
        }
    }
}

#[async_trait]
impl Tool for BudgetDelete {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "budget.delete"
    }
    fn description(&self) -> &str {
        "Delete a budget entry by id. Idempotent — \
         deleting a missing id succeeds with \
         `was_already_deleted: true` rather than \
         erroring. Same posture as \
         `calendar.delete_event`. Input: \
         `{id: string}`. Returns `{id, \
         was_already_deleted}`. Scope: \
         `budget.write`."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("budget.write").expect(
            "budget.write must parse — it is in KNOWN_BASES from Phase 143",
        )
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let id = match parse_delete_input(&input) {
            Ok(id) => id,
            Err(reason) => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: format!("budget.delete: {reason}"),
                });
            }
        };
        match self.store.delete(&id).await {
            Ok(outcome) => ToolOutcome::Completed {
                output: json!({
                    "id": outcome.id,
                    "was_already_deleted": outcome.was_already_deleted,
                }),
                verified: Verification::Verified,
            },
            Err(e) => ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("budget.delete: {e}"),
            }),
        }
    }
}

fn delete_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "id": { "type": "string" }
        },
        "required": ["id"],
        "additionalProperties": false
    })
}

fn parse_delete_input(input: &Value) -> Result<String, String> {
    let obj = input
        .as_object()
        .ok_or_else(|| "input must be a JSON object".to_string())?;
    let id = obj
        .get("id")
        .ok_or_else(|| "`id` is required".to_string())?
        .as_str()
        .ok_or_else(|| "`id` must be a string".to_string())?
        .to_string();
    if id.is_empty() {
        return Err("`id` must not be empty".to_string());
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn record_input_minimum_fields() {
        let parsed = parse_record_input(&json!({
            "amount": 12.50,
            "category": "food",
        }))
        .unwrap();
        assert_eq!(parsed.amount, 12.50);
        assert_eq!(parsed.category, "food");
        assert!(parsed.note.is_none());
    }

    #[test]
    fn record_input_with_note() {
        let parsed = parse_record_input(&json!({
            "amount": 4.75,
            "category": "coffee",
            "note": "morning",
        }))
        .unwrap();
        assert_eq!(parsed.note, Some("morning".to_string()));
    }

    #[test]
    fn record_input_rejects_missing_amount() {
        let err = parse_record_input(&json!({ "category": "food" })).unwrap_err();
        assert!(err.contains("`amount`"), "{err}");
    }

    #[test]
    fn record_input_rejects_non_number_amount() {
        let err =
            parse_record_input(&json!({ "amount": "five", "category": "food" }))
                .unwrap_err();
        assert!(err.contains("must be a number"), "{err}");
    }

    #[test]
    fn summary_default_period_is_this_week() {
        // 2026-06-03 is a Wednesday — ISO week
        // starts 2026-06-01 Mon; next 2026-06-08.
        let now = ts("2026-06-03T12:00:00Z");
        let parsed = parse_summary_input(&json!({}), now).unwrap();
        assert_eq!(parsed.period_label, "this_week");
        assert_eq!(parsed.since, ts("2026-06-01T00:00:00Z"));
        assert_eq!(parsed.until, ts("2026-06-08T00:00:00Z"));
    }

    #[test]
    fn summary_today_window() {
        let now = ts("2026-06-03T15:30:00Z");
        let parsed = parse_summary_input(&json!({"period": "today"}), now).unwrap();
        assert_eq!(parsed.since, ts("2026-06-03T00:00:00Z"));
        assert_eq!(parsed.until, ts("2026-06-04T00:00:00Z"));
    }

    #[test]
    fn summary_this_month_window() {
        let now = ts("2026-06-15T08:00:00Z");
        let parsed =
            parse_summary_input(&json!({"period": "this_month"}), now).unwrap();
        assert_eq!(parsed.since, ts("2026-06-01T00:00:00Z"));
        assert_eq!(parsed.until, ts("2026-07-01T00:00:00Z"));
    }

    #[test]
    fn summary_this_month_december_handles_year_rollover() {
        let now = ts("2026-12-15T08:00:00Z");
        let parsed =
            parse_summary_input(&json!({"period": "this_month"}), now).unwrap();
        assert_eq!(parsed.since, ts("2026-12-01T00:00:00Z"));
        assert_eq!(parsed.until, ts("2027-01-01T00:00:00Z"));
    }

    #[test]
    fn summary_this_year_window() {
        let now = ts("2026-06-15T08:00:00Z");
        let parsed = parse_summary_input(&json!({"period": "this_year"}), now).unwrap();
        assert_eq!(parsed.since, ts("2026-01-01T00:00:00Z"));
        assert_eq!(parsed.until, ts("2027-01-01T00:00:00Z"));
    }

    #[test]
    fn summary_explicit_since_until_override_period() {
        let now = ts("2026-06-03T12:00:00Z");
        let parsed = parse_summary_input(
            &json!({
                "period": "this_week",
                "since": "2026-05-01T00:00:00Z",
                "until": "2026-05-31T23:59:59Z",
            }),
            now,
        )
        .unwrap();
        // Period label preserved, but actual since/
        // until come from explicit overrides.
        assert_eq!(parsed.period_label, "this_week");
        assert_eq!(parsed.since, ts("2026-05-01T00:00:00Z"));
        assert_eq!(parsed.until, ts("2026-05-31T23:59:59Z"));
    }

    #[test]
    fn summary_inverted_window_rejected() {
        let now = ts("2026-06-03T12:00:00Z");
        let err = parse_summary_input(
            &json!({
                "since": "2026-06-04T00:00:00Z",
                "until": "2026-06-03T00:00:00Z",
            }),
            now,
        )
        .unwrap_err();
        assert!(err.contains("earlier than"), "{err}");
    }

    #[test]
    fn summary_unknown_period_rejected() {
        let now = ts("2026-06-03T12:00:00Z");
        let err = parse_summary_input(&json!({"period": "last_week"}), now).unwrap_err();
        assert!(err.contains("unknown period"), "{err}");
    }

    // ---- Phase 144 — update + delete input parsing ----

    #[test]
    fn update_input_id_only_leaves_all_fields_unchanged() {
        let parsed = parse_update_input(&json!({ "id": "abc" })).unwrap();
        assert_eq!(parsed.id, "abc");
        assert!(parsed.amount.is_none());
        assert!(parsed.category.is_none());
        assert!(parsed.note.is_none());
    }

    #[test]
    fn update_input_amount_and_category_replace() {
        let parsed = parse_update_input(&json!({
            "id": "abc",
            "amount": 7.50,
            "category": "transport",
        }))
        .unwrap();
        assert_eq!(parsed.amount, Some(7.50));
        assert_eq!(parsed.category, Some("transport".to_string()));
        // note absent → leave unchanged.
        assert!(parsed.note.is_none());
    }

    #[test]
    fn update_input_note_string_means_replace() {
        let parsed = parse_update_input(&json!({
            "id": "abc",
            "note": "new note",
        }))
        .unwrap();
        // Some(Some("new note")) — "explicit
        // replace".
        assert_eq!(parsed.note, Some(Some("new note".to_string())));
    }

    #[test]
    fn update_input_note_null_means_explicit_clear() {
        let parsed = parse_update_input(&json!({
            "id": "abc",
            "note": null,
        }))
        .unwrap();
        // Some(None) — "explicit clear".
        assert_eq!(parsed.note, Some(None));
    }

    #[test]
    fn update_input_missing_id_rejected() {
        let err = parse_update_input(&json!({ "amount": 5.0 })).unwrap_err();
        assert!(err.contains("`id`"), "{err}");
    }

    #[test]
    fn update_input_empty_id_rejected() {
        let err = parse_update_input(&json!({ "id": "" })).unwrap_err();
        assert!(err.contains("must not be empty"), "{err}");
    }

    #[test]
    fn delete_input_simple_id() {
        let id = parse_delete_input(&json!({ "id": "abc-123" })).unwrap();
        assert_eq!(id, "abc-123");
    }

    #[test]
    fn delete_input_missing_id_rejected() {
        let err = parse_delete_input(&json!({})).unwrap_err();
        assert!(err.contains("`id`"), "{err}");
    }
}
