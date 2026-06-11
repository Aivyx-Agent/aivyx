//! `aivyx-cost` — cost governance (Chapter K).
//!
//! Free-core **visibility + caps** over LLM spend (not customer billing). This
//! crate's K.1 surface is the **pricing primitive**: turn a [`TokenCounts`]
//! record + a model name into a dollar [`Cost`], via a [`Pricing`] table of
//! shipped defaults that operator config can override. The priced view over
//! the audit chain (K.2) and budgets (K.3) build on this.
//!
//! Two deliberate semantics keep the numbers honest:
//!
//! - **Local models are free.** Ollama / llama / qwen / gemma / … price at
//!   `$0` with `priced = true` — local inference costs no API dollars.
//! - **Unknown cloud models are flagged, not guessed.** A model with no known
//!   rate prices at `$0` with **`priced = false`**, so the report can surface
//!   "untracked spend — add a `[pricing]` rate" rather than silently
//!   under-counting.
//!
//! Shipped default rates are **approximate published USD/Mtok as of early
//! 2026** and are a convenience only — `[pricing.<model>]` in `aivyx.toml`
//! (wired in K.5) is authoritative.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub mod report;
pub use report::{CostReport, ModelLine};

/// Token counts for one priced unit — a single turn, or a whole run summed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenCounts {
    pub input: u64,
    pub output: u64,
    /// Cached input tokens read back (cheaper than fresh input).
    pub cache_read: u64,
    /// Tokens written into the prompt cache (a premium over fresh input).
    pub cache_write: u64,
}

impl TokenCounts {
    /// Plain input/output counts (no prompt caching).
    pub fn new(input: u64, output: u64) -> Self {
        TokenCounts {
            input,
            output,
            cache_read: 0,
            cache_write: 0,
        }
    }

    /// Total tokens across all classes — the model-agnostic volume figure the
    /// loop's existing token cap uses.
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_write
    }

    /// Accumulate another record (for run/day aggregation in K.2).
    pub fn add(&mut self, other: &TokenCounts) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
    }
}

/// A model's price, in **USD per million tokens**, by token class.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ModelRate {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cache_read: f64,
    #[serde(default)]
    pub cache_write: f64,
}

impl ModelRate {
    /// A rate with only input/output set (cache classes default to 0).
    pub const fn io(input: f64, output: f64) -> Self {
        ModelRate {
            input,
            output,
            cache_read: 0.0,
            cache_write: 0.0,
        }
    }

    /// Builder: set the cache read/write rates.
    pub const fn with_cache(mut self, read: f64, write: f64) -> Self {
        self.cache_read = read;
        self.cache_write = write;
        self
    }

    /// Apply this rate to a usage record → dollars.
    fn apply(&self, t: &TokenCounts) -> f64 {
        const PER_MTOK: f64 = 1_000_000.0;
        (t.input as f64 * self.input
            + t.output as f64 * self.output
            + t.cache_read as f64 * self.cache_read
            + t.cache_write as f64 * self.cache_write)
            / PER_MTOK
    }
}

/// The result of pricing a usage record.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Cost {
    pub usd: f64,
    /// `false` when no rate is known for the model (an *untracked*, $0 entry
    /// the report flags — distinct from a genuinely-free local run, which is
    /// `usd = 0.0, priced = true`).
    pub priced: bool,
}

impl Cost {
    /// A known, computed cost.
    fn known(usd: f64) -> Self {
        Cost { usd, priced: true }
    }
    /// No rate known — untracked.
    fn untracked() -> Self {
        Cost {
            usd: 0.0,
            priced: false,
        }
    }
}

/// A model → [`ModelRate`] table: built-in family defaults plus exact-match
/// operator overrides (overrides win).
#[derive(Debug, Clone, Default)]
pub struct Pricing {
    /// Exact-model overrides (from `[pricing.<model>]` config in K.5).
    overrides: HashMap<String, ModelRate>,
}

impl Pricing {
    /// A pricing table with the built-in family defaults active and no
    /// overrides.
    pub fn new() -> Self {
        Pricing::default()
    }

