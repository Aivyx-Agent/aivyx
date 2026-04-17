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

### Task 5 — `--mcp-server` CLI flag

Add a repeatable `--mcp-server name:command[:arg1,arg2,...]`
CLI flag for quick MCP server testing without editing config
files. CLI entries merge with `[[mcp_server]]` TOML entries.

## Task 5 ship record

**Files modified:**
- `crates/aivyx-channel/src/bin/aivyx.rs`: added `CliMcpServer`
  struct, `mcp_servers: Vec<CliMcpServer>` to `CliArgs`,
  `--mcp-server` parser arm with `splitn(3, ':')` format
  (`name:command` or `name:command:arg1,arg2,...`), merge of
  CLI entries into config `mcp_servers` vec before bridge
  startup, `cli_mcp_servers` parameter threaded through `run()`
  → `run_async()`. Seven new parser tests covering basic
  parsing, args, repeatability, missing value, malformed value,
  empty name, and default empty vec.
- `docs/PHASE_24.md`: task scaffold and ship record.

**Test delta:** +7 (610 → 617).
**Production-core streak:** extends to fourteen (hash unchanged).

### Task 6+ — Scope TBD at Task 5 exit

Candidates: SSE transport.

## Prediction vs. reality

- **DESIGN.md** — Predicted: **low risk**, amendment addendum
  may be needed. **Reality: correct.** DESIGN.md was edited in
  Task 4 to update the A4 workspace layout amendment inline
  reference (10→11 crates, added `aivyx-mcp` to crate tree).
  This was an addendum to an existing amendment, not a new
  architectural decision. New hash:
  `ceb538604bfac34a07a4cdb47b4777d6ab96c1c3891f6bc3246a3d3faf75a403`.

- **PRODUCT.md** — Predicted: **not at risk**.
  **Reality: correct.** Hash unchanged:
  `478cab6aa07ec94b49c1bfdf17619568cc66d6ccd8ca98dd93ef97de9a3ea1cf`.

- **Production-core `aivyx-core/src/lib.rs`** — Predicted:
  streak **extends to fourteen**. **Reality: correct.** All
  five tasks composed against existing trait shapes without
  modifying `lib.rs`. Hash unchanged:
  `d8ab203fc98c89b01a3dc7bd56653132786d4875fd912cfe11b47e22085eeb77`.

## Deferrals

**Rolling deferrals at exit (12 items, -1 closed):**

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
- **MCP SSE transport** — Phase 23. Untouched.

**Closed in Phase 24 (1 item):**

- **MCP config surface (`[[mcp_server]]` in `aivyx.toml`)** —
  Phase 23. **Closed by Task 2.**

**Rolling backlog: 13 → 12 (−1 closed, 0 net-new).**

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

## Exit criteria

- [x] `[[mcp_server]]` config entries shipped (Task 2,
  `b8228e8`): `McpServerConfig` struct, `RawMcpServer`
  deserialization, loader mapping with disabled-server
  filtering, 2 tests.
- [x] Phase 23 MCP config surface deferral closed.
- [x] Daemon-side MCP bridge lifecycle shipped (Task 3,
  `b847bb5`): eager startup, `discover_tools`, tool
  registration into `tool_list`, `kill_on_drop` safety net,
  explicit shutdown on daemon path.
- [x] Workspace layout amendment addendum shipped (Task 4,
  `d226ad1`): DESIGN.md + A4 amendment updated for 11-crate
  workspace, 24 known bases.
- [x] `--mcp-server` CLI flag shipped (Task 5, `53ce7bc`):
  repeatable flag, `splitn(3, ':')` format, merge with
  config entries, 7 parser tests.
- [x] Production-core streak extends to fourteen consecutive
  phases (hash unchanged).
- [x] PRODUCT.md unchanged (streak not tracked post-Phase-22
  amendment, but hash preserved).
- [x] Test count: 608 → 617 (+9, across Tasks 2 and 5).
- [x] Two Q-block questions resolved.
- [x] Prediction-vs-reality block recorded (all three
  correct).
- [x] Deferrals block recorded (−1 closed, 0 net-new).
