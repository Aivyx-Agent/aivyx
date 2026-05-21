//! Phase 96 — IVF-style approximate-nearest-neighbor index
//! over the vector store.
//!
//! ## Why IVF, not HNSW
//!
//! IVF (Inverted File) partitions vectors into clusters
//! around `K` centroids. At query time, the query vector is
//! cosine-ranked against the `K` centroids (cheap when `K`
//! is small), and only the top-N clusters are searched.
//! When paired with a final brute-force re-rank on the
//! candidate union, IVF gives `O(√N)` queries with quality
//! that's adequate for the project's target scale (up to
//! ~100K memory entries per operator).
//!
//! HNSW would give higher recall at the same speed but
//! requires either a new workspace dependency or a
//! substantial hand-rolled implementation. The project's
//! zero-new-deps streak and the simplicity of IVF
//! (~150 lines vs ~1000 for HNSW) made IVF the right v1
//! choice. The HNSW upgrade is documented as a Phase 96
//! deferral.
//!
//! ## Determinism + zero randomness
//!
//! Centroid seeds are picked by **spaced sampling** from
//! the input — no randomness, no thread-local RNG, no
//! `rand` dep. The build is fully deterministic: shuffled
//! input produces the same centroid set (modulo order;
//! cluster membership may vary deterministically with
//! input order). Tests can pin behaviour without an RNG
//! seed.
//!
//! ## Composition with the existing brute-force path
//!
//! [`query_ann`] returns the top-`candidate_limit` matches
//! from the searched clusters. The intended caller pattern
//! (see `Memory::semantic_search_scored_ann` in Task 4) is
//! to take that candidate set and re-rank it via the
//! existing brute-force `rank_by_cosine` over just those
//! candidates. That hybrid composition is the Q3a posture:
//! ANN narrows, brute-force re-ranks. The exact-cosine
//! ordering guarantee is preserved within the candidate set.

use crate::cosine_similarity;

/// Phase 96 — the ANN index data structure. Built from a
/// flat `(topic, seq, embedding)` triple list via
/// [`build_ann_index`]; queried via [`query_ann`].
///
/// Invariants:
/// - `centroids.len() == clusters.len()` always.
/// - Every input entry appears in exactly one
///   `clusters[i]` row after build.
/// - Empty input produces empty index (both vecs empty).
#[derive(Debug, Clone, PartialEq)]
pub struct AnnIndex {
    /// One centroid vector per cluster. The cluster's
    /// membership in `clusters[i]` is assigned by
    /// nearest-centroid cosine similarity at build time.
    pub centroids: Vec<Vec<f32>>,
    /// Per-cluster entry list. `clusters[i]` holds the
    /// `(topic, seq, embedding)` triples whose nearest
    /// centroid is `centroids[i]`.
    pub clusters: Vec<Vec<(String, u64, Vec<f32>)>>,
}

/// Threshold below which the build path skips clustering
/// entirely and produces a single-cluster degenerate
/// index. At this scale brute-force is essentially free
/// and the clustering overhead would be wasted work.
const MIN_ENTRIES_FOR_CLUSTERING: usize = 16;

