# Phase 46 — Web Search + Document Retrieval (Bundled MCP)

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Ship the first **bundled MCP server** inside the `aivyx` binary:
`aivyx mcp-server web-search`. Provides `web_search` (query →
result list) and `web_read` (URL → cleaned text) tools. Search
backend hierarchy: Brave Search API → SerpAPI → DuckDuckGo HTML
scraping (zero-config fallback). Exercises the full MCP client
pipeline (Phase 23 stdio, Phase 32 SSE) with a self-spawning
server that sets the pattern for future bundled capabilities.

## Why now

1. **Biggest capability gap.** Text-only interaction plus raw
   HTTP is not enough — users expect "search the web" to work.
   P10 locks the substrate at 8 tools, so new capabilities must
   come through MCP.

2. **MCP pipeline ready.** Config → bridge → discover → proxy →
   registry is complete. The missing piece is a server worth
   spawning.

3. **Phase 45 multimodal.** With image input shipped, the next
   barrier is information access — the agent can see but can't
   look things up.

## Entry baseline

- Tests: 893
- Clippy warnings: 0
- Deferral backlog: 0
- DESIGN.md streak: 4 phases (untouched since Phase 41)
- PRODUCT.md streak: 9 phases (untouched since Phase 37)
- lib.rs streak: 0 phases (touched in Phase 45)

## Streak predictions

| Streak target | Predicted | Notes |
|---|---|---|
| DESIGN.md | untouched (5) | No D3 contract change |
| PRODUCT.md | untouched (10) | MCP tools are not a product commitment |
| lib.rs | untouched (1) | No core type changes |

## Tasks

### Task 1 — Phase scaffold + `mcp-server` subcommand parser

Scaffold this file. Add `CliMode::McpServer(String)` variant.
Parse `aivyx mcp-server <name>` in `parse_cli_args_from()`.
Wire dispatch in `run()`. Create `mcp_server.rs` stub module.

### Task 2 — MCP server stdio harness

Build stdio JSON-RPC harness in `mcp_server.rs`. Dispatch
`initialize`, `tools/list`, `tools/call`, `shutdown`, `exit`.
Echo tool for smoke testing.

### Task 3 — `web_search` tool (DuckDuckGo zero-config)

Implement `web_search` tool handler with DuckDuckGo HTML
scraping. Canned-HTML unit tests.

### Task 4 — `web_read` tool (URL fetch + HTML-to-text)

Implement `web_read` tool handler. HTML-to-text extraction,
title extraction, 10 MiB body cap.

### Task 5 — `bundled` config flag + init wizard integration

Add `bundled` field to `McpServerConfig`. Resolve to
`current_exe()`. Init wizard web search prompt.

### Task 6 — Brave Search + SerpAPI backends

API-key detection, Brave/SerpAPI JSON parsers, backend
selector with fallback to DuckDuckGo.

### Task 7 — Exit freeze

## Ship records

| Task | Commit | Tests after |
|---|---|---|
| 1 | `095bbe6` | 897 |
| 2 | `261143c` | 904 |
| 3 | `2ec0eea` | 915 |
| 4 | `6b3bacb` | 925 |
| 5 | `71a3a7d` | 929 |
| 6 | `e53b510` | 936 |
| 7 (exit) | `074d167` | 936 |

## Exit criteria

- [x] `aivyx mcp-server web-search` subcommand parses and dispatches.
- [x] Stdio JSON-RPC harness handles full MCP lifecycle.
- [x] `web_search` tool with DuckDuckGo HTML scraping.
- [x] `web_read` tool with HTML-to-text extraction.
- [x] `bundled = true` config flag resolves to `current_exe()`.
- [x] Init wizard offers web search opt-in.
- [x] Brave Search API backend with JSON parser.
- [x] SerpAPI backend with JSON parser.
- [x] Backend selector hierarchy: Brave → SerpAPI → DuckDuckGo.
- [x] All tests pass with net-positive delta.
- [x] Zero clippy warnings.
- [x] DESIGN.md untouched (streak -> 5).
- [x] PRODUCT.md untouched (streak -> 10).

## Exit stats

- Tests: 893 -> 936 (+43)
- Clippy warnings: 0
- Deferral backlog: 0

### Streak outcomes

| Streak target | Predicted | Actual | New streak |
|---|---|---|---|
| DESIGN.md | untouched (5) | untouched | 5 |
| PRODUCT.md | untouched (10) | untouched | 10 |
| lib.rs | untouched (1) | untouched | 1 |

All three predictions correct.
