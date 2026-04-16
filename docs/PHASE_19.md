# Phase 19 — Daemon Migration: Multi-Connection + Telegram Port (phase 4 of N)

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Upgrade the daemon from single-connection to multi-connection,
then port the Telegram adapter behind the IPC boundary. By
Phase 19 exit, the daemon accepts concurrent frontends (CLI +
Telegram + future adapters) and the `aivyx --channel telegram`
path auto-attaches to a running daemon the same way
`aivyx --channel local` does after Phase 18.

Phase 19 is **Daemon Migration phase 4 of N**. Phase 16
settled the protocol; Phase 17 hardened the daemon; Phase 18
wired the CLI frontend. Phase 19 proves the protocol works
for a non-interactive, long-polling channel adapter — the
first real test of whether the daemon architecture generalizes
beyond local-mode.

## Why now

1. **Multi-connection is the next load-bearing architectural
   question.** The daemon's single-connection model (Phase 17
   non-goal, Phase 18 non-goal) must be solved before any
   second adapter can be ported. Telegram is the concrete
   forcing function.

2. **Telegram is the only non-local adapter in tree.** It
   shipped in Phase 8 (single-chat) and Phase 9 (multi-chat
   multiplexer). It has its own in-process agent stack, its own
   session function, its own `SessionConfig` analogue. Porting
   it behind the daemon means the daemon owns the agent stack,
   the audit chain, and the memory store — and the Telegram
   frontend becomes a thin long-poll + IPC bridge, same as the
   CLI frontend became in Phase 18.

3. **The Telegram-over-daemon deferral has been carried since
   Phase 16.** It was tagged "earliest plausible: Phase 19 or
   later." Phase 19 is that phase.

4. **The protocol is ready.** `FrontendMessage`, `DaemonMessage`,
   `DaemonLifecycleEvent`, and `StreamEventPayload` already
   carry everything a Telegram frontend needs: `SubmitInput`
   for inbound messages, `StreamEvent` for outbound text,
   `CancelTurn` for `/cancel` commands. The only protocol gap
   is session routing for multi-connection — the daemon must
   know which session's events go to which connection.

## Non-goals

- **No new channel adapters.** Only Telegram is ported. Discord,
  Slack, Matrix, Email remain forward concerns.
- **No wire-format upgrade.** Same as Phases 16–18. The
  length-prefixed JSON framing is sufficient.
- **No DESIGN.md or PRODUCT.md edits.** Same streak-protection
  discipline as Phases 16–18.
- **No `daemon status` / `daemon stop` subcommands.** Same
  Phase 17 deferral, still no urgency.
- **No Telegram-specific protocol extensions.** The Telegram
  frontend communicates with the daemon using the same
  `FrontendMessage` / `DaemonMessage` vocabulary the CLI
  frontend uses. If Telegram needs something the protocol
  doesn't have (e.g., attachment delivery), that's a deferral,
  not a Phase 19 scope expansion.
- **No multi-chat multiplexing inside the daemon.** The Phase 9
  multi-chat pump (`run_telegram_multi_session`) currently
  owns the `get_updates` cursor and routes to per-chat inner
  tasks. Phase 19's Telegram frontend retains this
  multiplexer — each per-chat turn is submitted to the daemon
  as a separate `SubmitInput`. The daemon doesn't need to know
  about Telegram's multi-chat semantics; it sees individual
  turn requests from a connected frontend.

## Entry criteria (all met from Phase 18 exit)

- [x] Phase 18 frozen at exit commit `6dd4f23` + hash
      backfill `24a8b84`. See PHASE_18.md.
