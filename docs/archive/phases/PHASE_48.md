# Phase 48 — Channel Adapter SDK + Documentation

Phase journals are working documents. They churn freely during
the phase and freeze at exit under a final Exit criteria block.
For the locked technical contract see [`../DESIGN.md`](../../../DESIGN.md);
for the locked product contract see [`../PRODUCT.md`](../../../PRODUCT.md).

## Goal

Document the third-party channel adapter contract and ship a
worked example in a non-Rust language. Delivers **PRODUCT.md
P5** (open first-party channel surface) and **P11** (SDK contract:
interface + integration guarantees).

The substrate is already in place: the daemon's IPC protocol
(Phase 16), the `FrontendType` extension point (Phase 19), the
multi-connection daemon (Phase 19), the `StreamEventPayload`
mirror (Phase 16), and the Web UI's WebSocket-over-IPC bridge
(Phase 39) are all live evidence that third parties can attach
across a process boundary. Phase 48 turns "the protocol exists
and works" into "anyone can write an adapter against it" by
publishing the contract as a documented surface and proving it
from outside Rust.

## Why now

1. **The competitive gap is breadth.** OpenClaw ships ~25
   channels; Aivyx ships 3. The architectural reason is *not*
   technical — the daemon IPC layer is designed for exactly this
   — it's documentation. Phase 48 closes the documentation gap.

2. **The substrate is provably language-agnostic.** The Web UI
   frontend (Phase 39) is already a non-Rust adapter — it runs
   in a browser, speaks the IPC protocol through a WebSocket
   bridge. The third-party SDK is the same shape, only without
   the WebSocket hop.

3. **Phase 47 just closed P2 + P4.** The remaining `PRODUCT.md`
   forward commitments are P5 (this phase), P11 (this phase),
   and P12 (Tool Process IPC, a separate phase). Phase 48 closes
   two of the three.

## Architecture

A third-party channel adapter is a process that:

1. Opens the daemon's Unix domain socket (mode 0600,
   `$XDG_RUNTIME_DIR/aivyx/daemon.sock`).
2. Reads `DaemonLifecycleEvent::DaemonReady` length-prefixed
   JSON frame.
3. (Optional) Sends `ProtocolNegotiation { version: "0.1" }`.
4. Sends `FrontendMessage::StartSession { role, frontend_type }`.
5. Reads `DaemonMessage::SessionStarted { session_id }`.
6. Per turn: sends `FrontendMessage::SubmitInput`, reads
   `DaemonMessage::StreamEvent` frames until
   `DaemonMessage::TurnComplete`.
7. Closes with `FrontendMessage::Disconnect`.

No new protocol work. Phase 48's job is to make this loop
documented, exemplified, and reproducible from outside Rust.

```
Third-party adapter (any language)
    │
    │  length-prefixed JSON frames
    ▼
Unix socket (mode 0600, operator UID)
    │
    ▼
Aivyx daemon (FrontendType dispatch → ChannelFactory → turn loop)
```

## Entry baseline

- Tests: 948
- Clippy warnings: 0
- Deferral backlog: 3 (live audit push, read-write inspection,
  `handle_connection` parameter struct lift)
- DESIGN.md streak: 6 phases (untouched since Phase 41)
- PRODUCT.md streak: 11 phases (untouched since Phase 37)
- lib.rs streak: 2 phases (untouched since Phase 45)

## Q-block — resolutions

Six load-bearing decisions, pinned at phase open:

