//! Phase 83 — the persistent cross-session co-occurrence
//! ledger.
//!
//! Phase 77 learns *which topics help*; Phase 82 made that
//! per-topic signal durable. This learns the relationships
//! *between* topics: which two topics get recalled **together**
//! in turns that go well. One row per canonical topic pair
//! holds a time-decayed EWMA of that joint-helpfulness, folded
//! in on the existing reflection cadence (Task 3), backed by
//! the HKDF-isolated
//! [`aivyx_storage::KeyDomain::CooccurrenceLedger`] so a
//! corrupt/pruned row degrades only the cross-session pattern
//! view — never memory, recall, the helpfulness ledger, or
//! proactive dedup.
//!
//! The design deliberately mirrors the Phase 82
//! `PersistentHelpfulnessLedger` (recency-weighted EWMA,
//! read-time decay, self-prune, zero-config) — the only
//! material difference is the **key**: a collision-safe
//! canonical encoding of the unordered topic pair.
//!
//! **Zero-config (Q4a).** Like Phase 77 / Phase 82: a passive
//! internal signal, auto-initialised, no `[…]` block, no
//! behaviour change on its own.

use serde::{Deserialize, Serialize};

use aivyx_storage::DomainHandle;

/// EWMA half-life for a pair's joint-helpfulness (~60 days),
/// matching the Phase 82 per-topic ledger so the two durable
/// signals age on the same clock.
pub const COOCCURRENCE_HALF_LIFE_SECS: u64 = 60 * 24 * 3600;

/// A pair whose decayed-to-now magnitude is below this is
/// "effectively zero" — eligible for pruning once also stale.
pub const COOCCURRENCE_PRUNE_EPSILON: f32 = 0.05;

/// A sub-epsilon pair is only pruned once it has *also* not
/// been folded into for this long (~90 days).
pub const COOCCURRENCE_PRUNE_HORIZON_SECS: u64 = 90 * 24 * 3600;

/// The deterministic O(n²) bound (Q4a): per recall event, only
/// the pairs among the **top-K highest-scoring distinct
/// topics** are folded. A turn that recalled 30 memories does
/// not explode into C(30,2)=435 pair rows; it contributes at
/// most C(8,2)=28. Bounds storage + write amplification while
/// keeping the strongest co-recalled relationships.
pub const COOCCURRENCE_TOP_K_HITS: usize = 8;

#[derive(Debug, thiserror::Error)]
pub enum CooccurrenceLedgerError {
    #[error("cooccurrence ledger storage error: {0}")]
    Storage(String),
    #[error("cooccurrence ledger encode error: {0}")]
    Encode(String),
}

/// One canonical topic pair's durable joint-helpfulness state.
/// Same shape as the Phase 82 `LedgerEntry`; kept local so the
/// two ledgers stay independent modules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairEntry {
    pub ewma_score: f32,
    pub samples: u32,
    pub last_update_secs: u64,
}

/// One affined topic pair, for the Phase 78 surface.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairScore {
    pub a: String,
    pub b: String,
    pub score: f32,
    pub samples: u32,
}

/// The durable, decayed top co-occurring topic pairs (the
/// cross-session pattern view — "topics that consistently help
/// together").
#[derive(
    Debug, Clone, Default, PartialEq, Serialize, Deserialize,
)]
pub struct CooccurrencePatterns {
    pub top_pairs: Vec<PairScore>,
}

/// Decay `score` from `last_update` forward to `now` by the
/// half-life. `dt == 0` → unchanged.
fn decayed(score: f32, last_update: u64, now: u64) -> f32 {
    let dt = now.saturating_sub(last_update);
    if dt == 0 {
        return score;
    }
    let factor = 0.5_f32.powf(
        dt as f32 / COOCCURRENCE_HALF_LIFE_SECS as f32,
    );
    score * factor
}

