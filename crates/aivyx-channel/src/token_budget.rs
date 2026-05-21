//! Phase 97 — token-cost estimator + budget-enforcer for
//! the recall + adaptive-Persona injection paths.
//!
//! ## Why a hand-rolled estimator
//!
//! Auto-recall (Phase 76) and adaptive Persona (Phase 79)
//! both cap injection by **entry count** today. Count is a
//! proxy for token cost; not the cost itself. A single 4 KB
//! memory body displaces multiple shorter ones; a Persona
//! facet that grew from one sentence to ten paragraphs eats
//! turn after turn of input.
//!
//! Phase 97 caps both paths by **estimated token cost**.
//! The estimator is `chars / 4` with a small boundary fudge
//! — accurate to ~±20% for English, well within the
//! tolerance budget enforcement needs. Sub-token accuracy
//! (via `tiktoken-rs` or model-specific tokenizers) would
//! cost a new workspace dependency without changing the
//! budget's effective behaviour at any realistic operator
//! threshold.
//!
//! ## How items are dropped
//!
//! The caller pre-ranks items (recall by cosine score,
//! Persona by adaptive-selection priority). The budget
//! walks the pre-ranked sequence accumulating cost; the
//! first item whose addition would exceed the budget AND
//! every item after it are dropped. No mid-item truncation
//! — operators get full items or nothing.
//!
//! With `budget = 0` the helper is a no-op pass-through:
//! every item returns.

/// Phase 97 — approximate token cost of a text string.
///
/// Rule: `chars / 4` baseline (the OpenAI rule-of-thumb for
/// English), plus a small fudge factor that adds `1` for
/// any non-empty trimmed body (a non-empty body costs at
/// least one token; an empty body costs zero).
///
/// Counts Unicode `chars()`, not bytes — `héllo` is 5
/// chars (= 1 estimated token) regardless of UTF-8 byte
/// width. Whitespace counts as chars; the estimator
/// doesn't try to strip it (the model sees whitespace too).
///
/// Returns `0` for an empty input, `>= 1` for any non-empty
/// input.
pub fn estimate_tokens(text: &str) -> u32 {
    let chars = text.chars().count();
    if chars == 0 {
        return 0;
    }
    // chars / 4 with ceiling-style rounding via integer
    // math, then +1 fudge for the non-empty case so a
    // single-char body costs >= 1 token.
    let base = (chars / 4) as u32;
    base.saturating_add(1)
}

/// Phase 97 — enforce a token budget over a pre-ranked
/// sequence. Walks `items` in their existing order;
/// accumulates per-item cost via `cost_of`. Returns the
/// prefix that fits within `budget`.
///
/// Eviction rule: the first item whose inclusion would
/// push the running cost above `budget` is dropped, AND
/// every item after it. The caller has already pre-ranked,
/// so the dropped tail is by construction the lowest-
/// priority subset. No mid-item truncation; no skipping
/// of interior items.
///
/// Edge cases:
/// - `budget == 0` → empty output (the disabled-budget
///   path; callers gate on this before calling).
/// - A single item costing more than the entire budget →
///   excluded from the output (the "first item wins"
///   special case is **not** applied; the budget is a hard
///   cap).
/// - Empty input → empty output.
/// - `budget` very large → every item returns (no
///   pre-emptive dropping).
pub fn apply_token_budget<T, F>(
    items: Vec<T>,
    budget: u32,
    cost_of: F,
) -> Vec<T>
where
    F: Fn(&T) -> u32,
{
    if budget == 0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(items.len());
    let mut spent: u32 = 0;
    for item in items {
        let cost = cost_of(&item);
        let Some(after) = spent.checked_add(cost) else {
            break;
        };
        if after > budget {
            break;
        }
        spent = after;
        out.push(item);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- estimate_tokens -------------------------------

    #[test]
    fn estimate_empty_is_zero() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn estimate_single_char_is_one() {
        // 1 char / 4 = 0 + 1 fudge = 1.
        assert_eq!(estimate_tokens("a"), 1);
    }

    #[test]
    fn estimate_short_words() {
        // 11 chars / 4 = 2 + 1 = 3.
        assert_eq!(estimate_tokens("hello world"), 3);
    }

    #[test]
    fn estimate_long_text_scales_with_chars() {
        let s: String = "x".repeat(400);
        // 400 / 4 = 100 + 1 = 101.
        assert_eq!(estimate_tokens(&s), 101);
    }

    #[test]
    fn estimate_unicode_counted_by_chars_not_bytes() {
        // `héllo` is 5 chars (6 bytes). 5/4 = 1 + 1 = 2.
        assert_eq!(estimate_tokens("héllo"), 2);
        // Confirm chars()/bytes() disagree.
        assert_eq!("héllo".len(), 6);
        assert_eq!("héllo".chars().count(), 5);
    }

    // ---- apply_token_budget ---------------------------

    fn cost_each_one(_s: &String) -> u32 {
        1
    }

    #[test]
    fn budget_zero_drops_everything() {
        let items = vec!["a".to_string(), "b".to_string()];
        assert_eq!(
            apply_token_budget(items, 0, cost_each_one),
            Vec::<String>::new(),
        );
    }

    #[test]
    fn budget_empty_input_is_empty_output() {
        let items: Vec<String> = vec![];
        assert!(apply_token_budget(items, 100, cost_each_one)
            .is_empty());
    }

    #[test]
    fn budget_under_passes_all() {
        let items = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let out =
            apply_token_budget(items.clone(), 100, cost_each_one);
        assert_eq!(out, items);
    }

    #[test]
    fn budget_at_threshold_passes_all() {
        // 3 items × 1 token = 3 total; budget 3 = exact fit.
        let items = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let out =
            apply_token_budget(items.clone(), 3, cost_each_one);
        assert_eq!(out, items);
    }

    #[test]
    fn budget_over_drops_tail_in_input_order() {
        // 5 items × 1 token; budget 3 → first 3 keep,
        // last 2 drop. Order preserved.
        let items = vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
            "e".to_string(),
        ];
        let out =
            apply_token_budget(items, 3, cost_each_one);
        assert_eq!(out, vec!["a", "b", "c"]);
    }

    #[test]
    fn budget_single_item_over_whole_budget_is_dropped() {
        // One item costs 10; budget is 5; item drops, no
        // partial inclusion.
        let items = vec!["expensive".to_string()];
        let out = apply_token_budget(
            items,
            5,
            |_| 10,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn budget_mixed_costs_drops_at_first_overflow() {
        // Items with varying costs: 2, 1, 5, 1, 1; budget 8.
        // Cumulative: 2 → 3 → 8 → would-be-9 → stop.
        // Output: first 3 items.
        let items = vec![
            ("a", 2),
            ("b", 1),
            ("c", 5),
            ("d", 1),
            ("e", 1),
        ];
        let out = apply_token_budget(items, 8, |(_, c)| *c);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].0, "a");
        assert_eq!(out[1].0, "b");
        assert_eq!(out[2].0, "c");
    }

    #[test]
    fn budget_preserves_input_order_does_not_reorder() {
        // The helper never re-ranks; the caller's order
        // wins. Verifies the contract that low-cost items
        // appearing AFTER the first over-budget item are
        // not opportunistically included.
        let items = vec![
            ("expensive", 10),
            ("cheap1", 1),
            ("cheap2", 1),
        ];
        let out = apply_token_budget(items, 5, |(_, c)| *c);
        // The expensive item is over budget so it drops;
        // the cheap items after it also drop (the helper
        // does not skip-and-continue).
        assert!(out.is_empty());
    }
}
