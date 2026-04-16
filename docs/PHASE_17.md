# Phase 17 — Daemon Migration: Production Hardening (phase 2 of N)

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Convert the Phase 16 PoC daemon into a production-ready daemon
that a real operator can launch, leave running, and interact
with through the existing `aivyx` CLI binary — closing three
of the five Phase 16 net-new deferrals (production lifecycle,
auto-spawn, CLI integration) and rewriting the LocalChannel
integration tests to exercise the IPC boundary.

Phase 17 is **Daemon Migration phase 2 of N**. Phase 16 settled
the protocol and proved it carries one turn. Phase 17 hardens
the daemon so that `aivyx` (with no special flags) auto-spawns
a background daemon on first launch, connects over IPC, and runs
an interactive REPL session. By Phase 17 exit, the default
`--channel local` path should be able to run over the daemon
transparently.

## Why now

1. **Phase 16 exited with a settled protocol and a working
   PoC.** The IPC shape (length-prefixed JSON over Unix domain
   sockets) is committed in `docs/DAEMON_IPC.md`. The PoC
   daemon server, client, and round-trip test all pass. The
   forward-investment helpers (`PROTOCOL_VERSION`,
   `default_socket_path()`, `render_for_cli()`) are in place.
   The substrate is ready for production hardening.

2. **Three of Phase 16's five net-new deferrals are tagged
   "earliest plausible: Phase 17."** Production lifecycle,
   auto-spawn (Q6), and `daemon` subcommand / `--daemon` flag
   are all Phase 17 scope by construction. Deferring further
   means the daemon PoC sits untested at production scale for
   another phase.

3. **The production-core streak faces its strongest test in
   Phase 17.** Phase 16's prediction-vs-reality block noted
   that the mitigations held for the PoC scope but may not
   hold for multi-connection dispatch or auto-spawn lifecycle.
   The honest move is to face that test now rather than
   interleaving a lighter phase and pushing the harder
   decisions further out.

4. **The LocalChannel regression-test rewrite is the most
   valuable test investment the project can make right now.**
   The existing integration tests at
   `crates/aivyx-channel/tests/` exercise the turn loop
   in-process. Rewriting them to run over the daemon IPC
   boundary proves the daemon can serve every scenario the
   in-process path already covers. This is the test that
   catches "the protocol works for one turn but breaks under
   tool calls / cancellation / multi-turn sessions."

## Non-goals

Phase 17 is **production hardening for the LocalChannel path
over the daemon** and nothing else.

- **No Telegram-over-daemon port.** `aivyx-telegram` remains
  in its Phase 8 in-process shape. The Telegram port is a
  dedicated phase (probably Phase 18) after LocalChannel
  production readiness lands. Phase 16 exit tagged this as
  "earliest plausible: Phase 18 or later."
- **No multi-daemon or multi-socket architecture.** Phase 17
  runs one daemon per user (one socket at the well-known
  path). Multi-daemon (per-role daemons, per-project daemons)
  is a forward question for the phase that introduces
  project-level daemon scoping.
- **No wire-format upgrade.** The length-prefixed JSON format
  from Phase 16 carries forward unchanged. If throughput
  becomes a pressure point (unlikely for interactive REPL
  use), Phase 18+ can upgrade the wire format by bumping
  `PROTOCOL_VERSION`. Phase 17 does not reopen Q3.
- **No tool-process IPC (P12).** Tool-as-process remains a
  separate milestone that couples to the channel IPC shape
  but has its own design surface.
- **No channel SDK extraction (P5).** The SDK milestone
  consumes the IPC shape; Phase 17 does not publish an SDK.
- **No DESIGN.md preventive edits.** Same rule as Phase 16:
  an amendment happens only if an implementation decision
  genuinely requires it. No preventive wording changes.
- **No rolling-deferral pickup from the inherited backlog.**
  Phase 17 has enough new scope with production hardening.
  The `CapabilitySet::grants` reflexivity investigation, the
  ▲-row doc-comment rewrite, and multi-level sub-agent
  nesting all stay deferred.

## Entry criteria (all met from Phase 16 exit)