- [x] `cargo test --workspace` is **546 green** (verified
      at Phase 18 exit, baseline for Phase 19's delta math).
- [x] `cargo clippy --workspace --tests -- -D warnings`
      clean at Phase 18 exit.
- [x] `crates/aivyx-core/src/lib.rs` byte-identical to
      `ba9a724`. **Streak at seven consecutive phases.**
- [x] `docs/DESIGN.md` byte-identical to `e0d6437`.
      **Streak at eighteen consecutive phases.**
- [x] `PRODUCT.md` byte-identical to `80189b4`. **Streak
      at six consecutive phases.**
- [x] Daemon substrate in place:
      - `daemon_server.rs` (307 lines) — single-connection
        multi-turn server.
      - `daemon_client.rs` (318 lines) — `DaemonSession`,
        `DaemonCancelHandle`, `spawn_daemon_and_wait`.
      - `daemon_session.rs` (179 lines) — `run_daemon_session`,
        `run_daemon_session_connected`, cancel-flag reset.
      - `daemon_ipc.rs` (505 lines) — protocol types,
        `render_for_cli()`.
      - `aivyx.rs` (2231 lines) — daemon-first dispatch for
        `ChannelKind::Local`, in-process fallback, ctrl-C
        cancellation.
      - `daemon_roundtrip_e2e.rs` — 10 e2e tests.
- [x] Telegram adapter in tree:
      - `aivyx-telegram/src/session.rs` (1108 lines) — single-
        chat and multi-chat session functions.
      - `aivyx-telegram/src/telegram_channel.rs` —
        `TelegramChannel<T>` implementing `ChannelContext`.
      - `aivyx-telegram/src/transport.rs` — `TelegramTransport`
        trait, `ReqwestTransport`, `ScriptedTransport`.
      - Binary's `ChannelKind::Telegram` branch — in-process
        agent-stack construction + long-poll session.
- [x] Rolling deferral backlog at sixteen items (see
      PHASE_18.md deferrals block). Phase 19 targets closing
      the Telegram-over-daemon deferral.

## Streaks at risk

- **DESIGN.md streak (18 → 19, low risk but not zero).**
  Multi-connection dispatch is the first structural change to
  the daemon since Phase 17. If the multi-connection model
  requires a new `ChannelContext` method, a new trait, or a
  new architectural invariant, DESIGN.md may need an amendment.
  The mitigation argument: the `IpcChannelBridge` pattern
  already isolates per-connection state; multi-connection is
  "accept in a loop" rather than "accept once." If the bridge
  pattern holds, no DESIGN.md edit is needed.

- **PRODUCT.md streak (6 → 7, not at risk).** Same
  protection as Phases 16–18: P4's deliberate-silence clause
  absorbs daemon dispatch decisions.

- **Production-core `aivyx-core/src/lib.rs` streak (7 →
  8, low risk).** The Telegram adapter's `ChannelContext`
  impl already exists in `aivyx-telegram`. Porting it behind
  the daemon's `IpcChannelBridge` should not require core-type
  changes. The risk scenario: if the daemon needs to know the
  platform or trust tier of a connected frontend (for routing
  or capability decisions), and that information isn't in the
  protocol, a new `FrontendMessage` variant might cascade into
  a core type. Mitigation: the protocol already carries `role`
  in `StartSession`, and the daemon's agent-stack wiring reads
  roles from the config file, not from the frontend.

- **Zero-new-dep streak (not at risk).** All pieces are
  already in the workspace.

**Honest position:** Phase 19 carries more streak risk than
Phase 18 because multi-connection is a structural change, not
pure wiring. The DESIGN.md streak is the one to watch. The
production-core streak is probably safe because the Telegram
adapter is already a complete `ChannelContext` impl — the
daemon just needs to host it, not reshape it.

## Open questions

### Q1 — How does the daemon dispatch multiple connections?

The daemon currently accepts one connection at line 60 of
`daemon_server.rs` and blocks until that connection
disconnects. Multi-connection requires accepting in a loop
and routing per-connection traffic.

- **(a) Spawn a task per connection.** `listener.accept()` in
  a loop; each connection gets a `tokio::spawn`'d handler task
  that reads `FrontendMessage` frames and dispatches turns
  through the shared `Agent`. The agent is `Arc<dyn Agent>`,
  already shared. The `IpcChannelBridge` is per-connection.
  Session IDs distinguish traffic. The daemon's existing
  `shutdown` token cancels all tasks.
- **(b) Fixed connection slots.** Pre-allocate N connection
  slots (e.g., 2: one for local CLI, one for Telegram). Reject
  connections beyond the limit. Simpler than unbounded (a) but
  artificially constrains future adapters.
- **(c) Single connection with multiplexed sessions.** Keep
  one connection but allow multiple `StartSession` calls on it.
  Each session gets its own session ID; events are tagged by
  session. Avoids multi-connection complexity but requires the
  Telegram frontend to be in the same process as the CLI
  frontend.

Initial lean: **(a)**. The tokio task-per-connection pattern
is idiomatic and scales to any number of frontends. The daemon
already has `Arc<dyn Agent>` and `Arc<C: ChannelContext>`, so
sharing is natural. The shutdown token cancels all tasks
uniformly.

### Q2 — How does the Telegram frontend submit turns to the daemon?

The Telegram adapter currently constructs its own
`ConcreteAgent` in `run_telegram_session_with_transport` and
calls `agent.turn(message, &channel)` directly. In daemon
mode, the frontend doesn't own the agent — it submits
`SubmitInput` over IPC and receives `StreamEvent` / `TurnComplete`
frames.

- **(a) Thin IPC bridge replaces the agent stack.** The
  Telegram frontend retains the multi-chat multiplexer (outer
  `get_updates` loop, per-chat `mpsc` routing) but each inner
  task, instead of calling `agent.turn()`, connects to the
  daemon via `DaemonSession` and calls `submit_input()`. The
  streamed events are forwarded to the `TelegramChannel` for
  rendering (via `send_message`). `/cancel` is forwarded as
  `CancelTurn`.
- **(b) New `TelegramDaemonFrontend` module.** A purpose-built
  Telegram frontend that owns the `get_updates` cursor and
  submits turns to the daemon, without reusing the existing
  multi-chat pump. Simpler code but duplicates the multiplexing
  logic.
- **(c) Daemon owns the long-poll loop.** The daemon itself
  calls `get_updates` and dispatches inbound messages as turns.
  The Telegram "frontend" is just configuration — no separate
  process or connection. Most architecturally clean but
  couples the daemon to Telegram's transport, violating the
  daemon-as-agent-host / frontend-as-transport separation.

Initial lean: **(a)**. The existing multi-chat multiplexer
is well-tested (Phase 8 + Phase 9) and its structure —
outer poll loop, per-chat mailbox, inner turn task — maps
directly onto the daemon model. Each inner task replaces
`agent.turn()` with `daemon_session.submit_input()`.

### Q3 — How does the Telegram frontend render streamed events?

The in-process Telegram path renders events through
`TelegramChannel::stream_event`, which calls
`transport.send_message(chat_id, text)`. In daemon mode,
the frontend receives `StreamEventPayload` frames over IPC.

- **(a) Forward `StreamEventPayload` to `send_message`.**
  Each `StreamEventPayload::Text` becomes a `send_message`
  call. Status and tool events are rendered to text first
  (using `render_for_cli` or a Telegram-specific renderer).
  Matches the current in-process rendering fidelity.
- **(b) Accumulate events per turn, send one message.** Buffer
  all events until `TurnComplete`, then send the full response
  as a single Telegram message. Simpler; avoids partial-message
  delivery; matches how most Telegram bots behave. Loses
  streaming UX (the user sees nothing until the turn finishes).
- **(c) Batch events with a debounce timer.** Accumulate
  events for up to N ms, then flush. Compromise between (a)'s
  per-event chattiness and (b)'s all-at-once delivery.

Initial lean: **(b)** for Phase 19. The in-process Telegram
path already sends one message per turn (the channel's
`finalize` method sends the final text; `stream_event` for
`Text` is a no-op in the Telegram adapter because token-by-
token streaming doesn't make sense over a chat API). Matching
this behavior is the simplest first step. Per-event streaming
(a) is a forward refinement.

### Q4 — Does the daemon need to know the frontend type?

The daemon currently treats every connection identically. A
Telegram frontend connecting via `DaemonSession` looks the
same as a CLI frontend.

- **(a) No — the daemon is frontend-agnostic.** All frontends
  speak the same protocol. The daemon routes turns through the
  shared agent regardless of who submitted them. The frontend
  type (CLI vs. Telegram vs. future) is an opaque detail.
- **(b) Yes — `StartSession` gains a `frontend_type` field.**
  The daemon uses this to select the `ChannelContext`
  implementation (e.g., `LocalChannel` for CLI,
  `TelegramChannel` for Telegram). This matters if the
  daemon's `ChannelContext` influences the agent's behavior
  (trust tier, platform, stream events).
- **(c) Yes — the `role` field in `StartSession` implicitly
  carries the frontend type.** Telegram sessions use a
  Telegram-specific role; CLI sessions use a local-specific
  role. The daemon resolves the `ChannelContext` from the role.

Initial lean: **(b)**. The daemon's `IpcChannelBridge`
wraps an inner `ChannelContext` that determines trust tier,
platform, and channel name. A Telegram frontend should run
under `SemiTrusted` (Telegram's trust tier) rather than
`Trusted` (LocalChannel's trust tier). The daemon needs to
construct the right inner channel for each connection, which
requires knowing the frontend type.

### Q5 — How does per-connection channel construction work?

Currently the daemon receives `Arc<C: ChannelContext>` as a
parameter and wraps it in `IpcChannelBridge` for every turn.
Multi-connection with different channel types requires a way
to construct a per-connection `ChannelContext`.

- **(a) Channel factory closure.** The daemon receives a
  closure `Fn(frontend_type) -> Arc<dyn ChannelContext>` and
  calls it per connection. The binary constructs the closure
  at startup, capturing the resources each channel type needs.
- **(b) Enum dispatch.** The daemon holds pre-constructed
  channels for each known frontend type and selects at
  connection time.
- **(c) Trait object with `Clone`.** The daemon holds one
  `Arc<dyn ChannelContext>` per frontend type in a map.

Initial lean: **(a)**. A factory closure matches the
existing `planner_factory` pattern on `ConcreteAgent` and
keeps the daemon generic over channel construction.

## Draft task breakdown

Five tasks, same cadence as Phases 14–18. Task 4 is the
working-session slot; Task 5 is exit freeze.

### Task 1 — Open commit (this document)

The phase's first commit is this doc, the `docs/README.md`
row flip from Frozen (Phase 18) to Active (Phase 19), and
the `docs/ROADMAP.md` Phase 19 entry update. No code, no
tests, no test-delta requirement.

**Acceptance:**

- `docs/PHASE_19.md` exists with the structure of this
  document.
- `docs/README.md` phase-status table has a Phase 19 row
  marked Active.
- `docs/ROADMAP.md` Phase 19 entry updated from scaffold
  to active description.
- Commit message: `docs(phase-19): open — Daemon Migration
  Multi-Connection + Telegram Port phase 4 of N`.

### Task 2 — Multi-connection daemon server

**Delivers:** the daemon accepts multiple concurrent
connections, each served by a spawned task, with shared
`Arc<dyn Agent>` and per-connection `IpcChannelBridge`.

**Cut:**

- `run_daemon` refactored from "accept one, serve, return"
  to "accept in a loop, spawn a handler task per connection."
- Each handler task reads `FrontendMessage` frames and
  dispatches turns through the shared agent.
- `StartSession` gains an optional `frontend_type` field
  (Q4). The daemon uses this to construct the appropriate
  `ChannelContext` per connection (Q5).
- Graceful shutdown: the `shutdown` token cancels all
  handler tasks. In-flight turns complete before the handler
  exits.
- The `run_poc_daemon` backward-compat alias is updated or
  removed.

**Test count target:** +2 (two-connection concurrent test,
connection-after-disconnect test).

**Acceptance:**

- The daemon serves two connections concurrently in a test.
- Q1, Q4, and Q5 resolved and recorded.

### Task 3 — Telegram frontend over IPC

**Delivers:** the `aivyx --channel telegram` path connects
to the daemon (auto-spawning if needed) and submits turns
over IPC instead of constructing an in-process agent stack.

**Cut:**

- New Telegram frontend module (or modifications to the
  existing multi-chat pump) that replaces `agent.turn()`
  with `DaemonSession::submit_input()`.
- Inbound `get_updates` loop retained in the frontend.
- Outbound `StreamEventPayload` events accumulated and
  forwarded to `TelegramChannel::send_message` (or a new
  rendering path).
- `/cancel` forwarded as `CancelTurn`.
- Binary's `ChannelKind::Telegram` branch wired to the
  daemon path (same daemon-first + in-process-fallback
  pattern as the CLI path).

**Test count target:** +2 (Telegram-over-daemon e2e test
with scripted transport, Telegram `/cancel` over IPC test).

**Acceptance:**

- Telegram turns route through the daemon in a test.
- Q2 and Q3 resolved and recorded.

### Task 4 — Working-session slot

Reserved for mid-implementation correction. Candidates:

- **Edge cases and polish.** Connection cleanup on frontend
  crash, per-connection session ID isolation, daemon log
  output for multi-connection diagnostics.
- **Trust-tier enforcement.** Verifying that a Telegram
  frontend's `SemiTrusted` ceiling is correctly applied
  when turns run through the daemon.
- **Binary line-count management.** If the binary grows
  past 2400 lines, lift Telegram daemon-frontend code into
  a library module.

None is pre-committed. Task 4 opens with a review of
what Tasks 2–3 surfaced.

### Task 5 — Exit freeze

Same shape as Phase 14–18 exit freezes:

- Ship records for Tasks 1–3 (and Task 4 if used).
- Decisions block recording how Q1–Q5 resolved.
- Deferrals block.
- Final Exit criteria checklist.
- `docs/README.md` phase-status row flipped.
- `docs/ROADMAP.md` Phase 19 entry replaced with frozen
  summary; Phase 20 scaffold.
- `docs/PRODUCT_ROADMAP.md` Daemon Migration milestone
  updated.
- Prediction-versus-reality block.

**Acceptance:**

- All task ship records and decisions block in this
  document.
- `cargo test --workspace` green at exit. Test delta
  across the full phase **≥ +4** against the 546-test
  entry baseline.
- `cargo clippy --workspace --tests -- -D warnings`
  clean.
- DESIGN.md byte-identical to `e0d6437` **or** a
  documented amendment. Streak either extends to
  nineteen consecutive phases or breaks with reason.
- PRODUCT.md byte-identical to `80189b4`. Streak
  extends to **seven consecutive phases.**
- `aivyx-core/src/lib.rs` byte-identical to `ba9a724`.
  Streak extends to **eight consecutive phases.**
- Zero-new-dep streak holds.
- `docs/README.md` phase-status table reflects exit.
- `docs/ROADMAP.md` Phase 19 frozen, Phase 20 scaffold.
- `docs/PRODUCT_ROADMAP.md` updated.
- Prediction-versus-reality block recorded.

## Decisions made at phase open

1. **Phase 19 is Daemon Migration phase 4 of N.** The
   scope is multi-connection + Telegram port — the first
   test of whether the daemon architecture generalizes
   beyond local-mode. Two deliverables: a multi-connection
   daemon server, and a Telegram frontend that speaks IPC
   instead of constructing an in-process agent stack.
2. **The in-process Telegram path is retained as fallback.**
   Same pattern as Phase 18: daemon-first, in-process on
   failure. The existing `run_telegram_session` /
   `run_telegram_multi_session` code stays for tests and
   as a `--no-daemon` fallback if the daemon fails.
3. **The Q-block is larger than Phase 18 (five questions)
   because multi-connection introduces structural decisions.**
   Q1 (connection dispatch), Q4 (frontend type awareness),
   and Q5 (per-connection channel construction) are the
   load-bearing questions. Q2 (turn submission) and Q3
   (event rendering) are wiring questions.
4. **Conservative scope continues.** Phase 19 could also
   add `daemon status`/`stop`, multi-level sub-agent
   nesting, or deferral cleanup. It does none of these.
   The deliverable is: the daemon accepts multiple
   frontends and Telegram is the second.

## Decisions made at Task 2

5. **Q1 → (a) task-per-connection.** `run_daemon` refactored
   to accept in a loop and `tokio::spawn` a handler task per
   connection. The shared `Arc<dyn Agent>` and a cloned
   `ChannelFactory` are moved into each task. The daemon's
   `CancellationToken` is propagated per-connection for
   graceful shutdown. The `IpcChannelBridge` is per-connection,
   not shared.
6. **Q4 → (b) `StartSession` gains `frontend_type`.** A new
   `FrontendType` enum (`Local`, `Telegram`) is added to
   `daemon_ipc.rs`. `StartSession` carries an optional
   `frontend_type` field (defaults to `Local` for backward
   compat). The daemon dispatches through the channel factory
   using this value.
7. **Q5 → (a) channel factory closure.** A new type alias
   `ChannelFactory = Arc<dyn Fn(FrontendType) -> Arc<dyn
   ChannelContext + Send + Sync> + Send + Sync>` is the
   daemon's per-connection channel constructor. The binary
   constructs this closure at startup, matching `FrontendType`
   to the appropriate `ChannelContext` impl. The existing
   `planner_factory` pattern on `ConcreteAgent` is the
   precedent.
8. **`IpcChannelBridge` de-genericized.** Changed from
   `IpcChannelBridge<C: ChannelContext>` to trait-object
   `Arc<dyn ChannelContext + Send + Sync>` as the inner
   channel, so different channel types can coexist across
   connections in the same daemon.
9. **Backward-compat surface preserved.** `run_poc_daemon`
   (single-connection, no shutdown token — used by Phase 16
   PoC test) delegates to a new private
   `run_single_connection_daemon`. `run_daemon_compat`
   (multi-connection with shutdown — used by Phase 17–18
   tests) wraps a single channel in a `ChannelFactory` and
   delegates to `run_daemon`.

## Task 2 ship record

- **Cut:** `daemon_server.rs` rewritten (~410 lines):
  multi-connection accept loop, `ChannelFactory` type,
  `handle_connection` per-task handler,
  `run_single_connection_daemon`, `run_daemon_compat`,
  de-genericized `IpcChannelBridge`.
- **Cut:** `daemon_ipc.rs` (+30 lines): `FrontendType` enum,
  `frontend_type` field on `StartSession`, IPC round-trip
  test updated.
- **Cut:** `daemon_client.rs`: `DaemonSession::connect` gains
  `frontend_type` parameter, propagated through `StartSession`.
- **Cut:** `daemon_session.rs`: `DaemonSessionConfig` gains
  `frontend_type` field.
- **Cut:** `aivyx.rs`: daemon-run branch uses `ChannelFactory`;
  CLI frontend passes `FrontendType::Local`.
- **Cut:** `daemon_roundtrip_e2e.rs`: all 10 existing tests
  updated for new signatures; `shutdown.cancel()` added for
  clean daemon exit; 2 new tests:
  `two_concurrent_connections`, `connection_after_disconnect`.
- **Test count:** 548 (546 entry + 2 new). Target met.
- **Clippy:** clean.

## Decisions made at Task 3

10. **Q2 → (b) variant: daemon-mode pump in the binary.** The
    Telegram frontend retains the outer `get_updates` loop and
    per-chat routing, but each inner task submits turns through
    a `DaemonSession` instead of constructing an agent. The
    pump lives in the binary (not `aivyx-telegram`) because the
    dependency cycle constraint forbids `aivyx-telegram` from
    importing `DaemonSession`. Transport types
    (`TelegramTransport`, `ReqwestTransport`, `IncomingMessage`,
    `OutgoingMessage`, `TransportError`) widened from
    `pub(crate)` to `pub` so the binary can access them.
11. **Q3 → (b) accumulate per turn, send one message.** Streamed
    `StreamEventPayload` events are accumulated into a `String`
    buffer per turn. On `TurnComplete`, the buffer is sent as a
    single Telegram message via `transport.send_message()`.
    Matches the in-process Telegram path's one-message-per-turn
    behavior (Phase 8's `TelegramChannel::finalize`).
12. **Daemon-first + in-process fallback for Telegram.** Same
    pattern as the Local branch (Phase 18 Task 3). If a daemon
    is listening, Telegram turns route through IPC; on failure,
    falls back to the original `run_telegram_multi_session`
    in-process path.
13. **`TelegramDaemonChannel` identity stub.** The daemon's
    `ChannelFactory` now dispatches on `FrontendType`: `Local`
    returns `LocalChannel`, `Telegram` returns a lightweight
    `TelegramDaemonChannel` that reports `SemiTrusted` trust
    tier and `Telegram` platform. The stub's `stream_event` and
    `finalize` are no-ops — the `IpcChannelBridge` handles
    those.

## Task 3 ship record

- **Cut:** `aivyx-telegram/src/transport.rs`: widened
  `OutgoingMessage`, `IncomingMessage`, `TransportError`,
  `TelegramTransport`, `ReqwestTransport` from `pub(crate)` to
  `pub`. Removed stale `#[allow(dead_code)]` annotations.
- **Cut:** `aivyx-telegram/src/lib.rs`: `transport` module
  visibility widened from `mod` to `pub mod`.
- **Cut:** `aivyx.rs` (+210 lines):
  - `TelegramDaemonChannel` identity stub for `ChannelFactory`.
  - `run_telegram_daemon_multi_session` — outer `get_updates`
    loop with per-chat routing, `DaemonSession` per inner task.
  - `run_telegram_daemon_chat_task` — per-chat inner task:
    connect, submit turns, accumulate events, send one Telegram
    message per turn, forward `/cancel` as `CancelTurn`.
  - `ChannelKind::Telegram` branch updated with daemon-first +
    in-process fallback.
  - Daemon-run `channel_factory` dispatches on `FrontendType`.
- **Cut:** `daemon_roundtrip_e2e.rs` (+150 lines):
  - `PlatformEchoAgent` — echoes channel platform and trust
    tier in turn outcome.
  - `TestTelegramChannel` — test-local identity stub.
  - `telegram_frontend_type_gets_telegram_channel` — proves
    `FrontendType::Telegram` dispatches through factory to
    `SemiTrusted`/`Telegram` channel.
  - `mixed_local_and_telegram_frontends_on_same_daemon` — two
    clients (Local + Telegram) on one daemon, each getting the
    correct platform and trust tier.
- **Test count:** 550 (548 + 2 new). Target met.
- **Clippy:** clean.

## Task 4 ship record

Task 4 used the working-session slot for **binary line-count
management** — the triggered candidate (2521 > 2400 threshold).

- **Extracted:** `TelegramDaemonChannel`, `run_telegram_daemon_
  multi_session`, `run_telegram_daemon_chat_task`, and
  `render_events_for_telegram` from `aivyx.rs` into a new
  `aivyx-channel/src/telegram_daemon_frontend.rs` (285 lines).
- **Binary:** 2521 → 2262 lines (259 lines removed, under the
  2400 threshold).
- **Library module:** `telegram_daemon_frontend` registered in
  `lib.rs` as `pub mod`, re-exported `TelegramDaemonChannel`
  and `run_telegram_daemon_multi_session`.
- **No new tests.** Pure extraction — same 550 tests pass, same
  clippy clean.
- **Other candidates reviewed and deferred:**
  - Trust-tier enforcement: already covered by
    `mixed_local_and_telegram_frontends_on_same_daemon` e2e test
    + `ConcreteAgent::turn` line 176 intersection.
  - Edge cases: connection crash cleanup covered by EOF check;
    session ID isolation proven by `two_concurrent_connections`.
