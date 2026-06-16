# Phase 39 — Web UI Channel (Phase 1: Chat Interface)

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../../../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../../../PRODUCT.md).

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
bridges directly to the daemon's Unix socket at the frame level —
sending `FrontendMessage` frames and forwarding `DaemonEnvelope`
frames in real time. The WebSocket wire format is identical JSON
to the IPC protocol. The web server is purely a protocol
translator — WebSocket frames to/from length-prefixed Unix socket
frames.

```
Browser ──WebSocket──► Web UI Server ──Unix Socket──► Daemon
  JS sends                TCP peek +          encode_frame /
  FrontendMessage JSON    tokio-tungstenite   decode_frame
```

`ChannelPlatform::Local` is used (not a new variant) — the
existing doc says "CLI, desktop app, local REST on 127.0.0.1."
This preserves the production-core `aivyx-core/src/lib.rs` streak.

## Streak predictions → reality

| Streak target | Predicted | Actual | Notes |
|---|---|---|---|
| DESIGN.md | untouched (15) | untouched (16) | no contract changes |
| PRODUCT.md | untouched (1) | untouched (2) | Web UI is a candidate, not commitment |
| lib.rs | untouched (2) | untouched (3) | `ChannelPlatform::Local` already exists |

## Ship record

| Task | Commit | What shipped |
|---|---|---|
| 1 | `756b3c8` | Open commit, PHASE_39.md scaffold, README + ROADMAP |
| 2 | `4b30d4b` | `FrontendType::Web`, `WebDaemonChannel` stub, `ChannelFactory` wiring |
| 3 | `b5bb9dd` | `tokio-tungstenite` dep, `run_web_ui_server()`, TCP peek routing |
| 4 | `612d003` | Embedded HTML/CSS/JS frontend — streaming chat, tool cards, gates |
| 5 | `4538624` | Config + CLI wiring (`--web-ui`, `[daemon] web_ui`), daemon spawn |
| 6 | _this commit_ | Exit freeze, docs update |

Test count: 788 → 801 (+13 new tests across 5 tasks).
Clippy warnings: 0 throughout.
New Cargo.lock entries: `tokio-tungstenite` (+ transitive deps).

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
- TCP peek routing (no hyper dependency in WS path)
- Routes: `GET /` serves HTML, `GET /ws` upgrades to WebSocket
- Each WS connection bridges to daemon Unix socket at frame level
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

- [x] `FrontendType::Web` variant added and tested.
- [x] `WebDaemonChannel` implements `ChannelContext` with correct
      platform and trust tier.
- [x] Web UI server binds localhost, serves HTML, upgrades to WS.
- [x] WebSocket bridge forwards `FrontendMessage`/`DaemonMessage`
      between browser and daemon.
- [x] Embedded HTML renders text, tool calls, and approval gates.
- [x] `--web-ui` flag and `[daemon] web_ui` config wired.
- [x] All tests pass (801, up from 788).
- [x] Zero clippy warnings.
- [x] DESIGN.md untouched (streak 16 from Phase 25).
- [x] PRODUCT.md untouched (streak 2 from Phase 38).
- [x] `aivyx-core/src/lib.rs` untouched (streak 3 from Phase 38).
