## Aivyx Daemon IPC Protocol — Phase 16

A cross-phase reference document specifying the IPC protocol shape
that the daemon (Phase 16+) and all frontends communicate over.
Settled during Phase 16 (Daemon Migration phase 1 of N). Future
daemon phases inherit these decisions as givens; changes require a
protocol-version bump and a documented migration path.

For the product commitment this protocol delivers against, see
[`../PRODUCT.md` P4](../PRODUCT.md) (Daemon-Default Architecture).
For the phase journal, see [`PHASE_16.md`](PHASE_16.md).

---

### Transport

**Unix domain socket** at a well-known path:

```text
$XDG_RUNTIME_DIR/aivyx/daemon.sock      (preferred)
$HOME/.local/share/aivyx/daemon.sock     (fallback when XDG_RUNTIME_DIR is unset)
```

The daemon creates the socket file with mode `0600`, owned by the
daemon's effective UID. Per P4.4, any process that can read the
socket file is by definition the operator. The daemon verifies
peer identity via `SO_PEERCRED` on Linux (UID match); other
platforms are deferred to their respective porting phases.

The daemon removes (unlinks) any stale socket file at startup
before binding. A frontend that finds no socket file (or gets
`ECONNREFUSED`) knows no daemon is running.

---

### Wire format

**Length-prefixed JSON frames.** Each frame is:

```text
┌──────────────────┬──────────────────────────────────────┐
│ 4 bytes          │ N bytes                              │
│ big-endian u32   │ UTF-8 JSON payload                   │
│ (payload length) │                                      │
└──────────────────┴──────────────────────────────────────┘
```

- **Max payload size:** 16 MiB (16,777,216 bytes). A frame whose
  length prefix exceeds this limit is a protocol error; the
  receiver closes the connection.
- **No trailer, no compression, no TLS.** The socket is local-only
  and OS-permission-protected; encryption is unnecessary. Compression
  is unnecessary at LLM-token-rate throughput.
- **`serde_json`** is the serialization library (already in the
  workspace). Zero new dependencies.

The choice of JSON over a binary format is deliberate for Phase 16:
the PoC's primary value is debuggability (inspect frames with a hex
dump or `socat`), not throughput. A future phase may upgrade to a
binary format by bumping the protocol version; the transport and
framing decisions survive that swap.

---

### Message types

Three top-level message envelopes flow over the socket. Each is a
JSON object with a `"type"` discriminator field.

#### `FrontendMessage` (frontend → daemon)

| Variant          | Payload fields                  | Semantics                                                |
|------------------|---------------------------------|----------------------------------------------------------|
| `StartSession`   | `role: Option<String>`          | Request a new session under the named role (or default).  |
| `SubmitInput`    | `session_id: String`, `text: String`, `mission_id: Option<String>` | Send one user input line. When `mission_id` is set and the turn escalates, the daemon creates a gate on that mission and emits `ApprovalGate`. |
| `CancelTurn`     | `session_id: String`            | Request cancellation of the in-flight turn.              |
| `ResolveGate`    | `mission_id: String`, `gate_id: String`, `approved: bool` | Operator resolves a pending mission approval gate. |
| `Disconnect`     | *(none)*                        | Graceful frontend disconnect. Daemon may keep the session alive. |

#### `DaemonMessage` (daemon → frontend)

| Variant           | Payload fields                        | Semantics                                                    |
|-------------------|---------------------------------------|--------------------------------------------------------------|
| `SessionStarted`  | `session_id: String`                  | Acknowledges `StartSession`; the session is ready for input. |
| `StreamEvent`     | `session_id: String`, `event: StreamEventPayload` | One streamed event from the turn loop.          |
| `TurnComplete`    | `session_id: String`, `outcome: String` | Terminal frame for a turn. `outcome` is human-readable.    |
| `Error`           | `code: String`, `message: String`     | Protocol-level or session-level error.                       |
| `MissionCreated`  | `mission_id: String`                  | Acknowledges mission creation.                               |
| `MissionStateChanged` | `mission_id: String`, `state: String` | Mission transitioned to a new state.                      |
| `GateResolved`    | `mission_id: String`, `gate_id: String`, `approved: bool` | Gate resolution confirmed.                    |

#### `DaemonLifecycleEvent` (daemon → frontend, separate from `DaemonMessage`)

| Variant           | Payload fields                  | Semantics                                             |
|-------------------|---------------------------------|-------------------------------------------------------|
| `DaemonReady`     | `version: String`               | Sent once after the frontend connects.                |
| `ShuttingDown`    | `reason: String`                | Daemon is shutting down; frontend should disconnect.  |