- [x] Phase 16 frozen at exit commit `1ed3f90` + hash
      backfill `e132e2e`. See PHASE_16.md.
- [x] `cargo test --workspace` is **533 green** (verified
      at Phase 16 exit, baseline for Phase 17's delta
      math).
- [x] `cargo clippy --workspace --tests -- -D warnings`
      clean at Phase 16 exit.
- [x] `crates/aivyx-core/src/lib.rs` byte-identical to
      `ba9a724`. **Streak at five consecutive phases** —
      Phase 16's prediction that it would "probably break"
      was wrong; every mitigation held.
- [x] `docs/DESIGN.md` byte-identical to `e0d6437`.
      **Streak at sixteen consecutive phases.**
- [x] `PRODUCT.md` byte-identical to `80189b4`. **Streak
      at four consecutive phases.**
- [x] `crates/aivyx-channel/src/bin/aivyx.rs` at **2072
      lines** (unchanged by Phase 16 — the PoC lived at
      the library level only). Phase 17 will grow this
      number: the `daemon` subcommand, the `--daemon` flag,
      and the auto-spawn dispatch path are all binary-level
      code.
- [x] `crates/aivyx-channel/tests/` directory has **nine**
      integration test files (six from pre-Phase 16 + one
      Phase 15 + two Phase 15). Phase 17 may add new test
      files for daemon-mode regression coverage.
- [x] Phase 16 daemon substrate in place:
      - `daemon_ipc.rs` (505 lines) — protocol types,
        `encode_frame`/`decode_frame`, `PROTOCOL_VERSION`,
        `default_socket_path()`, `render_for_cli()`.
      - `daemon_server.rs` (281 lines) — PoC server with
        `IpcChannelBridge`.
      - `daemon_client.rs` (146 lines) — PoC client with
        `DaemonTurnResult`.
      - `daemon_roundtrip_e2e.rs` (183 lines) — round-trip
        integration test.
- [x] `docs/DAEMON_IPC.md` exists as the cross-phase
      protocol reference.
- [x] Rolling deferral backlog at fifteen items (ten
      inherited from Phase 15 + five net-new from Phase 16).
      Phase 17 targets closing at least three of the five
      Phase 16 net-new items.

## Streaks at risk

Phase 17 inherits Phase 16's streak-risk discipline: name the
risks upfront, pin mitigations, and reckon honestly at exit.

- **DESIGN.md streak (16 → 17, low risk).** Phase 16
  proved D1's wording is process-boundary agnostic via
  the `IpcChannelBridge` pattern. Phase 17 extends that
  pattern to multi-turn sessions and auto-spawn — neither
  should require a D1 amendment. D3 (`ChannelContext`
  trait) is the same risk as Phase 16: if Q2 below
  resolves to (a), the trait stays unchanged. Risk is
  lower than Phase 16 because the proof-of-concept already
  validated the D1/D3 interpretation.

- **PRODUCT.md streak (4 → 5, not at risk).** Same
  protection as Phase 16: P4's deliberate-silence clause
  absorbs all daemon implementation decisions. Phase 17
  hardens what Phase 16 settled; no new product-shape
  decision is made.

