//! Daemon-backed REPL session — Phase 18 Task 2.
//!
//! Provides [`run_daemon_session`], a drop-in REPL loop that
//! auto-spawns a daemon (if needed), connects via [`DaemonSession`],
//! reads lines from stdin, submits each as a turn, and renders
//! streamed events to a writer via [`StreamEventPayload::render_for_cli`].
//!
//! This is the daemon-mode counterpart of [`crate::run_session`]:
//! same UX (banner, prompt, ctrl-D to exit), but the turn loop runs
//! in the background daemon process rather than in-process.

use std::io::{BufRead, Write};
use std::time::Duration;

use crate::daemon_client::{spawn_daemon_and_wait, DaemonSession};
use crate::session::SessionReport;

const AUTO_SPAWN_TIMEOUT: Duration = Duration::from_secs(10);

/// Configuration for a daemon-backed REPL session.
pub struct DaemonSessionConfig {
    pub socket_path: std::path::PathBuf,
    pub role: Option<String>,
    pub prompt: String,
    pub banner: Option<String>,
}

/// Drive a daemon-backed CLI session to completion.
///
/// The loop mirrors [`crate::run_session`]: read a line, submit it
/// to the daemon, render streamed events, repeat until EOF. Returns
/// a [`SessionReport`] with the same shape as the in-process path.
///
/// If no daemon is listening at `config.socket_path`, attempts to
/// auto-spawn one via `spawn_daemon_and_wait`. Returns `Err` if both
/// the spawn and the connect fail.
pub async fn run_daemon_session<R, W>(
    config: DaemonSessionConfig,
    mut reader: R,
    mut writer: W,
) -> Result<SessionReport, String>
where
    R: BufRead,
    W: Write,
{
    // Try to connect directly. If the connect fails (no daemon
    // listening), auto-spawn and retry. This avoids a probe connection
    // that would consume the daemon's single-connection slot.
    let mut session = match DaemonSession::connect(&config.socket_path, config.role.clone()).await {
        Ok(s) => s,
        Err(_) => {
            spawn_daemon_and_wait(&config.socket_path, AUTO_SPAWN_TIMEOUT).await?;
            DaemonSession::connect(&config.socket_path, config.role).await?
        }
    };

    // Banner.
    if let Some(banner) = config.banner.as_deref() {
        writeln!(writer, "{banner}").map_err(|e| format!("banner write: {e}"))?;
        writer.flush().map_err(|e| format!("banner flush: {e}"))?;
    }

    // REPL loop.
    let mut turns_run: usize = 0;
    let mut last_outcome_str: Option<String> = None;
    let mut line = String::new();

    loop {
        // Prompt.
        if !config.prompt.is_empty() {
            write!(writer, "{}", config.prompt).map_err(|e| format!("prompt write: {e}"))?;
            writer.flush().map_err(|e| format!("prompt flush: {e}"))?;
        }

        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(e) => return Err(format!("read error: {e}")),
        }

        let input = line.trim();
        if input.is_empty() {
            continue;
        }

        let (events, outcome) = session.submit_input(input.to_string()).await?;

        for event in &events {
            let rendered = event.render_for_cli();
            write!(writer, "{rendered}").map_err(|e| format!("render write: {e}"))?;
        }
        writer.flush().map_err(|e| format!("render flush: {e}"))?;

        turns_run += 1;
        last_outcome_str = Some(outcome);
    }

    let _ = session.disconnect().await;

    Ok(SessionReport {
        turns_run,
        last_outcome: last_outcome_str.map(|s| outcome_str_to_turn_outcome(&s)),
    })
}

/// Best-effort parse of the daemon's outcome string back into a
/// `TurnOutcome`. The daemon sends `format_outcome` strings like
/// "completed: Hello from daemon!". This is lossy — we recover the
/// variant and the message but not the metadata (duration, tool count).
fn outcome_str_to_turn_outcome(s: &str) -> aivyx_core::TurnOutcome {
    if let Some(msg) = s.strip_prefix("completed: ") {
        aivyx_core::TurnOutcome::Completed {
            final_message: msg.to_string(),
            tool_calls_made: 0,
            duration: Duration::ZERO,
        }
    } else if let Some(msg) = s.strip_prefix("failed: ") {
        aivyx_core::TurnOutcome::Failed(aivyx_core::AivyxError::Config(msg.to_string()))
    } else if s == "cancelled" {
        aivyx_core::TurnOutcome::Cancelled {
            tool_calls_made: 0,
        }
    } else if s == "timed out" {
        aivyx_core::TurnOutcome::TimedOut {
            tool_calls_made: 0,
            elapsed: Duration::ZERO,
        }
    } else if let Some(reason) = s.strip_prefix("escalated: ") {
        aivyx_core::TurnOutcome::Escalated {
            reason: reason.to_string(),
            pending_tool: aivyx_core::ToolId::new(),
            tool_calls_made: 0,
        }
    } else {
        aivyx_core::TurnOutcome::Completed {
            final_message: s.to_string(),
            tool_calls_made: 0,
            duration: Duration::ZERO,
        }
    }
}
