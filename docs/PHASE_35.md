# Phase 35 — P2 Completion + Delivery Status Refresh

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Close the last gap in PRODUCT.md P2 — Session Legibility and
Two Success Modes — by wiring `ToolOutcome::RequiresEscalation`
through the turn loop to `TurnOutcome::Escalated`, activating
the daemon's existing gate-creation code. Then refresh
PRODUCT.md's Delivery Status section from "Phase 21 exit"
to reflect the 14 phases of implementation since.

## Why now

1. **P2 is the only partially-delivered product commitment.**
   `mission.list`/`mission.status` shipped in Phase 28.
   Escalation→gate wiring is the sole remaining gap. Every
   other commitment is either fully delivered or not yet
   started (SDK phases).

2. **The infrastructure is already built.** Phase 21 built the
   mission state machine and gate model. Phase 23 wired
   daemon-side gate creation from `TurnOutcome::Escalated` +
   `mission_id` and gate resolution with turn resumption. The
   trigger path (Phase 27) already defers to "the normal gate
   path." The only missing piece is the turn loop producing
   `Escalated` instead of feeding `RequiresEscalation` back
   to the LLM.

3. **The Delivery Status section is 14 phases stale.** P8 is
   shipped (Phases 28–30), MCP Integration is complete (Phases
   23–24, 32), Multi-Provider is shipped (Phase 25), Scheduled
   Execution is shipped (Phases 26–27), P1 multi-level nesting
   is shipped (Phase 33). The contract's self-audit surface
   should reflect reality.

## Analysis of the gap

### Turn loop

`ConcreteAgent::turn` in `crates/aivyx-core/src/agent.rs`:

- `LoopOutcome` enum (line 329) has `Completed`, `Cancelled`,
  `TimedOut`, `MaxStepsExceeded` — no `Escalated` variant.
  The doc-comment at line 327 explicitly defers it.

- When a tool returns `ToolOutcome::RequiresEscalation`, the
  LLM planner (`llm_planner.rs:453`) converts it to a JSON
  error message and feeds it back as a `ToolResult` with
  `is_error: true`. The loop does **not** break.

- `TurnOutcome::Escalated` (lib.rs:348) exists with `reason`,
  `pending_tool`, `tool_calls_made` fields but is never
  constructed anywhere in the turn loop.

### Daemon server

`crates/aivyx-channel/src/daemon_server.rs` lines 263–321
already handle `TurnOutcome::Escalated`:

- When `(&outcome, &mid, &mission_store)` matches
  `(Escalated { reason, .. }, Some(mission_id), Some(store))`,
  it creates a gate via `mission::add_gate`, persists it,
  and emits `StreamEventPayload::ApprovalGate`.

- When `mission_id` is `None`, the escalation is silently
  ignored — the `TurnComplete` message goes out without a
  gate.

### Trigger path

`crates/aivyx-channel/src/trigger.rs` line 166: escalation
leaves the mission in `Running` state, expecting "the normal
gate path" to create the gate. But since the turn loop never
produces `Escalated`, trigger-fired escalations are also dead.

## Streak predictions

- **DESIGN.md** -- Low risk. No architecture change. The
  escalation→gate path was designed in Phase 21.
  Prediction: **untouched** (streak at 5 from Phase 34).

- **PRODUCT.md** -- **Will be edited.** Task 4 explicitly
  refreshes the Delivery Status section. This is intentional
  (not drift) — the contract text is unchanged, only the
  status table updates.
  Prediction: **edited** (streak breaks at 4).

- **Production-core `aivyx-core/src/lib.rs`** -- Low risk.
  The `TurnOutcome::Escalated` variant already exists. The
  change is in `agent.rs` (the `LoopOutcome` enum and the
  match arm), not `lib.rs`.
  Prediction: **untouched** (streak at 3 from Phase 34).

## Tasks

### Task 1 -- Open commit + PHASE_35.md scaffold

This file. Update `docs/README.md` to show Phase 35 as Open.
Update `docs/ROADMAP.md` with Phase 35 active pointer.

### Task 2 -- LoopOutcome::Escalated + turn loop wiring

Add `Escalated { reason, pending_tool }` to the `LoopOutcome`
enum. In the turn loop's tool-dispatch match arm, when the
outcome is `ToolOutcome::RequiresEscalation`, break the loop
with `LoopOutcome::Escalated` instead of feeding the error
back to the LLM.

Map `LoopOutcome::Escalated` → `TurnOutcome::Escalated` in
the outcome translation block at the end of `turn()`.

This is a targeted change in `crates/aivyx-core/src/agent.rs`
— the existing `TurnOutcome::Escalated` type and the daemon's
gate-creation handler need no changes.

### Task 3 -- Trigger-path gate creation for escalation

The trigger path (`trigger.rs`) currently leaves escalated
missions in `Running` state and expects "the normal gate path"
to handle it. With Task 2, the turn *does* produce `Escalated`,
but the trigger path runs its own turn loop outside the daemon
server's `SubmitInput` handler — it calls `agent.turn()`
directly and doesn't go through the daemon's gate-creation
code at lines 263–321.

Wire gate creation in the trigger path: when `wrap_mission` is
true and the turn outcome is `Escalated`, create a gate on the
mission (same pattern as daemon_server.rs lines 263–321) and
log the gate ID.

### Task 4 -- PRODUCT.md Delivery Status refresh

Bring the Delivery Status section (currently "as of Phase 21
exit") up to date at Phase 35 exit:

- Move P2 from Partially Delivered to Fully Delivered (now
  that escalation→gate is wired).
- Move P8 from Forward to Fully Delivered (shipped Phases
  28–30).
- Update P3/G1–G7 sub-items with current state.
- Note MCP Integration, Multi-Provider, Scheduled Execution,
  Reflection Layer as delivered under Forward Commitment
  Candidates.
- Update the "as of" label.

### Task 5 -- Tests

- Unit test in `agent.rs`: tool returning `RequiresEscalation`
  produces `TurnOutcome::Escalated`.
- Integration test: daemon server receives `Escalated` and
  creates a gate (confirm existing daemon code activates).
- Trigger path test: escalation with `wrap_mission = true`
  creates a gate on the mission.

### Task 6 -- Exit freeze + docs

Exit criteria checklist, prediction-vs-reality table,
streak report.
