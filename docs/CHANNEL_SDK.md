# Aivyx Channel SDK

**v0 — subject to change without deprecation policy.** Phase 48
ships the *contract*; API stability is deferred per
[`PRODUCT.md` P11](../PRODUCT.md) until the SDK has stabilized in
real third-party use. Expect minor breaking changes; expect
integration guarantees to hold.

This document is the third-party contract for building an Aivyx
channel adapter — a process that attaches to a running daemon and
relays user input + agent output to and from some transport
(messenger app, terminal, web UI, voice, IDE plugin).

It is the **operator-facing front of three** related docs:

| Doc | Audience | What it covers |
|---|---|---|
| [`CHANNEL_SDK.md`](CHANNEL_SDK.md) (this doc) | Third-party authors | The contract: what your adapter must do, and what you get for free. |
| [`DAEMON_IPC.md`](DAEMON_IPC.md) | Protocol implementers | The wire format: frame layout, message schemas, edge cases. |
| [`ADAPTER_PATTERN.md`](ADAPTER_PATTERN.md) | Adapter authors | The checklist: pragmatic patterns learned across in-tree adapters. |

If you're writing an out-of-tree adapter, read this doc first,
then drop into `DAEMON_IPC.md` for the wire details and
`ADAPTER_PATTERN.md` for the checklist.

For the threat model your adapter inherits, see
[`THREAT_MODEL.md`](THREAT_MODEL.md).

---

## 1. Who can write a channel adapter

Anyone, in any language, on the same machine as the daemon.

The wire format is length-prefixed UTF-8 JSON over a Unix domain
socket — implementable in ~50 lines of stdlib in most languages.
The reference implementation in this repo includes:

- `crates/aivyx-channel/src/local.rs` — Rust, CLI REPL (in-tree)
- `crates/aivyx-telegram/` — Rust, Telegram Bot API (in-tree)
- `crates/aivyx-channel/src/web_ui.rs` — Rust, WebSocket bridge
  for browsers (in-tree)
- `examples/python-channel/` — Python, CLI REPL (out-of-tree
  reference)

An adapter is just a process that:

1. Connects to `$XDG_RUNTIME_DIR/aivyx/daemon.sock` (mode 0600).
2. Reads and writes length-prefixed JSON frames.
3. Maps user input on its transport to `SubmitInput` frames, and
   `StreamEvent` / `TurnComplete` frames back to its transport.

That's it. Capability gating, audit logging, cancellation, role
attenuation — all delivered by the daemon, automatically.

---

## 2. Trust model — what your adapter inherits

The daemon authenticates frontends at the OS level. If you can
`read(2)` the socket, you are by definition the operator (see
[`PRODUCT.md` P6](../PRODUCT.md) and
[`THREAT_MODEL.md` §4.4](THREAT_MODEL.md)). There is no Aivyx-level
token, no password, no challenge.

What your adapter brings to the table is a **trust tier**, baked
into the `FrontendType` you declare in your `StartSession` frame:

| `FrontendType` | Trust tier | Typical example | Default ceiling |
|---|---|---|---|
| `Local` | `Trusted` | CLI on the operator's keyboard, browser on 127.0.0.1 | Near-total. `shell.exec`, `fs.delete` allowed with extra audit. |
| `Telegram` | `SemiTrusted` | Authenticated DM from allowlisted user | No `shell.exec`, no `fs.delete`, qualifiers required on `fs.*` and `net.post`. |
| `Web` | `Trusted` | Localhost-only browser session | Same as `Local`. |

The trust tier is **read from your adapter type, not set by it**.
You cannot ask the daemon for more authority than your
`FrontendType`'s ceiling. To add a new tier mapping (e.g., a
`Voice` adapter at `Trusted` because it requires microphone access
under the operator's account), file a phase to add the variant.

For the full per-scope ceiling table see [`DESIGN.md` D5](../DESIGN.md).

---

## 3. Lifecycle

