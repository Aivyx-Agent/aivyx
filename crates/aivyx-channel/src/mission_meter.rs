//! Chapter Ballast (Opp D) — per-mission spend metering.
//!
//! A loop-delegated team mission runs many specialist sub-turns, each a
//! `ConcreteAgent` turn that emits an [`AuditTag::LlmCost`] onto the shared
//! HMAC audit chain. The autonomous loop's run-window caps already see that
//! spend (they scan the whole chain window), but a *single* mission had no
//! aggregate self-limit beyond the per-call `max_tokens` generation cap — so a
//! runaway mission could spend without bound.
//!
//! [`MeteringAuditHook`] closes that gap with exact per-mission isolation: it
//! wraps the real audit hook, and on every `LlmCost` it (1) tallies the priced
//! tokens + USD for *this mission* into atomics and (2) forwards the event
//! unchanged to the real chain. Because each mission gets its own hook, the
//! atomics measure only that mission's spend — no audit-chain attribution
//! guesswork, and no contamination from concurrent daemon turns. The forward
//! keeps the loop's run-window accounting intact (delegated spend still counts
//! toward `max_run_tokens` / `max_run_usd`).
//!
//! The team mission driver reads the atomics at each wave boundary and halts
//! the mission gracefully when an [`aivyx_cost::MissionBudget`] cap trips.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use aivyx_core::{AuditHook, AuditTag};

/// Wraps a real [`AuditHook`], tallying this mission's priced spend while
/// forwarding every event through unchanged.
pub struct MeteringAuditHook {
    inner: Arc<dyn AuditHook>,
    pricing: Arc<aivyx_cost::Pricing>,
    /// Cumulative tokens (input + output + cache) across the mission.
    tokens: Arc<AtomicU64>,
    /// Cumulative priced spend in **micro-dollars** (USD × 1e6) so it stays an
    /// integer atomic; local models price at $0 and never advance it.
    micro_usd: Arc<AtomicU64>,
}

/// A cheap read-only handle to a [`MeteringAuditHook`]'s running totals, shared
/// with the driver's wave-boundary halt check.
#[derive(Clone)]
pub struct MissionMeter {
    tokens: Arc<AtomicU64>,
    micro_usd: Arc<AtomicU64>,
}

impl MissionMeter {
    /// Tokens metered so far this mission.
    pub fn tokens(&self) -> u64 {
        self.tokens.load(Ordering::Relaxed)
    }

    /// Priced USD metered so far this mission.
    pub fn usd(&self) -> f64 {
        self.micro_usd.load(Ordering::Relaxed) as f64 / 1_000_000.0
    }
}

impl MeteringAuditHook {
    /// Wrap `inner`, pricing spend with `pricing`, starting the running
    /// totals from `seed_tokens`/`seed_usd` rather than zero. Chapter
    /// Mission Control — used when a mission's drive is a resume (or a
    /// gate-approval continuation, or a Chapter Reprise retry), so a
    /// mission's [`aivyx_cost::MissionBudget`] cap tracks CUMULATIVE spend
    /// across the whole mission's lifetime, not just the current drive
    /// invocation. Without this, a `[budget]` cap resets every time
    /// `drive_registered` is called again for the same mission — trivially
    /// evadable via repeated pause/resume.
    pub fn with_seed(
        inner: Arc<dyn AuditHook>,
        pricing: Arc<aivyx_cost::Pricing>,
        seed_tokens: u64,
        seed_usd: f64,
    ) -> Self {
        // Same rounding as on_event's own usd -> micro-dollar conversion,
        // so a seeded value and an accumulated value are on identical
        // footing (no drift from two different rounding rules).
        let seed_micro_usd = (seed_usd.max(0.0) * 1_000_000.0).round() as u64;
        MeteringAuditHook {
            inner,
            pricing,
            tokens: Arc::new(AtomicU64::new(seed_tokens)),
            micro_usd: Arc::new(AtomicU64::new(seed_micro_usd)),
        }
    }

    /// Wrap `inner`, pricing spend with `pricing`, starting from zero.
    pub fn new(inner: Arc<dyn AuditHook>, pricing: Arc<aivyx_cost::Pricing>) -> Self {
        Self::with_seed(inner, pricing, 0, 0.0)
    }

    /// A read handle on the running totals for the driver's halt check.
    pub fn meter(&self) -> MissionMeter {
        MissionMeter {
            tokens: Arc::clone(&self.tokens),
            micro_usd: Arc::clone(&self.micro_usd),
        }
    }
}