    /// Set (or replace) the exact rate for a model — takes precedence over the
    /// built-in family defaults and the local-free heuristic.
    pub fn set_rate(&mut self, model: impl Into<String>, rate: ModelRate) {
        self.overrides.insert(model.into(), rate);
    }

    /// The rate that *would* apply to `model`, if any is known (override or
    /// default). `None` for local-free and unknown models — use
    /// [`cost_of`](Self::cost_of) for the full verdict.
    pub fn rate(&self, model: &str) -> Option<ModelRate> {
        if let Some(r) = self.overrides.get(model) {
            return Some(*r);
        }
        default_rate(model)
    }

    /// Price a usage record for `model`. Resolution order:
    /// 1. an exact operator override,
    /// 2. a **local** family → `$0`, *priced* (free inference),
    /// 3. a built-in cloud family default,
    /// 4. otherwise **untracked** (`$0`, `priced = false`).
    pub fn cost_of(&self, model: &str, tokens: &TokenCounts) -> Cost {
        if let Some(rate) = self.overrides.get(model) {
            return Cost::known(rate.apply(tokens));
        }
        if is_local_model(model) {
            return Cost::known(0.0);
        }
        match default_rate(model) {
            Some(rate) => Cost::known(rate.apply(tokens)),
            None => Cost::untracked(),
        }
    }
}

/// Whether a model name denotes **local** (free) inference. Matched by family
/// substring — local models run on the operator's own hardware, so they cost
/// no API dollars regardless of token volume.
pub fn is_local_model(model: &str) -> bool {
    const LOCAL_FAMILIES: &[&str] = &[
        "llama", "qwen", "gemma", "mistral", "mixtral", "phi", "deepseek", "codellama",
        "tinyllama", "nomic", "ollama", "vicuna", "starcoder", "granite",
    ];
    let m = model.to_ascii_lowercase();
    LOCAL_FAMILIES.iter().any(|f| m.contains(f))
}

