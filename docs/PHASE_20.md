# Phase 20 — Daemon Management + Deferral Cleanup

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Close six items from the sixteen-item rolling deferral backlog
with a focused cleanup phase: daemon lifecycle subcommands
(`daemon status`, `daemon stop`), PID file, `--no-daemon` flag,
daemon-mode banner parity, and the `CapabilitySet::grants`
reflexivity + `CEILING_SEMITRUSTED` doc-comment pair.

Phase 20 is **not a product-shape phase** — it ships no new
product primitive and advances no Product Commitment. It is
the same category as Phase 15 (Channel-Lib Consolidation):
a non-product-shape sub-phase that reduces accumulated
technical debt before the next keystone (Mission Primitive,
P2).

## Why now

1. **The deferral backlog has grown to sixteen items.** Five
   of those are daemon-management primitives deferred from
   Phases 17–18 because the daemon migration was the priority.
   The migration is complete (both adapters route through the
   daemon); the management surface is now the natural next
   step.

2. **`daemon status` / `daemon stop` are operator-facing
   gaps.** An operator whose daemon hangs or misbehaves has
   no subcommand to inspect or stop it — they have to `kill`
   the process manually. This is the first thing a real
   deployment will hit.

3. **The `CapabilitySet::grants` reflexivity investigation
   has been carried since Phase 13 (five phases).** It couples
   with the `CEILING_SEMITRUSTED` ▲-row doc-comment fix from
   Phase 15. Both are small, self-contained, and well-
   understood. Cleaning them up now prevents the pair from
   becoming permanently stale.

4. **Clearing the backlog before the Mission Primitive
   reduces noise.** The Mission phase (Phase 21, likely)
   will touch daemon infrastructure; having `daemon status`/
   `stop` and the PID file in place first means the Mission
   phase can compose against them rather than re-deferring
   them.

## Streak predictions

- **DESIGN.md** — Low risk. Daemon management subcommands
  and doc-comment fixes are plumbing, not architecture.
  Prediction: streak extends to **twenty consecutive phases**.
- **PRODUCT.md** — Not at risk. No product-shape work.
  Prediction: streak extends to **eight consecutive phases**.
- **Production-core `aivyx-core/src/lib.rs`** — Low risk.
  Tasks 2–5 touch `aivyx-channel` (binary + daemon modules).
  Task 6 touches `aivyx-capability` doc comments. None
  should need core lib changes. Prediction: streak extends
  to **nine consecutive phases** (would set a new record).

## Tasks

### Task 1 — Phase open (this commit)

Scaffold `docs/PHASE_20.md`. Update `docs/README.md`
phase-status table (Phase 20 → Open). Update
`docs/ROADMAP.md` Phase 20 entry.

### Task 2 — `daemon status` + `daemon stop` subcommands

Add `CliMode::DaemonStatus` and `CliMode::DaemonStop` to the
binary's CLI parser. Add a `FrontendMessage::Shutdown` variant
to `daemon_ipc.rs`. Implementation:

- `daemon status`: connect to the default socket path, send
  `StartSession` (or a lightweight ping), report whether a
  daemon is running and its protocol version from
  `DaemonReady`. If no daemon is listening, report that.
- `daemon stop`: connect, send `FrontendMessage::Shutdown`,
  wait for `ShuttingDown` lifecycle event, report success.
  The daemon server's receive loop handles `Shutdown` by
  cancelling its `CancellationToken`.

Parser tests for both subcommands. Integration test proving
`daemon stop` triggers graceful shutdown.

**Closes:** `daemon status / daemon stop subcommands`
(Phase 17 net-new).

### Task 3 — PID file at `$XDG_RUNTIME_DIR/aivyx/daemon.pid`

Write PID to `daemon.pid` (sibling of `daemon.sock`) on
daemon start. Remove on clean shutdown (Drop guard or
explicit cleanup in the shutdown path). `daemon status`
reads the PID file as a supplementary hint (process
existence check via `kill(pid, 0)`).

**Closes:** `PID file at $XDG_RUNTIME_DIR/aivyx/daemon.pid`
(Phase 17 net-new).

### Task 4 — `--no-daemon` flag for local mode

