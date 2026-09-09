# Amendment A1 — Daemon IPC Protocol

**Date:** 2026-04-17
**Phase:** 22 (first amendment in project history)
**Supersedes:** Extends D1 (Turn Loop Contract) and D3
(Agent Trait + Outcome Types). No text removed — additive only.
**Implementing phases:** 16, 17, 18, 19, 20

---

## What changed

D1's Turn Loop Contract describes a single-process execution
model: a `ChannelContext` delivers a `Message` directly to an
`Agent`. D3's type sketches show `channel.stream_event()` as
an in-process trait call. Both remain correct descriptions of
the **in-process turn loop**, but Phases 16–20 introduced a
second execution topology that now mediates every production
turn: the **daemon IPC layer**.

The turn loop still runs exactly as D1 describes — inside the
daemon process. What changed is **how messages arrive and how
events leave**. Channel frontends no longer call
`agent.turn(message, channel)` directly. Instead:

1. The frontend connects to a Unix domain socket.
2. The frontend sends a `FrontendMessage::SubmitInput` frame.
3. The daemon dispatches the input to the agent's turn loop.
4. The agent's `stream_event` calls flow through an
   `IpcChannelBridge` that serializes each `StreamEvent` into
   a `StreamEventPayload` and writes it as a `DaemonMessage`
   frame back to the frontend.
5. The frontend renders the events using its channel-specific
   renderer (`render_for_cli`, Telegram `send_message`, etc.).

This is the daemon-default architecture committed to in
PRODUCT.md P4.

---

## The IPC protocol

The full protocol specification lives in
[`docs/DAEMON_IPC.md`](../DAEMON_IPC.md). This amendment
summarizes the load-bearing decisions.

### Transport

Unix domain socket at:
```
$XDG_RUNTIME_DIR/aivyx-pa/daemon.sock   (preferred)
$HOME/.local/share/aivyx-pa/daemon.sock  (fallback)
```

Socket mode `0600`, owned by the daemon's effective UID.
Authentication is OS-level per P4.4 and P6: any process that
can read the socket is the operator.

### Wire format

Length-prefixed JSON frames: 4-byte big-endian u32 length
prefix, followed by a UTF-8 JSON payload. Max payload 16 MiB.
No compression, no TLS (local-only socket).

### Message envelopes

Three top-level envelopes:

**`FrontendMessage`** (frontend -> daemon):
`StartSession`, `SubmitInput`, `CancelTurn`, `ResolveGate`,
`Disconnect`, `Shutdown`.

**`DaemonMessage`** (daemon -> frontend):
`SessionStarted`, `StreamEvent`, `TurnComplete`, `Error`,
`MissionCreated`, `MissionStateChanged`, `GateResolved`.

**`DaemonLifecycleEvent`** (daemon -> frontend, separate type):
`DaemonReady`, `ShuttingDown`.

Lifecycle events are a separate message type from
`DaemonMessage` (Phase 16 Q4 resolution). The frontend's IPC
receive loop discriminates on the top-level `"type"` field.

### `StreamEventPayload` — the IPC-safe mirror

D3's `StreamEvent<'a>` is borrowed (lifetime-tied to agent
buffers). The IPC layer cannot send borrowed data over a
socket. `StreamEventPayload` is the owned, serializable
mirror:

| `StreamEvent<'a>` variant | `StreamEventPayload` variant |
|---|---|
| `Text(&'a str)` | `Text { text: String }` |
| `Status(&'a str)` | `Status { status: String }` |
| `ToolCallStarted { tool, input }` | `ToolCallStarted { tool_name: String, input: Value }` |
| `ToolCallFinished { tool, outcome_summary }` | `ToolCallFinished { tool_name: String, outcome_summary: String }` |
| — | `ToolOutput { tool_name: String, output: String }` |
| — | `ApprovalGate { mission_id, gate_id, reason, scope }` |

`ToolOutput` and `ApprovalGate` exist only in the IPC layer
(no in-process `StreamEvent` equivalent). `Attachment` is not
yet mirrored (deferred with Telegram-specific extensions).

### `FrontendType` and `ChannelFactory`

The daemon serves multiple connection types. Each frontend
declares its type at connection time via a `FrontendType`
field on `StartSession`:

```
enum FrontendType { Local, Telegram }
```

The daemon's `ChannelFactory` dispatches on `FrontendType` to
construct the appropriate `ChannelContext` implementation for
the agent turn.

### Multi-connection model

The daemon accepts multiple simultaneous connections (Phase
19). Each connection gets its own tokio task, its own
`DaemonSession`, and its own `IpcChannelBridge`. State
ownership (redb handle, audit chain, role registry, capability
ceiling) lives in the daemon's `Arc`-shared state, not in any
single connection.

### Auto-spawn and lifecycle

When a frontend launches and finds no running daemon:
1. Spawns the daemon as a detached child process
   (`daemon run` subcommand).
2. Waits for the socket file to appear.
3. Connects and proceeds transparently.

The daemon writes a PID file alongside the socket. Graceful
shutdown via `CancellationToken` propagation: `Shutdown` IPC
message or SIGTERM triggers cancellation, in-flight turns
complete, connections drain, socket is unlinked. `daemon
status` and `daemon stop` subcommands provide operator
management.

---

## How D1 and D3 should be read after this amendment

D1's paragraph remains the authoritative description of **what
happens during a turn**. This amendment describes **how turns
are initiated and how their events are delivered** in the
daemon-default architecture. The two are complementary:

- D1 says "a `ChannelContext` delivers an inbound `Message`
  to an `Agent`." In daemon mode, the `ChannelContext` is an
  `IpcChannelBridge` whose `stream_event` implementation
  serializes to `StreamEventPayload` and writes IPC frames.
  The delivery surface is mediated, not direct.

- D3's `ChannelContext` trait is implemented twice per channel:
  once for in-process mode (the original `LocalChannel`,
  `TelegramChannel`) and once for daemon mode
  (`IpcChannelBridge` on the daemon side,
  `TelegramDaemonChannel` as an identity stub). The trait
  shape is unchanged.

- D3's `TurnOutcome` gains operational significance in daemon
  mode: the daemon inspects the outcome to decide whether to
  create a mission gate (`Escalated`), log a failure
  (`Failed`), or simply acknowledge completion.

---

## Traceability

| Phase | What shipped | Commit |
|---|---|---|
| Phase 16 | PoC server + client + round-trip test, DAEMON_IPC.md | `1ed3f90` |
| Phase 17 | Multi-turn server, `daemon run`, `DaemonSession` | `277d910` |
| Phase 18 | REPL over IPC, auto-spawn, cancel handle | `6dd4f23` |
| Phase 19 | Multi-connection, Telegram port, `FrontendType` | `986c519` |
| Phase 20 | `daemon status`/`stop`, PID file, `--no-daemon` | `8e77075` |
