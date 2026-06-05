//! Phase 174 — the autonomous-loop gate runner.
//!
//! Driver-side verification that the tree is green. Phase 173
//! left gate enforcement entirely to the agent (via the
//! canonical prompt); Phase 174 makes the **driver** run the
//! operator-configured gate command (`[loop].gate_command`,
//! e.g. `"cargo test"`) before the first iteration and after
//! every iteration. A red result stops the run.
//!
//! The [`GateRunner`] trait keeps the driver's verification
//! logic testable without shelling out; the production
//! [`ShellGateRunner`] runs the command in `working_dir` via
//! `sh -c` with a wall-clock timeout.
//!
//! ## Privilege note
//!
//! The gate command runs at **daemon privilege**, executed
//! directly by the daemon — it is operator config (like a
//! cron command), not an agent tool, so it is deliberately not
//! capability-gated. An operator who points `gate_command` at
//! something destructive owns that.

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;

/// The result of one gate run. Only [`GateOutcome::Passed`] is
/// "green" — every other variant stops the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateOutcome {
    /// The gate command exited 0 — the tree is green.
    Passed,
    /// The gate command exited non-zero — the tree is red.
    Failed { code: Option<i32> },
    /// The gate command ran longer than the configured timeout
    /// and was killed — treated as red.
    TimedOut,
    /// The gate command could not be launched / waited on (e.g.
    /// the shell is missing, the working dir doesn't exist).
    /// Treated as red — better to halt than to charge ahead on
    /// an unverifiable tree.
    Errored { detail: String },
}

impl GateOutcome {
    /// Only `Passed` lets a run continue.
    pub fn is_green(&self) -> bool {
        matches!(self, GateOutcome::Passed)
    }

    /// Operator-readable one-liner for the stop reason / logs.
    pub fn label(&self) -> String {
        match self {
            GateOutcome::Passed => "gate passed".to_string(),
            GateOutcome::Failed { code: Some(c) } => {
                format!("gate failed (exit {c})")
            }
            GateOutcome::Failed { code: None } => {
                "gate failed (killed by signal)".to_string()
            }
            GateOutcome::TimedOut => "gate timed out".to_string(),
            GateOutcome::Errored { detail } => {
                format!("gate could not run: {detail}")
            }
        }
    }
}

/// Runs the loop's gate. Abstracted so the driver's
/// verification logic is unit-testable without a real process.
#[async_trait]
pub trait GateRunner: Send + Sync {
    async fn run(&self) -> GateOutcome;
}

/// Production gate runner: `sh -c "<command>"` in `working_dir`
/// with a wall-clock timeout. Stdio is inherited so the gate's
/// build/test output lands in the daemon's logs.
pub struct ShellGateRunner {
    command: String,
    working_dir: Option<PathBuf>,
    timeout: Duration,
}

impl ShellGateRunner {
    pub fn new(
        command: String,
        working_dir: Option<PathBuf>,
        timeout: Duration,
    ) -> Self {
        ShellGateRunner {
            command,
            working_dir,
            timeout,
        }
    }
}

#[async_trait]
impl GateRunner for ShellGateRunner {
    async fn run(&self) -> GateOutcome {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.arg("-c").arg(&self.command);
        if let Some(dir) = &self.working_dir {
            cmd.current_dir(dir);
        }
        // Inherit stdio: the gate's output is useful in the
        // daemon log, and a gate shouldn't need stdin.
        cmd.kill_on_drop(true);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return GateOutcome::Errored {
                    detail: format!("spawn failed: {e}"),
                };
            }
        };

        match tokio::time::timeout(self.timeout, child.wait()).await {
            Ok(Ok(status)) => {
                if status.success() {
                    GateOutcome::Passed
                } else {
                    GateOutcome::Failed { code: status.code() }
                }
            }
            Ok(Err(e)) => GateOutcome::Errored {
                detail: format!("wait failed: {e}"),
            },
            Err(_elapsed) => {
                // Timed out — kill the child + reap it. (Grand-
                // children spawned by the gate may linger; the
                // honest limitation is documented at sign-off.)
                let _ = child.start_kill();
                let _ = child.wait().await;
                GateOutcome::TimedOut
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_passed_is_green() {
        assert!(GateOutcome::Passed.is_green());
        assert!(!GateOutcome::Failed { code: Some(1) }.is_green());
        assert!(!GateOutcome::TimedOut.is_green());
        assert!(!GateOutcome::Errored {
            detail: "x".into()
        }
        .is_green());
    }

    #[test]
    fn labels_are_distinct() {
        assert!(GateOutcome::Passed.label().contains("passed"));
        assert!(GateOutcome::Failed { code: Some(2) }
            .label()
            .contains("exit 2"));
        assert!(GateOutcome::TimedOut.label().contains("timed out"));
        assert!(GateOutcome::Errored {
            detail: "boom".into()
        }
        .label()
        .contains("boom"));
    }

    #[tokio::test]
    async fn shell_true_passes() {
        let r = ShellGateRunner::new(
            "true".into(),
            None,
            Duration::from_secs(10),
        );
        assert_eq!(r.run().await, GateOutcome::Passed);
    }

    #[tokio::test]
    async fn shell_false_fails() {
        let r = ShellGateRunner::new(
            "false".into(),
            None,
            Duration::from_secs(10),
        );
        assert!(matches!(r.run().await, GateOutcome::Failed { .. }));
    }

    #[tokio::test]
    async fn shell_nonzero_exit_code_is_captured() {
        let r = ShellGateRunner::new(
            "exit 7".into(),
            None,
            Duration::from_secs(10),
        );
        assert_eq!(
            r.run().await,
            GateOutcome::Failed { code: Some(7) }
        );
    }

    #[tokio::test]
    async fn shell_timeout_kills_and_reports() {
        let r = ShellGateRunner::new(
            "sleep 30".into(),
            None,
            Duration::from_millis(150),
        );
        assert_eq!(r.run().await, GateOutcome::TimedOut);
    }

    #[tokio::test]
    async fn bad_working_dir_errors() {
        let r = ShellGateRunner::new(
            "true".into(),
            Some(PathBuf::from("/no/such/dir/aivyx-xyz")),
            Duration::from_secs(5),
        );
        // sh either fails to spawn (current_dir missing) or sh
        // runs and errors — both are non-green.
        assert!(!r.run().await.is_green());
    }
}
