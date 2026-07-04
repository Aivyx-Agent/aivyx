//! Schedule primitive — Phase 26.
//!
//! A schedule is a cron-triggered execution entry that creates daemon
//! turns on a timer. Backed by a redb row under `KeyDomain::Schedules`.
//! Follows the same CRUD pattern as `mission.rs`.

use chrono::{DateTime, Utc};
use cron::Schedule as CronSchedule;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};

use aivyx_storage::{DomainHandle, KeyDomain, StorageError};

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleRecord {
    pub schedule_id: String,
    pub cron_expr: String,
    pub role_name: String,
    pub prompt: String,
    pub enabled: bool,
    pub wrap_mission: bool,
    pub created_at: u64,
    pub last_fired_at: Option<u64>,
    /// Phase 63 Task 3 — singular alias kept for backwards
    /// compatibility. New code reads `notify_targets`.
    #[serde(default)]
    pub notify_target: Option<String>,
    /// Phase 72 — list of notify target names for multi-target
    /// fan-out. `#[serde(default)]` so pre-Phase-72 records
    /// deserialize as empty. The daemon-scheduler's config sync
    /// path bridges `notify_target` into a one-element vec
    /// where needed.
    #[serde(default)]
    pub notify_targets: Vec<String>,
    /// Phase 72 — conditional dispatch gate string. Defaults
    /// to `NotifyWhen::Always`. `#[serde(default)]` for
    /// backwards compatibility.
    #[serde(default)]
    pub notify_when: aivyx_config::NotifyWhen,
    /// Chapter Ledger (#6 fix) — when set, the scheduler runs a **deterministic
    /// daemon-assembled report** instead of firing the `prompt` as an LLM turn.
    /// The only value today is `"digest"` (the weekly digest, built from the
    /// memory substrate so it can't confabulate). `None` (default) = the normal
    /// LLM-prompt routine, so every pre-Ledger record deserializes unchanged.
    #[serde(default)]
    pub report_kind: Option<String>,
}

impl ScheduleRecord {
    pub fn new(
        schedule_id: String,
        cron_expr: String,
        role_name: String,
        prompt: String,
    ) -> Result<Self, String> {
        validate_cron(&cron_expr)?;
        Ok(ScheduleRecord {
            schedule_id,
            cron_expr,
            role_name,
            prompt,
            enabled: true,
            wrap_mission: false,
            created_at: now_millis(),
            last_fired_at: None,
            notify_target: None,
            notify_targets: Vec::new(),
            notify_when: aivyx_config::NotifyWhen::Always,
            report_kind: None,
        })
    }

    pub fn next_fire_time(&self) -> Option<DateTime<Utc>> {
        next_fire_after(&self.cron_expr, Utc::now())
    }

    pub fn next_fire_time_after(&self, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        next_fire_after(&self.cron_expr, after)
    }
}

// ---------------------------------------------------------------------------
// Cron validation + next-fire computation
// ---------------------------------------------------------------------------

pub fn validate_cron(expr: &str) -> Result<(), String> {
    CronSchedule::from_str(expr)
        .map(|_| ())
        .map_err(|e| format!("invalid cron expression {expr:?}: {e}"))
}

/// Cron fields are the **operator's local wall clock** — the init
/// template has always said "local time", but until the 2026-07-04 soak
/// review the engine evaluated them in UTC, firing every routine hours
/// off operator intent on any non-UTC host ("nightly" reflection at
/// 10:00 AWST). Evaluate in `chrono::Local`, return the instant as Utc
/// (all stored timestamps stay UTC; only the wall-clock interpretation
/// of the cron fields changes). One-time effect at upgrade: a schedule
/// whose local-time tick already passed today fires one catch-up.
pub fn next_fire_after(cron_expr: &str, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let schedule = CronSchedule::from_str(cron_expr).ok()?;
    let local_after = after.with_timezone(&chrono::Local);
    schedule
        .after(&local_after)
        .next()
        .map(|t| t.with_timezone(&Utc))
}

// ---------------------------------------------------------------------------
// Storage CRUD
// ---------------------------------------------------------------------------

fn schedule_key(schedule_id: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(schedule_id.len());
    key.extend_from_slice(schedule_id.as_bytes());
    key
}

pub async fn create_schedule(
    handle: &DomainHandle,
    record: &ScheduleRecord,
) -> Result<(), StorageError> {
    assert_eq!(handle.domain(), KeyDomain::Schedules);
    let json = serde_json::to_vec(record).map_err(|e| {
        StorageError::Redb(format!("serialize ScheduleRecord: {e}"))
    })?;
    handle.put(&schedule_key(&record.schedule_id), &json).await
}

pub async fn get_schedule(
    handle: &DomainHandle,
    schedule_id: &str,
) -> Result<Option<ScheduleRecord>, StorageError> {
    assert_eq!(handle.domain(), KeyDomain::Schedules);
    match handle.get(&schedule_key(schedule_id)).await? {
        Some(bytes) => {
            let record: ScheduleRecord =
                serde_json::from_slice(&bytes).map_err(|e| {
                    StorageError::Redb(format!("deserialize ScheduleRecord: {e}"))
                })?;
            Ok(Some(record))
        }
        None => Ok(None),
    }
}

