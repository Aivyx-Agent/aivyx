//! Phase 116 Task 2 — Keyword extraction primitive.
//!
//! Q1a's pattern-key shape. Pure, deterministic, zero LLM
//! cost. Given a user input string, produce:
//!
//! 1. A `Vec<String>` of the K most-distinctive alphanumeric
//!    tokens (lowercased, stopword-filtered, length-ordered).
//! 2. A `String` ledger key that's stable across permutations
//!    (sorted lexicographically, `|`-joined).
//!
//! ## Why keyword-sets and not embeddings
//!
//! The Q1b embedding-similarity option was rejected at sign-off
//! because it would add an embedding-call cost per turn and
//! require operators to have `[embedding]` configured. Keyword-
//! sets cost a few microseconds per turn and work for every
//! operator regardless of provider posture. Phase 95
//! skip-when-idle's precedent for cheap-deterministic-signal
//! substrate.
//!
//! ## Why length-ordered top-K
//!
//! Longer tokens are more distinctive than shorter ones. After
//! stopword filtering and the `min_token_length = 3` cutoff,
//! the longest remaining tokens (e.g. `"deployment"`,
//! `"benchmark"`) carry more semantic signal than the shortest
//! (`"set"`, `"run"`). Length-ordered top-K trims noise while
//! preserving the operator-readable keywords that actually
//! discriminate one turn pattern from another.

/// Minimum token length kept after stopword filtering. Tokens
/// shorter than this (e.g. `"a"`, `"to"`, `"if"`) get dropped
/// even if they're not on the stopword list — they don't
/// carry per-turn discriminating signal.
const MIN_TOKEN_LENGTH: usize = 3;

/// Built-in stopword set. ~60 common English words that
/// appear in most turns and carry no per-turn signal. Embedded
/// rather than externalized because the operator should never
/// need to tune this — the goal is "drop the words that show
/// up in every turn."
const STOPWORDS: &[&str] = &[
    // articles + determiners
    "the", "a", "an", "this", "that", "these", "those", "some", "any",
    "all", "each", "every", "no",
    // pronouns
    "i", "you", "he", "she", "it", "we", "they", "me", "him", "her",
    "us", "them", "my", "your", "his", "its", "our", "their",
    // be / have / do
    "is", "are", "was", "were", "be", "been", "being", "am",
    "have", "has", "had", "having", "do", "does", "did", "doing",
    // modals + auxiliaries
    "will", "would", "shall", "should", "can", "could", "may", "might",
    "must", "ought",
    // common prepositions
    "of", "in", "on", "at", "to", "from", "with", "by", "for", "as",
    "into", "onto", "upon", "about", "over", "under",
    // conjunctions
    "and", "or", "but", "if", "then", "else", "so", "because",
    "while", "when", "where", "what", "which", "who", "how", "why",
    "here", "there",
    // other high-frequency
    "not", "no", "yes", "ok", "okay",
];

/// Phase 116 — return the top-`max` keywords from a user
/// input. Output is length-ordered (longest first) so the
/// caller can take the top-K most distinctive tokens.
///
/// Algorithm:
/// 1. Lowercase the input.
/// 2. Split on non-alphanumeric characters.
/// 3. Drop tokens shorter than [`MIN_TOKEN_LENGTH`].
/// 4. Drop tokens in [`STOPWORDS`].
/// 5. Deduplicate (preserving first-seen order).
/// 6. Sort by length descending (ties broken by lex ascending
///    for determinism).
/// 7. Take the first `max`.
pub fn extract_keywords(input: &str, max: usize) -> Vec<String> {
    if max == 0 {
        return Vec::new();
    }
    let lowered = input.to_ascii_lowercase();
    let raw: Vec<String> = lowered
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| t.len() >= MIN_TOKEN_LENGTH)
        .filter(|t| !STOPWORDS.contains(t))
        .map(str::to_string)
        .collect();
    // Dedup preserving first-seen order.
    let mut seen: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    let mut unique: Vec<String> = Vec::new();
    for tok in raw {
        if seen.insert(tok.clone()) {
            unique.push(tok);
        }
    }
    // Sort by length desc, then lex asc.
    unique.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
    unique.truncate(max);
    unique
}

