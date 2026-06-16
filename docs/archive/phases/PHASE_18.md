# Phase 18 — Daemon Migration: Frontend Wiring (phase 3 of N)

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../../../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../../../PRODUCT.md).

## Goal

Wire the default `aivyx` invocation to auto-spawn a daemon
(if none is running), connect via `DaemonSession`, and run an
interactive REPL loop that renders `StreamEventPayload`s via
`render_for_cli()`. By Phase 18 exit, `aivyx --channel local`
(the default path) runs over the daemon transparently — the
operator sees the same banner, prompt, and streaming output
they saw before, but the turn loop runs in a background daemon
process.

Phase 18 is **Daemon Migration phase 3 of N**. Phase 16
settled the protocol; Phase 17 hardened the daemon and built
the client library. Phase 18 closes the loop: the frontend
binary becomes a thin IPC client that auto-attaches to the
daemon.

## Why now

1. **The substrate is complete.** Phase 17 delivered
   `DaemonSession` (multi-turn client), `spawn_daemon_and_wait`
   (auto-spawn with exponential backoff), `daemon_is_running`
   (socket-presence check), `daemon run` (foreground daemon
   entry point), and `render_for_cli` (IPC-side stream-event
   rendering). The only remaining work is wiring these pieces
   into the binary's session-mode dispatch path.

2. **Phase 17's REPL-mode-over-IPC deferral is the natural
   next step.** It was tagged "earliest plausible: Phase 18 or
   whenever the default path switches from in-process to
   daemon-backed." Phase 18 is that phase.

3. **P4.5 ("if no daemon is running, the frontend spawns one
   transparently") is the last undelivered P4 commitment
   that Phase 18 can close.** Once the default `aivyx`
   invocation auto-spawns and connects, the operator experience
   matches P4.5's promise. The Telegram port (P4 for non-
   local adapters) is a separate phase.

