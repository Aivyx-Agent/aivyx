# Mission topic-naming discipline, take 2 (POLISH_WAVES.md sub-project 5, remaining item) — design

**Status:** Approved, ready for planning.

## Motivation

`docs/POLISH_WAVES.md` sub-project 5's one remaining open item. Original
finding (`VITRINE.md` §4 P2): one mission filed memory writes under
inconsistent naming — three per-airport entries under bare ICAO codes
(`YPJT`/`YMML`/`YSSY`, genuinely distinct subjects, not the problem) plus a
mission-level summary artifact filed inconsistently as `overall_conditions`
by one step and `overall_conditions_summary` by another — no agreement
*within one mission* on what to call the same logical thing.

A first attempt (`docs/superpowers/specs/2026-08-30-missions-polish-design.md`
§D, `docs/superpowers/plans/2026-08-30-missions-polish.md`) wired the
existing `ConcreteAgent::with_memory_topic_prefix` mechanism into mission
specialist construction, keyed on a per-mission id. **Implemented, then
reverted at final review**, for two confirmed reasons:

1. It doesn't fix the finding. `with_memory_topic_prefix` **prepends** a
   namespace to whatever topic string the specialist's model chose
   (`crates/aivyx-core/src/agent.rs:1211`, `logical_with_role_prefix` in
   `crates/aivyx-memory/src/tools.rs:246`) — it never touches the
   *logical* topic name itself, which is what Concord's conflict-detector
   and the Memory screen's topic rail actually group by. A per-mission
   prefix isolates missions from each other; it cannot make two different
   strings (`overall_conditions` vs `overall_conditions_summary`) become
   the same string.
2. It actively harmed the sibling Concord conflict-detector item (needs
   ≥2 entries under one topic name to compare — a mission-fragmented
   namespace often produces exactly 1), plus added unbounded per-mission
   pages to knowledge-wiki synthesis and cluttered the Memory screen's
   topic rail.

The revert's own recorded lesson: *"the real fix likely needs the LEAD's
own plan decomposition to hand each step a canonical topic name to use,
not a prefix applied after the fact."* This design is that fix, re-grounded
against the real mission-execution path (not assumed) to confirm it's
architecturally sound before committing to it a second time.

## Confirmed architecture (read, not assumed)

- `TeamRuntime`'s step-execution loop (`crates/aivyx-team/src/runtime.rs`,
  around line 241-252) builds a **fresh** specialist agent per step
  execution via `self.pool.run(&member, &input, lead_channel)` →
  `SpecialistPool::run` (`crates/aivyx-team/src/pool.rs:277`) →
  `self.factory.build(member, &self.ceiling)` — **not** one shared agent
  instance reused across every step a given specialist name runs. A
  genuinely per-step topic assignment is therefore a small threading
  change, not a restructuring.
- The step object (`&Step`, carrying `step.kind`) is directly in scope at
  the `self.pool.run(...)` call site — no lookup needed to reach a step's
  own fields from the point that dispatches it.
- `crates/aivyx-core/src/agent.rs`'s turn-loop dispatch already has an
  established "rewrite the tool call's JSON input before scope-checking"
  pattern for exactly this class of problem — the session-partition
  injection (Phase 8 Task 2) and the `role_prefix` injection (Phase 11
  Task 2) both do this, at the same point, right before `tool.
  required_scope(&input)` runs (`agent.rs:1216`).
- `topic` is **not** a `memory.write`-exclusive field name —
  `memory.read`/`memory.forget` also use it (confirmed via
  `crates/aivyx-core/src/schema.rs`'s schema mirrors and
  `crates/aivyx-memory/src/tools.rs`). Any new dispatch-layer rewrite
  must gate on the tool's own name (`tool.name() == "memory.write"`),
  not merely on the input having a `topic` key.
- Exactly two places in the codebase construct `StepKind::Delegate { ... }`
  as a literal (everywhere else pattern-matches with `..`, which absorbs a
  new field automatically): `Step::delegate()`'s own builder
  (`crates/aivyx-team-types/src/mission.rs:81`, the canonical constructor
  essentially every caller and test uses) and `parse_step`
  (`crates/aivyx-team/src/orchestration.rs:73`, where the LEAD's JSON gets
  turned into a `Step`). Adding a field here is a two-site change, not a
  codebase-wide one.

## The mechanism

