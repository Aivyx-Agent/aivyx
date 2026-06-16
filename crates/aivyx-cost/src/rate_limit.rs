//! `RateLimiter` — tool-call rate limits & quotas (Chapter Throttle, TH.1).
//!
//! The dollar-budget's sibling for **call counts**. Capabilities gate *what* a
//! tool may do; [`BudgetEnforcer`](crate::BudgetEnforcer) gates *how much it
//! spends*; this gates *how often it is called*. It is **free core** —
//! self-protection + observability, not customer metering — and closes finding
//! **F2** of the 2026-06-16 backend audit (see `docs/RATE_LIMITS.md`).
//!
//! Same proven shape as the budget gate: a pure-logic evaluator plus in-memory
//! counters, an [`Alert`](RateAction::Alert) / [`Deny`](RateAction::Deny)
//! action, and a pre-call gate that mirrors [`BudgetEnforcer::reserve`] —
//! [`check`](RateLimiter::check) evaluates the configured limits and, when the
//! call may proceed, **records** it (so the next check sees it). A denied call
//! is blocked and *not* recorded (a burst of denials must not keep a window
//! full forever).
//!
//! ## The three limits
//!
//! 1. **Per-turn, per-tool** — at most `per_turn` calls to a tool in one turn.
//! 2. **Per-turn, total** — at most `per_turn_total` tool calls in one turn.
//! 3. **Sliding-window, per-tool** — at most `per_window` calls within
//!    `window_secs`, measured across turns / loop iterations.
//!
//! Per-turn state resets at [`reset_turn`](RateLimiter::reset_turn) (turn
//! boundary); the sliding window is daemon-lifetime, keyed by tool name. Time
//! is supplied by the caller as an [`Instant`] so window logic is deterministic
//! under test.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Default sliding-window length when `per_window` is set without `window_secs`.
const DEFAULT_WINDOW_SECS: u64 = 60;

/// What happens when a limit is exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RateAction {
    /// Warn but allow the call (it still executes and still counts).
    Alert,
    /// Block the call (the pre-call gate refuses; the call is not recorded).
    #[default]
    Deny,
}

/// Per-tool override of the rate limits. Any field left `None` falls back to the
/// section default (`default_per_turn_per_tool`) or disables that dimension.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolRateConfig {
    /// Max calls to this tool per turn (overrides `default_per_turn_per_tool`).
    #[serde(default)]
    pub per_turn: Option<u32>,
    /// Max calls to this tool within `window_secs` (the sliding window).
    #[serde(default)]
    pub per_window: Option<u32>,
    /// Sliding-window length in seconds. Defaults to 60 when `per_window` is set.
    #[serde(default)]
    pub window_secs: Option<u64>,
}

/// Call-frequency caps. Every field is optional; `None`/empty ⇒ that dimension
/// is unlimited. The whole section defaults to **uncapped** (opt-in), matching
/// budgets — an existing config behaves identically until the operator sets a
/// limit.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// Max tool calls in a single turn (across all tools).
    #[serde(default)]
    pub per_turn_total: Option<u32>,
    /// Per-tool, per-turn cap applied to every tool without a `[tools.*]` override.
    #[serde(default)]
    pub default_per_turn_per_tool: Option<u32>,
    /// Whether exceeding a limit alerts or denies.
    #[serde(default)]
    pub on_exceeded: RateAction,
    /// Per-tool overrides, keyed by tool name (e.g. `"web.fetch"`).
    #[serde(default)]
    pub tools: HashMap<String, ToolRateConfig>,
}

impl RateLimitConfig {
    /// Whether any limit is configured at all (cheap fast-path: an all-`None`
    /// config can skip the lock entirely in the hot dispatch loop).
    pub fn is_active(&self) -> bool {
        self.per_turn_total.is_some()
            || self.default_per_turn_per_tool.is_some()
            || !self.tools.is_empty()
    }
}

/// The outcome of a rate-limit evaluation. Mirrors `BudgetVerdict`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateVerdict {
    /// Within all limits — proceed.
    Ok,
    /// A limit is exceeded but its action is `Alert` — warn, but proceed.
    Alert(String),
    /// A `Deny` limit is exceeded — block.
    Deny(String),
}

impl RateVerdict {
    fn rank(&self) -> u8 {
        match self {
            RateVerdict::Ok => 0,
            RateVerdict::Alert(_) => 1,
            RateVerdict::Deny(_) => 2,
        }
    }
    /// Whether this verdict blocks the call.
    pub fn is_denied(&self) -> bool {
        matches!(self, RateVerdict::Deny(_))
    }
    /// The human-readable reason, if not `Ok`.
    pub fn reason(&self) -> Option<&str> {
        match self {
            RateVerdict::Ok => None,
            RateVerdict::Alert(r) | RateVerdict::Deny(r) => Some(r),
        }
    }
    /// Keep the more severe of two verdicts.
    fn worse(self, other: RateVerdict) -> RateVerdict {
        if other.rank() > self.rank() {
            other
        } else {
            self
        }
    }
}