/// The built-in default rate for a known **cloud** model family, by substring.
/// Approximate published USD/Mtok as of early 2026 — a convenience, overridden
/// by `[pricing.<model>]`. Anthropic cache: read ≈ 0.1× input, write ≈ 1.25×
/// input; OpenAI cached input ≈ 0.5× input (no separate write class).
fn default_rate(model: &str) -> Option<ModelRate> {
    let m = model.to_ascii_lowercase();
    // Order matters: more specific substrings (e.g. "gpt-4o-mini") first.
    if m.contains("opus") {
        Some(ModelRate::io(15.0, 75.0).with_cache(1.5, 18.75))
    } else if m.contains("sonnet") {
        Some(ModelRate::io(3.0, 15.0).with_cache(0.3, 3.75))
    } else if m.contains("haiku") {
        Some(ModelRate::io(0.8, 4.0).with_cache(0.08, 1.0))
    } else if m.contains("4o-mini") || m.contains("gpt-4o-mini") {
        Some(ModelRate::io(0.15, 0.6).with_cache(0.075, 0.0))
    } else if m.contains("gpt-4o") || m.contains("gpt-4.1") {
        Some(ModelRate::io(2.5, 10.0).with_cache(1.25, 0.0))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(input: u64, output: u64) -> TokenCounts {
        TokenCounts::new(input, output)
    }

    #[test]
    fn token_counts_total_and_add() {
        let mut a = TokenCounts {
            input: 100,
            output: 50,
            cache_read: 10,
            cache_write: 5,
        };
        assert_eq!(a.total(), 165);
        a.add(&TokenCounts::new(1, 2));
        assert_eq!(a.input, 101);
        assert_eq!(a.output, 52);
    }

    #[test]
    fn prices_a_known_cloud_model() {
        let p = Pricing::new();
        // 1M input + 1M output of Sonnet = $3 + $15 = $18.
        let c = p.cost_of("claude-sonnet-4-6", &counts(1_000_000, 1_000_000));
        assert!(c.priced);
        assert!((c.usd - 18.0).abs() < 1e-9, "got {}", c.usd);
    }

    #[test]
    fn opus_is_pricier_than_haiku() {
        let p = Pricing::new();
        let u = counts(1_000_000, 1_000_000);
        let opus = p.cost_of("claude-opus-4-8", &u).usd;
        let haiku = p.cost_of("claude-haiku-4-5", &u).usd;
        assert!(opus > haiku);
        assert!((opus - 90.0).abs() < 1e-9, "opus 15+75 per Mtok pair");
    }

    #[test]
    fn gpt_4o_mini_matches_before_gpt_4o() {
        let p = Pricing::new();
        // The mini substring must win over the broader gpt-4o rule.
        let mini = p.cost_of("gpt-4o-mini", &counts(1_000_000, 0)).usd;
        assert!((mini - 0.15).abs() < 1e-9, "got {mini}");
        let full = p.cost_of("gpt-4o", &counts(1_000_000, 0)).usd;
        assert!((full - 2.5).abs() < 1e-9, "got {full}");
    }

    #[test]
    fn cache_tokens_are_priced_by_class() {
        let p = Pricing::new();
        let u = TokenCounts {
            input: 0,
            output: 0,
            cache_read: 1_000_000,
            cache_write: 1_000_000,
        };
        // Sonnet cache: read 0.3 + write 3.75 = 4.05 per Mtok each.
        let c = p.cost_of("claude-sonnet-4-6", &u);
        assert!((c.usd - 4.05).abs() < 1e-9, "got {}", c.usd);
    }

    #[test]
    fn local_models_are_free_but_priced() {
        let p = Pricing::new();
        for model in ["llama3.1", "qwen3:27b", "gemma2", "mistral-nemo", "phi-4"] {
            let c = p.cost_of(model, &counts(5_000_000, 5_000_000));
            assert_eq!(c.usd, 0.0, "{model} local = free");
            assert!(c.priced, "{model} is tracked-free, not untracked");
        }
    }

    #[test]
    fn unknown_cloud_model_is_untracked_not_zero_priced() {
        let p = Pricing::new();
        let c = p.cost_of("some-new-frontier-model", &counts(1_000_000, 1_000_000));
        assert_eq!(c.usd, 0.0);
        assert!(!c.priced, "unknown ⇒ flagged untracked, not a real $0");
    }

    #[test]
    fn override_takes_precedence_over_defaults_and_local() {
        let mut p = Pricing::new();
        // Operator hosts a llama variant on a paid endpoint → real rate.
        p.set_rate("llama-hosted", ModelRate::io(1.0, 2.0));
        let c = p.cost_of("llama-hosted", &counts(1_000_000, 1_000_000));
        assert!(c.priced);
        assert!((c.usd - 3.0).abs() < 1e-9, "override beat the local-free heuristic");

        // And an override beats a built-in default.
        p.set_rate("claude-sonnet-4-6", ModelRate::io(99.0, 99.0));
        let c = p.cost_of("claude-sonnet-4-6", &counts(1_000_000, 0));
        assert!((c.usd - 99.0).abs() < 1e-9);
    }

    #[test]
    fn rate_lookup_reports_known_and_unknown() {
        let p = Pricing::new();
        assert!(p.rate("claude-opus-4-8").is_some());
        assert!(p.rate("llama3.1").is_none(), "local has no $ rate");
        assert!(p.rate("mystery-model").is_none());
    }

    #[test]
    fn zero_usage_is_zero_cost() {
        let p = Pricing::new();
        let c = p.cost_of("claude-opus-4-8", &TokenCounts::default());
        assert_eq!(c.usd, 0.0);
        assert!(c.priced);
    }

    #[test]
    fn model_rate_serde_roundtrip_defaults_cache_to_zero() {
        // Config may give only input/output; cache classes default to 0.
        let r: ModelRate = serde_json::from_str(r#"{"input":3.0,"output":15.0}"#).unwrap();
        assert_eq!(r.cache_read, 0.0);
        assert_eq!(r.cache_write, 0.0);
        let back = serde_json::to_string(&r).unwrap();
        let r2: ModelRate = serde_json::from_str(&back).unwrap();
        assert_eq!(r, r2);
    }

    #[test]
    fn case_insensitive_model_matching() {
        let p = Pricing::new();
        assert!(p.cost_of("CLAUDE-OPUS-4-8", &counts(1_000_000, 0)).priced);
        assert!(is_local_model("Llama3.1"));
    }
}
