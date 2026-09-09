# Python channel adapter — reference implementation

A minimal Aivyx PA channel adapter written in Python 3, demonstrating
that the daemon's IPC protocol is language-agnostic. Stdlib only —
no `pip install` step.

This is the worked example referenced from:

- [`docs/CHANNEL_SDK.md`](../../docs/CHANNEL_SDK.md) — the contract
- [`docs/ADAPTER_PATTERN.md`](../../docs/ADAPTER_PATTERN.md) §
  Out-of-tree adapters
- [`docs/DAEMON_IPC.md`](../../docs/DAEMON_IPC.md) — the wire format

## Files

| File | Role |
|---|---|
| `frame.py` | Length-prefixed JSON framing (4-byte BE u32 + UTF-8 body) |
| `client.py` | High-level `DaemonClient` with the handshake + turn loop |
| `main.py` | CLI REPL — reads stdin, renders streaming events to stdout |
| `tests/` | Conformance scenarios (happy path, cancel, gate, unknown-variant skip) |

## Run it

In one terminal, start the daemon:

```sh
aivyx-pa daemon run                  # or `aivyx-pa` for auto-spawn
```

In another, run the Python adapter:

```sh
python3 examples/python-channel/main.py
```

Type a message, press Enter, watch the agent's response stream
back. `Ctrl-C` cancels the current turn; `Ctrl-D` quits.

## What this proves

The adapter implements exactly the lifecycle described in
[`CHANNEL_SDK.md` § 3](../../docs/CHANNEL_SDK.md):

1. Connect to `$XDG_RUNTIME_DIR/aivyx-pa/daemon.sock`.
2. Read `DaemonReady`.
3. (Optional) `ProtocolNegotiation`.
4. `StartSession { role: <arg>, frontend_type: "Local" }`.
5. Per-turn `SubmitInput` → render `StreamEvent` until
   `TurnComplete`.
6. `Disconnect`.

It declares `FrontendType: "Local"` and therefore inherits
`TrustTier::Trusted` — the daemon enforces the per-tier capability
ceiling server-side. The adapter does not implement capability
checks, audit logging, or cancellation token plumbing; it receives
all of those by talking to the daemon.

## Limitations

- **Synchronous I/O.** `socket.recv` blocks the main thread.
  Sufficient for a CLI REPL; a GUI adapter would want
  `asyncio` / threads.
- **No reconnect-on-daemon-restart.** If the daemon dies, the
  client exits. Production adapters should backoff-reconnect.
- **No attachment support.** `submit_turn` always sends
  `attachments: []`. Adding image input is `base64.b64encode(...)`
  into the `attachments` list per the IPC schema.
- **No query/inspection.** This adapter exercises the chat-loop
  path. Phase 47's `Query` / `QueryResponse` envelope is documented
  in `CHANNEL_SDK.md` § 4 but not implemented here — the Web UI
  is the reference for those.

## Running the conformance tests

The `tests/` directory exercises the protocol contract against a
scripted "daemon" backed by `socket.socketpair()`. **No real
daemon required, no LLM provider needed, no API keys** — they
verify the SDK-level shapes (framing, lifecycle, unknown-variant
skip, error surface).

```sh
python3 -m unittest discover examples/python-channel/tests -v
```

Fifteen scenarios at the time of writing:

- `test_frame.py` — round-trip, BE u32 header, UTF-8 bodies,
  size cap, partial reads (byte-dripfed frames)
- `test_lifecycle.py` — happy path (`DaemonReady` → `StartSession`
  → `submit_turn` → events → `TurnComplete`), cancellation,
  approval gates, unknown-variant skip (both at the
  `DaemonMessage` and `StreamEventPayload` layers), error surfaces

To validate end-to-end against the real daemon, run the steps in
the "Run it" section above — that's a manual smoke test, not a
unittest suite.
