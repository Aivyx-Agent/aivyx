//! `aivyx learning` CLI — Phase 78.
//!
//! Terminal parity with the Web UI Learning pane. Read-only:
//! a window into what the self-learning loop has learned and
//! why each Pending Persona proposal exists. The render helper
//! is a pure function so unit tests drive it against fixtures
//! without IPC.

use std::path::Path;

use aivyx_channel::daemon_client::{
    daemon_is_running, get_learning_insights,
};
use aivyx_channel::daemon_ipc::default_socket_path;
use aivyx_channel::recall_insights::{
    LearningDigest, ProposalProvenance,
};

/// `aivyx learning [--window <secs>]`
pub async fn run_learning(
    window_secs: Option<u64>,
) -> Result<(), String> {
    let socket_path = default_socket_path()?;
    require_daemon_running(&socket_path).await?;
    let (digest, proposals) =
        get_learning_insights(&socket_path, window_secs)
            .await
            .map_err(|e| {
                format!("failed to fetch learning insights: {e}")
            })?;
    print!("{}", render_insights(&digest, &proposals));
    Ok(())
}

async fn require_daemon_running(
    socket_path: &Path,
) -> Result<(), String> {
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
fn render_insights(
    d: &LearningDigest,
    proposals: &[ProposalProvenance],
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
    out.push_str(&format!(
        "  retention: {} promoted, {} left to age out\n",
        d.promoted, d.not_promoted,
    ));
    out.push_str(&format!(
        "  recall-driven Persona proposals: {}\n",
        d.proposals_in_window,
    ));
    out.push_str("\nMost helpful topics:\n");
    out.push_str(&fmt_topics(&d.top_helpful));
    out.push_str("\nLeast helpful topics:\n");
    out.push_str(&fmt_topics(&d.top_unhelpful));

    if proposals.is_empty() {
        out.push_str(
            "\nNo recall-driven Persona proposals in this window.\n",
        );
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
            let outcome =
                c.outcome_kind.as_deref().unwrap_or("unmatched");
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
        }
    }

    #[test]
    fn render_digest_counts_and_topics() {
        let out = render_insights(&digest(), &[]);
        assert!(out.contains("last 2d — 2 days"));
        assert!(out.contains("10 total, 7 scored"));
        assert!(out.contains("3 promoted, 2 left to age out"));
        assert!(out.contains("+5  project/x"));
        assert!(out.contains("-2  scratch"));
        assert!(out.contains(
            "No recall-driven Persona proposals in this window."
        ));
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
        let out = render_insights(&digest(), &prov);
        assert!(out.contains(
            "recall-fb:project/x [pending] topic 'project/x' net +5"
        ));
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
        };
        let out = render_insights(&d, &[]);
        assert!(out.contains("last 3600s"));
        assert!(out.contains("0 total, 0 scored"));
        assert!(out.contains("Most helpful topics:\n  (none)"));
    }
}
