//! Chapter Loom (LM.2) — BM25 lexical scoring over memory entries.
//!
//! Phase 98's `recall_hybrid` already fuses a keyword ranker with the
//! semantic one (see `aivyx-channel`'s `recall_fusion`), but its keyword
//! ranker is `Memory::search` — a case-folded `contains` substring scan
//! with no term weighting. A two-word query ranks a memory that merely
//! contains one *common* word the same as one that contains the rare
//! *discriminating* term. BM25 fixes that: it weights each query term by
//! its corpus rarity (IDF) and saturates repeated-term frequency, so the
//! memory carrying the rare codename / acronym / identifier rises to the
//! top — exactly the recall the embedding model tends to miss.
//!
//! This module is **pure and deterministic** — no I/O, no RNG, no new
//! dependency (matching the IVF / canonicalizer precedent). The
//! `Memory::lexical_search_scored` default method scores the live corpus
//! with it; both substrate impls share this one scorer.
//!
//! ## The scoring function
//!
//! For a document `D` and query `Q` (`N` docs, average length `avgdl`):
//!
//! ```text
//! score(D,Q) = Σ_{t∈Q}  IDF(t) · ( f(t,D)·(k1+1) )
//!                        ───────────────────────────────────
//!                        f(t,D) + k1·(1 − b + b·|D|/avgdl)
//!
//! IDF(t) = ln( 1 + (N − n(t) + 0.5) / (n(t) + 0.5) )
//! ```
//!
//! `f(t,D)` is the term frequency, `n(t)` the document frequency. The
//! `1 +` form of IDF is the **BM25+ non-negative** variant — a plain BM25
//! IDF goes negative for terms in more than half the docs, which on a
//! small per-operator corpus would let a common word *subtract* score.
//! The non-negative form keeps every term a (weak) positive signal.

/// Standard BM25 term-frequency saturation parameter. `1.2` is the
/// canonical default; higher values make repeated terms count more
/// linearly.
pub const BM25_K1: f32 = 1.2;

/// Standard BM25 length-normalization parameter. `0.75` is the canonical
/// default; `0` disables length normalization, `1` applies it fully.
pub const BM25_B: f32 = 0.75;

/// Tokenize text into lowercased alphanumeric terms. Splits on any
/// non-alphanumeric character (Unicode-aware), lowercases (ASCII-fold,
/// consistent with the topic canonicalizer's scope), and drops empties.
/// Deterministic and allocation-simple — the corpus scale (≤ ~100K
/// entries) doesn't warrant a stemmer or stop-word list in v1.
pub fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .collect()
}

/// Tokenize a memory entry as `topic` terms followed by `body` terms, so
/// a query word in the topic counts as a hit just as `search` treats
/// topic and body alike.
pub fn tokenize_entry(topic: &str, body: &str) -> Vec<String> {
    let mut terms = tokenize(topic);
    terms.extend(tokenize(body));
    terms
}

