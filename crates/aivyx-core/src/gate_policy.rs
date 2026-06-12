//! Chapter H — the per-run **gate policy**: how a run treats an approval point
//! when there may be no operator to answer.
//!
//! Every approval point in Aivyx assumes a human is reachable — a tool's
//! [`ToolOutcome::RequiresEscalation`](crate::ToolOutcome) becomes
//! [`TurnOutcome::Escalated`](crate::TurnOutcome) and the daemon parks it
//! behind an operator gate; a team mission's `GateMode::Human` step pauses as
//! `AwaitingApproval`. An **unattended** run (a batch job, a cron-triggered
//! run, the autonomous loop) can't wait on an absent human, so it carries a
//! [`GatePolicy`] and resolves gates by policy instead.
//!
//! v1 is **reject-only** (the safe posture locked at scope): headless never
//! *approves* past a gate — it records the reason and aborts that branch. See
//! `docs/HEADLESS_MODE.md`.

/// How a run resolves an approval gate (Chapter H).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GatePolicy {
    /// Interactive: park the run and wait for the operator — today's behavior.
    /// The default, so every existing run path is byte-for-byte unchanged.
    #[default]
    Interactive,
    /// Headless (v1): never wait. At any gate — a single-agent tool escalation
    /// or a team-mission human gate — record the reason and **abort** that
    /// branch. Confirm-first / irreversible tools are covered by the same
    /// reject (unconfirmed, they escalate → they are rejected); a future
    /// `AutoApprove` posture would still have to leave them blocked.
    RejectAndAbort,
}

impl GatePolicy {
    /// Whether this run is unattended — no operator to wait on, so a gate is
    /// resolved by policy rather than by a person.
    pub fn is_headless(self) -> bool {
        matches!(self, GatePolicy::RejectAndAbort)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_interactive_and_not_headless() {
        assert_eq!(GatePolicy::default(), GatePolicy::Interactive);
        assert!(!GatePolicy::default().is_headless());
    }

    #[test]
    fn reject_and_abort_is_headless() {
        assert!(GatePolicy::RejectAndAbort.is_headless());
        assert!(!GatePolicy::Interactive.is_headless());
    }
}
