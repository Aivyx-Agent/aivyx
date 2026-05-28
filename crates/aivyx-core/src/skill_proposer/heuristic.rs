//! Phase 112 Task 2 — Cheap deterministic heuristic gate for
//! skill auto-proposal candidates.
//!
//! Q1b's first stage. Reads post-turn signals the caller has
//! already assembled from `TurnOutcome` + the audit log;
//! returns a boolean telling the auto-proposer whether to fire
//! the LLM judge against this turn. Zero LLM cost; pure
//! function on primitive inputs.
//!
//! ## Why pre-assembled signals (not raw TurnOutcome + audit
//! log refs)
//!
//! Keeping this module a pure function on a small input
//! struct means it can be tested without daemon scaffolding,
//! and the heuristic logic stays separable from where the
//! signals are sourced. The Task 4 background-task wiring is
//! responsible for assembling `TurnSignals` from whatever
//! sources it has access to (the in-process `TurnOutcome` for
//! `tool_calls_made` and `duration`; the audit log for
//! `distinct_tool_id_count` and `had_successful_gate_resolve`).
//!
//! ## Why four signals, not more
//!
//! Phase 95's `skip-when-idle` precedent landed on **one**
//! audit-growth signal. Four signals is a step up from that
//! because the auto-proposer has more axes a "complex turn"
//! can show on, but going past four would push into LLM-
//! judge territory — and that's Task 3's job. Each signal is
//! independently ablation-tested below so a future operator
//! can read the unit tests to understand what each threshold
//! actually gates on.

use std::time::Duration;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

/// Post-turn signals the heuristic reads. Caller-assembled
/// from `TurnOutcome` + audit-log entries for the turn; the
/// heuristic itself stays pure on this input struct.
///
/// All fields are post-turn observations (the turn has already
/// finalized when these are read). The heuristic never has to
/// reason about partial state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnSignals {
    /// Total tool calls made during the turn.
    /// Source: `TurnOutcome::Completed { tool_calls_made, .. }`
    /// (and the equivalent field on the other variants).
    pub tool_calls_made: u32,

    /// Number of distinct `ToolId`s used during the turn.
    /// Different from `tool_calls_made`: a turn that calls
    /// `fs.read` three times has `tool_calls_made=3,
    /// distinct_tool_id_count=1`. The heuristic uses this to
    /// distinguish "doing real multi-tool work" from
    /// "hammering one tool."
    /// Source: deduplicated walk of the turn's audit entries.
    pub distinct_tool_id_count: u32,

    /// Wall-clock duration of the turn.
    /// Source: `TurnOutcome::Completed { duration, .. }`
    /// (and the elapsed field on `TimedOut`).
    pub duration: Duration,

    /// Whether any approval gate was resolved successfully
    /// during the turn. Gate-resolve presence is a strong
    /// signal that the turn did something operator-noteworthy
    /// (the agent paused for approval and proceeded), which
    /// is exactly the kind of turn worth proposing a skill
    /// from.
    /// Source: walk of audit entries for
    /// `ApprovalGate` followed by `gate_resolved=true`.
    pub had_successful_gate_resolve: bool,
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// How the heuristic combines its four signals. `Any` (the
/// default) fires if ANY threshold is crossed; `All` requires
/// every threshold to be crossed.
///
/// `Any` is the operator-reasonable default — a turn with
/// three tool calls OR one that took ten seconds OR one that
/// resolved a gate is each individually a candidate. `All`
/// exists for operators who want the heuristic to act as a
/// noise filter rather than a candidate funnel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchMode {
    #[default]
    Any,
    All,
}

