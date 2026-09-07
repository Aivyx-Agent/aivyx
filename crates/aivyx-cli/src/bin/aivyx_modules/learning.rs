//! `aivyx learning` CLI — Phase 78.
//!
//! Terminal parity with the Web UI Learning pane. Read-only:
//! a window into what the self-learning loop has learned and
//! why each Pending Persona proposal exists. The render helper
//! is a pure function so unit tests drive it against fixtures
//! without IPC.

use std::path::Path;

use aivyx_channel::cooccurrence_ledger::CooccurrencePatterns;
use aivyx_channel::correction_consolidation::CorrectionConsolidationStat;
use aivyx_channel::correction_ledger::AccumulatedCorrections;
use aivyx_channel::daemon_client::{daemon_is_running, get_learning_insights};
use aivyx_channel::daemon_ipc::default_socket_path;
use aivyx_channel::helpfulness_ledger::AccumulatedHelpfulness;
use aivyx_channel::memory_recall::RecallClusterStat;
use aivyx_channel::persona_consolidation::PersonaConsolidationStat;
use aivyx_channel::persona_context::PersonaSelectionStat;
use aivyx_channel::persona_lifecycle::PersonaLifecycleStat;
use aivyx_channel::proactive_detect::ProactiveStat;
use aivyx_channel::recall_insights::{LearningDigest, ProposalProvenance};
use aivyx_channel::recall_judgment::{RecallJudgment, RecallJudgmentStat};
use aivyx_channel::reflection_scheduler::RecentReflectionStat;

/// `aivyx learning [--window <secs>]`
pub async fn run_learning(window_secs: Option<u64>) -> Result<(), String> {
    let socket_path = default_socket_path()?;
    require_daemon_running(&socket_path).await?;
    let (
        digest,
        proposals,
        persona_selection,
        proactive,
        persona_lifecycle,
        accumulated_helpfulness,
        cooccurrence,
        cluster_recall,
        persona_consolidation,
        accumulated_corrections,
        correction_consolidation,
        correction_judgment,
        recall_judgment,
        cadence,
    ) = get_learning_insights(&socket_path, window_secs)
        .await
        .map_err(|e| format!("failed to fetch learning insights: {e}"))?;
    print!(
        "{}",
        render_insights(
            &digest,
            &proposals,
            persona_selection.as_ref(),
            proactive.as_ref(),
            persona_lifecycle.as_ref(),
            accumulated_helpfulness.as_ref(),
            cooccurrence.as_ref(),
            cluster_recall.as_ref(),
            persona_consolidation.as_ref(),
            recall_judgment.as_ref(),
            &cadence,
            accumulated_corrections.as_ref(),
            correction_consolidation.as_ref(),
            correction_judgment.as_ref(),
        )
    );
    Ok(())
}

async fn require_daemon_running(socket_path: &Path) -> Result<(), String> {
    if daemon_is_running(socket_path).await {
        return Ok(());
    }
    Err(format!(
        "aivyx learning: no daemon running on socket {} — \
         start the daemon first with `aivyx daemon run`",
        socket_path.display(),
    ))
}

fn fmt_topics(pairs: &[(String, f32)]) -> String {
    if pairs.is_empty() {
        return "  (none)\n".to_string();
    }
    let mut s = String::new();
    for (topic, score) in pairs {
        s.push_str(&format!("  {score:+.0}  {topic}\n"));
    }
    s
}

