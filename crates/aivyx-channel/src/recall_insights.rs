//! Phase 78 — learning-observability derivation.
//!
//! Pure functions that turn the Phase 77 recall-feedback signal
//! into an operator-facing picture: a per-window
//! [`LearningDigest`] ("is the loop healthy, what is it leaning
//! toward") and per-proposal [`ProposalProvenance`] ("why did it
//! propose to change its Persona"). Everything is derived
//! on-query from the live recall log + audit outcomes + the
//! proposal chain (Q1a/Q4a) — no new storage, always consistent
//! with what the loop actually did because it shares
//! [`crate::recall_feedback::correlate_detailed`]'s single
//! matching pass.

use serde::{Deserialize, Serialize};

use crate::persona_proposal::PersonaProposal;
use crate::recall_feedback::{
    HelpfulnessTally, RecallContribution, PROMOTE_THRESHOLD,
};

/// Phase 77 files recall-driven proposals with this id prefix
/// (`recall-fb:{topic}`); the surface uses it to pick the
/// recall-attributable proposals out of the full chain and to
/// recover the topic without parsing the op.
pub const RECALL_PROPOSAL_PREFIX: &str = "recall-fb:";

/// How many topics each top-list shows. A trust surface wants
/// the headline, not the whole tally.
const TOP_N: usize = 5;

/// One turn that contributed to a topic's score, as the
/// operator sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContributingTurn {
    pub ts_secs: u64,
    /// Outcome label of the matched turn, or `None` if the
    /// recall matched no turn in the window.
    pub outcome_kind: Option<String>,
    /// Signed contribution (`+`/`−`weight), or `None` for a
    /// no-signal / unmatched recall.
    pub signal: Option<f32>,
    /// The seqs of *this topic's* memories injected on that
    /// turn.
    pub seqs: Vec<u64>,
}

/// Why one Pending (or resolved) recall-driven Persona proposal
/// exists, reconstructed from the recall log (Q4a).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProposalProvenance {
    pub proposal_id: String,
    pub topic: String,
    pub status: String,
    /// Net helpfulness across this topic that drove the
    /// proposal (the same sum `proposals_from_tally` thresholded
    /// on).
    pub net_score: f32,
    /// The agent's stated reason on the proposal record.
    pub reason: Option<String>,
    /// The recalls/turns that produced the score, newest first.
    pub contributing: Vec<ContributingTurn>,
}

/// Per-window operational picture of the self-learning loop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearningDigest {
    /// The lookback the digest was computed over (seconds).
    pub window_secs: u64,
    /// Recalls in the window (any — matched or not).
    pub recalls_total: usize,
    /// Recalls that produced a signal (matched a turn with a
    /// helpful/unhelpful outcome).
    pub recalls_scored: usize,
    /// Distinct `(topic, seq)` entries the retention actuator
    /// would keep warm this window.
    pub promoted: usize,
    /// Distinct scored entries below the promote threshold
    /// (net-negative or too weak) — left to age out.
    pub not_promoted: usize,
    /// Top helpful topics (net score, descending).
    pub top_helpful: Vec<(String, f32)>,
    /// Top unhelpful topics (net score, ascending = most
    /// negative first).
    pub top_unhelpful: Vec<(String, f32)>,
    /// Recall-driven Persona proposals visible in the chain.
    pub proposals_in_window: usize,
    /// Phase 93 — whether the recall-feedback correlator
    /// was running with per-hit judgment override
    /// (`[recall_feedback].use_judgment_signal = true`).
    /// `None` for pre-Phase-93 digests; `Some(false)`
    /// distinguishes "knob explicitly off" from "section
    /// absent" on the surface. `#[serde(default,
    /// skip_serializing_if = "Option::is_none")]` keeps the
    /// IPC wire-compat — older `aivyx learning` clients
    /// decode the digest unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub judgment_signal: Option<bool>,
}

/// Sum a tally to per-topic net scores.
fn per_topic(tally: &HelpfulnessTally) -> Vec<(String, f32)> {
    use std::collections::HashMap;
    let mut m: HashMap<String, f32> = HashMap::new();
    for (topic, _seq, score) in tally.ranked() {
        *m.entry(topic).or_insert(0.0) += score;
    }
    m.into_iter().collect()
}

