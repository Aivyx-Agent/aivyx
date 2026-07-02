//! Chapter Concord — the memory-contradiction dismissal set.
//!
//! On-demand contradiction detection re-derives conflicts every run, so a
//! false positive (two facts the operator knows are compatible) would be
//! re-flagged forever. When the operator says "keep both", we record the
//! conflict's deterministic id here; the detection pass then filters those
//! ids out. Backed by [`aivyx_storage::KeyDomain::ConflictDismissals`], a
//! dedicated encrypted domain so a corrupt row degrades only dismissal —
//! never memory or detection itself.
//!
//! Key layout: the [`aivyx_ipc::conflict::MemoryConflict`] id is the key
//! (point-lookup on the hot path); the value is the big-endian
//! dismissed-at timestamp. Best-effort: a lost row at worst re-flags one
//! already-dismissed pair once. The id is stable across re-detection (it
//! hashes both `(topic, seq)` pairs), so a dismissal keeps suppressing the
//! same pair until one of its entries actually changes.

use aivyx_storage::DomainHandle;

#[derive(Debug, thiserror::Error)]
pub enum ConflictDismissalError {
    #[error("conflict-dismissal storage error: {0}")]
    Storage(String),
}

/// Persistent set of dismissed conflict ids over
/// [`aivyx_storage::KeyDomain::ConflictDismissals`].
pub struct PersistentConflictDismissals {
    storage: DomainHandle,
}

impl PersistentConflictDismissals {
    pub fn new(storage: DomainHandle) -> Self {
        Self { storage }
    }

    /// Is this conflict id dismissed? Point lookup — O(1), called once
    /// per detected conflict per pass.
    pub async fn is_dismissed(
        &self,
        id: &str,
    ) -> Result<bool, ConflictDismissalError> {
        self.storage
            .get(id.as_bytes())
            .await
            .map(|v| v.is_some())
            .map_err(|e| ConflictDismissalError::Storage(e.to_string()))
    }

    /// Record that `id` was dismissed at `ts_secs`. Idempotent on the id.
    pub async fn dismiss(
        &self,
        id: &str,
        ts_secs: u64,
    ) -> Result<(), ConflictDismissalError> {
        self.storage
            .put(id.as_bytes(), &ts_secs.to_be_bytes())
            .await
            .map_err(|e| ConflictDismissalError::Storage(e.to_string()))
    }

    /// Filter a detected-conflict list down to those NOT dismissed.
    /// Best-effort: a storage error on any id keeps that conflict (better
    /// to over-surface than to silently swallow a real contradiction).
    pub async fn retain_undismissed(
        &self,
        conflicts: Vec<aivyx_ipc::conflict::MemoryConflict>,
    ) -> Vec<aivyx_ipc::conflict::MemoryConflict> {
        let mut kept = Vec::with_capacity(conflicts.len());
        for c in conflicts {
            if !self.is_dismissed(&c.id).await.unwrap_or(false) {
                kept.push(c);
            }
        }
        kept
    }

    // ---- Chapter Accord — Soul-conflict dismissals -----------------------
    //
    // Reuses this same domain (soul ids are namespace-distinct FNV hashes),
    // but namespaces the key with `soul:` so a memory and a Soul id can never
    // collide within the shared store.

    /// Record that a Soul-conflict `id` was dismissed at `ts_secs`.
    pub async fn dismiss_soul(
        &self,
        id: &str,
        ts_secs: u64,
    ) -> Result<(), ConflictDismissalError> {
        self.dismiss(&format!("soul:{id}"), ts_secs).await
    }

    /// Filter a detected Soul-conflict list down to those NOT dismissed.
    pub async fn retain_undismissed_soul(
        &self,
        conflicts: Vec<aivyx_ipc::soul_conflict::SoulConflict>,
    ) -> Vec<aivyx_ipc::soul_conflict::SoulConflict> {
        let mut kept = Vec::with_capacity(conflicts.len());
        for c in conflicts {
            if !self.is_dismissed(&format!("soul:{}", c.id)).await.unwrap_or(false) {
                kept.push(c);
            }
        }
        kept
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_crypto::MasterKey;
    use aivyx_ipc::conflict::{ConflictSide, MemoryConflict};
    use aivyx_storage::{KeyDomain, RedbStorage, Storage, StorageConfig};
    use std::path::PathBuf;
    use std::sync::Arc;

    struct Scratch {
        dir: PathBuf,
    }
    impl Scratch {
        fn new() -> Self {
            let tmp = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
            let dir = PathBuf::from(tmp)
                .join(format!("aivyx-dismiss-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            Scratch { dir }
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    async fn open(scratch: &Scratch) -> PersistentConflictDismissals {
        let storage: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(scratch.dir.join("s.redb")),
            MasterKey::from_raw([3u8; 32]),
        )
        .await
        .unwrap();
        PersistentConflictDismissals::new(
            storage.domain(KeyDomain::ConflictDismissals),
        )
    }

    fn conflict(id: &str) -> MemoryConflict {
        MemoryConflict {
            id: id.into(),
            a: ConflictSide {
                topic: "t".into(),
                seq: 1,
                body: "a".into(),
                created_at_secs: 1,
            },
            b: ConflictSide {
                topic: "t".into(),
                seq: 2,
                body: "b".into(),
                created_at_secs: 2,
            },
            reason: "r".into(),
        }
    }

    #[tokio::test]
    async fn dismiss_then_filtered_out_and_durable() {
        let scratch = Scratch::new();
        let store = open(&scratch).await;

        assert!(!store.is_dismissed("abc").await.unwrap());
        store.dismiss("abc", 100).await.unwrap();
        assert!(store.is_dismissed("abc").await.unwrap());

        let kept = store
            .retain_undismissed(vec![conflict("abc"), conflict("xyz")])
            .await;
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].id, "xyz", "dismissed id filtered, other kept");
    }

    fn soul_conflict(id: &str) -> aivyx_ipc::soul_conflict::SoulConflict {
        use aivyx_ipc::soul_conflict::{SoulConflict, SoulFacet};
        SoulConflict {
            id: id.into(),
            a: SoulFacet { category: "character_traits".into(), value: "x".into() },
            b: SoulFacet { category: "character_traits".into(), value: "y".into() },
            reason: "r".into(),
            cross_layer: false,
        }
    }

    #[tokio::test]
    async fn soul_dismiss_is_namespaced_and_filtered() {
        let scratch = Scratch::new();
        let store = open(&scratch).await;

        // Dismiss a Soul conflict id.
        store.dismiss_soul("s1", 100).await.unwrap();
        let kept = store
            .retain_undismissed_soul(vec![soul_conflict("s1"), soul_conflict("s2")])
            .await;
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].id, "s2", "dismissed soul id filtered, other kept");

        // Namespacing: the SAME id dismissed on the memory side must NOT
        // suppress the soul conflict (soul uses the `soul:` prefix).
        let store2_scratch = Scratch::new();
        let store2 = open(&store2_scratch).await;
        store2.dismiss("s1", 100).await.unwrap(); // memory-side dismiss of "s1"
        let kept2 = store2.retain_undismissed_soul(vec![soul_conflict("s1")]).await;
        assert_eq!(kept2.len(), 1, "memory dismiss of same id must not hide the soul conflict");
    }
}