- **Production-core `aivyx-core/src/lib.rs` streak (5 →
  6, genuinely at risk).** Phase 16's prediction-vs-
  reality block noted that the mitigations "held for the
  PoC scope" but flagged two mechanisms that Phase 17
  may stress:

  1. **Multi-turn session state.** The PoC runs one turn
     and exits. A production daemon manages a session
     across multiple turns, which may surface a need for
     session-lifecycle hooks on `ChannelContext` (e.g.,
     `session_started()`, `session_ended()`). If those
     hooks require a trait method, the streak breaks.
     **Mitigation:** manage session lifecycle in the
     daemon's dispatch loop, not in the `ChannelContext`
     trait. The `IpcChannelBridge` already wraps a
     session-ID string; the daemon can track session
     state in its own data structures without extending
     the trait.

  2. **Auto-spawn lifecycle signaling.** The frontend
     needs to know "daemon is starting" vs. "daemon is
     ready" vs. "daemon failed to start." If this
     lifecycle signaling needs to cross the `StreamEvent`
     boundary, the enum gains a variant and the streak
     breaks. **Mitigation:** Q4→(a) from Phase 16 —
     `DaemonLifecycleEvent` is a separate IPC message
     type. Auto-spawn can use the same message type
     for `DaemonStarting`/`DaemonReady` without touching
     `StreamEvent`.

  3. **Graceful shutdown propagation.** The daemon needs
     to tell connected frontends "I am shutting down."
     If this needs to interrupt an in-flight
     `stream_event()` call, the `CancellationToken`
     pattern (already on `ChannelContext`) may need
     extension. **Mitigation:** the existing
     `cancellation_token()` method plus
     `DaemonLifecycleEvent::ShuttingDown` (already
     defined in the Phase 16 protocol) should suffice.

  **Honest position:** the streak is **more likely to hold
  in Phase 17 than the Phase 16 open doc predicted for
  Phase 16**. The Phase 16 PoC proved the `IpcChannelBridge`
  pattern works for the core turn-loop path, and Phase 17's
  extensions (multi-turn, auto-spawn, graceful shutdown)
  are dispatch-level concerns, not trait-level concerns.
  But "more likely to hold" is not "certain to hold" — any
  of the three mechanisms above could surface a concrete
  need. The exit doc will reckon with this honestly.

- **Zero-new-dep streak (low risk).** Phase 17's likely
  new functionality (signal handling, process spawning,
  PID file management) should be expressible with `tokio`
  features already in the workspace plus `std::process`.
  No new crate is anticipated.

## Open questions

### Q1 — How does the daemon manage multi-turn sessions?

The Phase 16 PoC exits after one turn. A production daemon
must support an interactive REPL session: multiple
`SubmitInput` messages on the same connection, with the agent
maintaining conversation history across turns.

Three candidate shapes:

- **(a) Session state lives in the daemon's dispatch loop.**
  The daemon maintains a `HashMap<SessionId, SessionState>`
  that tracks conversation history, the active agent, and
  the channel bridge for each connected frontend. The
  `ChannelContext` trait is unchanged; session lifecycle
  is a daemon-internal concern.
- **(b) Session state is managed by a new `SessionManager`
  trait in `aivyx-core`.** The trait provides `start_session`,
  `end_session`, and `get_session` methods. The daemon and
  in-process paths both implement it. The production-core
  streak breaks.
- **(c) Session state is managed by extending `ChannelContext`
  with session-lifecycle hooks.** Same as mechanism #1 in the
  streaks-at-risk analysis. Production-core streak breaks.

Initial lean: **(a)**. The Phase 16 `IpcChannelBridge` already
carries a `session_id: String` field. The daemon's dispatch
loop can maintain session state (conversation history,
agent handle) in its own data structures. The turn loop
receives a `Message` and a `&dyn ChannelContext` per turn —
it does not need to know about session boundaries.

### Q2 — Does `ChannelContext` need changes for multi-connection support?

Phase 16 Q2 resolved to (a) — `ChannelContext` unchanged.
Phase 17 introduces multiple concurrent frontend connections.
Does the daemon need to multiplex turn-loop invocations
across connections, and does that multiplexing require a
trait change?

- **(a) No trait change.** Each connection gets its own
  `IpcChannelBridge` instance with its own `session_id`.
  The daemon serializes turn-loop invocations (one turn at
  a time) or spawns them on separate tokio tasks. The trait
  is unchanged.
- **(b) Trait gains a connection-identity method.** E.g.,
  `connection_id() -> ConnectionId`. Production-core
  streak breaks.

Initial lean: **(a)**. Connection multiplexing is a daemon
dispatch concern, not a trait concern. Each `IpcChannelBridge`
is already parameterized by its connection's write half. The
daemon's dispatch loop decides whether to serialize or
parallelize turns; the turn loop itself is unchanged.

### Q3 — How does auto-spawn work?

P4.5 commits to auto-spawn: "if no daemon is running, the
frontend spawns one transparently." Three candidate shapes:

- **(a) Fork + setsid (double-fork on Unix).** The frontend
  forks a child, the child calls `setsid()` to detach from
  the terminal, then exec's the daemon binary. The frontend
  polls for the socket file to appear, then connects.
  Classic Unix daemonization.
