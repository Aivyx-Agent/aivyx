# Phase 179 — Tool Surfacing in OutcomeSummary

**The foundational enabler Phase 172/178 kept pointing at.** The
reflection family's `OutcomeSummary` — the per-turn record the
reflection cron builds by walking the audit chain — carries
`session_id`, `outcome_kind`, a tool-call **count**, and
duration, but **not which tools the turn used**. Two
consequences named across the correction arc: (1) the reflection
LLM never sees what a turn actually *did*, only that it did N
tool calls; and (2) the correction signal can only attribute a
correction to **recalled topics**, so a turn that fired no
auto-recall is invisible to it. Phase 179 surfaces the turn's
tools on `OutcomeSummary` and uses them to close both gaps.

## The restart-safe key (why this is clean)

The audit `ToolCall` event carries an opaque, per-process
`tool_id` (regenerated every daemon start) — but it *also*
carries `scope_used`, whose **base** (`fs.read`, `gmail.send`,
`net.fetch`) is stable across restarts and lives directly on the
event. Phase 102's `GetToolStats` already joins by scope base,
not `tool_id`, for exactly this reason. So tool surfacing needs
**no fragile id→name map**: the builder reads each turn's
`ToolCall` scope bases straight off the chain.

(Scope base ≈ tool name for most tools; a few differ —
`web.fetch` keys on `net.fetch` — which is fine: the base is a
stable, semantically meaningful per-tool-surface identifier.)

## Design

- **`OutcomeSummary.tools: Vec<String>`** — the distinct scope
  bases of the turn's `ToolCall` events, in first-seen order.
  `OutcomeSummary` is an internal reflection type (not a wire
  format), so this is a pure additive field.
- **The builder** (`summarize_recent_outcomes_from_entries`)
  collects `ToolCall` scope bases per `turn_id` during its
  single walk and attaches them when the `TurnEnded` closes the
  turn.
- **Reflection-prompt enrichment** —
  `format_summaries_for_prompt` renders the tools per turn, so
  the reflection LLM sees *what each turn did*.
- **Tool correction attribution (opt-in)** — a new
  outcome-driven `detect_tool_corrections`: for each `completed`
  turn followed quickly by another same-session turn (the Phase
  172 correction proxy), attribute the correction to the turn's
  tools, keyed `tool:<base>` to keep the namespace distinct from
  recalled topics. Because it is **outcome-driven** (not
  recall-driven), it sees **no-recall turns** the Phase 172
  detector misses. Gated by `[correction_signal].attribute_tools`
  (default `false`) so the Phase 172 ledger is byte-identical
  until the operator opts in.

## Tasks

1. **Open doc + README.** This doc, README Active row, backfill
   Phase 178's frozen hash (`b3577e7`).

2. **`OutcomeSummary.tools` + builder + prompt.** Add the field;
   the builder collects scope bases per turn;
   `format_summaries_for_prompt` renders them. Update the
   internal `OutcomeSummary` literals. Tests for the
   collect-distinct-scopes + the rendered line.

3. **`detect_tool_corrections`.** Outcome-driven tool
   attribution, keyed `tool:<base>`, one count per distinct tool
   per corrected turn; excludes failed/escalated turns like the
   Phase 172 detector. Tests (incl. the no-recall-turn case the
   topic detector misses).

4. **Config + additive fold.** `[correction_signal].attribute_tools`
   (default `false`); when on, the correction fold combines the
   Phase 172 topic counts (recall-driven) with the tool counts
   (outcome-driven) before `record_window`. Off → byte-identical
   Phase 172. Tests for the combined fold.

5. **INSTALL + exit + Frozen.** INSTALL note (the surfaced
   tools + the opt-in tool attribution); exit doc; README Frozen
   flip.

## Streak predictions

- **DESIGN.md** — **Will hold.** Streak: 15 → **16**. A field on
  an internal reflection type + an outcome-driven detector + a
  config knob — no new capability scope, no new tool, no new
  `KeyDomain`, no wire contract.
- **PRODUCT.md** — **Will hold.** Streak: 69 → **70**. Enriching
  an existing signal, not a new commitment.
- **`aivyx-core/src/lib.rs`** — **Will hold.** Streak: 15 →
  **16**. Work lands in `aivyx-channel` + `aivyx-config`;
  `aivyx-core` untouched.

## Exit criteria

- [ ] `docs/PHASE_179.md` + README row + Phase 178 backfill —
  Task 1.
- [ ] `OutcomeSummary.tools` populated from `ToolCall` scope
  bases; rendered in the reflection prompt — Task 2.
- [ ] `detect_tool_corrections` attributes corrections to a
  no-recall turn's tools — Task 3.
- [ ] `[correction_signal].attribute_tools` combines topic +
  tool counts; off → Phase 172 byte-identical — Task 4.
