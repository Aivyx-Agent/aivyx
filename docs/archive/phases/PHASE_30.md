# Phase 30 — Runtime Role Mutation

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../../../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../../../PRODUCT.md).

## Goal

Complete **PRODUCT.md P8 — Outcome-Driven Audited Reflection**
by adding runtime role-config mutation. Phase 29 delivered the
memory-only reflection loop (observe -> propose -> approve ->
apply to memory). This phase extends `reflection.apply` to also
modify the agent's system prompt and tool allowlist at runtime,
completing the full P8 vision: the agent can observe its own
outcomes and, with operator approval, adjust its own behavior.

## Why now

1. **P8 is the last forward commitment with undelivered pieces.**
   Phase 29 delivered memory-only reflection. The remaining
   commitment is runtime role mutation — the agent modifying its
   own system prompt and tool allowlist under capability gating.

2. **The planner factory pattern makes this clean.** The planner
   is constructed per-turn via a factory closure that clones a
   config. Introducing a shared `Arc<RwLock<RoleOverrides>>`
   that the factory reads on each construction means prompt
   changes take effect on the next turn without session restart.

3. **The audit chain and gate machinery are already in place.**
   Every mutation lands in the audit chain. The gate-approval
   flow from Phase 29 ensures operator control.

## Streak predictions

- **DESIGN.md** -- Low risk. Runtime overrides are channel-level
  composition. Prediction: **untouched**.

- **PRODUCT.md** -- Possible touch to update P8 delivery status
  in the Delivery Status appendix. Prediction: **may break**.

- **Production-core `aivyx-core/src/lib.rs`** -- Moderate risk.
  The `RuntimeRoleOverrides` struct may land here if it needs to
  be visible to the `Agent` trait. Prediction: streak **may break**
  (currently at 1).

## Resolved questions

**Q1 -- Where does `RuntimeRoleOverrides` live?** Resolved **(b)**
  — in `aivyx-channel` as `role_overrides.rs`. The planner factory
  closure captures `Arc<RwLock<RoleOverrides>>` and reads it on
  each turn construction. No core changes needed.

**Q2 -- Scope of mutation.** Resolved **(a + c)** — append-only
  for prompts (the original prompt is always preserved), add/remove
  for allowlist. Full replacement deferred.

**Q3 -- Capability base name.** Resolved **(a) `role.update`** —
  separate from `reflection.apply`. Operators can grant
  reflection-to-memory without granting reflection-to-role-config.

## Tasks

### Task 1 -- Open commit + PHASE_30.md scaffold

This file. Update `docs/README.md` to show Phase 30 as Open.
Update `docs/ROADMAP.md` with Phase 30 active pointer.

**Ship record:** `f037c66`

### Task 2 -- `RoleOverrides` struct + `role.update` capability

Defined `RoleOverrides` struct in `aivyx-channel/src/role_overrides.rs`:
- `prompt_appendix: Option<String>` — appended to system prompt
- `allowlist_additions: Vec<String>` — tools to add
- `allowlist_removals: Vec<String>` — tools to remove
- `SharedRoleOverrides` type alias: `Arc<RwLock<RoleOverrides>>`
- `shared_role_overrides()` constructor

Added `role.update` scope to `KNOWN_BASES` and `CEILING_TRUSTED`
in `aivyx-capability`. 4 tests.

**Ship record:** `aa8d864`

### Task 3 -- `role.update` tool

New `RoleUpdateTool` in `aivyx-channel/src/role_update_tool.rs`.
Input: `prompt_append`, `allowlist_add`, `allowlist_remove` (all
optional, at least one required). Writes to shared
`Arc<RwLock<RoleOverrides>>` via OnceLock injection. Scoped to
`role.update`. Removals cancel pending additions. 2 tests.

**Ship record:** `dc2727d`

### Task 4 -- Planner factory integration

Both planner factory closures (daemon path in `aivyx.rs` and
in-process path in `session.rs`) now read from
`SharedRoleOverrides` on each turn construction. New
`apply_to_planner_config()` function in `role_overrides.rs`
applies prompt appendix (preserving original prompt) and
allowlist additions/removals to the per-turn `LlmPlannerConfig`
clone. `SessionConfig` gains optional `role_overrides` field.
`RoleUpdateTool` registered in binary with OnceLock injection.
5 test file updates for new field. 3 new tests.

**Ship record:** `207b4d1`

### Task 5 -- Extend `reflection.apply` for role updates

`ProposalRecord` gains optional `prompt_append: Option<String>`
and `allowlist_changes: Option<AllowlistChanges>` fields (serde
skip_serializing_if for backwards compatibility with Phase 29
proposals). `ReflectionApplyTool` accepts `SharedRoleOverrides`
via new `set_role_overrides` OnceLock setter. When an approved
proposal contains role mutation fields, the tool writes to the
shared handle in addition to executing memory writes. Output
gains `role_updated: bool` field. Binary wires shared handle
into both `role.update` and `reflection.apply`.

**Ship record:** `4359f59`

### Task 6 -- Exit freeze + docs

Exit criteria checklist, prediction-vs-reality table, streak
report. Update PRODUCT.md P8 delivery status.

## Exit criteria

- [x] `role.update` scope in KNOWN_BASES and CEILING_TRUSTED.
- [x] `RoleOverrides` struct with prompt appendix and allowlist
      add/remove fields.
- [x] `role.update` tool writing to shared state.
- [x] Planner factory reads overrides per-turn (both daemon and
      in-process paths).
- [x] `reflection.apply` writes to RoleOverrides when proposal
      contains role mutation fields.
- [x] PRODUCT.md P8 delivery status updated.
- [x] 710 workspace tests, 0 failures (701 -> 710, +9).
- [x] Deferral backlog unchanged at 11.

## Prediction vs reality

| Prediction              | Reality           | Correct? |
|-------------------------|-------------------|----------|
| DESIGN.md untouched     | Untouched         | Yes      |
| PRODUCT.md may break    | Broke (P8 status) | Yes      |
| Production-core may break | Untouched (streak 2) | No (survived) |

## Streak report

- **DESIGN.md:** untouched this phase. Streak extends (reset
  at Phase 22 amendment batch).
- **PRODUCT.md:** edited this phase (P8 and G3/G5 delivery
  status updates). Streak resets to 0.
- **Production-core `aivyx-core/src/lib.rs`:** untouched this
  phase. Streak extends to 2 (broken in Phase 28 at 18).

## New files

- `crates/aivyx-channel/src/role_overrides.rs` — RoleOverrides
  struct, SharedRoleOverrides alias, apply_to_planner_config fn.
- `crates/aivyx-channel/src/role_update_tool.rs` — RoleUpdateTool.

## Rolling deferrals at Phase 30 exit (11 items, unchanged)

No new deferrals. No closures of existing numbered deferrals.
The runtime role-config mutation was a forward commitment (P8),
not a numbered deferral item. P8 is now fully delivered.
