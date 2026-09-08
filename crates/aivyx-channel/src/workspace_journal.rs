//! Chapter O.5 — proactive journaling.
//!
//! A re-arming background task (sibling of [`crate::reflection_scheduler`] and
//! [`crate::reminder_driver`]) that, on a cadence, checks for recent activity
//! and — if there was any — fires a journaling turn so the agent appends to
//! its own workspace journal on its own initiative. The agent's system prompt
//! already tells it about its workspace (O.3) and it holds the `workspace.*`
//! tools (O.2), so the journaling turn just needs a nudge.
//!
//! Bounded: at most one fire per `interval`, and only when the lookback window
//! contains activity — an idle daemon never writes "nothing happened" entries
//! or burns LLM spend.

use std::sync::Arc;
use std::time::Duration;

use aivyx_audit::PersistentAuditLog;
use aivyx_core::CancellationToken;

use crate::reflection_scheduler::{summarize_recent_outcomes, OutcomeSummaryCache};
use crate::trigger::{TriggerDispatch, TriggerSource};

/// The user-message the journaling turn runs under the agent's normal
/// (workspace-aware) system prompt. Deliberately self-directed and quiet — it
/// journals, it does not message the operator.
const JOURNAL_PROMPT: &str = "Take a brief, private moment for yourself. \
    Reflecting on your recent activity, append a short entry to your workspace \
    journal using the `workspace.note` tool: what you worked on, anything you \
    noticed, and any ideas or plans worth keeping for later. Keep it concise \
    and personal. Do not send the operator a message — just journal.";

/// The re-arming journaling loop. Runs until `shutdown` is cancelled.
pub async fn run_workspace_journal_driver(
    dispatch: TriggerDispatch,
    audit_log: Arc<PersistentAuditLog>,
    interval: Duration,
    shutdown: CancellationToken,
) {
    eprintln!(
        "aivyx-pa workspace-journal: proactive journaling every {}s",
        interval.as_secs(),
    );
    let mut cache = OutcomeSummaryCache::new(2);
    loop {
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = shutdown.cancelled() => return,
        }

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        // Lookback = the interval: journal what happened since the last tick.
        let summaries = match summarize_recent_outcomes(
            &audit_log,
            interval.as_secs().max(1),
            now_ms,
            &mut cache,
        )
        .await
        {
            Ok(s) => s,
            Err(e) => {
                eprintln!("aivyx-pa workspace-journal: summarize failed: {e}");
                continue;
            }
        };

        if summaries.is_empty() {
            // Idle window — nothing to reflect on; skip the turn entirely.
            continue;
        }

        eprintln!(
            "aivyx-pa workspace-journal: firing — {} recent outcome(s) to reflect on",
            summaries.len(),
        );
        let _ = dispatch
            .fire(
                TriggerSource::Reflection,
                "workspace-journal",
                JOURNAL_PROMPT,
                false,                            // no mission wrapper
                &[],                              // no notify targets
                aivyx_config::NotifyWhen::Always, // unused (no targets)
            )
            .await;
    }
}
