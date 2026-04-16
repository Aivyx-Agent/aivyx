# Phase 21 — Mission Primitive (P2)

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Deliver the first concrete piece of **PRODUCT.md P2 — Mission
Primitive**: a long-running work item that survives across
process restarts, runs under a specific role's capability
envelope, and emits operator-visible approval gates as
`StreamEvent`s the channel adapter renders distinctively.

Phase 21 is a **product-shape phase** — it ships a new product
primitive and advances Product Commitment P2. It is the first
product-shape keystone since Phase 14 (Sub-Agent Role-Switching,
P1).

## Why now

1. **Both prerequisites are complete.** The daemon holds live
   state (P4, Phases 16–19) so missions can survive restarts.
   The role-config migration declares capability envelopes (P9,
   Phase 13) so missions can run under a specific role's scope.

2. **P2 is the next product-shape commitment on the critical
   path.** The product roadmap sequences P2 after P4 (daemon)
   and P9 (role-config). Both are delivered. The Mission
   Primitive milestone in `PRODUCT_ROADMAP.md` has been waiting
   for exactly this point.

3. **The deferral backlog is manageable.** Phase 20 reduced the
   rolling backlog from sixteen to ten items, none of which
   block mission work. A clean backlog means the phase can focus
   on product shape without deferral pressure.

4. **Missions are the load-bearing primitive for G5 (autonomous
   and scheduled execution).** The product contract's Goal 5
   explicitly depends on P2's mission semantics. Shipping the
   mission primitive now unblocks the goal-level commitment.

## Streak predictions

- **DESIGN.md** — Medium risk. The mission primitive may require
  a new design decision (mission storage schema, approval-gate
  protocol). Prediction: streak **may break** at twenty-one
  phases if an amendment is needed, or **extends to twenty-one**
  if the mission shape fits within existing design decisions.
- **PRODUCT.md** — Not at risk. Phase 21 advances P2, which is
  already committed. Prediction: streak extends to **nine
  consecutive phases**.
- **Production-core `aivyx-core/src/lib.rs`** — Medium risk.
  The mission primitive may need new infrastructure tool types
  or `ToolContext` extensions. Prediction: streak **may break**
  at ten phases, ending the record run.

## Tasks

### Task 1 — Phase open (shipped in prior commit)

Scaffold `docs/PHASE_21.md`. Update `docs/README.md`
phase-status table (Phase 21 → Open). Update
`docs/ROADMAP.md` Phase 21 entry.

### Task 2 — Mission primitive design (this commit)

Resolve Q1–Q3. Record decisions. Scope Tasks 3–8.

Investigated the existing composition surfaces:
- **Storage:** `KeyDomain` enum in `aivyx-storage` (5 domains,
  redb-backed, AEAD-encrypted per domain). Missions need a
  sixth domain.
- **Daemon IPC:** `FrontendMessage` (5 variants),
  `DaemonMessage` (4 variants), `StreamEventPayload`
  (5 variants). Missions need new variants in all three.
- **Agent turn loop:** `ToolOutcome::RequiresEscalation` and
  `TurnOutcome::Escalated` were forward-invested in Phase 1
  but never wired to any tool. These are the natural seam for
  approval gates.
- **Tool registration:** `Tool` trait + `ToolRegistry` in
  `aivyx-core`. Mission tools are infrastructure tools per
  PRODUCT.md P10's classification.
- **Capability scopes:** `KNOWN_BASES` in `aivyx-capability`
  (21 bases). Mission needs two new bases.

### Task 3 — `KeyDomain::Missions` + mission state model

Add `KeyDomain::Missions` variant to `aivyx-storage`. Define
the `MissionRecord` struct in a new `aivyx-channel/src/
mission.rs` module:

```
MissionRecord {
    mission_id: String,
    role_name: String,
    description: String,
    state: MissionState,       // Created | Running | GatePending | Completed | Failed | Cancelled
    gates: Vec<GateRecord>,    // history of all gates, each with resolution
    created_at: u64,           // unix millis
    updated_at: u64,
}

GateRecord {
    gate_id: String,
    reason: String,
    scope: Option<String>,     // capability scope that triggered the gate, if any
    state: GateState,          // Pending | Approved | Rejected
    created_at: u64,
    resolved_at: Option<u64>,
}
```

Storage CRUD: `create_mission`, `get_mission`,
`update_mission_state`, `list_missions`, `add_gate`,
`resolve_gate`. All via `DomainHandle` under the new
`Missions` domain.