/// BM25-rank a pre-tokenized corpus against a pre-tokenized query.
///
/// `docs[i]` is document `i`'s term list (see [`tokenize_entry`]).
/// Returns `(doc_index, score)` for every document with a **positive**
/// score (i.e. it contains at least one query term), sorted by score
/// descending then `doc_index` ascending for deterministic ties,
/// truncated to `limit`.
///
/// Edge cases: empty `docs`, empty `query`, or `limit == 0` → empty.
pub fn bm25_rank(
    docs: &[Vec<String>],
    query: &[String],
    k1: f32,
    b: f32,
    limit: usize,
) -> Vec<(usize, f32)> {
    if docs.is_empty() || query.is_empty() || limit == 0 {
        return Vec::new();
    }
    let n_docs = docs.len() as f32;
    let avgdl = {
        let total: usize = docs.iter().map(|d| d.len()).sum();
        // Guard the all-empty-documents corpus (avgdl would be 0 →
        // division by zero in the length norm). 1.0 makes the norm a
        // no-op; such docs score 0 anyway (no terms to match).
        let avg = total as f32 / n_docs;
        if avg > 0.0 { avg } else { 1.0 }
    };

    // Unique query terms in sorted order — dedup so a repeated query
    // word isn't double-counted, sorted so the per-doc score sum is
    // accumulated in a fixed (deterministic) term order.
    let mut q_terms: Vec<&String> = query.iter().collect();
    q_terms.sort();
    q_terms.dedup();

    // Document frequency n(t) and IDF(t) per query term.
    let idf: Vec<(&String, f32)> = q_terms
        .iter()
        .map(|t| {
            let n_t = docs.iter().filter(|d| d.contains(*t)).count() as f32;
            let idf = (1.0 + (n_docs - n_t + 0.5) / (n_t + 0.5)).ln();
            (*t, idf)
        })
        .collect();

    let mut scored: Vec<(usize, f32)> = Vec::new();
    for (i, doc) in docs.iter().enumerate() {
        let dl = doc.len() as f32;
        let norm = k1 * (1.0 - b + b * dl / avgdl);
        let mut score = 0.0_f32;
        for (term, term_idf) in &idf {
            let f = doc.iter().filter(|w| *w == *term).count() as f32;
            if f > 0.0 {
                score += term_idf * (f * (k1 + 1.0)) / (f + norm);
            }
        }
        if score > 0.0 {
            scored.push((i, score));
        }
    }

    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    scored.truncate(limit);
    scored
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_lowercases_splits_and_drops_empties() {
        let t = tokenize("Deploy the K8s-Cluster, now!");
        assert_eq!(t, vec!["deploy", "the", "k8s", "cluster", "now"]);
        assert!(tokenize("   ,.;  ").is_empty());
    }

    #[test]
    fn tokenize_entry_prepends_topic_terms() {
        let t = tokenize_entry("deploy", "ship it");
        assert_eq!(t, vec!["deploy", "ship", "it"]);
    }

    #[test]
    fn empty_query_or_corpus_or_limit_is_empty() {
        let docs = vec![tokenize("hello world")];
        assert!(bm25_rank(&docs, &[], BM25_K1, BM25_B, 10).is_empty());
        assert!(bm25_rank(&[], &tokenize("hello"), BM25_K1, BM25_B, 10).is_empty());
        assert!(bm25_rank(&docs, &tokenize("hello"), BM25_K1, BM25_B, 0).is_empty());
    }

    /// The core BM25 claim: a rare term discriminates. "the" appears in
    /// every doc (low IDF); "kubernetes" in one (high IDF). A query for
    /// the rare term ranks its lone carrier first; only that doc scores.
    #[test]
    fn rare_term_dominates_common_term() {
        let docs = vec![
            tokenize("the cat sat on the mat"),
            tokenize("the dog ran in the park"),
            tokenize("the kubernetes cluster the deploy"),
        ];
        let out = bm25_rank(&docs, &tokenize("kubernetes"), BM25_K1, BM25_B, 10);
        assert_eq!(out.len(), 1, "only doc 2 contains the rare term");
        assert_eq!(out[0].0, 2);

        // The ubiquitous term still scores (non-negative IDF) but spreads
        // across all three docs rather than discriminating.
        let common = bm25_rank(&docs, &tokenize("the"), BM25_K1, BM25_B, 10);
        assert_eq!(common.len(), 3);
        // And its top score is far below the rare term's.
        assert!(out[0].1 > common[0].1 * 2.0, "rare {} vs common {}", out[0].1, common[0].1);
    }

    /// A doc matching more distinct query terms outranks one matching
    /// fewer, and only matching docs appear.
    #[test]
    fn more_query_terms_matched_ranks_higher() {
        let docs = vec![
            tokenize("alpha beta gamma"),  // matches both
            tokenize("alpha only here"),   // matches one
            tokenize("nothing relevant"),  // matches none
        ];
        let out = bm25_rank(&docs, &tokenize("alpha beta"), BM25_K1, BM25_B, 10);
        assert_eq!(out.len(), 2, "doc 2 matches nothing");
        assert_eq!(out[0].0, 0, "two-term match wins");
        assert_eq!(out[1].0, 1);
    }

    #[test]
    fn limit_truncates_and_is_deterministic() {
        let docs = vec![
            tokenize("rust memory graph"),
            tokenize("rust async tokio"),
            tokenize("rust trait object"),
        ];
        let q = tokenize("rust");
        let out = bm25_rank(&docs, &q, BM25_K1, BM25_B, 2);
        assert_eq!(out.len(), 2);
        // All three have one "rust" and equal length → equal score →
        // tie-break by index asc → docs 0,1.
        assert_eq!(out[0].0, 0);
        assert_eq!(out[1].0, 1);
        for _ in 0..5 {
            assert_eq!(bm25_rank(&docs, &q, BM25_K1, BM25_B, 2), out);
        }
    }

    /// Term-frequency saturation: a doc with the query term twice scores
    /// higher than the same-length doc with it once, but sub-linearly
    /// (k1 saturation), not 2×.
    #[test]
    fn term_frequency_saturates() {
        let docs = vec![
            tokenize("deploy deploy filler filler"), // tf=2, len 4
            tokenize("deploy filler filler filler"), // tf=1, len 4
        ];
        let out = bm25_rank(&docs, &tokenize("deploy"), BM25_K1, BM25_B, 10);
        assert_eq!(out[0].0, 0, "tf=2 outranks tf=1");
        assert!(out[0].1 > out[1].1);
        assert!(out[0].1 < out[1].1 * 2.0, "saturated, not linear");
    }
}
