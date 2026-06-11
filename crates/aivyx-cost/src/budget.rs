//! `BudgetEnforcer` — spending caps over the priced ledger (Chapter K.3).
//!
//! Budgets turn the [`CostReport`](crate::CostReport) numbers into a *control*:
//! configurable **per-run** and **per-day** dollar caps that either **alert**
//! (warn, proceed) or **deny** (block the call). The committed-spend figures
//! are computed by the caller from the chain (a `CostReport` over the right
//! window) and passed in — so the enforcer itself is pure logic plus one piece
//! of state: **reservations**.
//!
//! ## Reservations — the team-concurrency gate
//!
//! A team runs specialists **concurrently**, so two in-flight LLM calls can
//! both pass a naive committed-spend check and *jointly* bust the cap (a
//! TOCTOU race). [`reserve`](BudgetEnforcer::reserve) closes it: under a lock
//! it checks `committed + already-reserved + this estimate` against the caps,
//! and only then records the reservation. After the call, the caller
//! [`release`](BudgetEnforcer::release)s it — the *actual* cost is by then on
//! the chain (an `LlmCost` event), so the next `committed` figure already
//! reflects it. Reserve is the pre-call **deny** gate; [`check`] is the
//! committed-spend **status** (used for alerts + `aivyx cost`).

use serde::{Deserialize, Serialize};
use std::sync::Mutex;

/// What happens when a cap is exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BudgetAction {
    /// Warn but allow the call.
    Alert,
    /// Block the call (the pre-call reserve gate refuses).
    #[default]
    Deny,
}

/// Spending caps. `None` ⇒ that dimension is unlimited.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BudgetConfig {
    /// Max USD per **run** — one mission / loop run / session.
    #[serde(default)]
    pub per_run_usd: Option<f64>,
    /// Max USD per **rolling day**.
    #[serde(default)]
    pub per_day_usd: Option<f64>,
    /// Whether exceeding a cap alerts or denies.
    #[serde(default)]
    pub on_exceeded: BudgetAction,
    /// Warn once spend crosses this fraction of a cap (e.g. `0.8` = 80%).
    /// `None` disables the early-warning tier.
    #[serde(default = "default_alert_at")]
    pub alert_at: Option<f64>,
}

fn default_alert_at() -> Option<f64> {
    Some(0.8)
}

impl Default for BudgetConfig {
    /// No caps (opt-in): the free core meters by default and only *enforces*
    /// when the operator sets a limit.
    fn default() -> Self {
        BudgetConfig {
            per_run_usd: None,
            per_day_usd: None,
            on_exceeded: BudgetAction::Deny,
            alert_at: default_alert_at(),
        }
    }
}

/// The outcome of a budget evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetVerdict {
    /// Within budget — proceed.
    Ok,
    /// Past an alert threshold (or over a cap whose action is `Alert`) — warn,
    /// but proceed.
    Alert(String),
    /// A `Deny` cap is exceeded — block.
    Deny(String),
}

impl BudgetVerdict {
    /// Severity rank for picking the worst across dimensions.
    fn rank(&self) -> u8 {
        match self {
            BudgetVerdict::Ok => 0,
            BudgetVerdict::Alert(_) => 1,
            BudgetVerdict::Deny(_) => 2,
        }
    }
    /// Whether this verdict blocks the call.
    pub fn is_denied(&self) -> bool {
        matches!(self, BudgetVerdict::Deny(_))
    }
    /// The human-readable reason, if not `Ok`.
    pub fn reason(&self) -> Option<&str> {
        match self {
            BudgetVerdict::Ok => None,
            BudgetVerdict::Alert(r) | BudgetVerdict::Deny(r) => Some(r),
        }
    }
    /// Keep the more severe of two verdicts.
    fn worse(self, other: BudgetVerdict) -> BudgetVerdict {
        if other.rank() > self.rank() {
            other
        } else {
            self
        }
    }
}

/// A held reservation — the estimated cost of an in-flight call. Returned by
/// [`BudgetEnforcer::reserve`]; hand it back to
/// [`release`](BudgetEnforcer::release) when the call finishes.
#[derive(Debug)]
#[must_use = "a reservation must be released (or the budget stays held)"]
pub struct Reservation {
    amount_usd: f64,
}

/// Evaluates spend against [`BudgetConfig`], with reservation state for
/// concurrent (team) calls.
pub struct BudgetEnforcer {
    config: BudgetConfig,
    /// Sum of active reservations' estimated cost (USD).
    reserved: Mutex<f64>,
}

impl BudgetEnforcer {
    pub fn new(config: BudgetConfig) -> Self {
        BudgetEnforcer {
            config,
            reserved: Mutex::new(0.0),
        }
    }

