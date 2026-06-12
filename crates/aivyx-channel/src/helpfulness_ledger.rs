//! Phase 82 — the persistent, longitudinal helpfulness ledger.
//!
//! For 81 phases the "did recalling this topic help" signal
//! has been ephemeral: Phase 77 recomputes a `HelpfulnessTally`
//! each reflection window and throws it away. This makes it
//! **durable**. One row per memory topic holds a time-decayed
//! EWMA of the windowed net helpfulness, folded in on the
//! existing reflection cadence (Task 3), backed by the
//! HKDF-isolated [`aivyx_storage::KeyDomain::HelpfulnessLedger`]
//! so a corrupt/pruned row degrades only the longitudinal view
//! — never memory, recall, or proactive dedup.
//!
//! **Recency-weighted (Q1a).** Each fold first *decays* the
//! stored EWMA toward zero by `0.5^(dt / half_life)`, then adds
//! the new window's net. A topic that used to help but stopped
//! fades on its own — the self-improving / things-change
//! posture — and that same decay makes bounded pruning trivial
//! (a row whose decayed magnitude is sub-epsilon and untouched
//! past a horizon is dropped).
//!
//! **Zero-config (Q4a).** Like `RecallEvents` and Phase 77
//! itself: a passive internal signal, auto-initialised, no
//! `[…]` block, no behaviour change on its own. The half-life
//! and prune bounds are code constants (tuning deferred,
//! exactly as `RECALL_LOG_RETAIN_SECS` is).

use serde::{Deserialize, Serialize};

use aivyx_storage::DomainHandle;

// The decayed-view data types (TopicScore, AccumulatedHelpfulness) moved to the
// wasm-clean `aivyx-ipc` crate (Chapter M.2d); re-exported here so the
// persistent ledger + IPC are unchanged.
pub use aivyx_ipc::ledgers::{AccumulatedHelpfulness, TopicScore};

/// EWMA half-life: the elapsed time over which an untouched
/// topic's accumulated helpfulness halves. ~60 days — long
/// enough that a genuinely stable preference persists across
/// many reflection cycles, short enough that a year-old habit
/// no longer dominates.
pub const HELPFULNESS_HALF_LIFE_SECS: u64 = 60 * 24 * 3600;

/// A row whose decayed-to-now magnitude is below this is
/// "effectively zero" — eligible for pruning once it has also
/// gone untouched past the horizon.
pub const HELPFULNESS_PRUNE_EPSILON: f32 = 0.05;

/// A sub-epsilon row is only pruned once it has *also* not been
/// folded into for this long (~90 days). The two-part guard
/// keeps a briefly-quiet-but-recent topic alive while reaping
/// genuinely dead ones — bounded growth that mirrors the
/// signal's own decay.
pub const HELPFULNESS_PRUNE_HORIZON_SECS: u64 = 90 * 24 * 3600;

#[derive(Debug, thiserror::Error)]
pub enum HelpfulnessLedgerError {
    #[error("helpfulness ledger storage error: {0}")]
    Storage(String),
    #[error("helpfulness ledger encode error: {0}")]
    Encode(String),
}

/// One topic's durable helpfulness state. `ewma_score` is the
/// stored (un-decayed-since-`last_update_secs`) value; callers
/// that read it (`topic_score`, `ranked`) receive a copy whose
/// `ewma_score` has already been decayed to "now".
#[derive(
    Debug, Clone, PartialEq, Serialize, Deserialize,
)]
pub struct LedgerEntry {
    /// Time-decayed exponentially-weighted net helpfulness.
    pub ewma_score: f32,
    /// How many reflection windows have folded into this topic
    /// — a confidence proxy (one EWMA point ≠ a trend).
    pub samples: u32,
    /// When this row was last folded into. Drives both the
    /// read-time decay and the prune horizon.
    pub last_update_secs: u64,
}

