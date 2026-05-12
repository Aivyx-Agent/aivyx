# Phase 49 — Tool Process IPC Foundation

Phase journals are working documents. They churn freely during
the phase and freeze at exit under a final Exit criteria block.
For the locked technical contract see [`../DESIGN.md`](../DESIGN.md);
for the locked product contract see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Ship the foundation for **PRODUCT.md P12** — third-party tools
run as their own OS processes, communicate with the daemon over
an IPC protocol, and are spawned + registered at daemon startup.
Closes the last remaining forward commitment in the
`PRODUCT.md` ledger.

**Foundation, not full delivery.** P12 has two halves:

1. **Third-party path: out-of-process tools.** This phase.
2. **First-party path: existing in-process substrate tools
   speak the same protocol they speak to.** Deferred to a
   future phase per Q5 — requires reshaping the `Tool` trait
   and rewiring all 8 substrate tools.

Phase 49 ships (1) cleanly and pins the protocol so (2) can land
later without amendment.

Also closes the load-bearing item in
[`THREAT_MODEL.md` § 5.6](THREAT_MODEL.md) — third-party tool
process isolation, where today every tool runs in the daemon's
address space.

## Why now

1. **Last forward commitment.** PRODUCT.md P12 is the only
   remaining unshipped commitment after Phase 48. Closing it
   completes the full P1-P12 ledger and ends the
   forward-commitment sequence.

2. **Threat-model gap.** `docs/THREAT_MODEL.md` § 5.6 was
   written admitting "every tool today runs in the daemon's
   process. A buffer overflow in a tool can in principle corrupt
   daemon memory." Phase 49 closes that gap for third-party
   tools.

3. **Phase 48 just shipped the channel SDK.** Channel SDK + tool
   SDK are the symmetric pair that together open the platform
   to outside contributions. Closing both before pivoting to
   anything else makes the SDK story coherent.

4. **MCP shows the shape works.** The existing `aivyx-mcp` crate
   already spawns subprocess tools and bridges them into the
   `Tool` trait. Phase 49 generalizes that pattern with
   Aivyx-specific guarantees (scope binding at handshake, audit
   correlation, verification semantics).

## Architecture

A third-party tool process:

1. Is spawned by the daemon at startup (`[[tool_process]]`
   config entry).
2. Performs a handshake on its stdin/stdout: receives a
   `ToolHello { protocol_version }`, replies with
   `ToolRegister { tools: [{name, description, input_schema,
   required_scope}, ...] }`.
3. Receives `InvokeTool { call_id, tool_name, input,
   turn_id }` frames per turn.
4. Streams back any number of `ToolEvent { call_id, event }`
   frames (status / chunks), then a `ToolResult { call_id,
   verified, output }` (or `ToolError { call_id, code, message }`).
5. Shuts down on `ToolShutdown` or daemon exit.

```
Operator config (aivyx.toml)
    │
    ▼
Daemon startup
    │ spawns child process
    ▼
Tool process (any language)
    │ stdin: receives ToolHello, InvokeTool, ToolShutdown
    │ stdout: sends ToolRegister, ToolEvent, ToolResult, ToolError
    │
    ▼
Length-prefixed JSON frames (same framing as daemon IPC)
```

The wire format is the **same length-prefixed JSON framing** as
the daemon IPC channel protocol (`docs/DAEMON_IPC.md`). The
transport differs — stdin/stdout instead of Unix socket — but
the framing layer is byte-identical, which means anyone who
wrote a Phase 48 channel adapter knows the shape immediately.

