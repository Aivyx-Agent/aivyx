//! Chapter Whetstone (WH.2) — the per-skill effectiveness ledger.
//!
//! The "did invoking this skill lead to a good turn" signal, made
//! **durable and decayed** — the measurement the WH.3 refinement loop
//! acts on. One row per [`LearnedSkill`](aivyx_ipc::persona::LearnedSkill)
//! name holds a time-decayed EWMA of windowed net effectiveness, folded
//! on the turn boundary from the `SkillInvocation` audit signal + the
//! turn outcome.
//!
//! It is a thin, skill-named wrapper over the proven Phase 82
//! [`PersistentHelpfulnessLedger`](crate::helpfulness_ledger) mechanics
//! (the EWMA decay, the same half-life, the same prune bounds), pointed
//! at the HKDF-isolated
//! [`aivyx_storage::KeyDomain::SkillHelpfulnessLedger`] — so a corrupt
//! row degrades only the refinement signal, never skills, the persona,
//! recall, or any other ledger. Zero-config, passive, no behaviour of
//! its own: it only *measures* (WH.3 reads it to *propose*).

use std::collections::BTreeSet;

use aivyx_audit::{AuditEvent, SignedEntry};
use aivyx_storage::DomainHandle;

use crate::helpfulness_ledger::{
    HelpfulnessLedgerError, LedgerEntry, PersistentHelpfulnessLedger,
};

/// Net folded for each skill invoked in a turn that went well.
pub const SKILL_HELPFUL_NET: f32 = 1.0;
/// Net folded (negatively) for each skill invoked in a turn that did not.
pub const SKILL_UNHELPFUL_NET: f32 = 1.0;

/// Chapter Strop (ST.1) — grade a turn for the effectiveness fold.
/// Before Strop this was `matches!(outcome, Completed)`, and since local
/// models complete nearly every turn — including turns where the skill's
/// result was claimed but never done — every skill's EWMA drifted
/// positive and the WH.3 refinement pass never found an underperformer
/// at the default floor. A turn now folds helpful only when it Completed
/// **without a Candor unfulfilled-claim annotation** (the turn loop's
/// own verdict, embedded in the final message with registry-accurate
/// tool names). Failed/Looping turns stay unhelpful, as before. Pure.
pub fn turn_folds_helpful(outcome: &aivyx_core::TurnOutcome) -> bool {
    match outcome {
        aivyx_core::TurnOutcome::Completed { final_message, .. } => {
            !aivyx_core::claim_check::has_unfulfilled_claim_annotation(
                final_message,
            )
        }
        _ => false,
    }
}

/// Durable, decayed per-skill effectiveness ledger over
/// [`aivyx_storage::KeyDomain::SkillHelpfulnessLedger`]. Key = skill
/// name; value = the shared [`LedgerEntry`] (decayed EWMA + samples).
pub struct SkillEffectivenessLedger {
    inner: PersistentHelpfulnessLedger,
}

impl SkillEffectivenessLedger {
    pub fn new(storage: DomainHandle) -> Self {
        Self { inner: PersistentHelpfulnessLedger::new(storage) }
    }

    /// Fold one turn-boundary window's per-skill net into the durable
    /// ledger (decay-then-add, the Phase 82 semantics).
    pub async fn record_window(
        &self,
        net_by_skill: &[(String, f32)],
        now_secs: u64,
    ) -> Result<(), HelpfulnessLedgerError> {
        self.inner.record_window(net_by_skill, now_secs).await
    }

    /// One skill's effectiveness, decayed to `now`.
    pub async fn skill_score(
        &self,
        skill: &str,
        now_secs: u64,
    ) -> Result<Option<LedgerEntry>, HelpfulnessLedgerError> {
        self.inner.topic_score(skill, now_secs).await
    }

    /// Every skill, decayed to `now`, score-descending.
    pub async fn ranked(
        &self,
        now_secs: u64,
    ) -> Result<Vec<(String, LedgerEntry)>, HelpfulnessLedgerError> {
        self.inner.ranked(now_secs).await
    }

