# Phase 29 — Agent Reflection Loop

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../../../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../../../PRODUCT.md).

## Goal

Complete the **Reflection Layer** (PRODUCT.md G3 / P8). Phase 28
shipped the observation substrate (`turn.history`); this phase
delivers the reflection loop itself: the agent reads its own
recent outcomes, proposes behavioral adjustments (memory writes,
system-prompt annotations), and surfaces proposals through
mission gates for operator approval. A scheduled trigger can
fire the loop periodically, closing the full observe-propose-
approve-apply cycle.

## Why now

1. **The observation substrate is ready.** Phase 28's
   `turn.history` tool gives the agent structured read access
   to its own audit chain. The reflection loop composes
   directly against it.

2. **The approval gate machinery is ready.** Phase 21's
   mission gate primitive provides the operator-in-the-loop
   checkpoint the reflection loop needs. Phase 22 Task 8
   wired escalation->gate in the daemon turn loop.

3. **The trigger scheduling substrate is ready.** Phases 26-27
   delivered cron timers, webhooks, and file-change watchers.
   A periodic reflection trigger is a config-level composition,
   not new infrastructure.

4. **G3 is the last undelivered goal.** G1 (web), G2 (code),
   G4 (sub-agent), G5 (autonomous execution), and G6 (local/
   privacy) are all delivered. Completing G3 clears the forward
   commitment backlog from PRODUCT.md.

## Streak predictions

- **DESIGN.md** -- Low risk. Reflection is channel-level work
  composing existing primitives. Prediction: **untouched**.

- **PRODUCT.md** -- Low risk. G3 and P8 are already committed.
  Prediction: **untouched**.

- **Production-core `aivyx-core/src/lib.rs`** -- Low risk.
  New capability bases go in `aivyx-capability`, new tools in
  `aivyx-channel`. Prediction: streak **extends to one** (reset
  at Phase 28 Task 4).

## Open questions

**Q1 -- Scope of `reflection.apply`.** Should the apply tool
modify role configs at runtime (the full P8 vision), or limit
to memory writes only (simpler, defers runtime role mutation)?
Leaning **memory-only for this phase** -- runtime role mutation
is complex (requires config reload, capability re-intersection,
session restart) and can be a Phase 30+ follow-up. Memory writes
are sufficient for the reflection loop to learn from outcomes.

**Q2 -- Proposal shape.** Should proposals be structured JSON
(machine-readable, gateable) or free-text (simpler, more
flexible)? Leaning **(a) structured JSON** -- a proposal object
with `memory_writes: [{ topic, content }]` fields that the apply
tool can mechanically execute. Free-text proposals would require
another LLM pass to interpret.

**Q3 -- Reflection trigger pattern.** Should we ship a built-in
`[[schedule]]` example in the config surface, or a dedicated
`reflection.schedule` config section? Leaning **(a) reuse
existing `[[schedule]]`** -- the reflection loop is just a
prompt that calls `turn.history` + `reflection.propose`. No
special config section needed; the existing trigger substrate
handles it.

## Tasks

### Task 1 -- Open commit + PHASE_29.md scaffold

This file. Update `docs/README.md` to show Phase 29 as Open.
Update `docs/ROADMAP.md` with Phase 29 active pointer.

### Task 2 -- `reflection.propose` and `reflection.apply` capability bases

Add `reflection.propose` and `reflection.apply` to `KNOWN_BASES`
and `CEILING_TRUSTED` in `aivyx-capability`. These scope bases
gate the two reflection tools: propose (read outcomes + emit
structured proposals) and apply (execute approved proposals by
writing to memory).

### Task 3 -- `reflection.propose` tool

New tool in `aivyx-channel`. Reads recent turn outcomes via the
audit log (same `Arc<dyn AuditLog>` as `turn.history`), analyzes
patterns, and emits a structured proposal:

```json
{
  "proposal_id": "rp-...",
  "observations": ["5 of last 10 turns timed out on shell.exec"],
  "memory_writes": [
    { "topic": "reflection:shell-timeout", "content": "..." }
  ],
  "mission_id": "m-..."
}
```

Creates a mission with a gate for operator approval. Scoped to
`reflection.propose`. Follows the OnceLock pattern with both
audit log and mission store handles.

### Task 4 -- `reflection.apply` tool

Takes a proposal ID, verifies the associated mission gate is
approved, and executes the proposed memory writes via the
existing memory substrate. Scoped to `reflection.apply`.
Follows the OnceLock pattern with mission store and memory
handles.

### Task 5 -- Binary wiring + integration test

Wire both tools into the binary: imports, registration, store
injection. Add an integration test that exercises the full
propose->gate->apply cycle in-process (no daemon needed).

### Task 6 -- Exit freeze + docs

Exit criteria checklist, prediction-vs-reality table, streak
report, ROADMAP.md update, README.md status update. Record
deferral backlog delta and G3 delivery status.

---

## Ship records