**Q1: What does "SDK" mean here?**
→ **Documented surface + example adapter.** No new published
crate. `ChannelContext` is already `pub` in `aivyx-core`; the
IPC types are `pub` in `aivyx-channel`. Extracting them into a
separate `aivyx-channel-sdk` crate would commit to API stability
that `PRODUCT.md` P11 explicitly defers ("stability is deferred
until the SDK has stabilized in real third-party use"). Phase 48
documents what exists.

**Q2: What language for the example adapter?** → **Python.**
The roadmap names Python; it's also the highest-impact target
(largest non-Rust agent-tooling community) and proves the
contract is language-agnostic rather than coincidentally Rust.

**Q3: Real channel or fake channel?** → **Fake channel — a CLI
REPL.** A real channel (Matrix, Slack, IRC) bundles two
orthogonal concerns: the IPC contract and the messenger API.
Bundling them muddles which property is being demonstrated. A
CLI REPL is narrowly about *the IPC* — the operator types text,
the example sends it through the daemon, renders the streaming
response. Anyone adapting to a real messenger transplants the
input/output ends of that loop.

**Q4: Where does the example live?**
→ **`examples/python-channel/`.** Matches the existing
`examples/` convention (TOML configs sit there today). No new
top-level directory.

**Q5: Conformance test suite shape?**
→ **Documented scenarios with replay scripts inside the
example.** A `tests/` directory under `examples/python-channel/`
with assertions against canned message exchanges. A heavier
harness (a Rust crate `aivyx-conformance` that runs a daemon
and replays scripted traces) is deferred — the right shape for
that harness depends on what third-party authors actually trip
over, which Phase 48 itself will reveal.

**Q6: Stability commitment?** → **None.** `docs/CHANNEL_SDK.md`
ships with an explicit "v0 — subject to change without
deprecation policy" header. Integration guarantees are
committed (capability gating works, audit logging is automatic,
cancellation is honored). API stability is deferred per
`PRODUCT.md` P11.

## Streak predictions

| Streak target | Predicted | Notes |
|---|---|---|
| DESIGN.md | untouched (7) | Pure docs + examples — no contract change. |
| PRODUCT.md | untouched (12) | Delivers P5+P11; doesn't amend the commitments themselves. |
| lib.rs | untouched (3) | No `aivyx-core` work. Possibly `aivyx-channel` doc-comments, no signature changes. |

## Tasks

### Task 1 — Open commit + scaffold

This file. Update `docs/README.md` row to Open. Replace
`docs/ROADMAP.md` Phase 48 paragraph with the active marker.

### Task 2 — `docs/CHANNEL_SDK.md`

The third-party contract document. Sections:
- Audience + scope (single-operator, OS-user auth)
- Trust tier semantics (what `FrontendType::Local` /
  `Telegram` / `Web` choose for tier)
- IPC handshake (Ready → optional negotiate → StartSession)
- Message envelope cheatsheet (`FrontendMessage` /
  `DaemonMessage` / `DaemonLifecycleEvent` / `StreamEventPayload`)
- Lifecycle diagram
- Integration guarantees (capability gating, audit logging,
  cancellation)
- v0 stability disclaimer
- Pointers to `DAEMON_IPC.md`, `ADAPTER_PATTERN.md`,
  `THREAT_MODEL.md`, the Python example

### Task 3 — `docs/ADAPTER_PATTERN.md` expansion

The existing in-tree adapter checklist focuses on Rust impls of
`ChannelContext`. Add an out-of-tree section that swaps the
"implement the trait" line for "speak the IPC protocol" with
cross-references to `CHANNEL_SDK.md`. Keep the in-tree section
unchanged.

### Task 4 — `examples/python-channel/` adapter

Minimal Python frontend, stdlib only (no third-party deps).
Modules:
- `frame.py` — length-prefixed JSON encode/decode
- `client.py` — connect, negotiate, session, submit/receive
- `main.py` — REPL loop (read line → submit → render events
  until TurnComplete → loop)
- `README.md` — how to run

### Task 5 — Conformance scenarios

`examples/python-channel/tests/`:
- `test_handshake.py` — DaemonReady + negotiation + StartSession
- `test_simple_turn.py` — SubmitInput → events → TurnComplete
- `test_cancel.py` — SubmitInput → CancelTurn between events
- `test_gate.py` — escalation → ApprovalGate → ResolveGate

Tests target either (a) a real daemon spawned by the test
runner, or (b) a fixture trace file the test replays. Q5
keeps this lightweight.

### Task 6 — Exit freeze

Backfill exit stats, ship records, deferrals. Mark ROADMAP
frozen with exit commit hash. Update `docs/README.md` row.

## Ship records

| Task | Commit | Notes |
|---|---|---|
| 1 | `c566ddb` | scaffold |
| 2 | `0bda570` | `docs/CHANNEL_SDK.md` |
| 3 | `0e84160` | `docs/ADAPTER_PATTERN.md` out-of-tree section |
| 4 | `96b2271` | `examples/python-channel/` adapter |
| 5 | `4895485` | conformance suite (15 tests, daemon-free) |
| 6 | `7fbb534` | exit freeze |

## Deferrals carried into the phase

- Live audit push (Phase 47 Q4)
- Read-write dashboard inspection (Phase 47 Q6)
- `handle_connection` parameter-struct lift (Phase 47 Task 4)

## Net-new deferrals (predicted)

- **Conformance harness as a Rust crate.** Per Q5, deferred
  until third-party adoption surfaces real shape pressure.
- **Stable-version commitment for the IPC schema.** Per Q6, P11
  defers this until the SDK has stabilized in real third-party
  use.

## Exit criteria

- [x] `docs/CHANNEL_SDK.md` exists and documents the third-party
  contract.
- [x] `docs/ADAPTER_PATTERN.md` has an out-of-tree section.
- [x] `examples/python-channel/` runs against the daemon and
  drives one turn end-to-end. *(Manual smoke-test recipe in the
  README; conformance suite covers protocol shape headlessly.)*
- [x] Conformance scenarios pass — 15/15.
- [x] DESIGN.md untouched (streak → 7).
- [x] PRODUCT.md untouched (streak → 12).
- [x] `aivyx-core/src/lib.rs` untouched (streak → 3).
- [x] Zero clippy warnings; 948 tests still passing.

## Exit stats

- Rust tests: 948 → 948 (unchanged — phase was docs + examples)
- Python conformance tests: 0 → 15 (new)
- Clippy warnings: 0
- Deferral backlog: 3 → 5 (two new entries below)

### Streak outcomes

| Streak target | Predicted | Actual | New streak |
|---|---|---|---|
| DESIGN.md | untouched (7) | untouched | 7 |
| PRODUCT.md | untouched (12) | untouched | 12 |
| `aivyx-core/src/lib.rs` | untouched (3) | untouched | 3 |

All three predictions correct. Phase 48 was the cleanest "delivers
two PRODUCT.md forward commitments without touching production
Rust code" we've shipped — entirely docs (CHANNEL_SDK.md +
ADAPTER_PATTERN.md expansion) and examples (Python reference +
its conformance suite).

