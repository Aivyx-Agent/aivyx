# Phase 22 — Post-Phase-21 Contract Refresh + Gate Wiring

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../../../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../../../PRODUCT.md).

## Goal

Two-part phase: (1) the first-ever **contract amendment batch**
— four amendments to `DESIGN.md` reflecting architecture that
shipped across Phases 1–21 but was never written back into the
technical contract, plus a **Delivery Status** section for
`PRODUCT.md` and a **milestone refresh** for
`PRODUCT_ROADMAP.md`; (2) the **escalation→gate turn-loop
wiring** that completes the Phase 21 mission approval-gate
lifecycle.

Phase 22 is a **mixed phase** — the first half is a
documentation-only contract refresh (Tasks 1–7), the second
half ships code completing P2 (Task 8).

## Why now

1. **The contract gap is wide.** `DESIGN.md` was locked at
   Phase 0 (2026-04-13) and has survived 21 phases
   byte-identical. That streak proves the original contract was
   well-written, but the 10-crate, 598-test workspace has grown
   far beyond what the 8 deliverables describe. A new session
   starting from `DESIGN.md` alone would have a badly incomplete
   picture of the daemon, the mission primitive, the IPC
   protocol, and the capability taxonomy.

2. **The amendment process has never been used.** Phase 22 is
   the first exercise of the amendment mechanism described in
   `docs/README.md`. This validates the process itself.

3. **The escalation→gate deferral is the last missing piece of
   P2.** All IPC, storage, and state-machine primitives are in
   place from Phase 21. The daemon just needs to orchestrate
   between `TurnOutcome::Escalated` and `mission::add_gate`.

## Streak predictions

- **DESIGN.md** — **Will break** at twenty-two phases.
  Intentionally. The amendment inline references are the first
  edits in project history. The streak's purpose (detecting
  silent drift) is fulfilled — breaking it through the formal
  amendment process is the mechanism working as designed.
  Hash at entry: `d39c222cbd83e97109a564038db8bb44f91d7919d55c8aa8683e6e7909fc4a6b`.

- **PRODUCT.md** — **Will break** at ten phases. Intentionally.
  The Delivery Status section is additive (no existing text
  edited), but it changes the file's bytes.
  Hash at entry: `73bb0a512ae022776bbddf8de3b38b03717caece1c2e55f78acb3f35fffae1e4`.

- **Production-core `aivyx-core/src/lib.rs`** — **Low risk.**
  The contract refresh (Tasks 1–7) is docs-only. Task 8
  (gate wiring) operates in `daemon_server.rs` and
  `mission.rs` — both in `aivyx-channel`. Prediction: streak
  **extends to eleven** consecutive phases (new record).
  Hash at entry: `d8ab203fc98c89b01a3dc7bd56653132786d4875fd912cfe11b47e22085eeb77`.

## Tasks

### Task 1 — Open commit + PHASE_22.md scaffold

This file. Create `docs/amendments/` directory (first in
project history). Update `docs/README.md` to show Phase 22
as Open.

## Task 1 ship record

**Files modified:**
- `docs/PHASE_22.md` (+216, new file): phase scaffold with
  goal, streak predictions, 8-task plan, deferrals, Q-block.
- `docs/README.md` (+2, -1): Phase 22 row as Open, Phase 23+
  planned.

**Commit:** `03cb103`.

### Task 2 — Amendment A1: Daemon IPC Protocol

Create `docs/amendments/2026-04-17-daemon-ipc-protocol.md`.
Update `DESIGN.md` D1 and D3 with inline amendment
references.

Covers: the daemon execution topology (turn loop inside
daemon, channels as IPC clients), length-prefixed JSON over
Unix domain sockets, `DaemonMessage` / `FrontendMessage` /
`DaemonLifecycleEvent` envelope types, `StreamEventPayload`
as the IPC-safe mirror of `StreamEvent`, `FrontendType` enum
and `ChannelFactory` dispatch, `default_socket_path()`,
auto-spawn, graceful shutdown via `CancellationToken`.

**Estimated streak risk:** DESIGN.md — breaks (intentional).

## Task 2 ship record

**Files modified:**
- `docs/amendments/2026-04-17-daemon-ipc-protocol.md` (+167,
  new file): amendment covering daemon execution topology, IPC
  envelope types, StreamEventPayload mirror, FrontendType,
  multi-connection model, auto-spawn, lifecycle management.
