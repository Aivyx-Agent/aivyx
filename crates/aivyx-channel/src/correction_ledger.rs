//! Phase 172 — the persistent, longitudinal correction ledger.
//!
//! Symmetric with the Phase 82 [`crate::helpfulness_ledger`].
//! Phase 172's [`crate::correction_detect`] recomputes a
//! per-topic [`CorrectionTally`] each reflection window and
//! throws it away; this makes it **durable**. One row per
//! memory topic holds a time-decayed EWMA of the windowed
//! correction count, folded on the existing reflection cadence,
//! backed by the HKDF-isolated
//! [`aivyx_storage::KeyDomain::CorrectionLedger`] so a corrupt /
//! pruned row degrades only the self-improvement signal — never
//! memory, recall, or the helpfulness / co-occurrence ledgers.
//!
//! **Recency-weighted.** Each fold first *decays* the stored
//! EWMA toward zero by `0.5^(dt / half_life)`, then adds the
//! new window's correction count. A topic the operator used to
//! rework but no longer does fades on its own — the
//! self-improving / things-change posture — and that decay
//! makes bounded pruning trivial.
//!
//! **Why a shorter half-life than helpfulness.** Helpfulness
//! uses a ~60-day half-life: a stable *preference* should
//! persist. A correction signal is a *transient annoyance*
//! signal — "you keep reworking this lately." It should fade
//! faster so a problem the operator has since stopped hitting
//! doesn't keep generating proposals. ~30 days.
//!
//! **Zero-config.** Like the helpfulness ledger and the Phase
//! 77 recall log: a passive internal signal, auto-initialised,
//! no `[…]` block, no behaviour change on its own. The
//! proposal-filing pass (Phase 172 Task 4) is the opt-in piece;
//! the ledger itself just accumulates.
//!
//! [`CorrectionTally`]: crate::correction_detect::CorrectionTally

use serde::{Deserialize, Serialize};

use aivyx_storage::DomainHandle;

/// EWMA half-life: the elapsed time over which an untouched
/// topic's accumulated correction pressure halves. ~30 days —
/// half the helpfulness half-life, because a correction signal
/// is a transient "lately" signal, not a durable preference.
pub const CORRECTION_HALF_LIFE_SECS: u64 = 30 * 24 * 3600;

/// A row whose decayed-to-now magnitude is below this is
/// "effectively zero" — eligible for pruning once it has also
/// gone untouched past the horizon.
pub const CORRECTION_PRUNE_EPSILON: f32 = 0.05;

/// A sub-epsilon row is only pruned once it has *also* not been
/// folded into for this long (~60 days). The two-part guard
/// keeps a briefly-quiet-but-recent topic alive while reaping
/// genuinely dead ones.
pub const CORRECTION_PRUNE_HORIZON_SECS: u64 = 60 * 24 * 3600;

#[derive(Debug, thiserror::Error)]
pub enum CorrectionLedgerError {
    #[error("correction ledger storage error: {0}")]
    Storage(String),
    #[error("correction ledger encode error: {0}")]
    Encode(String),
}

/// One topic's durable correction state. `ewma_count` is the
/// stored (un-decayed-since-`last_update_secs`) value; callers
/// that read it receive a copy already decayed to "now".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LedgerEntry {
    /// Time-decayed exponentially-weighted correction count.
    pub ewma_count: f32,
    /// How many reflection windows have folded into this topic
    /// — a confidence proxy (one window of corrections is not a
    /// pattern).
    pub samples: u32,
    /// When this row was last folded into. Drives both the
    /// read-time decay and the prune horizon.
    pub last_update_secs: u64,
}

/// Decay `count` from `last_update` forward to `now` by the
/// half-life. `dt == 0` → unchanged; far future → toward 0.
fn decayed(count: f32, last_update: u64, now: u64) -> f32 {
    let dt = now.saturating_sub(last_update);
    if dt == 0 {
        return count;
    }
    let factor =
        0.5_f32.powf(dt as f32 / CORRECTION_HALF_LIFE_SECS as f32);
    count * factor
}