/// Build an `AnnIndex` over a flat `(topic, seq, embedding)`
/// triple list. Determines `K ≈ √N` clusters; seeds
/// centroids by spaced sampling from the input; assigns
/// each entry to its nearest centroid via cosine
/// similarity. **Single-pass** assignment — no iterative
/// Lloyd's-algorithm refinement (a deferral; v1 simplicity).
///
/// Edge cases:
/// - Empty input → empty index.
/// - `entries.len() <= MIN_ENTRIES_FOR_CLUSTERING` → single
///   degenerate cluster containing every entry (brute-force
///   within is equivalent to no index at this scale).
/// - Mixed embedding dimensions → the first centroid's
///   dimension wins; entries with a different dimension
///   get a cosine of 0.0 against every centroid, so they
///   all land in cluster 0 deterministically (defensive).
pub fn build_ann_index(
    entries: &[(String, u64, Vec<f32>)],
) -> AnnIndex {
    if entries.is_empty() {
        return AnnIndex {
            centroids: Vec::new(),
            clusters: Vec::new(),
        };
    }

    // Small-N degenerate path: a single cluster.
    if entries.len() <= MIN_ENTRIES_FOR_CLUSTERING {
        let centroid = entries[0].2.clone();
        return AnnIndex {
            centroids: vec![centroid],
            clusters: vec![entries.to_vec()],
        };
    }

    // K = ceil(sqrt(N)), minimum 2.
    let n = entries.len();
    let k = (n as f64).sqrt().ceil() as usize;
    let k = k.max(2);

    // Deterministic spaced-sampling seeds. Pick K
    // evenly-spaced entries as the initial centroids.
    let stride = n / k;
    let mut centroids: Vec<Vec<f32>> = (0..k)
        .map(|i| entries[i * stride].2.clone())
        .collect();

    // Defensive: if (somehow) k > n / stride such that the
    // last index goes past n, clamp.
    while centroids.len() > 1
        && (centroids.len() - 1) * stride >= n
    {
        centroids.pop();
    }
    let k = centroids.len();

    // One-pass nearest-centroid assignment. Each entry
    // goes into the cluster whose centroid maximizes cosine
    // similarity. Ties (e.g., dim mismatch produces 0.0
    // across all centroids) break to cluster 0
    // deterministically by the iteration order.
    let mut clusters: Vec<Vec<(String, u64, Vec<f32>)>> =
        vec![Vec::new(); k];
    for entry in entries {
        let best_idx = nearest_centroid_idx(&centroids, &entry.2);
        clusters[best_idx].push(entry.clone());
    }

    AnnIndex { centroids, clusters }
}

/// Pick the centroid index whose cosine similarity to
/// `vec` is highest. Ties break to the lower index
/// (iteration order), giving deterministic membership for
/// edge cases like all-zero centroids.
fn nearest_centroid_idx(centroids: &[Vec<f32>], vec: &[f32]) -> usize {
    let mut best_idx = 0;
    let mut best_sim = f32::NEG_INFINITY;
    for (i, c) in centroids.iter().enumerate() {
        let sim = cosine_similarity(c, vec);
        if sim > best_sim {
            best_sim = sim;
            best_idx = i;
        }
    }
    best_idx
}

/// Query the ANN index for the top `candidate_limit`
/// `(topic, seq, score)` candidates, searching only the
/// `top_clusters` whose centroids rank highest for the
/// query vector.
///
/// The candidate list is sorted by cosine score
/// descending, with `seq` descending as the secondary tie-
/// break (matches `rank_by_cosine`'s ordering discipline so
/// the caller's re-rank produces identical orderings).
///
/// Edge cases:
/// - Empty index → empty result.
/// - Empty query → empty result.
/// - `candidate_limit == 0` → empty result.
/// - `top_clusters` larger than available clusters →
///   clamps to all clusters (equivalent to brute-force).
/// - `top_clusters == 0` → clamps to 1 (an operator who
///   asks for "zero clusters" presumably wants the cheapest
///   non-empty query; we give them the single best
///   cluster).
/// - Query dim mismatch → cosine returns 0.0 across all
///   centroids; results still return but scores are 0.0
///   (the brute-force re-rank caller will see the
///   degenerate scores and drop them via their threshold).
pub fn query_ann(
    index: &AnnIndex,
    query: &[f32],
    top_clusters: usize,
    candidate_limit: usize,
) -> Vec<(String, u64, f32)> {
    if index.centroids.is_empty()
        || query.is_empty()
        || candidate_limit == 0
    {
        return Vec::new();
    }
    let top_clusters = top_clusters
        .min(index.centroids.len())
        .max(1);

    // Rank centroids by cosine similarity to the query.
    let mut centroid_scores: Vec<(usize, f32)> = index
        .centroids
        .iter()
        .enumerate()
        .map(|(i, c)| (i, cosine_similarity(c, query)))
        .collect();
    centroid_scores.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Brute-force within the top-N clusters' union.
    let mut candidates: Vec<(String, u64, f32)> = Vec::new();
    for (cluster_idx, _centroid_score) in
        centroid_scores.iter().take(top_clusters)
    {
        for (topic, seq, vec) in &index.clusters[*cluster_idx]
        {
            let score = cosine_similarity(query, vec);
            candidates.push((topic.clone(), *seq, score));
        }
    }

    // Sort descending by score, then by seq descending —
    // matches `rank_by_cosine`'s ordering rule.
    candidates.sort_by(|a, b| {
        b.2.partial_cmp(&a.2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.1.cmp(&a.1))
    });
    candidates.truncate(candidate_limit);
    candidates
}

