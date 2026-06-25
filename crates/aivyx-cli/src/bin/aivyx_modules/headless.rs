//! `aivyx --headless "<task>"` — one-shot unattended turn (Chapter H
//! follow-on (a)).
//!
//! Chapter H made the daemon's turn path *honor* an unattended posture:
//! a headless run refuses (records the reason on the audit chain) at any
//! approval gate rather than parking for an operator (H.2/H.5/H.6). The
//! per-run `headless: true` IPC field and the `submit_input_headless`
//! client primitive already existed — this module is the missing CLI
//! consumer that makes the whole chapter reachable from a terminal,
//! a cron line, or a batch script.
//!
//! It connects to a **running daemon** (no in-process fallback — headless
//! relies on the daemon's gate interception), submits one turn over IPC,
//! streams the output, and maps the turn's terminal outcome onto a
//! process exit code so an unattended caller can branch on it:
//!
//! - `0` — the turn **completed**.
//! - `3` — the turn was **refused at a gate** (the headless posture: an
//!   escalation the daemon would normally hand to a human).
//! - `1` — any other non-completion (failed / timed out / step cap /
//!   cycle breaker / cancelled).

use std::path::Path;

use aivyx_channel::daemon_client::{daemon_is_running, DaemonSession};
use aivyx_channel::daemon_ipc::{default_socket_path, FrontendType};

/// Entry point for `aivyx --headless "<task>"`.
///
/// Returns `Err` for setup failures (no daemon, transport error) so the
/// caller renders them like any other CLI error (exit 1). A turn that
/// *ran* but did not complete (refused/failed/…) is **not** an `Err` — it
/// exits the process directly with the [`headless_exit_code`] mapping so
/// the distinct "refused" code (3) survives, which a `Result<(), String>`
/// (always exit 1) could not express.
pub async fn run_headless(task: &str) -> Result<(), String> {
    let socket_path = default_socket_path()?;
    require_daemon_running(&socket_path).await?;

    let mut session = DaemonSession::connect(&socket_path, None, Some(FrontendType::Local))
        .await
        .map_err(|e| {
            format!(
                "aivyx --headless: failed to connect to the daemon on {} — {e}",
                socket_path.display(),
            )
        })?;

    let (events, outcome) = session
        .submit_input_headless(task.to_string())
        .await
        .map_err(|e| format!("aivyx --headless: turn failed — {e}"))?;
    let _ = session.disconnect().await;

    // Stream the turn's output exactly as the interactive daemon REPL
    // would render it (reusing the shared `render_for_cli`).
    for event in &events {
        print!("{}", event.render_for_cli());
    }

    let code = headless_exit_code(&outcome);
    if code == 0 {
        // `completed: <final_message>` — the final message already
        // streamed as Text events; print the terminal line to stderr so
        // stdout stays the agent's answer.
        eprintln!("aivyx --headless: {outcome}");
        return Ok(());
    }

    // A turn that ran but did not complete. Surface why on stderr and
    // exit with the classified code so cron/batch callers can branch.
    eprintln!("aivyx --headless: {outcome}");
    std::process::exit(code);
}

async fn require_daemon_running(socket_path: &Path) -> Result<(), String> {
    if daemon_is_running(socket_path).await {
        return Ok(());
    }
    Err(format!(
        "aivyx --headless: no daemon running on socket {} — \
         start the daemon first with `aivyx daemon run`",
        socket_path.display(),
    ))
}

/// Map a daemon `TurnComplete` outcome string (see `format_outcome` in
/// `daemon_server.rs`) onto a process exit code. Pure so it can be unit
/// tested against the exact prefixes the daemon emits.
///
/// - `completed: …` → `0`
/// - `escalated: …` → `3` (the headless refusal — the one code a caller
///   most wants to distinguish from a hard failure)
/// - anything else → `1` (failed / timed out / aborted / stopped /
///   cancelled)
fn headless_exit_code(outcome: &str) -> i32 {
    if outcome.starts_with("completed:") {
        0
    } else if outcome.starts_with("escalated:") {
        3
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::headless_exit_code;

    #[test]
    fn completed_outcome_is_zero() {
        assert_eq!(headless_exit_code("completed: here is your answer"), 0);
    }

    #[test]
    fn escalated_outcome_is_the_distinct_refusal_code() {
        assert_eq!(
            headless_exit_code("escalated: shell.exec needs operator approval"),
            3
        );
    }

    #[test]
    fn other_non_completions_are_generic_failure() {
        assert_eq!(headless_exit_code("failed: provider error"), 1);
        assert_eq!(headless_exit_code("timed out"), 1);
        assert_eq!(headless_exit_code("aborted: planner exceeded 32 steps"), 1);
        assert_eq!(
            headless_exit_code("stopped: 3 repeated identical tool calls"),
            1
        );
        assert_eq!(headless_exit_code("cancelled"), 1);
    }
}