/// One topic's decayed accumulated correction pressure, for the
/// Phase 78 longitudinal surface. `samples` is the confidence
/// proxy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopicCorrections {
    pub topic: String,
    pub count: f32,
    pub samples: u32,
}

/// The durable, decayed most-corrected-per-topic view for the
/// Phase 78 learning surface.
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize,
)]
pub struct AccumulatedCorrections {
    pub top_corrected: Vec<TopicCorrections>,
}

/// Persistent correction ledger over
/// [`aivyx_storage::KeyDomain::CorrectionLedger`]. Key = topic
/// bytes; value = JSON [`LedgerEntry`].
pub struct PersistentCorrectionLedger {
    storage: DomainHandle,
}

impl PersistentCorrectionLedger {
    pub fn new(storage: DomainHandle) -> Self {
        Self { storage }
    }

    async fn get_entry(
        &self,
        topic: &str,
    ) -> Result<Option<LedgerEntry>, CorrectionLedgerError> {
        let raw = self
            .storage
            .get(topic.as_bytes())
            .await
            .map_err(|e| {
                CorrectionLedgerError::Storage(e.to_string())
            })?;
        match raw {
            Some(bytes) => {
                let e: LedgerEntry = serde_json::from_slice(&bytes)
                    .map_err(|e| {
                        CorrectionLedgerError::Encode(e.to_string())
                    })?;
                Ok(Some(e))
            }
            None => Ok(None),
        }
    }

    /// Fold one reflection window's per-topic correction counts
    /// into the durable ledger: for each topic, first decay the
    /// stored EWMA to `now`, then add this window's `count`,
    /// bump `samples`, and stamp `last_update`. A topic not seen
    /// before starts at this window's count with one sample.
    pub async fn record_window(
        &self,
        count_by_topic: &[(String, f32)],
        now_secs: u64,
    ) -> Result<(), CorrectionLedgerError> {
        for (topic, count) in count_by_topic {
            let entry = match self.get_entry(topic).await? {
                Some(prev) => LedgerEntry {
                    ewma_count: decayed(
                        prev.ewma_count,
                        prev.last_update_secs,
                        now_secs,
                    ) + count,
                    samples: prev.samples.saturating_add(1),
                    last_update_secs: now_secs,
                },
                None => LedgerEntry {
                    ewma_count: *count,
                    samples: 1,
                    last_update_secs: now_secs,
                },
            };
            let bytes = serde_json::to_vec(&entry).map_err(|e| {
                CorrectionLedgerError::Encode(e.to_string())
            })?;
            self.storage
                .put(topic.as_bytes(), &bytes)
                .await
                .map_err(|e| {
                    CorrectionLedgerError::Storage(e.to_string())
                })?;
        }
        Ok(())
    }

    /// Point lookup, decayed to `now` (so callers see the
    /// current value without forcing a write).
    pub async fn topic_corrections(
        &self,
        topic: &str,
        now_secs: u64,
    ) -> Result<Option<LedgerEntry>, CorrectionLedgerError> {
        Ok(self.get_entry(topic).await?.map(|e| LedgerEntry {
            ewma_count: decayed(
                e.ewma_count,
                e.last_update_secs,
                now_secs,
            ),
            samples: e.samples,
            last_update_secs: e.last_update_secs,
        }))
    }