Add `--no-daemon` to the CLI parser. When set, the
`ChannelKind::Local` branch skips daemon-first dispatch and
goes directly to the in-process `run_session` path (the
original Phase 3 loop). Useful for debugging and for
operators who want deterministic single-process behavior.

Parser tests. One e2e test proving `--no-daemon` bypasses
daemon dispatch.

**Closes:** `--no-daemon flag for local mode` (Phase 18
net-new).

### Task 5 — Daemon-mode banner parity

The in-process banner shows: version, fs sandbox path,
`memory: live`, `audit: N events verified`, active role.
The daemon banner shows only: version `(daemon)`, socket
path, active role.

Bring to parity by extending the `SessionStarted` daemon
message (or adding a `ServerInfo` field) to carry the
daemon's store metadata: fs sandbox root, verified audit
event count, memory status. The daemon session banner
renders the same information as the in-process banner plus
the daemon socket path.

**Closes:** `Daemon-mode banner parity with in-process
banner` (Phase 18 net-new).

### Task 6 — `CEILING_SEMITRUSTED` ▲-row doc-comment fix + reflexivity investigation

Two co-homed items:

1. **Doc-comment fix:** The ▲-row comment on
   `CEILING_SEMITRUSTED` (lines 558–560 of
   `aivyx-capability/src/lib.rs`) describes scopes "omitted
   from unqualified ceiling" but the phrasing implies they
   survive intersection when qualified — which is only true
   if the held scope is also qualified and the qualifier
   matches. Rewrite to be precise about D4 rule interactions.

2. **Reflexivity investigation:** `CapabilitySet::grants`
   delegates to `Scope::is_granted_by`. Verify and document
   whether `grants(&self, &self)` is always true (reflexive)
   for every well-formed scope. If not, document the
   counter-example and whether it matters in practice.

Add a test pinning the reflexivity finding.

**Closes:** `CapabilitySet::grants reflexivity investigation`
(Phase 13 Task 4) + `Misleading CEILING_SEMITRUSTED ▲-row
doc comment` (Phase 15 Task 4).

### Task 7 — Exit freeze

Standard exit procedure: deferrals block, prediction-vs-
reality, exit criteria checklist, docs flips, ROADMAP +
PRODUCT_ROADMAP updates.

## Decisions

(Populated as tasks are worked.)

## Open questions

**Q1 — Should `daemon stop` be graceful-only or support
`--force`?**

(a) Graceful only — send `Shutdown` over IPC, let the
daemon finish in-flight turns. If the daemon doesn't
respond, the operator uses `kill`.

(b) Add `--force` that sends `Shutdown` and then
`kill(pid, SIGTERM)` after a timeout using the PID file.

**Recommendation: (a).** Keep it simple. `--force` adds
complexity (timeout logic, SIGTERM, race conditions) for a
scenario the operator can handle with standard Unix tools.
Revisit if real usage shows graceful-only is insufficient.

**Q2 — Should `SessionStarted` carry server metadata (for
banner parity), or should there be a separate `ServerInfo`
message type?**

(a) Extend `SessionStarted` with optional metadata fields
(`fs_sandbox`, `audit_event_count`, `memory_status`).

(b) Add a new `DaemonMessage::ServerInfo { ... }` variant
sent immediately after `SessionStarted`.

(c) Add optional fields to `DaemonReady` (the lifecycle
event already sent on connection).

**Recommendation: (a).** `SessionStarted` is the natural
place — the frontend needs this information at session
start to render the banner. Adding a separate message type
for three fields is over-engineering. `DaemonReady` is a
connection-level event, not a session-level event, so (c)
would conflate the two.

## Deferrals targeted for closure

| # | Item | Origin | Target task |
|---|------|--------|-------------|
| 1 | `daemon status` / `daemon stop` | Phase 17 | Task 2 |
| 2 | PID file | Phase 17 | Task 3 |
| 3 | `--no-daemon` flag | Phase 18 | Task 4 |
| 4 | Banner parity | Phase 18 | Task 5 |
| 5 | `CapabilitySet::grants` reflexivity | Phase 13 | Task 6 |
| 6 | `CEILING_SEMITRUSTED` ▲-row doc | Phase 15 | Task 6 |

Rolling backlog: 16 → 10 at exit (if all six close).