```
┌────────────────────────────────────────────────────────────────┐
│  1. Connect to $XDG_RUNTIME_DIR/aivyx/daemon.sock              │
│  2. ← DaemonLifecycleEvent::DaemonReady { version }            │
│  3. (optional) → FrontendMessage::ProtocolNegotiation {        │
│                    version: "0.1"                              │
│                  }                                             │
│     ← DaemonMessage::ProtocolAccepted { version }              │
│  4. → FrontendMessage::StartSession {                          │
│         role: Option<String>,                                  │
│         frontend_type: Option<FrontendType>,                   │
│       }                                                        │
│     ← DaemonMessage::SessionStarted { session_id }             │
│                                                                │
│  ┌─── per turn ──────────────────────────────────────────┐     │
│  │ → FrontendMessage::SubmitInput {                       │     │
│  │     session_id, text, attachments?, mission_id?       │     │
│  │   }                                                    │     │
│  │ ← DaemonMessage::StreamEvent { session_id, event }    │     │
│  │ ← DaemonMessage::StreamEvent { ... }      (any count) │     │
│  │ ← DaemonMessage::StreamEvent { event: ApprovalGate }  │     │
│  │ (operator answers) → ResolveGate {...}                │     │
│  │ ← DaemonMessage::TurnComplete { session_id, outcome } │     │
│  │ (any operator-side cancel) → CancelTurn { session_id }│     │
│  └────────────────────────────────────────────────────────┘     │
│                                                                │
│  5. → FrontendMessage::Disconnect                              │
└────────────────────────────────────────────────────────────────┘
```

A few invariants:

- **The daemon sends `DaemonReady` first, unsolicited.** Read it
  before sending anything.
- **`StartSession` once per connection.** Re-`StartSession` is a
  protocol error. To switch roles, disconnect and reconnect.
- **`TurnComplete` is the only signal a turn is done.** Wait for
  it before sending the next `SubmitInput`, or the second submit
  will pile up against the still-running turn.
- **Approval gates are mid-turn `StreamEvent`s.** They do not
  pause the daemon — your adapter renders them and the daemon
  continues. The operator answers asynchronously via
  `ResolveGate`.

---

## 4. Message envelopes

All three top-level envelopes are length-prefixed JSON frames per
[`DAEMON_IPC.md`](DAEMON_IPC.md). The Rust authoritative source
is `crates/aivyx-channel/src/daemon_ipc.rs`.

### `FrontendMessage` — what you send

| Variant | When | Notes |
|---|---|---|
| `StartSession { role, frontend_type }` | Once per connection | `role` defaults to `default`; `frontend_type` defaults to `Local`. |
| `SubmitInput { session_id, text, attachments?, mission_id? }` | Per turn | `attachments` is `Vec<IpcAttachment>` (base64); `mission_id` ties the turn to an existing mission. |
| `CancelTurn { session_id }` | Mid-turn | Cancellation checked between LLM steps; the in-flight tool call finishes. |
| `ResolveGate { mission_id, gate_id, approved }` | When the operator answers an approval gate | Daemon resumes or aborts the mission accordingly. |
| `Disconnect` | At end of session | Polite close. The daemon also cleans up if you just close the socket. |
| `Shutdown` | Operator-driven daemon stop | Reserved for `aivyx daemon stop`; do not send from a normal adapter. |
| `ProtocolNegotiation { version }` | Optional, after `DaemonReady` | v0.1 always accepts. |
| `Query { id, payload }` | Inspection (Phase 47) | Read-only queries: `ListSessions`, `ListMissions`, `GetMission`, `ListAuditEntries`, `VerifyAuditChain`. |

### `DaemonMessage` — what you receive (turn traffic)