/// Build the digest. `proposals` is the full proposal chain
/// snapshot; only the recall-driven ones are counted.
/// Phase 93 — `use_judgment_signal` is recorded on the
/// digest so the CLI / Web UI surface can flag the
/// augment to the operator. Passing `None` is the
/// pre-Phase-93 default (the surface renders no judgment-
/// signal line).
pub fn build_digest(
    window_secs: u64,
    tally: &HelpfulnessTally,
    contributions: &[RecallContribution],
    proposals: &[PersonaProposal],
    use_judgment_signal: Option<bool>,
) -> LearningDigest {
    let recalls_scored =
        contributions.iter().filter(|c| c.signal.is_some()).count();

    let ranked = tally.ranked();
    let promoted = ranked
        .iter()
        .filter(|(_, _, s)| *s >= PROMOTE_THRESHOLD)
        .count();
    let not_promoted = ranked.len() - promoted;

    let mut topics = per_topic(tally);
    // Helpful: positive, score desc then topic for determinism.
    let mut top_helpful: Vec<(String, f32)> = topics
        .iter()
        .filter(|(_, s)| *s > 0.0)
        .cloned()
        .collect();
    top_helpful.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    top_helpful.truncate(TOP_N);
    // Unhelpful: negative, most-negative first.
    topics.retain(|(_, s)| *s < 0.0);
    topics.sort_by(|a, b| {
        a.1.partial_cmp(&b.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.0.cmp(&b.0))
    });
    topics.truncate(TOP_N);

    let proposals_in_window = proposals
        .iter()
        .filter(|p| p.id.starts_with(RECALL_PROPOSAL_PREFIX))
        .count();

    LearningDigest {
        window_secs,
        recalls_total: contributions.len(),
        recalls_scored,
        judgment_signal: use_judgment_signal,
        promoted,
        not_promoted,
        top_helpful,
        top_unhelpful: topics,
        proposals_in_window,
    }
}

/// Stable string label for a proposal status (mirrors the
/// chain's discriminator without depending on its Display).
fn status_label(p: &PersonaProposal) -> String {
    use crate::persona_proposal::ProposalStatus;
    match &p.status {
        ProposalStatus::Pending => "pending",
        ProposalStatus::Approved { .. } => "approved",
        ProposalStatus::Rejected { .. } => "rejected",
        ProposalStatus::Superseded { .. } => "superseded",
    }
    .to_string()
}