Unit tests for serialization round-trip and state
transitions.

**Estimated streak risk:** DESIGN.md — none (storage domain
addition is plumbing). Production-core — none (model lives
in `aivyx-channel`).

## Task 3 ship record

**Files modified:**
- `crates/aivyx-storage/src/lib.rs` (+12): `KeyDomain::Missions`
  variant, `as_bytes` → `b"missions"`, `table_name` →
  `aivyx_missions_v1`, `ALL` array `[5]` → `[6]`,
  `derive_all_subkeys` `[SubKey; 5]` → `[SubKey; 6]`,
  `subkey_for` match arm, struct field `[SubKey; 5]` →
  `[SubKey; 6]`, exhaustiveness test arm, doc comment update.
- `crates/aivyx-channel/src/mission.rs` (+313, new file):
  `MissionState` (6 variants), `GateState` (3 variants),
  `GateRecord`, `MissionRecord` with `new()`, `pending_gate()`,
  `is_terminal()`. Storage CRUD: `create_mission`,
  `get_mission`, `update_mission`, `list_missions`,
  `delete_mission`. State transitions: `transition_to_running`,
  `add_gate`, `resolve_gate`, `complete_mission`,
  `cancel_mission`. 16 unit tests covering full lifecycle,
  edge cases, and serde round-trip.
- `crates/aivyx-channel/src/lib.rs` (+1): `pub mod mission`
  registration.

**Test delta:** 569 → 585 (+16).
**All three byte-identity streaks held.**

### Task 4 — `mission.create` + `mission.gate` capability scopes

Add `mission.create` and `mission.gate` to `KNOWN_BASES` in
`aivyx-capability/src/lib.rs`. Update tier ceilings:

- **Kernel:** both bases granted unqualified.
- **Trusted:** `mission.create` granted unqualified;
  `mission.gate` granted unqualified (operator can resolve
  any gate).
- **SemiTrusted:** `mission.create` as ▲ row (omitted from
  unqualified ceiling — a SemiTrusted channel can only create
  missions if the role explicitly declares the scope).
  `mission.gate` omitted entirely (⊘ row — SemiTrusted
  channels cannot resolve gates).
- **Untrusted:** both omitted (⊘).

Tests: parse round-trip, tier ceiling grants/denials for
both bases.

**Estimated streak risk:** DESIGN.md — none.
Production-core — none (capability crate is independent).

## Task 4 ship record

