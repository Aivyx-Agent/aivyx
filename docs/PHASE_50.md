# Phase 50 — P12 Closeout: First-Party In-Process Protocol Unification

Phase journals are working documents. They churn freely during
the phase and freeze at exit under a final Exit criteria block.
For the locked technical contract see [`../DESIGN.md`](../DESIGN.md);
for the locked product contract see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Close out **PRODUCT.md P12** — the substrate-and-third-party
symmetry the foundation phase deliberately deferred. P12's
text:

> First-party tools special-case in-process for speed. The
> eight substrate tools from P10 ship in-process in the
> daemon for latency reasons. **They speak the same protocol
> third-party tools speak — the protocol is the contract —
> but they bypass the IPC hop.** This means a substrate tool
> can be extracted into a separate process later (or vice
> versa) without rewriting it.

Phase 49 shipped the third-party path and the protocol; what
remains is making the *"extractable without rewriting"* clause
a verifiable property. Phase 50 does three things:

1. Wires the two Phase 49 deferred bridge stubs
   (`ToolEvent` → channel relay; per-call cancellation).
2. Adds a generic harness — `run_tool_as_subprocess<T: Tool>` —
   that wraps any `aivyx_core::Tool` into a tool process binary.
3. Proves the property by extracting `FsReadTool` out-of-process
   and asserting the two paths produce identical `ToolOutcome`.

This is the first phase of **Chapter A — Foundation Closeout**
(Phases 50–54). Chapter A's job is to pay down the deferral
backlog left at Phase 49 exit and close every loose end in the
Phase 0–49 arc before the project pivots to its next chapter.

## Why now

1. **Forward-commitment ledger is closed.** All twelve
   `PRODUCT.md` commitments shipped at Phase 49. The honest
   completion of P12 is the natural first move.
2. **Two clearly-named bridge TODOs sit in `aivyx-tool`.**
   `bridge.rs:reader_loop` silently drops `ToolEvent` frames;
   `proxy.rs:execute` admits per-call cancellation is
   best-effort. Both are explicit Phase 49 deferrals.
3. **The "extractable" property is the load-bearing P12
   guarantee.** Without it, the contract reads as a hopeful
   claim rather than a verifiable one.

## Architecture

Today (Phase 49 foundation):

```
in-process:    Tool::execute(input, context) → ToolOutcome
out-of-process: ToolProxy → ToolProcessBridge → child stdio
                                                    │
                                                    ▼
                                              tool.execute()
                                              (in the child's
                                               own `Tool` impl)
```

After Phase 50:

```
Same picture, plus:
  - bridge relays ToolEvent (Status/OutputChunk/Log) back
    to channel.stream_event via the proxy
  - proxy sends CancelInvocation{call_id} when its
    context.cancellation fires
  - run_tool_as_subprocess<T: Tool>(tool) wraps any Tool
    impl into a child-process binary that speaks the wire
    protocol; the child's `tool.execute()` runs against
    a synthesized in-process ToolContext, and its result
    is serialized back to the bridge
```

The `Tool` trait shape does not change. `run_tool_as_subprocess`
is a free function, not a trait extension. This protects the
`aivyx-core/src/lib.rs` streak.

## Entry baseline

- Rust tests: 966
- Python conformance tests: 24 (15 channel + 9 tool)
- Workspace crates: 12
- Clippy warnings: 0
- Deferral backlog: 7
- DESIGN.md streak: 1 phase (A4 addendum at Phase 49)
- PRODUCT.md streak: 1 phase (Delivery Status refresh at Phase 49)
- `aivyx-core/src/lib.rs` streak: 5 phases (untouched since Phase 45)

## Q-block — resolutions

**Q1: Reshape `Tool` trait, or wrap it?**
→ **Wrap.** No change to `aivyx_core::Tool` or `ToolContext`.
`run_tool_as_subprocess` is a free function in `aivyx-tool` that
takes any `Tool` impl and serves it as a tool process. This
preserves the `aivyx-core/src/lib.rs` streak and keeps Phase 50
a purely additive change to `aivyx-tool`.

