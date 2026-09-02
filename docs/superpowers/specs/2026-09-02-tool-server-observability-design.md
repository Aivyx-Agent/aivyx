# Tool/server call-stat observability (POLISH_WAVES.md sub-project 8) — design

**Status:** Approved, ready for planning.

## Motivation

`docs/POLISH_WAVES.md` sub-project 8 bundles two findings from `VITRINE.md`
under one framing: "is this tool/server actually working," derived from the
audit chain rather than trusted from connection-time status alone. It
records that the sub-project "exists and why the two findings are joined"
but was never scoped in detail — this document is that scoping, grounded
against the current code (not the tracking doc's prose).

**The tracking doc's own framing is partly stale.** It describes "a shared
audit-chain call-stat aggregator" as work neither finding has today. Reading
the real code found that's only half true:

- **Phase 102 already shipped a general per-tool aggregator.**
  `QueryPayload::GetToolStats` / `QueryResponsePayload::ToolStats` /
  `ToolStat` (`crates/aivyx-ipc/src/protocol.rs`), backed by
  `fold_tool_stats` (`crates/aivyx-channel/src/daemon_server.rs:7402`) —
  real audit-chain-derived per-tool call counts, outcome breakdown, and
  duration, joined against the live tool registry. Already consumed by a
  working `aivyx tools` CLI command (`crates/aivyx-cli/src/bin/
  aivyx_modules/tools.rs`). This is item A as originally described, just
  shipped under a name the tracking doc's authors didn't cross-reference.
- **It has a real, unaddressed gap for MCP specifically.**
  `fold_tool_stats` groups by `scope_used.base()` (`Scope::base()`,
  `crates/aivyx-capability/src/lib.rs:589` — everything before the first
  `:`). Every MCP-bridged tool's capability scope is `mcp.call:<server>:
  <tool>` (`crates/aivyx-mcp/src/proxy.rs:71`-73, and the sibling resource/
  prompt proxies) — the server and tool names live entirely in the
  *qualifier*, which `fold_tool_stats` never inspects. So every configured
  `[[mcp_server]]`'s tool calls collapse into one shared `"mcp.call"`
  bucket today. The original finding this sub-project names — `web-search`
  showed green all day while DuckDuckGo silently refused its queries — is
  still real and unaddressed by Phase 102's work.
- **The MCP panel's `connected` status is a boot-time/manual-probe
  snapshot, confirmed, not a guess.** `McpServerStatusView`
  (`crates/aivyx-ipc/src/protocol.rs:859`) is served by `GetMcpStatus`
  (`crates/aivyx-channel/src/daemon_server.rs:4402`), which reads a JSON
  snapshot file the daemon writes once at the end of its MCP startup loop
  (`crates/aivyx-channel/src/mcp_status.rs` — its own doc comment: "Snapshot
  semantics are 'as of the last daemon start' — a file, not live daemon
  memory"). Plan 1 of sub-project 7 added a manual `TestMcpServerConnection`
  probe on top, but nothing rolling/automatic exists.

**Scope, corrected against this grounding:**

- **B — TUI Tools view** (`VITRINE.md` §12, `aivyx-tui`'s `View::Tools`) —
  genuinely unbuilt, but much smaller than the tracking doc assumed: no new
  backend needed at all, just a ratatui consumer of the already-shipped
  `GetToolStats` query.
- **C — MCP per-server health signal** (`VITRINE.md` §10, the Lantern
  screen) — genuinely unbuilt: a new audit-chain aggregation dimension
  (per-MCP-server, not per-tool-base) plus a new rolling health chip on
  `McpServerCard`, landing alongside (not replacing) the existing
  `connected` status.

**Out of scope:** rebuilding or changing `fold_tool_stats`/`GetToolStats`
itself (item A) — it already does its job for every non-MCP tool and stays
untouched; a new sibling function handles the MCP-specific dimension
instead of retrofitting the general one.

## B. TUI Tools view

`crates/aivyx-tui/src/render.rs:197`-199 currently renders `View::Tools` as
`placeholder_lines("the registered tools — provenance, capability scope,
and call stats")`. Replace with a real ratatui table, following the exact
pattern `View::Audit` already established for wiring a real IPC-backed view
into this TUI (poll-on-open via the same daemon-client query mechanism,
`AppState` gains a `tool_stats: Vec<ToolStat>` field mirroring
`audit_entries: Vec<AuditEntrySummary>`).

Query: the existing `QueryPayload::GetToolStats { window_secs: None }`
(whole-chain, matching the CLI's own default) — no new wire type. Table
columns: tool name, `[unregistered]` marker when `!registered` (mirroring
the CLI renderer's own convention), call count, outcome breakdown
(condensed, e.g. `12 ok / 1 failed`), average duration (`total_duration_ms
/ calls`, computed client-side same as the CLI does). No pagination — the
registered-tool count is small and bounded (unlike the audit log this
sub-project's sibling item B in sub-project 2 already paginated), a
scrollable list is enough.

## C. MCP per-server health signal

### New aggregation

A new function, `fold_mcp_server_stats`, living beside `fold_tool_stats` in
`crates/aivyx-channel/src/daemon_server.rs` and sharing its audit-walking/
cutoff-window logic (same `entries: &[SignedEntry]`, `cutoff:
Option<SystemTime>` shape). Difference: it only processes `ToolCall`
entries whose `scope_used.base() == "mcp.call"`, and groups by the server
name recovered from the qualifier.

**Splitting the qualifier**: `scope_used.qualifier()` for an MCP tool call
is `<server>:<tool>` (a single string, since `Scope::qualifier()` returns
everything after the *first* colon as one unit — the server/tool split is
this sub-project's own concern, not the `Scope` type's). MCP server names
have no colon-exclusion validation today (checked `write_mcp_server_section`
directly) but MCP tool names — sourced from the connected server's own tool
definitions — are conventionally simple identifiers. Split from the right
(`rsplit_once(':')`) rather than the left, so a colon-containing server
name (an edge case, not the common case) still parses correctly as long as
the tool name itself has no colon — which the qualifier's own construction
site (`proxy.rs`'s `format!("mcp.call:{server_name}:{tool_name}")`)
guarantees is the last segment either way.

```rust
struct McpServerAcc {
    calls: u64,
    outcomes: BTreeMap<String, u64>,
    total_duration_ms: u64,
}
// keyed by server name, extracted via qualifier.rsplit_once(':').map(|(s, _tool)| s)
```

Output type (new, in `aivyx-ipc`): `McpServerCallStats { server_name:
String, calls: u64, outcomes: BTreeMap<String, u64>, total_duration_ms:
u64 }` — deliberately NOT a new field on `McpServerStatusView`; see below
for why.

### Why a new query, not an extension of `GetMcpStatus`

`McpServerStatusView`/`GetMcpStatus` documents itself explicitly as a
boot-time file snapshot, not live daemon memory. Overloading that one wire
type with a second, different freshness contract (live/rolling vs.
as-of-last-start) would make every future reader of `GetMcpStatus` have to
reason about two different staleness models in one response. Instead: a
new `QueryPayload::GetMcpServerCallStats { window_secs: Option<u64> }` /
`QueryResponsePayload::McpServerCallStats { servers: Vec<McpServerCallStats>
}`, following the exact `GetToolStats` shape (including the same
`window_secs` semantics — `None` = whole chain, `Some(n)` = last `n`
seconds).

### UI: `McpServerCard`'s new health chip

`McpPanel` (`crates/aivyx-web/src/main.rs:4736`) fetches
`GetMcpServerCallStats { window_secs: Some(86_400) }` (fixed 24h window,
matching `[proactive]`'s own `DEFAULT_PROACTIVE_WINDOW_SECS` — no
operator-configurable picker; YAGNI) alongside its existing `GetMcpStatus`
fetch. `McpServerCard` (`main.rs:4838`) renders a second chip next to the
existing `connected`/`failed` pill, reusing the file's existing 3-tier chip
palette (`chip sage` / `chip amber` / `chip error`, already used elsewhere
in this file — no new CSS):

- No matching `McpServerCallStats` row (0 calls in the window) → neutral
  `chip` reading "no recent activity" — a configured-but-unused server is
  not itself unhealthy.
- Row present, `outcomes` has no `failed`/`denied` entries → `chip sage`,
  "N ok".
- Row present, some but not all calls failed → `chip amber`, "N ok / M
  failed".
- Row present, more than half the calls in the window failed → `chip
  error`, "N ok / M failed" (same text, red instead of amber — the
  boundary is `failed_count * 2 > calls`).

The existing `connected` pill and `error`/`stderr_tail` rendering are
unchanged — this is additive, not a replacement. An MCP server can show
`connected` (it answered the boot-time handshake) alongside a red call-stat
chip (its tools have been failing since) — that combination *is* the
finding this sub-project exists to surface.

## Testing

- `fold_mcp_server_stats`: unit tests mirroring `fold_tool_stats`'s own
  existing test shape — entries for 2+ different MCP servers' qualifiers
  correctly bucket separately; a non-`mcp.call` entry is ignored; a cutoff
  excludes entries appended before it; a qualifier with a colon in the
  server-name segment (`mcp.call:my:weird:server:tool`) still recovers the
  right tool name via `rsplit_once` (server name recovered as
  `"my:weird:server"`).
- `GetMcpServerCallStats`/`McpServerCallStats` wire round-trip test
  (serde), matching the existing `ToolStat`/`GetToolStats` test's shape.
- `View::Tools` (TUI): a render-function unit test against fixture
  `ToolStat` data, mirroring `render_tool_stats`'s own CLI test fixtures —
  the TUI's render helper should be a pure function over `&[ToolStat]`
  the same way the CLI's is, so both are testable without IPC.
- `McpServerCard`'s new chip: a component-level assertion isn't this
  codebase's convention for Dioxus UI (confirmed — no existing `#[cfg(test)]`
  coverage for `McpServerCard` itself), so this is verified by the
  chip-selection logic being a small, extractable pure function
  (`mcp_health_chip(stats: Option<&McpServerCallStats>) -> (&'static str,
  String)`) that CAN be unit-tested directly, same shape as `phase_class`/
  `eff_class`-style helpers already in this file.
- Full sweep before merge: `cargo clippy --workspace --exclude
  aivyx-desktop --all-targets -- -D warnings` and `cargo test --workspace
  --exclude aivyx-desktop` (or the `default-members`-only fallback this
  environment has needed before), plus `cargo test -p aivyx-web` and the
  wasm32 build/clippy pair, plus a `dist/` rebuild in the final task.
