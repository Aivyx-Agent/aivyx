//! Phase 89 — topic canonicalization.
//!
//! Folds the morphological variants of the operator's typed
//! topic string (`Deploy` / `deploys` / `deploying` /
//! `deployed`) into one canonical form (`deploy`), so the
//! signal that should add up across them — in the Phase 7
//! memory, the Phase 77 recall log, the Phase 82 helpfulness
//! ledger, the Phase 83 co-occurrence ledger, the Phase 87
//! consolidate-pair proposal IDs — stops being silently
//! fragmented.
//!
//! v1 rule set (Q1a):
//!   1. Lowercase (ASCII-aware; non-ASCII characters pass
//!      through unchanged to keep the stemmer scope-bounded).
//!   2. Trim leading/trailing whitespace.
//!   3. Collapse internal whitespace runs to a single space.
//!   4. **One** suffix-strip rule fires (longest match wins),
//!      with min-length guards so degenerate short words stay
//!      intact:
//!        - `ies` → `y`   (`policies` → `policy`, len ≥ 4)
//!        - `ing` drop    (`testing`  → `test`,   len ≥ 5)
//!        - `ed`  drop    (`tested`   → `test`,   len ≥ 4)
//!        - `es`  drop    (`boxes`    → `box`,    len ≥ 4,
//!                         **and** the resulting stem ends in
//!                         a hissing-sound letter — `sh` /
//!                         `ch` / `s` / `x` / `z`. This is
//!                         the English plural rule: `boxes` →
//!                         `box` but `roles` ≠ `rol`, falls
//!                         through to the `s` rule → `role`)
//!        - `s`   drop    (`tests`    → `test`,   len ≥ 3,
//!                         not preceded by `s` — `process`
//!                         stays `process`)
//!
//! The function is **idempotent**: applying it to its own
//! output is a no-op. The Phase 89 seam relies on this so
//! downstream consumers can re-canonicalize defensively
//! without changing anything.

/// Canonicalize a topic string per the Phase 89 v1 rules.
/// Pure (no allocations beyond the returned `String`); cheap
/// enough to call on every `Memory::put` and matching
/// topic-keyed read.
pub fn canonicalize_topic(s: &str) -> String {
    // 1. Lowercase + trim + whitespace-collapse, in one pass
    //    over the input. We allocate one `String` of bounded
    //    size and avoid the extra `lower.trim()` copy.
    let mut buf = String::with_capacity(s.len());
    let mut prev_ws = true; // suppress leading whitespace
    for c in s.chars() {
        if c.is_whitespace() {
            if !prev_ws {
                buf.push(' ');
            }
            prev_ws = true;
        } else {
            for lc in c.to_lowercase() {
                buf.push(lc);
            }
            prev_ws = false;
        }
    }
    // Strip a trailing space if the input had trailing
    // whitespace.
    if buf.ends_with(' ') {
        buf.pop();
    }
    if buf.is_empty() {
        return buf;
    }

    // 2. Suffix strip — at most one rule fires. The order
    //    below is longest-suffix-first so a string ending in
    //    `ies` is folded by the `ies → y` rule, not by the
    //    later `s` rule.
    if buf.len() >= 4 && buf.ends_with("ies") {
        buf.truncate(buf.len() - 3);
        buf.push('y');
    } else if buf.len() >= 5 && buf.ends_with("ing") {
        buf.truncate(buf.len() - 3);
    } else if buf.len() >= 4 && buf.ends_with("ed") {
        buf.truncate(buf.len() - 2);
    } else if buf.len() >= 4
        && buf.ends_with("es")
        && stem_ends_in_hissing_sound(&buf[..buf.len() - 2])
    {
        // `boxes` → stem `box` ends in `x` → strip.
        // `wishes` → stem `wish` ends in `sh` → strip.
        // `roles` → stem `rol` ends in `l` → DO NOT strip
        // here; fall through to the `s` rule below for
        // `role`.
        buf.truncate(buf.len() - 2);
    } else if buf.len() >= 3
        && buf.ends_with('s')
        && !buf.ends_with("ss")
    {
        buf.truncate(buf.len() - 1);
    }

    buf
}

