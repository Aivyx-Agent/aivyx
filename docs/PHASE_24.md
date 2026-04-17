# Phase 24 — MCP Integration: Config + Binary Wiring

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Wire MCP server discovery into the daemon startup path so
operators can declare MCP servers in `aivyx.toml` and have
their tools available to agents automatically. This is the
second phase of the **MCP Integration** milestone identified
in `PRODUCT_ROADMAP.md`.

Phase 24 is a **continuation phase** — it extends the
`aivyx-mcp` foundation from Phase 23 into production use.

## Why now

1. **The MCP adapter is built but not wired.** Phase 23
   shipped `McpServerBridge` + `McpToolProxy` with a
   programmatic API and 8 passing tests. But no operator can
   use it yet — there's no config surface and no daemon-side
   lifecycle management.

2. **Config wiring is the smallest step to production use.**
   `[[mcp_server]]` TOML entries + daemon-side bridge
   lifecycle is a focused, targeted task that makes MCP
   tools available end-to-end.

3. **The PRODUCT_ROADMAP identifies MCP Integration as the
   highest-leverage milestone.** One config entry unlocks
   the entire MCP ecosystem for an operator.

## Streak predictions

- **DESIGN.md** — Low risk. The workspace layout amendment
  (A4) may need an addendum for the 11th crate, but that's
  a minor edit if needed at all.

- **PRODUCT.md** — Not at risk. No product commitment edits
  expected.

- **Production-core `aivyx-core/src/lib.rs`** — Low risk.
  Config wiring lives in `aivyx-config` and `aivyx-channel`.
  Prediction: streak **extends to fourteen**.

## Tasks

### Task 1 — Open commit + PHASE_24.md scaffold

This file. Update `docs/README.md` to show Phase 24 as Open.

### Task 2 — `[[mcp_server]]` config entries in `aivyx-config`

Add `McpServerConfig` struct and `[[mcp_server]]` TOML array
support to `aivyx-config`. Fields: `name` (server identifier
used in scope qualifiers), `command` (executable path),
`args` (argument list), `enabled` (default true).

## Task 2 ship record

**Files modified:**
- `crates/aivyx-config/src/lib.rs`: added `RawMcpServer` struct
  (internal TOML deserialization target with `name`, `command`,
  `args: Option<Vec<String>>`, `enabled` with `default_true`),
  `mcp_servers: Option<Vec<RawMcpServer>>` on `RawToml` with
  `#[serde(default, rename = "mcp_server")]`, public
  `McpServerConfig` struct, `mcp_servers: Vec<McpServerConfig>`
  on `AivyxConfig`, and loader mapping that filters disabled
  servers and unwraps `Option<Vec<String>>` args to empty vec.
- `crates/aivyx-config/src/tests.rs`: two new tests —
  `mcp_server_entries_parse_from_toml` (three entries, one
  disabled, verifies filtering + field mapping) and
  `no_mcp_server_section_gives_empty_vec`.
- `crates/aivyx-channel/src/bin/aivyx.rs`: added `mcp_servers`
  to the `AivyxConfig` destructure (prefixed `_` for now — Task
  3 will use it).
- `examples/aivyx.toml`: added commented `[[mcp_server]]`
  section with two example entries (github, filesystem).

**Test delta:** +2 (608 → 610).
**Production-core streak:** extends to fourteen (hash unchanged).

### Task 3 — Daemon-side MCP bridge lifecycle

Wire `McpServerBridge::start` into daemon startup:
- Read `[[mcp_server]]` entries from config
- Start bridges, call `discover_tools`
- Merge discovered tools into `ToolRegistry`
- Shutdown bridges at daemon exit

## Task 3 ship record

**Design decision:** MCP bridges start eagerly at binary startup
(Q1 resolved → (a)), before `ToolRegistry::new`. Discovered tools
are pushed into `tool_list` alongside native tools. Bridge shutdown
uses a two-tier strategy: explicit `shutdown()` call on the daemon
path (clean MCP protocol goodbye), `kill_on_drop(true)` as safety
net on all paths (covers early returns from daemon-session and
channel branches).

