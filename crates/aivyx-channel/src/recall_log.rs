//! Phase 77 — the recall-feedback signal log.
//!
//! One row per turn that automatic recall (Phase 76) injected
//! memory into. The reflection loop reads a time window of
//! these and pairs them against the audit chain's `TurnEnded`
//! outcomes to learn which memories actually help (Q1a — a
//! structural, non-LLM signal).
//!
//! Backed by [`aivyx_storage::KeyDomain::RecallEvents`], a
//! dedicated encrypted domain so the signal can be GC-clamped
//! independently and a corrupt row degrades learning, never
//! recall or memory. This is **best-effort** infrastructure:
//! it is not a tamper-evident chain like the persona-proposal
//! log — a lost or skipped row just means the loop misses one
//! turn's worth of signal.

use serde::{Deserialize, Serialize};

use aivyx_core::SessionId;
use aivyx_storage::DomainHandle;

/// One memory that auto-recall injected into a turn, with the
/// cosine score it was ranked at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecallHit {
    pub topic: String,
    pub seq: u64,
    pub score: f32,
    /// Phase 84 — `true` iff this hit was injected by
    /// cluster-aware co-recall (a Phase 83 affined sibling the
    /// literal query missed), `false` for a primary
    /// keyword/semantic hit. `#[serde(default)]` keeps the
    /// recall-log + IPC round-trip back-compatible (older rows
    /// decode as `false`). Phase 77 `correlate_detailed` reads
    /// only `(topic, seq)` so it is unaffected; the Phase 83
    /// co-occurrence fold uses this to **exclude** cluster
    /// hits (the self-policing — the ledger never learns from
    /// its own expansion).
    #[serde(default)]
    pub cluster: bool,
}

/// The recall that happened on one turn. `ts_secs` is wall
/// clock at injection time; `session_id` ties it to the
/// conversation so the correlator can apply the
/// "operator-corrected-next-turn" heuristic per session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecallEvent {
    pub ts_secs: u64,
    pub session_id: SessionId,
    pub hits: Vec<RecallHit>,
}

#[derive(Debug, thiserror::Error)]
pub enum RecallLogError {
    #[error("recall log storage error: {0}")]
    Storage(String),
    #[error("recall log encode error: {0}")]
    Encode(String),
}

/// Persistent recall-event log over
/// [`aivyx_storage::KeyDomain::RecallEvents`].
///
/// Key layout: `ts_secs_be(8) || uuid(16)`. The big-endian
/// timestamp prefix makes a `scan_prefix(&[])` return rows in
/// time order; the UUID suffix makes every append
/// collision-free without a persisted counter (two recalls in
/// the same second still get distinct keys). Reads filter the
/// full scan by timestamp — the same bounded-scan discipline
/// `RedbMemory` uses, kept honest by the independent GC clamp.
pub struct PersistentRecallLog {
    storage: DomainHandle,
}

impl PersistentRecallLog {
    pub fn new(storage: DomainHandle) -> Self {
        Self { storage }
    }

    fn event_key(ts_secs: u64) -> Vec<u8> {
        let mut k = Vec::with_capacity(24);
        k.extend_from_slice(&ts_secs.to_be_bytes());
        k.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
        k
    }

    /// Append one recall event. Best-effort: callers treat a
    /// failure as "this turn's signal is lost," never fatal.
    pub async fn append(
        &self,
        event: &RecallEvent,
    ) -> Result<(), RecallLogError> {
        let key = Self::event_key(event.ts_secs);
        let value = serde_json::to_vec(event)
            .map_err(|e| RecallLogError::Encode(e.to_string()))?;
        self.storage
            .put(&key, &value)
            .await
            .map_err(|e| RecallLogError::Storage(e.to_string()))
    }

    /// Every event with `ts_secs >= since_secs`, ascending by
    /// timestamp. The reflection loop's lookback input.
    pub async fn events_since(
        &self,
        since_secs: u64,
    ) -> Result<Vec<RecallEvent>, RecallLogError> {
        let rows = self
            .storage
            .scan_prefix(&[])
            .await
            .map_err(|e| RecallLogError::Storage(e.to_string()))?;
        let mut out = Vec::new();
        for (_k, v) in &rows {
            let ev: RecallEvent = serde_json::from_slice(v)
                .map_err(|e| RecallLogError::Encode(e.to_string()))?;
            if ev.ts_secs >= since_secs {
                out.push(ev);
            }
        }
        out.sort_by_key(|e| e.ts_secs);
        Ok(out)
    }