4. **The in-process path remains as a fallback.** Phase 18
   does not delete the in-process `run_session` code path —
   it adds a daemon-backed alternative that becomes the
   default. The in-process path stays for tests, for the
   Telegram adapter (which hasn't been ported yet), and as
   a `--no-daemon` fallback if the daemon fails to start.

## Non-goals

- **No Telegram-over-daemon port.** Same as Phase 17.
  `aivyx-telegram` remains in its Phase 8 in-process shape.
- **No multi-connection daemon.** Phase 18's daemon still
  accepts one connection at a time. Multi-connection dispatch
  is a forward concern for the phase that needs it.
- **No `daemon status` / `daemon stop` subcommands.** These
  are Phase 17 deferrals with no urgency. The auto-spawn
  path only needs `daemon run` and socket-presence checking.
- **No wire-format upgrade.** Same as Phase 17.
- **No `--no-daemon` flag.** The in-process path remains
  reachable through tests and the Telegram branch, but
  Phase 18 does not add a user-facing flag to force it for
  local mode. That's a forward concern if operators ask for
  it.
- **No DESIGN.md or PRODUCT.md edits.** Same discipline as
  Phases 16–17: amendments only if an implementation decision
  genuinely requires one.

## Entry criteria (all met from Phase 17 exit)

- [x] Phase 17 frozen at exit commit `277d910` + hash
      backfill `64446f5`. See PHASE_17.md.
- [x] `cargo test --workspace` is **542 green** (verified
      at Phase 17 exit, baseline for Phase 18's delta math).
- [x] `cargo clippy --workspace --tests -- -D warnings`
      clean at Phase 17 exit.
- [x] `crates/aivyx-core/src/lib.rs` byte-identical to
      `ba9a724`. **Streak at six consecutive phases.**
- [x] `docs/DESIGN.md` byte-identical to `e0d6437`.
      **Streak at seventeen consecutive phases.**
- [x] `PRODUCT.md` byte-identical to `80189b4`. **Streak
      at five consecutive phases.**
- [x] Phase 17 daemon substrate in place:
      - `daemon_server.rs` (307 lines) — multi-turn server
        with `CancellationToken` shutdown.
      - `daemon_client.rs` (267 lines) — `DaemonSession`,
        `daemon_is_running`, `spawn_daemon_and_wait`.
      - `daemon_ipc.rs` (505 lines) — protocol types,
        `render_for_cli()`.
      - `daemon_roundtrip_e2e.rs` (521 lines) — 6 e2e tests.
      - `aivyx.rs` (2159 lines) — `CliMode::DaemonRun` branch
        with full agent-stack wiring.
- [x] Rolling deferral backlog at fifteen items (see
      PHASE_17.md deferrals block). Phase 18 targets closing
      the REPL-mode-over-IPC deferral.

## Streaks at risk

- **DESIGN.md streak (17 → 18, not at risk).** Phase 18
  wires existing library code into the binary's dispatch
  path. No new trait, no new architectural decision. D1's
  "turn loop as state machine" invariant is upheld by the
  daemon; D3's `ChannelContext` trait is not touched. This
  is the lowest-risk phase for DESIGN.md in the daemon arc.

- **PRODUCT.md streak (5 → 6, not at risk).** Same
  protection as Phases 16–17: P4's deliberate-silence clause
  absorbs frontend wiring decisions.

- **Production-core `aivyx-core/src/lib.rs` streak (6 →
  7, not at risk).** Phase 18's changes are entirely in the
  binary (`aivyx.rs`) and possibly `daemon_client.rs`. No
  trait-level or core-type changes are anticipated. The six-
  phase streak should extend to seven by construction.

- **Zero-new-dep streak (not at risk).** All pieces are
  already in the workspace.

**Honest position:** Phase 18 is the lowest-streak-risk
phase in the entire daemon arc. The load-bearing
architectural decisions were made in Phases 16–17; Phase 18
is wiring.

## Open questions

### Q1 — Does the daemon-backed REPL replace or sit alongside the in-process path?

The binary currently has one local-mode path: `CliMode::Session`
→ `run_session` (in-process). Phase 18 introduces a second
path: auto-spawn daemon → connect via `DaemonSession` → REPL
over IPC.

- **(a) Replace.** `CliMode::Session` with `--channel local`
  always goes through the daemon. The in-process
  `run_session` remains in the codebase but is only called
  by tests and the Telegram branch (which has its own
  session function).
- **(b) Sit alongside with auto-detection.** `CliMode::Session`
  tries the daemon path first; if `spawn_daemon_and_wait`
  fails (e.g., socket path is on a read-only filesystem),
  falls back to in-process `run_session` silently or with a
  warning.
- **(c) Sit alongside with a `--no-daemon` flag.** Like (b)
  but the operator can force in-process mode explicitly.

Initial lean: **(b)**. Silent fallback with a warning is the
most operator-friendly shape for a first release. The daemon
path is the happy path; the in-process path is the degraded
fallback. A `--no-daemon` flag is a Phase 19+ concern if
operators ask for it.

### Q2 — How does the daemon-backed REPL render streaming output?

The in-process REPL renders via `LocalChannel`'s writer handle
(which wraps `io::Stdout`). The daemon-backed REPL receives
`StreamEventPayload` frames over IPC.

- **(a) `render_for_cli()` directly to stdout.** The frontend
  calls `StreamEventPayload::render_for_cli()` for each
  received event and writes the result to stdout. Simple;
  matches the existing rendering fidelity.
- **(b) Convert `StreamEventPayload` back to `StreamEvent`
  and route through the existing rendering pipeline.** More
  complex; higher rendering fidelity if the in-process
  pipeline has features `render_for_cli` doesn't replicate.
- **(c) A new `DaemonFrontendChannel` implementing
  `ChannelContext`.** The frontend constructs a channel
  whose `stream_event` writes to stdout. The daemon sends
  events, the frontend's channel renders them. Most
  architecturally clean but may be over-engineered for
  Phase 18.

Initial lean: **(a)**. `render_for_cli()` was purpose-built
in Phase 16 for exactly this use case. The in-process
rendering pipeline and `render_for_cli` produce equivalent
output for all current `StreamEvent` variants.

### Q3 — How does the banner work in daemon mode?

The in-process REPL prints a banner before the first prompt
(version, fs sandbox path, memory status, audit event count,
active role). In daemon mode, some of this information lives
in the daemon's address space (audit event count, role) and
some in the frontend's (version for display purposes).

- **(a) Frontend prints its own banner from locally available
  information.** The frontend knows the version, the socket
  path, and can get the daemon version from `DaemonReady`.
  It constructs a daemon-mode banner that reports these plus
  "connected to daemon at <path>."
- **(b) Daemon sends a banner in `SessionStarted`.** The
  `SessionStarted` message gains an optional `banner` field
  that the daemon populates with the same information the
  in-process banner reports. The frontend renders it
  verbatim.
- **(c) No banner in daemon mode.** Simplest; the prompt
  alone indicates readiness.

Initial lean: **(a)**. A local banner avoids protocol
changes and keeps `SessionStarted` simple. The daemon-mode
banner doesn't need to match the in-process banner exactly —
it just needs to tell the operator "you're connected to a
daemon."

### Q4 — How does ctrl-C cancellation work in daemon mode?

The in-process REPL uses `CancellationToken` rotation:
first ctrl-C cancels the in-flight turn, second ctrl-C
exits the process. In daemon mode, the turn runs in the
daemon's address space.

- **(a) Frontend sends `CancelTurn` over IPC.** First ctrl-C
  sends `FrontendMessage::CancelTurn`; the daemon's
  `IpcChannelBridge` propagates the cancellation to the
  agent's `CancellationToken`. Second ctrl-C exits the
  frontend process. The `CancelTurn` message is already
  defined in the Phase 16 protocol.
- **(b) Frontend just disconnects on second ctrl-C.** No
  explicit cancellation; the daemon detects the broken
  connection and cleans up. Simpler but worse UX — the
  first ctrl-C doesn't cancel the turn, it kills the
  frontend.
- **(c) Defer cancellation.** Phase 18 doesn't implement
  turn cancellation in daemon mode. First ctrl-C exits the
  frontend; the daemon finishes the turn to nobody and the
  next connection picks up a clean state.

Initial lean: **(a)**. `CancelTurn` is already in the
protocol; wiring it to ctrl-C is straightforward. The
double-ctrl-C UX mirrors the in-process path.

## Draft task breakdown

Five tasks, same cadence as Phases 14–17. Task 4 is the
working-session slot; Task 5 is exit freeze.

### Task 1 — Open commit (this document)

The phase's first commit is this doc, the `docs/README.md`
row flip from Frozen (Phase 17) to Active (Phase 18), and
the `docs/ROADMAP.md` Phase 18 entry update. No code, no
tests, no test-delta requirement.

**Acceptance:**

- `docs/PHASE_18.md` exists with the structure of this
  document.
- `docs/README.md` phase-status table has a Phase 18 row
  marked Active.
- `docs/ROADMAP.md` Phase 18 entry updated from scaffold
  to active description.
- Commit message: `docs(phase-18): open — Daemon Migration
  Frontend Wiring phase 3 of N`.

### Task 2 — Daemon-backed REPL loop

**Delivers:** a `run_daemon_session` function (or equivalent)
in the binary that auto-spawns a daemon, connects via
`DaemonSession`, reads lines from stdin, calls `submit_input`
for each, and renders `StreamEventPayload`s to stdout via
`render_for_cli()`.

**Cut:**

- New function in the binary (or a library module if size
  warrants it) that implements the daemon-backed REPL loop.
- The function prints a daemon-mode banner, then enters
  a read-line / submit / render loop identical in UX to
  `run_session`.
- Auto-spawn: if `daemon_is_running` returns false, call
  `spawn_daemon_and_wait` before connecting.
- On `DaemonSession::connect` failure after auto-spawn,
  fall back to in-process `run_session` with a warning.
- Rendering: `render_for_cli()` output written to stdout.

**Test count target:** +2 (daemon-backed REPL integration
test with `FakeStreamingAgent`, fallback-to-in-process test
or banner-content test).

**Acceptance:**

- The daemon-backed REPL loop works end-to-end in a test.
- `render_for_cli()` output matches expected format.
- Q1 and Q2 resolved and recorded.

### Task 3 — Binary dispatch wiring + ctrl-C

**Delivers:** the binary's `CliMode::Session` path wired to
the daemon-backed REPL as the default local-mode path, with
ctrl-C cancellation via `CancelTurn`.

**Cut:**

- The `CliMode::Session` + `ChannelKind::Local` branch in
  `run_async` calls the daemon-backed REPL function instead
  of (or before falling back to) `run_session`.
- ctrl-C handling: first ctrl-C sends `CancelTurn` over IPC
  (via a new `DaemonSession::cancel_turn` method or by
  sending the frame directly); second ctrl-C exits the
  frontend.
- Banner for daemon mode (Q3).

**Test count target:** +2 (ctrl-C cancellation test if
testable without real signals, CLI dispatch test showing
daemon path is the default).

**Acceptance:**

- `aivyx` (default invocation) auto-spawns and connects to
  a daemon for local mode.
- ctrl-C cancels an in-flight turn in daemon mode.
- Q3 and Q4 resolved and recorded.

### Task 4 — Working-session slot

Reserved for mid-implementation correction. Candidates:

- **Edge cases and polish.** Daemon startup failure UX,
  fallback messaging, socket-path edge cases (read-only
  filesystem, missing `$XDG_RUNTIME_DIR`).
- **Session marker in daemon mode.** The in-process path
  writes a session marker to redb for cross-restart
  continuity. The daemon-backed path may need equivalent
  marker handling.
- **Binary line-count management.** If the binary grows
  past 2300 lines, lift daemon-frontend dispatch code
  into a library module.

None is pre-committed. Task 4 opens with a review of
what Tasks 2–3 surfaced.

### Task 5 — Exit freeze

Same shape as Phase 14–17 exit freezes:

- Ship records for Tasks 1–3 (and Task 4 if used).
- Decisions block recording how Q1–Q4 resolved.
- Deferrals block.
- Final Exit criteria checklist.
- `docs/README.md` phase-status row flipped.
- `docs/ROADMAP.md` Phase 18 entry replaced with frozen
  summary; Phase 19 scaffold.
- `docs/PRODUCT_ROADMAP.md` Daemon Migration milestone
  updated.
- Prediction-versus-reality block.

**Acceptance:**

- All task ship records and decisions block in this
  document.
- `cargo test --workspace` green at exit. Test delta
  across the full phase **≥ +4** against the 542-test
  entry baseline.
- `cargo clippy --workspace --tests -- -D warnings`
  clean.
- DESIGN.md byte-identical to `e0d6437` **or** a
  documented amendment. Streak either extends to
  eighteen consecutive phases or breaks with reason.
- PRODUCT.md byte-identical to `80189b4`. Streak
  extends to **six consecutive phases.**
- `aivyx-core/src/lib.rs` byte-identical to `ba9a724`.
  Streak extends to **seven consecutive phases.**
- Zero-new-dep streak holds.
- `docs/README.md` phase-status table reflects exit.
- `docs/ROADMAP.md` Phase 18 frozen, Phase 19 scaffold.
- `docs/PRODUCT_ROADMAP.md` updated.
- Prediction-versus-reality block recorded.

## Decisions made at phase open

1. **Phase 18 is Daemon Migration phase 3 of N.** The
   scope is frontend wiring only — connecting the existing
   daemon substrate to the binary's default local-mode
   dispatch path. No new protocol messages, no new daemon
   features, no new library crates.
2. **The in-process path is retained as fallback.** Phase 18
   does not delete `run_session` or the in-process
   `CliMode::Session` → `ChannelKind::Local` path. It adds
   a daemon-backed alternative that becomes the default,
   with silent fallback to in-process on failure.
3. **The Q-block is small (four questions) because the
   architectural decisions are already made.** Unlike
   Phases 16 (protocol) and 17 (production hardening),
   Phase 18's questions are about UX and dispatch mechanics,
   not architecture.
4. **Conservative scope continues.** Phase 18 could try to
   also port Telegram, add `daemon status`/`stop`, or
   implement multi-connection. It does none of these. The
   single deliverable is: `aivyx` auto-spawns and connects
   to a daemon for local mode.

## Decisions made during implementation

### Task 2 — Q1 resolution: try-connect-then-spawn (variant of option b)

**Resolved:** a variant of **(b)** (sit alongside with
auto-detection), but with a connect-first strategy instead
of a probe-first strategy. The `run_daemon_session` function
attempts `DaemonSession::connect` directly; if the connect
fails (no daemon listening), it calls `spawn_daemon_and_wait`
and retries the connect. This avoids the architectural problem
discovered during implementation: `daemon_is_running()` opens
a probe connection that consumes the daemon's single-connection
slot, causing the subsequent `DaemonSession::connect` to find
no listener accepting.

The in-process `run_session` remains in the codebase. The
binary dispatch wiring (Task 3) will choose between
`run_daemon_session` and `run_session` at runtime.

### Task 2 — Q2 resolution: render_for_cli() directly to writer (option a)

**Resolved:** option **(a)**. The `run_daemon_session` function
calls `StreamEventPayload::render_for_cli()` for each received
event and writes the result to the provided `Write` sink. The
`render_for_cli()` helper was purpose-built in Phase 16 for
exactly this use case. No conversion back to `StreamEvent` is
needed.

### Task 2 — implementation shape

**New module `daemon_session.rs`** (136 lines) in
`crates/aivyx-channel/src/`:

- **`DaemonSessionConfig` struct** — socket path, role, prompt
  string, optional banner.
- **`run_daemon_session<R, W>` function** — generic over `BufRead`
  and `Write` (same pattern as `run_session`). Connects to
  daemon (with try-connect-then-spawn fallback), prints banner,
  enters read-line / `submit_input` / `render_for_cli` loop,
  returns `SessionReport`.
- **`outcome_str_to_turn_outcome` helper** — best-effort parse
  of the daemon's `format_outcome` string back into a
  `TurnOutcome` (lossy — recovers variant and message but not
  metadata like duration or tool count).

**Two new integration tests** in `daemon_roundtrip_e2e.rs`:

1. **`run_daemon_session_renders_two_turns`** — spawns a daemon
   with `FakeStreamingAgent`, feeds two input lines, asserts
   banner, streamed text, prompt count, and turn count.
2. **`run_daemon_session_with_no_input_prints_banner_only`** —
   empty input (immediate EOF), asserts banner printed and
   zero turns run.

**Test delta:** +2 (workspace 542 → 544).

**Streak status after Task 2:**

- DESIGN.md byte-identical to `e0d6437`. Streak holds.
- PRODUCT.md byte-identical to `80189b4`. Streak holds.
- `aivyx-core/src/lib.rs` byte-identical to `ba9a724`. Streak
  holds. No trait changes.
- Zero-new-dep streak holds.

### Task 3 — Q3 resolution: frontend prints own daemon-mode banner (option a)

**Resolved:** option **(a)**. The frontend constructs a
daemon-mode banner from locally available information and
passes it to `DaemonSessionConfig.banner`. This avoids
protocol changes to `SessionStarted` and keeps the daemon
stateless with respect to presentation. The daemon-mode
banner is set by the binary's dispatch code, same as the
in-process path sets its own banner.

### Task 3 — Q4 resolution: first ctrl-C sends CancelTurn, second exits (option a)

**Resolved:** option **(a)**. The binary's daemon-mode
dispatch path spawns a `tokio::spawn` signal handler that:

1. On **first ctrl-C**: sends `CancelTurn` to the daemon via
   a `DaemonCancelHandle` (a cloneable handle wrapping
   `Arc<tokio::sync::Mutex<OwnedWriteHalf>>` + session ID).
   Prints a notice: "cancelling in-flight turn (ctrl-C again
   to exit)."
2. On **second ctrl-C**: exits the frontend with status 130.

This mirrors the in-process path's `CancellationToken`
rotation UX. The `CancelTurn` message was already defined
in the Phase 16 protocol; the daemon's `IpcChannelBridge`
propagates it to the agent's cancellation token.

### Task 3 — implementation shape

**`daemon_client.rs` changes** (267 → 318 lines):

- **Writer refactored to `Arc<tokio::sync::Mutex<OwnedWriteHalf>>`.**
  All write methods (`connect`, `submit_input`, `disconnect`)
  updated to lock the mutex. This enables shared access between
  the REPL loop and the signal handler without `&mut` exclusivity.
- **`cancel_turn(&mut self)` method** — sends `CancelTurn`
  for the current session.
- **`cancel_handle(&self) -> DaemonCancelHandle`** — returns
  a cloneable handle for use from signal handlers.
- **`DaemonCancelHandle` struct** — `Clone`, wraps
  `Arc<Mutex<OwnedWriteHalf>>` + session ID, has a
  `cancel(&self)` method that encodes and sends `CancelTurn`.

**`daemon_session.rs` changes** (136 → 167 lines):

- Split into three functions:
  - `run_daemon_session` — auto-connecting (try-connect-then-spawn).
  - `run_daemon_session_connected` — accepts a pre-connected
    `DaemonSession`, for callers that need to extract a
    `cancel_handle` before entering the REPL loop.
  - `run_daemon_session_inner` — shared implementation.

**`aivyx.rs` changes** (2159 → 2225 lines):

- `ChannelKind::Local` branch: tries daemon mode first via
  `DaemonSession::connect`. On success, extracts a cancel
  handle, spawns a ctrl-C signal handler task, runs
  `run_daemon_session_connected`. On failure, falls back
  silently to in-process `run_session` (existing Phase 3
  code path).

**One new integration test** in `daemon_roundtrip_e2e.rs`:

- **`run_daemon_session_connected_with_cancel_handle`** —
  pre-connects a `DaemonSession`, extracts a cancel handle,
  verifies the handle is `Clone`, runs one turn through
  `run_daemon_session_connected`, asserts events and turn
  count.

**Test delta:** +1 (workspace 544 → 545).

**Streak status after Task 3:**

- DESIGN.md byte-identical to `e0d6437`. Streak holds.
- PRODUCT.md byte-identical to `80189b4`. Streak holds.
- `aivyx-core/src/lib.rs` byte-identical to `ba9a724`. Streak
  holds. No trait changes — `TurnOutcome` variants read but
  not modified.
- Zero-new-dep streak holds.

### Task 4 — cancel-flag reset bug fix

**Bug discovered:** the daemon-mode ctrl-C signal handler
used a local `mut bool` (`cancelled_once`) captured by move
into `tokio::spawn`. Once the operator cancelled a turn, the
flag stayed `true` permanently — the next turn's first ctrl-C
would exit the process instead of sending `CancelTurn`. The
in-process path avoids this by rotating `CancellationToken`s
each turn, which implicitly resets the "already cancelled"
state.

**Fix:** replaced the local `bool` with a shared
`Arc<AtomicBool>`. Added `cancel_flag: Option<Arc<AtomicBool>>`
to `DaemonSessionConfig`. The REPL loop in
`run_daemon_session_inner` resets the flag to `false` (via
`Relaxed` store) before each `submit_input` call. The binary
creates the `AtomicBool`, shares one `Arc` with the signal
handler task and another with the config. Tests that don't
need cancellation pass `cancel_flag: None`.

**One new integration test** in `daemon_roundtrip_e2e.rs`:

- **`cancel_flag_resets_between_turns`** — sets cancel flag to
  `true` (simulating a prior cancel), runs a two-turn session
  via `run_daemon_session_connected`, asserts the flag is
  `false` after the session completes (proving the REPL reset
  it before each turn).

**Test delta:** +1 (workspace 545 → 546).

**Streak status after Task 4:**

- DESIGN.md byte-identical to `e0d6437`. Streak holds.
- PRODUCT.md byte-identical to `80189b4`. Streak holds.
- `aivyx-core/src/lib.rs` byte-identical to `ba9a724`. Streak
  holds.
- Zero-new-dep streak holds.

## Ship records

### Task 1 — Open commit

- **Commit:** `b7c6fc2`
- **Shape:** `docs/PHASE_18.md` (this document), `docs/README.md`
  phase-status flip, `docs/ROADMAP.md` Phase 18 entry update.
- **Test delta:** +0 (542 → 542).

### Task 2 — Daemon-backed REPL loop

- **Commit:** `d91ac64`
- **Shape:** new `daemon_session.rs` module (136 lines) with
  `DaemonSessionConfig`, `run_daemon_session`, and
  `outcome_str_to_turn_outcome`. Two new e2e tests
  (`run_daemon_session_renders_two_turns`,
  `run_daemon_session_with_no_input_prints_banner_only`).
- **Decisions:** Q1→variant of (b) try-connect-then-spawn,
  Q2→(a) `render_for_cli()` directly to writer.
- **Test delta:** +2 (542 → 544).

### Task 3 — Binary dispatch wiring + ctrl-C cancellation

- **Commit:** `66018dc`
- **Shape:** `daemon_client.rs` writer refactored to
  `Arc<Mutex<OwnedWriteHalf>>`, `DaemonCancelHandle` struct,
  `cancel_turn` and `cancel_handle` methods. `daemon_session.rs`
  split into three functions (`run_daemon_session`,
  `run_daemon_session_connected`, `run_daemon_session_inner`).
  `aivyx.rs` `ChannelKind::Local` branch wired to daemon-first
  dispatch with ctrl-C signal handler. One new e2e test
  (`run_daemon_session_connected_with_cancel_handle`).
- **Decisions:** Q3→(a) frontend prints own banner, Q4→(a) first
  ctrl-C sends `CancelTurn` via `DaemonCancelHandle`, second exits.
- **Test delta:** +1 (544 → 545).

### Task 4 — Cancel-flag reset bug fix

- **Commit:** `1a24b64`
- **Shape:** replaced local `bool` with `Arc<AtomicBool>` shared
  between signal handler and REPL loop. Added `cancel_flag` field
  to `DaemonSessionConfig`. REPL loop resets flag before each
  `submit_input`. One new e2e test (`cancel_flag_resets_between_turns`).
- **Bug fixed:** `cancelled_once` flag never reset between turns —
  after cancelling one turn, next turn's first ctrl-C exited instead
  of cancelling.
- **Test delta:** +1 (545 → 546).

### Task 5 — Exit freeze (this section)

- **Commit:** this commit.
- **Shape:** ship records, deferrals, prediction-vs-reality, exit
  criteria in this document. `docs/README.md` phase-status flip.
  `docs/ROADMAP.md` Phase 18 frozen, Phase 19 scaffold.
  `docs/PRODUCT_ROADMAP.md` milestone update.
- **Test delta:** +0 (546 → 546).

**Phase test delta: +4** (542 → 546), meeting the ≥ +4 target.

## Deferrals

**Rolling deferrals still open after Phase 18 (inherited):**

- **Forensic `ToolOutcome::NotInRole` variant** —
  Phase 11 Q1 deferral, untouched by Phase 18.
  Carries forward. Tagged: **Phase 11 Task 4,
  earliest plausible: whichever phase has a concrete
  forensic-tooling story.**
- **Second regression channel for the role
  primitive** — Phase 11 Q6 deferral. Untouched by
  Phase 18.
- **Response headers in audit payload** (Phase 12
  Q3 half). Untouched by Phase 18.
- **Non-GET verbs (POST/PUT/PATCH/DELETE).** Phase
  12 Q1 pinned GET-only. Deferred indefinitely.
- **Redirect following with per-hop scope re-check.**
  Phase 12 Q5 pinned `Policy::none()`. Deferred
  indefinitely.
- **Binary response bodies / non-UTF-8.** Deferred
  indefinitely.
- **Per-chunk Telegram rendering.** Phase 12 Task 1.
  Deferred reactively.
- **`CapabilitySet::grants` reflexivity
  investigation.** Phase 13 Task 4 deferral.
  Untouched by Phase 18.
- **Misleading `CEILING_SEMITRUSTED` ▲-row doc
  comment.** Phase 15 Task 4. Composes with the
  reflexivity investigation.
- **Multi-level sub-agent nesting.** Phase 14 Task 3.
  Untouched by Phase 18.
- **Telegram-over-daemon port.** Phase 16 net-new.
  Untouched by Phase 18. Tagged: **earliest
  plausible: Phase 19 or later.**
- **LocalChannel regression-test rewrite over IPC.**
  Phase 17 Q6→(c+). Untouched by Phase 18. Tagged:
  **reactive — reopens if an IPC-boundary-specific
  bug surfaces that the in-process tests miss.**
- **`daemon status` / `daemon stop` subcommands.**
  Phase 17 net-new. Untouched by Phase 18.
- **PID file at `$XDG_RUNTIME_DIR/aivyx/daemon.pid`.**
  Phase 17 net-new. Untouched by Phase 18.

**Inherited deferrals closed by Phase 18:**

- **REPL-mode frontend over IPC.** Phase 17 net-new.
  **Closed by Tasks 2–3** — `run_daemon_session` and
  `run_daemon_session_connected` deliver the REPL loop
  over IPC, and the binary's `ChannelKind::Local`
  branch wires daemon-first dispatch with fallback.

**Net-new deferrals from Phase 18 itself:**

- **`--no-daemon` flag for local mode.** Listed as a
  non-goal in the phase open doc. The in-process path
  is reachable as a fallback on daemon connection
  failure, but there is no explicit operator flag to
  force it. Tagged: **earliest plausible: reactive —
  adds if operators ask for it.**
- **Daemon-mode banner parity with in-process banner.**
  The daemon-mode banner reports version, socket path,
  and active role. The in-process banner additionally
  reports fs sandbox path, memory status, and audit
  event count. Parity deferred because the daemon-mode
  banner meets the operator's immediate need ("am I
  connected?") and full parity would require either
  protocol extension or local recalculation. Tagged:
  **earliest plausible: reactive.**

**Backlog shape at Phase 18 exit:** fourteen rolling items
inherited (fifteen inherited, one closed by Phase 18) +
two net-new from Phase 18 itself. Total **sixteen**. The
REPL-mode frontend deferral — the primary target of
Phase 18 — is closed.

## Prediction versus reality

The Phase 18 open doc made four explicit streak-risk
predictions. This block reckons with each.

### Prediction 1: "DESIGN.md streak (17 → 18, not at risk)"

**Reality: correct.** No trait, no architectural decision,
no protocol change. The daemon-backed REPL is pure wiring
of existing library code. DESIGN.md byte-identical to
`e0d6437`, streak at **eighteen consecutive phases.**

### Prediction 2: "PRODUCT.md streak (5 → 6, not at risk)"

**Reality: correct.** P4's deliberate-silence clause
continues to absorb all daemon frontend wiring decisions.
PRODUCT.md byte-identical to `80189b4`, streak at **six
consecutive phases.**

### Prediction 3: "Production-core aivyx-core/src/lib.rs streak (6 → 7, not at risk)"

**Reality: correct.** All changes were in `daemon_client.rs`,
`daemon_session.rs`, `aivyx.rs`, and the e2e test file.
`TurnOutcome` variants were read (in `outcome_str_to_turn_outcome`)
but not modified. `aivyx-core/src/lib.rs` byte-identical to
`ba9a724`, streak at **seven consecutive phases** (longest
production-core streak in project history, extending Phase
17's record).

### Prediction 4: "Zero-new-dep streak (not at risk)"

**Reality: correct.** No new workspace dependencies added.
All pieces (`tokio`, `tokio::signal`, `std::sync::atomic`)
were already in the workspace.

### Phase open "honest position" assessment

The open doc called Phase 18 "the lowest-streak-risk phase
in the entire daemon arc." This proved correct — all four
predictions held, and the only surprise was the `cancelled_once`
bug discovered in Task 4, which was a local state-management
issue in the binary, not an architectural concern. The
discovery-and-fix pattern validated the Task 4 working-session
slot: the reserve capacity absorbed a real bug rather than
going unused.

## Exit criteria

- [x] All task ship records in this document (Tasks 1–4
      above, Task 5 is this section).
- [x] Decisions block: Q1→variant of (b), Q2→(a), Q3→(a),
      Q4→(a). All four recorded in "Decisions made during
      implementation" above.
- [x] `cargo test --workspace` — **546 green**. Test delta
      **+4** against 542-test entry baseline, meeting the
      ≥ +4 target.
- [x] `cargo clippy --workspace --tests -- -D warnings`
      clean.
- [x] DESIGN.md byte-identical to `e0d6437`. **Streak at
      eighteen consecutive phases.**
- [x] PRODUCT.md byte-identical to `80189b4`. **Streak at
      six consecutive phases.**
- [x] `aivyx-core/src/lib.rs` byte-identical to `ba9a724`.
      **Streak at seven consecutive phases.**
- [x] Zero-new-dep streak holds.
- [x] `docs/README.md` phase-status table reflects exit.
- [x] `docs/ROADMAP.md` Phase 18 frozen, Phase 19 scaffold.
- [x] `docs/PRODUCT_ROADMAP.md` updated.
- [x] Prediction-versus-reality block recorded.
- [x] Deferrals block recorded. Rolling backlog at sixteen
      items (one closed, two net-new).