/// Collision-safe canonical key for an unordered topic pair.
/// The two topics are sorted (so `{A,B}` and `{B,A}` map to
/// one row), then **length-prefixed** rather than
/// delimiter-joined: `[len(lo) as u32 BE][lo bytes][hi bytes]`.
/// Length-prefixing is unambiguous where a delimiter is not —
/// `("a|","b")` and `("a","|b")` produce distinct keys.
fn pair_key(t1: &str, t2: &str) -> Vec<u8> {
    let (lo, hi) = if t1 <= t2 { (t1, t2) } else { (t2, t1) };
    let mut k =
        (lo.len() as u32).to_be_bytes().to_vec();
    k.extend_from_slice(lo.as_bytes());
    k.extend_from_slice(hi.as_bytes());
    k
}

/// Inverse of [`pair_key`]: recover `(lo, hi)` for the surface
/// / ranking. Returns `None` on a malformed key (skipped).
fn split_key(k: &[u8]) -> Option<(String, String)> {
    if k.len() < 4 {
        return None;
    }
    let mut len_buf = [0u8; 4];
    len_buf.copy_from_slice(&k[..4]);
    let lo_len = u32::from_be_bytes(len_buf) as usize;
    if 4 + lo_len > k.len() {
        return None;
    }
    let lo =
        String::from_utf8(k[4..4 + lo_len].to_vec()).ok()?;
    let hi =
        String::from_utf8(k[4 + lo_len..].to_vec()).ok()?;
    Some((lo, hi))
}

/// Persistent co-occurrence ledger over
/// [`aivyx_storage::KeyDomain::CooccurrenceLedger`]. Key = the
/// canonical pair encoding; value = JSON [`PairEntry`].
pub struct PersistentCooccurrenceLedger {
    storage: DomainHandle,
}

impl PersistentCooccurrenceLedger {
    pub fn new(storage: DomainHandle) -> Self {
        Self { storage }
    }

    async fn get_entry(
        &self,
        key: &[u8],
    ) -> Result<Option<PairEntry>, CooccurrenceLedgerError>
    {
        let raw =
            self.storage.get(key).await.map_err(|e| {
                CooccurrenceLedgerError::Storage(e.to_string())
            })?;
        match raw {
            Some(bytes) => {
                let e: PairEntry =
                    serde_json::from_slice(&bytes).map_err(
                        |e| {
                            CooccurrenceLedgerError::Encode(
                                e.to_string(),
                            )
                        },
                    )?;
                Ok(Some(e))
            }
            None => Ok(None),
        }
    }

    /// Fold one reflection window's per-pair net joint
    /// helpfulness into the durable ledger: decay the stored
    /// EWMA to `now`, add this window's net, bump `samples`,
    /// stamp `last_update`. Unseen pair → seeds at net / 1.
    pub async fn record_window(
        &self,
        pair_net: &[((String, String), f32)],
        now_secs: u64,
    ) -> Result<(), CooccurrenceLedgerError> {
        for ((a, b), net) in pair_net {
            let key = pair_key(a, b);
            let entry = match self.get_entry(&key).await? {
                Some(prev) => PairEntry {
                    ewma_score: decayed(
                        prev.ewma_score,
                        prev.last_update_secs,
                        now_secs,
                    ) + net,
                    samples: prev.samples.saturating_add(1),
                    last_update_secs: now_secs,
                },
                None => PairEntry {
                    ewma_score: *net,
                    samples: 1,
                    last_update_secs: now_secs,
                },
            };
            let bytes =
                serde_json::to_vec(&entry).map_err(|e| {
                    CooccurrenceLedgerError::Encode(
                        e.to_string(),
                    )
                })?;
            self.storage.put(&key, &bytes).await.map_err(
                |e| {
                    CooccurrenceLedgerError::Storage(
                        e.to_string(),
                    )
                },
            )?;
        }
        Ok(())
    }

    /// Point lookup for a pair, decayed to `now`.
    pub async fn pair_score(
        &self,
        t1: &str,
        t2: &str,
        now_secs: u64,
    ) -> Result<Option<PairEntry>, CooccurrenceLedgerError>
    {
        let key = pair_key(t1, t2);
        Ok(self.get_entry(&key).await?.map(|e| PairEntry {
            ewma_score: decayed(
                e.ewma_score,
                e.last_update_secs,
                now_secs,
            ),
            samples: e.samples,
            last_update_secs: e.last_update_secs,
        }))
    }