- **(b) `tokio::process::Command` spawn.** The frontend
  spawns the daemon as a detached child process using
  `Command::new("aivyx").arg("daemon").arg("run")` with
  stdout/stderr redirected to a log file. Simpler than
  double-fork; relies on process-group semantics for
  detachment.
- **(c) Systemd/launchd socket activation.** The daemon
  is registered as a user service; the frontend starts it
  via `systemctl --user start aivyx-daemon`. Most
  production-ready but most platform-specific.

Initial lean: **(b)** for Phase 17. `tokio::process::Command`
is already available in the workspace (tokio `process`
feature). Double-fork is more correct on Unix but adds
complexity the PoC doesn't need. Systemd integration is a
Phase 18+ concern. Phase 17 uses `Command` spawn with a
PID file for liveness checking and a timeout-based retry
loop for socket readiness.

### Q4 — What does graceful shutdown look like?

The PoC daemon exits after one turn. A production daemon
needs to handle SIGTERM/SIGINT gracefully.

- **(a) Signal handler cancels in-flight turns, sends
  `ShuttingDown` to all connected frontends, waits for
  drain, exits.** Uses `tokio::signal` (already in the
  workspace). The `DaemonLifecycleEvent::ShuttingDown`
  message is already defined in the Phase 16 protocol.
- **(b) Immediate exit on signal.** Frontends detect the
  broken socket and reconnect or report an error. Simplest
  but worst operator experience.

Initial lean: **(a)**. The protocol already has
`ShuttingDown`; Phase 17 should wire it to the signal
handler. The `CancellationToken` pattern (already on
`ChannelContext`) provides the in-flight turn cancellation
primitive.

### Q5 — How does the binary's CLI surface change?

The binary needs a daemon entry point and a frontend mode
switch.

- **(a) `aivyx daemon run` subcommand.** Launches the
  daemon in the foreground. Auto-spawn uses this
  internally. `aivyx` (no subcommand) auto-spawns if
  needed, then connects as a frontend.
- **(b) `--daemon` flag on the default command.** `aivyx
  --daemon` runs as the daemon; `aivyx` (without flag)
  auto-spawns and connects.
- **(c) Separate binary `aivyx-daemon`.** A second `[[bin]]`
  target in the channel crate. Clean separation but doubles
  the binary build surface.

Initial lean: **(a)**. A subcommand is the standard Unix
pattern for "this binary has multiple modes." `aivyx daemon
run` is explicit; `aivyx daemon status` and `aivyx daemon
stop` are natural extensions for later phases. The default
`aivyx` invocation auto-spawns transparently per P4.5.

### Q6 — How many existing integration tests get rewritten to run over IPC?

The nine test files at `crates/aivyx-channel/tests/` exercise
various turn-loop scenarios in-process. Rewriting all of them
to also run over IPC would be the most comprehensive validation
but may be too ambitious for one phase.

- **(a) All nine.** Every existing test gets a daemon-mode
  counterpart. Highest confidence; highest task budget.
- **(b) A representative subset.** Pick the tests that
  exercise the most daemon-relevant code paths (multi-turn,
  tool calls, cancellation) and rewrite those. The rest
  continue in-process.
- **(c) One new multi-turn daemon test only.** Extends the
  Phase 16 round-trip test to cover multi-turn sessions
  but does not rewrite existing tests.

Initial lean: **(b)**. A representative subset (likely 3–4
tests covering multi-turn, tool calls, and error handling)
gives high confidence without the task budget of rewriting
all nine. The remaining tests continue to validate the
in-process path, which is still the Telegram adapter's
execution model.

## Draft task breakdown

Five tasks, same cadence as Phases 11–16. Task 4 is the
working-session slot; Task 5 is exit freeze.

### Task 1 — Open commit (this document)

The phase's first commit is this doc, the `docs/README.md`
row flip from Frozen (Phase 16) to Active (Phase 17), and
the `docs/ROADMAP.md` Phase 17 entry update. No code, no
tests, no test-delta requirement.

**Acceptance:**

- `docs/PHASE_17.md` exists with the structure of this
  document.
