//! Phase 98 — Reciprocal Rank Fusion for hybrid
//! keyword+semantic recall.
//!
//! ## Why RRF
//!
//! Auto-recall currently ranks by cosine similarity over
//! embeddings. Embeddings encode semantic relationships
//! well but miss rare-term recall — acronyms, proper
//! nouns, code identifiers, project codenames. The
//! keyword search tool (Phase 74) handles those exact-
//! match cases via `Memory::search`. Phase 98 runs both
//! rankers on every recall when `recall_hybrid = true`
//! and fuses their rankings here.
//!
//! Reciprocal Rank Fusion is the standard production
//! approach to combining multiple rankers. The formula:
//!
//! ```text
//! score(item) = Σ over rankers of  1 / (k + rank_in_ranker + 1)
//! ```
//!
//! Where `rank_in_ranker` is 0-indexed position in that
//! ranker's output (smaller is better). `k` is a smoothing
//! constant — `60` is the industry-standard value, set as
//! `RRF_K`.
//!
//! ## Why RRF, not score fusion
//!
//! Cosine similarity scores are in `[-1, 1]`; substring
//! hit counts (or BM25 scores, if we add that later) are
//! in different ranges. Score fusion requires
//! normalization and an alpha parameter. RRF is **rank-
//! based**: it doesn't care about each ranker's score
//! scale, only the *position* the ranker put the item at.
//! That's why ~30 lines of pure arithmetic give a
//! production-quality fusion without normalization or
//! tuning knobs.
//!
//! ## Chapter Loom (LM.1) — per-source weighting
//!
//! Phase 98 fuses exactly two equal-weight rankers
//! (semantic + substring). Chapter Loom fuses **three**
//! sources — semantic, BM25 lexical (LM.2), and a multi-hop
//! co-occurrence walk (LM.3) — and wants the operator to be
//! able to bias toward exact-term recall (`lexical_weight`)
//! without recompiling. [`reciprocal_rank_fusion_weighted`]
//! adds a per-source weight multiplier to each ranker's RRF
//! contribution; the original [`reciprocal_rank_fusion`] is
//! now the all-weights-`1.0` case (it delegates, so its
//! proven behavior is unchanged). Both stay **pure** — LM.1
//! ships them inert; LM.4 is the seam that wires the third
//! source + the weights into live recall.

use std::collections::HashMap;

/// Phase 98 — Reciprocal Rank Fusion smoothing constant.
/// `60` is the well-known industry value first proposed
/// by Cormack, Clarke, and Buettcher; widely used in
/// production search systems. The project hard-codes this
/// (Q4's `recall_hybrid_k` operator knob is a documented
/// deferral) because every operator would set it to 60.
pub const RRF_K: usize = 60;

/// Phase 98 — fuse multiple per-ranker `(topic, seq)`
/// orderings into one fused ordering via Reciprocal Rank
/// Fusion. Each input `Vec<(String, u64)>` is one
/// ranker's output in best-first order; position 0 is the
/// top hit for that ranker.
///
/// Returns up to `limit` items sorted by fused score
/// descending. Ties on score break deterministically:
/// topic ascending, seq descending (matches
/// `rank_by_cosine`'s topic/seq disambiguation so the
/// downstream pipeline never sees an out-of-band
/// reorder).
///
/// Edge cases:
/// - Empty `rankings` (no rankers at all) → empty output.
/// - All rankers empty → empty output.
/// - Single ranker → same order, monotonic-descending
///   fused scores.
/// - `limit == 0` → empty output.
/// - `k == 0` → defended to `k = 1` (the formula uses
///   `k + rank + 1` so `k = 0` produces valid scores but
///   weights the top item heavily; defending against
///   nonsense operator values).
pub fn reciprocal_rank_fusion(
    rankings: &[Vec<(String, u64)>],
    k: usize,
    limit: usize,
) -> Vec<(String, u64, f32)> {
    // The original Phase 98 surface is the all-weights-1.0
    // case of the weighted helper. Delegate (no clone — pass
    // each ranker as a weight-1.0 slice) so both share one
    // accumulation + sort path and the proven behavior here
    // can never drift from the weighted version.
    fuse_inner(rankings.iter().map(|r| (1.0_f32, r.as_slice())), k, limit)
}