**Q2: Which substrate tool for the proof-of-concept?**
→ **`FsReadTool`.** Read-only, narrow `fs.read:<glob>` scope,
no channel-streaming side effects, deterministic output given
the same input file. The simplest substrate tool to drive
through a subprocess and assert equality. `ShellExecTool` would
exercise more of the protocol (streaming output via
`ToolEventPayload::OutputChunk`) but is the wrong choice for
the **canonical** proof — too many moving parts.

**Q3: Where does the proof binary live?**
→ **As a fixture inside the `aivyx-tool` test crate**, not as
a permanent example or substrate binary. Phase 50 is about
*demonstrating the property*, not shipping a runnable extracted
tool. The fixture is compiled as a test binary and spawned by
the conformance test.

**Q4: `aivyx-core/src/lib.rs` streak — at risk?**
→ **Protected by Q1.** No trait changes. Risk is that wiring
the `ToolEvent` relay requires `ToolProxy` to remember a
`channel` reference across `execute` calls. Mitigation: stash
the relay channel in a per-invocation parameter passed to
`bridge.invoke`, not on the proxy. The proxy stays
substrate-shaped; the bridge gains a slightly richer invoke
signature, internal to `aivyx-tool`.

**Q5: `Tool` trait stability — at risk?**
→ **No deliberate change.** Phase 50 is a strict addition to
the contract surface. The Phase 49 stability disclaimer
(`v0 — subject to change`) carries forward unchanged.

**Q6: Stability commitment for the protocol after Phase 50?**
→ **Still v0.** Phase 50 closes the protocol's *completeness*
(the bridge handles every wire variant; the harness proves the
property). API stability is a separate, later commitment per
`PRODUCT.md` P11.

## Streak predictions

| Streak target | Predicted | Notes |
|---|---|---|
| DESIGN.md | untouched (2) | No D-deliverable change. |
| PRODUCT.md | **break (0)** | Delivery Status updated for P12 ("foundation phase" → "fully delivered"). Honest break, same pattern as Phase 35/38/49. |
| `aivyx-core/src/lib.rs` | untouched (6) | Protected by Q1. Risk acknowledged in Q4 with mitigation. |

The production-core streak is the load-bearing one to protect.
PRODUCT.md is *expected* to break — closing P12 is the whole
point.

## Tasks

### Task 1 — Open commit + scaffold

This file. Update `docs/README.md` row to Open. Add Phase 50
entry to `docs/ROADMAP.md`.

### Task 2 — Bridge → channel `ToolEvent` relay

`aivyx-tool/src/bridge.rs::reader_loop` today drops
`ToolToDaemon::ToolEvent` frames with a TODO. Phase 50 wires
them through:

- `ToolEventPayload::Status { status }` → `StreamEvent::Status { status }`
- `ToolEventPayload::OutputChunk { chunk }` → `StreamEvent::ToolOutput { chunk, .. }`
- `ToolEventPayload::Log` → `eprintln!` (or daemon log, TBD)

Requires the bridge `invoke` API to take a per-call channel
reference (`&dyn ChannelContext`) or a sink callback. Internal
to `aivyx-tool` — no `aivyx-core` changes.

### Task 3 — Per-call cancellation

`ToolProxy::execute` today admits `// Best-effort: the bridge
does not currently expose the auto-generated call_id to the
caller`. Phase 50 lifts this:

- `bridge.invoke` exposes the `call_id` (either as a return
  value alongside the outcome, or via a small `InvocationHandle`)
- On `context.cancellation` firing mid-invocation, the proxy
  sends `CancelInvocation { call_id }` over the bridge before
  bailing
- The bridge's pending-call map sweeps the slot on cancellation

### Task 4 — `run_tool_as_subprocess` generic harness

A free function in `aivyx-tool` (probably in a new `harness`
module):

```rust
pub async fn run_tool_as_subprocess<T: Tool>(tool: T) -> !
```

