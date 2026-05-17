//! Phase 80 — the proactive-surfacing dedup log.
//!
//! One row per item the assistant has surfaced unprompted,
//! keyed by the item's deterministic id so the proactive pass
//! never re-surfaces the same thing across reflection cycles
//! (the assistant must not nag). Backed by
//! [`aivyx_storage::KeyDomain::ProactiveLog`], a dedicated
//! encrypted domain so a corrupt / GC'd row degrades only
//! proactive dedup — never memory or the recall signal.
//!
//! Key layout: the item id is the key (point-lookup dedup on
//! the hot path, every candidate every cycle); the value is the
//! big-endian surfaced-at timestamp (the GC clamp's only
//! input, no key parsing). Best-effort: a lost row at worst
//! re-surfaces one item once.

use aivyx_storage::DomainHandle;

#[derive(Debug, thiserror::Error)]
pub enum ProactiveLogError {
    #[error("proactive log storage error: {0}")]
    Storage(String),
}

/// Persistent dedup log over
/// [`aivyx_storage::KeyDomain::ProactiveLog`].
pub struct PersistentProactiveLog {
    storage: DomainHandle,
}

impl PersistentProactiveLog {
    pub fn new(storage: DomainHandle) -> Self {
        Self { storage }
    }

    /// Has this item id already been surfaced? Point lookup —
    /// O(1), called once per candidate per cycle.
    pub async fn was_surfaced(
        &self,
        id: &str,
    ) -> Result<bool, ProactiveLogError> {
        self.storage
            .get(id.as_bytes())
            .await
            .map(|v| v.is_some())
            .map_err(|e| ProactiveLogError::Storage(e.to_string()))
    }

    /// Record that `id` was surfaced at `ts_secs`. Idempotent
    /// on the id (re-marking just refreshes the timestamp).
    pub async fn mark_surfaced(
        &self,
        id: &str,
        ts_secs: u64,
    ) -> Result<(), ProactiveLogError> {
        self.storage
            .put(id.as_bytes(), &ts_secs.to_be_bytes())
            .await
            .map_err(|e| ProactiveLogError::Storage(e.to_string()))
    }

    /// Delete every dedup row older than `cutoff_secs`. Bounded-
    /// growth clamp run on the reflection cadence. A row older
    /// than the clamp window can re-surface — acceptable: the
    /// signal that produced it has long since changed.
    pub async fn gc_older_than(
        &self,
        cutoff_secs: u64,
    ) -> Result<usize, ProactiveLogError> {
        let rows = self
            .storage
            .scan_prefix(&[])
            .await
            .map_err(|e| ProactiveLogError::Storage(e.to_string()))?;
        let mut deleted = 0usize;
        for (k, v) in &rows {
            if v.len() < 8 {
                continue;
            }
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&v[..8]);
            if u64::from_be_bytes(buf) < cutoff_secs {
                self.storage.delete(k).await.map_err(|e| {
                    ProactiveLogError::Storage(e.to_string())
                })?;
                deleted += 1;
            }
        }
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_crypto::MasterKey;
    use aivyx_storage::{KeyDomain, RedbStorage, Storage, StorageConfig};
    use std::path::PathBuf;
    use std::sync::Arc;

    struct Scratch {
        dir: PathBuf,
    }
    impl Scratch {
        fn new() -> Self {
            let base = std::env::var("TMPDIR")
                .unwrap_or_else(|_| "/tmp".to_string());
            let dir = PathBuf::from(base).join(format!(
                "aivyx-proactive-log-test-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Scratch { dir }
        }
        fn store_path(&self) -> PathBuf {
            self.dir.join("store.redb")
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    async fn open_log(
        scratch: &Scratch,
        mb: u8,
    ) -> PersistentProactiveLog {
        let master = MasterKey::from_raw([mb; 32]);
        let storage: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(scratch.store_path()),
            master,
        )
        .await
        .expect("open store");
        PersistentProactiveLog::new(
            storage.domain(KeyDomain::ProactiveLog),
        )
    }

    #[tokio::test]
    async fn mark_then_was_surfaced_dedups() {
        let scratch = Scratch::new();
        let log = open_log(&scratch, 1).await;
        assert!(!log.was_surfaced("ttl:notes:7").await.unwrap());
        log.mark_surfaced("ttl:notes:7", 1000).await.unwrap();
        assert!(log.was_surfaced("ttl:notes:7").await.unwrap());
        // A different id is independent.
        assert!(!log.was_surfaced("due:notes:7").await.unwrap());
    }

    #[tokio::test]
    async fn gc_older_than_clamps_by_value_ts() {
        let scratch = Scratch::new();
        let log = open_log(&scratch, 2).await;
        log.mark_surfaced("old-a", 100).await.unwrap();
        log.mark_surfaced("old-b", 150).await.unwrap();
        log.mark_surfaced("fresh", 900).await.unwrap();

        let removed = log.gc_older_than(200).await.unwrap();
        assert_eq!(removed, 2);
        assert!(!log.was_surfaced("old-a").await.unwrap());
        assert!(!log.was_surfaced("old-b").await.unwrap());
        assert!(log.was_surfaced("fresh").await.unwrap());
    }

    #[tokio::test]
    async fn empty_log_reads_false_and_gc_zero() {
        let scratch = Scratch::new();
        let log = open_log(&scratch, 3).await;
        assert!(!log.was_surfaced("anything").await.unwrap());
        assert_eq!(log.gc_older_than(999).await.unwrap(), 0);
    }
}