| Variant | When |
|---|---|
| `SessionStarted { session_id }` | After your `StartSession` |
| `StreamEvent { session_id, event }` | Repeatedly, mid-turn |
| `TurnComplete { session_id, outcome }` | End of turn |
| `Error { code, message }` | Daemon-side failure for your last message |
| `MissionCreated { mission_id }` | Agent created a mission |
| `MissionStateChanged { mission_id, state }` | Mission state transition |
| `GateResolved { mission_id, gate_id, approved }` | Echo after your `ResolveGate` |
| `QueryResponse { id, payload }` | Answer to a `Query` |
| `ProtocolAccepted` / `ProtocolRejected` | Response to `ProtocolNegotiation` |

### `DaemonLifecycleEvent` — what you receive (out-of-band)

| Variant | When |
|---|---|
| `DaemonReady { version }` | First frame after connect |
| `ShuttingDown { reason }` | Daemon is about to exit |
| `RecoveryNotice { lost_sessions, lost_turns, stale_since }` | After an unclean previous shutdown |

### `StreamEventPayload` — the streaming render bus

Carried inside `StreamEvent`. Your adapter renders these:

| Variant | What it carries | Render hint |
|---|---|---|
| `Text { text }` | Plain text chunk from the LLM | Append to current assistant bubble. |
| `Status { status }` | Status line ("Searching…") | Show as transient spinner. |
| `ToolCallStarted { tool_id, tool_name, input }` | About to run a tool | Render a tool card; expandable input. |
| `ToolCallFinished { tool_id, tool_name, outcome_summary }` | Tool done | Mark the card done; show summary. |
| `ToolOutput { tool_id, tool_name, chunk }` | Streaming tool stdout | Append into the tool card body. |
| `ApprovalGate { mission_id, gate_id, reason, scope }` | Operator approval required | Render Approve/Deny UI; reply with `ResolveGate`. |

A minimal adapter can collapse everything to `Text` — the other
variants are advisory richness that GUI / TUI adapters use.

---

## 5. What you get for free

The daemon does **not** trust your adapter beyond its declared
`FrontendType`. Every tool call is still:

1. **Scope-checked.** The agent cannot exceed the trust tier
   ceiling, full stop. If you're a `SemiTrusted` adapter, no
   `SubmitInput` you send can result in a `shell.exec` call —
   the scope check happens server-side, before the tool executes.

2. **Audited.** Every `ToolCall`, `ScopeDenied`, `TurnStarted`,
   `TurnEnded`, and `MemoryAccess` is appended to the HMAC-chained
   audit log synchronously. Your adapter has no way to bypass this
   — the audit append happens inside the daemon's turn loop, not
   in your process.

3. **Cancellable.** A `CancelTurn` frame propagates through a
   `CancellationToken` the tool implementations check between
   steps. In-flight tool calls finish; the next LLM step bails.

4. **Role-attenuated.** The active role's `tool_allowlist` and
   declared scopes intersect the tier ceiling before the LLM sees
   anything. You don't enumerate tools; the daemon does.

5. **Cancellation-clean on disconnect.** If you close the socket
   mid-turn, the daemon's connection handler exits and any
   in-flight turn for your session terminates gracefully. The
   audit chain still records what got executed.

You **do not** need to:

- implement a scope check
- write audit entries
- track turn IDs
- maintain a capability set
- worry about provider rate limits, retries, or streaming SSE
  parsing (the daemon does that)

---

## 6. Minimum viable adapter

In ~80 lines of any language with sockets + JSON:

```text
1. Connect to $XDG_RUNTIME_DIR/aivyx/daemon.sock
2. read_frame() → expect DaemonReady
3. write_frame({"type": "StartSession",
                "role": null,
                "frontend_type": "Local"})
4. read_frame() → expect SessionStarted; capture session_id
5. Repeat:
   a. Read user input from your transport.
   b. write_frame({"type": "SubmitInput",
                   "session_id": <captured>,
                   "text": <user input>})
   c. Loop: read_frame() until type == "TurnComplete":
        - "StreamEvent" with event.kind == "Text"
          → render event.text to your transport
        - anything else → ignore or render richly
6. write_frame({"type": "Disconnect"})
```