- `docs/README.md` phase-status table has a Phase 17 row
  marked Active.
- `docs/ROADMAP.md` Phase 17 entry updated from scaffold
  to active description.
- Commit message: `docs(phase-17): open — Daemon Migration
  production hardening phase 2 of N`.

### Task 2 — Multi-turn daemon + graceful shutdown

**Delivers:** a production daemon server that supports
multi-turn interactive sessions and shuts down gracefully
on SIGTERM/SIGINT.

**Cut:** extend `daemon_server.rs` from single-turn to
multi-turn:

- The daemon reads `SubmitInput` messages in a loop
  (not just one) and dispatches each turn through the
  agent.
- Conversation history is maintained per session in the
  daemon's dispatch loop.
- SIGTERM/SIGINT trigger `DaemonLifecycleEvent::ShuttingDown`
  to all connected frontends, cancel in-flight turns via
  `CancellationToken`, and exit cleanly after drain.
- The `Disconnect` message from the frontend closes the
  connection cleanly.

**Test count target:** +3 (multi-turn session test,
graceful-shutdown test, disconnect test).

**Acceptance:**

- `daemon_server.rs` supports multi-turn sessions.
- Signal-based graceful shutdown works.
- `cargo test --workspace` green; delta ≥ +3.
- Q1 and Q4 resolved and recorded.
- Production-core streak status recorded.

### Task 3 — CLI integration + auto-spawn

**Delivers:** the `daemon` subcommand in the binary, the
auto-spawn logic in the default frontend path, and the
`daemon_client.rs` upgrade to support multi-turn REPL
interaction.

**Cut:**

- `parse_cli_args` gains `aivyx daemon run` (foreground
  daemon mode) and `aivyx daemon status` / `aivyx daemon
  stop` (lifecycle queries).
- The default `aivyx` invocation (no subcommand) checks
  for a running daemon at `default_socket_path()`. If
  none is found, spawns one via `tokio::process::Command`,
  waits for the socket, and connects.
- The frontend's REPL loop sends `SubmitInput` for each
  line and renders `StreamEvent`s via `render_for_cli()`.
- PID file at `$XDG_RUNTIME_DIR/aivyx/daemon.pid` (or
  fallback) for liveness checking.

**Test count target:** +4 (CLI arg parsing tests for daemon
subcommand, auto-spawn integration test, PID file lifecycle
test, multi-turn client test).

**Acceptance:**

- `aivyx daemon run` launches the daemon in foreground.
- `aivyx` auto-spawns a daemon if none is running.
- The REPL works interactively over IPC.
- `cargo test --workspace` green; delta ≥ +7 across
  Tasks 2 + 3 combined.
- Q2, Q3, Q5 resolved and recorded.

### Task 4 — Working-session slot

Reserved for mid-implementation correction. Candidates:

- **Integration test rewrite (Q6).** Rewrite a
  representative subset of existing integration tests
  to run over the daemon IPC boundary.
- **Protocol edge cases.** If Tasks 2–3 surface
  protocol gaps (e.g., backpressure, heartbeat,
  reconnection), Task 4 addresses them.
- **Binary line-count management.** If the binary
  grows past 2300 lines, Task 4 can lift daemon-
  specific dispatch code into a library module.

None is pre-committed. Task 4 opens with a review of
what Tasks 2–3 surfaced.

### Task 5 — Exit freeze

Same shape as Phase 11–16 exit freezes:

- Ship records for Tasks 1–3 (and Task 4 if used).
- Decisions block recording how Q1–Q6 resolved.
- Deferrals block inheriting the fifteen-item backlog
  from Phase 16 exit, closing at least three Phase 16
  net-new items, and recording any Phase 17 net-new.
- Final Exit criteria checklist.
- `docs/README.md` phase-status row flipped.
- `docs/ROADMAP.md` Phase 17 entry replaced with frozen
  summary; Phase 18 scaffold.
- `docs/PRODUCT_ROADMAP.md` Daemon Migration milestone
  updated.
- Prediction-versus-reality block (continuing the
  Phase 16 innovation).

**Acceptance:**

- All task ship records and decisions block in this
  document.
