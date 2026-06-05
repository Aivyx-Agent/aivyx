//! Phase 172 — the structural correction-signal detector.
//!
//! Pure, no LLM, no I/O. The self-improvement half of the
//! Aivyx Agent Review's §5.8 gap: the agent never noticed when
//! the operator *corrected* it. This module gives that event a
//! first-class signal.
//!
//! ## What a "correction" is (the honest definition)
//!
//! A correction is exactly the Phase-77 rapid-re-ask proxy: a
//! turn that **`completed`** but was **followed within
//! [`crate::recall_feedback::CORRECTION_WINDOW_MS`] by another
//! turn in the same session** — the operator immediately came
//! back. We reuse Phase 77's
//! [`crate::recall_feedback::followed_quickly`] and
//! [`crate::recall_feedback::match_outcome`] verbatim so the
//! two consumers can never drift on what that means.
//!
//! Crucially, this is **narrower** than the Phase-77/82
//! helpfulness `−1`:
//!
//! - `failed` / `timed_out` turns are *agent failures*, not
//!   operator corrections — **excluded**. The correction
//!   signal is about "the answer wasn't what you wanted," not
//!   "the tool broke."
//! - `escalated` / `cancelled` — operator-initiated / ambiguous
//!   — **excluded** (no signal, same as Phase 77).
//! - Only `completed`-then-rapid-followup fires.
//!
//! Each correction is attributed to the **distinct recalled
//! topics** injected into the corrected turn — the only
//! structural "what was this turn about" surface available
//! (`OutcomeSummary` carries no tool/topic). Cluster-injected
//! sibling hits are **excluded** (the Phase-84 self-policing
//! posture): the ledger should learn from the topics the
//! operator's turn organically recalled, never from the
//! agent's own expansion.
//!
//! ## Why a separate ledger from helpfulness
//!
//! Helpfulness nets `+1/−1` per `(topic, seq)` and drives
//! *memory retention* (Phase 77 Actuator A). The correction
//! signal **counts** only the rework events per topic and
//! drives *operator-preference* proposals (Phase 172 Task 4).
//! A topic can net positive helpfulness while still
//! accumulating corrections — those are precisely the topics
//! worth a Profile note. Two actuators on one raw event, the
//! established Phase 82/83/84/87 pattern.

use std::collections::HashMap;

use crate::recall_log::RecallEvent;
use crate::reflection_scheduler::OutcomeSummary;

/// Per-topic correction count for one reflection window.
/// Keyed by topic; value = how many distinct corrected turns
/// recalled that topic this window. Absent = no correction
/// signal. The Phase 172 ledger folds [`ranked`] into its
/// durable decayed view.
///
/// [`ranked`]: CorrectionTally::ranked
#[derive(Debug, Default, Clone, PartialEq)]
pub struct CorrectionTally {
    counts: HashMap<String, u32>,
}

impl CorrectionTally {
    /// How many corrected turns this window recalled `topic`.
    pub fn count(&self, topic: &str) -> u32 {
        self.counts.get(topic).copied().unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }

    pub fn len(&self) -> usize {
        self.counts.len()
    }

    /// Every `(topic, count)` with a non-zero count, sorted by
    /// count descending then topic ascending for determinism.
    /// The fold + the Phase 78 surface both iterate this.
    pub fn ranked(&self) -> Vec<(String, u32)> {
        let mut v: Vec<(String, u32)> = self
            .counts
            .iter()
            .filter(|(_, c)| **c > 0)
            .map(|(t, c)| (t.clone(), *c))
            .collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }

    fn add(&mut self, topic: &str) {
        *self.counts.entry(topic.to_string()).or_insert(0) += 1;
    }

    /// Test/seed constructor — build a tally from explicit
    /// `(topic, count)` pairs without running [`detect`].
    ///
    /// [`detect`]: detect_corrections
    #[cfg(test)]
    pub(crate) fn from_pairs(pairs: &[(&str, u32)]) -> Self {
        let mut t = Self::default();
        for (topic, count) in pairs {
            for _ in 0..*count {
                t.add(topic);
            }
        }
        t
    }
}