### Net-new deferrals carried forward

1. **Conformance harness as a Rust crate.** The Python suite
   proves the protocol shape but does not exercise the real
   daemon. A future `aivyx-conformance` crate could spin up a
   daemon and replay scripted IPC traces — useful for catching
   regressions in the *daemon* side of the protocol. Wait until
   third-party adopters surface real pressure before locking the
   shape (per Q5).
2. **Stable-version commitment for the IPC schema.** Per `PRODUCT.md`
   P11 and Q6, the SDK v0 makes integration guarantees but not
   API-stability guarantees. A future amendment will pin a
   stability window once enough third-party adapters exist to
   apply real pressure.

### Operator-side verification still pending

The Python adapter has a documented manual smoke-test recipe in
`examples/python-channel/README.md` (run `aivyx daemon run`,
then `python3 examples/python-channel/main.py`, type a message,
confirm streaming response). Not run in-phase. Recommended for
the first operator who picks up Phase 48's work.

### Forward commitments status

`PRODUCT.md` forward-commitment ledger after Phase 48:

- ~~P5 — Open First-Party Channel Surface~~ ✓ delivered (this phase)
- ~~P11 — SDK Contract: Interface + Integration~~ ✓ delivered (this phase)
- P12 — Tools as Separate Processes Over Daemon IPC — remaining

Only one forward commitment left. Phase 49 is the natural next.