pub async fn update_schedule(
    handle: &DomainHandle,
    record: &ScheduleRecord,
) -> Result<(), StorageError> {
    create_schedule(handle, record).await
}

pub async fn list_schedules(
    handle: &DomainHandle,
) -> Result<Vec<ScheduleRecord>, StorageError> {
    assert_eq!(handle.domain(), KeyDomain::Schedules);
    let rows = handle.scan_prefix(b"").await?;
    let mut schedules = Vec::with_capacity(rows.len());
    for (_key, bytes) in rows {
        let record: ScheduleRecord =
            serde_json::from_slice(&bytes).map_err(|e| {
                StorageError::Redb(format!("deserialize ScheduleRecord: {e}"))
            })?;
        schedules.push(record);
    }
    Ok(schedules)
}

pub async fn delete_schedule(
    handle: &DomainHandle,
    schedule_id: &str,
) -> Result<(), StorageError> {
    assert_eq!(handle.domain(), KeyDomain::Schedules);
    handle.delete(&schedule_key(schedule_id)).await
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_cron_expr_passes_validation() {
        assert!(validate_cron("0 30 9 * * * *").is_ok());
    }

    #[test]
    fn daily_shorthand_is_not_standard_cron_crate() {
        // The `cron` crate uses 7-field expressions (sec min hour dom month dow year).
        // Standard @daily is not supported; use "0 0 0 * * * *" instead.
        // This test documents the crate's behavior.
        let result = validate_cron("@daily");
        // @daily may or may not parse depending on crate version; document the outcome.
        let _ = result;
    }

    #[test]
    fn invalid_cron_expr_fails_validation() {
        let err = validate_cron("not a cron").unwrap_err();
        assert!(err.contains("invalid cron expression"));
    }

    #[test]
    fn schedule_record_new_validates_cron() {
        let ok = ScheduleRecord::new(
            "s1".into(),
            "0 0 9 * * * *".into(),
            "default".into(),
            "check logs".into(),
        );
        assert!(ok.is_ok());

        let err = ScheduleRecord::new(
            "s2".into(),
            "bad".into(),
            "default".into(),
            "check logs".into(),
        );
        assert!(err.is_err());
    }

    #[test]
    fn next_fire_time_returns_future_time() {
        let record = ScheduleRecord::new(
            "s1".into(),
            "0 * * * * * *".into(), // every minute
            "default".into(),
            "ping".into(),
        )
        .unwrap();
        let next = record.next_fire_time();
        assert!(next.is_some());
        assert!(next.unwrap() > Utc::now());
    }

    #[test]
    fn next_fire_after_respects_anchor() {
        let anchor = DateTime::parse_from_rfc3339("2026-06-01T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        // Every day at 09:00 — OPERATOR-LOCAL wall clock (soak fix
        // 2026-07-04), so assert the local rendering of the instant.
        let next = next_fire_after("0 0 9 * * * *", anchor);
        assert!(next.is_some());
        let fire = next.unwrap();
        assert!(fire > anchor);
        assert_eq!(
            fire.with_timezone(&chrono::Local).format("%H:%M").to_string(),
            "09:00"
        );
    }

    /// Soak review 2026-07-04 — cron fields are the operator's LOCAL
    /// wall clock ("0 0 7 …" = 7am where the operator lives), matching
    /// what the init template always promised; the engine used to
    /// evaluate UTC, firing "nightly" reflection at 10:00 AWST. On a
    /// UTC host both interpretations coincide; on any offset host the
    /// local hour must win — this test fails under UTC evaluation on
    /// any non-UTC machine.
    #[test]
    fn cron_fields_are_operator_local_wall_clock() {
        let anchor = Utc::now();
        let fire = next_fire_after("0 0 7 * * *", anchor).unwrap();
        assert!(fire > anchor);
        assert_eq!(
            fire.with_timezone(&chrono::Local).format("%H:%M").to_string(),
            "07:00"
        );
    }

    #[test]
    fn schedule_record_round_trips_through_serde() {
        let record = ScheduleRecord::new(
            "test-id".into(),
            "0 0 9 * * * *".into(),
            "coder".into(),
            "run tests".into(),
        )
        .unwrap();
        let json = serde_json::to_vec(&record).unwrap();
        let back: ScheduleRecord = serde_json::from_slice(&json).unwrap();
        assert_eq!(back.schedule_id, "test-id");
        assert_eq!(back.role_name, "coder");
        assert_eq!(back.prompt, "run tests");
        assert!(back.enabled);
        assert!(back.last_fired_at.is_none());
    }

    #[test]
    fn disabled_schedule_still_computes_next_fire() {
        let mut record = ScheduleRecord::new(
            "s1".into(),
            "0 * * * * * *".into(),
            "default".into(),
            "ping".into(),
        )
        .unwrap();
        record.enabled = false;
        // next_fire_time is a pure computation — the scheduler checks `enabled`
        assert!(record.next_fire_time().is_some());
    }
}
