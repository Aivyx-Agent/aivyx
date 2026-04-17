# Phase 23 — Escalation→Gate Wiring + MCP Foundation

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Two-part phase: (1) wire the **escalation→gate turn-loop
orchestration** in the daemon — the Phase 21 deferral that
completes P2's mission approval-gate lifecycle end-to-end;
(2) begin the **MCP client adapter** — the highest-leverage
integration identified in the Phase 22 gap analysis.

Phase 23 is a **mixed phase** — the first half closes a
product-shape deferral (P2), the second half opens a new
integration surface (MCP).

## Why now

1. **The escalation→gate wiring is the oldest targeted
   deferral.** All IPC, storage, and state-machine primitives
   are in place from Phase 21. The daemon just needs to
   orchestrate between `TurnOutcome::Escalated` and
   `mission::add_gate`. This is a small, targeted task.

2. **MCP is the highest-leverage single integration effort.**
   The `Tool` trait's shape maps near-1:1 to MCP's tool
   interface. One adapter unlocks the entire MCP ecosystem
   (Claude tools, third-party MCP servers, IDE integrations).
   The Phase 22 gap analysis and PRODUCT_ROADMAP.md milestone
   both identify this as the top priority.

3. **The contract refresh is complete.** Phase 22 updated all
   three contract documents. The roadmap is clear, the
   amendments are written, and the path forward is documented.

## Streak predictions

- **DESIGN.md** — Low risk. The amendment references are
  already in place. New MCP-related architecture may warrant
  a future amendment but is unlikely to need one in Phase 23's
  foundation work.

- **PRODUCT.md** — Not at risk. No product commitment edits
  expected.

- **Production-core `aivyx-core/src/lib.rs`** — Medium risk.
  The gate wiring operates in `daemon_server.rs` (safe). The
  MCP adapter may need `Tool` trait extensions or new types
  in `aivyx-core`. Prediction: streak **may break** at twelve
  if MCP requires core trait changes, or **extends to twelve**
  if the adapter composes against existing shapes.

## Tasks

### Task 1 — Open commit + PHASE_23.md scaffold

This file. Update `docs/README.md` to show Phase 23 as Open.

### Task 2 — Escalation→gate turn-loop wiring

Wire daemon-side orchestration between
`TurnOutcome::Escalated` and `mission::add_gate`:

- When the agent turn loop returns `TurnOutcome::Escalated`
  for a mission turn, the daemon: (1) persists a `GateRecord`
  to redb via `mission::add_gate`, (2) transitions the mission
  to `GatePending`, (3) emits `StreamEventPayload::ApprovalGate`
  to the connected frontend.
- When `FrontendMessage::ResolveGate` arrives and the gate is
  approved, the daemon starts a new turn with the approval
  context as input message.
- When rejected, the daemon transitions the mission to
  `Failed`.

Integration test: trigger an escalation in a mission turn,
verify gate creation, resolve, verify state transitions and
resume.

This closes the Phase 21 net-new deferral and completes the
P2 approval-gate lifecycle.

### Task 3+ — MCP client adapter (scope TBD at Task 2 exit)

Shape to be refined after the gate wiring lands. Expected
scope:

- New crate `aivyx-mcp` or module in `aivyx-channel`
- MCP client transport (stdio or SSE)
- Tool bridge: MCP server tools → Aivyx `Tool` trait impls
- Capability scoping: each MCP tool gets a declared scope
- Config surface: `[[mcp_server]]` entries in `aivyx.toml`
- Integration test against a mock MCP server

## Deferrals

**Rolling deferrals carried from Phase 22 (12 items):**

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
  **Targeted by Task 2.**
- **`mission.list` / `mission.status` read-only tools** —
  Phase 21. Untouched.

## Open questions

**Q1 — Should the MCP adapter live in a new crate
(`aivyx-mcp`) or as a module in `aivyx-channel`?** Leaning
(a) new crate — MCP brings its own dependencies
(jsonrpc, transport) and is conceptually a separate
integration surface.

**Q2 — Should MCP tools register as third-party tools under
P12's process model, or as a "bridge" category?** Leaning
(b) bridge — MCP tools run in-process but delegate execution
over MCP's protocol. They're not separate OS processes (P12),
but they're not in-tree tools either.

**Q3 — What MCP transport should ship first?** Leaning
(a) stdio — simplest, works with local MCP servers, matches
the most common MCP deployment pattern.