impl AuditHook for MeteringAuditHook {
    fn on_event(&self, tag: AuditTag) {
        // Tally before forwarding; `LlmCost` is the only spend-bearing tag.
        if let AuditTag::LlmCost { model, usage, .. } = &tag {
            let counts = aivyx_cost::TokenCounts {
                input: usage.input_tokens as u64,
                output: usage.output_tokens as u64,
                cache_read: usage.cache_read_input_tokens as u64,
                cache_write: usage.cache_creation_input_tokens as u64,
            };
            let total_tokens = counts.input
                + counts.output
                + counts.cache_read
                + counts.cache_write;
            self.tokens.fetch_add(total_tokens, Ordering::Relaxed);
            let usd = self.pricing.cost_of(model, &counts).usd;
            if usd > 0.0 {
                let micro = (usd * 1_000_000.0).round() as u64;
                self.micro_usd.fetch_add(micro, Ordering::Relaxed);
            }
        }
        self.inner.on_event(tag);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_core::{TokenUsage, TurnId};
    use std::sync::Mutex;

    /// Records the tags it receives, to prove the metering hook forwards.
    #[derive(Default)]
    struct CapturingHook {
        events: Mutex<Vec<AuditTag>>,
    }
    impl AuditHook for CapturingHook {
        fn on_event(&self, tag: AuditTag) {
            self.events.lock().unwrap().push(tag);
        }
    }

    fn llm_cost(model: &str, input: u32, output: u32) -> AuditTag {
        AuditTag::LlmCost {
            turn_id: TurnId::new(),
            model: model.to_string(),
            usage: TokenUsage {
                input_tokens: input,
                output_tokens: output,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
                context_tokens_before_pruning: 0,
                context_tokens_after_pruning: 0,
            },
        }
    }

    #[test]
    fn meters_tokens_and_forwards_every_event() {
        let inner = Arc::new(CapturingHook::default());
        let hook = MeteringAuditHook::new(
            inner.clone(),
            Arc::new(aivyx_cost::Pricing::default()),
        );
        let meter = hook.meter();

        hook.on_event(llm_cost("some-local-model", 100, 50));
        hook.on_event(llm_cost("some-local-model", 200, 25));

        // Tokens accumulate across sub-turns…
        assert_eq!(meter.tokens(), 100 + 50 + 200 + 25);
        // …and every event still reached the real chain.
        assert_eq!(inner.events.lock().unwrap().len(), 2);
    }

    #[test]
    fn unknown_or_local_models_price_at_zero_usd() {
        let inner = Arc::new(CapturingHook::default());
        let hook = MeteringAuditHook::new(
            inner,
            Arc::new(aivyx_cost::Pricing::default()),
        );
        let meter = hook.meter();
        // A model with no known rate prices at $0 (the cost ledger's contract),
        // so the dollar meter stays flat while tokens still accrue.
        hook.on_event(llm_cost("ollama:qwen3", 1000, 1000));
        assert_eq!(meter.tokens(), 2000);
        assert_eq!(meter.usd(), 0.0);
    }

    #[test]
    fn with_seed_starts_from_the_given_totals_not_zero() {
        let inner = Arc::new(CapturingHook::default());
        let hook = MeteringAuditHook::with_seed(
            inner,
            Arc::new(aivyx_cost::Pricing::default()),
            500,
            0.02,
        );
        let meter = hook.meter();
        assert_eq!(meter.tokens(), 500);
        assert!((meter.usd() - 0.02).abs() < 1e-9);
    }

    #[test]
    fn new_still_starts_from_zero() {
        let inner = Arc::new(CapturingHook::default());
        let hook = MeteringAuditHook::new(inner, Arc::new(aivyx_cost::Pricing::default()));
        let meter = hook.meter();
        assert_eq!(meter.tokens(), 0);
        assert_eq!(meter.usd(), 0.0);
    }

    #[test]
    fn non_cost_events_do_not_meter() {
        let inner = Arc::new(CapturingHook::default());
        let hook = MeteringAuditHook::new(
            inner.clone(),
            Arc::new(aivyx_cost::Pricing::default()),
        );
        let meter = hook.meter();
        hook.on_event(AuditTag::HeadlessRefusal {
            run_id: "m1".into(),
            step: "s1".into(),
            reason: "x".into(),
        });
        assert_eq!(meter.tokens(), 0);
        assert_eq!(meter.usd(), 0.0);
        assert_eq!(inner.events.lock().unwrap().len(), 1);
    }
}