    /// Every pair, decayed to `now`, sorted by score
    /// descending (then `(lo, hi)`, deterministic).
    pub async fn ranked(
        &self,
        now_secs: u64,
    ) -> Result<
        Vec<(String, String, PairEntry)>,
        CooccurrenceLedgerError,
    > {
        let rows = self
            .storage
            .scan_prefix(&[])
            .await
            .map_err(|e| {
                CooccurrenceLedgerError::Storage(e.to_string())
            })?;
        let mut out: Vec<(String, String, PairEntry)> =
            Vec::with_capacity(rows.len());
        for (k, v) in &rows {
            let Some((lo, hi)) = split_key(k) else {
                continue;
            };
            let e: PairEntry = serde_json::from_slice(v)
                .map_err(|e| {
                    CooccurrenceLedgerError::Encode(
                        e.to_string(),
                    )
                })?;
            out.push((
                lo,
                hi,
                PairEntry {
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
        out.sort_by(|x, y| {
            y.2.ewma_score
                .partial_cmp(&x.2.ewma_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| (&x.0, &x.1).cmp(&(&y.0, &y.1)))
        });
        Ok(out)
    }

    /// The decayed top affined pairs (score > 0, ≤ `top_n`)
    /// for the Phase 78 surface.
    pub async fn top_affinities(
        &self,
        now_secs: u64,
        top_n: usize,
    ) -> Result<CooccurrencePatterns, CooccurrenceLedgerError>
    {
        let ranked = self.ranked(now_secs).await?;
        let top_pairs: Vec<PairScore> = ranked
            .iter()
            .filter(|(_, _, e)| e.ewma_score > 0.0)
            .take(top_n)
            .map(|(a, b, e)| PairScore {
                a: a.clone(),
                b: b.clone(),
                score: e.ewma_score,
                samples: e.samples,
            })
            .collect();
        Ok(CooccurrencePatterns { top_pairs })
    }

    /// Drop pairs whose decayed-to-`now` magnitude is below
    /// [`COOCCURRENCE_PRUNE_EPSILON`] **and** untouched for
    /// [`COOCCURRENCE_PRUNE_HORIZON_SECS`]. Run on the
    /// reflection cadence; returns the prune count.
    pub async fn prune(
        &self,
        now_secs: u64,
    ) -> Result<usize, CooccurrenceLedgerError> {
        let rows = self
            .storage
            .scan_prefix(&[])
            .await
            .map_err(|e| {
                CooccurrenceLedgerError::Storage(e.to_string())
            })?;
        let mut pruned = 0usize;
        for (k, v) in &rows {
            let Ok(e) =
                serde_json::from_slice::<PairEntry>(v)
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
                > COOCCURRENCE_PRUNE_HORIZON_SECS;
            if mag < COOCCURRENCE_PRUNE_EPSILON && stale {
                self.storage.delete(k).await.map_err(|e| {
                    CooccurrenceLedgerError::Storage(
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
                "aivyx-cooccurrence-test-{}",
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
    ) -> PersistentCooccurrenceLedger {
        let master = MasterKey::from_raw([mb; 32]);
        let storage: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(scratch.store_path()),
            master,
        )
        .await
        .expect("open store");
        PersistentCooccurrenceLedger::new(
            storage.domain(KeyDomain::CooccurrenceLedger),
        )
    }

    #[test]
    fn pair_key_is_order_invariant_and_collision_safe() {
        // Order-invariant: {A,B} == {B,A}.
        assert_eq!(pair_key("alpha", "beta"), pair_key("beta", "alpha"));
        // Collision-safe across a delimiter-ambiguous case
        // that a naive `a|b` join would conflate.
        assert_ne!(pair_key("a|", "b"), pair_key("a", "|b"));
        // Round-trips through split_key (sorted).
        let (lo, hi) =
            split_key(&pair_key("zed", "abe")).unwrap();
        assert_eq!((lo.as_str(), hi.as_str()), ("abe", "zed"));
    }

    #[tokio::test]
    async fn empty_ledger_reads_none_and_prunes_zero() {
        let s = Scratch::new();
        let l = open_ledger(&s, 1).await;
        assert!(l
            .pair_score("x", "y", 100)
            .await
            .unwrap()
            .is_none());
        assert_eq!(l.prune(100).await.unwrap(), 0);
        assert!(l
            .top_affinities(100, 10)
            .await
            .unwrap()
            .top_pairs
            .is_empty());
    }

    #[tokio::test]
    async fn fold_seeds_then_accumulates_order_invariant() {
        let s = Scratch::new();
        let l = open_ledger(&s, 2).await;
        l.record_window(
            &[(("deploy".into(), "rollback".into()), 3.0)],
            1_000,
        )
        .await
        .unwrap();
        // Lookup with reversed order hits the same row.
        let e = l
            .pair_score("rollback", "deploy", 1_000)
            .await
            .unwrap()
            .expect("seeded");
        assert!((e.ewma_score - 3.0).abs() < 1e-4);
        assert_eq!(e.samples, 1);

        // Same instant, reversed input order → straight add.
        l.record_window(
            &[(("rollback".into(), "deploy".into()), 2.0)],
            1_000,
        )
        .await
        .unwrap();
        let e = l
            .pair_score("deploy", "rollback", 1_000)
            .await
            .unwrap()
            .unwrap();
        assert!((e.ewma_score - 5.0).abs() < 1e-4);
        assert_eq!(e.samples, 2);
    }

    #[tokio::test]
    async fn stored_pair_decays_over_time() {
        let s = Scratch::new();
        let l = open_ledger(&s, 3).await;
        l.record_window(&[(("a".into(), "b".into()), 8.0)], 0)
            .await
            .unwrap();
        let half = l
            .pair_score("a", "b", COOCCURRENCE_HALF_LIFE_SECS)
            .await
            .unwrap()
            .unwrap();
        assert!(
            (half.ewma_score - 4.0).abs() < 1e-2,
            "one half-life → ~half, got {}",
            half.ewma_score
        );
    }

    #[tokio::test]
    async fn top_affinities_is_score_desc_positive_only() {
        let s = Scratch::new();
        let l = open_ledger(&s, 4).await;
        l.record_window(
            &[
                (("a".into(), "b".into()), 9.0),
                (("c".into(), "d".into()), 2.0),
                (("e".into(), "f".into()), -5.0),
            ],
            10,
        )
        .await
        .unwrap();
        let p = l.top_affinities(10, 10).await.unwrap();
        // Negative pair excluded; positives score-desc.
        assert_eq!(p.top_pairs.len(), 2);
        assert_eq!(
            (p.top_pairs[0].a.as_str(), p.top_pairs[0].b.as_str()),
            ("a", "b")
        );
        assert_eq!(
            (p.top_pairs[1].a.as_str(), p.top_pairs[1].b.as_str()),
            ("c", "d")
        );
        // top_n caps.
        assert_eq!(
            l.top_affinities(10, 1)
                .await
                .unwrap()
                .top_pairs
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn prune_drops_only_decayed_and_stale() {
        let s = Scratch::new();
        let l = open_ledger(&s, 5).await;
        l.record_window(&[(("d".into(), "e".into()), 0.10)], 0)
            .await
            .unwrap();
        l.record_window(
            &[(("s".into(), "t".into()), 5_000.0)],
            0,
        )
        .await
        .unwrap();
        let now = COOCCURRENCE_PRUNE_HORIZON_SECS + 1;
        assert_eq!(l.prune(now).await.unwrap(), 1);
        assert!(l
            .pair_score("d", "e", now)
            .await
            .unwrap()
            .is_none());
        assert!(l
            .pair_score("s", "t", now)
            .await
            .unwrap()
            .is_some());
        // Fresh tiny pair is sub-epsilon but not stale → kept.
        l.record_window(
            &[(("fresh".into(), "tiny".into()), 0.01)],
            now,
        )
        .await
        .unwrap();
        assert_eq!(l.prune(now).await.unwrap(), 0);
    }
}