The daemon side lives in a new crate `aivyx-tool` (12th
workspace member, mirroring `aivyx-mcp`'s shape from Phase 23).

## Entry baseline

- Tests: 948 (Rust) + 15 (Python conformance) = 963
- Clippy warnings: 0
- Deferral backlog: 5
  (live audit push, read-write dashboard, `handle_connection`
  param-struct lift, Rust conformance harness, IPC stability
  window)
- DESIGN.md streak: 7 phases (untouched since Phase 41)
- PRODUCT.md streak: 12 phases (untouched since Phase 37)
- `aivyx-core/src/lib.rs` streak: 3 phases (untouched since Phase 45)

## Q-block — resolutions

Six load-bearing decisions, pinned at phase open:

**Q1: Wire protocol — extend daemon IPC, extend MCP, or invent a
new one?** → **New protocol mirroring daemon IPC framing.**
Length-prefixed JSON over stdin/stdout. New message types
(`ToolHello`, `ToolRegister`, `InvokeTool`, `ToolResult`,
`ToolEvent`, `ToolError`, `ToolShutdown`). The framing is
byte-identical to `daemon_ipc.rs` so authors who shipped a
Phase 48 channel adapter recognize the shape. MCP support stays
in `aivyx-mcp` — parallel adapter, not deprecated.

**Q2: Lifecycle — daemon spawns at startup, or tools register
dynamically?** → **Spawn at startup, no runtime registration.**
Operator declares `[[tool_process]]` in `aivyx.toml`. The daemon
spawns each at startup, performs the handshake, registers tools
in the global registry. Dynamic registration is rejected.
Matches the immutable-at-runtime posture of roles, schedules,
MCP servers.

**Q3: Capability scope binding — declared by tool or by
operator?** → **Tool declares, operator confirms or attenuates.**
The tool's `ToolRegister` frame announces each tool's
`required_scope`. The operator's `[[tool_process]]` block may
override scopes per-tool (attenuation only — operator can only
narrow, not widen). The daemon **rejects at handshake** any
tool whose declared scope falls outside the operator's active
role envelope. This is the integration guarantee that makes
P12 a meaningful security boundary.

**Q4: Example tool — what does it do?** → **A trivial
`wordcount` tool in Python.** Takes `{text: String}`, returns
`{words, chars, lines}`. Same posture as the Phase 48 channel
example: pure protocol demonstration, no real-world domain
bundling. Stdlib only. Reuses `examples/python-channel`'s
framing pattern with its own copy under `examples/python-tool/`.

**Q5: First-party in-process unification — in scope?** → **Out
of scope. Foundation phase only.** P12 commits that first-party
tools share the protocol but ship in-process for speed. That
needs `Tool` trait reshaping and 8 substrate tool migrations —
a separate phase. Phase 49 ships **the protocol + the third-
party path**. The roadmap name "Foundation" was chosen
deliberately.

**Q6: Stability commitment?** → **None.** v0, same posture as
Phase 48. `docs/TOOL_SDK.md` carries the explicit "v0 — subject
to change" header. Integration guarantees (capability gating,
scope rejection at handshake, audit logging, cancellation) are
committed. Wire schema stability deferred per `PRODUCT.md` P11.

## Streak predictions

| Streak target | Predicted | Notes |
|---|---|---|
| DESIGN.md | untouched (8) | Protocol extension; no D-deliverable amendment. A4 workspace-layout addendum likely (10→11→12 crates). |
| PRODUCT.md | untouched (13) | Delivers P12; doesn't amend the commitment text. |
| `aivyx-core/src/lib.rs` | untouched (4) | At risk if `Tool` trait needs new context for proxy types. Mitigation: keep trait unchanged, ship `ToolProxy` as a fresh `Tool` impl in `aivyx-tool`. |

## Workspace change

Phase 49 adds **`aivyx-tool`** as the 12th workspace crate,
mirroring `aivyx-mcp`'s shape (Phase 23). Requires an addendum
to amendment A4 (workspace layout). The crate dependency edge
is `aivyx-tool → aivyx-core, aivyx-capability` (same shape as
`aivyx-mcp`).

## Tasks

### Task 1 — Open commit + scaffold

This file. Update `docs/README.md` row to Open. Replace
`docs/ROADMAP.md` Phase 49 paragraph with the active marker.

### Task 2 — `docs/TOOL_SDK.md`

The third-party tool contract. Sections:
- Audience + scope (single-operator, OS-user auth inherited)
- Lifecycle diagram
- Handshake (`ToolHello` → `ToolRegister`)
- Message envelope (`InvokeTool` / `ToolResult` / `ToolEvent` /
  `ToolError` / `ToolShutdown`)
- Capability scope declaration + operator override semantics
- Integration guarantees (scope rejection at handshake, audit
  logging, cancellation, verification)
- v0 stability disclaimer
- Pointers to `DAEMON_IPC.md`, `CHANNEL_SDK.md`,
  `THREAT_MODEL.md`, the Python example

### Task 3 — `aivyx-tool` crate

New 12th workspace crate. Modules:
- `wire` — `ToolHello`, `ToolRegister`, `ToolDescriptor`,
  `InvokeTool`, `ToolResult`, `ToolEvent`, `ToolError`,
  `ToolShutdown`. Serde round-trip tests.
