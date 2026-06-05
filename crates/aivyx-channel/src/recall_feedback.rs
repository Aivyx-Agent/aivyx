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
use std::sync::Arc;

use aivyx_memory::Memory;

use crate::persona::{
    PersonaDeltaCategory, PersonaDeltaOp, ProposedPersonaDelta,
};
use crate::persona_proposal::PersistentPersonaProposalLog;
use crate::recall_log::{RecallEvent, RecallJudgment};
use crate::reflection_scheduler::OutcomeSummary;

/// Minimum net helpfulness for the retention actuator to
/// promote an entry. `WEIGHT` = one net-clean turn: a single
/// good turn is enough to keep a memory warm; consistently
/// unhelpful ones (score < this, including all negatives) are
/// simply not promoted and lose under the existing LRU pass.
pub const PROMOTE_THRESHOLD: f32 = WEIGHT;

/// Minimum *per-topic* net helpfulness before Actuator B will
/// emit a Persona proposal. `3 * WEIGHT` mirrors the reflection
/// loop's own "a pattern must recur in at least 3 distinct
/// turns" discipline — one or two good turns is not an identity
/// signal, it's noise. The operator gate is the final say
/// regardless; this threshold just keeps the queue meaningful.
pub const PROPOSAL_TOPIC_THRESHOLD: f32 = 3.0 * WEIGHT;

/// How long recall events are retained before the
/// same-cadence GC clamp drops them. Generous relative to any
/// realistic reflection lookback so the loop always has a full
/// window of signal; bounded so the dedicated RecallEvents
/// domain can't grow without limit. ~30 days. (A `[recall_feedback]`
/// knob to tune this is a documented Phase 77 deferral.)
pub const RECALL_LOG_RETAIN_SECS: u64 = 30 * 24 * 3600;

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

    /// Test/seed constructor — build a tally from explicit
    /// `(topic, seq, score)` triples without running
    /// `correlate`.
    #[cfg(test)]
    pub(crate) fn from_triples(
        triples: &[(&str, u64, f32)],
    ) -> Self {
        let mut t = Self::default();
        for (topic, seq, score) in triples {
            t.add(topic, *seq, *score);
        }
        t
    }
}

/// Actuator A — retention self-tuning. For every entry whose
/// net helpfulness is at or above [`PROMOTE_THRESHOLD`], refresh
/// its LRU heat via [`Memory::promote_recall_helpful`] so the
/// existing Phase 74 eviction pass protects it. Net-negative
/// and below-threshold entries are deliberately left untouched
/// — under the same LRU pass they lose to the promoted ones,
/// which *is* the "decay faster" half (no new eviction
/// primitive). Returns how many entries were actually promoted
/// (an entry whose body was GC'd in the meantime counts as
/// not-promoted — the vector/signal is allowed to lag entry
/// GC, same backstop discipline as semantic_search).
pub async fn apply_retention_feedback(
    memory: &Arc<dyn Memory>,
    tally: &HelpfulnessTally,
) -> usize {
    let mut promoted = 0usize;
    for (topic, seq, score) in tally.ranked() {
        if score < PROMOTE_THRESHOLD {
            // `ranked()` is score-descending — once we drop
            // below the threshold nothing later qualifies.
            break;
        }
        if matches!(
            memory.promote_recall_helpful(&topic, seq).await,
            Ok(true)
        ) {
            promoted += 1;
        }
    }
    promoted
}

/// One operator-reviewable proposal Actuator B wants to file.
/// `proposal_id` is deterministic in the topic so re-running
/// the loop never queues a duplicate (the emitter skips an id
/// already present in the chain).
#[derive(Debug, Clone, PartialEq)]
pub struct RecallProposal {
    pub proposal_id: String,
    pub proposed_op: ProposedPersonaDelta,
}

