//! Lightweight budget-entry storage for the
//! `budget.*` tools.
//!
//! Phase 143. JSON file at
//! `~/.aivyx/tool-processes/toolkit/budget.json`
//! (0600 perms, atomic write-then-rename — same
//! pattern as Phase 125's task_store).
//!
//! ## Concurrency model
//!
//! `BudgetStore` holds the loaded entry list
//! behind a [`tokio::sync::Mutex`]. The harness
//! can dispatch multiple `budget.*` tools in
//! parallel; each operation takes the lock,
//! mutates in-memory state, persists to disk,
//! drops the lock. Operations are short
//! (microseconds + one fsync) so contention is
//! acceptable.
//!
//! ## On-disk format
//!
//! ```json
//! {
//!   "schema_version": 1,
//!   "entries": [
//!     {
//!       "id": "uuid-v4",
//!       "amount": 12.50,
//!       "category": "food",
//!       "note": "lunch" (or null),
//!       "recorded_at": "2026-06-03T12:30:00Z"
//!     }
//!   ]
//! }
//! ```
//!
//! `schema_version` lets a future phase extend
//! the shape without breaking existing operator
//! installs (same posture as task_store and
//! Gmail's token file).

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::fs;
use tokio::sync::Mutex;
use uuid::Uuid;

const CURRENT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum BudgetStoreError {
    #[error("budget storage I/O failed at {path:?}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("budget storage parse failed at {path:?}: {reason}")]
    Parse { path: PathBuf, reason: String },
    #[error("budget storage at {path:?} has schema_version {found} but this build only understands up to {supported}")]
    SchemaTooNew {
        path: PathBuf,
        found: u32,
        supported: u32,
    },
    #[error("`amount` value {0} is not finite — must be a regular number")]
    BadAmount(f64),
    #[error("`category` must not be empty")]
    EmptyCategory,
}

/// A single budget entry. Operator records an
/// amount (positive = expense, negative = income/
/// refund), a category (free-text), and an
/// optional note. The id + recorded_at are
/// assigned by the store at record time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetEntry {
    pub id: String,
    pub amount: f64,
    pub category: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub recorded_at: DateTime<Utc>,
}

/// On-disk wrapper around the entry list,
/// versioned at the file level for forward-compat
/// (same shape as task_store::StoredTasks).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredEntries {
    schema_version: u32,
    #[serde(default)]
    entries: Vec<BudgetEntry>,
}

/// The budget store. Holds the loaded entry list
/// under a Mutex; one instance per tool process
/// (constructed once in main and shared via
/// `Arc<BudgetStore>` to both `budget.*` tool
/// impls).
#[derive(Debug)]
pub struct BudgetStore {
    path: PathBuf,
    entries: Mutex<Vec<BudgetEntry>>,
}

impl BudgetStore {
    /// Open the store at `path`. Loads the file
    /// if it exists; initializes with an empty
    /// list if not. Either way the store is
    /// ready for reads and writes.
    pub async fn open(path: PathBuf) -> Result<Self, BudgetStoreError> {
        let entries = match fs::read_to_string(&path).await {
            Ok(body) => {
                let stored: StoredEntries = serde_json::from_str(&body).map_err(|e| {
                    BudgetStoreError::Parse {
                        path: path.clone(),
                        reason: e.to_string(),
                    }
                })?;
                if stored.schema_version > CURRENT_SCHEMA_VERSION {
                    return Err(BudgetStoreError::SchemaTooNew {
                        path,
                        found: stored.schema_version,
                        supported: CURRENT_SCHEMA_VERSION,
                    });
                }
                stored.entries
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => {
                return Err(BudgetStoreError::Io { path, source: e });
            }
        };
        Ok(Self {
            path,
            entries: Mutex::new(entries),
        })
    }

    /// Append a new entry with a fresh UUID v4
    /// id and `recorded_at` = now. Persists to
    /// disk before returning. Validates that
    /// `amount` is finite (NaN / infinity
    /// rejected) and `category` is non-empty
    /// after trimming.
    pub async fn record(
        &self,
        amount: f64,
        category: String,
        note: Option<String>,
    ) -> Result<BudgetEntry, BudgetStoreError> {
        if !amount.is_finite() {
            return Err(BudgetStoreError::BadAmount(amount));
        }
        let category = category.trim().to_string();
        if category.is_empty() {
            return Err(BudgetStoreError::EmptyCategory);
        }
        let entry = BudgetEntry {
            id: Uuid::new_v4().to_string(),
            amount,
            category,
            note: note.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()),
            recorded_at: Utc::now(),
        };
        let mut guard = self.entries.lock().await;
        guard.push(entry.clone());
        save_to_disk(&self.path, &guard).await?;
        Ok(entry)
    }

