//! `aivyx loop` CLI — Phase 173 (the Aivyx Ralph loop).
//!
//! IPC-backed control surface for the autonomous loop: stock the
//! backlog (`add` / `list`) and drive runs (`start` / `stop` /
//! `status`). All five subcommands talk to the running daemon —
//! the backlog is daemon-owned, and `start` flips the daemon's
//! shared run state. The render helpers are pure functions so
//! they unit-test against fixtures without IPC.

use std::path::Path;

use aivyx_channel::daemon_client::{
    daemon_is_running, loop_add, loop_list, loop_log, loop_skip,
    loop_start, loop_status, loop_stop,
};
use aivyx_channel::daemon_ipc::default_socket_path;
use aivyx_channel::loop_backlog::{Story, StoryStatus};
use aivyx_channel::loop_driver::LoopRunState;

use crate::LoopSubcommand;

/// `aivyx loop <subcommand>`.
pub async fn run_loop(sub: LoopSubcommand) -> Result<(), String> {
    let socket_path = default_socket_path()?;
    require_daemon_running(&socket_path).await?;

    match sub {
        LoopSubcommand::Add {
            title,
            body,
            priority,
        } => {
            let id = loop_add(&socket_path, title, body, priority)
                .await
                .map_err(|e| format!("loop add failed: {e}"))?;
            println!("added story {id}");
            Ok(())
        }
        LoopSubcommand::List => {
            let stories = loop_list(&socket_path)
                .await
                .map_err(|e| format!("loop list failed: {e}"))?;
            print!("{}", render_backlog(&stories));
            Ok(())
        }
        LoopSubcommand::Start { max_iterations } => {
            let (ok, message) = loop_start(&socket_path, max_iterations)
                .await
                .map_err(|e| format!("loop start failed: {e}"))?;
            println!("{message}");
            if ok {
                Ok(())
            } else {
                Err("loop start did not take effect".to_string())
            }
        }
        LoopSubcommand::Stop => {
            let (ok, message) = loop_stop(&socket_path)
                .await
                .map_err(|e| format!("loop stop failed: {e}"))?;
            println!("{message}");
            if ok {
                Ok(())
            } else {
                Err("loop stop did not take effect".to_string())
            }
        }
        LoopSubcommand::Status => {
            let (
                state,
                remaining,
                armed,
                gate_enabled,
                max_run_secs,
                max_run_tokens,
                max_run_usd,
                max_idle_iterations,
            ) = loop_status(&socket_path)
                .await
                .map_err(|e| format!("loop status failed: {e}"))?;
            print!(
                "{}",
                render_status(
                    &state,
                    remaining,
                    armed,
                    gate_enabled,
                    max_run_secs,
                    max_run_tokens,
                    max_run_usd,
                    max_idle_iterations,
                )
            );
            Ok(())
        }
        LoopSubcommand::Log { limit } => {
            let notes = loop_log(&socket_path, limit)
                .await
                .map_err(|e| format!("loop log failed: {e}"))?;
            print!("{}", render_log(&notes));
            Ok(())
        }
        LoopSubcommand::Skip { story_id } => {
            let (ok, message) = loop_skip(&socket_path, story_id)
                .await
                .map_err(|e| format!("loop skip failed: {e}"))?;
            println!("{message}");
            if ok {
                Ok(())
            } else {
                Err("loop skip did not take effect".to_string())
            }
        }
    }
}

/// Pure renderer — the progress log, oldest-first (the order the
/// agent learned them), matching how the driver injects them.
fn render_log(notes: &[String]) -> String {
    if notes.is_empty() {
        return "Loop progress log is empty. The agent records \
                learnings with `loop.note` during a run.\n"
            .to_string();
    }
    let mut out =
        format!("Loop progress log ({} note(s), oldest first):\n", notes.len());
    for note in notes.iter().rev() {
        out.push_str("  - ");
        out.push_str(note.trim());
        out.push('\n');
    }
    out
}

async fn require_daemon_running(socket_path: &Path) -> Result<(), String> {
    if daemon_is_running(socket_path).await {
        return Ok(());
    }
    Err(format!(
        "aivyx loop: no daemon running on socket {} — start the daemon \
         first with `aivyx daemon run`",
        socket_path.display(),
    ))
}

fn status_label(status: &StoryStatus) -> &'static str {
    match status {
        StoryStatus::Pending => "pending",
        StoryStatus::Done { .. } => "done",
        StoryStatus::Skipped { .. } => "skipped",
    }
}