Reads `ToolHello` from stdin; writes `ToolRegister` with the
tool's single descriptor; loops on `InvokeTool` →
`tool.execute()` → `ToolResult`. Synthesizes a minimal
`ToolContext` for the in-child execute call (the child has no
channel or audit; the parent bridge does the audit; the channel
relay is via `ToolEvent` frames).

### Task 5 — `FsReadTool` in-process vs subprocess conformance

The canonical demonstration. Builds a tiny test-binary
fixture that calls `run_tool_as_subprocess(FsReadTool)`.
Conformance test:

1. Set up a temp file with known content.
2. Run `FsReadTool::execute(input, ctx)` in-process. Capture
   the `ToolOutcome`.
3. Spawn the fixture via `ToolProcessBridge`. `invoke()` with
   the same input. Capture the outcome.
4. Assert the two `ToolOutcome` values are equivalent
   (same `output`, same `verified`).

### Task 6 — Contract updates

- `docs/TOOL_SDK.md` gains a new section: **"First-party tools
  speak this protocol too — extractable without rewriting."**
  Points at `run_tool_as_subprocess` as the load-bearing proof.
- `PRODUCT.md` Delivery Status entry for P12 updated:
  - "Foundation phase — third-party path complete; first-party
    in-process unification deferred" →
  - "Fully delivered. Phase 49 shipped the third-party path and
    the wire protocol; Phase 50 closed first-party symmetry by
    proving the `run_tool_as_subprocess` round-trip."

### Task 7 — Exit freeze

Backfill exit stats, ship records, deferral list. Mark ROADMAP
frozen. Update `docs/README.md`. Backlog 7 → 5 (closes Phase 49
deferral #1 first-party unification, plus the ToolEvent relay
+ per-call cancel deferrals from Phase 49 bridge code).

## Ship records

| Task | Commit | Notes |
|---|---|---|
| 1 | `7c6132e` | scaffold |
| 2 | `f2104b0` | ToolEvent relay + per-call cancellation (Tasks 2 + 3 fused) — 968 tests |
| 4 | `305a392` | `run_tool_as_subprocess<T: Tool>` harness — 972 tests |
| 5 | `d87842e` | P12 equivalence proof: `FsReadTool` in-process == subprocess — 973 tests |
| 6 | _this commit_ | Contract updates: TOOL_SDK.md §8.5 + PRODUCT.md P12 → Fully Delivered |

## Deferrals carried into the phase

1. Live audit push (P47 Q4)
2. Read-write dashboard inspection (P47 Q6)
3. `handle_connection` parameter-struct lift (P47 T4)
4. Conformance harness as a Rust crate (P48 Q5)
5. IPC stability window commitment (P48 Q6)
6. First-party in-process protocol unification (P49 Q5) ← **closing this**
7. Per-tool sandboxing on top of process isolation (P49)

## Net-new deferrals (predicted)

- None expected. Phase 50 is closing existing deferrals, not
  opening new architectural surface. If the harness reveals
  protocol gaps that need follow-up work, they will be recorded
  at exit.

## Exit criteria

- [ ] `ToolEvent::{Status, OutputChunk, Log}` relay through the
  bridge, observable from the channel.
- [ ] `CancelInvocation { call_id }` sent when the proxy's
  cancellation fires mid-invocation.
- [ ] `run_tool_as_subprocess<T: Tool>` lives in `aivyx-tool`.
- [ ] Conformance test: `FsReadTool` in-process and subprocess
  paths produce identical `ToolOutcome`.
- [ ] `docs/TOOL_SDK.md` documents the first-party symmetry.
- [ ] `PRODUCT.md` Delivery Status reflects P12 fully delivered.
- [ ] DESIGN.md untouched (streak → 2).
- [ ] `aivyx-core/src/lib.rs` untouched (streak → 6).
- [ ] Zero clippy warnings.
- [ ] Rust tests net-positive.

## Exit stats

_To fill at exit._
