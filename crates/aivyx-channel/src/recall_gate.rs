//! Phase 90 — heuristic recall gate.
//!
//! The third move in the input-quality arc: where Phase 86
//! sharpened *what* gets embedded (the conversation window)
//! and Phase 89 sharpened *how* signals key (canonicalization),
//! Phase 90 sharpens **when** recall fires at all.
//!
//! For 89 phases both auto-recall (Phase 76) and adaptive
//! Persona selection (Phase 79) fired on every conversational
//! turn — including turns where the user message is a single-
//! token acknowledgment (`ok` / `thanks` / `yes` / `cool`) that
//! cannot meaningfully steer recall or facet selection. The
//! bare-message embed on those turns is essentially a random
//! vector that pollutes the ranker; the recall block / adaptive
//! Persona selection injected on top of that ranker is noise
//! the planner has to defend against.
//!
//! Phase 90 adds the smallest possible gate: a **length-based
//! check** at the top of both relevance hooks that skips the
//! embed (and everything downstream) when the trimmed user
//! message is shorter than the operator-tunable
//! `recall_gate_min_chars` threshold. Both providers (Phase 76
//! `SemanticMemoryContext::recall` + Phase 79
//! `PersonaContextRefiner::refine`) call this function;
//! returning `true` short-circuits both to `None`, which is the
//! existing best-effort fallback contract.
//!
//! The default is `0` (gate disabled = byte-identical to
//! pre-Phase-90); the project's 89-phase
//! behaviour-change-is-opt-in discipline.

/// Phase 90 — decide whether to gate the recall + adaptive
/// Persona work for a turn whose user message is `text`.
///
/// Rule:
/// - `min_chars == 0` → never gate (opt-out / pre-Phase-90
///   default).
/// - Otherwise, trim whitespace and count Unicode characters
///   (not bytes — a 5-character word like `héllo` is 6 bytes
///   but 5 chars, and the operator's threshold is about
///   semantic message length, not encoding size). If the
///   count is **strictly less than** `min_chars`, return
///   `true` (gate the turn).
///
/// Whitespace-only input gates regardless of the threshold
/// (since the trimmed character count is zero); the providers
/// fall through to their existing best-effort `None`
/// short-circuit.
pub fn should_gate_recall(text: &str, min_chars: usize) -> bool {
    if min_chars == 0 {
        return false;
    }
    text.trim().chars().count() < min_chars
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `min_chars = 0` is the opt-out: every turn passes,
    /// regardless of input. This is the byte-identical-to-
    /// pre-Phase-90 path.
    #[test]
    fn min_chars_zero_never_gates() {
        assert!(!should_gate_recall("", 0));
        assert!(!should_gate_recall(" ", 0));
        assert!(!should_gate_recall("ok", 0));
        assert!(!should_gate_recall("a", 0));
        assert!(!should_gate_recall(
            "how do I deploy", 0
        ));
    }

    /// Exact threshold boundary: `min_chars = 4` gates
    /// strings of trimmed length 0, 1, 2, 3 and passes
    /// strings of trimmed length 4+.
    #[test]
    fn threshold_boundary_is_strictly_less_than() {
        assert!(should_gate_recall("", 4));
        assert!(should_gate_recall("a", 4));
        assert!(should_gate_recall("ok", 4));
        assert!(should_gate_recall("yes", 4));
        assert!(!should_gate_recall("test", 4));
        assert!(!should_gate_recall("hello", 4));
        assert!(!should_gate_recall(
            "how do I deploy", 4
        ));
    }

    /// Whitespace-only inputs gate at any non-zero threshold
    /// (the trimmed-char count is zero).
    #[test]
    fn whitespace_only_gates_at_any_nonzero_threshold() {
        assert!(should_gate_recall("", 1));
        assert!(should_gate_recall("   ", 1));
        assert!(should_gate_recall("\t\n  ", 1));
        assert!(should_gate_recall(" ", 100));
    }

    /// The trim step runs before the count — leading and
    /// trailing whitespace doesn't shield a short message
    /// from gating.
    #[test]
    fn trim_runs_before_count() {
        // `"  ok  "` trims to `"ok"` (2 chars) and gates at
        // threshold 4.
        assert!(should_gate_recall("  ok  ", 4));
        // `"  ok  "` trims to `"ok"` (2 chars) and passes at
        // threshold 2 (strictly-less-than → 2 < 2 is false).
        assert!(!should_gate_recall("  ok  ", 2));
    }

    /// Unicode characters count as chars, not bytes. `héllo`
    /// is 5 Unicode characters (6 UTF-8 bytes); at threshold
    /// 5 it should NOT gate.
    #[test]
    fn unicode_counted_as_chars_not_bytes() {
        // `é` is 2 UTF-8 bytes but 1 char. `héllo` is 5
        // chars / 6 bytes.
        assert_eq!("héllo".len(), 6);
        assert_eq!("héllo".chars().count(), 5);
        // At threshold 5: 5 chars NOT < 5, passes.
        assert!(!should_gate_recall("héllo", 5));
        // At threshold 6: 5 chars < 6, gates.
        assert!(should_gate_recall("héllo", 6));
    }

    /// Internal whitespace counts toward the trimmed
    /// character count — only leading/trailing whitespace is
    /// stripped. `"hi !"` is four chars after trim.
    #[test]
    fn internal_whitespace_counts() {
        assert!(!should_gate_recall("hi !", 4));
        assert!(should_gate_recall("hi !", 5));
    }
}