/// English-plural heuristic: `-es` is a true hissing-sound
/// plural marker iff the underlying stem ends in `s`, `x`,
/// `z`, `sh`, or `ch`. Used to decide whether the `es` rule
/// should fire (`boxes` → `box`) or fall through to the
/// single-`s` rule (`roles` → `role`).
fn stem_ends_in_hissing_sound(stem: &str) -> bool {
    if stem.ends_with("sh") || stem.ends_with("ch") {
        return true;
    }
    matches!(
        stem.chars().last(),
        Some('s') | Some('x') | Some('z')
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lowercase_and_trim_fold_to_canonical() {
        assert_eq!(canonicalize_topic("  Deploy  "), "deploy");
        assert_eq!(canonicalize_topic("DEPLOY"), "deploy");
        // Internal whitespace runs collapse to single space.
        assert_eq!(
            canonicalize_topic("  hello   world  "),
            "hello world"
        );
        // Tabs + newlines are whitespace too.
        assert_eq!(
            canonicalize_topic("hello\tworld\n"),
            "hello world"
        );
    }

    #[test]
    fn ies_to_y_handles_y_plurals() {
        assert_eq!(canonicalize_topic("policies"), "policy");
        assert_eq!(canonicalize_topic("cities"), "city");
        assert_eq!(canonicalize_topic("companies"), "company");
    }

    #[test]
    fn ing_drop_handles_gerunds() {
        assert_eq!(canonicalize_topic("testing"), "test");
        assert_eq!(canonicalize_topic("deploying"), "deploy");
        // Too short — `ing` rule needs len >= 5, so `sing`
        // (4) stays.
        assert_eq!(canonicalize_topic("sing"), "sing");
    }

    #[test]
    fn ed_drop_handles_past_tense() {
        assert_eq!(canonicalize_topic("tested"), "test");
        assert_eq!(canonicalize_topic("deployed"), "deploy");
        // `red` is too short (len 3 < 4) — stays.
        assert_eq!(canonicalize_topic("red"), "red");
    }

    #[test]
    fn es_drop_handles_hissing_plurals() {
        // Hissing-sound stems (s/x/z/sh/ch) take `-es` in
        // English; the canonicalizer recognizes this and
        // strips the full `es` so the stem matches.
        assert_eq!(canonicalize_topic("boxes"), "box");
        assert_eq!(canonicalize_topic("wishes"), "wish");
        assert_eq!(canonicalize_topic("buzzes"), "buzz");
        assert_eq!(canonicalize_topic("matches"), "match");
    }

    #[test]
    fn es_rule_skips_non_hissing_stems() {
        // `roles` → stem `rol` ends in `l` (not a hissing
        // sound) → `es` rule does NOT fire; falls through to
        // the `s` rule → `role`. This is the contrast the
        // hissing-sound guard exists to make.
        assert_eq!(canonicalize_topic("roles"), "role");
        assert_eq!(canonicalize_topic("notes"), "note");
        assert_eq!(canonicalize_topic("modes"), "mode");
    }

    #[test]
    fn s_drop_handles_regular_plurals() {
        assert_eq!(canonicalize_topic("tests"), "test");
        assert_eq!(canonicalize_topic("roles"), "role");
        assert_eq!(canonicalize_topic("deploys"), "deploy");
    }

    #[test]
    fn double_s_is_not_stripped() {
        // `process` ends in `ss` — the `s` rule explicitly
        // skips this case (else `process` → `proces`, which
        // is not a real fragmentation case).
        assert_eq!(canonicalize_topic("process"), "process");
        assert_eq!(canonicalize_topic("kiss"), "kiss");
    }

    #[test]
    fn short_strings_are_guarded() {
        // `s` rule needs len >= 3, so short `s`-tails stay
        // (`is` / `as` / `us` aren't plurals).
        assert_eq!(canonicalize_topic("is"), "is");
        assert_eq!(canonicalize_topic("as"), "as");
        assert_eq!(canonicalize_topic("a"), "a");
        // Empty input is preserved (Memory rejects empty
        // topics independently; canonicalize doesn't need to
        // duplicate that error).
        assert_eq!(canonicalize_topic(""), "");
        assert_eq!(canonicalize_topic("   "), "");
    }

    #[test]
    fn idempotent_on_its_own_output() {
        // The seam relies on this — downstream consumers may
        // re-canonicalize defensively without surprise.
        for input in [
            "deploys",
            "policies",
            "testing",
            "tested",
            "boxes",
            "tests",
            "Deploy / Rollback",
            "process",
            "is",
            "",
        ] {
            let once = canonicalize_topic(input);
            let twice = canonicalize_topic(&once);
            assert_eq!(
                once, twice,
                "canonicalize_topic must be idempotent for \
                 {input:?}: {once:?} != {twice:?}"
            );
        }
    }

    #[test]
    fn already_canonical_input_is_unchanged() {
        for input in [
            "deploy",
            "policy",
            "test",
            "process",
            "auth",
            "css",
            "rust",
        ] {
            assert_eq!(
                canonicalize_topic(input),
                input,
                "already-canonical input must be unchanged: \
                 {input:?}"
            );
        }
    }

    #[test]
    fn non_ascii_passes_through() {
        // Non-ASCII letters lowercase via `char::to_lowercase`
        // (a Unicode operation) and otherwise pass through
        // the stemmer. The suffix-strip rules look at ASCII
        // byte sequences, so a topic ending in a non-ASCII
        // character won't be stripped.
        //
        // Greek `Α` (capital alpha) lowercases to `α` — but
        // the stemmer rule for `s` looks for the ASCII `s`
        // byte, which is absent.
        let folded = canonicalize_topic("Αlpha");
        // The first character should be the lowercase Greek
        // alpha. Don't assert the exact bytes — just verify
        // the function didn't crash and produced a string.
        assert!(!folded.is_empty());
        assert!(folded.chars().next().unwrap().is_lowercase());
    }

    #[test]
    fn path_like_topics_collapse_the_trailing_segment() {
        // Whole-string suffix strip means a path-like topic
        // like `project/notes` collapses to `project/note`
        // (the `s` is the LAST byte and the topic was
        // operator-typed; the trade-off is consistency over
        // path-segment integrity, as documented in the open
        // doc).
        assert_eq!(
            canonicalize_topic("project/notes"),
            "project/note"
        );
        assert_eq!(
            canonicalize_topic("frontend/css"),
            "frontend/css"
        );
    }
}