- `cargo test --workspace` green at exit. Test delta
  across the full phase **≥ +7** against the 533-test
  entry baseline.
- `cargo clippy --workspace --tests -- -D warnings`
  clean.
- DESIGN.md byte-identical to `e0d6437` **or** a
  documented amendment. Streak either extends to
  seventeen consecutive phases or breaks with reason.
- PRODUCT.md byte-identical to `80189b4`. Streak
  extends to **five consecutive phases.**
- `aivyx-core/src/lib.rs` — may or may not break.
  If it holds, streak extends to **six**. If it breaks,
  the exit doc records which mechanism broke it.
- Zero-new-dep streak — expected to hold. No new crate
  anticipated.
- `docs/README.md` phase-status table reflects exit.
- `docs/ROADMAP.md` Phase 17 frozen, Phase 18 scaffold.
- `docs/PRODUCT_ROADMAP.md` updated.
- Prediction-versus-reality block recorded.

## Decisions made at phase open

1. **Phase 17 is Daemon Migration phase 2 of N.** The
   full migration continues; Phase 17 hardens the daemon
   for the LocalChannel path only. Telegram port and other
   adapter ports are later phases.
2. **The Q-block settles production concerns, not protocol
   concerns.** Unlike Phase 16 (protocol-heavy), Phase 17's
   Qs are about lifecycle, CLI surface, and test strategy.
   The protocol shape is inherited as a given.
3. **Phase 17 closes Phase 16 deferrals, not inherited
   rolling deferrals.** The fifteen-item backlog is too
   large to carry indefinitely, but Phase 17's scope is
   focused enough that picking up unrelated deferrals
   would be scope drift.
4. **The conservative scope is again load-bearing.** The
   aggressive version ("production daemon + Telegram port +
   full test rewrite in one phase") would produce the
   seven-task, mid-correction shape Phase 16's open doc
   warned about. Phase 17 scopes to "LocalChannel over
   daemon, production-ready, with representative test
   coverage."
5. **Streak-risk honesty continues.** Phase 16 innovated
   the prediction-vs-reality block. Phase 17 continues the
   practice, naming the production-core streak as
   "genuinely at risk" while noting the mitigations are
   stronger than at Phase 16 open.

## Decisions made during implementation

### Task 2 — Q1 resolution: session state in daemon dispatch loop (option a)

**Resolved:** option **(a)**. The daemon's frame-read loop
maintains session state (`_session_id`) in its own local
variables. After a `TurnComplete` frame is sent, the loop
continues reading the next `SubmitInput` rather than exiting.
No `SessionManager` trait or `ChannelContext` extension was
needed. The `Agent::turn` method receives a fresh `Message`
per turn; conversation history management is the agent
implementation's concern (in production, `ConcreteAgent`
maintains history internally; in tests, `FakeStreamingAgent`
ignores it). **Production-core streak holds.**

### Task 2 — Q4 resolution: signal-driven shutdown via CancellationToken (option a)

**Resolved:** option **(a)**. The `run_daemon` function accepts
a `CancellationToken` parameter. When cancelled, the daemon
finishes the current frame-read iteration (it does not
interrupt an in-flight turn mid-execution), sends
`DaemonLifecycleEvent::ShuttingDown` to the connected
frontend, and returns `Ok(())`. The `ShuttingDown` message
was already defined in the Phase 16 protocol; Phase 17 wires
it to the shutdown token.

In production, the binary will wire `tokio::signal::ctrl_c()`
to `shutdown.cancel()` (Task 3). In tests, the test code calls
`shutdown.cancel()` directly. The `CancellationToken` is from
`tokio_util::sync`, already re-exported by `aivyx_core`.

### Task 2 — implementation shape

`daemon_server.rs` rewritten from single-turn PoC to multi-turn
production server:

- **`run_daemon<C>` function** — new entry point accepting a
  `CancellationToken` for graceful shutdown. The frame-read
  loop continues after `TurnComplete` instead of returning.
  `tokio::select!` on both the socket read and the shutdown
  token ensures the daemon responds to shutdown even while
  blocked waiting for client input.