- `DESIGN.md` (+11): inline amendment references at end of D1
  and end of D3.

**Commit:** `6f5ae60`.
**DESIGN.md byte-identity streak ended at 21 phases
(intentional).**

### Task 3 — Amendment A2: Mission State Machine

Create `docs/amendments/2026-04-17-mission-state-machine.md`.
Update `DESIGN.md` D1 (fifth termination condition:
`TurnOutcome::Escalated`) and D4 (`mission.create`,
`mission.gate` capability bases).

Covers: six-state machine (`Created → Running → GatePending →
Completed | Failed | Cancelled`), turn-boundary gate
suspension (Decision 4), `MissionCreateTool` with OnceLock
factory pattern, daemon `ResolveGate` handler, gate rendering
per channel.

## Task 3 ship record

**Files modified:**
- `docs/amendments/2026-04-17-mission-state-machine.md` (+109,
  new file): amendment covering six-state mission machine,
  turn-boundary gate suspension, MissionCreateTool, gate
  rendering, two new infrastructure capability bases.
- `DESIGN.md` (+9): inline amendment reference at end of D4.

**Commit:** `51b4619`.

### Task 4 — Amendment A3: Capability Taxonomy Growth

Create `docs/amendments/2026-04-17-capability-taxonomy-growth.md`.
Update `DESIGN.md` D4.

Covers: full 23-base capability inventory (was 12 at Phase 0),
infrastructure-vs-substrate tool distinction per P10,
`QualifierKind` dispatch shapes, `role.switch` scope with
`SimpleGlob` qualifier semantics, tier ceiling tables for new
bases.

## Task 4 ship record

**Files modified:**
- `docs/amendments/2026-04-17-capability-taxonomy-growth.md`
  (+130, new file): full 23-base inventory, QualifierKind
  dispatch, tier ceiling placement, three-tier tool taxonomy.

**Commit:** `cbe616c`.

### Task 5 — Amendment A4: Workspace Layout

Create `docs/amendments/2026-04-17-workspace-layout.md`.
Update `DESIGN.md` D8.

Covers: 10-crate workspace (was 7 at Phase 0),
`aivyx-telegram` crate (Phase 8), `aivyx-channel` module map
(daemon_server, daemon_client, daemon_session, daemon_ipc,
mission, mission_tool, telegram_daemon_frontend, role_envelope,
role_render, local), binary extraction pattern (~2400 line
threshold).

## Task 5 ship record

**Files modified:**
- `docs/amendments/2026-04-17-workspace-layout.md` (+113,
  new file): 10-crate workspace layout, aivyx-channel module
  map (15 modules), binary extraction pattern.
- `DESIGN.md` (+7): inline amendment reference at end of D8.

**Commit:** `36c9620`.

### Task 6 — PRODUCT.md Delivery Status section

Add a Delivery Status appendix after the existing appendix.
Maps all 12 product commitments to Fully Delivered / Partially
Delivered / Forward status with phase references. Documents
forward commitment candidates (MCP Integration, Multi-Provider,
Web UI Channel, Scheduled Execution) as identified by the
seven-dimension gap analysis.

**Estimated streak risk:** PRODUCT.md — breaks (intentional).

## Task 6 ship record

**Files modified:**
- `PRODUCT.md` (+131): Delivery Status section after appendix.
  Maps P1–P12 to Fully/Partially/Forward status. Four forward
  commitment candidates documented.

**Commit:** `2c26539`.
**PRODUCT.md byte-identity streak ended at 9 phases
(intentional).**

### Task 7 — PRODUCT_ROADMAP.md milestone refresh

Add four new milestones: MCP Integration, Multi-Provider
Support, Web UI Channel, Scheduled Execution. Mark Daemon
Migration as architecturally complete. Update Reflection
Layer with Phase 21 gate-substrate note. Revise sequencing
notes for the post-Phase-22 world.

## Task 7 ship record

**Files modified:**
- `docs/PRODUCT_ROADMAP.md` (+102, -19): four new milestones
  (MCP Integration, Multi-Provider, Web UI Channel, Scheduled
  Execution), Daemon Migration marked architecturally complete,
  Reflection Layer Phase 21 note, revised sequencing notes.