/// Chapter Loom (LM.1) — Reciprocal Rank Fusion with a
/// per-source **weight** multiplier. Each input is
/// `(weight, ranker)`; the ranker's RRF contribution at each
/// position is scaled by `weight`, so a source with weight
/// `2.0` counts like two identical equal-weight sources and a
/// source with weight `0.0` drops out entirely. With every
/// weight `1.0` this is byte-identical to
/// [`reciprocal_rank_fusion`].
///
/// Weights are defended like `k`: a non-finite weight (NaN /
/// ∞) falls back to `1.0`, and a negative weight clamps to
/// `0.0` (a ranker can be silenced but never subtract). Same
/// `(topic, seq)` identity, tie-break, and edge cases as the
/// unweighted form.
pub fn reciprocal_rank_fusion_weighted(
    rankings: &[(f32, Vec<(String, u64)>)],
    k: usize,
    limit: usize,
) -> Vec<(String, u64, f32)> {
    fuse_inner(
        rankings.iter().map(|(w, r)| (*w, r.as_slice())),
        k,
        limit,
    )
}

/// Shared accumulation + sort core for both public fusers.
/// Takes an iterator of `(weight, ranker_slice)` so neither
/// public form needs to clone its input.
fn fuse_inner<'a>(
    rankings: impl Iterator<Item = (f32, &'a [(String, u64)])>,
    k: usize,
    limit: usize,
) -> Vec<(String, u64, f32)> {
    if limit == 0 {
        return Vec::new();
    }
    let k = k.max(1);

    // Accumulate weighted fused scores per (topic, seq).
    let mut fused: HashMap<(String, u64), f32> = HashMap::new();
    for (weight, ranker) in rankings {
        // Defend operator-supplied weights: NaN/∞ → 1.0,
        // negative → 0.0 (silence, never subtract).
        let weight = if weight.is_finite() { weight.max(0.0) } else { 1.0 };
        // A zero-weight source is silenced entirely — skip it so
        // its unique items don't enter the fused set with score
        // 0.0 (they'd otherwise rank last but still appear).
        if weight == 0.0 {
            continue;
        }
        for (rank, (topic, seq)) in ranker.iter().enumerate() {
            let contribution =
                weight * (1.0_f32 / (k as f32 + rank as f32 + 1.0));
            *fused
                .entry((topic.clone(), *seq))
                .or_insert(0.0) += contribution;
        }
    }

    if fused.is_empty() {
        return Vec::new();
    }

    let mut out: Vec<(String, u64, f32)> = fused
        .into_iter()
        .map(|((topic, seq), score)| (topic, seq, score))
        .collect();
    // Sort by score desc; tie-break by topic asc, seq
    // desc. Topic ordering is the natural secondary
    // signal and matches the alphabetical convention used
    // elsewhere; seq desc keeps newer entries above older
    // on a tie within the same topic.
    out.sort_by(|a, b| {
        b.2.partial_cmp(&a.2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
            .then_with(|| b.1.cmp(&a.1))
    });
    out.truncate(limit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(items: &[(&str, u64)]) -> Vec<(String, u64)> {
        items
            .iter()
            .map(|(t, s)| ((*t).to_string(), *s))
            .collect()
    }

    #[test]
    fn empty_rankings_empty_output() {
        let out = reciprocal_rank_fusion(&[], RRF_K, 10);
        assert!(out.is_empty());
    }

    #[test]
    fn all_empty_rankings_empty_output() {
        let out = reciprocal_rank_fusion(
            &[vec![], vec![]],
            RRF_K,
            10,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn zero_limit_empty_output() {
        let ranking = r(&[("a", 1), ("b", 2)]);
        let out = reciprocal_rank_fusion(
            &[ranking],
            RRF_K,
            0,
        );
        assert!(out.is_empty());
    }

    /// Phase 98 — a single ranker's order is preserved.
    /// Fused scores are monotonic-descending (each later
    /// position contributes a strictly smaller `1 / (k +
    /// rank + 1)` term).
    #[test]
    fn single_ranker_preserves_order() {
        let ranking = r(&[("a", 1), ("b", 2), ("c", 3)]);
        let out = reciprocal_rank_fusion(
            &[ranking],
            RRF_K,
            10,
        );
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].0, "a");
        assert_eq!(out[1].0, "b");
        assert_eq!(out[2].0, "c");
        // Monotonic descending.
        assert!(out[0].2 > out[1].2);
        assert!(out[1].2 > out[2].2);
    }

    /// Phase 98 — two identical rankings → same order;
    /// scores double per item (each contributes its
    /// position-based score from each ranker).
    #[test]
    fn two_identical_rankings_double_scores() {
        let ranking = r(&[("a", 1), ("b", 2)]);
        let single = reciprocal_rank_fusion(
            std::slice::from_ref(&ranking),
            RRF_K,
            10,
        );
        let double = reciprocal_rank_fusion(
            &[ranking.clone(), ranking],
            RRF_K,
            10,
        );
        assert_eq!(single.len(), double.len());
        for (s, d) in single.iter().zip(double.iter()) {
            assert_eq!(s.0, d.0);
            assert_eq!(s.1, d.1);
            assert!((d.2 - 2.0 * s.2).abs() < 1e-6);
        }
    }

    /// Phase 98 — the core RRF claim: an item appearing
    /// in BOTH rankings ranks above items appearing in
    /// only one. With `k=60`, even a position-0 item
    /// scores `1/61 ≈ 0.0164`; an item at position 5 in
    /// BOTH scores `2/66 ≈ 0.0303` — strictly higher.
    #[test]
    fn item_in_both_rankings_outranks_single_ranker_hits() {
        // Ranker A: shared at position 5; A-only at 0.
        let a = r(&[
            ("a_only", 1),
            ("filler1", 2),
            ("filler2", 3),
            ("filler3", 4),
            ("filler4", 5),
            ("shared", 100),
        ]);
        // Ranker B: shared at position 5; B-only at 0.
        let b = r(&[
            ("b_only", 6),
            ("filler5", 7),
            ("filler6", 8),
            ("filler7", 9),
            ("filler8", 10),
            ("shared", 100),
        ]);
        let out = reciprocal_rank_fusion(
            &[a, b],
            RRF_K,
            12,
        );
        // `shared` should win even though it's at position
        // 5 in each ranker — because it gets contributions
        // from both.
        assert_eq!(out[0].0, "shared");
    }

    /// Phase 98 — `limit` truncates correctly. 5 items
    /// across two rankers, ask for top-2.
    #[test]
    fn limit_truncates_results() {
        let a = r(&[("x", 1), ("y", 2), ("z", 3)]);
        let b = r(&[("y", 2), ("w", 4), ("v", 5)]);
        let out = reciprocal_rank_fusion(
            &[a, b],
            RRF_K,
            2,
        );
        assert_eq!(out.len(), 2);
    }

    /// Phase 98 — defended `k = 0`: the helper clamps to
    /// 1 (the formula uses `k + rank + 1`, so `k = 0`
    /// gives finite scores, but a position-0 item gets
    /// score 1.0 while position-1 gets 0.5 — extreme
    /// weighting. Defending makes the helper robust to
    /// nonsense operator values).
    #[test]
    fn defended_k_zero_does_not_panic_and_returns_results() {
        let ranking = r(&[("a", 1), ("b", 2)]);
        let out = reciprocal_rank_fusion(
            &[ranking],
            0,
            10,
        );
        assert_eq!(out.len(), 2);
        // Order still preserved.
        assert_eq!(out[0].0, "a");
        assert_eq!(out[1].0, "b");
    }

    /// Phase 98 — disjoint rankings: each item is in one
    /// ranker only. The fused list contains every item.
    /// Top hit from each ranker should appear near the
    /// top of the fused result (position 0 in either
    /// gives the same contribution).
    #[test]
    fn disjoint_rankings_keep_all_items() {
        let a = r(&[("a", 1), ("c", 3)]);
        let b = r(&[("b", 2), ("d", 4)]);
        let out = reciprocal_rank_fusion(
            &[a, b],
            RRF_K,
            10,
        );
        assert_eq!(out.len(), 4);
        // Position-0 items ("a" and "b") tie on score
        // (each appears at position 0 in exactly one
        // ranker). Tie-break is topic ascending, so "a"
        // ranks above "b".
        assert_eq!(out[0].0, "a");
        assert_eq!(out[1].0, "b");
    }

    /// Phase 98 — determinism: same input always
    /// produces same output (no HashMap iteration order
    /// leaking into the sort).
    #[test]
    fn fusion_is_deterministic() {
        let a = r(&[("a", 1), ("b", 2), ("c", 3)]);
        let b = r(&[("c", 3), ("d", 4), ("e", 5)]);
        let first = reciprocal_rank_fusion(
            &[a.clone(), b.clone()],
            RRF_K,
            10,
        );
        for _ in 0..5 {
            let again = reciprocal_rank_fusion(
                &[a.clone(), b.clone()],
                RRF_K,
                10,
            );
            assert_eq!(first, again);
        }
    }

    // ---- Chapter Loom (LM.1) — weighted RRF ----

    /// All-weights-`1.0` weighted fusion is byte-identical to
    /// the unweighted form — the invariant that lets the
    /// unweighted public fn delegate safely.
    #[test]
    fn weighted_all_ones_equals_unweighted() {
        let a = r(&[("a", 1), ("b", 2), ("c", 3)]);
        let b = r(&[("c", 3), ("d", 4), ("e", 5)]);
        let plain = reciprocal_rank_fusion(&[a.clone(), b.clone()], RRF_K, 10);
        let weighted = reciprocal_rank_fusion_weighted(
            &[(1.0, a), (1.0, b)],
            RRF_K,
            10,
        );
        assert_eq!(plain, weighted);
    }

    /// A weight of `2.0` scales a source's contribution like
    /// two copies of it: one source at weight 2.0 matches the
    /// same ranker listed twice at weight 1.0.
    #[test]
    fn weight_two_matches_duplicated_source() {
        let a = r(&[("a", 1), ("b", 2)]);
        let doubled = reciprocal_rank_fusion(
            &[a.clone(), a.clone()],
            RRF_K,
            10,
        );
        let weighted = reciprocal_rank_fusion_weighted(
            &[(2.0, a)],
            RRF_K,
            10,
        );
        assert_eq!(doubled.len(), weighted.len());
        for (d, w) in doubled.iter().zip(weighted.iter()) {
            assert_eq!(d.0, w.0);
            assert_eq!(d.1, w.1);
            assert!((d.2 - w.2).abs() < 1e-6, "score {d:?} vs {w:?}");
        }
    }

    /// `lexical_weight`-style biasing: raising a source's
    /// weight lifts an item it ranks #1 above an item the
    /// other source ranks #1. Tunable exact-term preference.
    #[test]
    fn higher_weight_biases_toward_that_source() {
        let semantic = r(&[("sem_top", 1)]);
        let lexical = r(&[("lex_top", 2)]);
        // Equal weight → tie broken by topic asc ("lex_top" < "sem_top").
        let even = reciprocal_rank_fusion_weighted(
            &[(1.0, semantic.clone()), (1.0, lexical.clone())],
            RRF_K,
            10,
        );
        assert_eq!(even[0].0, "lex_top");
        // Bias semantic heavily → its top wins outright.
        let biased = reciprocal_rank_fusion_weighted(
            &[(5.0, semantic), (1.0, lexical)],
            RRF_K,
            10,
        );
        assert_eq!(biased[0].0, "sem_top");
    }

    /// Weight `0.0` silences a source entirely — items unique
    /// to it never appear in the fused output.
    #[test]
    fn zero_weight_drops_a_source() {
        let keep = r(&[("keep", 1)]);
        let drop = r(&[("dropped", 2)]);
        let out = reciprocal_rank_fusion_weighted(
            &[(1.0, keep), (0.0, drop)],
            RRF_K,
            10,
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, "keep");
    }

    /// Defended weights: a negative weight clamps to 0.0
    /// (silence, never subtract) and a non-finite weight
    /// (NaN) falls back to 1.0.
    #[test]
    fn defended_weights_negative_and_nan() {
        let a = r(&[("a", 1)]);
        let b = r(&[("b", 2)]);
        // Negative weight on b → b silenced.
        let neg = reciprocal_rank_fusion_weighted(
            &[(1.0, a.clone()), (-3.0, b.clone())],
            RRF_K,
            10,
        );
        assert_eq!(neg.len(), 1);
        assert_eq!(neg[0].0, "a");
        // NaN weight → treated as 1.0, so both survive and
        // the result matches plain equal-weight fusion.
        let nan = reciprocal_rank_fusion_weighted(
            &[(f32::NAN, a.clone()), (1.0, b.clone())],
            RRF_K,
            10,
        );
        let plain = reciprocal_rank_fusion(&[a, b], RRF_K, 10);
        assert_eq!(nan, plain);
    }
}