Per Phase 16 Q4 resolution (a): lifecycle events are a **separate
message type** from `DaemonMessage`. The frontend's IPC receive
loop demuxes on the `"type"` discriminator into three categories
(`FrontendMessage`, `DaemonMessage`, `DaemonLifecycleEvent`). This
keeps `StreamEvent` in `aivyx-core/src/lib.rs` untouched and
preserves the production-core streak.

---

### `StreamEventPayload`

The `StreamEventPayload` carried inside `DaemonMessage::StreamEvent`
is a JSON-serializable mirror of `aivyx_core::StreamEvent<'a>`. The
core enum uses borrowed references (`&str`, `&[u8]`) and is not
`Serialize`; the IPC layer defines an owned, serializable counterpart
that converts to/from the core type at the process boundary.

| Variant              | Fields                                                 |
|----------------------|--------------------------------------------------------|
| `Text`               | `text: String`                                         |
| `Status`             | `status: String`                                       |
| `ToolCallStarted`    | `tool_id: String`, `tool_name: String`, `input: Value` |
| `ToolCallFinished`   | `tool_id: String`, `tool_name: String`, `outcome_summary: String` |
| `ToolOutput`         | `tool_id: String`, `tool_name: String`, `chunk: String` |
| `ApprovalGate`       | `mission_id: String`, `gate_id: String`, `reason: String`, `scope: Option<String>` |

`Attachment` is excluded from the Phase 16 PoC. Binary payloads
over JSON require base64 encoding; the complexity is deferred to a
phase that actually exercises attachments over IPC.

---

### Error model

Every `FrontendMessage` that expects a response gets exactly one
terminal frame (`SessionStarted`, `TurnComplete`, or `Error`) plus
zero or more intermediate `StreamEvent` frames between
`SubmitInput` and `TurnComplete`. A frontend that receives `Error`
in response to `StartSession` knows the session was not created.

Error codes are short string tags, not numeric. Phase 16 defines:

- `"invalid_message"` — the daemon could not parse the frame.
- `"unknown_session"` — `session_id` does not match a live session.
- `"no_mission_store"` — `ResolveGate` received but no mission store configured.
- `"gate_create_failed"` — escalation→gate creation failed (missing mission, wrong state).
- `"gate_resolve_failed"` — gate resolution failed (missing mission/gate, wrong state).
- `"internal"` — catch-all for unexpected daemon-side failures.

### Escalation→gate turn-loop wiring (Phase 23)

When a `SubmitInput` carries a `mission_id` and the agent's turn returns
`TurnOutcome::Escalated`, the daemon:

1. Loads the mission from redb.
2. Creates a `GateRecord` via `mission::add_gate`, transitioning the
   mission to `GatePending`.
3. Emits `StreamEventPayload::ApprovalGate` to the frontend.
4. Sends `TurnComplete` with outcome `"escalated: <reason>"`.

When the frontend sends `ResolveGate` with `approved: true`, the daemon:

1. Resolves the gate, transitioning the mission back to `Running`.
2. Sends `GateResolved`.
3. Starts a new turn with the approval context as input, streaming
   events and ending with a second `TurnComplete`.

When rejected, the mission transitions to `Failed` and no resume turn
occurs.

---

### Auth model

**OS-user ownership per P4.4.** The daemon sets the socket file to
mode `0600`. On Linux, the daemon additionally verifies via
`SO_PEERCRED` that the connecting process's effective UID matches the
daemon's own UID. A UID mismatch is a hard `Error` and the
connection is closed immediately.

There is no Aivyx-level password, token, or challenge-response
handshake. Per P6, identity equals OS user.

---

### Protocol versioning

The `DaemonReady` lifecycle event carries a `version` string. Phase
16 defines version `"0.1"`. The frontend checks the version on
connect; a version mismatch is a warning (not a hard error) in
Phase 16, becoming a hard error once the protocol stabilizes in a
future SDK phase.

---

### Decisions record (for PHASE_16.md cross-reference)

| Question | Resolution | Rationale |
|----------|------------|-----------|
| **Q3** — Wire format | **(a)** Hand-rolled length-prefixed JSON | Debuggability over throughput for the PoC phase. `serde_json` already in tree. Zero new deps. |
| **Q4** — Daemon lifecycle shape | **(a)** Separate message type, never crosses `StreamEvent` | Preserves production-core streak. Frontend demuxes on `"type"` discriminator. |
