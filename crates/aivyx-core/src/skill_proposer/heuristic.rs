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
///
/// Phase 118 — extended with two operator-staged refinement
/// signals (`keyword_key_prior_total_count`,
/// `recent_scope_denied_count`) that feed the Profile/Role
/// auto-proposer paths. Both fields default to zero when the
/// caller hasn't sourced them (test-fixture builds that
/// pre-date Phase 118 still compile with `..Default::default()`
/// since `TurnSignals` derives `Default` from Phase 118 onward).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
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

    /// Phase 118 — cumulative outcome count for the current
    /// turn's keyword_key (Phase 116) in the relevance
    /// ledger. Sum of `success_count + failure_count` across
    /// every `OutcomeRow` under the keyword_key BEFORE this
    /// turn's own outcomes are recorded. Zero when the
    /// keyword_key has never been seen, or when the caller
    /// has no relevance ledger to query.
    ///
    /// The heuristic compares this against
    /// `profile_pattern_recurrence_min` to decide whether the
    /// operator's request shape has repeated enough times to
    /// be worth proposing a `ProfileHint`.
    pub keyword_key_prior_total_count: u32,

    /// Phase 118 — count of `ScopeDenied` audit events the
    /// caller observed in the recent session window. The
    /// window definition is caller-chosen (the daemon's
    /// audit-walk window is the natural default); the
    /// heuristic only cares about the count vs threshold.
    /// Zero when the caller has no audit-log access or
    /// when no scope-denials occurred.
    ///
    /// The heuristic compares this against
    /// `role_shape_scope_denied_min` to decide whether the
    /// current role's tool_allowlist / system_prompt
    /// envelope is misfit for the operator's request shape
    /// (worth proposing a `RoleDefinitionSuggestion` over).
    pub recent_scope_denied_count: u32,
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
///
/// Phase 118 — extended with two thresholds for the new
/// Profile/Role signals. Both are `#[serde(default)]` so
/// existing TOML files (pre-Phase-118) parse unchanged: the
/// absent fields receive the Phase 118 defaults below.
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

    /// Phase 118 — minimum cumulative outcome count under the
    /// current keyword_key for the
    /// `profile_pattern_repeated` signal to cross. Default
    /// `5`: the keyword_key has seen five+ prior
    /// (tool/skill) outcomes, suggesting the operator's
    /// request shape repeats.
    ///
    /// `#[serde(default)]` so pre-Phase-118 TOML parses; the
    /// `default_profile_pattern_recurrence_min` function
    /// supplies the default.
    #[serde(default = "default_profile_pattern_recurrence_min")]
    pub profile_pattern_recurrence_min: u32,

    /// Phase 118 — minimum count of recent `ScopeDenied`
    /// audit events (caller-defined window) for the
    /// `role_shape_recurring` signal to cross. Default `2`:
    /// two or more scope-denials in the window signal that
    /// the current role's envelope misfits the operator's
    /// request shape.
    ///
    /// `#[serde(default)]` for the same wire-compat reason
    /// as `profile_pattern_recurrence_min`.
    #[serde(default = "default_role_shape_scope_denied_min")]
    pub role_shape_scope_denied_min: u32,
}

fn default_profile_pattern_recurrence_min() -> u32 {
    5
}

fn default_role_shape_scope_denied_min() -> u32 {
    2
}

