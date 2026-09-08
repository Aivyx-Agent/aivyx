//! `ChannelRateGate` — the concrete [`aivyx_core::RateGate`] (Chapter Throttle,
//! TH.3).
//!
//! `aivyx_core::RateGate` is a thin trait so `ConcreteAgent` stays ignorant of
//! `[rate_limit]` config and the counters. This is the concrete side: a
//! [`RateLimiter`] (the TH.1 limiter core) plus the trait glue that maps its
//! [`RateVerdict`] to the turn loop's admit/block decision.
//!
//! - **`Deny`** → `Err(reason)`. The turn loop turns that into
//!   [`ToolOutcome::RateLimited`](aivyx_core::ToolOutcome::RateLimited) and emits
//!   the dedicated `AuditTag::RateLimited` record, so the *deny* path is fully on
//!   the chain.
//! - **`Alert`** → a stderr warning (operator-visible in the daemon log) and
//!   `Ok(())`; the call proceeds and gets its normal `ToolCall` audit entry.
//! - **`Ok`** → `Ok(())`.
//!
//! Built only when the config is *active* (`RateLimitConfig::is_active`), so an
//! uncapped config attaches no gate and adds zero per-call overhead.

use std::sync::Arc;
use std::time::Instant;

use aivyx_core::RateGate;
use aivyx_cost::{RateLimitConfig, RateLimiter, RateVerdict};

/// The concrete rate gate: a [`RateLimiter`] over the operator's `[rate_limit]`.
pub struct ChannelRateGate {
    limiter: RateLimiter,
}

impl ChannelRateGate {
    /// Build a gate from the operator's `[rate_limit]` config, or `None` when
    /// no limit is configured (so callers attach nothing and keep ungated
    /// behavior byte-for-byte).
    pub fn new(config: RateLimitConfig) -> Option<Self> {
        config
            .is_active()
            .then(|| ChannelRateGate { limiter: RateLimiter::new(config) })
    }

    /// Convenience for agent-construction sites: build the gate already boxed as
    /// the `aivyx_core::RateGate` trait object, or `None` when uncapped.
    pub fn new_gate(config: RateLimitConfig) -> Option<Arc<dyn RateGate>> {
        Self::new(config).map(|g| Arc::new(g) as Arc<dyn RateGate>)
    }
}

impl RateGate for ChannelRateGate {
    fn admit_tool_call(&self, tool: &str) -> Result<(), String> {
        match self.limiter.check(tool, Instant::now()) {
            RateVerdict::Ok => Ok(()),
            RateVerdict::Alert(reason) => {
                eprintln!("aivyx-pa rate-limit: {reason} (alert — proceeding)");
                Ok(())
            }
            RateVerdict::Deny(reason) => Err(reason),
        }
    }

    fn begin_turn(&self) {
        self.limiter.reset_turn();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_cost::RateAction;
    use std::collections::HashMap;

    #[test]
    fn uncapped_config_builds_no_gate() {
        assert!(ChannelRateGate::new(RateLimitConfig::default()).is_none());
        assert!(ChannelRateGate::new_gate(RateLimitConfig::default()).is_none());
    }

    #[test]
    fn active_config_builds_a_gate() {
        let cfg = RateLimitConfig {
            default_per_turn_per_tool: Some(2),
            ..Default::default()
        };
        assert!(ChannelRateGate::new(cfg).is_some());
    }

    #[test]
    fn deny_blocks_after_cap_and_begin_turn_resets() {
        let cfg = RateLimitConfig {
            default_per_turn_per_tool: Some(2),
            on_exceeded: RateAction::Deny,
            ..Default::default()
        };
        let gate = ChannelRateGate::new(cfg).unwrap();
        assert!(gate.admit_tool_call("web.fetch").is_ok());
        assert!(gate.admit_tool_call("web.fetch").is_ok());
        assert!(gate.admit_tool_call("web.fetch").is_err(), "3rd over the cap of 2");
        // a new turn resets the per-turn quota
        gate.begin_turn();
        assert!(gate.admit_tool_call("web.fetch").is_ok());
    }

    #[test]
    fn alert_action_never_blocks() {
        let mut tools = HashMap::new();
        tools.insert(
            "shell.exec".to_string(),
            aivyx_cost::ToolRateConfig { per_turn: Some(1), ..Default::default() },
        );
        let cfg = RateLimitConfig {
            on_exceeded: RateAction::Alert,
            tools,
            ..Default::default()
        };
        let gate = ChannelRateGate::new(cfg).unwrap();
        for _ in 0..5 {
            assert!(gate.admit_tool_call("shell.exec").is_ok(), "alert never blocks");
        }
    }
}
