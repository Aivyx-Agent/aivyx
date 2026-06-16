# Phase 23 — Escalation→Gate Wiring + MCP Foundation

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../../../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../../../PRODUCT.md).

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

## Task 2 ship record

**Design decision:** `mission_id: Option<String>` added to
`FrontendMessage::SubmitInput` rather than `TurnOutcome::Escalated`
in aivyx-core, keeping all changes in `aivyx-channel` and
preserving the production-core byte-identity streak at twelve.

**Files modified:**
- `crates/aivyx-channel/src/daemon_ipc.rs`: added `mission_id:
  Option<String>` to `SubmitInput` variant with `#[serde(default)]`
  for backward compatibility. Updated two test constructions.
- `crates/aivyx-channel/src/daemon_server.rs`: wired escalation→gate
  creation in `SubmitInput` handler (load mission, `add_gate`,
  persist, emit `ApprovalGate`). Wired gate resume in `ResolveGate`
  handler (on approved, start new turn with approval context).
  Renamed `_session_id` to `session_id` for resume use.
- `crates/aivyx-channel/src/daemon_client.rs`: extracted
  `send_and_collect` helper, added `submit_input_for_mission`
  method. Existing `submit_input` passes `mission_id: None`.
- `crates/aivyx-channel/tests/daemon_roundtrip_e2e.rs`: added
  `FakeEscalatingAgent` (returns `Escalated` on first turn,
  `Completed` on subsequent), two integration tests:
  `escalation_gate_wiring_approve_resumes_turn` (full lifecycle)
  and `escalation_gate_wiring_reject_fails_mission`.
- `docs/DAEMON_IPC.md`: documented `SubmitInput.mission_id` field,
  three new error codes, escalation→gate turn-loop wiring section.

**Test delta:** +2 (598 → 600).
**Production-core streak:** extends to twelve (hash unchanged).

### Task 3 — MCP client adapter foundation

New crate `aivyx-mcp` (11th workspace member): stdio transport,
JSON-RPC 2.0 framing, MCP protocol messages (`initialize`,
`tools/list`, `tools/call`), tool bridge (`McpToolProxy` implements
`Tool` trait), `McpServerBridge` spawns a child process and manages
the connection lifecycle.

Key decisions:
- **(a) New crate** — MCP brings its own protocol and dependency
  surface; conceptually separate from channel adapters.
- **(b) `mcp.call` scope base** — added to `KNOWN_BASES` in
  `aivyx-capability`. Qualifier: `<server>:<tool>` (e.g.,
  `mcp.call:github:create_issue`). Trusted-tier ceiling only.
- **(c) Stdio transport** — spawns MCP server as child process,
  newline-delimited JSON-RPC over stdin/stdout. No external MCP
  SDK dependency.
- **(d) Bridge pattern** — `McpServerBridge::start(cmd, args, name)`
  → `initialize` + `notifications/initialized` → `discover_tools()`
  returns `Vec<Arc<dyn Tool>>`.
- **(e) Config deferred** — `[[mcp_server]]` TOML entries deferred.
  Programmatic API only for now.

## Task 3 ship record

**Files created:**
- `crates/aivyx-mcp/Cargo.toml`: new crate, depends on `aivyx-core`
  and `aivyx-capability`.
- `crates/aivyx-mcp/src/lib.rs`: crate root, re-exports
  `McpToolProxy` and `McpServerBridge`.
- `crates/aivyx-mcp/src/jsonrpc.rs`: minimal JSON-RPC 2.0
  `Request`/`Response`/`RpcError` types.
- `crates/aivyx-mcp/src/protocol.rs`: MCP protocol message types
  (`InitializeParams`, `McpToolDef`, `ToolsCallParams`,
  `ToolsCallResult`, `ContentBlock`).
- `crates/aivyx-mcp/src/transport.rs`: `McpServerBridge` —
  spawns child process, stdio transport, `initialize` +
  `tools/list` + `tools/call` + `discover_tools` + `shutdown`.
- `crates/aivyx-mcp/src/proxy.rs`: `McpToolProxy` — one `Tool`
  trait impl per discovered MCP tool, delegates `execute` over
  stdio JSON-RPC to the server process.
- `crates/aivyx-mcp/tests/mock_mcp_server.py`: mock MCP server
  (Python) implementing two tools (`echo`, `add`).
