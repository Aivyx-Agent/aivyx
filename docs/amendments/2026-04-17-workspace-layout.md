# Amendment A4 — Workspace Layout

**Date:** 2026-04-17
**Phase:** 22
**Supersedes:** Extends D8 (Repo Skeleton). No text removed —
additive only.
**Implementing phases:** 8, 14, 15, 16, 17, 18, 19, 20, 21, 23, 24

---

## What changed

D8's repo skeleton described a 9-crate workspace of stubs.
The workspace now has **12 crates** with substantial module
growth, most of it concentrated in `aivyx-channel` which
serves as the platform's integration hub.

> **Phase 49 addendum (2026-05-12):** crate count bumped
> from 11 to 12 with the addition of `aivyx-tool`. See the
> traceability row for Phase 49 below.
>
> **Phase 107 addendum (2026-05-27):** crate count bumped
> from 12 to 13 with the addition of `aivyx-discord`. Third
> channel adapter, following the `aivyx-telegram` precedent
> (private 2-method transport trait, scripted-double
> testing, `run_*_session` sibling pattern). `ChannelPlatform::Discord`
> already lived in `aivyx-core` since Phase 8's
> forward-looking enumeration, so the lib.rs streak held
> byte-identical across the Task 2 crate-skeleton landing.
> See the traceability row for Phase 107 below.
>
> **Phase 108 addendum (2026-05-27):** crate count bumped
> from 13 to 14 with the addition of `aivyx-slack`. Fourth
> channel adapter and the four-data-point confirmation for
> the adapter pattern Phase 107 promoted to confirmed-at-
> three. Same private-2-method-transport-trait shape;
> scripted-double testing; multi-channel multiplexer +
> per-channel mailbox structure mirroring `aivyx-discord`.
> `ChannelPlatform::Slack` lived in `aivyx-core` since
> Phase 8's forward-enumeration (same Phase 107 surprise
> pattern), so the lib.rs streak holds again. `session_partition()`
> stringifies `(team_id, channel_id)` per Q3a, confirming
> the `Option<String>` return type at four data points.
> See the traceability row for Phase 108 below.

---

## Current workspace layout

```
~/Projects/aivyx/
├── Cargo.toml            workspace manifest (resolver = "2", edition 2024)
├── Cargo.lock            committed
├── rust-toolchain.toml   pinned stable + rustfmt + clippy
├── DESIGN.md             locked contract (with amendments)
├── PRODUCT.md            locked product contract
├── LICENSE               MIT
├── TRADEMARK.md          MIT + branded usage rule
├── README.md
├── .gitignore
├── examples/
│   ├── aivyx-pa.toml        four-role worked example (Phase 13)
│   └── aivyx-semitrusted.toml  semitrusted tier example (Phase 15)
├── docs/
│   ├── README.md         phase status table
│   ├── ROADMAP.md        technical roadmap
│   ├── PRODUCT_ROADMAP.md  product roadmap
│   ├── ADAPTER_PATTERN.md  channel adapter checklist
│   ├── DAEMON_IPC.md     IPC protocol spec (Phase 16)
│   ├── PHASE_0.md..PHASE_22.md  phase journals
│   └── amendments/       contract amendments (Phase 22+)
└── crates/
    ├── aivyx-core/       turn loop, Agent/Tool traits, TurnOutcome
    ├── aivyx-crypto/     HKDF, ChaCha20-Poly1305, Argon2id
    ├── aivyx-capability/ Scope, CapabilitySet, TrustTier, 24 known bases
    ├── aivyx-audit/      HMAC-chained audit log
    ├── aivyx-config/     config loading, role TOML parsing, secret-field resolution
    ├── aivyx-storage/    redb-backed, KeyDomain (5 domains), Storage trait
    ├── aivyx-llm/        LlmProvider trait + Anthropic reference impl
    ├── aivyx-memory/     memory.{read,write,forget} tools
    ├── aivyx-channel/    platform integration hub (see module map below)
    ├── aivyx-telegram/   Telegram transport: ReqwestTransport, scripted mock
    ├── aivyx-discord/    Discord transport: twilight-rs wrapper, scripted mock (Phase 107)
    ├── aivyx-slack/      Slack transport: slack-morphism Socket Mode wrapper, scripted mock (Phase 108)
    ├── aivyx-mcp/        MCP client adapter: McpServerBridge, McpToolProxy
    └── aivyx-tool/       Tool process IPC: ToolProcessBridge, ToolProxy (Phase 49)
```

### New crate: `aivyx-telegram` (Phase 8)