/// Walk the window's recalls against its outcomes and count,
/// per distinct topic, how many **corrected** turns recalled
/// it. A corrected turn is one whose matched outcome
/// `completed` *and* was followed quickly by another turn in
/// the same session (Phase 77's proxy).
///
/// Pure + deterministic. Recalls that match no turn (turn not
/// yet ended / outside the window), match a non-`completed`
/// turn, or whose turn was *not* followed quickly contribute
/// nothing. Cluster-injected hits are excluded; a turn whose
/// only hits are cluster siblings contributes nothing.
pub fn detect_corrections(
    recalls: &[RecallEvent],
    outcomes: &[OutcomeSummary],
) -> CorrectionTally {
    let mut tally = CorrectionTally::default();
    for recall in recalls {
        let Some(outcome) =
            crate::recall_feedback::match_outcome(recall, outcomes)
        else {
            continue;
        };
        // Only `completed`-then-rapid-followup is a correction.
        if outcome.outcome_kind != "completed" {
            continue;
        }
        if !crate::recall_feedback::followed_quickly(outcome, outcomes)
        {
            continue;
        }
        // Distinct, non-cluster topics for this corrected turn:
        // one increment per topic per turn (multiple seqs of the
        // same topic do not double-count).
        let mut seen: Vec<&str> = Vec::new();
        for hit in &recall.hits {
            if hit.cluster {
                continue;
            }
            if !seen.contains(&hit.topic.as_str()) {
                seen.push(hit.topic.as_str());
                tally.add(&hit.topic);
            }
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
        hits: &[(&str, u64, bool)],
    ) -> RecallEvent {
        RecallEvent {
            ts_secs,
            session_id: session,
            hits: hits
                .iter()
                .map(|(t, s, cluster)| RecallHit {
                    topic: (*t).into(),
                    seq: *s,
                    score: 0.9,
                    cluster: *cluster,
                    judgment: None,
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
    fn completed_then_quick_followup_is_a_correction() {
        // t1 completes at 100_000+1_000; t2 starts 5s later,
        // same session → operator came right back.
        let s = SessionId::new();
        let sid = s.to_string();
        let recalls = [recall(100, s, &[("auth", 1, false)])];
        let outcomes = [
            outcome(&sid, "t1", 100_000, 1_000, "completed"),
            outcome(&sid, "t2", 106_000, 1_000, "completed"),
        ];
        let tally = detect_corrections(&recalls, &outcomes);
        assert_eq!(tally.count("auth"), 1);
        assert_eq!(tally.ranked(), vec![("auth".into(), 1)]);
    }

    #[test]
    fn clean_turn_no_followup_is_not_a_correction() {
        let s = SessionId::new();
        let recalls = [recall(100, s, &[("auth", 1, false)])];
        let outcomes = [outcome(
            &s.to_string(),
            "t1",
            100_000,
            2_000,
            "completed",
        )];
        let tally = detect_corrections(&recalls, &outcomes);
        assert!(
            tally.is_empty(),
            "a clean completion with no rapid follow-up is not \
             a correction"
        );
    }

    #[test]
    fn late_followup_is_not_a_correction() {
        let s = SessionId::new();
        let sid = s.to_string();
        let recalls = [recall(100, s, &[("auth", 1, false)])];
        let outcomes = [
            outcome(&sid, "t1", 100_000, 1_000, "completed"),
            // 2 minutes later — outside the correction window.
            outcome(&sid, "t2", 221_000, 1_000, "completed"),
        ];
        assert!(detect_corrections(&recalls, &outcomes).is_empty());
    }

    #[test]
    fn failed_and_timed_out_are_not_corrections() {
        // Agent failures, not operator corrections — even with a
        // quick follow-up.
        let s = SessionId::new();
        let sid = s.to_string();
        for kind in ["failed", "timed_out"] {
            let recalls = [recall(100, s, &[("auth", 1, false)])];
            let outcomes = [
                outcome(&sid, "t1", 100_000, 1_000, kind),
                outcome(&sid, "t2", 106_000, 1_000, "completed"),
            ];
            let tally = detect_corrections(&recalls, &outcomes);
            assert!(
                tally.is_empty(),
                "{kind} is an agent failure, not a correction"
            );
        }
    }

    #[test]
    fn escalated_and_cancelled_are_not_corrections() {
        let s = SessionId::new();
        let sid = s.to_string();
        for kind in ["escalated", "cancelled", "weird_unknown"] {
            let recalls = [recall(100, s, &[("auth", 1, false)])];
            let outcomes = [
                outcome(&sid, "t1", 100_000, 1_000, kind),
                outcome(&sid, "t2", 106_000, 1_000, "completed"),
            ];
            assert!(
                detect_corrections(&recalls, &outcomes).is_empty(),
                "{kind} yields no correction signal"
            );
        }
    }

    #[test]
    fn cluster_hits_are_excluded() {
        // The corrected turn recalled `primary` organically and
        // `sibling` via cluster expansion. Only `primary` counts.
        let s = SessionId::new();
        let sid = s.to_string();
        let recalls = [recall(
            100,
            s,
            &[("primary", 1, false), ("sibling", 2, true)],
        )];
        let outcomes = [
            outcome(&sid, "t1", 100_000, 1_000, "completed"),
            outcome(&sid, "t2", 106_000, 1_000, "completed"),
        ];
        let tally = detect_corrections(&recalls, &outcomes);
        assert_eq!(tally.count("primary"), 1);
        assert_eq!(tally.count("sibling"), 0);
        assert_eq!(tally.len(), 1);
    }

    #[test]
    fn turn_with_only_cluster_hits_contributes_nothing() {
        let s = SessionId::new();
        let sid = s.to_string();
        let recalls = [recall(100, s, &[("sibling", 2, true)])];
        let outcomes = [
            outcome(&sid, "t1", 100_000, 1_000, "completed"),
            outcome(&sid, "t2", 106_000, 1_000, "completed"),
        ];
        assert!(detect_corrections(&recalls, &outcomes).is_empty());
    }

    #[test]
    fn distinct_topics_count_once_per_turn() {
        // Same topic recalled at two seqs in one corrected turn →
        // counts once. A second topic counts once too.
        let s = SessionId::new();
        let sid = s.to_string();
        let recalls = [recall(
            100,
            s,
            &[("auth", 1, false), ("auth", 9, false), ("db", 3, false)],
        )];
        let outcomes = [
            outcome(&sid, "t1", 100_000, 1_000, "completed"),
            outcome(&sid, "t2", 106_000, 1_000, "completed"),
        ];
        let tally = detect_corrections(&recalls, &outcomes);
        assert_eq!(tally.count("auth"), 1, "distinct per turn");
        assert_eq!(tally.count("db"), 1);
        assert_eq!(tally.len(), 2);
    }

    #[test]
    fn corrections_accumulate_across_turns() {
        // `auth` is recalled in TWO separate corrected turns → 2.
        let s = SessionId::new();
        let sid = s.to_string();
        let recalls = [
            recall(100, s, &[("auth", 1, false)]),
            recall(300, s, &[("auth", 1, false)]),
        ];
        let outcomes = [
            outcome(&sid, "t1", 100_000, 1_000, "completed"),
            outcome(&sid, "t2", 106_000, 1_000, "completed"),
            outcome(&sid, "t3", 300_000, 1_000, "completed"),
            outcome(&sid, "t4", 306_000, 1_000, "completed"),
        ];
        let tally = detect_corrections(&recalls, &outcomes);
        assert_eq!(tally.count("auth"), 2);
    }

    #[test]
    fn unmatched_recall_contributes_nothing() {
        let s = SessionId::new();
        // Outcome is a different session — no match.
        let recalls = [recall(100, s, &[("auth", 1, false)])];
        let outcomes = [
            outcome("other", "t1", 100_000, 1_000, "completed"),
            outcome("other", "t2", 106_000, 1_000, "completed"),
        ];
        assert!(detect_corrections(&recalls, &outcomes).is_empty());
    }

    #[test]
    fn ranked_is_count_desc_then_topic() {
        let t = CorrectionTally::from_pairs(&[
            ("low", 1),
            ("high", 5),
            ("mid", 3),
        ]);
        let order: Vec<String> =
            t.ranked().into_iter().map(|(topic, _)| topic).collect();
        assert_eq!(order, vec!["high", "mid", "low"]);
    }

    #[test]
    fn empty_inputs_are_empty() {
        assert!(detect_corrections(&[], &[]).is_empty());
    }
}