- `frame` — encode/decode length-prefixed JSON (mirrors
  `daemon_ipc.rs` framing).
- `bridge` — `ToolProcessBridge` spawns child, performs
  handshake, dispatches invocations. Lifecycle managed via
  `Drop` + `kill_on_drop` safety net.
- Errors: `ToolBridgeError` enum.

### Task 4 — `Tool` impl proxying through bridge

`ToolProxy` implements `aivyx_core::Tool`, delegates `execute`
to `ToolProcessBridge::invoke`. Captures cancellation,
surfaces `ToolOutcome::{Completed, Denied, Failed}` with proper
`Verification` mapping (the tool process declares it in
`ToolResult`). Integration tests against a faked child process.

### Task 5 — Config + daemon registration

`aivyx-config`:
- `ToolProcessConfig { name, command, args, env, scope_overrides }`
- `[[tool_process]]` TOML schema validation

Binary (`aivyx daemon run`):
- Spawn each `[[tool_process]]` at startup
- Perform handshake; on scope-out-of-envelope, log and refuse
  the tool (not the whole daemon — one broken tool process
  doesn't break the daemon)
- Register `ToolProxy` instances into `ToolRegistry`
- Shutdown — `kill_on_drop` ensures children die with the
  daemon

### Task 6 — `examples/python-tool/`

`wordcount` tool process, Python 3, stdlib only:
- `frame.py` — copy of channel example's framing (or shared)
- `tool.py` — main loop reading invocations, writing results
- `README.md` — what it does, how the operator wires it up

### Task 7 — Conformance tests

`examples/python-tool/tests/`:
- `test_handshake.py` — `ToolHello` → `ToolRegister` shape
- `test_invocation.py` — `InvokeTool` → `ToolResult` round trip
- `test_invalid_scope.py` — tool declaring an unrecognized
  scope is gracefully reported by the bridge
- `test_unknown_variant.py` — unknown wire variants graceful
  skip

### Task 8 — Exit freeze

Backfill exit stats, ship records, deferrals. Mark ROADMAP
frozen with exit commit hash. Update `docs/README.md`. Add A4
amendment addendum for the 12-crate workspace. PRODUCT.md
delivery-status update: P12 → "Fully Delivered (foundation
phase; first-party in-process unification deferred)".

## Ship records

| Task | Commit | Notes |
|---|---|---|
| 1 | `addc4eb` | scaffold |
| 2 | `a6cb144` | `docs/TOOL_SDK.md` |
| 3 | `8e3e55a` | `aivyx-tool` crate (wire + frame + bridge) — 959 tests |
| 4 | `ba8c7dd` | `ToolProxy` (aivyx_core::Tool impl) + 3 e2e tests — 963 tests |
| 5 | `584a349` | `[[tool_process]]` config + daemon spawn loop — 966 tests |
| 6 | `67cc7c8` | `examples/python-tool/` wordcount reference |
| 7 | `6673afc` | conformance suite (9 tests, real subprocess) |
| 8 | _this commit_ | exit freeze — A4 addendum + PRODUCT.md refresh |

## Deferrals carried into the phase

- Live audit push (Phase 47 Q4)
- Read-write dashboard inspection (Phase 47 Q6)
- `handle_connection` parameter-struct lift (Phase 47 Task 4)
- Conformance harness as a Rust crate (Phase 48 Q5)
- IPC stability window commitment (Phase 48 Q6)

## Net-new deferrals (predicted)

- **First-party in-process protocol unification.** Per Q5. The
  8 substrate tools currently implement `Tool` directly. P12
  commits they speak "the same protocol third-party tools
  speak" — Phase 50+ rewires them.
