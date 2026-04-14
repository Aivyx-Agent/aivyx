# Phase 10 — Tool-layer refinement + memory substrate

**Status:** Active (opened 2026-04-14). This document will churn
during the phase and freeze at exit under a final Exit criteria
block at the bottom, matching the Phase 7–9 precedent.
**Predecessor:** [PHASE_9.md](PHASE_9.md) (exit commit `7052ecc`,
hash backfill `1853673`)
**Contract:** [`../DESIGN.md`](../DESIGN.md) (Deliverables 1–8, all
LOCKED — unchanged since `e0d6437`, **nine phases running** at Phase
10 entry, target **ten** at Phase 10 exit)

## Goal

Phase 10 is the **last pure-foundation phase** before the codebase
pivots from foundation work to product work in Phase 11. Its job
is to close three rolling deferrals that have been sitting in the
phase-journal backlog since Phases 6–7, so the Phase 11+ product
sequence inherits a clean slate:

1. **Cross-topic `memory.read`** — the Phase 6 Q3 deferral, rolled
   forward through Phases 6→7→8→9 on the grounds that "no concrete
   use case has asked for it." The calculus changes in Phase 10
   because the *next* phase (Phase 11 role system) will immediately
   want it for role-scoped memory introspection, and paying it down
   now means Phase 11 doesn't have to design a substrate primitive
   under product pressure.
2. **Runtime JSON-schema validation for tool input** — deferred
   since Phase 7. Every Phase 11+ tool (`shell.exec`, `edit.patch`,
   `web.fetch`) will want schema-validated input on day one. Doing
   it now as a default-`None` trait refinement means new tools
   inherit validation for free, rather than requiring a retrofit
   sweep across three shipped tools.
3. **Tool name in `StreamEvent::ToolCallStarted`** — deferred since
   Phase 7. A small DX gap: the renderer and audit bridge can see
   "a tool started" but not *which* tool. 1–2 hour fix that has
   been sitting because no phase had it as a theme.

The headline outcome: when Phase 10 closes, the deferred-items
rolling history in each phase journal should be **empty of pure-
foundation items**. Everything still on the list after Phase 10
(structured identity / Q7, third channel adapter, real-bot smoke
test) is deferred because it needs *product pressure* to resolve,
not because it is polish waiting for a free afternoon.

## Why now

Three structural reasons:

1. **Foundation maturity.** Phase 9 exited with the DESIGN.md
   streak at nine, the production-core byte-identity streak at
   seven (since `c3883be`), and 326 green tests. The foundation
   is as stable as it has ever been. This is the right moment
   to pay down small debts because the debts are small and the
   foundation is not moving under them.
2. **Product pivot is next.** The previous conversation turn
   established that Phase 11 will open the product sequence with
   a Role system and `shell.exec`. Every item in Phase 10's scope
   is a dependency of that phase — cross-topic read unblocks
   role-scoped memory, schema validation hardens the first
   user-visible dangerous tool (`shell.exec`), and the streaming
   tool name is what makes "my coder ran `shell.exec`" legible in
   the UI. Phase 10 is not polish-for-polish's-sake; it is
   *preparing the runway* for Phase 11.