/// Reconstruct provenance for every recall-driven proposal
/// (Q4a): match the proposal's topic (recovered from its
/// deterministic id) back to the contributing recalls/turns.
/// Non-recall proposals are skipped (they have other origins).
pub fn build_provenance(
    contributions: &[RecallContribution],
    proposals: &[PersonaProposal],
) -> Vec<ProposalProvenance> {
    let mut out = Vec::new();
    for p in proposals {
        let Some(topic) = p.id.strip_prefix(RECALL_PROPOSAL_PREFIX)
        else {
            continue;
        };
        let mut net_score = 0.0f32;
        let mut contributing: Vec<ContributingTurn> = Vec::new();
        for c in contributions {
            let seqs: Vec<u64> = c
                .topic_seqs
                .iter()
                .filter(|(t, _)| t == topic)
                .map(|(_, s)| *s)
                .collect();
            if seqs.is_empty() {
                continue;
            }
            if let Some(sig) = c.signal {
                // Each hit of this topic on this turn took `sig`.
                net_score += sig * seqs.len() as f32;
            }
            contributing.push(ContributingTurn {
                ts_secs: c.ts_secs,
                outcome_kind: c.outcome_kind.clone(),
                signal: c.signal,
                seqs,
            });
        }
        contributing
            .sort_by_key(|c| std::cmp::Reverse(c.ts_secs));
        out.push(ProposalProvenance {
            proposal_id: p.id.clone(),
            topic: topic.to_string(),
            status: status_label(p),
            net_score,
            reason: p.proposed_op.reason.clone(),
            contributing,
        });
    }
    out.sort_by(|a, b| a.proposal_id.cmp(&b.proposal_id));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persona::{
        PersonaDeltaCategory, PersonaDeltaOp, ProposedPersonaDelta,
    };
    use crate::persona_proposal::{PersonaProposal, ProposalStatus};
    use crate::recall_feedback::correlate_detailed;
    use crate::recall_log::{RecallEvent, RecallHit};
    use crate::reflection_scheduler::OutcomeSummary;
    use aivyx_core::SessionId;

    fn recall(
        ts: u64,
        s: SessionId,
        hits: &[(&str, u64)],
    ) -> RecallEvent {
        RecallEvent {
            ts_secs: ts,
            session_id: s,
            query_text: String::new(),
            hits: hits
                .iter()
                .map(|(t, q)| RecallHit {
                    topic: (*t).into(),
                    seq: *q,
                    score: 0.9,
                    cluster: false,
                    judgment: None,
                })
                .collect(),
        }
    }

    fn outcome(
        sid: &str,
        turn: &str,
        start_ms: u64,
        kind: &str,
    ) -> OutcomeSummary {
        OutcomeSummary {
            session_id: sid.into(),
            turn_id: turn.into(),
            started_at_unix_ms: start_ms,
            outcome_kind: kind.into(),
            tool_calls_made: 0,
            duration_ms: 100,
        }
    }

    fn recall_proposal(topic: &str) -> PersonaProposal {
        PersonaProposal {
            id: format!("recall-fb:{topic}"),
            proposed_at_unix_ms: 1,
            source_reflection_session_id: "reflection:nightly".into(),
            proposed_op: ProposedPersonaDelta {
                category: PersonaDeltaCategory::LearnedContext,
                op: PersonaDeltaOp::AppendList {
                    value: "v".into(),
                },
                reason: Some("net +3".into()),
                supersedes_proposal_id: None,
            },
            status: ProposalStatus::Pending,
        }
    }

    #[test]
    fn digest_counts_and_top_lists() {
        let s = SessionId::new();
        let sid = s.to_string();
        // 3 clean turns for "good" (+3), 1 failed for "bad" (−1).
        let recalls = vec![
            recall(100, s, &[("good", 1)]),
            recall(5000, s, &[("good", 1)]),
            recall(10000, s, &[("good", 2)]),
            recall(15000, s, &[("bad", 9)]),
        ];
        let outcomes = vec![
            outcome(&sid, "t1", 100_000, "completed"),
            outcome(&sid, "t2", 5_000_000, "completed"),
            outcome(&sid, "t3", 10_000_000, "completed"),
            outcome(&sid, "t4", 15_000_000, "failed"),
        ];
        let (tally, detail) =
            correlate_detailed(&recalls, &outcomes, false);
        let props = vec![recall_proposal("good")];
        let d = build_digest(3600, &tally, &detail, &props, None);

        assert_eq!(d.window_secs, 3600);
        assert_eq!(d.recalls_total, 4);
        assert_eq!(d.recalls_scored, 4);
        // good/1 (+2), good/2 (+1) → 2 promoted; bad/9 (−1) not.
        assert_eq!(d.promoted, 2);
        assert_eq!(d.not_promoted, 1);
        assert_eq!(d.top_helpful[0], ("good".to_string(), 3.0));
        assert_eq!(d.top_unhelpful[0], ("bad".to_string(), -1.0));
        assert_eq!(d.proposals_in_window, 1);
    }

    #[test]
    fn empty_inputs_yield_empty_digest() {
        let (tally, detail) = correlate_detailed(&[], &[], false);
        let d = build_digest(60, &tally, &detail, &[], None);
        assert_eq!(d.recalls_total, 0);
        assert_eq!(d.recalls_scored, 0);
        assert_eq!(d.promoted, 0);
        assert!(d.top_helpful.is_empty());
        assert!(d.top_unhelpful.is_empty());
        assert_eq!(d.proposals_in_window, 0);
    }

    #[test]
    fn provenance_reconstructs_only_recall_proposals() {
        let s = SessionId::new();
        let sid = s.to_string();
        let recalls = vec![
            recall(100, s, &[("proj", 1)]),
            recall(5000, s, &[("proj", 1)]),
            recall(9000, s, &[("other", 7)]),
        ];
        let outcomes = vec![
            outcome(&sid, "t1", 100_000, "completed"),
            outcome(&sid, "t2", 5_000_000, "failed"),
            outcome(&sid, "t3", 9_000_000, "completed"),
        ];
        let (_t, detail) = correlate_detailed(&recalls, &outcomes, false);

        // A recall proposal for "proj" + a non-recall proposal
        // that must be ignored.
        let mut foreign = recall_proposal("proj");
        foreign.id = "manual-xyz".into();
        let props = vec![recall_proposal("proj"), foreign];

        let prov = build_provenance(&detail, &props);
        assert_eq!(prov.len(), 1, "only recall-fb: proposals");
        let p = &prov[0];
        assert_eq!(p.proposal_id, "recall-fb:proj");
        assert_eq!(p.topic, "proj");
        assert_eq!(p.status, "pending");
        // proj/1 helpful (+1) then unhelpful (−1) → net 0.
        assert!((p.net_score - 0.0).abs() < 1e-6);
        // Two contributing turns for proj, newest first.
        assert_eq!(p.contributing.len(), 2);
        assert_eq!(p.contributing[0].ts_secs, 5000);
        assert_eq!(p.contributing[0].outcome_kind.as_deref(), Some("failed"));
        assert_eq!(p.contributing[1].ts_secs, 100);
        assert_eq!(p.contributing[1].seqs, vec![1]);
    }

    #[test]
    fn provenance_empty_when_no_proposals() {
        let (_t, detail) = correlate_detailed(&[], &[], false);
        assert!(build_provenance(&detail, &[]).is_empty());
    }
}