A new, per-step **canonical-topic override** — the LEAD assigns an exact
topic name to a step; the turn loop **replaces** (not prefixes) whatever
topic the specialist's own model chooses for `memory.write` calls made
during that step.

### 1. Plan shape

`StepKind::Delegate` (`crates/aivyx-team-types/src/mission.rs`) gains:

```rust
Delegate {
    specialist: String,
    prompt: String,
    /// Sub-project 5 — the LEAD's canonical memory-topic assignment for
    /// this step's `memory.write` calls, if any. `None` (the default,
    /// and every pre-existing plan's implicit value via `#[serde(default)]`)
    /// means the specialist chooses its own topic, today's behavior.
    /// When `Some`, every `memory.write` call this step's specialist
    /// makes is rewritten to use this exact topic — REPLACING the
    /// model's own choice, not merely namespacing it (see "Why replace,
    /// not prefix" below).
    #[serde(default)]
    memory_topic: Option<String>,
},
```

`Step::delegate()` sets `memory_topic: None`; a new consuming builder on
`Step`, `with_memory_topic(mut self, topic: impl Into<String>) -> Self`
(mirroring the existing `.after(deps)` builder exactly), sets it — for
programmatic/test construction. `StepKind` gains a `memory_topic(&self)
-> Option<&str>` accessor mirroring the existing `member()` accessor
(`Gate` steps return `None`, since only delegate steps write memory).

### 2. The LEAD's decomposition surfaces

Two independent places currently describe the `{goal, steps}` JSON shape
to an LLM and must both gain the field + the same guidance text, since
they're hand-authored separately, not derived from one schema:

- `DecomposeTaskTool`'s JSON schema and `description()`
  (`crates/aivyx-team/src/orchestration.rs`, `DecomposeTaskTool::new`)
  — adds a `"memory_topic": { "type": "string" }` property to a step
  object, and extends the tool description.
- `decompose_goal`'s hand-written prompt string
  (`crates/aivyx-team/src/planner.rs`, around line 138's `delegate: {{...}}`
  shape) — same field, same guidance, in that prompt's own JSON-shape
  description.

Guidance text (used in both, adapted to each site's existing voice):
*"Optional `memory_topic` (string): when two or more steps write memory
about the same logical subject (e.g. both refine a shared summary), give
them the SAME `memory_topic` so their writes land under one consistent
name instead of each specialist inventing its own. Leave unset when a
step's memory writes don't need to share a name with any other step."*

`parse_step` (`orchestration.rs`) reads `v.get("memory_topic").and_then
(Value::as_str)` into the new field, matching every other optional
string field's existing parsing convention in that function.

### 3. Threading the override to the specialist's agent

Three call sites, each a one-parameter addition, no restructuring:

- `TeamRuntime`'s step loop (`runtime.rs`, in the `futures.map` closure
  around line 241): extracts `step.kind.memory_topic()` and passes it to
  `self.pool.run(&member, &input, memory_topic, lead_channel)`.
- `SpecialistPool::run` (`pool.rs:277`) gains a `memory_topic: Option<&str>`
  parameter, passed to `self.factory.build(member, &self.ceiling,
  memory_topic)`.
- `SpecialistFactory::build` (`factory.rs:148`) gains the same parameter;
  right after the existing `.with_checkpointer(self.checkpointer.clone())`
  chain call (`factory.rs:204`), adds
  `.with_memory_topic_override(memory_topic.map(String::from))`.

### 4. The enforcement mechanism — a new, separate `ConcreteAgent` field

`crates/aivyx-core/src/agent.rs` gains a new field, `memory_topic_override:
Option<String>`, and builder `with_memory_topic_override(mut self, topic:
Option<String>) -> Self` — **deliberately separate from the existing
`memory_topic_prefix` field**, not a repurposing of it (the two mechanisms
have different semantics and different callers: `memory_topic_prefix` is
the interactive/operator-role path, unrelated to missions; this is
mission-only).

Dispatch-layer enforcement, added right after the existing `role_prefix`
injection block (`agent.rs`, right before `let needed: Scope =
tool.required_scope(&input);`):

```rust
// Sub-project 5 — LEAD-assigned canonical memory topic. Unlike the
// role_prefix injection just above (invisible to the model, preserved
// in the audit chain as the logical topic the agent actually typed),
// this REWRITES the topic itself: the whole point is that Concord's
// conflict-detector and the Memory screen's topic rail see ONE name
// across every step the LEAD assigned it to, not each specialist's own
// guess. Gated on the tool's own name, not merely "has a topic field" —
// memory.read/memory.forget also use `topic`, and rewriting theirs
// would be a correctness bug (a read/forget under a topic the operator
// or a different tool call didn't ask for).
if tool.name() == "memory.write"
    && let Some(topic) = self.memory_topic_override.as_ref()
    && let Some(obj) = input.as_object_mut()
{
    obj.insert("topic".to_string(), serde_json::Value::String(topic.clone()));
}
```

Because this rewrites `input["topic"]` in place (not a new side-channel
key), every downstream consumer of `topic_from_input` — `required_scope`'s
capability-scope derivation, `execute`'s audit event
(`AuditTag::MemoryAccess { query_or_key: topic.clone(), .. }`), and the
physical storage key — all see the same overridden name with zero changes
needed inside `aivyx-memory`. This is the direct fix for what the reverted
mechanism couldn't do: the *logical* topic itself changes, not just a
prefix in front of it.

### Why replace, not prefix (explicit trade-off)

The existing `role_prefix` mechanism is deliberately invisible to the
model and preserved verbatim in the audit chain, by design (interactive
sessions want per-role storage isolation without changing what the
operator sees the agent call the topic). This mechanism's entire purpose
is the opposite: making the audit-visible, Memory-screen-visible name
*agree* across steps. The trade-off, stated plainly: a specialist's own
response text (generated from its own tool-call decision, before the
override applies) could reference a topic name that differs from what
actually got written under, if the specialist's reasoning mentions the
topic by name. Rare (specialists don't typically narrate their own
memory-topic choices back to the operator) and benign (the memory entry
itself is correct; only incidental prose might mention a stale name) —
accepted, not silently glossed over.

### Why this doesn't repeat the reverted attempt's two harms

No per-mission (or any other blanket) namespacing is introduced. This is
an exact-name override, opt-in per step, active only when the LEAD
explicitly assigns one. Consequences of that framing:
- No unbounded per-mission wiki-synthesis pages (nothing scopes per
  mission at all).
- No Memory-screen topic-rail clutter (a LEAD-assigned topic is a normal,
  human-readable name like `overall_conditions`, not a mission-id-derived
  one).
- Concord's conflict-detector directly benefits: two steps the LEAD
  assigns the same topic now genuinely produce ≥2 entries under one real
  name — the exact precondition its comparison needs, which the reverted
  per-mission prefix mechanism starved by fragmenting topics instead.

## Testing

- `StepKind::memory_topic()` accessor: `Gate` → `None`; `Delegate` with/
  without the field → the expected value. Round-trip a `Step` through
  serde with the field absent (old-plan compat, `#[serde(default)]`) and
  present.