/// Phase 116 — produce a deterministic ledger-key string from
/// a user input. The key is the top-`max` keywords sorted
/// lexicographically and joined with `|`. Identical key for
/// turns that share the same top-K keyword set regardless of
/// original order in the user input.
///
/// Returns the empty string when no keywords survive filtering
/// (e.g. a turn that's purely stopwords + short tokens). The
/// ledger lookup treats the empty key as "no matched signal"
/// — that's the same fail-safe as having no entries.
pub fn keyword_key(input: &str, max: usize) -> String {
    let mut kw = extract_keywords(input, max);
    kw.sort();
    kw.join("|")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_drops_stopwords_and_short_tokens() {
        let kw = extract_keywords("the agent should research the topic", 10);
        // "the", "should" stopwords; "the" appears twice.
        // "agent" (5), "research" (8), "topic" (5) survive.
        assert!(kw.contains(&"research".to_string()));
        assert!(kw.contains(&"agent".to_string()));
        assert!(kw.contains(&"topic".to_string()));
        assert!(!kw.contains(&"the".to_string()));
        assert!(!kw.contains(&"should".to_string()));
    }

    #[test]
    fn extract_drops_tokens_shorter_than_minimum_length() {
        let kw = extract_keywords("go to it", 10);
        // "go" (2), "to" (stopword), "it" (stopword) — all dropped.
        assert!(kw.is_empty());
    }

    #[test]
    fn extract_is_case_insensitive() {
        let kw = extract_keywords("Research Code Repository", 10);
        assert!(kw.contains(&"research".to_string()));
        assert!(kw.contains(&"repository".to_string()));
        assert!(kw.contains(&"code".to_string()));
    }

    #[test]
    fn extract_dedups_repeated_tokens() {
        let kw = extract_keywords("research research research code", 10);
        assert_eq!(kw.iter().filter(|t| *t == "research").count(), 1);
        assert!(kw.contains(&"code".to_string()));
    }

    #[test]
    fn extract_sorts_by_length_descending() {
        let kw = extract_keywords("rust code deployment", 10);
        // deployment (10), code (4), rust (4) → deployment, code, rust
        // (code before rust because length tie → lex ascending).
        assert_eq!(kw, vec!["deployment", "code", "rust"]);
    }

    #[test]
    fn extract_respects_max_parameter() {
        let kw = extract_keywords(
            "research deploy build configure test verify analyze report",
            3,
        );
        assert_eq!(kw.len(), 3);
        // Longest three: configure (9), research (8), analyze (7).
        assert_eq!(kw, vec!["configure", "research", "analyze"]);
    }

    #[test]
    fn extract_zero_max_returns_empty() {
        let kw = extract_keywords("research the code", 0);
        assert!(kw.is_empty());
    }

    #[test]
    fn extract_handles_punctuation_and_special_chars() {
        let kw = extract_keywords(
            "research-the_repo's API @ https://github.com/aivyx",
            10,
        );
        assert!(kw.contains(&"research".to_string()));
        // "the" gets dropped (stopword); "repo" survives.
        assert!(kw.contains(&"repo".to_string()));
        assert!(kw.contains(&"github".to_string()));
        assert!(kw.contains(&"com".to_string()));
        assert!(kw.contains(&"api".to_string()));
        assert!(kw.contains(&"aivyx".to_string()));
        assert!(!kw.contains(&"the".to_string()));
    }

    #[test]
    fn extract_empty_input_returns_empty() {
        assert!(extract_keywords("", 10).is_empty());
        assert!(extract_keywords("   ", 10).is_empty());
        assert!(extract_keywords("...!!!???", 10).is_empty());
    }

    // ----- keyword_key -----

    #[test]
    fn keyword_key_is_lex_sorted_and_pipe_joined() {
        let key = keyword_key("rust code deployment", 10);
        // Lex sort of (rust, code, deployment) → code, deployment, rust.
        assert_eq!(key, "code|deployment|rust");
    }

    #[test]
    fn keyword_key_is_stable_across_input_order() {
        let a = keyword_key("rust deployment code", 10);
        let b = keyword_key("code rust deployment", 10);
        let c = keyword_key("deployment code rust", 10);
        assert_eq!(a, b);
        assert_eq!(b, c);
    }

    #[test]
    fn keyword_key_returns_empty_string_for_no_keywords() {
        assert_eq!(keyword_key("", 10), "");
        assert_eq!(keyword_key("the a or but", 10), "");
        assert_eq!(keyword_key("hi", 10), ""); // below min length
    }

    #[test]
    fn keyword_key_respects_max_before_sorting() {
        // Take top-2 by length, then lex-sort.
        // Tokens by length: deployment(10), research(8), build(5), test(4).
        // Top-2 = [deployment, research]; lex-sorted = [deployment, research].
        let key = keyword_key("test build research deployment", 2);
        assert_eq!(key, "deployment|research");
    }

    #[test]
    fn keyword_key_handles_unicode_in_input_by_dropping_non_ascii() {
        // Non-ASCII chars treated as separators (alphanumeric is
        // ASCII-only per the doc). The ASCII tokens around the
        // emoji survive.
        let key = keyword_key("research the code 🚀 deployment", 10);
        assert_eq!(key, "code|deployment|research");
    }

    #[test]
    fn stopwords_list_has_expected_coverage() {
        // Sanity check on a few canonical stopwords every test
        // assumed are present.
        for w in ["the", "a", "is", "with", "and"] {
            assert!(
                STOPWORDS.contains(&w),
                "stopword list missing canonical word: {w}"
            );
        }
    }
}