    /// Aggregate entries within `[since, until)`
    /// (inclusive lower, exclusive upper) into a
    /// `BudgetSummary`. Pure read; the lock is
    /// released after the snapshot copy so
    /// concurrent writes aren't blocked during
    /// aggregation.
    pub async fn summary(
        &self,
        since: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> BudgetSummary {
        let snapshot: Vec<BudgetEntry> = {
            let guard = self.entries.lock().await;
            guard
                .iter()
                .filter(|e| e.recorded_at >= since && e.recorded_at < until)
                .cloned()
                .collect()
        };
        aggregate(&snapshot, since, until)
    }
}

/// Aggregated view of a slice of entries within
/// `[since, until)`. Pure substrate so the
/// aggregation itself can be tested without
/// touching the store.
#[derive(Debug, Clone, Serialize)]
pub struct BudgetSummary {
    pub since: DateTime<Utc>,
    pub until: DateTime<Utc>,
    pub total: f64,
    pub entry_count: usize,
    /// One bucket per category present in the
    /// window. Sorted descending by `total`; ties
    /// broken alphabetically on `category`.
    pub by_category: Vec<CategoryTotal>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CategoryTotal {
    pub category: String,
    pub total: f64,
    pub count: usize,
}

fn aggregate(
    entries: &[BudgetEntry],
    since: DateTime<Utc>,
    until: DateTime<Utc>,
) -> BudgetSummary {
    let mut by_cat: HashMap<String, (f64, usize)> = HashMap::new();
    let mut total: f64 = 0.0;
    for e in entries {
        total += e.amount;
        let bucket = by_cat.entry(e.category.clone()).or_insert((0.0, 0));
        bucket.0 += e.amount;
        bucket.1 += 1;
    }
    let mut by_category: Vec<CategoryTotal> = by_cat
        .into_iter()
        .map(|(category, (total, count))| CategoryTotal {
            category,
            total,
            count,
        })
        .collect();
    by_category.sort_by(|a, b| {
        b.total
            .partial_cmp(&a.total)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.category.cmp(&b.category))
    });
    BudgetSummary {
        since,
        until,
        total,
        entry_count: entries.len(),
        by_category,
    }
}

// ---------- on-disk plumbing (mirrors task_store) -----------

async fn save_to_disk(
    path: &Path,
    entries: &[BudgetEntry],
) -> Result<(), BudgetStoreError> {
    let stored = StoredEntries {
        schema_version: CURRENT_SCHEMA_VERSION,
        entries: entries.to_vec(),
    };
    let body =
        serde_json::to_string_pretty(&stored).map_err(|e| BudgetStoreError::Parse {
            path: path.to_path_buf(),
            reason: format!("serialize: {e}"),
        })?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            create_dir_all_secure(parent)
                .await
                .map_err(|source| BudgetStoreError::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
        }
    }
    let tmp_path = with_tmp_suffix(path);
    write_secure(&tmp_path, body.as_bytes())
        .await
        .map_err(|source| BudgetStoreError::Io {
            path: tmp_path.clone(),
            source,
        })?;
    fs::rename(&tmp_path, path)
        .await
        .map_err(|source| BudgetStoreError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    Ok(())
}

fn with_tmp_suffix(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".tmp");
    PathBuf::from(s)
}

async fn create_dir_all_secure(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o700);
        fs::set_permissions(dir, perms).await?;
    }
    Ok(())
}