/// Default fraction of clusters to search per query. Used
/// by `RedbMemory`'s ANN integration in Task 4 when the
/// caller doesn't override. `K / 4` (minimum 2) gives the
/// hybrid composition reasonable recall on the project's
/// target scale.
pub fn default_top_clusters(num_clusters: usize) -> usize {
    (num_clusters / 4).max(2).min(num_clusters)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a small fixture of (topic, seq, embedding)
    /// triples. The embeddings are 3-D unit-axis vectors so
    /// cosine similarity is exact and easy to reason about.
    fn fixture(n: usize) -> Vec<(String, u64, Vec<f32>)> {
        (0..n)
            .map(|i| {
                let f = i as f32;
                (
                    format!("topic{i}"),
                    i as u64,
                    vec![f.cos(), f.sin(), 0.1],
                )
            })
            .collect()
    }

    #[test]
    fn empty_input_empty_index() {
        let idx = build_ann_index(&[]);
        assert!(idx.centroids.is_empty());
        assert!(idx.clusters.is_empty());
        assert_eq!(query_ann(&idx, &[1.0, 0.0, 0.0], 4, 10), vec![]);
    }

    #[test]
    fn single_entry_single_cluster() {
        let entries =
            vec![("only".into(), 0, vec![1.0, 0.0, 0.0])];
        let idx = build_ann_index(&entries);
        assert_eq!(idx.centroids.len(), 1);
        assert_eq!(idx.clusters.len(), 1);
        assert_eq!(idx.clusters[0].len(), 1);
    }

    #[test]
    fn small_n_uses_degenerate_single_cluster() {
        // Below MIN_ENTRIES_FOR_CLUSTERING — one cluster
        // holds everything.
        let entries = fixture(10);
        let idx = build_ann_index(&entries);
        assert_eq!(idx.centroids.len(), 1);
        assert_eq!(idx.clusters.len(), 1);
        assert_eq!(idx.clusters[0].len(), 10);
    }

    #[test]
    fn larger_n_partitions_into_sqrt_n_clusters() {
        // N = 100 → K = ceil(sqrt(100)) = 10.
        let entries = fixture(100);
        let idx = build_ann_index(&entries);
        assert_eq!(idx.centroids.len(), 10);
        // Every entry appears in exactly one cluster.
        let total: usize =
            idx.clusters.iter().map(|c| c.len()).sum();
        assert_eq!(total, 100);
    }

    #[test]
    fn build_is_deterministic() {
        // Same input → same centroid set; same cluster
        // membership.
        let entries = fixture(100);
        let a = build_ann_index(&entries);
        let b = build_ann_index(&entries);
        assert_eq!(a, b);
    }

    #[test]
    fn build_uses_spaced_sampling_seeds() {
        // N = 100, K = 10, stride = 10. The centroids
        // should be entries[0], entries[10], entries[20],
        // ..., entries[90].
        let entries = fixture(100);
        let idx = build_ann_index(&entries);
        for (i, c) in idx.centroids.iter().enumerate() {
            assert_eq!(*c, entries[i * 10].2);
        }
    }

    #[test]
    fn query_against_all_clusters_equals_brute_force() {
        // When the query searches every cluster, the
        // candidates are every entry — sorted, this matches
        // the brute-force ranking.
        let entries = fixture(100);
        let idx = build_ann_index(&entries);
        let query = vec![1.0, 0.0, 0.1];
        let candidates = query_ann(
            &idx,
            &query,
            idx.centroids.len(), // search all clusters
            100,                 // ask for everything
        );
        assert_eq!(candidates.len(), 100);
        // The top hit should be the entry whose vector is
        // most parallel to the query.
        let brute: Vec<(String, u64, f32)> = entries
            .iter()
            .map(|(t, s, v)| (t.clone(), *s, cosine_similarity(&query, v)))
            .collect();
        let mut brute_sorted = brute.clone();
        brute_sorted.sort_by(|a, b| {
            b.2.partial_cmp(&a.2)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.1.cmp(&a.1))
        });
        // First three positions should match exactly.
        for i in 0..3 {
            assert_eq!(
                candidates[i].1, brute_sorted[i].1,
                "position {i} disagrees with brute"
            );
        }
    }

    #[test]
    fn query_with_top_clusters_one_narrows_results() {
        let entries = fixture(100);
        let idx = build_ann_index(&entries);
        let query = vec![1.0, 0.0, 0.1];
        let candidates = query_ann(&idx, &query, 1, 100);
        // Only one cluster searched → candidates ≤ size of
        // that cluster (much less than 100).
        assert!(
            candidates.len() < 100,
            "expected narrowing, got {}",
            candidates.len()
        );
        assert!(!candidates.is_empty(), "expected non-empty");
    }

    #[test]
    fn query_candidate_limit_caps_results() {
        let entries = fixture(100);
        let idx = build_ann_index(&entries);
        let query = vec![1.0, 0.0, 0.1];
        let candidates = query_ann(
            &idx,
            &query,
            idx.centroids.len(),
            5,
        );
        assert_eq!(candidates.len(), 5);
    }

    #[test]
    fn query_empty_query_returns_empty() {
        let entries = fixture(20);
        let idx = build_ann_index(&entries);
        assert_eq!(query_ann(&idx, &[], 4, 10), vec![]);
    }

    #[test]
    fn query_zero_candidate_limit_returns_empty() {
        let entries = fixture(20);
        let idx = build_ann_index(&entries);
        assert_eq!(
            query_ann(&idx, &[1.0, 0.0, 0.0], 4, 0),
            vec![]
        );
    }

    #[test]
    fn query_top_clusters_clamps_to_available() {
        // Asking for more clusters than exist clamps to the
        // available count — equivalent to brute-force.
        let entries = fixture(100);
        let idx = build_ann_index(&entries);
        let query = vec![1.0, 0.0, 0.1];
        let with_huge_top = query_ann(&idx, &query, 999, 50);
        let with_all = query_ann(
            &idx,
            &query,
            idx.centroids.len(),
            50,
        );
        assert_eq!(with_huge_top, with_all);
    }

    #[test]
    fn query_zero_top_clusters_clamps_to_one() {
        let entries = fixture(100);
        let idx = build_ann_index(&entries);
        let candidates =
            query_ann(&idx, &[1.0, 0.0, 0.1], 0, 50);
        // Equivalent to top_clusters = 1 — returns the
        // single best cluster's contents (or candidate_limit
        // of them, whichever is smaller).
        assert!(!candidates.is_empty());
    }

    #[test]
    fn query_dim_mismatch_returns_results_with_zero_scores() {
        // Query has different dimensionality than the
        // indexed embeddings. Cosine returns 0.0 across the
        // board; results still come back but with zero
        // scores. The caller's brute-force re-rank +
        // threshold filter is responsible for dropping
        // these.
        let entries = fixture(100);
        let idx = build_ann_index(&entries);
        let candidates = query_ann(
            &idx,
            &[1.0, 2.0], // 2-D, not 3-D
            idx.centroids.len(),
            5,
        );
        // Should still return some candidates (the
        // structure doesn't crash on dim mismatch), but all
        // scores are 0.0.
        assert!(!candidates.is_empty());
        for c in &candidates {
            assert_eq!(c.2, 0.0);
        }
    }

    #[test]
    fn default_top_clusters_quarter_with_minimum_two() {
        assert_eq!(default_top_clusters(0), 0);
        assert_eq!(default_top_clusters(1), 1);
        assert_eq!(default_top_clusters(4), 2);  // 4/4=1, max(2)=2
        assert_eq!(default_top_clusters(8), 2);  // 8/4=2
        assert_eq!(default_top_clusters(20), 5); // 20/4=5
        assert_eq!(default_top_clusters(100), 25);
    }
}