- `crates/aivyx-mcp/tests/mcp_bridge_e2e.rs`: 8 integration tests
  covering discovery, listing, calling, error handling, trait
  compliance, and full `execute` through `ToolContext`.

**Files modified:**
- `Cargo.toml`: added `crates/aivyx-mcp` to workspace members.
- `crates/aivyx-capability/src/lib.rs`: added `mcp.call` to
  `KNOWN_BASES` and `CEILING_TRUSTED`.

**Test delta:** +8 (600 → 608).
**Production-core streak:** extends to thirteen (hash unchanged).
**Zero-new-dep streak:** holds (no new workspace-level
dependencies — `aivyx-mcp` uses only workspace deps).

## Prediction vs. reality

- **DESIGN.md** — Predicted: **low risk**.
  **Reality: correct.** Hash unchanged:
  `629b12e6e800f54f5d2f0874b0492540314a57fdf1b664e65218eb5665ae427a`.
  No amendment needed for foundation-level MCP work.

- **PRODUCT.md** — Predicted: **not at risk**.
  **Reality: correct.** Hash unchanged:
  `478cab6aa07ec94b49c1bfdf17619568cc66d6ccd8ca98dd93ef97de9a3ea1cf`.

- **Production-core `aivyx-core/src/lib.rs`** — Predicted:
  streak **may break** at twelve if MCP requires core trait
  changes, or **extends to twelve** if it composes against
  existing shapes. **Reality: extends to thirteen** — both
  Task 2 (gate wiring in `aivyx-channel`) and Task 3 (MCP
  adapter in `aivyx-mcp`) composed entirely against the
  existing `Tool` trait without modification. The open-doc
  prediction was conservative; the trait's `required_scope`
  + `execute` surface proved sufficient for both a daemon
  orchestration change and a new protocol bridge.
  Hash unchanged:
  `d8ab203fc98c89b01a3dc7bd56653132786d4875fd912cfe11b47e22085eeb77`.

## Deferrals

**Rolling deferrals at exit (11 items, -1 closed):**

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
- **`mission.list` / `mission.status` read-only tools** —
  Phase 21. Untouched.

**Net-new deferrals from Phase 23 (2 items):**

- **MCP config surface (`[[mcp_server]]` in `aivyx.toml`)** —
  Phase 23 Task 3 decision (e). Programmatic API shipped;
  TOML config deferred.
- **MCP SSE transport** — Phase 23 Q3. Stdio shipped first;
  SSE transport for remote MCP servers deferred.

**Rolling backlog: 12 → 13 (−1 closed, +2 net-new).**

## Exit criteria

- [x] Escalation→gate turn-loop wiring shipped (Task 2,
  `da0f6e6`): `SubmitInput.mission_id` → escalation creates
  gate → `ApprovalGate` emitted → `ResolveGate` approved
  resumes turn / rejected fails mission.
- [x] Phase 21 escalation→gate deferral closed.
- [x] MCP client adapter foundation shipped (Task 3,
  `83ef47e`): new `aivyx-mcp` crate, stdio transport,
  `McpServerBridge` + `McpToolProxy`, `mcp.call` scope base.
- [x] `mcp.call` added to `KNOWN_BASES` and
  `CEILING_TRUSTED` in `aivyx-capability`.
- [x] 11-crate workspace (was 10).
- [x] Production-core streak extends to thirteen consecutive
  phases (new record).
- [x] Test count: 598 → 608 (+10, across two tasks).
- [x] Three Q-block questions resolved.
- [x] Prediction-vs-reality block recorded (all three
  correct, production-core prediction conservative).
- [x] Deferrals block recorded (−1 closed, +2 net-new).

## Open questions

**Q1 — Should the MCP adapter live in a new crate
(`aivyx-mcp`) or as a module in `aivyx-channel`?** →
**(a), resolved in Task 3.** New crate. MCP brings its own
protocol types and is conceptually a separate integration
surface. No dependency on `aivyx-channel`.

**Q2 — Should MCP tools register as third-party tools under
P12's process model, or as a "bridge" category?** →
**(b), resolved in Task 3.** Bridge category. `McpToolProxy`
implements the `Tool` trait directly, running in-process but
delegating execution over stdio JSON-RPC. The `mcp.call`
scope base distinguishes them from native tools.

**Q3 — What MCP transport should ship first?** →
**(a), resolved in Task 3.** Stdio. Spawns MCP server as
child process, newline-delimited JSON-RPC over stdin/stdout.
SSE transport deferred.