async fn write_secure(path: &Path, body: &[u8]) -> io::Result<()> {
    fs::write(path, body).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        fs::set_permissions(path, perms).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn scratch_dir() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let tmp = std::env::var("TMPDIR")
            .or_else(|_| std::env::var("TEMP"))
            .unwrap_or_else(|_| "/tmp".to_string());
        let dir = PathBuf::from(tmp).join(format!(
            "aivyx-toolkit-budget-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn entry(amount: f64, category: &str, recorded_at: &str) -> BudgetEntry {
        BudgetEntry {
            id: Uuid::new_v4().to_string(),
            amount,
            category: category.to_string(),
            note: None,
            recorded_at: ts(recorded_at),
        }
    }

    #[test]
    fn aggregate_sums_totals_and_counts_by_category() {
        let entries = vec![
            entry(12.50, "food", "2026-06-01T08:00:00Z"),
            entry(4.75, "food", "2026-06-01T12:00:00Z"),
            entry(20.00, "transport", "2026-06-01T18:00:00Z"),
        ];
        let s = aggregate(
            &entries,
            ts("2026-06-01T00:00:00Z"),
            ts("2026-06-02T00:00:00Z"),
        );
        assert_eq!(s.entry_count, 3);
        assert!((s.total - 37.25).abs() < 1e-9);
        assert_eq!(s.by_category.len(), 2);
        // Sorted desc by total: transport (20.00),
        // food (17.25).
        assert_eq!(s.by_category[0].category, "transport");
        assert!((s.by_category[0].total - 20.00).abs() < 1e-9);
        assert_eq!(s.by_category[0].count, 1);
        assert_eq!(s.by_category[1].category, "food");
        assert!((s.by_category[1].total - 17.25).abs() < 1e-9);
        assert_eq!(s.by_category[1].count, 2);
    }

    #[test]
    fn aggregate_ties_broken_alphabetically() {
        let entries = vec![
            entry(10.0, "transport", "2026-06-01T00:00:00Z"),
            entry(10.0, "food", "2026-06-01T00:00:00Z"),
        ];
        let s = aggregate(
            &entries,
            ts("2026-06-01T00:00:00Z"),
            ts("2026-06-02T00:00:00Z"),
        );
        assert_eq!(s.by_category[0].category, "food");
        assert_eq!(s.by_category[1].category, "transport");
    }

    #[test]
    fn aggregate_empty_input_yields_empty_summary() {
        let s = aggregate(
            &[],
            ts("2026-06-01T00:00:00Z"),
            ts("2026-06-02T00:00:00Z"),
        );
        assert_eq!(s.entry_count, 0);
        assert_eq!(s.total, 0.0);
        assert!(s.by_category.is_empty());
    }

    #[tokio::test]
    async fn record_then_summary_round_trip() {
        let dir = scratch_dir();
        let path = dir.join("budget.json");
        let store = BudgetStore::open(path.clone()).await.unwrap();
        let now_before = Utc::now();
        let e1 = store
            .record(12.50, "food".to_string(), Some("lunch".to_string()))
            .await
            .unwrap();
        let e2 = store
            .record(20.00, "transport".to_string(), None)
            .await
            .unwrap();
        let now_after = Utc::now();

        let s = store
            .summary(
                now_before - chrono::Duration::seconds(1),
                now_after + chrono::Duration::seconds(1),
            )
            .await;
        assert_eq!(s.entry_count, 2);
        assert!((s.total - 32.50).abs() < 1e-9);
        // Both entries persisted with ids.
        assert!(!e1.id.is_empty());
        assert!(!e2.id.is_empty());
        assert_eq!(e1.note, Some("lunch".to_string()));
        assert_eq!(e2.note, None);
    }

    #[tokio::test]
    async fn record_persists_across_reopen() {
        let dir = scratch_dir();
        let path = dir.join("budget.json");
        {
            let store = BudgetStore::open(path.clone()).await.unwrap();
            store
                .record(5.00, "snacks".to_string(), None)
                .await
                .unwrap();
        }
        // Re-open and verify.
        let store = BudgetStore::open(path.clone()).await.unwrap();
        let s = store
            .summary(ts("2020-01-01T00:00:00Z"), ts("2030-01-01T00:00:00Z"))
            .await;
        assert_eq!(s.entry_count, 1);
        assert!((s.total - 5.00).abs() < 1e-9);
    }

    #[tokio::test]
    async fn record_rejects_non_finite_amount() {
        let dir = scratch_dir();
        let path = dir.join("budget.json");
        let store = BudgetStore::open(path).await.unwrap();
        let err = store
            .record(f64::NAN, "food".to_string(), None)
            .await
            .unwrap_err();
        assert!(matches!(err, BudgetStoreError::BadAmount(_)));
    }

    #[tokio::test]
    async fn record_rejects_blank_category() {
        let dir = scratch_dir();
        let path = dir.join("budget.json");
        let store = BudgetStore::open(path).await.unwrap();
        let err = store
            .record(5.00, "   ".to_string(), None)
            .await
            .unwrap_err();
        assert!(matches!(err, BudgetStoreError::EmptyCategory));
    }

    #[tokio::test]
    async fn summary_window_excludes_entries_outside_range() {
        let dir = scratch_dir();
        let path = dir.join("budget.json");
        let store = BudgetStore::open(path).await.unwrap();
        store
            .record(10.00, "food".to_string(), None)
            .await
            .unwrap();
        // since/until = empty window before now → no
        // entries in range.
        let s = store
            .summary(ts("2020-01-01T00:00:00Z"), ts("2020-01-02T00:00:00Z"))
            .await;
        assert_eq!(s.entry_count, 0);
        assert_eq!(s.total, 0.0);
    }

    #[tokio::test]
    async fn open_rejects_future_schema_version() {
        let dir = scratch_dir();
        let path = dir.join("budget.json");
        fs::write(
            &path,
            r#"{"schema_version": 99, "entries": []}"#,
        )
        .await
        .unwrap();
        let err = BudgetStore::open(path).await.unwrap_err();
        assert!(matches!(err, BudgetStoreError::SchemaTooNew { .. }));
    }
}