**Files modified:**
- `crates/aivyx-channel/Cargo.toml`: added `aivyx-mcp`
  dependency (binary-only, for MCP bridge startup in `aivyx.rs`).
- `crates/aivyx-channel/src/bin/aivyx.rs`: wired MCP bridge
  lifecycle — iterates `mcp_servers` from config, starts each
  `McpServerBridge`, calls `discover_tools`, extends `tool_list`
  with discovered MCP tools, prints per-server diagnostic line.
  Daemon path explicitly shuts down bridges after `run_daemon`
  returns. Non-daemon paths rely on `kill_on_drop`.
- `crates/aivyx-mcp/src/transport.rs`: added `kill_on_drop(true)`
  to the child-process `Command` builder so MCP server processes
  are cleaned up on bridge drop (all code paths, not just explicit
  `shutdown()`).

**Test delta:** +0 (610 → 610). Binary wiring is glue code; the
bridge itself is covered by 8 tests in `mcp_bridge_e2e.rs` and
the config surface by 2 tests from Task 2.
**Production-core streak:** extends to fourteen (hash unchanged).

### Task 4 — Workspace Layout Amendment addendum for `aivyx-mcp`

Update the A4 workspace layout amendment and DESIGN.md inline
reference to reflect the 11-crate workspace (was 10).

## Task 4 ship record

**Docs-only task.** Updated the A4 workspace layout amendment
and DESIGN.md to reflect the 11-crate workspace.

**Files modified:**
- `docs/amendments/2026-04-17-workspace-layout.md`: 10→11
  crates, added `aivyx-mcp` to crate tree and description
  section, updated traceability table with Phase 23 + 24
  entries, updated capability count 23→24 known bases,
  added Phase 23 + 24 to implementing phases.
- `DESIGN.md`: updated inline amendment reference "9 to 10"
  → "9 to 11", added `aivyx-mcp` to the Phase 0 crate tree,
  updated crate count "9 crates" → "11 crates".

**Test delta:** +0 (610 → 610).
**Production-core streak:** extends to fourteen (hash unchanged).

### Task 5+ — Scope TBD at Task 4 exit

Candidates: SSE transport, binary-level `--mcp-server` CLI flag.

## Deferrals

**Rolling deferrals carried from Phase 23 (13 items):**

- **Forensic `ToolOutcome::NotInRole` variant** —
  Phase 11 Q1. Untouched.
- **Second regression channel for the role primitive** —
  Phase 11 Q6. Untouched.
- **Response headers in audit payload** — Phase 12 Q3 half.
  Untouched.
- **Non-GET verbs (POST/PUT/PATCH/DELETE)** — Phase 12 Q1.
  Deferred indefinitely.
- **Redirect following with per-hop scope re-check** —
  Phase 12 Q5. Deferred indefinitely.
- **Binary response bodies / non-UTF-8** — Deferred
  indefinitely.
- **Per-chunk Telegram rendering** — Phase 12 Task 1.
  Deferred reactively.
- **Multi-level sub-agent nesting** — Phase 14 Task 3.
  Untouched.
- **LocalChannel regression-test rewrite over IPC** —
  Phase 17 Q6→(c+). Tagged: **reactive.**
- **Telegram-specific protocol extensions (attachment
  delivery, inline keyboards, etc.)** — Phase 19. Untouched.
- **`mission.list` / `mission.status` read-only tools** —
  Phase 21. Untouched.
- **MCP config surface (`[[mcp_server]]` in `aivyx.toml`)** —
  Phase 23. **Closed by Task 2.**
- **MCP SSE transport** — Phase 23. Untouched.

## Open questions

**Q1 — Should MCP bridges be started eagerly at daemon boot
or lazily on first tool call?** → **(a), resolved in Task 3.**
Eagerly. Bridges start before `ToolRegistry::new` so discovered
tools are in the registry from the first turn. Lazy init would
require a mutable registry.

**Q2 — Should MCP tool names be prefixed with the server
name to avoid collisions?** → **(b), resolved in Task 3.**
No prefix needed at the `name()` level — `ToolRegistry`
looks up by `ToolId` (UUID), not name. The scope system
(`mcp.call:<server>:<tool>`) disambiguates at the capability
layer. Tool names shown to the LLM are the raw MCP names.