Separated from `aivyx-channel` to isolate the Telegram HTTP
API dependency. Contains `ReqwestTransport` (real HTTP) and
`ScriptedTransport` (deterministic test double). The transport
trait (`TelegramTransport`) enables E2E tests without network
access.

### New crate: `aivyx-mcp` (Phase 23)

MCP (Model Context Protocol) client adapter. Spawns MCP servers
as child processes, communicates over stdio with JSON-RPC 2.0,
and bridges discovered tools into the `Tool` trait via
`McpToolProxy`. The `mcp.call` scope base in `aivyx-capability`
gates access per-server-per-tool. Phase 24 wired config-driven
startup (`[[mcp_server]]` TOML entries) and daemon-side lifecycle
management.

### `aivyx-channel` module map

`aivyx-channel` is the largest crate in the workspace. It
contains the binary, the daemon infrastructure, all channel
adapters, and the mission/role machinery.

| Module | Lines | Purpose | Added |
|---|---|---|---|
| `bin/aivyx.rs` | ~2488 | Binary entry point, CLI arg parsing, agent construction, channel dispatch | Phase 1 |
| `lib.rs` | ~30 | Module re-exports | Phase 1 |
| `local.rs` | ~200 | `LocalChannel` — CLI REPL adapter | Phase 1 |
| `session.rs` | ~150 | In-process session runner | Phase 2 |
| `render.rs` | ~100 | CLI rendering utilities | Phase 3 |
| `passphrase.rs` | ~50 | Passphrase prompt for redb encryption | Phase 7 |
| `role_envelope.rs` | ~200 | `assemble_role_envelope` — role config → CapabilitySet | Phase 14 |
| `role_render.rs` | ~150 | `--print-role` debug surface renderer | Phase 15 |
| `daemon_ipc.rs` | ~450 | IPC types: FrontendMessage, DaemonMessage, StreamEventPayload, framing | Phase 16 |
| `daemon_server.rs` | ~500 | Daemon server: multi-connection, ChannelFactory, ResolveGate | Phase 16 |
| `daemon_client.rs` | ~350 | DaemonSession client library, auto-spawn, resolve_gate | Phase 16 |
| `daemon_session.rs` | ~200 | CLI REPL over daemon IPC | Phase 18 |
| `telegram_daemon_frontend.rs` | ~385 | Multi-chat Telegram pump over daemon IPC | Phase 19 |
| `mission.rs` | ~300 | Mission state machine, CRUD, gate resolution | Phase 21 |
| `mission_tool.rs` | ~190 | MissionCreateTool with OnceLock factory | Phase 21 |

### Binary line-count management

The binary (`bin/aivyx.rs`) is monitored against a ~2400 line
threshold. When it approaches the threshold, private functions
are extracted into library modules in `aivyx-channel/src/`.
This pattern was established in Phase 15 (role_render.rs
extraction, -635 lines) and repeated in Phase 19
(telegram_daemon_frontend.rs extraction). The extraction
preserves the production-core streak by keeping
`aivyx-core/src/lib.rs` untouched.

---

## Traceability

| Phase | Workspace change |
|---|---|
| Phase 0 | 9-crate skeleton, all stubs |
| Phase 8 | +`aivyx-telegram` crate (10 crates) |
| Phase 14 | +`role_envelope.rs` module |
| Phase 15 | +`role_render.rs` module (binary extraction) |
| Phase 16 | +`daemon_ipc.rs`, `daemon_server.rs`, `daemon_client.rs`, `DAEMON_IPC.md` |
| Phase 18 | +`daemon_session.rs` |
| Phase 19 | +`telegram_daemon_frontend.rs` (binary extraction) |
| Phase 21 | +`mission.rs`, `mission_tool.rs` |
| Phase 23 | +`aivyx-mcp` crate (11 crates), `mcp.call` scope base |
| Phase 24 | `[[mcp_server]]` config entries, daemon-side MCP bridge lifecycle |
| Phase 49 | +`aivyx-tool` crate (**12 crates**), `[[tool_process]]` config, `ToolProcessBridge` for third-party out-of-process tool IPC — delivers PRODUCT.md P12 |
| Phase 107 | +`aivyx-discord` crate (**13 crates**), `[discord]` TOML config (token + application_id), `ChannelKind::Discord`, twilight-rs SDK adoption — third channel adapter per Hermes-comparison-driven Chapter D |
| Phase 108 | +`aivyx-slack` crate (**14 crates**), `[slack]` TOML config (bot_token + app_token + team_id), `ChannelKind::Slack`, slack-morphism SDK adoption with Socket Mode — fourth channel adapter and the four-data-point confirmation for ADAPTER_PATTERN.md |
