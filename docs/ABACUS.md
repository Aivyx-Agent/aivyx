# A Utilities Pack — calc / convert / date (Chapter Abacus)

> **Status:** ✅ **COMPLETE (AB.0–AB.5).** The second **new-tools breadth** chapter
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
| **AB.1** ✅ | **`calc.eval` + the governance spine** | DONE. `calc.eval` base added to `KNOWN_BASES` (88) + both ceilings — present in `CEILING_SEMITRUSTED` (the first toolkit base below Trusted) and re-listed in `CEILING_TRUSTED`; A3-addendum table + running count (87→88) recorded. Evaluator is **hand-rolled** recursive descent (OQ-3 resolved: zero new deps), supporting `+ - * / % ^`, parens, unary `±`, and `sqrt/abs/round/floor/ceil/min/max`; conventional precedence (`-2^2 == -4`, right-assoc `^`, `2^-3` works); non-finite results (÷0, `sqrt(-1)`) are tool errors. `CalcEval` registered in `main.rs` (16 tools) + `tools/mod.rs`; `docs/TOOLS.md` catalog entry added (passes the Atlas drift-guard). Scope/tier/granularity decisions locked: per-group base (OQ-1) + SemiTrusted floor (OQ-2). 22 unit tests (precedence/functions/error matrix + tool metadata); toolkit + capability suites green; clippy clean. |
| **AB.2** ✅ | **`convert.units` + `convert.time`** | DONE. One `convert.units` group base (per OQ-1) gates both tools, added to `KNOWN_BASES` (89) + both ceilings (SemiTrusted floor), A3 addendum + count (88→89) recorded. **`convert.units`** — hand-rolled curated table (zero deps) over length/mass/temperature(affine)/volume/digital families; cross-family + unknown-unit are errors. **`convert.time`** — naive-local-in-`from`-zone → `to`-zone via **`chrono-tz`** (the chapter's one new dep, MIT/Apache, `cargo deny` clean); DST gap → error, ambiguity → earliest. Both registered in `main.rs` (18 tools) + `tools/mod.rs`; `docs/TOOLS.md` entry added. 19 unit tests (each family + temperature + DST/UTC timezone cases + tool metadata); toolkit + capability suites green; clippy `-D warnings` clean. |
| **AB.3** ✅ | **`date.diff` + `date.add`** | DONE. One `date.compute` group base gates both tools, added to `KNOWN_BASES` (90) + both ceilings (SemiTrusted floor), A3 addendum + count (89→90) recorded. **`date.diff`** — signed span (`to - from`) as whole `days` + exact `seconds`; **`date.add`** — combine weeks/days/hours/minutes/seconds (negatives subtract) onto a base date. Shared `parse_datetime` accepts bare date (midnight UTC) / naive datetime (UTC) / RFC 3339 with offset; calendar-correct over `chrono` (no new dep). The one non-pure edge documented: a **missing date defaults to now** (`Utc::now()`) — the pack's only clock dependence. Both registered in `main.rs` (20 tools) + `tools/mod.rs`; `docs/TOOLS.md` entry added. 16 unit tests (parse shapes + leap-day/signed diff + duration combine/negative/validation + 90-day calendar correctness + tool metadata); toolkit + capability suites green; clippy `-D warnings` clean. |
| **AB.4** ✅ | **Catalog + drift-guard** | DONE. `docs/TOOLS.md` entries for all five tools / three bases were kept current across AB.1–AB.3, so the `tools_catalog_documents_every_known_base` drift-guard passes (confirmed). New `crates/aivyx-toolkit/tests/tool_quality.rs` extends the Atlas AT.3 `check_tool_quality` sweep to the **tool-process tier** (Atlas's own recommendation): a `tokio` test builds all 20 toolkit tools exactly as `main.rs` does (cheap scratch stores) and asserts every one meets the name/description/schema floor — zero issues. clippy `-D warnings` clean. |
| **AB.5** ✅ | **Finalize** | DONE. **Live-verified** by driving the real `target/release/aivyx-toolkit` binary over its actual length-prefixed-JSON IPC (the same `multi_harness` path the daemon uses): `ToolHello`→`ToolList` reported **20 tools** (all five Abacus tools registered), and a real `InvokeTool` per tool returned a correct `ToolResult` — `calc.eval (18.5/100)*2140`→`395.9`, `1 mi`→`1.609344 km`, `100 c`→`212 f`, `14:00 America/New_York`→`20:00+02:00 Europe/Berlin`, `2026-06-21`→`2026-12-25` = `187 days`, `2026-01-01 + 90d`→`2026-04-01`. (Short of the LLM *selecting* the tool — model behavior, not chapter code, the same mechanism-vs-model line Bridle/Emboss drew.) Full workspace suite + clippy `-D warnings` + `cargo deny` (advisories/bans/licenses/sources) green; `docs/ATLAS.md` §6 marked utilities ✅; chapter memory recorded; status → COMPLETE. |

**Discipline:** AB.1 carries the governance spine (base + ceiling + registration)
so AB.2–AB.3 add tools against a proven path, not by re-deriving the wiring each
time. Test band: **moderate** — pure functions are dense but trivially testable
(boundary/units/parse-error cases dominate); price **~25–35 new tests**, heaviest in
AB.1 (the evaluator's parse/precedence/error matrix) and AB.2 (the conversion table).

## 5. Open questions (resolve in-phase)

- **OQ-1 — base granularity (AB.1).** ✅ **Resolved: per-group base.** AB.1 shipped
  `calc.eval` as its own base; AB.2/AB.3 follow with `convert.units` / `date.compute`.
  Per-group won for catalog legibility over a single `util.compute` base.
- **OQ-2 — tier floor (AB.1).** ✅ **Resolved: `SemiTrusted`.** `calc.eval` is in
  `CEILING_SEMITRUSTED` — reachable like `web.fetch`, the conservative pick over
  `Untrusted` (revisit if a use case wants math from a fully-untrusted context).
- **OQ-3 — evaluator dependency (AB.1).** ✅ **Resolved: hand-rolled, zero deps.**
  A recursive-descent evaluator over a hand-written tokenizer; the function set is
  small enough that a crate would be more surface than it saves.
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