The Python reference at `examples/python-channel/` is this loop
plus a few quality-of-life affordances.

---

## 7. Integration guarantees (committed) vs API surface (v0)

Phase 48 commits to these properties — they hold across phase
boundaries:

| Property | Committed |
|---|---|
| OS-level peer auth on the IPC socket (mode 0600) | ✓ |
| Capability gating before every tool call | ✓ |
| HMAC-chained audit log for every tool call | ✓ |
| Trust-tier ceiling enforcement | ✓ |
| `CancelTurn` honored between LLM steps | ✓ |
| Role allowlist attenuation | ✓ |
| Wire format (length-prefixed JSON over Unix socket) | ✓ — version-negotiated; bump = `ProtocolNegotiation` reject |

These properties are **stable** in the sense that an adapter
written against them today will continue to receive them in
future phases. If a property weakens, that's an amendment.

The following are **not** stable:

| Surface | Why not stable |
|---|---|
| Field-level shape of `FrontendMessage` / `DaemonMessage` enums | New variants will be added. Existing variants may gain fields with `#[serde(default)]`. Removal would be a protocol bump. |
| `StreamEventPayload` variants | New variants may be added (e.g., for richer UI hints). Treat unknown variants as "ignore". |
| `DaemonLifecycleEvent` variants | Same; new variants may be added. |
| The set of `Query` payloads (Phase 47+) | Will grow as the inspection surface grows. |
| The set of `FrontendType` values | Will grow as new adapter classes ship. |

The pattern that survives all of these: **accept unknown
variants by tag and ignore them** rather than failing on
deserialization. The Python reference does this via
`try: dispatch(); except UnknownTag: continue`.

---

## 8. Common pitfalls

- **Reading frames partially.** The wire format is *4 bytes
  big-endian u32 length, then N bytes UTF-8 JSON*. A naive
  `recv(4096)` may give you a partial frame. Buffer until you
  have ≥4 bytes, decode the length, then buffer until you have
  ≥4+N bytes.

- **Sending before reading `DaemonReady`.** The daemon sends
  `DaemonReady` unsolicited as soon as the connection is
  accepted. If your adapter writes before reading, the daemon
  still serves the request, but `DaemonReady` will appear later
  in your read stream, which is confusing.

- **Failing on unknown variants.** New `StreamEventPayload` /
  `DaemonMessage` variants land in every phase. Build your
  decoder so unknown `type` / `kind` values are skipped, not
  errored.

- **Holding the socket open with no `SubmitInput` traffic.** The
  daemon is happy to hold the connection idle indefinitely.
  Don't add timeouts in your adapter unless your transport
  requires one.

- **Sending a second `SubmitInput` before `TurnComplete` arrives.**
  The daemon will queue or reject — neither is friendly. Wait
  for `TurnComplete` (or `Error`) before submitting the next
  turn.

---

## 9. Where to look next

- The Python reference: `examples/python-channel/`
- The protocol wire details: [`DAEMON_IPC.md`](DAEMON_IPC.md)
- The adapter checklist (in-tree patterns): [`ADAPTER_PATTERN.md`](ADAPTER_PATTERN.md)
- The threat model your adapter inherits:
  [`THREAT_MODEL.md`](THREAT_MODEL.md)
- The Rust authoritative source for the message envelopes:
  `crates/aivyx-channel/src/daemon_ipc.rs`
- The in-tree adapters as worked examples:
  - `crates/aivyx-channel/src/local.rs` (Rust CLI)
  - `crates/aivyx-telegram/` (Rust + frankenstein for the Bot API)
  - `crates/aivyx-channel/src/web_ui.rs` + `web_ui_static.html`
    (Rust + tokio-tungstenite + embedded HTML/JS)

If you're stuck, the canonical "does my adapter work?" test is
to drive a successful turn end-to-end against a live daemon
under Ollama — same setup we use for `aivyx daemon run`
acceptance.