- **`run_poc_daemon<C>` function** — backward-compatible alias
  that creates an uncancelled `CancellationToken` and delegates
  to `run_daemon`. Existing Phase 16 test unchanged.
- **`send_shutting_down` helper** — sends the
  `DaemonLifecycleEvent::ShuttingDown` frame, ignoring write
  errors (the frontend may already be gone).
- **`format_outcome` helper** — extracted from the inline match
  in the PoC for reuse across the multi-turn loop.

Three new integration tests in `daemon_roundtrip_e2e.rs`:

1. **`multi_turn_session_streams_both_turns`** — connects,
   starts a session, sends two `SubmitInput` messages on the
   same connection, asserts both turns stream two events each
   and both outcomes match. Proves the daemon does not exit
   after the first turn.
2. **`graceful_shutdown_sends_shutting_down`** — connects,
   reads `DaemonReady`, then cancels the shutdown token. Asserts
   the daemon sends a `ShuttingDown` frame with a reason
   containing "shutdown" before the connection closes.
3. **`frontend_disconnect_stops_daemon_cleanly`** — connects,
   reads `DaemonReady`, then drops the connection. Asserts the
   daemon task completes without panic (EOF on the read side
   causes a clean exit).

**Test delta:** +3 (1 multi-turn, 1 graceful shutdown,
1 disconnect). Combined phase delta Tasks 1–2: +3 (target
was ≥ +3 for Task 2). Workspace test count: **536** (entry
baseline 533).

**Streak status after Task 2:**

- DESIGN.md byte-identical to `e0d6437`. Streak at **seventeen
  consecutive phases** (pending exit confirmation).
- PRODUCT.md byte-identical to `80189b4`. Streak at **five
  consecutive phases** (pending exit confirmation).
- `aivyx-core/src/lib.rs` byte-identical to `ba9a724`. Streak
  at **six consecutive phases** (pending exit confirmation).
  Q1→(a) and Q4→(a) both avoided trait-level changes.
- Zero-new-dep streak holds.

### Task 3 — Q2 resolution: no ChannelContext changes for multi-connection (option a)

**Resolved:** option **(a)**. Each connection gets its own
`IpcChannelBridge` instance. The daemon currently accepts one
connection (Task 2 scope); multi-connection support is a
Phase 18+ concern. The `ChannelContext` trait is unchanged.
**Production-core streak holds.**

### Task 3 — Q3 resolution: Command spawn for auto-spawn (option b)

**Resolved:** option **(b)** is the target shape. Phase 17
Task 3 lands the `daemon run` foreground entry point that
auto-spawn will invoke. The actual auto-spawn logic (detect
no socket → spawn `aivyx daemon run` → wait for socket →
connect) is **deferred to Task 4** as a working-session-slot
candidate. Task 3 delivers the daemon subcommand and the
CLI surface; auto-spawn wiring is the natural follow-up.

### Task 3 — Q5 resolution: `daemon run` subcommand (option a)

**Resolved:** option **(a)**. The CLI gains a `daemon run`
subcommand that launches the daemon in the foreground. The
existing `--verify-only`, `--print-role`, and default session
modes are refactored into a `CliMode` enum: `Session`,
`VerifyOnly`, `PrintRole(String)`, `DaemonRun`. The parser
checks for `daemon run` as a positional subcommand before
falling into the flag-parsing loop.

### Task 3 — implementation shape

**Binary changes (`aivyx.rs`):**

- **`CliMode` enum** — replaces the previous `verify_only: bool`
  + `print_role: Option<String>` fields with a single
  discriminated enum. Four variants: `Session`, `VerifyOnly`,
  `PrintRole(String)`, `DaemonRun`. All existing tests updated.
- **`daemon run` subcommand parsing** — detected as a positional
  pair before the flag loop. `daemon` alone (without `run`)
  gives a helpful "did you mean `daemon run`?" error.
  Extra args after `daemon run` are rejected.
- **`run_async` daemon branch** — when `mode == DaemonRun`, the
  function builds a `ConcreteAgent` with the full provider/audit/
  tool/capability stack (identical to the in-process path), wires
  `tokio::signal::ctrl_c()` to a `CancellationToken`, and calls
  `run_daemon` from `daemon_server.rs`. The daemon listens on
  `default_socket_path()`.

