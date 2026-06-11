//! `aivyx cost` — the priced spend report (Chapter K.4).
//!
//! Read-only, offline (the same cold-start storage open as `audit export` /
//! `--verify-only`: passphrase required, no session, no daemon). It scans the
//! HMAC chain for `AuditEvent::LlmCost` events — each carries the model + token
//! usage of one LLM-backed turn — prices them with the default [`Pricing`]
//! table, and renders a [`CostReport`]: total spend, a per-model breakdown,
//! and any **untracked** spend (models with no known rate). `--today` scopes
//! to the last 24h (the rolling window the per-day budget uses).
//!
//! Pricing here is the shipped defaults; operator `[pricing]` overrides land
//! with K.5.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use aivyx_audit::{AuditEvent, PersistentAuditLog, SignedEntry};
use aivyx_core::TokenUsage;
use aivyx_cost::{CostReport, Pricing, TokenCounts};
use aivyx_storage::Storage;

const PAGE_SIZE: usize = 1024;
const DAY: Duration = Duration::from_secs(24 * 3600);

/// Map an audit `TokenUsage` to the cost crate's `TokenCounts`
/// (cache_creation ⇒ cache_write, cache_read ⇒ cache_read).
fn to_counts(u: &TokenUsage) -> TokenCounts {
    TokenCounts {
        input: u.input_tokens as u64,
        output: u.output_tokens as u64,
        cache_read: u.cache_read_input_tokens as u64,
        cache_write: u.cache_creation_input_tokens as u64,
    }
}

/// `aivyx cost [--today]` — scan the chain's `LlmCost` events and print the
/// priced report. Offline; mirrors `run_audit_export`'s cold-start posture.
pub async fn run_cost(
    storage: Arc<dyn Storage>,
    audit_chain_key: [u8; 32],
    today: bool,
    pricing: Pricing,
) -> Result<(), String> {
    let log = PersistentAuditLog::open(storage, audit_chain_key)
        .await
        .map_err(|e| format!("failed to open audit chain: {e}"))?;

    let cutoff = today.then(|| SystemTime::now().checked_sub(DAY)).flatten();

    // Collect (model, counts) for every LlmCost event in the window.
    let mut entries: Vec<(String, TokenCounts)> = Vec::new();
    let mut cursor = 0u64;
    loop {
        let batch = log
            .entries_range(cursor, PAGE_SIZE)
            .map_err(|e| format!("audit-chain read failed at seq={cursor}: {e}"))?;
        if batch.is_empty() {
            break;
        }
        for entry in &batch {
            if let Some(pair) = llm_cost_in_window(entry, cutoff) {
                entries.push(pair);
            }
        }
        cursor = batch.last().map(|e| e.seq + 1).unwrap_or(cursor);
    }

    let report = CostReport::build(entries.iter().map(|(m, c)| (m.as_str(), *c)), &pricing);
    print!("{}", render_cost(&report, today));
    Ok(())
}

/// Extract `(model, counts)` from an entry iff it's an `LlmCost` event within
/// the window (`cutoff = None` ⇒ all time).
fn llm_cost_in_window(
    entry: &SignedEntry,
    cutoff: Option<SystemTime>,
) -> Option<(String, TokenCounts)> {
    let AuditEvent::LlmCost { model, usage, .. } = &entry.event else {
        return None;
    };
    if let Some(cutoff) = cutoff {
        if entry.appended_at < cutoff {
            return None;
        }
    }
    Some((model.clone(), to_counts(usage)))
}

/// Render a [`CostReport`] as an operator-readable block. Pure — the unit of
/// `aivyx cost`.
pub fn render_cost(report: &CostReport, today: bool) -> String {
    let window = if today { "last 24h" } else { "all time" };
    if report.is_empty() {
        return format!(
            "Cost report — {window}\n  no LLM spend recorded yet.\n  (local models are free; \
             cloud turns land here once they run.)\n"
        );
    }

    let mut out = format!("Cost report — {window}\n");
    out.push_str(&format!(
        "  total: {} over {} turn{}\n\n",
        usd(report.usd),
        report.turns,
        if report.turns == 1 { "" } else { "s" },
    ));

    out.push_str(&format!(
        "  {:<26}{:>7}{:>10}{:>10}{:>12}\n",
        "model", "turns", "input", "output", "cost",
    ));
    for line in report.by_model.values() {
        let cost = if !line.priced {
            "untracked".to_string()
        } else if line.usd == 0.0 {
            "free".to_string()
        } else {
            usd(line.usd)
        };
        out.push_str(&format!(
            "  {:<26}{:>7}{:>10}{:>10}{:>12}\n",
            truncate(&line.model, 26),
            line.turns,
            fmt_tokens(line.tokens.input),
            fmt_tokens(line.tokens.output),
            cost,
        ));
    }

    let untracked = report.untracked_models();
    if !untracked.is_empty() {
        out.push_str(&format!(
            "\n  \u{26a0} {} model{} untracked: {} — add a [pricing] rate to count it.\n",
            untracked.len(),
            if untracked.len() == 1 { "" } else { "s" },
            untracked.join(", "),
        ));
    }
    out
}

fn usd(v: f64) -> String {
    format!("${v:.2}")
}

/// Compact token counts: `1.2M`, `340K`, `512`.
fn fmt_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.0}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}…", &s[..max.saturating_sub(1)])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_cost::TokenCounts;

    fn io(i: u64, o: u64) -> TokenCounts {
        TokenCounts::new(i, o)
    }

    fn sample_report() -> CostReport {
        let entries = vec![
            ("claude-opus-4-8", io(1_000_000, 1_000_000)),
            ("claude-opus-4-8", io(500_000, 0)),
            ("llama3.1", io(2_000_000, 2_000_000)),
            ("mystery-model", io(1_000_000, 0)),
        ];
        CostReport::build(entries, &Pricing::new())
    }

    #[test]
    fn token_usage_maps_to_counts() {
        let u = TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            cache_creation_input_tokens: 7,
            cache_read_input_tokens: 3,
            ..Default::default()
        };
        let c = to_counts(&u);
        assert_eq!(c.input, 100);
        assert_eq!(c.output, 50);
        assert_eq!(c.cache_write, 7, "creation ⇒ write");
        assert_eq!(c.cache_read, 3);
    }

    #[test]
    fn renders_total_models_and_untracked() {
        let out = render_cost(&sample_report(), false);
        assert!(out.contains("Cost report — all time"));
        assert!(out.contains("claude-opus-4-8"));
        assert!(out.contains("llama3.1"));
        assert!(out.contains("free"), "local model shows free");
        assert!(out.contains("untracked"), "unknown model flagged in its row");
        assert!(out.contains("[pricing]"), "untracked warning points at config");
        // Opus: (1M in + 1M out) + (0.5M in) = $15+$75 + $7.5 = $97.50.
        assert!(out.contains("$97.50"), "opus line priced;\n{out}");
    }

    #[test]
    fn today_window_label() {
        assert!(render_cost(&sample_report(), true).contains("last 24h"));
    }

    #[test]
    fn empty_report_renders_hint() {
        let empty = CostReport::build(Vec::<(&str, TokenCounts)>::new(), &Pricing::new());
        let out = render_cost(&empty, false);
        assert!(out.contains("no LLM spend recorded yet"));
    }

    #[test]
    fn token_formatting() {
        assert_eq!(fmt_tokens(512), "512");
        assert_eq!(fmt_tokens(2_400), "2K");
        assert_eq!(fmt_tokens(1_500_000), "1.5M");
    }
}
