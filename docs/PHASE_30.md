# Phase 30 — Runtime Role Mutation

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

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

## Open questions

**Q1 -- Where does `RuntimeRoleOverrides` live?** Options:
  (a) In `aivyx-core` — visible to `ConcreteAgent` and `TurnPlanner`.
  (b) In `aivyx-channel` — only visible to the binary's planner
  factory closure. The planner factory is a closure, not a trait
  method, so it can capture anything.
  Leaning **(b)** — the overrides are a channel-level concern. The
  planner factory closure captures `Arc<RwLock<RoleOverrides>>` and
  reads it on each `planner_config.clone()`. No core changes needed.

**Q2 -- Scope of mutation.** Should the agent be able to:
  (a) Append to the system prompt only (safe, additive-only).
  (b) Replace the system prompt entirely (powerful but risky).
  (c) Modify the tool allowlist (add/remove tools).
  Leaning **(a + c)** — append-only for prompts (the original
  prompt is always preserved), add/remove for allowlist. Full
  replacement is deferred.

**Q3 -- Capability base name.** `role.update` (new base) or
  reuse `reflection.apply`? Leaning **(a) `role.update`** — it's
  a distinct operation from memory writes, and a separate scope
  lets operators grant reflection-to-memory without granting
  reflection-to-role-config.

## Tasks

### Task 1 -- Open commit + PHASE_30.md scaffold

This file. Update `docs/README.md` to show Phase 30 as Open.
Update `docs/ROADMAP.md` with Phase 30 active pointer.

### Task 2 -- `RuntimeRoleOverrides` + `role.update` capability

Define `RoleOverrides` struct in `aivyx-channel` with:
- `prompt_appendix: Option<String>` — appended to system prompt
- `allowlist_additions: Vec<String>` — tools to add
- `allowlist_removals: Vec<String>` — tools to remove

Add `role.update` scope to `KNOWN_BASES` and `CEILING_TRUSTED`.

### Task 3 -- `role.update` tool

New tool in `aivyx-channel`. Input:
```json
{
  "prompt_append": "When using shell.exec, prefer smaller commands.",
  "allowlist_add": ["tool_name"],
  "allowlist_remove": ["tool_name"]
}
```

Writes to the shared `Arc<RwLock<RoleOverrides>>`. Every mutation
emits an `AuditTag::ToolCall` via the normal tool execution path.
Scoped to `role.update`.

### Task 4 -- Planner factory integration

Modify the planner factory closure in the binary to read from
`RoleOverrides` on each turn. When `prompt_appendix` is set,
append it to the system prompt in the `LlmPlannerConfig`. When
allowlist changes are set, apply them to the tool allowlist.

### Task 5 -- Extend `reflection.apply` for role updates

Add `prompt_append: Option<String>` and `allowlist_changes`
fields to `ProposalRecord`. When `reflection.apply` executes
an approved proposal with these fields, it writes to the
`RuntimeRoleOverrides` in addition to memory writes.

### Task 6 -- Exit freeze + docs

Exit criteria checklist, prediction-vs-reality table, streak
report. Update PRODUCT.md P8 delivery status if warranted.
Record deferral backlog delta.