/// Operator-configurable thresholds for the heuristic.
/// Mirrors the `[skills.auto_propose.heuristic]` TOML section
/// that Task 5 wires in.
///
/// Defaults are tuned for "fires on real multi-tool work but
/// skips most chats" — see the test module's
/// `default_config_does_not_fire_on_chit_chat_turns` and
/// `default_config_fires_on_multi_tool_research_turns` for
/// the actual cases that calibrated the numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeuristicConfig {
    /// Minimum tool calls required (inclusive). Default `3`.
    pub tool_call_count_min: u32,

    /// Minimum distinct tool-ids required (inclusive).
    /// Default `2`.
    pub distinct_tool_id_min: u32,

    /// Minimum duration required (inclusive). Default `5s`.
    pub duration_ms_min: u64,

    /// Whether a successful gate-resolve is a sufficient
    /// signal on its own (counts as one threshold crossing
    /// under `MatchMode::Any`; required to be true under
    /// `MatchMode::All`).
    /// Default `false` — gate-resolve is uncommon enough that
    /// requiring it under All would make the heuristic
    /// almost never fire.
    pub require_gate_resolve: bool,

    /// How to combine the signals. Default `MatchMode::Any`.
    pub mode: MatchMode,
}

impl Default for HeuristicConfig {
    fn default() -> Self {
        HeuristicConfig {
            tool_call_count_min: 3,
            distinct_tool_id_min: 2,
            duration_ms_min: 5000,
            require_gate_resolve: false,
            mode: MatchMode::Any,
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 115 — Failure-outcome heuristic
// ---------------------------------------------------------------------------

/// Phase 115 — the kind of failure that triggered the
/// auto-proposer's negative-feedback path. Mirrors the
/// `TurnOutcome` failure variants but is its own enum so
/// `aivyx-core::skill_proposer` stays independent of the
/// `TurnOutcome` shape (which lives elsewhere in
/// `aivyx-core`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// `TurnOutcome::Failed(error)` — a system/planner
    /// failure carried an `AivyxError`.
    Failed,
    /// `TurnOutcome::Cancelled` — the operator cancelled
    /// the turn mid-flight. Default-disabled because most
    /// cancellations are operator-driven (the operator
    /// changed their mind) rather than learnable signal.
    Cancelled,
    /// `TurnOutcome::TimedOut` — the agent exceeded its
    /// per-turn budget. Strong signal that the
    /// agent-Persona pair didn't recognize the task was
    /// too large for one turn; default-enabled.
    TimedOut,
    /// `TurnOutcome::Escalated` — the agent escalated to
    /// the operator. Default-disabled because escalation
    /// is the agent doing the right thing under D1's
    /// Tier-2 rules; the operator's `/approve` / `/reject`
    /// resolves the gate separately.
    Escalated,
}

impl FailureKind {
    /// Short stable label for the failure kind — used in
    /// audit events and operator-facing diagnostics.
    pub fn label(&self) -> &'static str {
        match self {
            FailureKind::Failed => "failed",
            FailureKind::Cancelled => "cancelled",
            FailureKind::TimedOut => "timed_out",
            FailureKind::Escalated => "escalated",
        }
    }
}

/// Phase 115 — per-failure-outcome enable flags. The
/// operator can tune which failure types fire the auto-
/// correction pipeline through the
/// `[persona.auto_propose.failure_outcomes]` TOML
/// sub-section that Phase 115 Task 6 wires in.
///
/// Defaults are operator-conservative: `Failed` and
/// `TimedOut` are on (clear failure signal); `Cancelled`
/// and `Escalated` are off (operator-driven; not
/// learnable without further classification).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureHeuristicConfig {
    pub failed: bool,
    pub cancelled: bool,
    pub timed_out: bool,
    pub escalated: bool,
}

impl Default for FailureHeuristicConfig {
    fn default() -> Self {
        FailureHeuristicConfig {
            failed: true,
            cancelled: false,
            timed_out: true,
            escalated: false,
        }
    }
}

/// Phase 115 — is this failure worth firing the auto-
/// correction LLM-judge call from? Pure function; same
/// shape as `is_candidate` but for the negative-feedback
/// path.
pub fn is_failure_candidate(
    kind: FailureKind,
    config: &FailureHeuristicConfig,
) -> bool {
    match kind {
        FailureKind::Failed => config.failed,
        FailureKind::Cancelled => config.cancelled,
        FailureKind::TimedOut => config.timed_out,
        FailureKind::Escalated => config.escalated,
    }
}

// ---------------------------------------------------------------------------
// The function itself
// ---------------------------------------------------------------------------

