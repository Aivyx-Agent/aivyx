# Amendment A2 — Mission State Machine

**Date:** 2026-04-17
**Phase:** 22
**Supersedes:** Extends D1 (Turn Loop Contract, termination
conditions) and D4 (Capability Taxonomy, new bases). No text
removed — additive only.
**Implementing phase:** 21

---

## What changed

D1 names four termination conditions: final assistant message,
`RequiresEscalation`, timeout, channel cancel. D3's
`TurnOutcome` enum has five variants (the four plus `Failed`).
Phase 21 gave `TurnOutcome::Escalated` its first concrete
production semantics: **mission gate suspension**.

D4's capability taxonomy listed 12 initial scope bases. Phase
21 added two infrastructure bases (`mission.create`,
`mission.gate`) for the mission lifecycle.

---

## The mission primitive

A **mission** is a long-running operator intent that may span
multiple turns, multiple process restarts, and multiple gate
checkpoints. It is the first-class primitive for PRODUCT.md
P2's "open-ended missions with operator-approval gates."

### State machine

```
Created --> Running --> GatePending --> Completed
                  |          |
                  |          +--> Running (gate approved)
                  |          |
                  |          +--> Failed  (gate rejected)
                  |
                  +--> Completed
                  |
                  +--> Failed
                  |
                  +--> Cancelled
```

Six states: `Created`, `Running`, `GatePending`, `Completed`,
`Failed`, `Cancelled`. The daemon drives all state transitions.
Each `MissionRecord` is persisted to redb under
`KeyDomain::Missions` and survives process restarts.

### Turn-boundary gate suspension (Decision 4)

When a mission turn hits a gate, the current turn ends with
`TurnOutcome::Escalated`. This is a **turn-boundary**
mechanism, not a coroutine suspension:

1. A tool returns `ToolOutcome::RequiresEscalation`.
2. The turn loop terminates with `TurnOutcome::Escalated`.
3. The daemon persists a `GateRecord` to redb.
4. The daemon transitions the mission to `GatePending`.
5. The daemon emits `StreamEventPayload::ApprovalGate` to the
   connected frontend.
6. The frontend renders the gate distinctively and collects
   the operator's decision.
7. The operator resolves the gate via
   `FrontendMessage::ResolveGate`.
8. If approved: the daemon transitions back to `Running` and
   starts a **new turn** with the approval context as input.
   If rejected: the daemon transitions to `Failed`.

The trade-off: the agent loses in-flight context at each gate
boundary. The mission record, gate history, and memory provide
the context the agent needs to resume coherently.

### `MissionCreateTool`

An infrastructure tool (per P10's substrate/infrastructure
taxonomy) that lives in `aivyx-channel`, not `aivyx-core`.
Uses the OnceLock factory pattern established by Phase 14's
`RoleSwitchTool`: storage handle and role name injected at
binary startup time, avoiding any `ToolContext` extension.

- `name()`: `"mission.create"`
- `required_scope()`: `mission.create`
- `execute()`: creates a `MissionRecord`, persists to redb,
  returns `ToolOutcome::Completed` with the `mission_id`.

### Gate rendering per channel

- **CLI:** `[MISSION GATE] {mission_id}: {reason}` followed
  by `Approve? [y/N]:` interactive prompt.
- **Telegram:** `/approve {mission_id} {gate_id}` and
  `/reject {mission_id} {gate_id}` text commands with a
  reply-hint message.

---

## How D1 should be read after this amendment

D1's paragraph says the loop terminates "when a tool call
returns `RequiresEscalation`." That path is now exercised by
the mission gate mechanism: the escalation triggers a
persistent gate record, and the turn ends with
`TurnOutcome::Escalated`. The "fifth termination condition"
note in D3 ("`Escalated` — reason, pending tool, tool calls
made") was forward-invested at Phase 0; Phase 21 gave it
production semantics.

## How D4 should be read after this amendment

D4's capability taxonomy gains two infrastructure bases:

| Base | Category | Tier ceiling |
|---|---|---|
| `mission.create` | Infrastructure | Trusted: granted. SemiTrusted: conditional (triangle). Untrusted: denied. |
| `mission.gate` | Infrastructure | Trusted: granted. SemiTrusted: denied. Untrusted: denied. |

These are not substrate tools (not counted against P10's
seven-tool cap). They are the machinery the agent uses to
manage its own mission lifecycle per P10's infrastructure
tool category.

---

## Traceability

| Phase | What shipped | Commit |
|---|---|---|
| Phase 21 | Mission state model, capability bases, IPC extensions, MissionCreateTool, ResolveGate handler, gate rendering | `05cc349` |