/// Decay `score` from `last_update` forward to `now` by the
/// half-life. `dt == 0` → unchanged; far future → toward 0.
fn decayed(score: f32, last_update: u64, now: u64) -> f32 {
    let dt = now.saturating_sub(last_update);
    if dt == 0 {
        return score;
    }
    let factor = 0.5_f32.powf(
        dt as f32 / HELPFULNESS_HALF_LIFE_SECS as f32,
    );
    score * factor
}


/// Persistent helpfulness ledger over
/// [`aivyx_storage::KeyDomain::HelpfulnessLedger`]. Key = topic
/// bytes; value = JSON [`LedgerEntry`].
pub struct PersistentHelpfulnessLedger {
    storage: DomainHandle,
}

impl PersistentHelpfulnessLedger {
    pub fn new(storage: DomainHandle) -> Self {
        Self { storage }
    }

    async fn get_entry(
        &self,
        topic: &str,
    ) -> Result<Option<LedgerEntry>, HelpfulnessLedgerError> {
        let raw = self
            .storage
            .get(topic.as_bytes())
            .await
            .map_err(|e| {
                HelpfulnessLedgerError::Storage(e.to_string())
            })?;
        match raw {
            Some(bytes) => {
                let e: LedgerEntry =
                    serde_json::from_slice(&bytes).map_err(
                        |e| {
                            HelpfulnessLedgerError::Encode(
                                e.to_string(),
                            )
                        },
                    )?;
                Ok(Some(e))
            }
            None => Ok(None),
        }
    }

    /// Fold one reflection window's per-topic net helpfulness
    /// into the durable ledger (Q1a): for each topic, first
    /// decay the stored EWMA to `now`, then add this window's
    /// `net`, bump `samples`, and stamp `last_update`. A topic
    /// not seen before starts at this window's net with one
    /// sample.
    pub async fn record_window(
        &self,
        net_by_topic: &[(String, f32)],
        now_secs: u64,
    ) -> Result<(), HelpfulnessLedgerError> {
        for (topic, net) in net_by_topic {
            let entry = match self.get_entry(topic).await? {
                Some(prev) => LedgerEntry {
                    ewma_score: decayed(
                        prev.ewma_score,
                        prev.last_update_secs,
                        now_secs,
                    ) + net,
                    samples: prev.samples.saturating_add(1),
                    last_update_secs: now_secs,
                },
                None => LedgerEntry {
                    ewma_score: *net,
                    samples: 1,
                    last_update_secs: now_secs,
                },
            };
            let bytes =
                serde_json::to_vec(&entry).map_err(|e| {
                    HelpfulnessLedgerError::Encode(
                        e.to_string(),
                    )
                })?;
            self.storage
                .put(topic.as_bytes(), &bytes)
                .await
                .map_err(|e| {
                    HelpfulnessLedgerError::Storage(
                        e.to_string(),
                    )
                })?;
        }
        Ok(())
    }

    /// Point lookup, decayed to `now` (so callers see the
    /// current value without forcing a write).
    pub async fn topic_score(
        &self,
        topic: &str,
        now_secs: u64,
    ) -> Result<Option<LedgerEntry>, HelpfulnessLedgerError> {
        Ok(self.get_entry(topic).await?.map(|e| LedgerEntry {
            ewma_score: decayed(
                e.ewma_score,
                e.last_update_secs,
                now_secs,
            ),
            samples: e.samples,
            last_update_secs: e.last_update_secs,
        }))
    }