/// Returns `true` if the turn's post-finalize signals cross
/// the heuristic gate — i.e. the turn is a candidate for an
/// LLM-judge call to decide whether to propose a skill.
///
/// Pure function; no side effects. Read the unit tests below
/// for the calibration cases.
pub fn is_candidate(signals: &TurnSignals, config: &HeuristicConfig) -> bool {
    let crossings = [
        signals.tool_calls_made >= config.tool_call_count_min,
        signals.distinct_tool_id_count >= config.distinct_tool_id_min,
        signals.duration.as_millis() as u64 >= config.duration_ms_min,
        // Gate-resolve is treated as a binary signal: if the
        // operator requires it (`require_gate_resolve = true`)
        // it counts as a crossing only when actually present;
        // otherwise it's treated as a passive bonus signal
        // (still counts in `Any` mode, ignored in `All` mode
        // since absence wouldn't fail the All check otherwise).
        if config.require_gate_resolve {
            signals.had_successful_gate_resolve
        } else {
            // In Any mode, a gate-resolve is a free signal
            // (counts as a crossing). In All mode, it's not
            // required, so we treat it as "satisfied" (always
            // true) so it doesn't block the All check.
            match config.mode {
                MatchMode::Any => signals.had_successful_gate_resolve,
                MatchMode::All => true,
            }
        },
    ];

    match config.mode {
        MatchMode::Any => crossings.iter().any(|&c| c),
        MatchMode::All => crossings.iter().all(|&c| c),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_signals() -> TurnSignals {
        TurnSignals {
            tool_calls_made: 0,
            distinct_tool_id_count: 0,
            duration: Duration::from_millis(0),
            had_successful_gate_resolve: false,
        }
    }

    // ----- Default config calibration -----

    #[test]
    fn default_config_does_not_fire_on_chit_chat_turns() {
        // Pure-conversation turn: zero tools, sub-second.
        let signals = TurnSignals {
            tool_calls_made: 0,
            distinct_tool_id_count: 0,
            duration: Duration::from_millis(800),
            had_successful_gate_resolve: false,
        };
        let config = HeuristicConfig::default();
        assert!(!is_candidate(&signals, &config));
    }

    #[test]
    fn default_config_does_not_fire_on_single_tool_quick_turns() {
        // One `fs.read`, fast. Not "complex" — common case.
        let signals = TurnSignals {
            tool_calls_made: 1,
            distinct_tool_id_count: 1,
            duration: Duration::from_millis(1200),
            had_successful_gate_resolve: false,
        };
        let config = HeuristicConfig::default();
        assert!(!is_candidate(&signals, &config));
    }

    #[test]
    fn default_config_fires_on_multi_tool_research_turns() {
        // Real research-style turn: fs.read + web.fetch +
        // shell.exec, ~8 seconds. Worth proposing a skill
        // from if the pattern recurs.
        let signals = TurnSignals {
            tool_calls_made: 4,
            distinct_tool_id_count: 3,
            duration: Duration::from_millis(8200),
            had_successful_gate_resolve: false,
        };
        let config = HeuristicConfig::default();
        assert!(is_candidate(&signals, &config));
    }

    #[test]
    fn default_config_fires_on_gate_resolved_turns() {
        // Gate-resolve alone (under MatchMode::Any) is a
        // sufficient signal — the operator paused, approved,
        // the agent proceeded. That sequence is a candidate
        // skill pattern.
        let signals = TurnSignals {
            tool_calls_made: 1,
            distinct_tool_id_count: 1,
            duration: Duration::from_millis(2000),
            had_successful_gate_resolve: true,
        };
        let config = HeuristicConfig::default();
        assert!(is_candidate(&signals, &config));
    }

    // ----- Per-signal ablation (Any mode) -----

    #[test]
    fn any_mode_fires_on_tool_call_count_alone() {
        let signals = TurnSignals {
            tool_calls_made: 3, // crosses default min=3
            distinct_tool_id_count: 1,
            duration: Duration::from_millis(500),
            had_successful_gate_resolve: false,
        };
        assert!(is_candidate(&signals, &HeuristicConfig::default()));
    }

    #[test]
    fn any_mode_fires_on_distinct_tool_id_alone() {
        let signals = TurnSignals {
            tool_calls_made: 2,
            distinct_tool_id_count: 2, // crosses default min=2
            duration: Duration::from_millis(500),
            had_successful_gate_resolve: false,
        };
        assert!(is_candidate(&signals, &HeuristicConfig::default()));
    }

    #[test]
    fn any_mode_fires_on_duration_alone() {
        let signals = TurnSignals {
            tool_calls_made: 1,
            distinct_tool_id_count: 1,
            duration: Duration::from_millis(5000), // crosses default min=5000
            had_successful_gate_resolve: false,
        };
        assert!(is_candidate(&signals, &HeuristicConfig::default()));
    }

    #[test]
    fn any_mode_fires_on_gate_resolve_alone() {
        let signals = TurnSignals {
            tool_calls_made: 0,
            distinct_tool_id_count: 0,
            duration: Duration::from_millis(0),
            had_successful_gate_resolve: true,
        };
        assert!(is_candidate(&signals, &HeuristicConfig::default()));
    }

    // ----- All mode -----

    #[test]
    fn all_mode_requires_every_signal_to_cross() {
        let config = HeuristicConfig {
            mode: MatchMode::All,
            ..HeuristicConfig::default()
        };
        // Three of four cross; gate-resolve absent but
        // require_gate_resolve=false → All-mode treats the
        // gate axis as satisfied so the turn still fires.
        let signals = TurnSignals {
            tool_calls_made: 3,
            distinct_tool_id_count: 2,
            duration: Duration::from_millis(5000),
            had_successful_gate_resolve: false,
        };
        assert!(is_candidate(&signals, &config));
    }

    #[test]
    fn all_mode_with_require_gate_resolve_fails_without_gate() {
        let config = HeuristicConfig {
            mode: MatchMode::All,
            require_gate_resolve: true,
            ..HeuristicConfig::default()
        };
        let signals = TurnSignals {
            tool_calls_made: 10,
            distinct_tool_id_count: 5,
            duration: Duration::from_millis(30_000),
            had_successful_gate_resolve: false, // the missing axis
        };
        assert!(!is_candidate(&signals, &config));
    }

    #[test]
    fn all_mode_with_require_gate_resolve_passes_with_gate() {
        let config = HeuristicConfig {
            mode: MatchMode::All,
            require_gate_resolve: true,
            ..HeuristicConfig::default()
        };
        let signals = TurnSignals {
            tool_calls_made: 3,
            distinct_tool_id_count: 2,
            duration: Duration::from_millis(5000),
            had_successful_gate_resolve: true,
        };
        assert!(is_candidate(&signals, &config));
    }

    // ----- Threshold boundary behavior -----

    #[test]
    fn thresholds_are_inclusive() {
        // Default mins are 3, 2, 5000. Exact-value signals
        // should cross (inclusive bound).
        let signals = TurnSignals {
            tool_calls_made: 3,
            distinct_tool_id_count: 2,
            duration: Duration::from_millis(5000),
            had_successful_gate_resolve: false,
        };
        let config = HeuristicConfig {
            mode: MatchMode::All,
            ..HeuristicConfig::default()
        };
        assert!(is_candidate(&signals, &config));
    }

    #[test]
    fn just_below_thresholds_does_not_cross() {
        let signals = TurnSignals {
            tool_calls_made: 2,
            distinct_tool_id_count: 1,
            duration: Duration::from_millis(4999),
            had_successful_gate_resolve: false,
        };
        let config = HeuristicConfig {
            mode: MatchMode::All,
            ..HeuristicConfig::default()
        };
        assert!(!is_candidate(&signals, &config));
    }

    // ----- Defaults sanity -----

    #[test]
    fn defaults_match_documented_values() {
        let c = HeuristicConfig::default();
        assert_eq!(c.tool_call_count_min, 3);
        assert_eq!(c.distinct_tool_id_min, 2);
        assert_eq!(c.duration_ms_min, 5000);
        assert!(!c.require_gate_resolve);
        assert_eq!(c.mode, MatchMode::Any);
    }

    #[test]
    fn empty_signals_under_default_config_do_not_fire() {
        assert!(!is_candidate(&empty_signals(), &HeuristicConfig::default()));
    }

    // ----- Serde round-trip (the config will round-trip
    // through TOML in Task 5; pin the serde shape now so
    // the TOML wiring slots in without a surprise) -----

    #[test]
    fn heuristic_config_round_trips_through_serde_json() {
        let original = HeuristicConfig {
            tool_call_count_min: 7,
            distinct_tool_id_min: 4,
            duration_ms_min: 12_000,
            require_gate_resolve: true,
            mode: MatchMode::All,
        };
        let s = serde_json::to_string(&original).expect("serialize");
        let back: HeuristicConfig =
            serde_json::from_str(&s).expect("deserialize");
        assert_eq!(back, original);
    }

    #[test]
    fn match_mode_serializes_as_lowercase_string() {
        let s = serde_json::to_string(&MatchMode::Any).unwrap();
        assert_eq!(s, "\"any\"");
        let s = serde_json::to_string(&MatchMode::All).unwrap();
        assert_eq!(s, "\"all\"");
    }

    // ----- Phase 115 — Failure-outcome heuristic -----

    #[test]
    fn failure_heuristic_default_fires_on_failed_and_timed_out() {
        let cfg = FailureHeuristicConfig::default();
        assert!(is_failure_candidate(FailureKind::Failed, &cfg));
        assert!(is_failure_candidate(FailureKind::TimedOut, &cfg));
        // Default-disabled kinds.
        assert!(!is_failure_candidate(FailureKind::Cancelled, &cfg));
        assert!(!is_failure_candidate(FailureKind::Escalated, &cfg));
    }

    #[test]
    fn failure_heuristic_explicit_enable_overrides_defaults() {
        let cfg = FailureHeuristicConfig {
            failed: false,
            cancelled: true,
            timed_out: false,
            escalated: true,
        };
        assert!(!is_failure_candidate(FailureKind::Failed, &cfg));
        assert!(is_failure_candidate(FailureKind::Cancelled, &cfg));
        assert!(!is_failure_candidate(FailureKind::TimedOut, &cfg));
        assert!(is_failure_candidate(FailureKind::Escalated, &cfg));
    }

    #[test]
    fn failure_heuristic_all_disabled_fires_on_nothing() {
        let cfg = FailureHeuristicConfig {
            failed: false,
            cancelled: false,
            timed_out: false,
            escalated: false,
        };
        for kind in [
            FailureKind::Failed,
            FailureKind::Cancelled,
            FailureKind::TimedOut,
            FailureKind::Escalated,
        ] {
            assert!(!is_failure_candidate(kind, &cfg));
        }
    }

    #[test]
    fn failure_heuristic_all_enabled_fires_on_everything() {
        let cfg = FailureHeuristicConfig {
            failed: true,
            cancelled: true,
            timed_out: true,
            escalated: true,
        };
        for kind in [
            FailureKind::Failed,
            FailureKind::Cancelled,
            FailureKind::TimedOut,
            FailureKind::Escalated,
        ] {
            assert!(is_failure_candidate(kind, &cfg));
        }
    }

    #[test]
    fn failure_kind_label_is_stable_lowercase() {
        assert_eq!(FailureKind::Failed.label(), "failed");
        assert_eq!(FailureKind::Cancelled.label(), "cancelled");
        assert_eq!(FailureKind::TimedOut.label(), "timed_out");
        assert_eq!(FailureKind::Escalated.label(), "escalated");
    }

    #[test]
    fn failure_kind_serializes_as_snake_case_string() {
        // Operator-readable wire form matches the labels above.
        let s = serde_json::to_string(&FailureKind::Failed).unwrap();
        assert_eq!(s, "\"failed\"");
        let s = serde_json::to_string(&FailureKind::TimedOut).unwrap();
        assert_eq!(s, "\"timed_out\"");
        let s = serde_json::to_string(&FailureKind::Cancelled).unwrap();
        assert_eq!(s, "\"cancelled\"");
        let s = serde_json::to_string(&FailureKind::Escalated).unwrap();
        assert_eq!(s, "\"escalated\"");
    }

    #[test]
    fn failure_heuristic_config_round_trips_through_serde_json() {
        let original = FailureHeuristicConfig {
            failed: true,
            cancelled: true,
            timed_out: false,
            escalated: false,
        };
        let s = serde_json::to_string(&original).unwrap();
        let back: FailureHeuristicConfig =
            serde_json::from_str(&s).unwrap();
        assert_eq!(back, original);
    }
}
