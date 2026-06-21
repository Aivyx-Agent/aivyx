# A Utilities Pack — calc / convert / date (Chapter Abacus)

> **Status:** 🟡 **PLANNED (AB.0–AB.5).** The second **new-tools breadth** chapter
> after [Chapter Forge](FORGE.md), and the next item off the [Atlas §6
> backlog](ATLAS.md): a small pack of **pure-compute utility tools** — expression
> evaluation, unit conversion, timezone conversion, and date arithmetic — that the
> LLM keeps reaching for and currently fakes (and gets wrong) in its head. They land
> as a new tool group inside the **existing `aivyx-toolkit` binary** (the Chapter G
> multi-tool tool-process that already bundles `web.search` + `task.*` + `health.*`
> + `budget.*`), so the chapter adds **no new crate, no new binary, no new
> tool-process config, and no OAuth/API keys**. Unlike every other toolkit surface,
> these tools are **side-effect-free and offline** — no network, no filesystem, no
> state store — which is the chapter's one genuinely new governance note (see §2).
> Opt-in by virtue of the tool-process being configured; default behavior of an
> existing operator is unchanged (new tools simply appear in the catalog).

## 1. Why this chapter

Forge widened the *substrate* (a page-reader and a committer); Abacus widens the
*toolkit* with the deterministic helpers a language model is structurally bad at.
An LLM asked "what's 18.5% of $2,140, and what's that in EUR-per-month over 14
weeks" will confidently emit arithmetic it didn't actually compute; asked "convert
this 2026-06-21T14:00 America/New_York meeting to my Berlin colleague's time" it
guesses an offset. These are exactly the cases where a **tiny, exact, audited tool
call** beats a token-predicted answer. The Atlas §6 backlog named four:

- **`calc.eval`** — evaluate an arithmetic expression (`+ - * / ^`, parentheses,
  a curated set of functions) and return the exact numeric result.
- **`convert.units`** — convert a value between units (length, mass, temperature,
  volume, digital storage) via a curated conversion table.
- **`convert.time`** — convert a timestamp between named IANA timezones.
- **`date.diff` / `date.add`** — date arithmetic beyond the existing `time.now`
  (duration between two dates; add/subtract a duration).

All four are **deterministic pure functions**: same input → same output, no I/O,
no clock dependence except where a date defaults to "now". That property is what
makes this the *cheapest* and *safest* §6 cut, and it drives the governance below.

## 2. Architecture & governance decisions (locked)

### Lives in the existing `aivyx-toolkit` binary — no new crate/binary/config
These tools register alongside the current 15 in `aivyx-toolkit/src/main.rs`'s
`Vec<Arc<dyn Tool>>`, served by the shared `run_multi_tool_subprocess` harness
(`aivyx_tool::multi_harness`). Same `Tool` trait, same IPC bridge, same
`[[tool_process]]` the operator already has. No second process to spawn, no new
`config.toml` section (these tools take **no** keys or operator config), no new
state store (nothing is persisted). This is purely additive surface on a binary
that already exists.

### Side-effect-free + offline — the one new governance note: a **lower tier**
Every existing toolkit base (`web.search`, `task.*`, `health.*`, `budget.*`) is
**Trusted-only by default**, gated like `email.*` because it touches the network or
the operator's personal data and must not be reachable from a remote channel
without an explicit role grant. **The Abacus tools touch neither.** A calculator, a
unit table, and date math leak nothing, mutate nothing, and call nothing — there is
no SSRF surface, no exfiltration path, no destructive action. So the locked
decision is to gate them at a **lower ceiling — `SemiTrusted`** (the tier that
`web.fetch` sits at), so even a semi-trusted remote context can do arithmetic
without a Trusted grant. This is the first toolkit surface that is *not*
Trusted-pinned, and the §2 reasoning (no side effects, no data) is exactly why.
*(Open question OQ-2 weighs `Untrusted` instead.)*