impl Default for HeuristicConfig {
    fn default() -> Self {
        HeuristicConfig {
            tool_call_count_min: 3,
            distinct_tool_id_min: 2,
            duration_ms_min: 5000,
            require_gate_resolve: false,
            mode: MatchMode::Any,
            profile_pattern_recurrence_min: default_profile_pattern_recurrence_min(),
            role_shape_scope_denied_min: default_role_shape_scope_denied_min(),
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
pub fn is_failure_candidate(kind: FailureKind, config: &FailureHeuristicConfig) -> bool {
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
///
/// Phase 118 — the two new operator-staged refinement
/// signals (`profile_pattern_repeated`,
/// `role_shape_recurring`) are treated as passive bonus
/// crossings under `MatchMode::Any` (each is a candidate path
/// on its own) and as "satisfied" (always true) under
/// `MatchMode::All` so they don't block the existing All
/// semantics — exact mirror of the gate-resolve handling.
pub fn is_candidate(signals: &TurnSignals, config: &HeuristicConfig) -> bool {
    let profile_signal_crossed =
        signals.keyword_key_prior_total_count >= config.profile_pattern_recurrence_min;
    let role_signal_crossed =
        signals.recent_scope_denied_count >= config.role_shape_scope_denied_min;

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
        // Phase 118 — profile_pattern_repeated. Passive bonus
        // signal: a crossing under Any-mode (the recurring
        // request shape alone is candidate-worthy); "satisfied"
        // under All-mode so existing four-signal All
        // calibrations don't break against the new axis.
        match config.mode {
            MatchMode::Any => profile_signal_crossed,
            MatchMode::All => true,
        },
        // Phase 118 — role_shape_recurring. Same treatment as
        // profile_pattern_repeated.
        match config.mode {
            MatchMode::Any => role_signal_crossed,
            MatchMode::All => true,
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
        // Phase 118 — derive(Default) on TurnSignals lets the
        // empty fixture stay one line. The pre-Phase-118
        // fields zero exactly as the old explicit struct
        // literal did.
        TurnSignals::default()
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
            ..TurnSignals::default()
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
            ..TurnSignals::default()
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
            ..TurnSignals::default()
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
            ..TurnSignals::default()
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
            ..TurnSignals::default()
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
            ..TurnSignals::default()
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
            ..TurnSignals::default()
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
            ..TurnSignals::default()
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
            ..TurnSignals::default()
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
            ..TurnSignals::default()
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
            ..TurnSignals::default()
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
            ..TurnSignals::default()
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
            ..TurnSignals::default()
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
            profile_pattern_recurrence_min: 10,
            role_shape_scope_denied_min: 4,
        };
        let s = serde_json::to_string(&original).expect("serialize");
        let back: HeuristicConfig = serde_json::from_str(&s).expect("deserialize");
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
        let back: FailureHeuristicConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back, original);
    }

    // ----- Phase 118 — Profile/Role signals -----

    #[test]
    fn phase_118_defaults_match_documented_values() {
        // Pin the calibration numbers from the open doc. Both
        // thresholds are operator-tunable; the defaults
        // reflect "fires when the operator's request shape
        // genuinely repeats" (5 prior outcomes) and "fires
        // when the role's envelope clearly misfits" (2
        // scope-denials).
        let c = HeuristicConfig::default();
        assert_eq!(c.profile_pattern_recurrence_min, 5);
        assert_eq!(c.role_shape_scope_denied_min, 2);
    }

    #[test]
    fn any_mode_fires_on_profile_pattern_repeated_alone() {
        // No tool calls, no duration, no gate — but the
        // keyword_key has prior outcomes crossing the
        // recurrence threshold. Phase 118 candidate path
        // for ProfileHint proposals.
        let signals = TurnSignals {
            keyword_key_prior_total_count: 5,
            ..TurnSignals::default()
        };
        assert!(is_candidate(&signals, &HeuristicConfig::default()));
    }

    #[test]
    fn any_mode_fires_on_role_shape_recurring_alone() {
        // No tool calls, no duration, no gate, no prior
        // keyword_key recurrence — but recent scope-denials
        // crossed the role-shape threshold. Phase 118
        // candidate path for RoleDefinitionSuggestion.
        let signals = TurnSignals {
            recent_scope_denied_count: 2,
            ..TurnSignals::default()
        };
        assert!(is_candidate(&signals, &HeuristicConfig::default()));
    }

    #[test]
    fn profile_pattern_signal_does_not_fire_below_threshold() {
        let signals = TurnSignals {
            keyword_key_prior_total_count: 4, // one short of default 5
            ..TurnSignals::default()
        };
        assert!(!is_candidate(&signals, &HeuristicConfig::default()));
    }

    #[test]
    fn role_shape_signal_does_not_fire_below_threshold() {
        let signals = TurnSignals {
            recent_scope_denied_count: 1, // one short of default 2
            ..TurnSignals::default()
        };
        assert!(!is_candidate(&signals, &HeuristicConfig::default()));
    }

    #[test]
    fn phase_118_signals_are_satisfied_under_all_mode_without_blocking() {
        // All-mode requires every signal to cross. The Phase
        // 118 signals are passive bonuses: under All-mode
        // they're treated as "satisfied" so they don't
        // block the existing 4-signal All calibrations.
        // This test mirrors `all_mode_requires_every_signal_to_cross`
        // but with the Phase 118 signals at zero.
        let config = HeuristicConfig {
            mode: MatchMode::All,
            ..HeuristicConfig::default()
        };
        let signals = TurnSignals {
            tool_calls_made: 3,
            distinct_tool_id_count: 2,
            duration: Duration::from_millis(5000),
            had_successful_gate_resolve: false,
            keyword_key_prior_total_count: 0,
            recent_scope_denied_count: 0,
        };
        assert!(is_candidate(&signals, &config));
    }

    #[test]
    fn phase_118_threshold_recurrence_is_inclusive() {
        let signals = TurnSignals {
            keyword_key_prior_total_count: 5, // exactly at default 5
            ..TurnSignals::default()
        };
        assert!(is_candidate(&signals, &HeuristicConfig::default()));
    }

    #[test]
    fn phase_118_threshold_scope_denied_is_inclusive() {
        let signals = TurnSignals {
            recent_scope_denied_count: 2, // exactly at default 2
            ..TurnSignals::default()
        };
        assert!(is_candidate(&signals, &HeuristicConfig::default()));
    }

    #[test]
    fn pre_phase_118_toml_decodes_with_default_thresholds() {
        // Operator's pre-Phase-118 TOML has no
        // profile_pattern_recurrence_min /
        // role_shape_scope_denied_min fields. The
        // #[serde(default)] attribute on the new fields must
        // supply the Phase 118 defaults without error.
        let pre_118 = r#"{
            "tool_call_count_min": 3,
            "distinct_tool_id_min": 2,
            "duration_ms_min": 5000,
            "require_gate_resolve": false,
            "mode": "any"
        }"#;
        let config: HeuristicConfig = serde_json::from_str(pre_118).expect("pre-118 TOML parses");
        assert_eq!(config.profile_pattern_recurrence_min, 5);
        assert_eq!(config.role_shape_scope_denied_min, 2);
    }
}
