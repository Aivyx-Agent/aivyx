# Cost Governance — Token Accounting & Budgets (Chapter K)

> **Status:** design contract. This is the spec Chapter K scaffolds from.
>
> Cost governance gives the operator **visibility and control over LLM
> spend**: every turn's token usage is priced into dollars, aggregated into
> a report, and bounded by **budgets** (caps that *alert* or *deny* when
> exceeded). It is **free core** — observability + safety, not customer
> billing — and it matters most exactly where Aivyx now spends the most:
> the **autonomous loop** and **multi-agent team missions**, which multiply
> token consumption.
>
> Lineage: re-grounded from the pre-rebuild archive's `aivyx-billing` crate
> (CostLedger + BudgetEnforcer), **simplified to single-operator** and
> **leveraging usage the core already records**.

---

## 1. The key insight — usage is already on the chain

Aivyx already records, on the **one HMAC audit chain**, an
`AuditEvent::TurnEnded { usage: TokenUsage }` for every turn — and the
autonomous loop already *aggregates* it (`loop_driver::sum_turn_usage` sums
`input_tokens + output_tokens` over `TurnEnded` events to enforce
`max_run_tokens`). So Chapter K is **not** a from-scratch ledger: the chain
*is* the token ledger. K adds the missing pieces:

1. **Pricing** — turn `TokenUsage` into **dollars** (per-model rates).
2. **Aggregation + reporting** — daily / session / per-agent **$ totals**,
   generalising the loop's per-run token sum.
3. **Budgets** — **$ caps** (per-run / per-day) that *alert* or *deny*,
   generalising the loop's single `max_run_tokens` cap.
4. **Surfaces** — an `aivyx cost` report + `[budget]` / `[pricing]` config.

This is **leaner than the archive**, which built a separate encrypted ledger
store. We reuse the chain as the source of truth and add a thin priced view
over it.

## 2. What's deliberately *not* here (vs. the SaaS archive)

- **Multi-tenant.** Dropped. The free core is **one operator**; no
  `TenantId`. (A future paid Fleet/multi-tenant tier can re-add it.)
- **Model routing** (`ModelRouter`). Out of scope — that's a cost
  *optimisation*, a later concern; K is about *visibility + caps*.
- **A separate ledger store.** The audit chain already holds usage; we add a
  priced aggregation over it, not a second write path. (If per-call query
  performance ever demands an index, a `KeyDomain::CostLedger` is the
  escape hatch — deferred until measured.)

## 3. The model

```rust
// aivyx-cost (new free-core crate)

/// Token counts for one priced unit (a turn, or a whole run).
struct TokenCounts { input, output, cache_read, cache_write: u64 }

/// $ / million-tokens for a model, by token class.
struct ModelRate { input, output, cache_read, cache_write: f64 /* USD per Mtok */ }

/// The outcome of pricing a usage record.
struct Cost { usd: f64, priced: bool }   // priced=false ⇒ no rate known (untracked, $0)

/// A model→rate table: shipped defaults + operator overrides.
struct Pricing { /* HashMap<model, ModelRate> */ }
impl Pricing {
    fn with_defaults() -> Self;                 // common cloud models + $0 local families
    fn cost_of(&self, model: &str, t: &TokenCounts) -> Cost;
    fn set_rate(&mut self, model, rate);        // from [pricing.<model>] config
}
```

- **Local models are free.** Ollama/llama/qwen/gemma/… price at **$0**
  (`priced = true`) — local inference costs no API dollars.
- **Unknown cloud models** price at `$0` with **`priced = false`** — a flag
  the report surfaces so the operator adds a rate rather than silently
  under-counting.
- **Defaults are overridable.** Shipped rates are a convenience, clearly
  marked; `[pricing.<model>]` in `aivyx.toml` is authoritative.

## 4. Budgets

```rust
enum BudgetAction { Alert, Deny }      // archive's Pause → Deny
struct BudgetConfig {
    per_run_usd:  Option<f64>,         // one mission / loop run / session
    per_day_usd:  Option<f64>,         // rolling calendar day
    on_exceeded:  BudgetAction,
    alert_at:     Option<f64>,         // fraction, e.g. 0.8 → warn at 80%
}
struct BudgetEnforcer { /* ledger view + config */ }
impl BudgetEnforcer {
    fn check(&self, spent_today, spent_this_run) -> BudgetVerdict;  // Ok | Alert | Deny
}
```

- **Deny is a pre-call gate**: checked *before* an LLM call so the cap is a
  ceiling, not a post-hoc notice. (The loop's existing post-iteration token
  stop stays; the $ pre-call gate is additive.)
- **Concurrency (teams).** A team runs specialists **concurrently**, so two
  in-flight calls can both pass a naive check and jointly bust the cap. K.3
  carries the archive's **reservation** idea (reserve an estimated cost
  under a lock; commit actual on completion, release on failure) to close
  that TOCTOU — the one piece of archive machinery the new concurrency
  genuinely needs.

## 5. Phase plan

*Test bands priced by dense components, not family label (the recurring
Chapter-J lesson: these came in ~40–60% under).* 

| Phase | Goal | Tests |
|---|---|---|
| **K.1 Pricing** | the `aivyx-cost` crate; `TokenCounts` / `ModelRate` / `Cost` / `Pricing` (defaults + `cost_of` + overrides); local-free + unknown-flagged semantics. Pure, no storage. | ~15–20 |
| **K.2 Priced ledger over the chain** | a `CostReport` that scans `TurnEnded` usage, prices it, and aggregates (per-day / per-session / total, priced vs untracked). Decide + (if taken) add `model` to `TurnEnded` for per-turn precision. | ~15–25 |
| **K.3 BudgetEnforcer** | `BudgetConfig` + `check` (Alert/Deny) + reservations (team concurrency). Pure logic over a ledger view. | ~15–25 |
| **K.4 Wiring** | record/price each turn; **pre-call $ gate** in the turn loop; generalise the autonomous-loop budget to $; bound team missions. `aivyx cost` report CLI. | ~15–25 |
| **K.5 Config** | `[budget]` + `[pricing.<model>]` in `aivyx.toml` (aivyx-config), threaded through the daemon. | ~10–15 |

```
K.1 ─▶ K.2 ─▶ K.3 ─▶ K.4 ─▶ K.5
```

## 6. Locked decisions

1. **Crate** — a new free-core **`aivyx-cost`** (governance/observability,
   not "billing").
2. **Single-operator** — no multi-tenant; the chain is per-daemon.
3. **Chain is the token source of truth** — price a view over `TurnEnded`
   usage; no second ledger store unless query cost forces one.
4. **Local = $0, unknown cloud = flagged** — never silently under-count.
5. **Deny is a pre-call gate; reservations cover team concurrency.**