    pub fn config(&self) -> &BudgetConfig {
        &self.config
    }

    /// Total currently reserved (in-flight) USD.
    pub fn reserved(&self) -> f64 {
        *self.reserved.lock().expect("reservation lock")
    }

    /// Evaluate **committed** spend against the caps — the status verdict
    /// (alerts, `aivyx cost`). Does not consider reservations.
    pub fn check(&self, day_usd: f64, run_usd: f64) -> BudgetVerdict {
        let run = self.eval("run", run_usd, self.config.per_run_usd);
        let day = self.eval("day", day_usd, self.config.per_day_usd);
        run.worse(day)
    }

    /// One dimension's verdict.
    fn eval(&self, label: &str, spent: f64, cap: Option<f64>) -> BudgetVerdict {
        let Some(cap) = cap else {
            return BudgetVerdict::Ok;
        };
        if spent >= cap {
            let reason = format!("{label} budget exceeded: ${spent:.2} of ${cap:.2}");
            return match self.config.on_exceeded {
                BudgetAction::Deny => BudgetVerdict::Deny(reason),
                BudgetAction::Alert => BudgetVerdict::Alert(reason),
            };
        }
        if let Some(frac) = self.config.alert_at {
            if frac > 0.0 && spent >= frac * cap {
                return BudgetVerdict::Alert(format!(
                    "{label} budget at {:.0}%: ${spent:.2} of ${cap:.2}",
                    spent / cap * 100.0
                ));
            }
        }
        BudgetVerdict::Ok
    }

    /// Atomically reserve `estimate_usd` for an upcoming call, checking
    /// `committed + reserved + estimate` against any **`Deny`** cap. Returns
    /// the [`Reservation`] on success, or `Err(BudgetVerdict::Deny)` when a
    /// deny-cap would be busted (the call must not proceed). When the action
    /// is `Alert`, reservations never block.
    pub fn reserve(
        &self,
        day_usd: f64,
        run_usd: f64,
        estimate_usd: f64,
    ) -> Result<Reservation, BudgetVerdict> {
        let mut reserved = self.reserved.lock().expect("reservation lock");
        if self.config.on_exceeded == BudgetAction::Deny {
            for (label, spent, cap) in [
                ("run", run_usd, self.config.per_run_usd),
                ("day", day_usd, self.config.per_day_usd),
            ] {
                if let Some(cap) = cap {
                    let projected = spent + *reserved + estimate_usd;
                    if projected > cap {
                        return Err(BudgetVerdict::Deny(format!(
                            "{label} budget would be exceeded: ${spent:.2} spent + ${:.2} reserved \
                             + ${estimate_usd:.2} estimated > ${cap:.2}",
                            *reserved
                        )));
                    }
                }
            }
        }
        *reserved += estimate_usd;
        Ok(Reservation {
            amount_usd: estimate_usd,
        })
    }