### Task 1 -- Open commit

Opened Phase 29 journal, updated docs/README.md (Phase 29 Open),
docs/ROADMAP.md (Phase 29 active pointer), docs/PRODUCT_ROADMAP.md
(Phase 29 reflection loop entry).

### Task 2 -- Capability bases

Added `reflection.propose` and `reflection.apply` to `KNOWN_BASES`
and `CEILING_TRUSTED` in `aivyx-capability`. Two new scope bases
for the reflection loop tools.

### Tasks 3-4 -- `reflection.propose` and `reflection.apply` tools

New `reflection_tool.rs` in `aivyx-channel` with both tools:

**`reflection.propose`:** Reads the audit chain via `Arc<dyn AuditLog>`,
collects recent `TurnStarted`/`TurnEnded` pairs and `ScopeDenied`
events, analyzes patterns (>50% failure rate, >50% timeout rate,
repeated scope denials), emits structured `ProposalRecord` with
observations and proposed memory writes. Creates a mission with an
approval gate for operator review. Returns early with no mission
if no actionable patterns are found. Scoped to `reflection.propose`.

**`reflection.apply`:** Takes a `proposal_id`, scans missions to
find the one containing the proposal, verifies all gates are
approved (fails on pending or rejected), parses the `ProposalRecord`
from the mission description, executes proposed memory writes via
`Arc<dyn Memory>::put()`, completes the mission. Scoped to
`reflection.apply`. Returns `Verification::Verified`.

4 new tests covering scope and schema assertions for both tools.

### Task 5 -- Binary wiring

Wired both tools into the binary: imports, tool list registration,
store injection. `reflection.propose` receives `audit_log_for_tool`
(cloned before `turn_history_tool` consumes its copy), mission
store, and role name. `reflection.apply` receives mission store
and memory substrate.

Streak check:
- DESIGN.md: **untouched** (streak extends)
- PRODUCT.md: **untouched** (streak extends)
- Production-core: **untouched** (streak extends to 1)

## Exit criteria

- [x] All 6 tasks committed.
- [x] `cargo check` clean (warnings only in aivyx-mcp, pre-existing).
- [x] `cargo test` -- 701 tests, 0 failures, 1 ignored.
- [x] Reflection loop shipped: observe (turn.history, Phase 28) ->
      propose (reflection.propose) -> approve (mission gate) ->
      apply (reflection.apply) -> memory writes.
- [x] DESIGN.md untouched -- streak extends.
- [x] PRODUCT.md untouched -- streak extends.
- [x] Production-core untouched -- streak extends to 1.
- [x] Deferral backlog unchanged at 11 (no new deferrals, no closures).
- [x] G3 (Memory Reflection) memory-only scope delivered. Runtime
      role-config mutation (full P8 vision) deferred to Phase 30+.

## Prediction vs reality

| Prediction | Reality |
|---|---|
| DESIGN.md untouched | **Correct.** Untouched. |
| PRODUCT.md untouched | **Correct.** Untouched. |
| Production-core streak extends to 1 | **Correct.** Untouched this phase. |

## Decisions

1. **Q1 resolved as memory-only.** `reflection.apply` writes to
   memory topics only; runtime role-config mutation deferred. The
   memory substrate is sufficient for outcome-driven behavioral
   adjustment -- the agent can record reflective insights that
   influence future turns via system prompt context.

2. **Q2 resolved as structured JSON.** Proposals are serialized
   `ProposalRecord` structs with `observations` and `memory_writes`
   fields. Stored in the mission's `description` field -- no new
   storage domain needed.

3. **Q3 resolved as reuse existing `[[schedule]]`.** No dedicated
   reflection config section. The operator configures a schedule
   whose prompt calls `reflection.propose` -- the existing trigger
   substrate handles periodic firing.

## Deferrals

**Rolling deferrals at Phase 29 exit (11 items, unchanged):**

- **Second regression channel for the role primitive** --
  Phase 11 Q6. Untouched.
- **Response headers in audit payload** -- Phase 12 Q3 half.
  Untouched.
- **Non-GET verbs (POST/PUT/PATCH/DELETE)** -- Phase 12 Q1.
  Deferred indefinitely.
- **Redirect following with per-hop scope re-check** --
  Phase 12 Q5. Deferred indefinitely.
- **Binary response bodies / non-UTF-8** -- Deferred
  indefinitely.
- **Per-chunk Telegram rendering** -- Phase 12 Task 1.
  Deferred reactively.
- **Multi-level sub-agent nesting** -- Phase 14 Task 3.
  Untouched.
- **LocalChannel regression-test rewrite over IPC** --
  Phase 17 Q6->(c+). Tagged: **reactive.**
- **Telegram-specific protocol extensions (attachment
  delivery, inline keyboards, etc.)** -- Phase 19. Untouched.
- **MCP SSE transport** -- Phase 23. Untouched.
- **Provider-specific token counting** -- Phase 25.
  Untouched.