    /// Every topic, decayed to `now`, sorted by count descending
    /// (then topic, for deterministic output). For the Phase 78
    /// longitudinal surface and the Phase 172 consolidation
    /// selector.
    pub async fn ranked(
        &self,
        now_secs: u64,
    ) -> Result<Vec<(String, LedgerEntry)>, CorrectionLedgerError>
    {
        let rows = self
            .storage
            .scan_prefix(&[])
            .await
            .map_err(|e| {
                CorrectionLedgerError::Storage(e.to_string())
            })?;
        let mut out: Vec<(String, LedgerEntry)> =
            Vec::with_capacity(rows.len());
        for (k, v) in &rows {
            let topic = String::from_utf8_lossy(k).into_owned();
            let e: LedgerEntry =
                serde_json::from_slice(v).map_err(|e| {
                    CorrectionLedgerError::Encode(e.to_string())
                })?;
            out.push((
                topic,
                LedgerEntry {
                    ewma_count: decayed(
                        e.ewma_count,
                        e.last_update_secs,
                        now_secs,
                    ),
                    samples: e.samples,
                    last_update_secs: e.last_update_secs,
                },
            ));
        }
        out.sort_by(|a, b| {
            b.1.ewma_count
                .partial_cmp(&a.1.ewma_count)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        Ok(out)
    }

    /// The decayed accumulated most-corrected-per-topic view (≤
    /// `top_n`) for the Phase 78 surface. Only topics with a
    /// positive decayed count appear; zero-count topics are
    /// omitted.
    pub async fn accumulated(
        &self,
        now_secs: u64,
        top_n: usize,
    ) -> Result<AccumulatedCorrections, CorrectionLedgerError> {
        let ranked = self.ranked(now_secs).await?;
        let top_corrected: Vec<TopicCorrections> = ranked
            .iter()
            .filter(|(_, e)| e.ewma_count > 0.0)
            .take(top_n)
            .map(|(t, e)| TopicCorrections {
                topic: t.clone(),
                count: e.ewma_count,
                samples: e.samples,
            })
            .collect();
        Ok(AccumulatedCorrections { top_corrected })
    }

    /// Drop rows whose decayed-to-`now` magnitude is below
    /// [`CORRECTION_PRUNE_EPSILON`] **and** which have not been
    /// folded into for [`CORRECTION_PRUNE_HORIZON_SECS`]. Run on
    /// the reflection cadence; returns the prune count.
    pub async fn prune(
        &self,
        now_secs: u64,
    ) -> Result<usize, CorrectionLedgerError> {
        let rows = self
            .storage
            .scan_prefix(&[])
            .await
            .map_err(|e| {
                CorrectionLedgerError::Storage(e.to_string())
            })?;
        let mut pruned = 0usize;
        for (k, v) in &rows {
            let Ok(e) = serde_json::from_slice::<LedgerEntry>(v)
            else {
                continue;
            };
            let mag =
                decayed(e.ewma_count, e.last_update_secs, now_secs)
                    .abs();
            let stale = now_secs.saturating_sub(e.last_update_secs)
                > CORRECTION_PRUNE_HORIZON_SECS;
            if mag < CORRECTION_PRUNE_EPSILON && stale {
                self.storage.delete(k).await.map_err(|e| {
                    CorrectionLedgerError::Storage(e.to_string())
                })?;
                pruned += 1;
            }
        }
        Ok(pruned)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_crypto::MasterKey;
    use aivyx_storage::{
        KeyDomain, RedbStorage, Storage, StorageConfig,
    };
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
                "aivyx-correction-ledger-test-{}",
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

    async fn open_ledger(
        scratch: &Scratch,
        mb: u8,
    ) -> PersistentCorrectionLedger {
        let master = MasterKey::from_raw([mb; 32]);
        let storage: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(scratch.store_path()),
            master,
        )
        .await
        .expect("open store");
        PersistentCorrectionLedger::new(
            storage.domain(KeyDomain::CorrectionLedger),
        )
    }

    #[tokio::test]
    async fn empty_ledger_reads_none_and_prunes_zero() {
        let s = Scratch::new();
        let l = open_ledger(&s, 1).await;
        assert!(l
            .topic_corrections("anything", 100)
            .await
            .unwrap()
            .is_none());
        assert_eq!(l.prune(100).await.unwrap(), 0);
        assert!(l.ranked(100).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn first_window_seeds_then_accumulates() {
        let s = Scratch::new();
        let l = open_ledger(&s, 2).await;
        l.record_window(&[("auth".into(), 2.0)], 1_000)
            .await
            .unwrap();
        let e = l
            .topic_corrections("auth", 1_000)
            .await
            .unwrap()
            .expect("seeded");
        assert!((e.ewma_count - 2.0).abs() < 1e-4);
        assert_eq!(e.samples, 1);

        // Same instant (no decay) → straight add, samples bump.
        l.record_window(&[("auth".into(), 3.0)], 1_000)
            .await
            .unwrap();
        let e = l
            .topic_corrections("auth", 1_000)
            .await
            .unwrap()
            .unwrap();
        assert!((e.ewma_count - 5.0).abs() < 1e-4);
        assert_eq!(e.samples, 2);
    }

    #[tokio::test]
    async fn stored_count_decays_over_time() {
        let s = Scratch::new();
        let l = open_ledger(&s, 3).await;
        l.record_window(&[("k".into(), 8.0)], 0).await.unwrap();
        // Exactly one half-life later → ~half.
        let half = l
            .topic_corrections("k", CORRECTION_HALF_LIFE_SECS)
            .await
            .unwrap()
            .unwrap();
        assert!(
            (half.ewma_count - 4.0).abs() < 1e-2,
            "got {}",
            half.ewma_count
        );
    }

    #[tokio::test]
    async fn ranked_is_count_desc_then_topic() {
        let s = Scratch::new();
        let l = open_ledger(&s, 4).await;
        l.record_window(
            &[
                ("low".into(), 1.0),
                ("high".into(), 9.0),
                ("mid".into(), 5.0),
            ],
            10,
        )
        .await
        .unwrap();
        let r = l.ranked(10).await.unwrap();
        let order: Vec<&str> =
            r.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(order, vec!["high", "mid", "low"]);
    }

    #[tokio::test]
    async fn accumulated_lists_top_corrected() {
        let s = Scratch::new();
        let l = open_ledger(&s, 6).await;
        l.record_window(
            &[
                ("auth".into(), 7.0),
                ("db".into(), 3.0),
                ("ui".into(), 0.0),
            ],
            50,
        )
        .await
        .unwrap();
        let a = l.accumulated(50, 10).await.unwrap();
        let topics: Vec<&str> = a
            .top_corrected
            .iter()
            .map(|t| t.topic.as_str())
            .collect();
        // Zero-count `ui` omitted; ordered by count desc.
        assert_eq!(topics, vec!["auth", "db"]);
        assert_eq!(a.top_corrected[0].samples, 1);
        // top_n caps the list.
        let capped = l.accumulated(50, 1).await.unwrap();
        assert_eq!(capped.top_corrected.len(), 1);
        assert_eq!(capped.top_corrected[0].topic, "auth");
    }

    #[tokio::test]
    async fn prune_drops_only_decayed_and_stale() {
        let s = Scratch::new();
        let l = open_ledger(&s, 5).await;
        // A tiny, ancient row → decays sub-epsilon AND stale.
        l.record_window(&[("dead".into(), 0.10)], 0)
            .await
            .unwrap();
        // A strong, ancient row → stale but NOT sub-epsilon.
        l.record_window(&[("strong".into(), 5_000.0)], 0)
            .await
            .unwrap();
        let now = CORRECTION_PRUNE_HORIZON_SECS + 1;
        let pruned = l.prune(now).await.unwrap();
        assert_eq!(pruned, 1);
        assert!(l
            .topic_corrections("dead", now)
            .await
            .unwrap()
            .is_none());
        assert!(l
            .topic_corrections("strong", now)
            .await
            .unwrap()
            .is_some());

        // A fresh tiny row is sub-epsilon but NOT stale → kept.
        l.record_window(&[("freshtiny".into(), 0.01)], now)
            .await
            .unwrap();
        assert_eq!(l.prune(now).await.unwrap(), 0);
        assert!(l
            .topic_corrections("freshtiny", now)
            .await
            .unwrap()
            .is_some());
    }
}
