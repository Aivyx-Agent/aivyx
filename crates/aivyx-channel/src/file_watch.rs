//! File-watch trigger primitive — Phase 27 Task 4.
//!
//! A file watch is a filesystem-change-triggered execution entry that
//! creates daemon turns when files matching a path pattern are modified.
//! Backed by a redb row under `KeyDomain::FileWatches`. Follows the same
//! CRUD pattern as `schedule.rs` and `webhook.rs`.

use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use aivyx_storage::{DomainHandle, KeyDomain, StorageError};

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileWatchRecord {
    pub watch_id: String,
    /// Path to watch. Can be a file or directory.
    pub path: String,
    pub role_name: String,
    pub prompt: String,
    pub enabled: bool,
    pub wrap_mission: bool,
    /// Debounce interval in milliseconds. Events within this window
    /// after the last fire are suppressed.
    pub debounce_ms: u64,
    pub created_at: u64,
    pub last_fired_at: Option<u64>,
    /// Phase 63 Task 3 — see [`crate::schedule::ScheduleRecord::notify_target`].
    #[serde(default)]
    pub notify_target: Option<String>,
}

/// Default debounce interval: 2 seconds. Prevents rapid re-fires from
/// editor save storms (write → rename → chmod sequences).
pub const DEFAULT_DEBOUNCE_MS: u64 = 2000;

impl FileWatchRecord {
    pub fn new(
        watch_id: String,
        path: String,
        role_name: String,
        prompt: String,
    ) -> Self {
        FileWatchRecord {
            watch_id,
            path,
            role_name,
            prompt,
            enabled: true,
            wrap_mission: false,
            debounce_ms: DEFAULT_DEBOUNCE_MS,
            created_at: now_millis(),
            last_fired_at: None,
            notify_target: None,
        }
    }

    /// Returns true if enough time has passed since the last fire
    /// to allow another fire (debounce check).
    pub fn should_fire(&self) -> bool {
        match self.last_fired_at {
            Some(last) => {
                let now = now_millis();
                now.saturating_sub(last) >= self.debounce_ms
            }
            None => true,
        }
    }
}

// ---------------------------------------------------------------------------
// Storage CRUD
// ---------------------------------------------------------------------------

fn watch_key(watch_id: &str) -> Vec<u8> {
    let mut key = Vec::with_capacity(watch_id.len());
    key.extend_from_slice(watch_id.as_bytes());
    key
}

pub async fn create_file_watch(
    handle: &DomainHandle,
    record: &FileWatchRecord,
) -> Result<(), StorageError> {
    assert_eq!(handle.domain(), KeyDomain::FileWatches);
    let json = serde_json::to_vec(record).map_err(|e| {
        StorageError::Redb(format!("serialize FileWatchRecord: {e}"))
    })?;
    handle.put(&watch_key(&record.watch_id), &json).await
}

pub async fn get_file_watch(
    handle: &DomainHandle,
    watch_id: &str,
) -> Result<Option<FileWatchRecord>, StorageError> {
    assert_eq!(handle.domain(), KeyDomain::FileWatches);
    match handle.get(&watch_key(watch_id)).await? {
        Some(bytes) => {
            let record: FileWatchRecord =
                serde_json::from_slice(&bytes).map_err(|e| {
                    StorageError::Redb(format!("deserialize FileWatchRecord: {e}"))
                })?;
            Ok(Some(record))
        }
        None => Ok(None),
    }
}

pub async fn update_file_watch(
    handle: &DomainHandle,
    record: &FileWatchRecord,
) -> Result<(), StorageError> {
    create_file_watch(handle, record).await
}

pub async fn list_file_watches(
    handle: &DomainHandle,
) -> Result<Vec<FileWatchRecord>, StorageError> {
    assert_eq!(handle.domain(), KeyDomain::FileWatches);
    let rows = handle.scan_prefix(b"").await?;
    let mut watches = Vec::with_capacity(rows.len());
    for (_key, bytes) in rows {
        let record: FileWatchRecord =
            serde_json::from_slice(&bytes).map_err(|e| {
                StorageError::Redb(format!("deserialize FileWatchRecord: {e}"))
            })?;
        watches.push(record);
    }
    Ok(watches)
}

pub async fn delete_file_watch(
    handle: &DomainHandle,
    watch_id: &str,
) -> Result<(), StorageError> {
    assert_eq!(handle.domain(), KeyDomain::FileWatches);
    handle.delete(&watch_key(watch_id)).await
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
    fn file_watch_record_new_sets_defaults() {
        let r = FileWatchRecord::new(
            "fw-1".into(),
            "/tmp/data".into(),
            "default".into(),
            "process new data".into(),
        );
        assert_eq!(r.watch_id, "fw-1");
        assert_eq!(r.path, "/tmp/data");
        assert_eq!(r.role_name, "default");
        assert_eq!(r.prompt, "process new data");
        assert!(r.enabled);
        assert_eq!(r.debounce_ms, DEFAULT_DEBOUNCE_MS);
        assert!(r.last_fired_at.is_none());
    }

    #[test]
    fn file_watch_record_round_trips_through_serde() {
        let record = FileWatchRecord::new(
            "test-fw".into(),
            "/var/log/app".into(),
            "ops".into(),
            "check logs".into(),
        );
        let json = serde_json::to_vec(&record).unwrap();
        let back: FileWatchRecord = serde_json::from_slice(&json).unwrap();
        assert_eq!(back.watch_id, "test-fw");
        assert_eq!(back.path, "/var/log/app");
        assert_eq!(back.role_name, "ops");
        assert!(back.enabled);
    }

    #[test]
    fn should_fire_returns_true_when_never_fired() {
        let r = FileWatchRecord::new(
            "fw-1".into(),
            "/tmp".into(),
            "default".into(),
            "test".into(),
        );
        assert!(r.should_fire());
    }

    #[test]
    fn should_fire_returns_false_within_debounce_window() {
        let mut r = FileWatchRecord::new(
            "fw-1".into(),
            "/tmp".into(),
            "default".into(),
            "test".into(),
        );
        r.last_fired_at = Some(now_millis()); // just fired
        assert!(!r.should_fire());
    }

    #[test]
    fn should_fire_returns_true_after_debounce_window() {
        let mut r = FileWatchRecord::new(
            "fw-1".into(),
            "/tmp".into(),
            "default".into(),
            "test".into(),
        );
        // Fired well in the past
        r.last_fired_at = Some(now_millis().saturating_sub(10_000));
        assert!(r.should_fire());
    }

    #[test]
    fn watch_key_is_stable() {
        let k = watch_key("my-watch");
        assert_eq!(k, b"my-watch");
    }
}