3. **Intentional streak break is safer here than later.** Tasks 2
   and 3 touch `crates/aivyx-core/src/lib.rs` (the `Tool` trait
   and `StreamEvent` enum), which will break the production-core
   byte-identity streak held since Phase 8 Task 2's `c3883be`.
   That streak has value as *evidence of discipline*, but it is
   not a constraint the contract requires — D3 explicitly allows
   additive trait refinement. Breaking the streak deliberately,
   in a phase whose journal records the reason, preserves the
   evidence's value ("the team chose to break it and said why")
   while an unnoticed break would destroy it ("the team no longer
   noticed when core changed"). Doing this in a foundation phase
   with no product pressure is the cheapest possible time to pay
   the cost.

## Non-goals

- **No new channel adapter.** Phase 9's Q1 Fork B decision to
  refuse a third adapter holds. `ADAPTER_PATTERN.md` stays marked
  tentative until a real third adapter validates it in a later
  phase — probably Phase 13's `aivyx-google` service adapter,
  which is a different kind of third data point.
- **No `shell.exec`, no new external-world tools.** Those belong
  in Phase 11 where their design pressure is real. Phase 10 only
  touches the `Tool` trait surface, not the tool catalog.
- **No Q7 structured-identity work.** Still correctly deferred
  to the first adapter that needs it.
- **No refactor of `aivyx-core/src/agent.rs` for aesthetics.**
  The 1040-line turn loop is fine. Touching it for style is
  exactly the kind of drift the phase discipline exists to
  prevent. Task 2's validator insertion is the only planned
  `agent.rs` edit, and it is a surgical addition at the point
  where `required_scope` runs.
- **No Phase 11 work, even speculatively.** No `Role` type, no
  `shell.exec`, no `aivyx-config` changes beyond what Task 1
  might need. If a Phase 10 task is tempted to "prepare the way"
  for a Phase 11 feature, the temptation is the wrong answer —
  the Phase 11 journal will design its own types, and Phase 10
  exposing premature scaffolding for them is drift.

## Entry criteria (all met from Phase 9 exit)

- [x] Phase 9 is frozen. Exit commit `7052ecc`, hash backfill
      `1853673`. `docs/README.md` phase-status table reflects
      both.
- [x] `cargo test --workspace` is 326 green.
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
      is clean.
- [x] DESIGN.md byte-identical to `e0d6437`. Streak at **nine**.
- [x] `crates/aivyx-core/` production code byte-identical to
      `c3883be` (the Phase 8 Task 2 `session_partition` additive
      refinement). Streak at **seven phases** and expected to
      break in Phase 10 Task 2 — the break is an intentional
      phase-scope decision, not a regression.
- [x] Pre-commit hook (`scripts/pre-commit.sh`, installed via
      `scripts/install-hooks.sh`) runs `cargo clippy` workspace-
      wide with `-D warnings` before every commit. The hook
      caught zero regressions across Phase 9 Tasks 4 and 5,
      evidence the Q4 Level 1 decision was correct.

## Draft task breakdown

Four tasks. Ordered so the two riskiest pieces (Tasks 1 and 2)
land first, with Task 3 as a small DX payoff and Task 4 as the
exit freeze. Each task gets a working-session commit and closes
before the next one opens — the Phase 7–9 cadence.

### Task 1 — Cross-topic `memory.read` substrate + tool wiring

**What lands:**

- `RedbMemory::scan_topics()` primitive in `aivyx-memory/src/
  redb.rs`. Walks `KeyDomain::Memory` by key-prefix (the
  `\x01s\x01<session>\x01` prefix from the Phase 8 Task 2
  layout), collecting `(topic, entries)` pairs for every topic
  present in the given session. Read-only, no mutation path.
- New `Scope` shape: `memory.read:topic:*:session:<session>`.
  The `*` topic segment is a wildcard distinct from a literal
  topic named `"*"`. Added to the `aivyx-capability` active-
  scope list — scope count goes from 21 to 22. **Explicitly
  not** in `TrustTier::Trusted`'s default ceiling or any other
  tier's default ceiling. An agent that wants cross-topic read
  must be granted the wildcard scope by name.
- `MemoryReadTool` in `aivyx-memory/src/tools.rs` gains an
  optional `"topics": "*"` input variant alongside the existing
  `"topic": "<name>"` variant. When the wildcard variant is
  selected, the tool's `required_scope` emits the wildcard
  scope shape. Mutually exclusive with the single-topic
  variant.

**Discipline guardrails:**

- `aivyx-core` production code stays byte-identical in Task 1.
  All the work is in `aivyx-memory` and `aivyx-capability`.
- The wildcard scope must pass through `Scope::attenuate()`
  correctly — an agent granted `memory.read:topic:*:session:A`
  must not see session B's memory. This is the critical safety
  test and gets three dedicated unit tests.
- Scripted unit tests covering: (a) wildcard fan-out across
  two topics in one session returns both, (b) per-session
  isolation holds, (c) attenuation denies wildcard when not
  granted, (d) a regression test pinning that the literal
  string `"*"` as a topic name in the single-topic variant
  does **not** trigger wildcard semantics.

**Expected test delta:** +4 tests (326 → 330).

**Stressed question:** Do we use a distinct `Scope` type variant
for the wildcard, or a textual sentinel inside the existing
topic segment? Leaning textual sentinel — matches how the session
segment already works, keeps the scope grammar stable. Resolve
by reading `aivyx-capability/src/lib.rs:86` `Scope::new()` and
picking whichever shape costs fewer lines.

### Task 2 — Runtime JSON-schema validation for tool input (hand-rolled)

**What lands:**

- New method on the `Tool` trait at `crates/aivyx-core/src/lib.rs:
  473`: `fn input_schema(&self) -> Option<serde_json::Value> {
  None }`. Default `None` means "no validation" — the trait is
  backwards-compatible and every shipped tool picks up the
  default for free.
- Hand-rolled validator in `aivyx-core` (likely a new
  `src/schema.rs` module under 100 lines) covering the three
  shapes tools actually use: (1) object-with-typed-fields
  (e.g., `{path: string, content: string}`), (2) required-field
  presence check, (3) enum-constrained strings (for tools that
  take an operation kind). No external dependency — the
  validator is intentionally narrow to the JSON shapes in-tree
  tools emit, and any richer schema constructs get added when
  a future tool needs them.
- Turn loop at `crates/aivyx-core/src/agent.rs` calls
  `tool.input_schema()` and runs the validator **before**
  `required_scope` runs. On validation failure, emit
  `ToolOutcome::Failed { reason: "schema mismatch: <detail>" }`
  — the planner already knows how to observe and recover from
  `Failed` outcomes, so no planner changes.

**Discipline guardrails:**

- **Intentional break** of the production-core byte-identity
  streak held since `c3883be`. Record in the ship log with the
  reason ("first additive Tool-trait refinement, within D3's
  contract"). Future streak accounting reports the new baseline.
- Every existing tool (`FsReadTool`, `FsWriteTool`,
  `MemoryReadTool`, `MemoryWriteTool`, `MemoryForgetTool`) gets
  a real `input_schema()` override in this task — not default
  `None`. The point is that schema validation is live on every
  shipped tool at Phase 10 exit, not just on whatever Phase 11
  ships next. Retrofitting the five shipped tools is cheaper
  than leaving them unvalidated and paying the retrofit cost
  across a future refactor.
- **Zero new dependencies.** The hand-rolled validator is
  under 100 lines and handles exactly the JSON shapes tools
  use. The zero-new-dep streak broke at Phase 9 Task 3 (toml),
  but holding the line on subsequent phases is still
  meaningful.

**Expected test delta:** +6–8 tests (four positive shapes
covered, two negative shapes, one round-trip test pinning that
a shipped tool's schema matches its actual input type).

**Stressed question:** Where does the validator live — a new
`aivyx-core/src/schema.rs` module, or inline in `agent.rs`
alongside the call site? Leaning new module; it keeps `agent.rs`
focused on the turn loop and makes the validator unit-testable
in isolation.

### Task 3 — Tool name in `StreamEvent::ToolCallStarted`

**What lands:**

- `StreamEvent::ToolCallStarted` at `crates/aivyx-core/src/lib.rs:
  205` gains a `tool: &'a str` field (plus any existing fields).
  The lifetime already exists on the enum, so no new generics.
- The emission site in `llm_planner.rs` (and anywhere else
  that emits `ToolCallStarted`) passes the tool name — the
  planner already knows it.
- Consumers updated: local REPL renderer in `aivyx-channel/src/
  render.rs`, Telegram session renderer in `aivyx-telegram/src/
  session.rs`, audit bridge in `aivyx-audit`. Each consumer's
  pre-Task-3 rendering is preserved for other `StreamEvent`
  variants — only `ToolCallStarted` gains the new field.
- Every test site that pattern-matches on `ToolCallStarted`
  gets updated. Expected count based on Phase 8/9 test density:
  5–10 sites across `aivyx-core`, `aivyx-telegram`, and
  `aivyx-audit`.

**Discipline guardrails:**

- Task 3 is the *second* production-core byte-identity touch
  of Phase 10, but it is **not** a second streak break — Tasks
  2 and 3 are bundled in the ship log as "the one intentional
  Phase 10 streak break across `aivyx-core/src/lib.rs`." The
  streak re-accounts to a new baseline at Phase 10 exit (the
  exit commit's SHA).
- The field is a `&'a str` borrowed from the tool registry,
  not an owned `String`. Phase 0 D3 picked `&'a str` lifetimes
  on `StreamEvent` deliberately to avoid allocating in the
  hot streaming path. Task 3 preserves that discipline.

**Expected test delta:** +0 new tests. Existing tests that
match on `ToolCallStarted` gain the new field in their
assertions; no new tests are needed because the feature is
already covered by existing tests that will now pin the field
value.

**Stressed question:** Should the tool name on the event match
the tool's `name()` exactly, or the scope root (`fs.read` vs
`fs`)? Leaning exact `name()` — matches what the audit chain
records, and renderer output should match audit output for
forensic correlation.

### Task 4 — Phase 10 exit freeze

**What lands:**

- `PHASE_10.md` frozen with per-task ship records. Status
  header flips from Active to Frozen with exit date. Each
  Task 1–3 ship record includes: what landed, subtle
  implementation notes, test delta, any unexpected findings.
- Streak accounting block, honest about the intentional break.
  The production-core byte-identity streak re-baselines to
  Phase 10's exit commit. The DESIGN.md streak rolls to **ten**
  phases. The zero-new-dep streak either holds (if Task 2
  stayed hand-rolled) or breaks with the reason recorded.
- `docs/README.md` phase-status table: Phase 10 row flipped
  from Active to Frozen with exit commit hash. Phase 11 row
  added pointing at ROADMAP.md as Planned.
- `docs/ROADMAP.md`: Phase 10 entry removed (now frozen in
  PHASE_10.md). Phase 11 entry refined with whatever Phase 10
  taught us about the Tool-trait surface — if schema
  validation surfaced a design question, Phase 11's first task
  inherits it.
- `README.md` (top-level): phase count rolls from 9 → 10,
  feature bullets gain a "schema-validated tool input" line.
- Exit criteria verified: clippy clean, tests green, DESIGN.md
  byte-identical to `e0d6437`.

**Exit commit shape:** single `docs(phase-10): freeze Phase 10,
update README + ROADMAP` commit covering all four files, then a
follow-up `docs(phase-10): backfill Phase 10 exit commit hash in
docs/README.md table` to fill in the self-referential hash. Same
pattern as Phase 9 exit (`7052ecc` + `1853673`).

## Open questions

Numbered so resolutions can be cited in ship records. Each question
picks up as much of the context as a fresh-session reader would
need to answer it.

### Q1 — Does the wildcard scope use a textual sentinel or a new enum variant?

Task 1 needs a scope shape meaning "any topic within this session."
Two options:

- **Q1 Option A — Textual sentinel `*` inside the existing topic
  segment.** Scope string is `memory.read:topic:*:session:<session>`.
  Matches how `session:*` would work if we ever needed it. Cheapest
  change to the capability grammar. Risk: a topic literally named
  `"*"` would trip the wildcard path — but Phase 6 already
  restricts topic names to `[a-zA-Z0-9_]+` or similar, so `*` is
  not a legal literal topic name and this risk is syntactic-level
  only. Needs a regression test pinning that `*` is not a legal
  literal topic name.

- **Q1 Option B — New `Scope::MemoryReadWildcard` variant or
  equivalent.** Type-system-enforced distinction between wildcard
  and literal. Cleaner but touches `aivyx-capability` more deeply.
  Would likely require refactoring `Scope::attenuate()` to handle
  the variant, and the prefix-attenuation semantics would need
  explicit thought.

**Leaning Option A.** Resolve at Task 1 kickoff by reading
`aivyx-capability/src/lib.rs` around `Scope::new` and picking
whichever shape costs fewer lines. Record the resolution in the
Task 1 ship record.

### Q2 — Does the hand-rolled validator handle nested objects?

Task 2's validator needs to cover every shape a shipped tool's
input JSON takes. Current tool inputs (surveyed quickly):

- `fs.read`: `{path: string}` — flat object
- `fs.write`: `{path: string, content: string}` — flat object
- `memory.read`: `{topic: string}` or (after Task 1)
  `{topics: "*"}` — flat object with enum-constrained variant
- `memory.write`: `{topic: string, content: string}` — flat object
- `memory.forget`: `{topic: string}` — flat object

All flat. **Validator does not need nested-object support in
Phase 10** — a future tool that needs nesting can extend the
validator when it lands. Record this as "validator is narrow by
design" in the Task 2 ship log.

### Q3 — Does Task 2 ship an `input_schema()` override for every shipped tool, or leave them at default `None`?

Both options preserve backwards compatibility. The difference:

- **Q3 Option A — Every shipped tool gets a real schema.** More
  work in Task 2 (five tools to retrofit). Payoff: at Phase 10
  exit, every tool in the workspace is schema-validated, and
  Phase 11's `shell.exec` inherits a "every tool has a schema"
  norm rather than a "some tools have schemas" norm.

- **Q3 Option B — Only new tools ship with schemas; shipped
  tools stay at default `None`.** Less work in Task 2. Risk: the
  "some tools have schemas" world creates a visible two-tier
  system that is exactly the kind of drift the phase discipline
  exists to prevent.

**Leaning Option A.** The retrofit is small (five tools, flat
schemas, under 50 lines total), and the norm-setting value is
disproportionate. Resolve at Task 2 kickoff by counting the
actual LOC and confirming.

### Q4 — Do we split Phase 10 if Task 1 turns out bigger than expected?

Task 1 has a latent risk: the `scan_topics` primitive may hit
redb-key-ordering subtleties I have not foreseen (the Phase 8
`\x01s\x01` prefix layout was picked for a reason, and walking
it for scan semantics may surface ordering constraints that
require a data-layout revision). If Task 1 turns out to be half
a phase on its own, the correct move is **split Phase 10 in
two**: Phase 10 ends at Task 1 + exit freeze, and Tasks 2–3
carry forward to Phase 11's opening or a new Phase 10.5
foundation sub-phase.

**Resolution rule:** if Task 1 is not green in one working
session, stop Task 1 mid-flight and split. Do not push through.
The Phase 9 Task 5 precedent ("stop mid-task to present shape
options to the user") is the template.

### Q5 — Phase 11 opens immediately after Phase 10, or a dogfood gap?

The Phase 12-style operator-verification phase pattern is
scheduled for *after* Phase 11 (coder role dogfood). Phase 11
does not need an operator-verification gap before it opens —
Phase 10 ships no user-visible behavior change, so there is
nothing for an operator pass to verify. **Resolution: Phase 11
opens directly after Phase 10 freezes.** Record in Phase 10
exit ship record.

## Exit criteria (draft — revised as work lands)

These are the conditions for calling Phase 10 complete. The
draft list will be revised into a final checklist at the Task 4
freeze, matching the Phase 7–9 pattern.

- [ ] Task 1 shipped: `scan_topics` primitive exists, wildcard
      scope exists, `MemoryReadTool` accepts the `topics: "*"`
      variant, four unit tests cover fan-out + isolation +
      attenuation-denied + literal-sentinel regression.
- [ ] Task 2 shipped: `Tool::input_schema()` exists with default
      `None`, hand-rolled validator in `aivyx-core/src/schema.rs`
      under 100 lines with zero new dependencies, turn loop
      calls validator before `required_scope`, every shipped
      tool has a real schema (Q3 Option A).
- [ ] Task 3 shipped: `StreamEvent::ToolCallStarted` has a
      `tool: &'a str` field, all emission sites updated, all
      consumers (local + telegram + audit) updated, existing
      `ToolCallStarted` tests pin the new field.
- [ ] `cargo test --workspace` green. Expected count 335–340
      (326 entry + 4 Task 1 + ~7 Task 2 + 0 Task 3). Above the
      "≥ +10" consolidation heuristic.
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
      clean at exit. Pre-commit hook caught zero regressions
      across the phase, matching Phase 9's discipline outcome.
- [ ] `DESIGN.md` still byte-identical to `e0d6437`. **Streak
      rolls to ten phases.** No amendment needed.
- [ ] Production-core byte-identity streak **re-baselines** to
      Phase 10's exit commit. The break is recorded in the
      Task 2 and Task 3 ship logs with the reason (additive
      trait refinement within D3's contract). Future streak
      accounting reports from the new baseline.
- [ ] Zero-new-dep streak: **held** if Q3 resolved to hand-
      rolled validator. Phase 10 adds no new dependencies.
- [ ] Q1 (wildcard shape), Q2 (nesting scope), Q3 (retrofit
      scope), Q4 (split rule), and Q5 (dogfood gap) all
      resolved and recorded under a "Decisions made during
      Phase 10 that aren't in DESIGN.md" block in the freeze.
- [ ] Rolling deferred-items list: cross-topic `memory.read`
      **closed**, runtime JSON-schema validation **closed**,
      tool name in `StreamEvent::ToolCallStarted` **closed**.
      The foundation backlog is empty at Phase 10 exit.
- [ ] Phase 11 ROADMAP entry refined with whatever Phase 10
      Task 2 taught us about the `Tool` trait surface.
