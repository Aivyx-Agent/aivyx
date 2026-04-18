# Phase 32 — MCP SSE Transport

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Add HTTP/SSE transport for remote MCP servers, complementing
the existing stdio transport. This closes the Phase 23 deferral
and completes the MCP Integration milestone.

## Why now

1. **Last piece of the MCP milestone.** Phase 23 shipped the
   stdio transport; Phase 24 shipped the config + binary
   wiring. SSE is the only remaining item. Closing it marks
   the MCP Integration milestone as fully delivered.

2. **Unlocks remote MCP servers.** Stdio requires a local
   process; SSE connects to hosted MCP endpoints over HTTP.
   This is the transport most cloud-hosted MCP tools expose.

3. **Infrastructure already in place.** `reqwest` (with `stream`
   feature), `futures-util`, and `bytes` are already workspace
   deps. The LLM provider's SSE parser (`aivyx-llm/src/
   anthropic/sse.rs`) demonstrates the SSE parsing pattern.

## Streak predictions

- **DESIGN.md** -- Low risk. Transport addition, not
  architecture change. Prediction: **untouched**.

- **PRODUCT.md** -- Low risk. MCP is not yet a locked
  product commitment. Prediction: **untouched** (streak
  at 1 from Phase 31).

- **Production-core `aivyx-core/src/lib.rs`** -- Very low
  risk. MCP transport lives entirely in `aivyx-mcp`.
  Prediction: **untouched** (streak begins at 0 after
  Phase 31 reset).

## Architecture

### Transport trait

Extract the I/O concern from `McpServerBridge` into a trait:

```rust
#[async_trait]
pub trait McpTransport: Send + Sync {
    async fn send(&self, request: &str) -> Result<(), String>;
    async fn receive(&self) -> Result<String, String>;
}
```

`StdioTransport` wraps the existing stdin/stdout child-process
I/O. `SseTransport` wraps an HTTP client that POSTs requests
and reads SSE responses.

### MCP SSE protocol

The MCP SSE transport follows this flow:

1. Client GETs the server's SSE endpoint — a long-lived
   connection that receives server-to-client messages.
2. Server sends an `endpoint` SSE event containing a URL
   for client-to-server messages.
3. Client POSTs JSON-RPC requests to the endpoint URL.
4. Server sends `message` SSE events on the SSE stream
   containing JSON-RPC responses.

### Config extension

`McpServerConfig` gains:
- `transport: McpTransportKind` enum (`Stdio` | `Sse`)
- `url: Option<String>` — the SSE endpoint URL (required
  for SSE, absent for stdio)
- `command: Option<String>` — becomes optional (required
  for stdio, absent for SSE)

## Tasks

### Task 1 -- Open commit + PHASE_32.md scaffold

This file. Update `docs/README.md` to show Phase 32 as Open.
Update `docs/ROADMAP.md` with Phase 32 active pointer.

### Task 2 -- Transport trait extraction

Extract `McpTransport` trait from `McpServerBridge`. Refactor
`McpServerBridge` to be generic over transport. Move the
existing stdio I/O into a `StdioTransport` struct. All
existing tests must still pass with zero behavior change.

**Ship record:** `0947e6f`. Extracted `McpTransport` trait into
`transport_trait.rs`. `StdioTransport` in `stdio.rs` owns the
child process and implements `McpTransport` over stdin/stdout.
`McpServerBridge` now holds `Arc<dyn McpTransport>` and gains
`from_transport()` constructor. `McpToolProxy` refactored to
use `Arc<dyn McpTransport>` instead of raw stdin/stdout handles.
`start()` preserved as convenience for backward compat. 8
existing tests pass unchanged.

### Task 3 -- SSE transport implementation

Implement `SseTransport` using `reqwest` for HTTP and a
minimal SSE parser for the response stream. The transport:
- GETs the SSE endpoint on connect
- Parses the `endpoint` event to discover the POST URL
- POSTs JSON-RPC requests to the endpoint URL
- Reads JSON-RPC responses from `message` SSE events

**Ship record:** `0b1c19e`. `SseTransport` in `sse.rs` connects
via reqwest GET, parses `endpoint` event with labeled-break
pattern, spawns background tokio task to read `message` SSE events
into mpsc channel. Self-contained SSE parser (no `aivyx-llm`
dependency) handles LF/CRLF, comments, data-only events. POST
for send, channel receive for receive. 9 unit tests for parser
and URL resolution. Dependencies: `reqwest`, `futures-util`,
`bytes` added to `aivyx-mcp/Cargo.toml`.

### Task 4 -- Config + binary wiring

Extend `McpServerConfig` with `transport` and `url` fields.
Wire SSE transport into the binary's MCP startup path. Add
`--mcp-sse` CLI flag.

**Ship record:** `80d95e1`. `McpTransportKind` enum
(`Stdio` | `Sse`) in `aivyx-config`. `RawMcpServer` gains
`transport` (defaults `"stdio"`) and `url` fields; `command`
becomes `Option`. Config-time validation: `sse` requires `url`,
`stdio` requires `command`. Binary startup loop branches on
transport kind. `--mcp-sse name:url` CLI flag with `splitn(2,
':')` for URL-safe parsing. Banner shows transport label
(`stdio`/`sse`). 8 new tests (3 config, 5 CLI).

### Task 5 -- Integration tests

Tests exercising a non-stdio transport through the full bridge
lifecycle. Prove initialize + tools/list + tools/call works
over a channel-backed transport the same as over stdio.

**Ship record:** `9e06577`. `ChannelTransport` mock implements
`McpTransport` over tokio mpsc channels with in-line JSON-RPC
handler. 6 integration tests: discover, echo, add, unknown-tool
error, server name, proxy Tool::execute. All pass identically
to the stdio e2e tests.

### Task 6 -- Exit freeze + docs

Exit criteria checklist, prediction-vs-reality table, streak
report. Update PRODUCT_ROADMAP.md to mark MCP Integration
milestone as delivered.

## Exit criteria

- [x] Task 1 shipped: Phase 32 scaffold, README + ROADMAP
      updated.
- [x] Task 2 shipped at `0947e6f`: transport trait extracted.
- [x] Task 3 shipped at `0b1c19e`: SSE transport implementation.
      Phase 23 SSE deferral closed.
- [x] Task 4 shipped at `80d95e1`: config + binary wiring.
- [x] Task 5 shipped at `9e06577`: integration tests.
- [x] Task 6: this section.
- [x] 736 tests, 0 failures.
- [x] `cargo check` clean (only pre-existing MCP warnings).
- [x] DESIGN.md untouched.
- [x] PRODUCT.md untouched.

## Prediction vs reality

| Prediction | Reality | Notes |
|---|---|---|
| DESIGN.md untouched | Untouched | Correct |
| PRODUCT.md untouched | Untouched | Correct — streak extends to 2 |
| Production-core untouched (streak 0) | Untouched | Correct — streak extends to 1 |

## Streak report

| Target | Streak at entry | This phase | Streak at exit |
|---|---|---|---|
| DESIGN.md | extends | untouched | extends |
| PRODUCT.md | 1 | untouched | 2 |
| Production-core `lib.rs` | 0 (reset Phase 31) | untouched | 1 |

## Rolling deferrals at Phase 32 exit (7 items, -1 closed)

**Closed this phase:**
- MCP SSE transport (Phase 23) — Tasks 2–5

**Remaining (7 items):**
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
  Phase 17 Q6->(c+). Tagged: reactive.
- **Telegram-specific protocol extensions** — Phase 19.
  Untouched.