**Commit:** `e9f2da6`.

### Task 8 — Exit freeze

Standard exit procedure. Escalation→gate turn-loop wiring
carried forward as a deferral (now the oldest net-new item
from Phase 21, targeted for Phase 23).

## Deferrals

**Rolling deferrals still open after Phase 22 (12 items,
unchanged — docs-only phase):**

- **Forensic `ToolOutcome::NotInRole` variant** —
  Phase 11 Q1. Untouched.
- **Second regression channel for the role primitive** —
  Phase 11 Q6. Untouched.
- **Response headers in audit payload** — Phase 12 Q3 half.
  Untouched.
- **Non-GET verbs (POST/PUT/PATCH/DELETE)** — Phase 12 Q1.
  Deferred indefinitely.
- **Redirect following with per-hop scope re-check** —
  Phase 12 Q5. Deferred indefinitely.
- **Binary response bodies / non-UTF-8** — Deferred
  indefinitely.
- **Per-chunk Telegram rendering** — Phase 12 Task 1.
  Deferred reactively.
- **Multi-level sub-agent nesting** — Phase 14 Task 3.
  Untouched.
- **LocalChannel regression-test rewrite over IPC** —
  Phase 17 Q6→(c+). Tagged: **reactive.**
- **Telegram-specific protocol extensions (attachment
  delivery, inline keyboards, etc.)** — Phase 19. Untouched.
- **Escalation→gate turn-loop wiring** — Phase 21.
  **Targeted for Phase 23.**
- **`mission.list` / `mission.status` read-only tools** —
  Phase 21. Untouched.

**Rolling backlog: 12 → 12 (zero net change). Zero net-new
deferrals from Phase 22.**

## Prediction vs. reality

- **DESIGN.md** — Predicted: **will break** at twenty-two.
  **Reality: correct.** Broke intentionally in Task 2
  (`6f5ae60`). The streak served its purpose for 21 phases;
  the formal amendment process worked as designed.

- **PRODUCT.md** — Predicted: **will break** at ten.
  **Reality: correct.** Broke intentionally in Task 6
  (`2c26539`). Additive only — no existing text edited.

- **Production-core `aivyx-core/src/lib.rs`** — Predicted:
  streak **extends to eleven**. **Reality: correct.** Hash
  unchanged: `d8ab203fc98c89b01a3dc7bd56653132786d4875fd912cfe11b47e22085eeb77`.
  Docs-only phase — zero Rust files modified.

## Exit criteria

- [x] Four amendments created in `docs/amendments/` (first
  amendments in project history):
  - A1: Daemon IPC Protocol (D1, D3)
  - A2: Mission State Machine (D1, D4)
  - A3: Capability Taxonomy Growth (D4)
  - A4: Workspace Layout (D8)
- [x] `DESIGN.md` updated with inline amendment references at
  D1, D3, D4, D8 (four sections).
- [x] `PRODUCT.md` Delivery Status section added: 6 Fully
  Delivered, 2 Partially Delivered, 4 Forward, 4 Forward
  Candidates.
- [x] `PRODUCT_ROADMAP.md` refreshed: 4 new milestones, Daemon
  Migration marked architecturally complete, Reflection Layer
  updated, sequencing notes revised.
- [x] Three Q-block questions resolved.
- [x] Production-core streak extended to eleven consecutive
  phases (new record).
- [x] Test count unchanged: 598 (docs-only phase).
- [x] Deferrals block recorded (zero net-new).
- [x] Prediction-vs-reality block recorded (all three
  correct).

## Open questions

**Q1 — Should amendments reference specific commit hashes or
phase numbers?** → **(a), resolved in Tasks 2–5.** Phase
numbers used throughout. Commit hashes appear only in
traceability tables at the bottom of each amendment.

**Q2 — Should the PRODUCT.md Delivery Status section include
forward commitment candidates (P13+), or only status of
existing P1–P12?** → **(b), resolved in Task 6.** Included
as a "Forward Commitment Candidates" subsection, clearly
labelled as non-binding.

**Q3 — Should the DESIGN.md amendment inline references be
footnote-style or inline paragraph insertions?** → **(a),
resolved in Task 2.** Blockquote-style notes at the end of
the affected subsection, using markdown `>` prefix for visual
distinction from the original locked text.