/// Pure renderer — the backlog as an operator-readable list,
/// ordered by priority then insertion (matching the driver's
/// pick order), so the operator sees what runs next at the top.
fn render_backlog(stories: &[Story]) -> String {
    if stories.is_empty() {
        return "Backlog is empty. Add stories with `aivyx loop add \
                <title>`.\n"
            .to_string();
    }
    let mut sorted: Vec<&Story> = stories.iter().collect();
    sorted.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then(a.created_seq.cmp(&b.created_seq))
    });
    let pending =
        stories.iter().filter(|s| status_label(&s.status) == "pending").count();
    let mut out = format!(
        "Loop backlog ({} stor{}, {pending} pending):\n",
        stories.len(),
        if stories.len() == 1 { "y" } else { "ies" },
    );
    for s in sorted {
        out.push_str(&format!(
            "  [{}] p{:<4} {}  ({})\n",
            status_label(&s.status),
            s.priority,
            s.title,
            s.id,
        ));
    }
    out
}

/// Pure renderer — the loop run state.
#[allow(clippy::too_many_arguments)]
fn render_status(
    state: &LoopRunState,
    remaining: usize,
    armed: bool,
    gate_enabled: bool,
    max_run_secs: Option<u64>,
    max_run_tokens: Option<u64>,
    max_run_usd: Option<f64>,
    max_idle_iterations: u32,
) -> String {
    let mut out = String::from("Loop status:\n");
    if !armed {
        out.push_str(
            "  driver: not armed (set `[loop] enabled = true` in \
             aivyx.toml and restart the daemon)\n",
        );
    } else if state.active {
        out.push_str(&format!(
            "  driver: RUNNING — iteration {} of max {}\n",
            state.iteration, state.max_iterations,
        ));
        // Chapter Circuit (CI.5) — surface the live stall streak so a
        // run spinning without progress is visible before it trips.
        if max_idle_iterations > 0 && state.consecutive_idle > 0 {
            out.push_str(&format!(
                "  ⚠ no progress for {} of {} iteration(s) before the \
                 stall breaker stops the run\n",
                state.consecutive_idle, max_idle_iterations,
            ));
        }
    } else {
        out.push_str("  driver: idle (armed)\n");
        if let Some(reason) = &state.last_stop_reason {
            out.push_str(&format!(
                "  last run ended after {} iteration(s): {reason}\n",
                state.iteration,
            ));
        }
    }
    if armed {
        // Phase 174 — surface the safety config so the operator
        // can confirm what guards a run before starting one.
        out.push_str(&format!(
            "  gate verification: {}\n",
            if gate_enabled {
                "on (driver re-runs the gate command each iteration)"
            } else {
                "off (driver trusts the agent's loop.complete)"
            },
        ));
        out.push_str(&format!(
            "  wall-clock cap: {}\n",
            match max_run_secs {
                Some(s) => format!("{s}s"),
                None => "none".to_string(),
            },
        ));
        out.push_str(&format!(
            "  token budget: {}\n",
            match max_run_tokens {
                Some(t) => format!("{t} tokens / run"),
                None => "none".to_string(),
            },
        ));
        // Chapter K (K.4.2) — the per-run dollar cap, alongside the
        // token budget. Local-model runs price at $0, so the cap only
        // advances on cloud spend.
        out.push_str(&format!(
            "  dollar cap: {}\n",
            match max_run_usd {
                Some(d) => format!("${d:.2} / run"),
                None => "none".to_string(),
            },
        ));
        // Chapter Circuit (CI.5) — the cross-iteration stall breaker.
        out.push_str(&format!(
            "  stall breaker: {}\n",
            if max_idle_iterations == 0 {
                "off".to_string()
            } else {
                format!(
                    "stop after {max_idle_iterations} idle iteration(s)"
                )
            },
        ));
        // Phase 177 — live spend, once a run has had an iteration.
        if state.tokens_used > 0 || state.active {
            out.push_str(&format!(
                "  tokens used: {}{}\n",
                state.tokens_used,
                match max_run_tokens {
                    Some(t) => format!(" / {t}"),
                    None => String::new(),
                },
            ));
        }
        // Chapter K — live priced spend (cents on the state snapshot),
        // shown against the cap when one is set.
        if state.spent_cents > 0 || state.active {
            out.push_str(&format!(
                "  spend used: ${:.2}{}\n",
                state.spent_cents as f64 / 100.0,
                match max_run_usd {
                    Some(d) => format!(" / ${d:.2}"),
                    None => String::new(),
                },
            ));
        }
    }
    out.push_str(&format!("  backlog: {remaining} pending\n"));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn story(
        id: &str,
        priority: u32,
        seq: u64,
        title: &str,
        status: StoryStatus,
    ) -> Story {
        Story {
            id: id.into(),
            priority,
            title: title.into(),
            body: String::new(),
            created_at_unix_ms: 0,
            created_seq: seq,
            status,
        }
    }

    #[test]
    fn empty_backlog_renders_hint() {
        let out = render_backlog(&[]);
        assert!(out.contains("Backlog is empty"));
    }

    #[test]
    fn backlog_sorted_by_priority_then_insertion() {
        let stories = vec![
            story("a", 10, 0, "low prio", StoryStatus::Pending),
            story("b", 1, 1, "high prio", StoryStatus::Pending),
            story(
                "c",
                1,
                2,
                "done one",
                StoryStatus::Done {
                    resolved_at_unix_ms: 5,
                },
            ),
        ];
        let out = render_backlog(&stories);
        // High-priority pending first; "3 stories, 2 pending".
        assert!(out.contains("3 stories, 2 pending"));
        let b_pos = out.find("high prio").unwrap();
        let a_pos = out.find("low prio").unwrap();
        assert!(b_pos < a_pos, "lower priority number sorts first");
        assert!(out.contains("[done]"));
        assert!(out.contains("[pending]"));
    }

    #[test]
    fn status_not_armed() {
        let out =
            render_status(&LoopRunState::default(), 3, false, false, None, None, None, 0);
        assert!(out.contains("not armed"));
        assert!(out.contains("3 pending"));
        // Safety config is only shown when armed.
        assert!(!out.contains("gate verification"));
    }

    #[test]
    fn status_running_shows_iteration_and_safety_config() {
        let state = LoopRunState {
            active: true,
            iteration: 4,
            max_iterations: 25,
            started_at_unix_ms: 1,
            last_stop_reason: None,
            tokens_used: 12_345,
            spent_cents: 250,
            consecutive_idle: 2,
        };
        let out =
            render_status(&state, 7, true, true, Some(3600), Some(500000), Some(5.0), 3);
        assert!(out.contains("RUNNING — iteration 4 of max 25"));
        assert!(out.contains("7 pending"));
        assert!(out.contains("gate verification: on"));
        assert!(out.contains("wall-clock cap: 3600s"));
        assert!(out.contains("token budget: 500000 tokens / run"));
        assert!(out.contains("tokens used: 12345 / 500000"));
        // Chapter K (K.4.2) — the dollar cap and live priced spend.
        assert!(out.contains("dollar cap: $5.00 / run"));
        assert!(out.contains("spend used: $2.50 / $5.00"));
        // Chapter Circuit (CI.5) — the stall-breaker config + live idle streak.
        assert!(out.contains("stall breaker: stop after 3 idle iteration(s)"));
        assert!(out.contains("no progress for 2 of 3 iteration(s)"));
    }

    #[test]
    fn status_idle_shows_last_stop_reason_and_gate_off() {
        let state = LoopRunState {
            active: false,
            iteration: 12,
            max_iterations: 25,
            started_at_unix_ms: 1,
            last_stop_reason: Some("backlog complete".into()),
            tokens_used: 98_000,
            spent_cents: 0,
            consecutive_idle: 0,
        };
        let out = render_status(&state, 0, true, false, None, None, None, 0);
        assert!(out.contains("idle (armed)"));
        assert!(out.contains("ended after 12 iteration(s): backlog complete"));
        assert!(out.contains("gate verification: off"));
        assert!(out.contains("wall-clock cap: none"));
        assert!(out.contains("dollar cap: none"));
        // CI.5 — stall breaker disabled (0) renders "off".
        assert!(out.contains("stall breaker: off"));
        // Last run's spend shown, no cap suffix.
        assert!(out.contains("tokens used: 98000\n"));
    }

    #[test]
    fn log_empty_and_oldest_first() {
        assert!(render_log(&[]).contains("empty"));
        // Notes arrive newest-first; render oldest-first.
        let notes = vec![
            "newest".to_string(),
            "middle".to_string(),
            "oldest".to_string(),
        ];
        let out = render_log(&notes);
        assert!(out.contains("3 note(s)"));
        let oldest = out.find("oldest").unwrap();
        let newest = out.find("newest").unwrap();
        assert!(oldest < newest);
    }
}