    /// Delete every event older than `cutoff_secs`. Bounded-
    /// growth clamp, run on the same reflection cadence so the
    /// signal log can't grow without limit. Uses the key's
    /// timestamp prefix — no value decode needed. Returns the
    /// number of rows removed.
    pub async fn gc_older_than(
        &self,
        cutoff_secs: u64,
    ) -> Result<usize, RecallLogError> {
        let rows = self
            .storage
            .scan_prefix(&[])
            .await
            .map_err(|e| RecallLogError::Storage(e.to_string()))?;
        let mut deleted = 0usize;
        for (k, _v) in &rows {
            if k.len() < 8 {
                continue;
            }
            let mut buf = [0u8; 8];
            buf.copy_from_slice(&k[..8]);
            if u64::from_be_bytes(buf) < cutoff_secs {
                self.storage
                    .delete(k)
                    .await
                    .map_err(|e| RecallLogError::Storage(e.to_string()))?;
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
                "aivyx-recall-log-test-{}",
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

    async fn open_log(scratch: &Scratch, mb: u8) -> PersistentRecallLog {
        let master = MasterKey::from_raw([mb; 32]);
        let storage: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(scratch.store_path()),
            master,
        )
        .await
        .expect("open store");
        PersistentRecallLog::new(storage.domain(KeyDomain::RecallEvents))
    }

    fn ev(ts: u64, topic: &str, seq: u64, score: f32) -> RecallEvent {
        RecallEvent {
            ts_secs: ts,
            session_id: SessionId::new(),
            hits: vec![RecallHit {
                topic: topic.into(),
                seq,
                score,
                cluster: false,
            }],
        }
    }

    #[tokio::test]
    async fn append_then_events_since_round_trips_and_filters() {
        let scratch = Scratch::new();
        let log = open_log(&scratch, 1).await;
        log.append(&ev(100, "a", 0, 0.9)).await.unwrap();
        log.append(&ev(200, "b", 1, 0.8)).await.unwrap();
        log.append(&ev(300, "c", 2, 0.7)).await.unwrap();

        // Window cuts off everything before 200.
        let got = log.events_since(200).await.unwrap();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].ts_secs, 200);
        assert_eq!(got[1].ts_secs, 300);
        assert_eq!(got[0].hits[0].topic, "b");
        assert!((got[1].hits[0].score - 0.7).abs() < 1e-6);

        // since=0 returns all, ascending.
        let all = log.events_since(0).await.unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].ts_secs, 100);
    }

    #[tokio::test]
    async fn same_second_appends_do_not_collide() {
        let scratch = Scratch::new();
        let log = open_log(&scratch, 2).await;
        // Three recalls in the very same wall-clock second.
        log.append(&ev(500, "x", 0, 0.5)).await.unwrap();
        log.append(&ev(500, "y", 1, 0.6)).await.unwrap();
        log.append(&ev(500, "z", 2, 0.7)).await.unwrap();
        let all = log.events_since(0).await.unwrap();
        assert_eq!(all.len(), 3, "uuid suffix must keep keys distinct");
    }

    #[tokio::test]
    async fn gc_older_than_clamps_and_counts() {
        let scratch = Scratch::new();
        let log = open_log(&scratch, 3).await;
        log.append(&ev(100, "old", 0, 0.1)).await.unwrap();
        log.append(&ev(150, "old2", 1, 0.1)).await.unwrap();
        log.append(&ev(900, "fresh", 2, 0.9)).await.unwrap();

        let removed = log.gc_older_than(200).await.unwrap();
        assert_eq!(removed, 2);
        let left = log.events_since(0).await.unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].hits[0].topic, "fresh");
    }

    #[tokio::test]
    async fn empty_log_reads_empty_not_error() {
        let scratch = Scratch::new();
        let log = open_log(&scratch, 4).await;
        assert!(log.events_since(0).await.unwrap().is_empty());
        assert_eq!(log.gc_older_than(999).await.unwrap(), 0);
    }

    /// Phase 84 — the `cluster` marker round-trips, and a row
    /// written before Phase 84 (no `cluster` key) decodes as
    /// `false` via `#[serde(default)]` (back-compat).
    #[test]
    fn cluster_marker_round_trips_and_defaults_false() {
        let hit = RecallHit {
            topic: "deploy".into(),
            seq: 7,
            score: 0.9,
            cluster: true,
        };
        let j = serde_json::to_string(&hit).unwrap();
        let back: RecallHit =
            serde_json::from_str(&j).unwrap();
        assert_eq!(back, hit);
        assert!(back.cluster);

        // A pre-Phase-84 row has no `cluster` field at all.
        let legacy = r#"{"topic":"t","seq":1,"score":0.5}"#;
        let decoded: RecallHit =
            serde_json::from_str(legacy).unwrap();
        assert!(
            !decoded.cluster,
            "missing `cluster` must default to false"
        );
    }
}
