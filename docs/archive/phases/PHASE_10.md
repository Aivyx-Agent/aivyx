# Phase 10 — Tool-layer refinement + memory substrate

**Status:** Active (opened 2026-04-14). This document will churn
during the phase and freeze at exit under a final Exit criteria
block at the bottom, matching the Phase 7–9 precedent.
**Predecessor:** [PHASE_9.md](PHASE_9.md) (exit commit `7052ecc`,
hash backfill `1853673`)
**Contract:** [`../DESIGN.md`](../../../DESIGN.md) (Deliverables 1–8, all
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

## Task 1 — design resolutions recorded mid-implementation

Before writing code, a close reading of `aivyx-memory/src/tools.rs`,
`aivyx-memory/src/redb.rs`, and `aivyx-core/src/lib.rs` surfaced
architectural facts that revise the draft task breakdown above.
Recording them here so the ship record can cite the resolution IDs.

### DQ1 — `scan_topics` substrate shape: **resolved as Option B**

Task 1 adds one new method to the `Memory` trait:

```rust
async fn scan_prefix(&self, topic_prefix: &str)
    -> Result<Vec<(String, Vec<MemoryEntry>)>, MemoryError>;
```

Returns `(physical_topic, entries)` pairs for every topic whose
physical form starts with `topic_prefix`, with `entries` sorted
newest-first per topic (matching `get_recent`'s ordering). The
substrate stays session-oblivious — it walks raw topic-string
prefixes. The tool layer (not the substrate) is responsible for
knowing that the `\x01s\x01<session>\x01` prefix structure means
"everything in this session."

This preserves the two-layer architecture `aivyx-memory` already
has: `Memory` trait speaks in topic strings, tool wrappers
(`MemoryReadTool`) speak in logical topics + optional sessions.

### DQ2 — Wildcard response shape: **resolved as grouped**

`MemoryReadTool::execute` with `topics: "*"` returns:

```json
{
  "topics": [
    {"topic": "notes", "entries": [...], "count": 3},
    {"topic": "todos", "entries": [...], "count": 5}
  ],
  "topic_count": 2
}
```

One object per logical topic found in the session. Logical topic
names are extracted from the physical topic on the way out (strip
the `\x01s\x01<session>\x01` prefix). This matches the intent of
the wildcard variant — cross-topic discovery rather than flat
concatenation — and the response shape is distinguishable from
the single-topic `{"topic": ..., "entries": [...]}` shape at the
top-level field name, so a consumer can dispatch on either.

### DQ3 — Wildcard limit semantics: **resolved as per-topic with lower default**

The wildcard variant reuses the existing `limit` parameter but:

- **Default** drops from `DEFAULT_READ_LIMIT` (16) to a new
  `DEFAULT_WILDCARD_READ_LIMIT` (4). Rationale: the wildcard
  variant is for discovery, not deep recall — showing 4 recent
  entries per topic is plenty to see what each topic is about.
- **Max** stays at `MAX_READ_LIMIT` (64) — reusing the same hard
  cap means there is still one number to reason about.
- Per-topic semantics. A session with 20 topics and `limit: 4`
  returns up to 80 entries. Worst case is `64 × number_of_topics`,
  which is bounded by how many topics a session actually has.

### DQ4 — redb-key walking: **informational heads-up, no surprise**

`RedbMemory::scan_prefix` walks `e\0 || topic_prefix` at the redb
level. The `\0` separator between physical_topic and seq_be is
distinct from `\x01` bytes inside the physical topic, so the key
decoder can locate the seq tail unambiguously. Implementation is
~40 lines following the existing `seed_counter_from_storage`
pattern (same `scan_prefix` helper on `DomainHandle`, same
`seq_from_entry_key` decoder).

## Task 2 — correction recorded mid-implementation

The draft task breakdown above says Task 2 adds `input_schema()`
to the `Tool` trait as an additive refinement. **This is wrong.**

A close reading of `aivyx-core/src/lib.rs:477` revealed that
`Tool::input_schema(&self) -> &serde_json::Value` has existed on
the trait since Phase 6. Every shipped tool already returns a real
schema (not `None`), and `llm_planner.rs:125` forwards it into the
Anthropic `tool_input_schema` field. The schema is **declarative
advice to the LLM**, not a runtime-enforced gate.

Task 2 therefore becomes **narrower and does not touch the `Tool`
trait at all**:

1. Add a hand-rolled validator in `aivyx-core/src/schema.rs` (new
   module, under 100 lines) covering the JSON Schema subset the
   shipped tools actually emit — `type: object`, `properties`,
   `required`, `additionalProperties: false`, `type: string`,
   `type: integer` with `minimum`/`maximum`.
2. Wire the validator into `agent.rs` at the admission point
   (between the session-partition injection at `agent.rs:326–330`
   and the `required_scope` call at `agent.rs:332`). On mismatch,
   return `ToolOutcome::Failed` with a detail describing which
   field failed.
3. No `Tool` trait change. No shipped-tool schema retrofits (the
   five shipped tools already have real schemas). No intentional
   streak break on `aivyx-core/src/lib.rs` — **the production-
   core byte-identity streak held since `c3883be` may survive
   Phase 10** if Task 3 can also be done without touching
   `lib.rs`. Task 3's `StreamEvent::ToolCallStarted` refinement
   does still touch `lib.rs`, so the streak *does* break in
   Task 3 alone, but for a smaller reason than originally
   documented.

**Rationale for recording the correction here:** discovering an
incorrect task sketch mid-phase is exactly what the "draft task
breakdown — revised as work lands" framing exists to handle. The
draft stays in place above as historical context; the resolutions
below are the definitive Phase 10 plan.

### Task 2 — second mid-phase correction: validation order vs. session injection

While wiring the validator into `agent.rs`, a second subtlety
emerged. The draft above said to run the validator "between the
session-partition injection at `agent.rs:326–330` and the
`required_scope` call at `agent.rs:332`." **This is also wrong.**

Running validation *after* session injection means that every
`memory.*` tool call fails validation on any channel that sets a
session partition. Here's why:

- `memory.read`, `memory.write`, and `memory.forget` schemas declare
  `additionalProperties: false`.
- `session` is injected into the tool input as a reserved key by
  the turn loop (Phase 8 Task 2), and is **deliberately not** in
  any tool's `input_schema` — it is a loop-internal routing field,
  not an agent-visible contract.
- A validator running after injection would see `{"topic": "notes",
  "session": "chat-42"}`, note that `session` is not in
  `properties`, and reject it via `UnknownField`.

Resolution: **validation runs before session injection**, not
after. The call order in `agent.rs` is now:

1. Look up the tool.
2. **Validate the raw planner input against `tool.input_schema()`**.
   → If invalid, return `ToolOutcome::Failed` with detail.
3. Inject `session` if the channel provides one.
4. Compute `required_scope(input)`.
5. Capability gate.
6. `execute`.

This preserves the Phase 6 invariant that the schema contract is
"what the LLM may emit," strictly separate from the "what the
runtime reshapes the input into before `required_scope` sees it"
concern that session injection lives in.

Validation failures route through `ToolOutcome::Failed`, not
`ToolOutcome::Denied`. These two paths carry different semantics:
`Denied` is a capability-layer signal the agent lacks scope;
`Failed` is "the tool call is structurally broken." Prompt-
injection attempts emitting malformed JSON must not be
indistinguishable from an under-capabilitied agent in the audit
chain, so they stay on the Failed track.

The two integration tests in `agent.rs::tests` lock this in:

- `malformed_tool_input_is_rejected_before_required_scope` — the
  FakeTool's `scope_fn` is a panicking closure; if validation ever
  regresses to run after `required_scope`, the panic fires and the
  test fails loudly rather than silently accepting.
- `well_formed_tool_input_passes_validation_and_runs` — the
  counterpart proving the validator isn't over-rejecting (a buggy
  validator that rejected everything would still pass the negative
  test alone).

## Task 3 — mid-phase scope expansion: close the emission gap

The draft breakdown framed Task 3 as "add a `tool_name: &'a str`
field to `StreamEvent::ToolCallStarted` / `ToolCallFinished` and
update consumers." During implementation a grep for
`ToolCallStarted` / `ToolCallFinished` revealed a larger truth:
**the turn loop never emitted these events.** The variants have
existed on the enum since Phase 5. The channel renderers
(`aivyx-channel/src/render.rs`, `aivyx-telegram/src/telegram_channel.rs`)
have had match arms for them since Phase 5 / Phase 8. The audit
bridge in `aivyx-audit` has extracted tool IDs from them. But
nothing in `agent.rs` ever called `ctx.stream_event(...)` with a
`ToolCallStarted` or `ToolCallFinished`. All the render paths and
all the existing tests were exercising dead code.

This is a latent emission gap, silently carried for five phases,
that only a grep during a seemingly-unrelated field refinement
surfaced.

Options presented to the user:

- **Option A — minimum viable:** add the field, update consumers
  and tests, leave the emission gap untouched. Preserves the
  exact scope of the Phase 10 draft. Leaves the gap for a later
  phase to close.
- **Option B — fully close the gap:** add the field **and** wire
  emission from the turn loop, so every tool call produces a
  `ToolCallStarted` before `tool.execute` and a
  `ToolCallFinished` after. Expands Task 3 by one integration
  point but closes a dead-code wiring debt with known-correct
  consumers already in place.
- **Option C — emit now, name later:** wire the emission first
  with the `tool_name` field deferred to a separate follow-up.
  Splits Task 3 into two commits. Not materially cheaper than B.

User resolution: **Option B.** The rationale is that the consumers
are already written, tested, and reviewed — the only thing missing
is the four lines that actually call `stream_event` with the
variant. Closing the gap in Task 3 retires the wiring debt for
free rather than letting it rot for another phase.

### Denied calls suppress stream events

A consequence of emitting from the turn loop: the emission point
must sit **after** the capability gate, not before. Denied calls
must not emit a `ToolCallStarted`, because the channel's user-
visible text would then see a `→ tool_name` line for a call that
never ran, which is worse than seeing nothing. The
`denied_tool_call_emits_no_stream_events` integration test locks
this in: a FakeTool configured to require an absent scope runs,
and the `RecordingChannel` must see zero `ToolCallStarted` /
`ToolCallFinished` events.

### Tests added in Task 3

- `tool_call_emits_started_and_finished_events_with_tool_name` —
  happy path. Records that the turn loop emits both events, that
  `tool_name` matches the `Tool::name()` the registry returned,
  and that `ToolCallFinished.outcome_summary` is a non-empty
  static label.
- `denied_tool_call_emits_no_stream_events` — the suppression
  invariant above.
- `aivyx-channel/src/render.rs` — two rewritten renderer unit
  tests (`tool_call_started_renders_tool_name_and_arrow`,
  `tool_call_finished_renders_tool_name_and_summary`) that assert
  the human tool name appears (`→ memory.read`, `← memory.write`)
  and that the `ToolId` UUID does **not** leak into human
  terminal output.
- `aivyx-telegram/src/tests.rs` — updated event construction and
  assertions for the same rendering contract.

### Streak accounting

The production-core byte-identity streak (held since `c3883be`)
breaks in Phase 10. Both Task 2 and Task 3 contributed, for
different reasons:

- **Task 2 (`65ae30a`)** — one-line addition `pub mod schema;`
  at the top of `aivyx-core/src/lib.rs`. The validator logic
  itself lives in the new `schema.rs` module, not in `lib.rs`,
  but declaring a new module counts as a `lib.rs` byte change
  at streak-measurement level. The mid-phase Task 2 correction
  block above optimistically said the streak "may survive
  Phase 10" if Task 3 also avoided `lib.rs` — that statement
  was wrong the moment the module declaration landed. Recording
  the correction here rather than silently back-editing the
  earlier block preserves the journal as an honest trace of
  what was believed when.
- **Task 3 (`74d97ec`)** — `tool_name: &'a str` fields added to
  `StreamEvent::ToolCallStarted` and `StreamEvent::ToolCallFinished`.
  The larger of the two Phase 10 `lib.rs` touches, and the one
  that materially refines a contract type rather than just
  declaring a module.

Both changes are additive refinements within D3's contract. The
Phase 10 exit ship record attributes the break to both tasks and
re-baselines the streak to Phase 10's exit commit.

Task 1 left `aivyx-core/src/lib.rs` byte-identical. If a future
foundation phase needs to re-establish the streak from a single-
task baseline, Task 1 is the shape to copy.

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

## Task 1 — shipped (2026-04-14)

**Commit:** `b1706ca` — `Phase 10 task 1: cross-topic memory.read
substrate + tool wiring`.

Adds `Memory::scan_prefix` to the session-oblivious substrate
(`InMemoryMemory` and `RedbMemory` both implement it following the
DQ1/DQ4 resolutions above), and wires `MemoryReadTool` to accept
a `{topics: "*"}` variant that fans out across every logical topic
in the session, grouping entries per topic at the tool-response
level per DQ2. The wildcard request uses `DEFAULT_WILDCARD_READ_LIMIT
= 4` per DQ3 — discovery shape, not deep recall.

The wildcard scope lands as a textual sentinel — Q1 Option A —
`memory.read:topic:*:session:<session>`. The `globset`-based
`Scope` matcher already handles `*` inside topic segments, so no
new `Scope` variant is needed. The literal-star regression test
(`literal_star_topic_does_not_match_wildcard_scope`) pins that an
agent cannot spoof wildcard access by naming a topic `"*"` —
Phase 6's topic-name grammar already forbids `*` as a literal, so
the test encodes that grammar rather than adding a new check.

Tests: +19 (9 tool-layer wildcard tests in `tools.rs`, 5
`InMemoryMemory::scan_prefix` tests in `lib.rs`, 5
`RedbMemory::scan_prefix` tests in `redb.rs`). The on-disk tests
include a sibling-prefix boundary lock (`notes` vs `notesfoo`) and
a metadata-key exclusion lock (`m\x00` vs `e\x00` discriminators)
— neither tested in the substrate before Task 1 because the old
per-topic API never exposed the prefix-walk path.

`aivyx-core/src/lib.rs` byte-identical after Task 1 — the entire
change lives in `aivyx-memory` and `aivyx-capability`.

## Task 2 — shipped (2026-04-14)

**Commit:** `65ae30a` — `Phase 10 task 2: hand-rolled JSON-schema
validator in the turn loop`.

Adds `aivyx-core::schema::validate`, a hand-rolled JSON-Schema
subset validator. The module declaration `pub mod schema;` is the
only change to `aivyx-core/src/lib.rs` — the validator logic and
its ~20 unit tests live in the new `schema.rs` file. Zero new
dependencies. Validator covers `type: object/string/integer`,
`properties`, `required`, `additionalProperties: false`, `enum` on
strings, and `minimum`/`maximum` on integers. Q2 (nesting) resolved
as "narrow by design, extend when a real nested schema lands"; Q3
resolved as Option A (every shipped tool already has a real schema
from Phase 6 — the Task 2 correction block above records the
discovery that `Tool::input_schema()` has existed since Phase 6 and
the task's original "additive trait refinement" framing was wrong).

The turn loop in `agent.rs` now validates the raw planner input
against `tool.input_schema()` at the admission point —
**critically, before the Phase 8 session-partition injection**,
per the second mid-phase correction above. Validation failures
route through `ToolOutcome::Failed`, not `ToolOutcome::Denied`, so
prompt-injection attempts emitting malformed JSON stay
distinguishable from under-capabilitied agents in the audit trail.

Tests: +22 (20 schema-module unit tests, 2 `agent.rs` integration
tests — `malformed_tool_input_is_rejected_before_required_scope`
with a panicking `scope_fn` that proves ordering and
`well_formed_tool_input_passes_validation_and_runs` as the positive
counterpart).

Task 2 is the first of two Phase 10 tasks that touch
`aivyx-core/src/lib.rs`, at a minimum (one-line module declaration
only). The full streak-accounting rationale is in the Task 3
streak-accounting subsection above.

## Task 3 — shipped (2026-04-14)

**Commit:** `74d97ec` — `Phase 10 task 3: tool name in StreamEvent
+ close emission gap`.

Adds `tool_name: &'a str` to both `StreamEvent::ToolCallStarted`
and `StreamEvent::ToolCallFinished`. Closes the rolling "tool name
in `StreamEvent::ToolCallStarted`" deferral that carried forward
from Phase 8 → 9 → 10. Both renderers (`aivyx-channel::render` and
`aivyx-telegram::telegram_channel`) were updated to display the
human tool name (e.g. `→ memory.read`) instead of the
`ToolId`-derived short UUID fallback that pre-dated Task 3, and
their unit tests now pin that the `ToolId` UUID does **not** leak
into human-visible output.

**Mid-phase scope expansion:** grep during Task 3 revealed that the
turn loop has never emitted `ToolCallStarted` or `ToolCallFinished`
— the variants existed on the enum since Phase 5, all consumers
were written and tested against them, but `agent.rs` never called
`ctx.stream_event` with either variant. Task 3 was expanded to
Option B — close the gap fully — per the user's explicit
resolution. The turn loop now emits both events around
`tool.execute`, **after the capability gate**, so denied calls
suppress both. Recorded in the "Task 3 — mid-phase scope expansion"
block above.

Tests: +2 integration tests in `agent.rs`
(`tool_call_emits_started_and_finished_events_with_tool_name` as
happy path, `denied_tool_call_emits_no_stream_events` as the
denial-suppression invariant), 2 rewritten renderer tests in
`aivyx-channel::render` (which also deletes the dead `short_id`
helper and its 2 tests), and updated assertions in
`aivyx-telegram::tests`. Net test delta across Task 3 is +2 (2
new integration + 2 new renderer rewrites - 2 deleted `short_id`
tests + 0 other deltas).

Task 3 adds `tool_name: &'a str` fields to two `StreamEvent`
variants in `aivyx-core/src/lib.rs` — the second and larger of
Phase 10's two `lib.rs` touches. Together with Task 2's one-line
module declaration, these two commits break the production-core
byte-identity streak held since `c3883be` (Phase 8 Task 2).

## Task 4 — shipped (2026-04-14) — Phase 10 exit freeze

**Commit:** _this commit_ — `docs(phase-10):` exit freeze.

Phase 10 closes with the contract unchanged and the `DESIGN.md`
empty-diff streak rolling forward to **ten phases**. This task is
a docs-only commit that freezes PHASE_10.md, updates `README.md`
and `docs/ROADMAP.md` to reflect the new status, and refines the
Phase 11 roadmap entry with what Phase 10 learned about the
`Tool` trait surface (short version: the trait did not need to
grow).

### What landed in Phase 10 (one-line per task)

1. **Task 1** (`b1706ca`) — cross-topic `memory.read` via a
   substrate `Memory::scan_prefix` primitive and a
   `{topics: "*"}` tool variant behind the
   `memory.read:topic:*:session:<session>` wildcard scope.
   Closes the seven-phase cross-topic deferral. +19 tests.
2. **Task 2** (`65ae30a`) — hand-rolled JSON-Schema subset
   validator in `aivyx-core::schema`. Validates planner input
   before session injection and before `required_scope`.
   Discovered mid-phase that `Tool::input_schema()` already
   existed since Phase 6; no trait change landed. +22 tests.
3. **Task 3** (`74d97ec`) — `tool_name: &'a str` field added to
   `StreamEvent::ToolCallStarted` and `ToolCallFinished`, plus
   the mid-phase Option B expansion to **close the five-phase
   latent emission gap** (turn loop now actually emits the
   variants). +2 tests (net; 2 deleted dead `short_id` tests
   offset by 2 new integration tests and 2 rewritten renderer
   tests).
4. **Task 4** — this exit freeze.

### Exit-criteria results

See the Exit criteria (final) checklist below for the item-by-
item rollup. Headline numbers:

- **`cargo test --workspace`**: green at **367 tests** (Phase 10
  entry baseline: 326). Net delta **+41**, well above the
  "≥ +10" consolidation heuristic. The bulk is Task 1's
  substrate-plus-tool retrofit (+19) and Task 2's validator
  module (+22).
- **`cargo clippy --workspace --all-targets -- -D warnings`**:
  clean at exit. Matching Phase 9, the pre-commit hook caught
  every would-be regression at its own commit time; no task
  left a regression for the exit sweep to discover.
- **`DESIGN.md` empty-diff streak**: byte-identical to
  `e0d6437` (the contract-lock commit). **Streak rolls to ten
  consecutive phases** on an unchanged core contract. No
  amendment file under `docs/amendments/` was needed — the
  directory still does not exist. Note that the `StreamEvent`
  code block in DESIGN.md (lines 228–240) still shows the
  Phase 5 shape of `ToolCallStarted` / `ToolCallFinished`
  without the `tool_name` field that Task 3 added to `lib.rs`.
  This is **not** a drift to amend — the code blocks in
  `DESIGN.md` are illustrative sketches of contract intent,
  not byte-exact API definitions. Precedent: Phase 6 added
  `Tool::input_schema()` to the `Tool` trait without touching
  its `DESIGN.md` code block, and the D3 "key commitments"
  bullet list explicitly permits `StreamEvent` variants to
  grow ("start how we mean to go on… channels are free to
  ignore any variant they don't care about"). Adding a new
  read-only field to an existing variant falls cleanly inside
  that commitment.
- **Production-core byte-identity streak**: **broken** in Phase
  10, re-baselined at this commit. The break is attributable to
  Task 2 (`65ae30a`, one-line `pub mod schema;`) and Task 3
  (`74d97ec`, two-field `StreamEvent` refinement). Both changes
  are additive refinements within D3's contract. Task 1 left
  `lib.rs` byte-identical — useful template for a future
  foundation phase that wants to re-establish the streak from a
  single-task baseline.
- **Zero-new-dep streak**: **held**. Phase 10 added no new
  workspace dependencies. The validator is ~100 lines of
  hand-rolled code against `serde_json::Value`, not a pulled-in
  crate.

### Decisions made during Phase 10 that aren't in DESIGN.md

- **Q1 — wildcard scope shape:** **Option A — textual sentinel
  `*` inside the topic segment.** Scope string
  `memory.read:topic:*:session:<session>`. Reuses the existing
  `globset`-based `Scope` matcher and requires no new
  `Scope` variant. The literal-star regression test encodes
  Phase 6's topic-name grammar as the defense against an agent
  naming a topic `"*"` to spoof wildcard access. Resolved at
  Task 1 kickoff.
- **Q2 — validator nesting support:** **narrow by design.** The
  five shipped-tool schemas are all flat; the validator
  handles `type: object/string/integer` with flat
  `properties`, and a future tool that needs nesting extends
  the validator when it lands. Recorded in Task 2 ship log.
- **Q3 — shipped-tool schema retrofit scope:** **not applicable
  — superseded by Task 2 correction.** The draft task
  breakdown assumed `Tool::input_schema()` did not yet exist;
  it has existed since Phase 6 and every shipped tool already
  returns a real schema. No retrofit was needed. Recorded as
  the first mid-phase Task 2 correction above.
- **DQ1 / DQ2 / DQ3 / DQ4 — Task 1 substrate and response
  shape decisions:** resolved as Option B for the substrate
  (session-oblivious `scan_prefix`), grouped for the wildcard
  response shape, per-topic limit with `DEFAULT_WILDCARD_READ_LIMIT
  = 4`, and redb-key walking on `e\0 || topic_prefix`. Full
  text in the Task 1 design-resolutions block above.
- **Validation ordering (Task 2 second correction):**
  **validation runs before session injection.** Resolution
  recorded in the second Task 2 correction block above. The
  `malformed_tool_input_is_rejected_before_required_scope`
  integration test locks the ordering with a panicking
  `scope_fn`.
- **Failed vs Denied routing for validation failures:**
  **Failed**, not Denied. Prompt-injection attempts emitting
  malformed JSON must stay distinguishable from
  under-capability agents in the audit chain. Recorded in
  the Task 2 second correction.
- **Task 3 mid-phase scope — Option B, close the emission
  gap:** resolved by the user explicitly
  ("Option B, Lets fully close the gap"). The turn loop now
  emits `ToolCallStarted` / `ToolCallFinished` around
  `tool.execute`, after the capability gate so denied calls
  suppress both. Recorded in the Task 3 scope-expansion block
  above.
- **Q4 — split Phase 10 if Task 1 over-runs:** **not
  triggered.** Task 1 landed green in one working session, so
  the split rule did not fire. Kept on the record as a
  template for future foundation phases.
- **Q5 — dogfood gap before Phase 11:** **no gap.** Phase 10
  ships no user-visible behavior change, so an
  operator-verification pass would have nothing to verify.
  Phase 11 opens directly after this commit.

### Phase 10 deferrals paid down

Phase 10 inherited three concrete rolling deferrals and paid
down all three:

- **Cross-topic `memory.read`** (Phase 6 Q3 → Phases 7 → 8 → 9
  → 10): **closed in Task 1.** The substrate primitive exists,
  the wildcard tool variant exists, the wildcard scope is out
  of every default tier ceiling so cross-topic access is a
  deliberate opt-in, and nineteen tests pin the contract.
- **Runtime JSON-schema validation for tool input** (Phase 7
  → 8 → 9 → 10): **closed in Task 2.** The validator runs on
  every tool call at the turn-loop admission point. Zero new
  dependencies.
- **Tool name in `StreamEvent::ToolCallStarted`** (Phase 8 → 9
  → 10): **closed in Task 3.** Additionally, Task 3 closed the
  five-phase emission gap that the same grep surfaced — the
  turn loop now actually emits the variants that had existed
  since Phase 5.

**The foundation backlog is empty at Phase 10 exit.** Future
phases open with no pre-existing rolling deferrals — a
position Aivyx has not been in since Phase 6 opened.

### Exit criteria (final)

- [x] Task 1 shipped: `Memory::scan_prefix` primitive exists,
      wildcard scope exists, `MemoryReadTool` accepts the
      `topics: "*"` variant, nineteen unit tests cover
      substrate prefix walks + sibling-prefix boundary +
      metadata exclusion + tool-layer fan-out + session
      isolation + attenuation-denied + literal-sentinel
      regression.
- [x] Task 2 shipped: validator in `aivyx-core::schema`
      (~100 LOC), zero new dependencies, turn loop calls
      validator **before** session injection and before
      `required_scope`, and every shipped tool already had a
      real schema from Phase 6. The draft task sketch's
      "additive `Tool` trait refinement" framing was corrected
      mid-phase to "validator module only; no trait change."
- [x] Task 3 shipped: `StreamEvent::ToolCallStarted` and
      `ToolCallFinished` have a `tool_name: &'a str` field,
      all renderers updated, `ToolId` UUID no longer leaks
      into human output. **And** the turn-loop emission gap
      for both variants is closed per the Option B mid-phase
      expansion.
- [x] `cargo test --workspace` green at **367 tests**. Net
      Phase 10 delta **+41**, well above the "≥ +10"
      consolidation heuristic.
- [x] `cargo clippy --workspace --all-targets -- -D warnings`
      clean at exit. Pre-commit hook caught every
      would-be regression at its own commit time — matching
      Phase 9's discipline outcome.
- [x] `DESIGN.md` still byte-identical to `e0d6437`. **Streak
      rolls to ten phases.** No amendment needed.
- [x] Production-core byte-identity streak **broken** in
      Phase 10 by Task 2 (`65ae30a`) and Task 3 (`74d97ec`),
      re-baselined at this commit. Both changes are additive
      refinements within D3's contract. Task 1 left `lib.rs`
      byte-identical.
- [x] Zero-new-dep streak: **held.** Phase 10 added no new
      workspace dependencies.
- [x] Q1, Q2, Q3, Q4, Q5 all resolved and recorded under
      "Decisions made during Phase 10 that aren't in
      DESIGN.md" above.
- [x] Rolling deferred-items list: cross-topic `memory.read`
      **closed**, runtime JSON-schema validation **closed**,
      tool name in `StreamEvent::ToolCallStarted` **closed**.
      The foundation backlog is empty at Phase 10 exit.
- [x] Phase 11 ROADMAP entry refined with whatever Phase 10
      Task 2 taught us about the `Tool` trait surface.
