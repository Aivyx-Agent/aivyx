//! Phase 77 — the structural recall-feedback correlator (Q1a).
//!
//! Pure, no LLM, no I/O. Given the recall events captured this
//! window (`crate::recall_log::RecallEvent`) and the audit
//! chain's outcome summaries for the same window
//! (`crate::reflection_scheduler::OutcomeSummary`), it decides
//! — structurally — which recalled `(topic, seq)` memories
//! helped and which didn't, and accumulates a per-entry
//! helpfulness score the actuators (Tasks 6/7) consume.
//!
//! The heuristic is deliberately coarse per turn but reliable
//! in aggregate:
//!
//! - The recall is matched to the turn it was injected into
//!   (same session, turn start ≈ recall timestamp).
//! - `completed` with no rapid operator follow-up → the
//!   recalled memories get **+1** (the turn went fine and the
//!   operator didn't immediately come back to re-ask).
//! - `completed` *followed within 60 s by another turn in the
//!   same session* → **−1**: the operator immediately came
//!   back, the structural proxy for "that didn't land"
//!   (Q1a's operator-corrected-next-turn signal).
//! - `failed` / `timed_out` → **−1** (recall didn't save it).
//! - `escalated` / `cancelled` → **0** (ambiguous /
//!   operator-initiated — no signal either way).
//!
//! None of this judges *content*; it never asks an LLM whether
//! recall was useful. That self-judgement weakness is exactly
//! what the structural design avoids.

use std::collections::HashMap;

use crate::recall_log::RecallEvent;
use crate::reflection_scheduler::OutcomeSummary;

/// A recall injected into a turn that ended within this many ms
/// before another turn started (same session) is treated as
/// "the operator immediately came back" — weak-negative.
pub const CORRECTION_WINDOW_MS: u64 = 60_000;

/// A recall event is matched to the outcome whose turn start is
/// within this slack of the recall timestamp (recall fires at
/// `begin_turn`, i.e. essentially turn start).
pub const MATCH_TOLERANCE_MS: u64 = 5_000;

/// Per-hit score magnitude. Symmetric: one clean turn is worth
/// exactly as much as one bad one, so a memory has to be
/// *consistently* helpful to net positive.
pub const WEIGHT: f32 = 1.0;

/// Accumulated helpfulness, keyed by `(topic, seq)`. Positive =
/// net helpful, negative = net unhelpful, absent = no signal
/// yet. The actuators read this; they do not re-derive it.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct HelpfulnessTally {
    scores: HashMap<(String, u64), f32>,
}

impl HelpfulnessTally {
    pub fn score(&self, topic: &str, seq: u64) -> f32 {
        self.scores
            .get(&(topic.to_string(), seq))
            .copied()
            .unwrap_or(0.0)
    }

    pub fn is_empty(&self) -> bool {
        self.scores.is_empty()
    }

    pub fn len(&self) -> usize {
        self.scores.len()
    }

    /// Every `(topic, seq, score)` with a non-zero net score,
    /// sorted by score descending then `(topic, seq)` for
    /// determinism. Actuators iterate this.
    pub fn ranked(&self) -> Vec<(String, u64, f32)> {
        let mut v: Vec<(String, u64, f32)> = self
            .scores
            .iter()
            .filter(|(_, s)| **s != 0.0)
            .map(|((t, q), s)| (t.clone(), *q, *s))
            .collect();
        v.sort_by(|a, b| {
            b.2.partial_cmp(&a.2)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
                .then(a.1.cmp(&b.1))
        });
        v
    }

    fn add(&mut self, topic: &str, seq: u64, delta: f32) {
        *self.scores.entry((topic.to_string(), seq)).or_insert(0.0) +=
            delta;
    }
}

/// Did this outcome's session see another turn start within
/// `CORRECTION_WINDOW_MS` of this turn *ending*? Proxy for
/// "operator immediately came back."
fn followed_quickly(
    this: &OutcomeSummary,
    all: &[OutcomeSummary],
) -> bool {
    let ended = this.started_at_unix_ms.saturating_add(this.duration_ms);
    all.iter().any(|o| {
        o.session_id == this.session_id
            && o.turn_id != this.turn_id
            && o.started_at_unix_ms >= ended
            && o.started_at_unix_ms.saturating_sub(ended)
                <= CORRECTION_WINDOW_MS
    })
}

/// The signed contribution of one matched turn to its recalled
/// memories: `Some(+WEIGHT)` helpful, `Some(-WEIGHT)`
/// unhelpful, `None` no signal.
fn turn_signal(
    outcome: &OutcomeSummary,
    all: &[OutcomeSummary],
) -> Option<f32> {
    match outcome.outcome_kind.as_str() {
        "completed" => {
            if followed_quickly(outcome, all) {
                Some(-WEIGHT)
            } else {
                Some(WEIGHT)
            }
        }
        "failed" | "timed_out" => Some(-WEIGHT),
        // escalated / cancelled / anything unknown → no signal.
        _ => None,
    }
}

/// Match a recall event to the turn it was injected into: same
/// session, the outcome whose start is closest to the recall
/// timestamp within `MATCH_TOLERANCE_MS`.
fn match_outcome<'a>(
    recall: &RecallEvent,
    outcomes: &'a [OutcomeSummary],
) -> Option<&'a OutcomeSummary> {
    let sid = recall.session_id.to_string();
    let recall_ms = recall.ts_secs.saturating_mul(1000);
    outcomes
        .iter()
        .filter(|o| o.session_id == sid)
        .map(|o| {
            let diff = o
                .started_at_unix_ms
                .abs_diff(recall_ms);
            (diff, o)
        })
        .filter(|(diff, _)| *diff <= MATCH_TOLERANCE_MS)
        .min_by_key(|(diff, _)| *diff)
        .map(|(_, o)| o)
}

