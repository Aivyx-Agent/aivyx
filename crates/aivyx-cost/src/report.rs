//! `CostReport` — the priced aggregation over a turn's-worth of usage
//! (Chapter K.2).
//!
//! The audit chain holds one `AuditEvent::LlmCost { model, usage }` per
//! LLM-backed turn. The daemon maps those to `(model, TokenCounts)` pairs and
//! feeds them here; this module is **pure** — it prices each pair via a
//! [`Pricing`] table and rolls them up by model and in total. Period scoping
//! (today / this run / this session) is the *caller's* job: filter the entries
//! to the window, then build a report over them.
//!
//! Untracked spend is surfaced, never hidden: a turn on a model with no known
//! rate contributes `$0` but increments [`CostReport::untracked_turns`] and
//! lands in [`CostReport::untracked_models`], so a report can warn "add a
//! `[pricing]` rate" rather than quietly under-counting.

use std::collections::BTreeMap;

use crate::{Pricing, TokenCounts};

/// One model's rolled-up line in a [`CostReport`].
#[derive(Debug, Clone, PartialEq)]
pub struct ModelLine {
    pub model: String,
    /// LLM-backed turns attributed to this model.
    pub turns: u64,
    pub tokens: TokenCounts,
    /// Priced spend (USD). `0.0` when `priced == false`.
    pub usd: f64,
    /// Whether a rate is known for this model (`false` ⇒ untracked).
    pub priced: bool,
}

/// A priced roll-up of LLM usage over some set of turns.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CostReport {
    /// Per-model lines, keyed (and iterated) by model name.
    pub by_model: BTreeMap<String, ModelLine>,
    /// Total LLM-backed turns in the report.
    pub turns: u64,
    /// Total tokens across every model + class.
    pub tokens: TokenCounts,
    /// Total **priced** spend (USD). Untracked turns add `0.0`.
    pub usd: f64,
    /// Turns whose model had no known rate (their spend is uncounted).
    pub untracked_turns: u64,
}

impl CostReport {
    /// Price + aggregate `entries` (each a `(model, usage)` pair, e.g. mapped
    /// from `LlmCost` chain events) against `pricing`.
    pub fn build<'a, I>(entries: I, pricing: &Pricing) -> Self
    where
        I: IntoIterator<Item = (&'a str, TokenCounts)>,
    {
        let mut report = CostReport::default();
        for (model, tokens) in entries {
            let cost = pricing.cost_of(model, &tokens);
            let line = report
                .by_model
                .entry(model.to_string())
                .or_insert_with(|| ModelLine {
                    model: model.to_string(),
                    turns: 0,
                    tokens: TokenCounts::default(),
                    usd: 0.0,
                    priced: cost.priced,
                });
            line.turns += 1;
            line.tokens.add(&tokens);
            line.usd += cost.usd;

            report.turns += 1;
            report.tokens.add(&tokens);
            report.usd += cost.usd;
            if !cost.priced {
                report.untracked_turns += 1;
            }
        }
        report
    }

    /// No turns recorded.
    pub fn is_empty(&self) -> bool {
        self.turns == 0
    }

    /// Models that ran but have no known rate — the spend the operator should
    /// price by adding a `[pricing]` entry. Sorted, de-duplicated.
    pub fn untracked_models(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self
            .by_model
            .values()
            .filter(|l| !l.priced)
            .map(|l| l.model.as_str())
            .collect();
        v.sort_unstable();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn io(input: u64, output: u64) -> TokenCounts {
        TokenCounts::new(input, output)
    }

    fn report() -> CostReport {
        // Two sonnet turns, one opus, one local (free), one unknown cloud.
        let entries = vec![
            ("claude-sonnet-4-6", io(1_000_000, 1_000_000)), // $18
            ("claude-sonnet-4-6", io(1_000_000, 0)),         // $3
            ("claude-opus-4-8", io(1_000_000, 0)),           // $15
            ("llama3.1", io(5_000_000, 5_000_000)),          // $0 (priced, local)
            ("mystery-model", io(1_000_000, 1_000_000)),     // untracked
        ];
        CostReport::build(entries, &Pricing::new())
    }

    #[test]
    fn totals_sum_priced_spend_only() {
        let r = report();
        assert_eq!(r.turns, 5);
        // 18 + 3 + 15 + 0 (local) + 0 (untracked) = 36.
        assert!((r.usd - 36.0).abs() < 1e-9, "got {}", r.usd);
        assert_eq!(r.untracked_turns, 1);
    }

    #[test]
    fn aggregates_repeated_models() {
        let r = report();
        let sonnet = &r.by_model["claude-sonnet-4-6"];
        assert_eq!(sonnet.turns, 2);
        assert_eq!(sonnet.tokens.input, 2_000_000);
        assert_eq!(sonnet.tokens.output, 1_000_000);
        assert!((sonnet.usd - 21.0).abs() < 1e-9, "18 + 3");
        assert!(sonnet.priced);
    }

    #[test]
    fn local_model_line_is_priced_zero() {
        let r = report();
        let local = &r.by_model["llama3.1"];
        assert_eq!(local.usd, 0.0);
        assert!(local.priced, "local is tracked-free, not untracked");
    }

    #[test]
    fn untracked_models_are_flagged() {
        let r = report();
        assert_eq!(r.untracked_models(), vec!["mystery-model"]);
        let mystery = &r.by_model["mystery-model"];
        assert!(!mystery.priced);
        assert_eq!(mystery.usd, 0.0);
        // Its tokens are still counted in the volume total.
        assert!(r.tokens.input >= 1_000_000);
    }

    #[test]
    fn total_tokens_span_every_model() {
        let r = report();
        // input: 1M+1M+1M+5M+1M = 9M.
        assert_eq!(r.tokens.input, 9_000_000);
    }

    #[test]
    fn empty_report() {
        let r = CostReport::build(Vec::<(&str, TokenCounts)>::new(), &Pricing::new());
        assert!(r.is_empty());
        assert_eq!(r.usd, 0.0);
        assert!(r.untracked_models().is_empty());
    }
}