**Files modified:**
- `crates/aivyx-capability/src/lib.rs` (+47):
  `mission.create` and `mission.gate` added to `KNOWN_BASES`
  (23 bases total). Both added to `CEILING_TRUSTED`
  (unqualified). Both omitted from `CEILING_SEMITRUSTED`
  (⊘ — added to the doc comment's ⊘ list) and
  `CEILING_UNTRUSTED` (⊘). Kernel gets them automatically
  via `KNOWN_BASES` iteration. 5 new tests: parse round-trip,
  Kernel/Trusted/SemiTrusted/Untrusted ceiling behaviour.
  Reflexivity test extended with 2 mission scope cases.

**Test delta:** 585 → 590 (+5).
**All three byte-identity streaks held.**

### Task 5 — IPC protocol extensions

Extend the daemon IPC vocabulary:

**`StreamEventPayload` (new variant):**
```
ApprovalGate {
    mission_id: String,
    gate_id: String,
    reason: String,
    scope: Option<String>,
}
```

**`FrontendMessage` (new variant):**
```
ResolveGate {
    mission_id: String,
    gate_id: String,
    approved: bool,
}
```

**`DaemonMessage` (new variants):**
```
MissionCreated { mission_id: String }
MissionStateChanged { mission_id: String, state: String }
GateResolved { mission_id: String, gate_id: String, approved: bool }
```

Update `DaemonEnvelope` to include the new `DaemonMessage`
variants. Round-trip serialization tests for all new variants.
Update `docs/DAEMON_IPC.md` with the new message types.

**Estimated streak risk:** DESIGN.md — low (IPC extensions
are additive). Production-core — none.

## Task 5 ship record

**Files modified:**
- `crates/aivyx-channel/src/daemon_ipc.rs` (+55):
  `FrontendMessage::ResolveGate` variant,
  `DaemonMessage::MissionCreated` / `MissionStateChanged` /
  `GateResolved` variants,
  `StreamEventPayload::ApprovalGate` variant with
  `render_for_cli` implementation,
  `DaemonEnvelope` mission variants,
  round-trip test cases for all new variants (within existing
  test functions), 2 new `render_for_cli` tests.
- `crates/aivyx-channel/src/daemon_server.rs` (+3):
  `ResolveGate` stub arm in `handle_connection` (wired in
  Task 6).
- `crates/aivyx-channel/src/telegram_daemon_frontend.rs` (+10):
  `ApprovalGate` rendering arm in Telegram message builder.
- `docs/DAEMON_IPC.md` (+5): `ResolveGate`, `MissionCreated`,
  `MissionStateChanged`, `GateResolved`, `ApprovalGate`
  documented in protocol spec tables.

**Test delta:** 590 → 592 (+2).
**All three byte-identity streaks held.**

### Task 6 — `MissionCreateTool` + daemon mission registry

Implement `MissionCreateTool` as an infrastructure tool in
`aivyx-channel` (not `aivyx-core` — it needs storage access):

- `required_scope`: `Scope::parse("mission.create").unwrap()`
- `execute`: creates a `MissionRecord` in redb, returns
  `ToolOutcome::Completed` with the `mission_id`.
- The tool is registered in the binary's tool-registry
  construction, same pattern as `RoleSwitchTool`.

Daemon mission registry in `daemon_server.rs`:
- A `HashMap<String, MissionRecord>` (or similar) held at
  the daemon level (not per-connection).
- On `SubmitInput` for a mission turn, the daemon checks
  whether the mission is in `GatePending` state and rejects
  input until the gate is resolved.
- On a tool returning `ToolOutcome::RequiresEscalation`, the
  daemon: (1) persists a `GateRecord` to redb, (2) transitions
  the mission to `GatePending`, (3) emits `ApprovalGate` to
  the connected frontend, (4) the turn ends with
  `TurnOutcome::Escalated`.
- On `ResolveGate`, the daemon: (1) updates the `GateRecord`
  in redb, (2) transitions the mission back to `Running`,
  (3) if approved, starts a new turn with the approval context
  as input; if rejected, transitions to `Failed`.

Integration tests: create a mission, trigger a gate, resolve
the gate, verify state transitions.

**Estimated streak risk:** DESIGN.md — medium (the mission
registry may represent a new architectural primitive).
Production-core — medium (may need `ToolContext` extension
for storage access, or may use the same `OnceLock` factory
pattern as `RoleSwitchTool`).

## Task 6 ship record

**Files modified:**
- `crates/aivyx-channel/src/mission_tool.rs` (+189, new file):
  `MissionCreateTool` struct with `OnceLock<DomainHandle>` +
  `OnceLock<String>` for role name. `set_mission_store` and
  `set_role_name` initializers. `Tool` impl: `name()` →
  `"mission.create"`, `required_scope` → `mission.create`,
  `execute` generates UUID-based `mission_id`, creates
  `MissionRecord`, persists via `mission::create_mission`.
  `mission_create_input_schema()` helper. 2 unit tests.
- `crates/aivyx-channel/src/lib.rs` (+1): `pub mod mission_tool`
  registration.
- `crates/aivyx-channel/src/bin/aivyx.rs` (+18):
  `MissionCreateTool` import, construction alongside
  `RoleSwitchTool`, push to `tool_list`, `set_mission_store`
  with `storage.domain(KeyDomain::Missions)`,
  `set_role_name(active_role_name)`. `run_daemon` call updated
  with mission store handle.
- `crates/aivyx-channel/src/daemon_server.rs` (+45):
  `run_daemon` gains `mission_store: Option<DomainHandle>`
  parameter. `handle_connection` receives `Arc<DomainHandle>`.
  `ResolveGate` handler: loads mission from redb, calls
  `mission::resolve_gate`, persists back, sends
  `DaemonMessage::GateResolved` on success or
  `DaemonMessage::Error` on failure. `run_daemon_compat` and
  `run_single_connection_daemon` pass `None`.
- `crates/aivyx-channel/tests/daemon_roundtrip_e2e.rs` (+3):
  Three direct `run_daemon` call sites updated with trailing
  `None` argument for mission store.

**Test delta:** 592 → 594 (+2).
**All three byte-identity streaks held.**

### Task 7 — Frontend rendering (CLI + Telegram)

**CLI:** When `ApprovalGate` arrives in the REPL loop,
render a distinctive prompt:
```
[MISSION GATE] mission-abc: reason text
  Approve? [y/N]:
```
Read operator input, send `ResolveGate`.

**Telegram:** Render the gate as a message with inline
keyboard buttons (Approve / Reject). On callback, send
`ResolveGate`. (If inline keyboards are deferred, render
as a text message with `/approve mission-abc gate-xyz`
command syntax.)

**Estimated streak risk:** Production-core — none (frontend
code lives in `aivyx-channel`).

### Task 8 — Exit freeze

Standard exit procedure: deferrals block, prediction-vs-
reality, exit criteria checklist, docs flips, ROADMAP +
PRODUCT_ROADMAP updates.

## Decisions

**Decision 1 (Q1→(a)): A mission is a redb row with a state
machine.** States: `Created → Running → GatePending →
Completed | Failed | Cancelled`. The daemon drives state
transitions. Each mission is tied to a role name for envelope
lookup. This composes with the existing storage layer (new
`KeyDomain::Missions` domain) and survives restarts because
redb is persistent. Alternatives (b) and (c) were rejected:
(b) ties missions to connection lifetime, making restart
survival complex; (c) over-structures the primitive before
a concrete recursive use case exists.

**Decision 2 (Q2): Approval gates are `StreamEventPayload`
variants resolved by a `FrontendMessage`.** The gate lifecycle:
a tool returns `ToolOutcome::RequiresEscalation` → daemon
persists a `GateRecord` to redb → daemon emits
`StreamEventPayload::ApprovalGate` to the frontend → frontend
renders distinctively and collects operator decision →
frontend sends `FrontendMessage::ResolveGate` → daemon
updates redb and resumes (approved) or fails (rejected) the
mission. This reuses the Phase 1 forward-invested escalation
path that has been dormant for twenty phases.

**Decision 3 (Q3): Two new capability bases — `mission.create`
and `mission.gate`.** `mission.create` gates who can initiate
missions; `mission.gate` gates who can resolve approval gates.
Both are infrastructure tools per PRODUCT.md P10's
classification (the agent uses them to manage itself). Tier
placement: Trusted gets both unqualified, SemiTrusted gets
`mission.create` as ▲ (conditionally granted) and
`mission.gate` as ⊘ (denied), Untrusted gets neither.

**Decision 4: Gate suspension is turn-boundary, not
coroutine-based.** When a mission hits a gate, the current
turn ends with `TurnOutcome::Escalated`. The gate is
persisted to redb. When the gate is resolved, a *new* turn
begins with the approval context injected as the input
message. This avoids suspending async tasks across process
restarts — the mission is a sequence of turns with gate-checks
between them. The trade-off is that the agent loses in-flight
context at each gate boundary, but the mission record and
gate history provide the context the agent needs to resume
coherently.

**Decision 5: `MissionCreateTool` lives in `aivyx-channel`,
not `aivyx-core`.** It needs storage access (`DomainHandle`
for the `Missions` domain), which is not available through
`ToolContext`. Same architectural pattern as `RoleSwitchTool`:
an `OnceLock`-backed factory closure constructed in the
binary's startup path, capturing the storage handle. This
preserves the production-core streak if no `ToolContext`
extension is needed.

## Open questions

**Q1 — What is a mission, concretely?** → **(a), resolved
in Decision 1.**

**Q2 — What is the shape of an approval gate?** →
**Resolved in Decision 2.**

**Q3 — Does the mission primitive need new capability
scopes?** → **Yes, resolved in Decision 3.**

**Q4 — Should `MissionCreateTool` extend `ToolContext` or
use the `OnceLock` factory pattern?**

(a) Extend `ToolContext` with an optional `&dyn MissionStore`
field. Clean access pattern but breaks the production-core
streak.

(b) Use the `OnceLock` factory pattern from `RoleSwitchTool`
(Phase 14). The tool captures the storage handle at
construction time. Preserves the streak but adds another
factory closure.

**Recommendation: (b).** The factory pattern is validated
(Phase 14) and the production-core streak at nine consecutive
phases is worth preserving. Deferred to Task 6.

**Q5 — Should the Telegram gate UX use inline keyboards or
text commands?**

(a) Inline keyboards (`InlineKeyboardMarkup` with Approve /
Reject buttons). Richer UX but requires Telegram callback
query handling not yet in the codebase.

(b) Text commands (`/approve mission-abc gate-xyz`). Simpler,
works with the existing message-based Telegram adapter.

**Recommendation: (b) for Phase 21, defer (a) as a net-new
item.** Deferred to Task 7.