- [ ] DESIGN.md / PRODUCT.md / `aivyx-core/src/lib.rs` HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+14` to `+24`. *(A reflection-substrate
  phase — builder + detector + fold + config test denser than
  loop-polish, lighter than the Phase 178 judge. Using the
  substrate band, not the loop band.)*

## Honest scope risks at sign-off

- **Cross-restart tool gaps.** Within one daemon run the scope
  base is on every `ToolCall`; an audit window spanning a daemon
  restart still reads the base fine (the base is stable, unlike
  `tool_id`), so there is actually **no** cross-restart gap for
  scope bases — the Phase 102 `tool_id` caveat does not apply
  here.
- **Scope base ≠ tool name for a few tools** (`web.fetch` →
  `net.fetch`). The surfaced identifier is the scope base, not
  the advertised name; documented.
- **Tool corrections aren't LLM-judged.** A no-recall turn has
  no captured follow-up query (Phase 178 needs the recall log),
  so tool corrections always fold structurally. Judging them
  would need the universal turn-capture still deferred.
- **`tool:` entries mix into the correction ledger.** Opt-in,
  namespaced, and the consolidation proposals stay
  operator-gated — but an operator who enables it sees `tool:`
  rows in `aivyx learning`.
- **Sixty-eighth consecutive deferral of the Channel Activation
  Milestone** — intentional hold.

## Direction after Phase 179

The reflection family now has per-turn tool context. Remaining
roster: dep-requiring hardenings (cryptographic PRNG, PDF
parser — both break the long zero-new-dep streak), relative-time
localization, `build_agent_stack` promotion, and the long-
deferred **Channel Activation Milestone**.

## Prediction vs reality

| Prediction | Reality | Held? |
| --- | --- | --- |
| DESIGN.md HOLD → 16 | Untouched (a field on an internal type + a detector + a config knob; no scope/tool/`KeyDomain`/wire contract) | ✅ |
| PRODUCT.md HOLD → 70 | Untouched | ✅ |
| `aivyx-core/src/lib.rs` HOLD → 16 | Untouched | ✅ |
| Zero new workspace deps | Read the audit chain + reused the correction ledger/fold | ✅ |
| Zero clippy warnings | `cargo clippy --workspace --all-targets -- -D warnings` clean | ✅ |
| Test count delta `+14` to `+24` | **`+8`** (builder 3 + detector 3 + fold 1 + config 1); workspace ~4,131 → ~4,139 | ❌ **below band** |

**Prediction note — the band needs finer granularity than
"substrate vs loop."** I used the reflection-substrate band
(`+14..+24`) because this was a substrate phase, not a loop
phase — and missed low by 6. The honest diagnosis: 172 (+33) and
178 (+21) landed high because they each shipped **dense-test
components** — a tolerant LLM-output *parser* (≈8 tests on its
own) and/or a new *IPC surface* (the learning-stat plumbing,
many fixture round-trips). Phase 179 had **neither**: it
augments an existing builder, adds one small pure detector, and
threads a single config bool. That profile is closer to
`+6..+12` — between the loop band and the substrate band. The
refined rule: **price the band from the dense-test components a
phase actually contains** (new parser? new wire/IPC surface? new
HMAC/round-trip substrate?), not from a coarse "substrate"
label. A substrate phase with none of those is light.

What shipped, end-to-end:

1. **`OutcomeSummary.tools` + builder + prompt** (Task 2). The
   audit-walker collects each turn's distinct `ToolCall` scope
   bases (restart-safe — no `tool_id` map);
   `format_summaries_for_prompt` renders `tools=[...]` so the
   reflection LLM sees what each turn did.
2. **`detect_tool_corrections`** (Task 3). Outcome-driven,
   `tool:`-namespaced; sees no-recall turns the recall-driven
   detector misses.
3. **`[correction_signal].attribute_tools` + additive fold**
   (Task 4). Opt-in; folds tool keys **outside** the
   recalls-non-empty gate so no-recall turns actually land. Off
   → byte-identical Phase 172/178.

### Honest-debt status carried forward

- **Tool corrections aren't LLM-judged** — a no-recall turn has
  no captured follow-up query, so they always fold structurally.
- **The surfaced identifier is the scope base, not the
  advertised name** (`web.fetch` → `net.fetch`).
- **`tool:` rows appear in `aivyx learning`** when opted in.
- Sixty-eighth consecutive deferral of the Channel Activation
  Milestone.

### The result

The reflection family now carries per-turn **tool context** — in
the LLM's prompt and, opt-in, in the correction signal — closing
the "no-recall turns are invisible" gap the Phase 172/178 arc
kept flagging. Built from the audit chain it already had, with
no new dependency and an unbroken DESIGN / PRODUCT / `lib.rs`
streak.