- **Per-tool sandboxing on top of process isolation.** Phase 49
  ships process isolation only. seccomp / containers / separate
  UIDs are valid future hardening on top, per PRODUCT.md P12
  ("a future phase may add OS-level sandboxing on top of the
  IPC isolation"). Out of scope here.

## Exit criteria

- [x] `docs/TOOL_SDK.md` exists and documents the contract.
- [x] `aivyx-tool` crate builds, has serde round-trip tests for
  every wire variant.
- [x] `ToolProcessBridge` spawns and handshakes against a
  fake child process in integration tests.
- [x] `[[tool_process]]` TOML loads + spawns at daemon startup.
- [x] `examples/python-tool/` runs and demonstrates an
  invocation end-to-end *(verified end-to-end with an inline
  harness during Task 6)*.
- [x] Conformance scenarios pass — 9/9 Python tests.
- [x] DESIGN.md A4 addendum filed for the 12-crate workspace.
- [x] PRODUCT.md Delivery Status refreshed (prediction broken
  — PRODUCT.md streak ends, see Streak outcomes).
- [x] `aivyx-core/src/lib.rs` untouched (streak → 4).
- [x] Zero clippy warnings.
- [x] Rust tests 948 → 966 (+18); Python conformance suites:
  channel 15, tool 9 (new).

## Exit stats

- Rust tests: 948 → 966 (+18: 12 aivyx-tool unit + 3 proxy_e2e
  integration + 3 aivyx-config tool_process loader tests)
- Python conformance tests: 15 (channel) → 24 (channel + tool, +9 new)
- Workspace crates: 11 → 12 (aivyx-tool added)
- Clippy warnings: 0
- Deferral backlog: 5 → 7 (two new entries below)

### Streak outcomes

| Streak target | Predicted | Actual | New streak |
|---|---|---|---|
| DESIGN.md | untouched (8) | A4 addendum filed | **0 (broken)** |
| PRODUCT.md | untouched (13) | Delivery Status refreshed (P5/P11/P12 moved to Fully Delivered, header line updated to Phase 49 exit) | **0 (broken)** |
| `aivyx-core/src/lib.rs` | untouched (4) | untouched | 4 |

Two predictions broken — both deliberate.

**DESIGN.md break:** The A4 amendment was always going to need
an addendum for the 12th crate (Phase 49 plan flagged this
explicitly). The addendum is a single Phase 49 row at the
bottom of the traceability table plus a one-paragraph note
near the top — narrow surface, contract still load-bearing.

**PRODUCT.md break:** When a phase delivers a forward
commitment, the Delivery Status section gets refreshed. Phase
35 and Phase 38 established this pattern. The header line
(*"as of Phase 38 exit"*) is necessarily wrong after Phase
49 ships the last three commitments. Honesty over streak
preservation (Phase 6 Q5).

The production-core `aivyx-core/src/lib.rs` streak held at 4 —
the at-risk prediction from the Q-block survived. All Phase
49 code lives in the new `aivyx-tool` crate plus
`aivyx-config` and the binary; the `Tool` trait was
deliberately not reshaped per Q5.

### Net-new deferrals carried forward

1. **First-party in-process protocol unification.** Per Q5.
   The 8 substrate tools (`fs.read`, `fs.write`, `memory.*`,
   `shell.exec`, `web.fetch`, `web.post`) currently implement
   `Tool` directly; P12 commits they speak "the same protocol
   third-party tools speak." A future phase rewires them
   without bumping the API contract.
2. **Per-tool sandboxing on top of process isolation.** Phase
   49 ships process isolation only. seccomp / containers /
   separate UIDs are valid future hardening on top, per
   `PRODUCT.md` P12 ("a future phase may add OS-level
   sandboxing on top of the IPC isolation"). Out of scope
   here.

### Operator-side verification still pending

The Python tool example has a documented manual smoke-test
recipe in `examples/python-tool/README.md` (add to
`aivyx.toml`, run `aivyx daemon run`, ask the agent to call
the tool). Not run in-phase. Recommended for the first
operator who picks up Phase 49's work.

### Forward commitments status

`PRODUCT.md` forward-commitment ledger after Phase 49:

- ~~P1~~ ✓ (Phase 14, 33)
- ~~P2~~ ✓ (Phases 21, 23, 28, 35)
- ~~P4~~ ✓ (Phases 16–20)
- ~~P5~~ ✓ (Phase 48)
- ~~P6~~ ✓ (always — construction)
- ~~P7~~ ✓ (Phases 11, 13)
- ~~P8~~ ✓ (Phases 28–30)
- ~~P9~~ ✓ (Phase 13)
- ~~P10~~ ✓ (always; A5 at Phase 38)
- ~~P11~~ ✓ (Phases 48, 49)
- ~~P12~~ ✓ (Phase 49 foundation)
- ~~P3~~ ✓ vision document, all seven goals (G1–G7) shipped

**The forward-commitment ledger is closed.** Future phases
work against deferrals, hardening, and post-P12 refinements
rather than against new product-shape commitments.