### New capability bases — but **not** a P10 substrate amendment
Each scope a tool requires must exist in `aivyx_capability::KNOWN_BASES` (the audit
chain rejects an unknown base). So Abacus grows `KNOWN_BASES` — capability-taxonomy
growth, the **A3-style addition** that every Chapter F/G tool-process surface made
(`web.search`, `calendar.read`, `drive.read`, …). Crucially this is **not** a
PRODUCT.md **P10 substrate-count amendment** (the capped, amendment-gated core that
Forge's `git.write` touched): these bases live in the **uncapped tool-process
tier**, exactly where `web.search`/`task.*`/`budget.*` already sit. Each new base is
also assigned a tier ceiling (added to `CEILING_TRUSTED`/`CEILING_KERNEL` per the
existing pattern, but reachable from `SemiTrusted` per the decision above).

### Scope shape: **one base per tool group**, no read/write split
The existing toolkit splits `task.read`/`task.write` because those tools *mutate*
state. Abacus tools mutate nothing, so a read/write split is meaningless. The locked
shape is **one base per group**: `calc.eval`, `convert.units` (covering both
`convert.units` and `convert.time`), and `date.compute` (covering `date.diff` +
`date.add`). *(Open question OQ-1 weighs collapsing all of it into a single
`util.compute` base to minimize taxonomy growth.)*

### Dependencies kept minimal, license-clean
- **`calc.eval`** — prefer a **hand-rolled shunting-yard** evaluator (a few dozen
  lines for `+ - * / ^`, parens, unary minus, and a small function whitelist) to
  add **zero** dependencies; fall back to a vetted MIT/Apache crate (`fasteval`)
  only if the in-phase scope grows. Resolved in AB.1 against `cargo deny`.
- **`convert.units`** — a **hand-rolled curated table** (no dep); units are a fixed,
  auditable set, and a table is clearer than a general dimensional-analysis crate.
- **`convert.time` / `date.*`** — **`chrono`** is already a toolkit dependency;
  named-timezone support needs **`chrono-tz`** (MIT/Apache-2.0). One new, permissive,
  widely-vetted dep — the only addition the chapter is likely to make.

## 3. Scope

**In:** the five pure-compute tools (`calc.eval`, `convert.units`, `convert.time`,
`date.diff`, `date.add`); their `KNOWN_BASES` bases + tier-ceiling wiring at the
`SemiTrusted` gate; registration in `aivyx-toolkit/src/main.rs`; their unit tests;
the `docs/TOOLS.md` catalog entries (kept honest by Atlas's
`tools_catalog_documents_every_known_base` drift-guard); a live-verify turn.
**Out:** the rest of the §6 backlog — **structured-data readers** (CSV/PDF/
spreadsheet, their own chapter) and **integrations** (GitHub/weather/Google Tasks,
each a keyed tool-process per the Chapter F pattern); any persistence (these tools
are stateless); any P10 substrate change; currency conversion (needs a live rates
feed → network → an integration, not a pure-compute util).

## 4. Phase plan (docs-first, small phases per convention)

| Phase | Deliverable | Notes |
|---|---|---|
| **AB.0** 🟡 | **This design contract** | Locked reference; banner flips per phase. |
| **AB.1** | **`calc.eval` + the governance spine** | First tool *and* the path everything reuses: add the `calc.eval` base to `KNOWN_BASES` + tier ceiling (SemiTrusted), implement the evaluator (dep decision resolved here vs `cargo deny`), register it in `main.rs`, prove end-to-end through the harness. Lands the base/ceiling/registration pattern once so AB.2–AB.3 are pure tool adds. |
| **AB.2** | **`convert.units` + `convert.time`** | The `convert.units` base; the curated unit table; `chrono-tz` added for named-timezone conversion (the one new dep, cleared by `cargo deny`). |
| **AB.3** | **`date.diff` + `date.add`** | The `date.compute` base; duration arithmetic over `chrono`; defaults-to-now behavior documented (the one non-pure edge). |
| **AB.4** | **Catalog + drift-guard** | `docs/TOOLS.md` entries for the new tools/bases; confirm `tools_catalog_documents_every_known_base` passes; extend `check_tool_quality` coverage to the new tools (the Atlas AT.3 guard, now over the toolkit tier per Atlas's own recommendation). |
| **AB.5** | **Finalize** | Live-verify a real agent turn calling each tool through the running daemon; full workspace suite + clippy + `cargo deny` green; `docs/ATLAS.md` §6 backlog updated (utilities ✅); the chapter memory recorded; status → COMPLETE. |

**Discipline:** AB.1 carries the governance spine (base + ceiling + registration)
so AB.2–AB.3 add tools against a proven path, not by re-deriving the wiring each
time. Test band: **moderate** — pure functions are dense but trivially testable
(boundary/units/parse-error cases dominate); price **~25–35 new tests**, heaviest in
AB.1 (the evaluator's parse/precedence/error matrix) and AB.2 (the conversion table).

## 5. Open questions (resolve in-phase)

- **OQ-1 — base granularity (AB.1).** One base per group (`calc.eval` /
  `convert.units` / `date.compute`, locked default) vs. a **single `util.compute`**
  base for the whole pack (minimal `KNOWN_BASES` growth, coarser gating). Decide in
  AB.1; lean per-group for catalog legibility unless the single base proves cleaner.
- **OQ-2 — tier floor (AB.1).** `SemiTrusted` (locked default — reachable like
  `web.fetch`) vs. **`Untrusted`** (a calculator is arguably safe for *any* context).
  Confirm against the trust-tier model; `SemiTrusted` is the conservative pick.
- **OQ-3 — evaluator dependency (AB.1).** Hand-rolled shunting-yard (zero deps,
  locked default) vs. `fasteval`/`meval` (more functions, a dep to vet). Resolve
  against `cargo deny` + the actual function set the agent needs.
- **OQ-4 — `calc.eval` function whitelist (AB.1).** Which functions beyond bare
  arithmetic (`sqrt`, `min`/`max`, `round`, `%`)? Keep it small and auditable; grow
  only on demand. No user-defined functions, no variables (that's a REPL, not a tool).
- **OQ-5 — unit coverage (AB.2).** Which unit families ship first (length / mass /
  temperature / volume / digital)? Curated and finite by design; add families on
  demand rather than chasing completeness.

---

*Chapter Abacus is the toolkit's pocket calculator: the small, exact, deterministic
helpers a language model is worst at doing in its head — arithmetic, conversions,
date math — handed over as audited tool calls instead of confident guesses. It costs
no new binary, no keys, and (almost) no dependencies, and it's the first toolkit
surface safe enough to reach below the Trusted tier, precisely because it touches
nothing and computes everything.*