**No new library files.** All changes are in the binary. The
daemon server (`daemon_server.rs`) is consumed as-is from
Task 2.

**Test delta:** +4 (4 CLI arg parsing tests:
`daemon_run_parses_to_daemon_mode`,
`daemon_without_run_is_an_error`,
`daemon_run_rejects_extra_args`,
`daemon_run_is_not_combinable_with_channel_flag`).
Combined phase delta Tasks 1–3: +7 (target was ≥ +7).
Workspace test count: **540** (entry baseline 533).

**Binary line count:** 2159 (up from 2072, net +87 for
daemon subcommand + CliMode refactor + 4 tests).

**Streak status after Task 3:**

- DESIGN.md byte-identical to `e0d6437`. Streak holds.
- PRODUCT.md byte-identical to `80189b4`. Streak holds.
- `aivyx-core/src/lib.rs` byte-identical to `ba9a724`. Streak
  holds. No trait changes.
- Zero-new-dep streak holds.

### Task 4 — Q6 resolution: multi-turn client library + auto-spawn, not full test rewrite (option c+)

**Resolved:** a shape between **(b)** and **(c)**. Task 4 used
the working-session slot for the auto-spawn + multi-turn client
library that Task 3 deferred, plus two new integration tests
exercising the client library directly. A full test rewrite
(option a) or representative subset rewrite (option b) was not
needed: the six daemon e2e tests already cover multi-turn,
graceful shutdown, and disconnect from both the raw-IPC and
client-library perspectives. The remaining in-process tests
continue validating the Telegram adapter's execution model.

### Task 4 — implementation shape

**`daemon_client.rs` rewritten** from 147-line single-turn PoC
to ~267-line multi-turn client library:

- **`DaemonSession` struct** — holds the split `UnixStream`
  (reader + writer), a frame buffer, and the negotiated
  `session_id` and `daemon_version`. Created by
  `DaemonSession::connect()`, which reads `DaemonReady`, sends
  `StartSession`, reads `SessionStarted`, and returns the handle.
- **`DaemonSession::submit_input()`** — sends `SubmitInput`,
  collects `StreamEvent`s in a loop until `TurnComplete`.
  Returns the events and outcome string. The connection stays
  open for the next turn.
- **`DaemonSession::disconnect()`** — sends `Disconnect` and
  consumes `self` (the connection drops on return).
- **`daemon_is_running()`** — async function that attempts a
  `UnixStream::connect` and returns `true`/`false`. Used by
  auto-spawn logic to check whether a daemon is already
  listening.
- **`spawn_daemon_and_wait()`** — spawns `aivyx daemon run` via
  `tokio::process::Command`, then polls `daemon_is_running()`
  with exponential backoff (20ms → 500ms cap) until the socket
  appears or the timeout expires. Returns the socket path on
  success.
- **`run_poc_client()`** — reimplemented on top of
  `DaemonSession` for backward compatibility with the existing
  Phase 16 e2e test.

**Two new integration tests** in `daemon_roundtrip_e2e.rs`:

1. **`daemon_session_multi_turn_via_client_library`** — uses
   `DaemonSession::connect`, two `submit_input` calls, and
   `disconnect`. Proves the client library supports multi-turn
   without manual frame manipulation.
2. **`daemon_is_running_returns_false_for_absent_socket`** —
   verifies the utility function returns `false` when no daemon
   is listening at the socket path.

**Test delta:** +2 (1 multi-turn client library, 1 utility).
Combined phase delta Tasks 1–4: +9 (target was ≥ +7).
Workspace test count: **542** (entry baseline 533).

**`DaemonTurnResult`** retained for backward compat with
`run_poc_client`. New code should use `DaemonSession` directly.

**Streak status after Task 4:**

- DESIGN.md byte-identical to `e0d6437`. Streak holds.
- PRODUCT.md byte-identical to `80189b4`. Streak holds.
- `aivyx-core/src/lib.rs` byte-identical to `ba9a724`. Streak
  holds. No trait changes needed — `DaemonSession` and
  auto-spawn are purely client-library concerns.
- Zero-new-dep streak holds.