    /// Every topic, decayed to `now`, sorted by score
    /// descending (then topic, for deterministic output). For
    /// the Phase 78 longitudinal surface.
    pub async fn ranked(
        &self,
        now_secs: u64,
    ) -> Result<Vec<(String, LedgerEntry)>, HelpfulnessLedgerError>
    {
        let rows = self
            .storage
            .scan_prefix(&[])
            .await
            .map_err(|e| {
                HelpfulnessLedgerError::Storage(e.to_string())
            })?;
        let mut out: Vec<(String, LedgerEntry)> =
            Vec::with_capacity(rows.len());
        for (k, v) in &rows {
            let topic = String::from_utf8_lossy(k).into_owned();
            let e: LedgerEntry = serde_json::from_slice(v)
                .map_err(|e| {
                    HelpfulnessLedgerError::Encode(
                        e.to_string(),
                    )
                })?;
            out.push((
                topic,
                LedgerEntry {
                    ewma_score: decayed(
                        e.ewma_score,
                        e.last_update_secs,
                        now_secs,
                    ),
                    samples: e.samples,
                    last_update_secs: e.last_update_secs,
                },
            ));
        }
        out.sort_by(|a, b| {
            b.1.ewma_score
                .partial_cmp(&a.1.ewma_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        Ok(out)
    }

    /// The decayed accumulated top-helpful / top-unhelpful
    /// per-topic view (≤ `top_n` each) for the Phase 78
    /// longitudinal surface. Helpful = score > 0 (highest
    /// first); unhelpful = score < 0 (most negative first).
    /// Zero-score topics are omitted.
    pub async fn accumulated(
        &self,
        now_secs: u64,
        top_n: usize,
    ) -> Result<AccumulatedHelpfulness, HelpfulnessLedgerError>
    {
        // `ranked` is already score-desc, then topic.
        let ranked = self.ranked(now_secs).await?;
        let top_helpful: Vec<TopicScore> = ranked
            .iter()
            .filter(|(_, e)| e.ewma_score > 0.0)
            .take(top_n)
            .map(|(t, e)| TopicScore {
                topic: t.clone(),
                score: e.ewma_score,
                samples: e.samples,
            })
            .collect();
        let mut unhelpful: Vec<TopicScore> = ranked
            .iter()
            .filter(|(_, e)| e.ewma_score < 0.0)
            .map(|(t, e)| TopicScore {
                topic: t.clone(),
                score: e.ewma_score,
                samples: e.samples,
            })
            .collect();
        // Most-negative first (ranked had them least-negative
        // first since it is score-desc).
        unhelpful.reverse();
        unhelpful.truncate(top_n);
        Ok(AccumulatedHelpfulness {
            top_helpful,
            top_unhelpful: unhelpful,
        })
    }

    /// Drop rows whose decayed-to-`now` magnitude is below
    /// [`HELPFULNESS_PRUNE_EPSILON`] **and** which have not
    /// been folded into for [`HELPFULNESS_PRUNE_HORIZON_SECS`].
    /// Run on the reflection cadence; returns the prune count.
    pub async fn prune(
        &self,
        now_secs: u64,
    ) -> Result<usize, HelpfulnessLedgerError> {
        let rows = self
            .storage
            .scan_prefix(&[])
            .await
            .map_err(|e| {
                HelpfulnessLedgerError::Storage(e.to_string())
            })?;
        let mut pruned = 0usize;
        for (k, v) in &rows {
            let Ok(e) =
                serde_json::from_slice::<LedgerEntry>(v)
            else {
                continue;
            };
            let mag = decayed(
                e.ewma_score,
                e.last_update_secs,
                now_secs,
            )
            .abs();
            let stale = now_secs
                .saturating_sub(e.last_update_secs)
                > HELPFULNESS_PRUNE_HORIZON_SECS;
            if mag < HELPFULNESS_PRUNE_EPSILON && stale {
                self.storage.delete(k).await.map_err(|e| {
                    HelpfulnessLedgerError::Storage(
                        e.to_string(),
                    )
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
                "aivyx-helpfulness-ledger-test-{}",
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
    ) -> PersistentHelpfulnessLedger {
        let master = MasterKey::from_raw([mb; 32]);
        let storage: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(scratch.store_path()),
            master,
        )
        .await
        .expect("open store");
        PersistentHelpfulnessLedger::new(
            storage.domain(KeyDomain::HelpfulnessLedger),
        )
    }

    #[tokio::test]
    async fn empty_ledger_reads_none_and_prunes_zero() {
        let s = Scratch::new();
        let l = open_ledger(&s, 1).await;
        assert!(l
            .topic_score("anything", 100)
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
        l.record_window(&[("rust".into(), 3.0)], 1_000)
            .await
            .unwrap();
        let e = l
            .topic_score("rust", 1_000)
            .await
            .unwrap()
            .expect("seeded");
        assert!((e.ewma_score - 3.0).abs() < 1e-4);
        assert_eq!(e.samples, 1);

        // Same instant (no decay) → straight add, samples bump.
        l.record_window(&[("rust".into(), 2.0)], 1_000)
            .await
            .unwrap();
        let e = l
            .topic_score("rust", 1_000)
            .await
            .unwrap()
            .unwrap();
        assert!((e.ewma_score - 5.0).abs() < 1e-4);
        assert_eq!(e.samples, 2);
    }

    #[tokio::test]
    async fn stored_score_decays_over_time() {
        let s = Scratch::new();
        let l = open_ledger(&s, 3).await;
        l.record_window(&[("k".into(), 8.0)], 0).await.unwrap();
        // Exactly one half-life later → ~half.
        let half =
            l.topic_score("k", HELPFULNESS_HALF_LIFE_SECS)
                .await
                .unwrap()
                .unwrap();
        assert!(
            (half.ewma_score - 4.0).abs() < 1e-2,
            "got {}",
            half.ewma_score
        );
        // A fold one half-life later decays-then-adds.
        l.record_window(
            &[("k".into(), 1.0)],
            HELPFULNESS_HALF_LIFE_SECS,
        )
        .await
        .unwrap();
        let e = l
            .topic_score("k", HELPFULNESS_HALF_LIFE_SECS)
            .await
            .unwrap()
            .unwrap();
        assert!(
            (e.ewma_score - 5.0).abs() < 1e-2,
            "decay(8)=4 + 1 = 5, got {}",
            e.ewma_score
        );
    }

    #[tokio::test]
    async fn ranked_is_score_desc_then_topic() {
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
    async fn accumulated_splits_helpful_and_unhelpful() {
        let s = Scratch::new();
        let l = open_ledger(&s, 6).await;
        l.record_window(
            &[
                ("good".into(), 7.0),
                ("bad".into(), -4.0),
                ("worse".into(), -9.0),
                ("meh".into(), 0.0),
            ],
            50,
        )
        .await
        .unwrap();
        let a = l.accumulated(50, 10).await.unwrap();
        // Helpful: only positive, highest first.
        assert_eq!(a.top_helpful.len(), 1);
        assert_eq!(a.top_helpful[0].topic, "good");
        assert_eq!(a.top_helpful[0].samples, 1);
        // Unhelpful: only negative, MOST negative first.
        let un: Vec<&str> = a
            .top_unhelpful
            .iter()
            .map(|t| t.topic.as_str())
            .collect();
        assert_eq!(un, vec!["worse", "bad"]);
        // top_n caps each side.
        let capped = l.accumulated(50, 1).await.unwrap();
        assert_eq!(capped.top_unhelpful.len(), 1);
        assert_eq!(capped.top_unhelpful[0].topic, "worse");
    }

    #[tokio::test]
    async fn prune_drops_only_decayed_and_stale() {
        let s = Scratch::new();
        let l = open_ledger(&s, 5).await;
        // A tiny, ancient row → decays sub-epsilon AND stale.
        l.record_window(&[("dead".into(), 0.10)], 0)
            .await
            .unwrap();
        // A strong, ancient row → stale but NOT sub-epsilon
        // even after decay.
        l.record_window(&[("strong".into(), 5_000.0)], 0)
            .await
            .unwrap();
        let now = HELPFULNESS_PRUNE_HORIZON_SECS + 1;
        let pruned = l.prune(now).await.unwrap();
        assert_eq!(pruned, 1);
        assert!(l
            .topic_score("dead", now)
            .await
            .unwrap()
            .is_none());
        assert!(l
            .topic_score("strong", now)
            .await
            .unwrap()
            .is_some());

        // A fresh tiny row is sub-epsilon but NOT stale → kept.
        l.record_window(&[("freshtiny".into(), 0.01)], now)
            .await
            .unwrap();
        assert_eq!(l.prune(now).await.unwrap(), 0);
        assert!(l
            .topic_score("freshtiny", now)
            .await
            .unwrap()
            .is_some());
    }
}
