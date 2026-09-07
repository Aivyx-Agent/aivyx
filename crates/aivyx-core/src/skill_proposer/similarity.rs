//! Phase 120 Task 2 — Token-set Jaccard similarity for tool/skill
//! name matching.
//!
//! Lifted from Phase 112's `aivyx-channel/src/skill_auto_proposer.rs`
//! (the Q4b fuzzy-match pre-filter for the skill auto-proposer) into
//! `aivyx-core` so the Phase 120 planner-side tool-name recovery
//! path can share the same primitive. The `aivyx-channel` site stays
//! as a `pub use` re-export so every existing caller (and every test
//! in `skill_auto_proposer_e2e.rs`) keeps working unchanged.
//!
//! ## Algorithm
//!
//! 1. Lowercase both inputs.
//! 2. Replace any non-alphanumeric character with `-`.
//! 3. Compute the Jaccard similarity of the resulting token sets
//!    (split on `-`).
//!
//! Pure function; no allocation beyond the two token sets. Cheap
//! enough to fire on every candidate without measurable cost.
//!
//! ## What this catches (and what it doesn't)
//!
//! Catches the obvious cases:
//! - `memory.gc` vs `memory_gc` (separator normalization)
//! - `Memory.GC` vs `memory.gc` (case insensitivity)
//! - `fs_read` vs `fs.read` (Phase 120's most-observed
//!   local-LLM hallucination — qwen3.6:27b emits `fs_read`
//!   when the registered tool is `fs.read`)
//! - `research-topic` vs `topic-research` (token-reorder)
//! - `aivyx-mcp-recipes` vs `mcp-recipes-aivyx`
//!
//! Does NOT catch deep semantic similarity — those land on the
//! LLM-judge `is_duplicate_of` path (Phase 112 Q4b stage 2 for the
//! skill auto-proposer; Phase 120 doesn't need that surface because
//! the bad name comes from a hallucinating model, not an LLM-judge
//! verdict).

/// Compute a normalized title similarity in `[0.0, 1.0]` between
/// two names. See module docs for the algorithm and the
/// catches/non-catches sets.
///
/// Returns `1.0` when both inputs are empty (degenerate but
/// well-defined). Returns `0.0` when the tokenized union is empty
/// after normalization but one input had content (numerator-zero
/// case).
pub fn title_similarity(a: &str, b: &str) -> f32 {
    fn tokens(s: &str) -> std::collections::HashSet<String> {
        s.to_ascii_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|t| !t.is_empty())
            .map(|t| t.to_string())
            .collect()
    }
    let ta = tokens(a);
    let tb = tokens(b);
    if ta.is_empty() && tb.is_empty() {
        return 1.0;
    }
    let intersection = ta.intersection(&tb).count();
    let union = ta.union(&tb).count();
    if union == 0 {
        return 0.0;
    }
    intersection as f32 / union as f32
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
//
// The Phase 112 in-module tests in `aivyx-channel/src/skill_auto_proposer.rs`
// stay in place against the `pub use` re-export so the existing
// coverage doesn't lose its current home. The tests below cover the
// Phase 120 hallucination cases the function NEWLY targets — the
// `fs_read` vs `fs.read` and `web_fetch` vs `web.fetch`
// observations from the phase-99-local-builds memory.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_inputs_score_one() {
        assert_eq!(title_similarity("fs.read", "fs.read"), 1.0);
    }

    #[test]
    fn both_empty_inputs_score_one() {
        // Degenerate but well-defined — tokenized union is empty
        // on both sides; treated as a perfect "match" so the
        // caller decides what to do with empty-vs-empty.
        assert_eq!(title_similarity("", ""), 1.0);
    }

    #[test]
    fn separator_normalization_catches_underscore_hallucination() {
        // Phase 120's most-observed case: qwen3.6 emits `fs_read`
        // when the registered tool is `fs.read`. Both tokenize
        // to {"fs", "read"} → Jaccard 1.0.
        assert_eq!(title_similarity("fs_read", "fs.read"), 1.0);
        assert_eq!(title_similarity("web_fetch", "web.fetch"), 1.0);
        assert_eq!(title_similarity("git_status", "git.status"), 1.0);
    }

    #[test]
    fn case_normalization() {
        assert_eq!(title_similarity("FS.Read", "fs.read"), 1.0);
        assert_eq!(title_similarity("FS_READ", "fs.read"), 1.0);
    }

    #[test]
    fn partial_overlap_returns_jaccard() {
        // {memory, gc} ∩ {memory, gcollect} = {memory}; union = 3;
        // Jaccard = 1/3.
        let sim = title_similarity("memory.gc", "memory.gcollect");
        assert!((sim - 1.0 / 3.0).abs() < 1e-6, "expected 1/3, got {sim}");
    }

    #[test]
    fn fully_disjoint_inputs_score_zero() {
        assert_eq!(title_similarity("fs.read", "web.fetch"), 0.0);
        assert_eq!(title_similarity("do.this", "do_the_thing"), {
            // {do, this} ∩ {do, the, thing} = {do}; union = 4 →
            // 1/4.
            1.0 / 4.0
        });
    }

    #[test]
    fn token_reorder_scores_one() {
        // The Phase 112 token-reorder case still works for the
        // Phase 120 hallucination shapes.
        assert_eq!(title_similarity("research-topic", "topic-research"), 1.0);
    }

    #[test]
    fn pure_punctuation_input_against_real_name_scores_zero() {
        // A model emitting nothing-but-separators is degenerate.
        // The function returns 0.0 (union has the real-name tokens
        // but intersection is empty) — caller treats this as
        // "no fuzzy match found."
        let sim = title_similarity("....", "fs.read");
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn empty_one_side_against_populated_side_scores_zero() {
        assert_eq!(title_similarity("", "fs.read"), 0.0);
        assert_eq!(title_similarity("fs.read", ""), 0.0);
    }
}