/// Correlate the window's recalls against its outcomes into a
/// helpfulness tally. Recalls with no matchable turn (the turn
/// hasn't ended yet, or fell outside the audit window) simply
/// contribute nothing — they'll match on a later cycle once
/// their `TurnEnded` is in the window, or age out with the
/// recall-log GC clamp.
pub fn correlate(
    recalls: &[RecallEvent],
    outcomes: &[OutcomeSummary],
) -> HelpfulnessTally {
    let mut tally = HelpfulnessTally::default();
    for recall in recalls {
        let Some(outcome) = match_outcome(recall, outcomes) else {
            continue;
        };
        let Some(signal) = turn_signal(outcome, outcomes) else {
            continue;
        };
        for hit in &recall.hits {
            tally.add(&hit.topic, hit.seq, signal);
        }
    }
    tally
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recall_log::RecallHit;
    use aivyx_core::SessionId;

    fn recall(
        ts_secs: u64,
        session: SessionId,
        hits: &[(&str, u64)],
    ) -> RecallEvent {
        RecallEvent {
            ts_secs,
            session_id: session,
            hits: hits
                .iter()
                .map(|(t, s)| RecallHit {
                    topic: (*t).into(),
                    seq: *s,
                    score: 0.9,
                })
                .collect(),
        }
    }

    fn outcome(
        session: &str,
        turn: &str,
        start_ms: u64,
        dur_ms: u64,
        kind: &str,
    ) -> OutcomeSummary {
        OutcomeSummary {
            session_id: session.into(),
            turn_id: turn.into(),
            started_at_unix_ms: start_ms,
            outcome_kind: kind.into(),
            tool_calls_made: 0,
            duration_ms: dur_ms,
        }
    }

    #[test]
    fn clean_turn_no_followup_is_positive() {
        let s = SessionId::new();
        let recalls = [recall(100, s, &[("notes", 7)])];
        let outcomes = [outcome(
            &s.to_string(),
            "t1",
            100_000, // 100s in ms == recall ts
            2_000,
            "completed",
        )];
        let tally = correlate(&recalls, &outcomes);
        assert_eq!(tally.score("notes", 7), WEIGHT);
        assert_eq!(tally.ranked(), vec![("notes".into(), 7, WEIGHT)]);
    }

    #[test]
    fn failed_turn_is_negative() {
        let s = SessionId::new();
        let recalls = [recall(100, s, &[("notes", 7)])];
        let outcomes =
            [outcome(&s.to_string(), "t1", 100_000, 500, "failed")];
        assert_eq!(
            correlate(&recalls, &outcomes).score("notes", 7),
            -WEIGHT
        );
    }

    #[test]
    fn completed_then_quick_followup_is_negative() {
        // Turn t1 completes at 100_000+1_000; t2 starts 5s
        // later, same session → operator came right back.
        let s = SessionId::new();
        let sid = s.to_string();
        let recalls = [recall(100, s, &[("a", 1)])];
        let outcomes = [
            outcome(&sid, "t1", 100_000, 1_000, "completed"),
            outcome(&sid, "t2", 106_000, 1_000, "completed"),
        ];
        // t1's recalled memory is penalized; t2 had no recall.
        assert_eq!(correlate(&recalls, &outcomes).score("a", 1), -WEIGHT);
    }

    #[test]
    fn completed_then_late_followup_stays_positive() {
        let s = SessionId::new();
        let sid = s.to_string();
        let recalls = [recall(100, s, &[("a", 1)])];
        let outcomes = [
            outcome(&sid, "t1", 100_000, 1_000, "completed"),
            // 2 minutes later — outside the correction window.
            outcome(&sid, "t2", 221_000, 1_000, "completed"),
        ];
        assert_eq!(correlate(&recalls, &outcomes).score("a", 1), WEIGHT);
    }

    #[test]
    fn escalated_and_cancelled_yield_no_signal() {
        let s = SessionId::new();
        let sid = s.to_string();
        for kind in ["escalated", "cancelled", "weird_unknown"] {
            let recalls = [recall(100, s, &[("a", 1)])];
            let outcomes =
                [outcome(&sid, "t1", 100_000, 100, kind)];
            let tally = correlate(&recalls, &outcomes);
            assert_eq!(tally.score("a", 1), 0.0);
            assert!(tally.is_empty(), "{kind} must produce no entry");
        }
    }

    #[test]
    fn unmatched_recall_contributes_nothing() {
        let s = SessionId::new();
        // Outcome is a different session — no match.
        let recalls = [recall(100, s, &[("a", 1)])];
        let outcomes =
            [outcome("other-session", "t1", 100_000, 100, "completed")];
        assert!(correlate(&recalls, &outcomes).is_empty());

        // Outcome too far from the recall timestamp.
        let outcomes2 =
            [outcome(&s.to_string(), "t1", 999_000, 100, "completed")];
        assert!(correlate(&recalls, &outcomes2).is_empty());
    }

    #[test]
    fn repeated_signal_accumulates_per_entry() {
        let s = SessionId::new();
        let sid = s.to_string();
        // Same memory recalled in two clean turns → +2.
        let recalls = [
            recall(100, s, &[("fav", 3)]),
            recall(500, s, &[("fav", 3)]),
        ];
        let outcomes = [
            outcome(&sid, "t1", 100_000, 100, "completed"),
            outcome(&sid, "t2", 500_000, 100, "completed"),
        ];
        assert_eq!(
            correlate(&recalls, &outcomes).score("fav", 3),
            2.0 * WEIGHT
        );
    }
}