/// Aggregate the tally by topic and turn each
/// strongly-net-helpful topic into a single conservative
/// `LearnedContext` proposal. Pure + deterministic (sorted by
/// topic) so it is unit-testable without a chain and so the
/// `proposal_id` is stable across cycles. Phrased as an
/// observation the operator approves or rejects — never an
/// authoritative identity claim (the gate is the authority,
/// Phase 70 P14 rule).
pub fn proposals_from_tally(
    tally: &HelpfulnessTally,
) -> Vec<RecallProposal> {
    // Sum per topic across all of its entries.
    let mut by_topic: HashMap<String, f32> = HashMap::new();
    for (topic, _seq, score) in tally.ranked() {
        *by_topic.entry(topic).or_insert(0.0) += score;
    }
    let mut topics: Vec<(String, f32)> = by_topic
        .into_iter()
        .filter(|(_, s)| *s >= PROPOSAL_TOPIC_THRESHOLD)
        .collect();
    topics.sort_by(|a, b| a.0.cmp(&b.0));

    topics
        .into_iter()
        .map(|(topic, score)| {
            let value = format!(
                "Operator consistently benefits from recalled \
                 memory under topic '{topic}' — keep surfacing \
                 it proactively."
            );
            RecallProposal {
                proposal_id: format!("recall-fb:{topic}"),
                proposed_op: ProposedPersonaDelta {
                    category: PersonaDeltaCategory::LearnedContext,
                    op: PersonaDeltaOp::AppendList { value },
                    reason: Some(format!(
                        "Structural recall-feedback signal: net \
                         helpfulness {score:+.0} across recalls of \
                         topic '{topic}' (no LLM judgement; the \
                         operator decides)."
                    )),
                    // Phase 92 — recall-feedback proposals never
                    // supersede; only the consolidation pass
                    // files supersession-linked pairs.
                    supersedes_proposal_id: None,
                },
            }
        })
        .collect()
}

/// Actuator B — file each proposal as `Pending` in the
/// operator-gated persona-proposal chain, **skipping any
/// proposal_id already in the chain** so a periodic loop never
/// re-queues (or re-nags after a rejection). Best-effort: an
/// individual append failure is swallowed (the next cycle
/// retries). Returns how many *new* proposals were filed.
pub async fn emit_persona_proposals(
    log: &PersistentPersonaProposalLog,
    tally: &HelpfulnessTally,
    now_unix_ms: u64,
    source_session: &str,
) -> usize {
    let mut filed = 0usize;
    for proposal in proposals_from_tally(tally) {
        if log.get(&proposal.proposal_id).is_some() {
            continue; // already pending/approved/rejected
        }
        if log
            .append_pending(
                proposal.proposal_id,
                now_unix_ms,
                source_session.to_string(),
                proposal.proposed_op,
            )
            .await
            .is_ok()
        {
            filed += 1;
        }
    }
    filed
}

/// Did this outcome's session see another turn start within
/// `CORRECTION_WINDOW_MS` of this turn *ending*? Proxy for
/// "operator immediately came back."
///
/// `pub(crate)` since Phase 172: the correction-signal detector
/// (`crate::correction_detect`) reuses the exact same proxy so
/// the two consumers can never drift on what "the operator came
/// right back" means.
/// Phase 178 — the *earliest* follow-up turn that makes `this` a
/// correction: same session, different turn, started within
/// `CORRECTION_WINDOW_MS` of `this` ending. `None` if there is
/// none (i.e. `this` was not followed quickly). The correction-
/// judgment detector uses the returned outcome to locate the
/// follow-up's captured query.
pub(crate) fn followup_outcome<'a>(
    this: &OutcomeSummary,
    all: &'a [OutcomeSummary],
) -> Option<&'a OutcomeSummary> {
    let ended = this.started_at_unix_ms.saturating_add(this.duration_ms);
    all.iter()
        .filter(|o| {
            o.session_id == this.session_id
                && o.turn_id != this.turn_id
                && o.started_at_unix_ms >= ended
                && o.started_at_unix_ms.saturating_sub(ended)
                    <= CORRECTION_WINDOW_MS
        })
        .min_by_key(|o| o.started_at_unix_ms)
}