- `parse_step`: a `decompose_task`-shaped JSON step with `memory_topic`
  set parses into a `Step` carrying it; without the key, `None`.
- Threading: a `SpecialistFactory::build(..., Some("overall_conditions"))`
  call produces a `ConcreteAgent` whose `memory_topic_override` is set —
  test via the same fixture-building pattern
  `crates/aivyx-team/src/factory.rs`'s own existing `SpecialistFactory::
  build`-driven tests already use (referenced in that file around line
  548-616).
- The dispatch-layer rewrite itself (`agent.rs`): a `ConcreteAgent` built
  `with_memory_topic_override(Some("x"))`, driven through a turn that
  calls `memory.write` with a *different* topic in its own tool-call
  input — assert the resulting audit `MemoryAccess` event and the stored
  entry both show topic `"x"`, not the model's original choice. A
  sibling test confirms `memory.read`/`memory.forget` calls in the same
  turn are **not** rewritten (the tool-name gate holds).
- End-to-end: a 2-delegate-step mission plan where both steps' prompts
  ask for a `memory.write` and both steps carry the same `memory_topic`
  — assert both entries land under that one topic (real fix for the
  original finding), using `aivyx-team`'s existing mission-runtime test
  harness (`FakeLeadChannel`/`FakeProvider`/`team_pool`, already used by
  `runtime.rs`'s own test module).
- Full sweep before merge: `cargo clippy --workspace --exclude
  aivyx-desktop --all-targets -- -D warnings` and `cargo test --workspace
  --exclude aivyx-desktop` (or this environment's established
  `default-members`-only fallback).