    /// Chapter Whetstone — the skills the WH.3 loop should consider
    /// refining: decayed effectiveness strictly **below** `floor` and at
    /// least `min_samples` folded windows (so one bad turn never triggers
    /// a refinement). Returned worst-first (most negative score), capped
    /// by the caller. A confidence-gated, recency-weighted underperformer
    /// query — the whole point of the ledger.
    pub async fn underperformers(
        &self,
        floor: f32,
        min_samples: u32,
        now_secs: u64,
    ) -> Result<Vec<(String, LedgerEntry)>, HelpfulnessLedgerError> {
        let mut rows: Vec<(String, LedgerEntry)> = self
            .ranked(now_secs)
            .await?
            .into_iter()
            .filter(|(_, e)| e.ewma_score < floor && e.samples >= min_samples)
            .collect();
        // `ranked` is score-descending; worst-first is the useful order here.
        rows.reverse();
        Ok(rows)
    }
}

/// Chapter Whetstone — fold one finished turn's skill effectiveness.
///
/// Extracts the **distinct** skills the turn invoked (its
/// `SkillInvocation` audit entries) and folds each by the turn outcome:
/// `+SKILL_HELPFUL_NET` when `helpful`, `-SKILL_UNHELPFUL_NET` otherwise.
/// A turn that invoked no skill folds nothing. Failure-isolated — a
/// ledger error is logged and swallowed, never touching the turn (the
/// turn is already committed by the time this runs).
pub async fn record_turn_skills(
    ledger: &SkillEffectivenessLedger,
    audit_entries: &[SignedEntry],
    helpful: bool,
    now_secs: u64,
) {
    let mut skills: BTreeSet<String> = BTreeSet::new();
    for entry in audit_entries {
        if let AuditEvent::SkillInvocation { skill_name, .. } = &entry.event {
            skills.insert(skill_name.clone());
        }
    }
    if skills.is_empty() {
        return;
    }
    let net = if helpful { SKILL_HELPFUL_NET } else { -SKILL_UNHELPFUL_NET };
    let folds: Vec<(String, f32)> =
        skills.into_iter().map(|s| (s, net)).collect();
    if let Err(e) = ledger.record_window(&folds, now_secs).await {
        eprintln!("aivyx skill-effectiveness: record_window failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_crypto::MasterKey;
    use aivyx_storage::{KeyDomain, RedbStorage, StorageConfig};

    // ---- Chapter Strop (ST.1) — the graded fold ---------------------

    #[test]
    fn completed_clean_turn_folds_helpful() {
        let outcome = aivyx_core::TurnOutcome::Completed {
            final_message: "Saved to memory under coffee-preferences.".into(),
            tool_calls_made: 1,
            duration: std::time::Duration::from_secs(1),
        };
        assert!(turn_folds_helpful(&outcome));
    }

    #[test]
    fn candor_annotated_completion_folds_unhelpful() {
        // The dogfood shape: the model invoked a skill, CLAIMED the
        // save, never called the tool — the turn still Completed, and
        // Candor appended its note. Pre-Strop this folded +1.
        let notes = aivyx_core::claim_check::detect_unfulfilled_claims(
            "Done — I saved that to memory for you.",
            &[String::from("web.search")],
        );
        assert_eq!(notes.len(), 1, "fixture must trip a real rule");
        let outcome = aivyx_core::TurnOutcome::Completed {
            final_message: format!(
                "Done — I saved that to memory for you.\n\n⚠ {}",
                notes[0]
            ),
            tool_calls_made: 1,
            duration: std::time::Duration::from_secs(1),
        };
        assert!(!turn_folds_helpful(&outcome));
    }

    #[test]
    fn non_completed_turns_stay_unhelpful() {
        let looping = aivyx_core::TurnOutcome::Looping {
            final_message: "…".into(),
            tool_calls_made: 9,
            duration: std::time::Duration::from_secs(30),
            repeat_limit: 3,
        };
        assert!(!turn_folds_helpful(&looping));
        let failed = aivyx_core::TurnOutcome::Failed(
            aivyx_core::AivyxError::Internal("provider down".into()),
        );
        assert!(!turn_folds_helpful(&failed));
    }

    async fn ledger() -> SkillEffectivenessLedger {
        let base = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let dir = std::path::PathBuf::from(base)
            .join(format!("aivyx-skilleff-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = RedbStorage::open(
            StorageConfig::new(dir.join("s.redb")),
            MasterKey::from_raw([7u8; 32]),
        )
        .await
        .unwrap();
        SkillEffectivenessLedger::new(
            store.domain(KeyDomain::SkillHelpfulnessLedger),
        )
    }

    fn skill_entry(seq: u64, name: &str) -> SignedEntry {
        // A SkillInvocation audit entry for `name`; only skill_name
        // matters to the fold, the rest are filler.
        SignedEntry {
            seq,
            appended_at: std::time::SystemTime::now(),
            event: AuditEvent::SkillInvocation {
                turn_id: aivyx_core::TurnId::new(),
                session_id: aivyx_core::SessionId::new(),
                skill_name: name.to_string(),
            },
            mac: [0u8; 32],
            prev_mac: [0u8; 32],
        }
    }

    #[tokio::test]
    async fn fold_accumulates_and_decays() {
        let l = ledger().await;
        // Two good turns, then one bad, for "checklist".
        l.record_window(&[("checklist".into(), 1.0)], 1000).await.unwrap();
        l.record_window(&[("checklist".into(), 1.0)], 2000).await.unwrap();
        let s = l.skill_score("checklist", 2000).await.unwrap().unwrap();
        assert!(s.ewma_score > 1.9, "two +1 folds accumulate: {}", s.ewma_score);
        assert_eq!(s.samples, 2);
        // A far-future read decays the score toward zero.
        let later = 2000 + crate::helpfulness_ledger::HELPFULNESS_HALF_LIFE_SECS;
        let decayed = l.skill_score("checklist", later).await.unwrap().unwrap();
        assert!(decayed.ewma_score < s.ewma_score, "decays with time");
    }

    #[tokio::test]
    async fn underperformers_are_confidence_gated() {
        let l = ledger().await;
        // "bad" — three negative folds → well-sampled underperformer.
        for t in [1000u64, 2000, 3000] {
            l.record_window(&[("bad".into(), -1.0)], t).await.unwrap();
        }
        // "thin" — one negative fold → below floor but under-sampled.
        l.record_window(&[("thin".into(), -1.0)], 1000).await.unwrap();
        // "good" — positive, never an underperformer.
        l.record_window(&[("good".into(), 2.0)], 1000).await.unwrap();

        let under = l.underperformers(0.0, 3, 3000).await.unwrap();
        let names: Vec<_> = under.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, vec!["bad"], "only the well-sampled negative skill");
    }

    #[tokio::test]
    async fn record_turn_skills_folds_distinct_by_outcome() {
        let l = ledger().await;
        // A bad turn that invoked "checklist" twice + "deploy" once.
        let entries = vec![
            skill_entry(0, "checklist"),
            skill_entry(1, "deploy"),
            skill_entry(2, "checklist"),
        ];
        record_turn_skills(&l, &entries, false, 1000).await;
        // Each distinct skill folded exactly once, negatively.
        let c = l.skill_score("checklist", 1000).await.unwrap().unwrap();
        assert_eq!(c.samples, 1, "deduped — not folded per invocation");
        assert!(c.ewma_score < 0.0, "bad turn → negative");
        assert!(l.skill_score("deploy", 1000).await.unwrap().unwrap().ewma_score < 0.0);

        // A turn with no skills folds nothing.
        record_turn_skills(&l, &[], true, 2000).await;
        assert!(l.skill_score("nope", 2000).await.unwrap().is_none());
    }
}