/// Pure renderer — terminal text for the digest + provenance.
// One read-only display param accretes per learning phase
// (79/80/81/82/83/84…); a params struct would just move the
// churn. Consistent with the codebase's existing
// `#[allow(too_many_arguments)]` (Phase 81 fire_reflection).
#[allow(clippy::too_many_arguments)]
fn render_insights(
    d: &LearningDigest,
    proposals: &[ProposalProvenance],
    persona_selection: Option<&PersonaSelectionStat>,
    proactive: Option<&ProactiveStat>,
    persona_lifecycle: Option<&PersonaLifecycleStat>,
    accumulated: Option<&AccumulatedHelpfulness>,
    cooccurrence: Option<&CooccurrencePatterns>,
    cluster_recall: Option<&RecallClusterStat>,
    persona_consolidation: Option<&PersonaConsolidationStat>,
    recall_judgment: Option<&RecallJudgmentStat>,
    cadence: &[(String, RecentReflectionStat)],
    accumulated_corrections: Option<&AccumulatedCorrections>,
    correction_consolidation: Option<&CorrectionConsolidationStat>,
    correction_judgment: Option<&aivyx_channel::correction_judgment::CorrectionJudgmentStat>,
) -> String {
    let mut out = String::new();
    let days = d.window_secs / 86_400;
    out.push_str(&format!(
        "Learning insights (last {} — {} day{})\n",
        if days >= 1 {
            format!("{days}d")
        } else {
            format!("{}s", d.window_secs)
        },
        days,
        if days == 1 { "" } else { "s" },
    ));
    out.push_str(&format!(
        "  recalls: {} total, {} scored\n",
        d.recalls_total, d.recalls_scored,
    ));
    // Phase 93 — only surfaces when the operator opted into
    // judgment-driven recall feedback. Pre-Phase-93 digests
    // (and operators who haven't enabled the augment) carry
    // `None` and the line is omitted.
    if let Some(true) = d.judgment_signal {
        out.push_str(
            "    signal source: judgment-driven \
             (Phase 93 — augmenting structural)\n",
        );
    }
    out.push_str(&format!(
        "  retention: {} promoted, {} left to age out\n",
        d.promoted, d.not_promoted,
    ));
    out.push_str(&format!(
        "  recall-driven Persona proposals: {}\n",
        d.proposals_in_window,
    ));
    match persona_selection {
        Some(p) => out.push_str(&format!(
            "  adaptive Persona: {}/{} facets injected last turn\n",
            p.selected, p.total,
        )),
        None => out.push_str(
            "  adaptive Persona: not engaged (no [embedding] / \
             small Soul / none yet)\n",
        ),
    }
    match proactive {
        Some(p) => {
            out.push_str(&format!(
                "  proactive: {} surfaced last cycle \
                 ({} deduped, {} capped)\n",
                p.surfaced.len(),
                p.deduped,
                p.capped,
            ));
            for s in &p.surfaced {
                out.push_str(&format!(
                    "    - {:?} '{}' — {}\n",
                    s.kind, s.topic, s.reason,
                ));
            }
        }
        None => out.push_str("  proactive: not engaged (off, or no cycle yet)\n"),
    }
    match persona_lifecycle {
        Some(p) => {
            out.push_str(&format!(
                "  persona lifecycle: {} proposed last cycle \
                 ({} deduped)\n",
                p.proposed.len(),
                p.deduped,
            ));
            for pr in &p.proposed {
                out.push_str(&format!(
                    "    - {} {} '{}' — {}\n",
                    pr.kind,
                    pr.category.label(),
                    pr.value,
                    pr.reason,
                ));
            }
        }
        None => out.push_str(
            "  persona lifecycle: not engaged \
             (off, or no cycle yet)\n",
        ),
    }
    // Phase 95 — reflection-cron cadence. Renders one line
    // per schedule with non-zero stats. Schedules with both
    // counts at zero (or no entry at all) are omitted to
    // avoid noise for operators who haven't enabled
    // skip-when-idle.
    let cadence_visible: Vec<&(String, RecentReflectionStat)> = cadence
        .iter()
        .filter(|(_, s)| s.fired > 0 || s.skipped > 0)
        .collect();
    if !cadence_visible.is_empty() {
        out.push_str("\nReflection cadence (Phase 95):\n");
        for (name, stat) in &cadence_visible {
            out.push_str(&format!(
                "  {name}: {} fired, {} skipped\n",
                stat.fired, stat.skipped,
            ));
        }
    }

    out.push_str("\nMost helpful topics:\n");
    out.push_str(&fmt_topics(&d.top_helpful));
    out.push_str("\nLeast helpful topics:\n");
    out.push_str(&fmt_topics(&d.top_unhelpful));

    out.push_str("\nAccumulated helpfulness (all-time, decayed):\n");
    match accumulated {
        Some(a) if !a.top_helpful.is_empty() || !a.top_unhelpful.is_empty() => {
            for t in &a.top_helpful {
                out.push_str(&format!(
                    "  {:+.1}  {}  ({} sample{})\n",
                    t.score,
                    t.topic,
                    t.samples,
                    if t.samples == 1 { "" } else { "s" },
                ));
            }
            for t in &a.top_unhelpful {
                out.push_str(&format!(
                    "  {:+.1}  {}  ({} sample{})\n",
                    t.score,
                    t.topic,
                    t.samples,
                    if t.samples == 1 { "" } else { "s" },
                ));
            }
        }
        _ => out.push_str("  (none yet)\n"),
    }

    out.push_str("\nTopics that consistently help together:\n");
    match cooccurrence {
        Some(c) if !c.top_pairs.is_empty() => {
            for p in &c.top_pairs {
                out.push_str(&format!(
                    "  {:+.1}  {} + {}  ({} sample{})\n",
                    p.score,
                    p.a,
                    p.b,
                    p.samples,
                    if p.samples == 1 { "" } else { "s" },
                ));
            }
        }
        _ => out.push_str("  (none yet)\n"),
    }

    out.push_str("\nCluster co-recall (last turn, opt-in):\n");
    match cluster_recall {
        Some(c) if c.injected > 0 => {
            out.push_str(&format!("  {} affined sibling(s) injected\n", c.injected,));
            for (driver, sib) in &c.pairs {
                out.push_str(&format!("    {driver} → {sib}\n"));
            }
        }
        Some(_) => out.push_str("  engaged, 0 injected last turn\n"),
        None => out.push_str("  not engaged (off, or no turn yet)\n"),
    }

    out.push_str("\nPattern-driven Persona proposals (last cycle, opt-in):\n");
    match persona_consolidation {
        Some(c) if c.filed > 0 => {
            // Phase 92 — surface the supersession count
            // when non-zero. Each supersession event lands
            // two chain entries already included in `filed`.
            let supersede_note = if c.superseded > 0 {
                format!(", superseded {}", c.superseded)
            } else {
                String::new()
            };
            out.push_str(&format!("  {} filed last cycle{supersede_note}\n", c.filed,));
            for (a, b) in &c.pairs {
                out.push_str(&format!("    {a} + {b}\n"));
            }
        }
        Some(c) if c.llm_unavailable => out.push_str(
            "  engaged, 0 filed last cycle \
             (LLM unavailable)\n",
        ),
        Some(_) => out.push_str("  engaged, 0 filed last cycle\n"),
        None => out.push_str("  not engaged (off, or no cycle yet)\n"),
    }

    // Phase 172 — the durable correction view (topics the
    // operator most often reworks) + the last correction-
    // consolidation cycle.
    out.push_str("\nMost-reworked topics (accumulated):\n");
    match accumulated_corrections {
        Some(a) if !a.top_corrected.is_empty() => {
            for tc in &a.top_corrected {
                out.push_str(&format!(
                    "  {:.1}  {} ({} window{})\n",
                    tc.count,
                    tc.topic,
                    tc.samples,
                    if tc.samples == 1 { "" } else { "s" },
                ));
            }
        }
        _ => out.push_str("  (none yet)\n"),
    }

    out.push_str("\nCorrection-driven Persona proposals (last cycle, opt-in):\n");
    match correction_consolidation {
        Some(c) if c.filed > 0 => {
            out.push_str(&format!("  {} filed last cycle\n", c.filed,));
            for topic in &c.topics {
                out.push_str(&format!("    {topic}\n"));
            }
        }
        Some(c) if c.llm_unavailable => {
            out.push_str("  engaged, 0 filed last cycle (LLM unavailable)\n")
        }
        Some(_) => out.push_str("  engaged, 0 filed last cycle\n"),
        None => out.push_str("  not engaged (off, or no cycle yet)\n"),
    }

    // Phase 178 — LLM-judged correction classification.
    out.push_str("\nLLM-judged corrections (last cycle, opt-in):\n");
    match correction_judgment {
        Some(c) if c.judged > 0 || c.structural_fallback > 0 => {
            out.push_str(&format!(
                "  {} judged (rework {}, praise {}, unrelated {}), \
                 {} structural{}\n",
                c.judged,
                c.rework,
                c.praise,
                c.unrelated,
                c.structural_fallback,
                if c.llm_unavailable {
                    " (LLM unavailable)"
                } else {
                    ""
                },
            ));
        }
        _ => out.push_str("  not engaged (off, or no corrections this cycle)\n"),
    }

    out.push_str("\nLLM-judged recall usefulness (last cycle, opt-in):\n");
    match recall_judgment {
        Some(j) if j.judged > 0 => {
            out.push_str(&format!(
                "  {} judged last cycle (used={}, \
                 irrelevant={}, hurt={}, skipped={})\n",
                j.judged, j.used, j.irrelevant, j.hurt, j.skipped,
            ));
            for (topic, judgment) in &j.pairs {
                let label = match judgment {
                    RecallJudgment::Used => "used",
                    RecallJudgment::Irrelevant => "irrelevant",
                    RecallJudgment::Hurt => "hurt",
                };
                out.push_str(&format!("    {topic} → {label}\n",));
            }
        }
        Some(j) if j.llm_unavailable => out.push_str(
            "  engaged, 0 judged last cycle \
             (LLM unavailable)\n",
        ),
        Some(j) => out.push_str(&format!(
            "  engaged, 0 judged last cycle ({} skipped)\n",
            j.skipped,
        )),
        None => out.push_str("  not engaged (off, or no cycle yet)\n"),
    }

    if proposals.is_empty() {
        out.push_str("\nNo recall-driven Persona proposals in this window.\n");
        return out;
    }
    out.push_str("\nProposal provenance:\n");
    for p in proposals {
        out.push_str(&format!(
            "  {} [{}] topic '{}' net {:+.0}\n",
            p.proposal_id, p.status, p.topic, p.net_score,
        ));
        if let Some(reason) = &p.reason {
            out.push_str(&format!("    reason: {reason}\n"));
        }
        for c in &p.contributing {
            let sig = match c.signal {
                Some(s) if s > 0.0 => "helped",
                Some(_) => "hurt",
                None => "no-signal",
            };
            let outcome = c.outcome_kind.as_deref().unwrap_or("unmatched");
            out.push_str(&format!(
                "    - t={} {outcome} ({sig}) seqs={:?}\n",
                c.ts_secs, c.seqs,
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_channel::recall_insights::ContributingTurn;

    fn digest() -> LearningDigest {
        LearningDigest {
            window_secs: 172_800,
            recalls_total: 10,
            recalls_scored: 7,
            promoted: 3,
            not_promoted: 2,
            top_helpful: vec![("project/x".into(), 5.0)],
            top_unhelpful: vec![("scratch".into(), -2.0)],
            proposals_in_window: 1,
            judgment_signal: None,
        }
    }

    #[test]
    fn render_digest_counts_and_topics() {
        let out = render_insights(
            &digest(),
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
        );
        assert!(out.contains("last 2d — 2 days"));
        assert!(out.contains("10 total, 7 scored"));
        assert!(out.contains("3 promoted, 2 left to age out"));
        assert!(out.contains("+5  project/x"));
        assert!(out.contains("-2  scratch"));
        assert!(out.contains("No recall-driven Persona proposals in this window."));
    }

    /// Phase 93 — the judgment-signal banner renders only
    /// when the operator has opted in
    /// (`judgment_signal = Some(true)`); the default-off and
    /// pre-Phase-93 paths emit the line unchanged.
    #[test]
    fn render_judgment_signal_banner() {
        let mut d = digest();
        // Default (None) — banner absent.
        let out_none = render_insights(
            &d,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
        );
        assert!(!out_none.contains("signal source"));
        // Explicit-off (Some(false)) — banner still absent
        // (the operator declared but the knob is off; no
        // augment in effect).
        d.judgment_signal = Some(false);
        let out_off = render_insights(
            &d,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
        );
        assert!(!out_off.contains("signal source"));
        // Augment on — the banner fires.
        d.judgment_signal = Some(true);
        let out_on = render_insights(
            &d,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
        );
        assert!(
            out_on.contains("signal source: judgment-driven (Phase 93 — augmenting structural)")
        );
    }

    #[test]
    fn render_provenance_block() {
        let prov = vec![ProposalProvenance {
            proposal_id: "recall-fb:project/x".into(),
            topic: "project/x".into(),
            status: "pending".into(),
            net_score: 5.0,
            reason: Some("net +5".into()),
            contributing: vec![
                ContributingTurn {
                    ts_secs: 1_715_000_000,
                    outcome_kind: Some("completed".into()),
                    signal: Some(1.0),
                    seqs: vec![9],
                },
                ContributingTurn {
                    ts_secs: 1_714_000_000,
                    outcome_kind: Some("failed".into()),
                    signal: Some(-1.0),
                    seqs: vec![3],
                },
            ],
        }];
        let out = render_insights(
            &digest(),
            &prov,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
        );
        assert!(out.contains("recall-fb:project/x [pending] topic 'project/x' net +5"));
        assert!(out.contains("reason: net +5"));
        assert!(out.contains("completed (helped) seqs=[9]"));
        assert!(out.contains("failed (hurt) seqs=[3]"));
    }

    #[test]
    fn render_empty_window_is_graceful() {
        let d = LearningDigest {
            window_secs: 3600,
            recalls_total: 0,
            recalls_scored: 0,
            promoted: 0,
            not_promoted: 0,
            top_helpful: vec![],
            top_unhelpful: vec![],
            proposals_in_window: 0,
            judgment_signal: None,
        };
        let out = render_insights(
            &d,
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
        );
        assert!(out.contains("last 3600s"));
        assert!(out.contains("0 total, 0 scored"));
        assert!(out.contains("Most helpful topics:\n  (none)"));
        assert!(out.contains("LLM-judged corrections"));
        assert!(out.contains("not engaged"));
    }

    #[test]
    fn render_correction_judgment_stat() {
        use aivyx_channel::correction_judgment::CorrectionJudgmentStat;
        let stat = CorrectionJudgmentStat {
            ts_secs: 1,
            judged: 5,
            rework: 3,
            praise: 1,
            unrelated: 1,
            structural_fallback: 2,
            llm_unavailable: false,
        };
        let out = render_insights(
            &digest(),
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            None,
            None,
            Some(&stat),
        );
        assert!(out.contains("5 judged (rework 3, praise 1, unrelated 1), 2 structural"));
    }

    /// Phase 172 — the most-reworked-topics view + the
    /// correction-driven proposal block render in both the
    /// None (not engaged / none yet) and Some shapes.
    #[test]
    fn render_corrections_some_and_none() {
        use aivyx_channel::correction_consolidation::CorrectionConsolidationStat;
        use aivyx_channel::correction_ledger::{AccumulatedCorrections, TopicCorrections};

        // None → "(none yet)" + "not engaged".
        let none_out = render_insights(
            &digest(),
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
        );
        assert!(none_out.contains("Most-reworked topics (accumulated):"));
        assert!(none_out.contains("(none yet)"));
        assert!(none_out.contains("Correction-driven Persona proposals (last cycle, opt-in):"));

        // Some → decayed count + topic + samples, and a filed
        // proposal line.
        let acc = AccumulatedCorrections {
            top_corrected: vec![TopicCorrections {
                topic: "auth".into(),
                count: 4.0,
                samples: 3,
            }],
        };
        let stat = CorrectionConsolidationStat {
            ts_secs: 1_715_006_000,
            filed: 1,
            llm_unavailable: false,
            topics: vec!["auth".into()],
        };
        let some_out = render_insights(
            &digest(),
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            Some(&acc),
            Some(&stat),
            None,
        );
        assert!(some_out.contains("4.0  auth (3 windows)"));
        assert!(some_out.contains("1 filed last cycle"));
        assert!(some_out.contains("    auth\n"));
    }

    #[test]
    fn render_persona_selection_some_and_none() {
        // None → "not engaged" line.
        let none_out = render_insights(
            &digest(),
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
        );
        assert!(none_out.contains("adaptive Persona: not engaged"));

        // Some → selected/total.
        let stat = PersonaSelectionStat {
            ts_secs: 1_715_002_000,
            selected: 6,
            total: 20,
        };
        let some_out = render_insights(
            &digest(),
            &[],
            Some(&stat),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
        );
        assert!(some_out.contains("adaptive Persona: 6/20 facets injected last turn"));
    }

    #[test]
    fn render_proactive_some_and_none() {
        use aivyx_channel::proactive_detect::{ProactiveKind, ProactiveStat, ProactiveSurfaced};

        // None → "not engaged" line.
        let none_out = render_insights(
            &digest(),
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
        );
        assert!(none_out.contains("proactive: not engaged"));

        // Some → count line + per-item lines.
        let stat = ProactiveStat {
            ts_secs: 1_715_003_000,
            surfaced: vec![ProactiveSurfaced {
                kind: ProactiveKind::DueReminder,
                topic: "reminders".into(),
                reason: "1 item due".into(),
            }],
            deduped: 2,
            capped: 1,
        };
        let some_out = render_insights(
            &digest(),
            &[],
            None,
            Some(&stat),
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
        );
        assert!(some_out.contains("proactive: 1 surfaced last cycle (2 deduped, 1 capped)"));
        assert!(some_out.contains("DueReminder 'reminders' — 1 item due"));
    }

    #[test]
    fn render_persona_lifecycle_some_and_none() {
        use aivyx_channel::persona_lifecycle::{
            PersonaLifecycleProposed, PersonaLifecycleStat, SoftCategory,
        };

        // None → "not engaged" line.
        let none_out = render_insights(
            &digest(),
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
        );
        assert!(none_out.contains("persona lifecycle: not engaged"));

        // Some → count line + per-proposal lines.
        let stat = PersonaLifecycleStat {
            ts_secs: 1_715_004_000,
            proposed: vec![PersonaLifecycleProposed {
                kind: "consolidate".into(),
                category: SoftCategory::LearnedContext,
                value: "dup a".into(),
                reason: "2 near-duplicate facets".into(),
            }],
            deduped: 3,
        };
        let some_out = render_insights(
            &digest(),
            &[],
            None,
            None,
            Some(&stat),
            None,
            None,
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
        );
        assert!(some_out.contains(
            "persona lifecycle: 1 proposed last cycle \
             (3 deduped)"
        ));
        assert!(some_out.contains(
            "consolidate learned_context 'dup a' — \
             2 near-duplicate facets"
        ));
    }

    #[test]
    fn render_accumulated_helpfulness_some_and_none() {
        use aivyx_channel::helpfulness_ledger::{AccumulatedHelpfulness, TopicScore};

        // None → header + "(none yet)".
        let none_out = render_insights(
            &digest(),
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
        );
        assert!(none_out.contains("Accumulated helpfulness (all-time, decayed):"));
        assert!(none_out.contains("(none yet)"));

        // Some → signed score + topic + sample count.
        let acc = AccumulatedHelpfulness {
            top_helpful: vec![TopicScore {
                topic: "rust".into(),
                score: 4.5,
                samples: 2,
            }],
            top_unhelpful: vec![TopicScore {
                topic: "scratch".into(),
                score: -3.0,
                samples: 1,
            }],
        };
        let some_out = render_insights(
            &digest(),
            &[],
            None,
            None,
            None,
            Some(&acc),
            None,
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
        );
        assert!(some_out.contains("+4.5  rust  (2 samples)"));
        assert!(some_out.contains("-3.0  scratch  (1 sample)"));
    }

    #[test]
    fn render_cooccurrence_some_and_none() {
        use aivyx_channel::cooccurrence_ledger::{CooccurrencePatterns, PairScore};

        // None → header + "(none yet)".
        let none_out = render_insights(
            &digest(),
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
        );
        assert!(none_out.contains("Topics that consistently help together:"));

        // Some → signed score + "a + b" + sample count.
        let cooc = CooccurrencePatterns {
            top_pairs: vec![PairScore {
                a: "deploy".into(),
                b: "rollback".into(),
                score: 8.0,
                samples: 5,
            }],
        };
        let some_out = render_insights(
            &digest(),
            &[],
            None,
            None,
            None,
            None,
            Some(&cooc),
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
        );
        assert!(some_out.contains("+8.0  deploy + rollback  (5 samples)"));
    }

    /// Phase 95 — the reflection-cadence block renders one
    /// line per schedule with non-zero stats, and is omitted
    /// entirely when the cadence slice is empty OR all
    /// schedules have zero counts.
    #[test]
    fn render_reflection_cadence_block() {
        // Empty → no cadence section.
        let out_empty = render_insights(
            &digest(),
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
        );
        assert!(!out_empty.contains("Reflection cadence"));

        // All-zero schedule → also omitted.
        let zero = vec![(
            "nightly".to_string(),
            RecentReflectionStat {
                fired: 0,
                skipped: 0,
            },
        )];
        let out_zero = render_insights(
            &digest(),
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &zero,
            None,
            None,
            None,
        );
        assert!(!out_zero.contains("Reflection cadence"));

        // Mixed: one schedule with activity, one without →
        // only the active one renders.
        let mixed = vec![
            (
                "nightly".to_string(),
                RecentReflectionStat {
                    fired: 7,
                    skipped: 2,
                },
            ),
            (
                "hourly".to_string(),
                RecentReflectionStat {
                    fired: 0,
                    skipped: 0,
                },
            ),
        ];
        let out_mixed = render_insights(
            &digest(),
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &mixed,
            None,
            None,
            None,
        );
        assert!(out_mixed.contains("Reflection cadence (Phase 95):"));
        assert!(out_mixed.contains("nightly: 7 fired, 2 skipped"));
        assert!(
            !out_mixed.contains("hourly:"),
            "hourly schedule with zero counts must be omitted"
        );
    }

    #[test]
    fn render_cluster_recall_some_and_none() {
        use aivyx_channel::memory_recall::RecallClusterStat;

        // None → "not engaged".
        let none_out = render_insights(
            &digest(),
            &[],
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
        );
        assert!(none_out.contains("Cluster co-recall (last turn, opt-in):"));
        assert!(none_out.contains("not engaged (off, or no turn yet)"));

        // Some with injections → count + driver→sibling pairs.
        let cr = RecallClusterStat {
            ts_secs: 1_715_005_000,
            injected: 1,
            pairs: vec![("deploy".into(), "rollback".into())],
        };
        let some_out = render_insights(
            &digest(),
            &[],
            None,
            None,
            None,
            None,
            None,
            Some(&cr),
            None,
            None,
            &[],
            None,
            None,
            None,
        );
        assert!(some_out.contains("1 affined sibling(s) injected"));
        assert!(some_out.contains("deploy → rollback"));

        // Some but zero injected → "engaged, 0 injected".
        let cr0 = RecallClusterStat {
            ts_secs: 1,
            injected: 0,
            pairs: vec![],
        };
        let z = render_insights(
            &digest(),
            &[],
            None,
            None,
            None,
            None,
            None,
            Some(&cr0),
            None,
            None,
            &[],
            None,
            None,
            None,
        );
        assert!(z.contains("engaged, 0 injected last turn"));
    }
}
