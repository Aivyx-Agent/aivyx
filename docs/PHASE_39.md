# Phase 39 — Web UI Channel (Phase 1: Chat Interface)

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Add a localhost-only web chat interface that connects to the
daemon over the existing IPC protocol. The daemon architecture
(`FrontendType`, `ChannelFactory`, `StreamEventPayload` rendering)
was explicitly designed to make new frontends cheap — the Web UI
is the third frontend after CLI REPL and Telegram.

Phase 1 scope: minimal chat UI with message input, streaming
response rendering, tool-call cards, and approval-gate buttons.
Phase 2 (future): mission management dashboard, audit inspection
surface.

## Why now

1. **Highest-impact undelivered feature.** Every high-impact
   milestone is shipped (daemon, missions, MCP, multi-provider,
   scheduled execution, reflection). The Web UI is the last "big"
   surface that changes how an operator interacts with Aivyx.

2. **Substrate is ready.** The daemon IPC protocol,
   `FrontendType` extension point, and `StreamEventPayload`
   rendering pipeline were designed with this in mind. The
   `FrontendType::Web` variant and `ChannelFactory` dispatch
   are mechanical additions.

3. **Codebase health is excellent.** Phase 38 left zero clippy
   warnings, 788 tests, 1 deferral. Good time for a feature.

## Architecture

The web server is a background task spawned inside `run_daemon`
(same pattern as the webhook listener). Each WebSocket connection
creates a `DaemonSession` that connects back to the daemon's Unix
socket. The WebSocket wire format is identical JSON to the IPC
protocol (`FrontendMessage`/`DaemonMessage` serde tags). The web
server is purely a protocol translator — WebSocket frames to/from
length-prefixed Unix socket frames.

```
Browser ──WebSocket──► Web UI Server ──Unix Socket──► Daemon
  JS sends                hyper +              DaemonSession
  FrontendMessage JSON    tokio-tungstenite    bridges to IPC
```

`ChannelPlatform::Local` is used (not a new variant) — the
existing doc says "CLI, desktop app, local REST on 127.0.0.1."
This preserves the production-core `aivyx-core/src/lib.rs` streak.

## Streak predictions

| Streak target | Predicted | Notes |
|---|---|---|
| DESIGN.md | untouched (15) | no contract changes |
| PRODUCT.md | untouched (1) | Web UI is a candidate, not commitment |
| lib.rs | untouched (2) | `ChannelPlatform::Local` already exists |

## Tasks

### Task 1 — Open commit + PHASE_39.md scaffold

This file. Update `docs/README.md` to show Phase 39 as Open.
Update `docs/ROADMAP.md` with Phase 39 entry.

### Task 2 — `FrontendType::Web` + `WebDaemonChannel` stub

Add `Web` variant to `FrontendType` in `daemon_ipc.rs`. Create
`web_ui.rs` module with `WebDaemonChannel` struct following the
`TelegramDaemonChannel` pattern: `ChannelPlatform::Local`,
`TrustTier::Trusted`, no-op `stream_event`/`finalize`. Wire into
`ChannelFactory` in the binary. Add serde round-trip and metadata
tests.

### Task 3 — WebSocket HTTP server + `tokio-tungstenite` dep

Add `tokio-tungstenite` to workspace deps. Implement
`run_web_ui_server()` in `web_ui.rs`:
- Binds `127.0.0.1:<port>` (default 7843)
- `hyper` HTTP/1.1 (same pattern as `webhook_listener.rs`)
- Routes: `GET /` serves HTML, `GET /ws` upgrades to WebSocket
- Each WS connection bridges to daemon via `DaemonSession`
- Shutdown via `CancellationToken`

### Task 4 — Embedded HTML/CSS/JS frontend

Single `web_ui_static.html` file embedded via `include_str!`.
Chat UI with message input, streaming text, tool-call cards,
approval-gate Approve/Deny buttons, cancel button. Dark theme.

### Task 5 — Config + CLI wiring + daemon spawn

Add `[daemon] web_ui` and `web_ui_port` to config. Add `--web-ui`
and `--web-ui-port` CLI flags. Add `web_ui_port` parameter to
`run_daemon()`. Spawn `run_web_ui_server` conditionally (same
pattern as webhook listener).

### Task 6 — Exit freeze + docs

## Exit criteria

- [ ] `FrontendType::Web` variant added and tested.
- [ ] `WebDaemonChannel` implements `ChannelContext` with correct
      platform and trust tier.
- [ ] Web UI server binds localhost, serves HTML, upgrades to WS.
- [ ] WebSocket bridge forwards `FrontendMessage`/`DaemonMessage`
      between browser and daemon.
- [ ] Embedded HTML renders text, tool calls, and approval gates.
- [ ] `--web-ui` flag and `[daemon] web_ui` config wired.
- [ ] All tests pass (788 + new).
- [ ] Zero clippy warnings.
- [ ] DESIGN.md untouched (streak 15 from Phase 25).
- [ ] PRODUCT.md untouched (streak 1 from Phase 38).
- [ ] `aivyx-core/src/lib.rs` untouched (streak 2 from Phase 38).