pub(crate) fn followed_quickly(
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
///
/// `pub(crate)` since Phase 172 so `crate::correction_detect`
/// matches recalls to turns identically to the Phase-77 pass.
pub(crate) fn match_outcome<'a>(
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

/// Per-recall correlation detail. The single source of truth
/// for "what happened to this recall" — both the aggregate
/// [`correlate`] and the Phase 78 learning-insights surface
/// derive from this so the operator never sees numbers that
/// disagree with what the loop actually did.
#[derive(Debug, Clone, PartialEq)]
pub struct RecallContribution {
    pub ts_secs: u64,
    pub session_id: String,
    /// The `(topic, seq)` memories this recall injected.
    pub topic_seqs: Vec<(String, u64)>,
    /// Outcome label of the turn this recall matched, or `None`
    /// if it matched no turn (turn not yet ended / outside the
    /// window).
    pub outcome_kind: Option<String>,
    /// Signed signal applied to every hit: `Some(+WEIGHT)`
    /// helpful, `Some(-WEIGHT)` unhelpful, `None` matched a
    /// no-signal outcome (escalated / cancelled) or no turn.
    pub signal: Option<f32>,
}

/// Phase 93 — per-hit verdict → signed contribution.
/// `Used` and `Hurt` produce the symmetric `±WEIGHT` signal
/// (same magnitude as the structural turn-level proxy);
/// `Irrelevant` produces `None` (no contribution — the hit
/// was dead weight but not actively harmful, so it neither
/// rewards nor punishes the entry).
fn judgment_signal(verdict: RecallJudgment) -> Option<f32> {
    match verdict {
        RecallJudgment::Used => Some(WEIGHT),
        RecallJudgment::Hurt => Some(-WEIGHT),
        RecallJudgment::Irrelevant => None,
    }
}

/// Correlate the window's recalls against its outcomes,
/// returning both the aggregate [`HelpfulnessTally`] and the
/// per-recall [`RecallContribution`] detail. The detail is what
/// the Phase 78 insights surface reconstructs provenance from;
/// keeping one matching pass guarantees the surface and the
/// actuators can never diverge.
///
/// Phase 93 — when `use_judgment_signal = true`, each hit's
/// per-hit `RecallJudgment` (Phase 91) overrides the turn-
/// level structural signal for that specific hit. Un-judged
/// hits (`judgment: None`) fall back to the turn-level
/// signal, so the augment is incremental as the Phase 91 cron
/// processes hits. When `use_judgment_signal = false`, the
/// behaviour is byte-identical to pre-Phase-93: the structural
/// signal applies uniformly to every hit on the matched turn.
pub fn correlate_detailed(
    recalls: &[RecallEvent],
    outcomes: &[OutcomeSummary],
    use_judgment_signal: bool,
) -> (HelpfulnessTally, Vec<RecallContribution>) {
    let mut tally = HelpfulnessTally::default();
    let mut detail = Vec::with_capacity(recalls.len());
    for recall in recalls {
        let topic_seqs: Vec<(String, u64)> = recall
            .hits
            .iter()
            .map(|h| (h.topic.clone(), h.seq))
            .collect();
        let matched = match_outcome(recall, outcomes);
        let outcome_kind =
            matched.map(|o| o.outcome_kind.clone());
        let turn_sig = matched.and_then(|o| turn_signal(o, outcomes));
        for hit in &recall.hits {
            let hit_signal = match (
                use_judgment_signal,
                hit.judgment,
            ) {
                (true, Some(verdict)) => judgment_signal(verdict),
                _ => turn_sig,
            };
            if let Some(sig) = hit_signal {
                tally.add(&hit.topic, hit.seq, sig);
            }
        }
        detail.push(RecallContribution {
            ts_secs: recall.ts_secs,
            session_id: recall.session_id.to_string(),
            topic_seqs,
            outcome_kind,
            signal: turn_sig,
        });
    }
    (tally, detail)
}

/// Correlate the window's recalls against its outcomes into a
/// helpfulness tally. Recalls with no matchable turn (the turn
/// hasn't ended yet, or fell outside the audit window) simply
/// contribute nothing — they'll match on a later cycle once
/// their `TurnEnded` is in the window, or age out with the
/// recall-log GC clamp.
///
/// Phase 93 — `use_judgment_signal` toggles per-hit judgment
/// override of the turn-level structural signal. See
/// [`correlate_detailed`].
pub fn correlate(
    recalls: &[RecallEvent],
    outcomes: &[OutcomeSummary],
    use_judgment_signal: bool,
) -> HelpfulnessTally {
    correlate_detailed(recalls, outcomes, use_judgment_signal).0
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
            query_text: String::new(),
            hits: hits
                .iter()
                .map(|(t, s)| RecallHit {
                    topic: (*t).into(),
                    seq: *s,
                    score: 0.9,
                    cluster: false,
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
            tools: Vec::new(),
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
        let tally = correlate(&recalls, &outcomes, false);
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
            correlate(&recalls, &outcomes, false).score("notes", 7),
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
        assert_eq!(correlate(&recalls, &outcomes, false).score("a", 1), -WEIGHT);
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
        assert_eq!(correlate(&recalls, &outcomes, false).score("a", 1), WEIGHT);
    }

    #[test]
    fn escalated_and_cancelled_yield_no_signal() {
        let s = SessionId::new();
        let sid = s.to_string();
        for kind in ["escalated", "cancelled", "weird_unknown"] {
            let recalls = [recall(100, s, &[("a", 1)])];
            let outcomes =
                [outcome(&sid, "t1", 100_000, 100, kind)];
            let tally = correlate(&recalls, &outcomes, false);
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
        assert!(correlate(&recalls, &outcomes, false).is_empty());

        // Outcome too far from the recall timestamp.
        let outcomes2 =
            [outcome(&s.to_string(), "t1", 999_000, 100, "completed")];
        assert!(correlate(&recalls, &outcomes2, false).is_empty());
    }

    // ---- Actuator A — retention self-tuning --------------------

    async fn last_read_of(
        memory: &Arc<dyn Memory>,
        topic: &str,
        seq: u64,
    ) -> u64 {
        // scan_prefix does NOT stamp last_read (Phase 76), so it
        // observes the field without perturbing it.
        let groups =
            memory.scan_prefix("", usize::MAX).await.unwrap();
        for (t, entries) in groups {
            if t == topic {
                for e in entries {
                    if e.seq == seq {
                        return e.last_read_at_secs;
                    }
                }
            }
        }
        panic!("entry {topic}/{seq} not found");
    }

    #[tokio::test]
    async fn promotes_only_helpful_entries() {
        let memory: Arc<dyn Memory> =
            Arc::new(aivyx_memory::InMemoryMemory::new());
        let good = memory.put("notes", "kept").await.unwrap();
        let bad = memory.put("notes", "unhelpful").await.unwrap();
        let meh = memory.put("notes", "weak").await.unwrap();
        assert_eq!(last_read_of(&memory, "notes", good).await, 0);

        let tally = HelpfulnessTally::from_triples(&[
            ("notes", good, 2.0 * WEIGHT), // clearly helpful
            ("notes", bad, -3.0 * WEIGHT), // net negative
            ("notes", meh, 0.5 * WEIGHT),  // below threshold
        ]);
        let n = apply_retention_feedback(&memory, &tally).await;
        assert_eq!(n, 1, "only the net-helpful entry is promoted");

        assert!(
            last_read_of(&memory, "notes", good).await > 0,
            "helpful entry must be LRU-promoted"
        );
        assert_eq!(
            last_read_of(&memory, "notes", bad).await,
            0,
            "net-negative entry must NOT be promoted"
        );
        assert_eq!(
            last_read_of(&memory, "notes", meh).await,
            0,
            "below-threshold entry must NOT be promoted"
        );
    }

    #[tokio::test]
    async fn empty_tally_promotes_nothing() {
        let memory: Arc<dyn Memory> =
            Arc::new(aivyx_memory::InMemoryMemory::new());
        memory.put("t", "x").await.unwrap();
        let tally = HelpfulnessTally::default();
        assert_eq!(apply_retention_feedback(&memory, &tally).await, 0);
    }

    #[tokio::test]
    async fn promoting_a_vanished_entry_is_not_counted() {
        // Signal references a (topic, seq) that no longer
        // exists (entry GC'd). Must not panic, must not count.
        let memory: Arc<dyn Memory> =
            Arc::new(aivyx_memory::InMemoryMemory::new());
        let tally =
            HelpfulnessTally::from_triples(&[("gone", 999, 5.0)]);
        assert_eq!(apply_retention_feedback(&memory, &tally).await, 0);
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
            correlate(&recalls, &outcomes, false).score("fav", 3),
            2.0 * WEIGHT
        );
    }

    // ---- Phase 93 — per-hit judgment-signal augmentation ------

    /// Helper: build a recall whose three hits carry the
    /// explicit per-hit judgments `(Used, Hurt, None)` in
    /// that order on topics `(used, hurt, none)`. All scores
    /// fixed at 0.9, all `cluster = false` (Phase 84 — these
    /// are primary hits, not cluster-injected siblings).
    fn recall_with_judgments(
        ts_secs: u64,
        session: SessionId,
        triples: &[(&str, u64, Option<RecallJudgment>)],
    ) -> RecallEvent {
        RecallEvent {
            ts_secs,
            session_id: session,
            query_text: String::new(),
            hits: triples
                .iter()
                .map(|(t, s, j)| RecallHit {
                    topic: (*t).into(),
                    seq: *s,
                    score: 0.9,
                    cluster: false,
                    judgment: *j,
                })
                .collect(),
        }
    }

    /// Phase 93 — direct test of the per-verdict mapping.
    /// `Used` and `Hurt` are symmetric `±WEIGHT`;
    /// `Irrelevant` is `None` (no contribution).
    #[test]
    fn judgment_signal_per_verdict_mapping() {
        assert_eq!(
            judgment_signal(RecallJudgment::Used),
            Some(WEIGHT)
        );
        assert_eq!(
            judgment_signal(RecallJudgment::Hurt),
            Some(-WEIGHT)
        );
        assert_eq!(judgment_signal(RecallJudgment::Irrelevant), None);
    }

    /// Phase 93 — knob off (the default): per-hit judgment is
    /// ignored. A turn whose structural signal is `+WEIGHT`
    /// applies it uniformly to every hit regardless of
    /// `judgment`. Byte-identical to pre-Phase-93.
    #[test]
    fn knob_off_ignores_judgment_and_uses_structural() {
        let s = SessionId::new();
        let sid = s.to_string();
        let recalls = [recall_with_judgments(
            100,
            s,
            &[
                ("used", 1, Some(RecallJudgment::Used)),
                ("hurt", 1, Some(RecallJudgment::Hurt)),
                ("none", 1, None),
            ],
        )];
        let outcomes =
            [outcome(&sid, "t1", 100_000, 1_000, "completed")];
        let tally = correlate(&recalls, &outcomes, false);
        assert_eq!(tally.score("used", 1), WEIGHT);
        assert_eq!(tally.score("hurt", 1), WEIGHT);
        assert_eq!(tally.score("none", 1), WEIGHT);
    }

    /// Phase 93 — knob on, per-hit judgment overrides the
    /// turn-level structural signal for each judged hit;
    /// un-judged hits fall back to the structural signal.
    /// Three-way mixed fixture per Q4a.
    #[test]
    fn knob_on_per_hit_judgment_overrides_with_fallback() {
        let s = SessionId::new();
        let sid = s.to_string();
        let recalls = [recall_with_judgments(
            100,
            s,
            &[
                ("used", 1, Some(RecallJudgment::Used)),
                ("hurt", 1, Some(RecallJudgment::Hurt)),
                ("none", 1, None),
            ],
        )];
        // Turn-level structural signal = +WEIGHT (clean
        // completion, no quick follow-up).
        let outcomes =
            [outcome(&sid, "t1", 100_000, 1_000, "completed")];
        let tally = correlate(&recalls, &outcomes, true);
        // Used overrides → +WEIGHT (same magnitude as
        // structural, but sourced from the verdict).
        assert_eq!(tally.score("used", 1), WEIGHT);
        // Hurt overrides → -WEIGHT despite the +WEIGHT turn.
        assert_eq!(tally.score("hurt", 1), -WEIGHT);
        // Un-judged falls back to structural → +WEIGHT.
        assert_eq!(tally.score("none", 1), WEIGHT);
    }

    /// Phase 93 — `Irrelevant` with the knob on contributes
    /// nothing (the hit was dead weight but not harmful —
    /// the entry is neither rewarded nor punished). The
    /// tally has no entry for that hit.
    #[test]
    fn irrelevant_with_knob_on_contributes_nothing() {
        let s = SessionId::new();
        let sid = s.to_string();
        let recalls = [recall_with_judgments(
            100,
            s,
            &[("irr", 1, Some(RecallJudgment::Irrelevant))],
        )];
        // Even on a clean +WEIGHT turn, the judgment-driven
        // path returns None for Irrelevant → no accumulation.
        let outcomes =
            [outcome(&sid, "t1", 100_000, 1_000, "completed")];
        let tally = correlate(&recalls, &outcomes, true);
        assert_eq!(tally.score("irr", 1), 0.0);
        assert!(
            tally.is_empty(),
            "Irrelevant must not produce a tally entry"
        );
    }

    /// Phase 93 — when the turn-level structural signal is
    /// `None` (escalated/cancelled), judgment-driven hits
    /// still fire. The structural fallback for `None`
    /// judgments also produces nothing — so an escalated
    /// turn with one Used hit + one un-judged hit yields
    /// only the Used contribution.
    #[test]
    fn judgment_fires_even_when_structural_silent() {
        let s = SessionId::new();
        let sid = s.to_string();
        let recalls = [recall_with_judgments(
            100,
            s,
            &[
                ("used", 1, Some(RecallJudgment::Used)),
                ("none", 1, None),
            ],
        )];
        // escalated → turn_signal returns None.
        let outcomes =
            [outcome(&sid, "t1", 100_000, 100, "escalated")];
        let tally = correlate(&recalls, &outcomes, true);
        assert_eq!(
            tally.score("used", 1),
            WEIGHT,
            "Used must accumulate from the verdict even when \
             structural is silent",
        );
        assert_eq!(
            tally.score("none", 1),
            0.0,
            "un-judged hit on silent turn contributes nothing"
        );
    }

    // ---- Actuator B — operator-gated Persona proposals ---------

    #[test]
    fn proposals_only_for_strongly_helpful_topics() {
        // topic "strong": 2 entries summing to 4 (>= 3 thresh).
        // topic "weak": sums to 2 (< 3). topic "neg": negative.
        let tally = HelpfulnessTally::from_triples(&[
            ("strong", 1, 2.0),
            ("strong", 2, 2.0),
            ("weak", 1, 2.0),
            ("neg", 1, -5.0),
        ]);
        let props = proposals_from_tally(&tally);
        assert_eq!(props.len(), 1);
        assert_eq!(props[0].proposal_id, "recall-fb:strong");
        match &props[0].proposed_op.op {
            PersonaDeltaOp::AppendList { value } => {
                assert!(value.contains("topic 'strong'"));
            }
            other => panic!("expected AppendList, got {other:?}"),
        }
        assert_eq!(
            props[0].proposed_op.category,
            PersonaDeltaCategory::LearnedContext
        );
        assert!(props[0].proposed_op.reason.is_some());
    }

    #[test]
    fn proposals_are_deterministic_and_topic_sorted() {
        let tally = HelpfulnessTally::from_triples(&[
            ("zeta", 1, 5.0),
            ("alpha", 1, 5.0),
        ]);
        let a = proposals_from_tally(&tally);
        let b = proposals_from_tally(&tally);
        assert_eq!(a, b, "must be deterministic");
        assert_eq!(a[0].proposal_id, "recall-fb:alpha");
        assert_eq!(a[1].proposal_id, "recall-fb:zeta");
    }

    #[test]
    fn no_proposals_from_empty_or_weak_tally() {
        assert!(
            proposals_from_tally(&HelpfulnessTally::default())
                .is_empty()
        );
        assert!(proposals_from_tally(&HelpfulnessTally::from_triples(
            &[("t", 1, 1.0)]
        ))
        .is_empty());
    }

    #[tokio::test]
    async fn emit_files_pending_and_dedups_across_cycles() {
        use crate::persona_proposal::{
            PersistentPersonaProposalLog, ProposalStatusFilter,
        };
        use aivyx_crypto::MasterKey;
        use aivyx_storage::{
            KeyDomain, RedbStorage, Storage, StorageConfig,
        };

        let base =
            std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let dir = std::path::PathBuf::from(base).join(format!(
            "aivyx-recall-prop-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("store.redb")),
            MasterKey::from_raw([78u8; 32]),
        )
        .await
        .unwrap();
        let log = PersistentPersonaProposalLog::open(
            store.domain(KeyDomain::PersonaProposals),
            b"recall-prop-test-key".to_vec(),
        )
        .await
        .unwrap();

        let tally =
            HelpfulnessTally::from_triples(&[("proj", 1, 5.0)]);

        // First cycle files one Pending proposal.
        let n1 =
            emit_persona_proposals(&log, &tally, 1_000, "refl-1").await;
        assert_eq!(n1, 1);
        let pending = log.list(ProposalStatusFilter::Pending);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "recall-fb:proj");

        // Second cycle, same signal → deterministic id already
        // present → nothing re-filed (no operator nagging).
        let n2 =
            emit_persona_proposals(&log, &tally, 2_000, "refl-2").await;
        assert_eq!(n2, 0);
        assert_eq!(log.list(ProposalStatusFilter::Pending).len(), 1);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