/// Mutable counters, behind one lock.
#[derive(Default)]
struct State {
    /// Per-tool call count for the current turn.
    turn_per_tool: HashMap<String, u32>,
    /// Total tool calls in the current turn.
    turn_total: u32,
    /// Sliding-window call timestamps, per tool (only tools with a window).
    window: HashMap<String, VecDeque<Instant>>,
}

/// Evaluates tool calls against [`RateLimitConfig`], holding the per-turn and
/// sliding-window counters. One instance per daemon (shared across turns); the
/// turn loop calls [`reset_turn`](RateLimiter::reset_turn) at each turn boundary.
pub struct RateLimiter {
    config: RateLimitConfig,
    state: Mutex<State>,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        RateLimiter {
            config,
            state: Mutex::new(State::default()),
        }
    }

    pub fn config(&self) -> &RateLimitConfig {
        &self.config
    }

    /// Clear the per-turn counters at a turn boundary. The sliding window
    /// persists across turns (it bounds *sustained* rate, not per-turn bursts).
    pub fn reset_turn(&self) {
        let mut state = self.state.lock().expect("rate-limit lock");
        state.turn_per_tool.clear();
        state.turn_total = 0;
    }

    /// The effective sliding-window length for a tool.
    fn window_len(tcfg: &ToolRateConfig) -> Duration {
        Duration::from_secs(tcfg.window_secs.unwrap_or(DEFAULT_WINDOW_SECS))
    }

    /// Turn an exceeded-limit reason into a verdict per the configured action.
    fn exceeded(&self, reason: String) -> RateVerdict {
        match self.config.on_exceeded {
            RateAction::Deny => RateVerdict::Deny(reason),
            RateAction::Alert => RateVerdict::Alert(reason),
        }
    }

    /// Evaluate `tool` against every configured limit (does not mutate). The
    /// most severe verdict wins; the reason names the breached limit.
    fn evaluate(&self, tool: &str, state: &State, now: Instant) -> RateVerdict {
        let mut verdict = RateVerdict::Ok;

        // 1. per-turn total
        if let Some(cap) = self.config.per_turn_total {
            if state.turn_total >= cap {
                verdict = verdict.worse(self.exceeded(format!(
                    "per-turn tool-call cap reached: {} of {cap}",
                    state.turn_total
                )));
            }
        }

        // 2. per-turn, per-tool (override beats section default)
        let per_tool_cap = self
            .config
            .tools
            .get(tool)
            .and_then(|t| t.per_turn)
            .or(self.config.default_per_turn_per_tool);
        if let Some(cap) = per_tool_cap {
            let used = state.turn_per_tool.get(tool).copied().unwrap_or(0);
            if used >= cap {
                verdict = verdict.worse(self.exceeded(format!(
                    "per-turn cap for `{tool}` reached: {used} of {cap}"
                )));
            }
        }

        // 3. sliding window, per-tool
        if let Some(tcfg) = self.config.tools.get(tool) {
            if let Some(wcap) = tcfg.per_window {
                let window = Self::window_len(tcfg);
                let count = state
                    .window
                    .get(tool)
                    .map(|q| q.iter().filter(|t| now.duration_since(**t) < window).count())
                    .unwrap_or(0) as u32;
                if count >= wcap {
                    verdict = verdict.worse(self.exceeded(format!(
                        "sliding-window cap for `{tool}` reached: {count} in {}s of {wcap}",
                        window.as_secs()
                    )));
                }
            }
        }

        verdict
    }

    /// Pre-call gate. Evaluates `tool` at `now` against the configured limits;
    /// when the call may proceed (`Ok` or `Alert`) it is **recorded** so the
    /// next check sees it. A `Deny` blocks the call and records nothing.
    ///
    /// Mirrors [`BudgetEnforcer::reserve`]: the deny path is the enforcement
    /// boundary; `Alert` still proceeds (and still counts).
    pub fn check(&self, tool: &str, now: Instant) -> RateVerdict {
        let mut state = self.state.lock().expect("rate-limit lock");

        // Prune this tool's window to `now` so counts are current.
        if let Some(tcfg) = self.config.tools.get(tool) {
            if tcfg.per_window.is_some() {
                let window = Self::window_len(tcfg);
                if let Some(q) = state.window.get_mut(tool) {
                    while q.front().is_some_and(|t| now.duration_since(*t) >= window) {
                        q.pop_front();
                    }
                }
            }
        }

        let verdict = self.evaluate(tool, &state, now);
        if verdict.is_denied() {
            return verdict; // blocked → do not record
        }

        // Ok or Alert: the call proceeds, so it counts.
        *state.turn_per_tool.entry(tool.to_string()).or_insert(0) += 1;
        state.turn_total += 1;
        if self
            .config
            .tools
            .get(tool)
            .and_then(|t| t.per_window)
            .is_some()
        {
            state.window.entry(tool.to_string()).or_default().push_back(now);
        }
        verdict
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(total: Option<u32>, default_per_tool: Option<u32>, action: RateAction) -> RateLimitConfig {
        RateLimitConfig {
            per_turn_total: total,
            default_per_turn_per_tool: default_per_tool,
            on_exceeded: action,
            tools: HashMap::new(),
        }
    }

    // ---- no caps ----

    #[test]
    fn no_caps_always_ok() {
        let rl = RateLimiter::new(RateLimitConfig::default());
        let t = Instant::now();
        for _ in 0..1000 {
            assert_eq!(rl.check("web.fetch", t), RateVerdict::Ok);
        }
        assert!(!RateLimitConfig::default().is_active());
    }

    // ---- per-turn per-tool ----

    #[test]
    fn per_turn_per_tool_denies_the_n_plus_first() {
        let rl = RateLimiter::new(cfg(None, Some(3), RateAction::Deny));
        let t = Instant::now();
        for i in 0..3 {
            assert_eq!(rl.check("shell.exec", t), RateVerdict::Ok, "call {i} fits");
        }
        let v = rl.check("shell.exec", t);
        assert!(v.is_denied());
        assert!(v.reason().unwrap().contains("per-turn cap for `shell.exec`"));
        // A *different* tool is unaffected by shell.exec's count.
        assert_eq!(rl.check("web.fetch", t), RateVerdict::Ok);
    }

    #[test]
    fn per_tool_override_beats_default() {
        let mut c = cfg(None, Some(10), RateAction::Deny);
        c.tools.insert(
            "web.fetch".into(),
            ToolRateConfig { per_turn: Some(2), ..Default::default() },
        );
        let rl = RateLimiter::new(c);
        let t = Instant::now();
        assert_eq!(rl.check("web.fetch", t), RateVerdict::Ok);
        assert_eq!(rl.check("web.fetch", t), RateVerdict::Ok);
        assert!(rl.check("web.fetch", t).is_denied(), "tighter override applies");
        // the default (10) still governs other tools
        for _ in 0..10 {
            assert!(!rl.check("git.status", t).is_denied());
        }
        assert!(rl.check("git.status", t).is_denied());
    }

    // ---- per-turn total ----

    #[test]
    fn per_turn_total_caps_across_tools() {
        let rl = RateLimiter::new(cfg(Some(4), None, RateAction::Deny));
        let t = Instant::now();
        assert!(!rl.check("a", t).is_denied());
        assert!(!rl.check("b", t).is_denied());
        assert!(!rl.check("c", t).is_denied());
        assert!(!rl.check("d", t).is_denied());
        let v = rl.check("e", t);
        assert!(v.is_denied());
        assert!(v.reason().unwrap().contains("per-turn tool-call cap"));
    }

    // ---- reset_turn ----

    #[test]
    fn reset_turn_clears_per_turn_but_not_window() {
        let mut c = cfg(Some(2), None, RateAction::Deny);
        c.tools.insert(
            "web.fetch".into(),
            ToolRateConfig { per_window: Some(3), window_secs: Some(60), ..Default::default() },
        );
        let rl = RateLimiter::new(c);
        let t = Instant::now();
        assert!(!rl.check("web.fetch", t).is_denied());
        assert!(!rl.check("web.fetch", t).is_denied());
        assert!(rl.check("web.fetch", t).is_denied(), "per-turn total = 2 hit");
        rl.reset_turn();
        // per-turn total is cleared → fits again, but the window already holds
        // 2 of its cap of 3, so only one more (the 3rd) fits …
        assert!(!rl.check("web.fetch", t).is_denied());
        // … and the 4th hits the sliding-window cap, not the (reset) per-turn one.
        let v = rl.check("web.fetch", t);
        assert!(v.is_denied());
        assert!(v.reason().unwrap().contains("sliding-window"));
    }

    // ---- sliding window ----

    #[test]
    fn sliding_window_caps_then_expires() {
        let mut c = cfg(None, None, RateAction::Deny);
        c.tools.insert(
            "web.fetch".into(),
            ToolRateConfig { per_window: Some(2), window_secs: Some(30), ..Default::default() },
        );
        let rl = RateLimiter::new(c);
        let t0 = Instant::now();
        assert!(!rl.check("web.fetch", t0).is_denied());
        assert!(!rl.check("web.fetch", t0 + Duration::from_secs(5)).is_denied());
        // third within 30s → over the window cap of 2
        assert!(rl.check("web.fetch", t0 + Duration::from_secs(10)).is_denied());
        // once the first two age out (>30s past them) → fits again
        assert!(!rl.check("web.fetch", t0 + Duration::from_secs(40)).is_denied());
    }

    // ---- alert action ----

    #[test]
    fn alert_action_never_denies_and_keeps_counting() {
        let rl = RateLimiter::new(cfg(Some(2), None, RateAction::Alert));
        let t = Instant::now();
        assert_eq!(rl.check("x", t), RateVerdict::Ok);
        assert_eq!(rl.check("x", t), RateVerdict::Ok);
        // over the cap, but action is Alert → warn + proceed, every time
        for _ in 0..5 {
            match rl.check("x", t) {
                RateVerdict::Alert(r) => assert!(r.contains("per-turn tool-call cap")),
                other => panic!("alert action must not deny, got {other:?}"),
            }
        }
    }

    #[test]
    fn denied_call_is_not_recorded() {
        // A window of 1: the 2nd is denied; that denial must NOT consume window
        // slots, so after the first ages out exactly one slot frees.
        let mut c = cfg(None, None, RateAction::Deny);
        c.tools.insert(
            "web.fetch".into(),
            ToolRateConfig { per_window: Some(1), window_secs: Some(10), ..Default::default() },
        );
        let rl = RateLimiter::new(c);
        let t0 = Instant::now();
        assert!(!rl.check("web.fetch", t0).is_denied());
        // many denials in the window — none recorded
        for s in 1..5 {
            assert!(rl.check("web.fetch", t0 + Duration::from_secs(s)).is_denied());
        }
        // the single recorded call ages out at >10s → one slot frees, fits again
        assert!(!rl.check("web.fetch", t0 + Duration::from_secs(11)).is_denied());
    }

    // ---- severity ordering ----

    #[test]
    fn most_severe_limit_wins() {
        // total cap (deny) + per-tool default (deny): hitting the total first
        // still yields deny; reason names a breached limit.
        let rl = RateLimiter::new(cfg(Some(1), Some(5), RateAction::Deny));
        let t = Instant::now();
        assert!(!rl.check("a", t).is_denied());
        let v = rl.check("a", t); // total cap 1 already hit
        assert!(v.is_denied());
    }

    // ---- config ----

    #[test]
    fn config_default_is_uncapped_deny() {
        let c = RateLimitConfig::default();
        assert!(c.per_turn_total.is_none());
        assert!(c.default_per_turn_per_tool.is_none());
        assert_eq!(c.on_exceeded, RateAction::Deny);
        assert!(c.tools.is_empty());
        assert!(!c.is_active());
    }

    #[test]
    fn config_serde_roundtrip_and_partial() {
        let mut c = cfg(Some(40), Some(10), RateAction::Deny);
        c.tools.insert(
            "web.fetch".into(),
            ToolRateConfig { per_turn: Some(6), per_window: Some(20), window_secs: Some(60) },
        );
        let json = serde_json::to_string(&c).unwrap();
        let back: RateLimitConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
        // A partial config fills the rest from defaults.
        let partial: RateLimitConfig =
            serde_json::from_str(r#"{"per_turn_total":25}"#).unwrap();
        assert_eq!(partial.per_turn_total, Some(25));
        assert!(partial.default_per_turn_per_tool.is_none());
        assert_eq!(partial.on_exceeded, RateAction::Deny);
        assert!(partial.tools.is_empty());
    }

    #[test]
    fn action_serde_is_snake_case() {
        assert_eq!(serde_json::to_string(&RateAction::Deny).unwrap(), "\"deny\"");
        assert_eq!(serde_json::to_string(&RateAction::Alert).unwrap(), "\"alert\"");
    }

    #[test]
    fn window_secs_defaults_to_60_when_omitted() {
        let mut c = cfg(None, None, RateAction::Deny);
        c.tools.insert(
            "web.fetch".into(),
            ToolRateConfig { per_window: Some(1), window_secs: None, ..Default::default() },
        );
        let rl = RateLimiter::new(c);
        let t0 = Instant::now();
        assert!(!rl.check("web.fetch", t0).is_denied());
        // still inside the default 60s window → denied
        assert!(rl.check("web.fetch", t0 + Duration::from_secs(30)).is_denied());
        // past 60s → fits
        assert!(!rl.check("web.fetch", t0 + Duration::from_secs(61)).is_denied());
    }
}