    /// Release a reservation once its call has finished (succeeded *or*
    /// failed). The actual cost, if any, is already on the chain — so the next
    /// `committed` figure reflects it; this just frees the held estimate.
    pub fn release(&self, reservation: Reservation) {
        let mut reserved = self.reserved.lock().expect("reservation lock");
        *reserved = (*reserved - reservation.amount_usd).max(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(run: Option<f64>, day: Option<f64>, action: BudgetAction) -> BudgetConfig {
        BudgetConfig {
            per_run_usd: run,
            per_day_usd: day,
            on_exceeded: action,
            alert_at: Some(0.8),
        }
    }

    // ---- check (committed status) ----

    #[test]
    fn no_caps_always_ok() {
        let e = BudgetEnforcer::new(BudgetConfig::default());
        assert_eq!(e.check(9_999.0, 9_999.0), BudgetVerdict::Ok);
    }

    #[test]
    fn under_threshold_is_ok() {
        let e = BudgetEnforcer::new(caps(Some(10.0), None, BudgetAction::Deny));
        assert_eq!(e.check(0.0, 5.0), BudgetVerdict::Ok, "50% < 80% alert");
    }

    #[test]
    fn alert_tier_warns_before_the_cap() {
        let e = BudgetEnforcer::new(caps(Some(10.0), None, BudgetAction::Deny));
        match e.check(0.0, 8.5) {
            BudgetVerdict::Alert(r) => assert!(r.contains("run budget at")),
            other => panic!("expected Alert, got {other:?}"),
        }
    }

    #[test]
    fn deny_at_or_over_the_cap() {
        let e = BudgetEnforcer::new(caps(Some(10.0), None, BudgetAction::Deny));
        let v = e.check(0.0, 10.0);
        assert!(v.is_denied());
        assert!(v.reason().unwrap().contains("run budget exceeded"));
    }

    #[test]
    fn alert_action_never_denies_even_over_cap() {
        let e = BudgetEnforcer::new(caps(Some(10.0), None, BudgetAction::Alert));
        match e.check(0.0, 50.0) {
            BudgetVerdict::Alert(_) => {}
            other => panic!("alert action must not deny, got {other:?}"),
        }
    }

    #[test]
    fn run_and_day_are_independent_most_severe_wins() {
        let e = BudgetEnforcer::new(caps(Some(10.0), Some(100.0), BudgetAction::Deny));
        // Run over (deny) but day fine → deny names run.
        let v = e.check(20.0, 11.0);
        assert!(v.is_denied());
        assert!(v.reason().unwrap().contains("run"));
        // Day alert + run ok → alert.
        assert!(matches!(e.check(85.0, 1.0), BudgetVerdict::Alert(r) if r.contains("day")));
    }

    // ---- reservations (concurrency gate) ----

    #[test]
    fn reserve_under_cap_succeeds_and_tracks() {
        let e = BudgetEnforcer::new(caps(Some(20.0), None, BudgetAction::Deny));
        let r = e.reserve(0.0, 0.0, 12.0).expect("under cap");
        assert!((e.reserved() - 12.0).abs() < 1e-9);
        e.release(r);
        assert_eq!(e.reserved(), 0.0);
    }

    #[test]
    fn concurrent_reservations_cannot_jointly_bust_the_cap() {
        // The TOCTOU case: two in-flight calls, each fine alone.
        let e = BudgetEnforcer::new(caps(Some(20.0), None, BudgetAction::Deny));
        let _r1 = e.reserve(0.0, 0.0, 12.0).expect("first fits");
        // Second: 0 spent + 12 reserved + 12 = 24 > 20 → denied.
        let err = e.reserve(0.0, 0.0, 12.0).unwrap_err();
        assert!(err.is_denied());
        assert!(err.reason().unwrap().contains("reserved"));
    }

    #[test]
    fn reserve_accounts_for_committed_spend() {
        let e = BudgetEnforcer::new(caps(Some(20.0), None, BudgetAction::Deny));
        // 15 already spent, reserve 10 → 25 > 20 → denied.
        let err = e.reserve(0.0, 15.0, 10.0).unwrap_err();
        assert!(err.is_denied());
    }

    #[test]
    fn releasing_frees_the_reservation() {
        let e = BudgetEnforcer::new(caps(Some(20.0), None, BudgetAction::Deny));
        let r = e.reserve(0.0, 0.0, 15.0).unwrap();
        assert!(e.reserve(0.0, 0.0, 10.0).is_err(), "15 + 10 > 20");
        e.release(r);
        assert!(e.reserve(0.0, 0.0, 10.0).is_ok(), "freed → fits");
    }

    #[test]
    fn alert_action_reservations_never_block() {
        let e = BudgetEnforcer::new(caps(Some(1.0), None, BudgetAction::Alert));
        // Way over, but action is Alert → reserve still succeeds.
        assert!(e.reserve(0.0, 100.0, 100.0).is_ok());
    }

    #[test]
    fn day_cap_also_gates_reservations() {
        let e = BudgetEnforcer::new(caps(None, Some(50.0), BudgetAction::Deny));
        let err = e.reserve(45.0, 0.0, 10.0).unwrap_err();
        assert!(err.is_denied());
        assert!(err.reason().unwrap().contains("day"));
    }

    // ---- config ----

    #[test]
    fn config_default_is_uncapped_deny_with_80pct_alert() {
        let c = BudgetConfig::default();
        assert!(c.per_run_usd.is_none() && c.per_day_usd.is_none());
        assert_eq!(c.on_exceeded, BudgetAction::Deny);
        assert_eq!(c.alert_at, Some(0.8));
    }

    #[test]
    fn config_serde_roundtrip_and_partial() {
        let c = caps(Some(5.0), Some(50.0), BudgetAction::Deny);
        let json = serde_json::to_string(&c).unwrap();
        let back: BudgetConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(c, back);
        // A partial config (only per_day) fills the rest from defaults.
        let partial: BudgetConfig = serde_json::from_str(r#"{"per_day_usd":25.0}"#).unwrap();
        assert_eq!(partial.per_day_usd, Some(25.0));
        assert!(partial.per_run_usd.is_none());
        assert_eq!(partial.on_exceeded, BudgetAction::Deny);
        assert_eq!(partial.alert_at, Some(0.8));
    }

    #[test]
    fn action_serde_is_snake_case() {
        assert_eq!(serde_json::to_string(&BudgetAction::Deny).unwrap(), "\"deny\"");
        assert_eq!(serde_json::to_string(&BudgetAction::Alert).unwrap(), "\"alert\"");
    }
}
