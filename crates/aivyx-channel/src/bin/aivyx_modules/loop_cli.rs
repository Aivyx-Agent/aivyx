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
    daemon_is_running, loop_add, loop_list, loop_start, loop_status,
    loop_stop,
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
            let (state, remaining, armed) = loop_status(&socket_path)
                .await
                .map_err(|e| format!("loop status failed: {e}"))?;
            print!("{}", render_status(&state, remaining, armed));
            Ok(())
        }
    }
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
fn render_status(state: &LoopRunState, remaining: usize, armed: bool) -> String {
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
    } else {
        out.push_str("  driver: idle (armed)\n");
        if let Some(reason) = &state.last_stop_reason {
            out.push_str(&format!(
                "  last run ended after {} iteration(s): {reason}\n",
                state.iteration,
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
        let out = render_status(&LoopRunState::default(), 3, false);
        assert!(out.contains("not armed"));
        assert!(out.contains("3 pending"));
    }

    #[test]
    fn status_running_shows_iteration() {
        let state = LoopRunState {
            active: true,
            iteration: 4,
            max_iterations: 25,
            started_at_unix_ms: 1,
            last_stop_reason: None,
        };
        let out = render_status(&state, 7, true);
        assert!(out.contains("RUNNING — iteration 4 of max 25"));
        assert!(out.contains("7 pending"));
    }

    #[test]
    fn status_idle_shows_last_stop_reason() {
        let state = LoopRunState {
            active: false,
            iteration: 12,
            max_iterations: 25,
            started_at_unix_ms: 1,
            last_stop_reason: Some("backlog complete".into()),
        };
        let out = render_status(&state, 0, true);
        assert!(out.contains("idle (armed)"));
        assert!(out.contains("ended after 12 iteration(s): backlog complete"));
    }
}
