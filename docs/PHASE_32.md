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
- `transport: McpTransport` enum (`Stdio` | `Sse`)
- `url: Option<String>` — the SSE endpoint URL (required
  for SSE, absent for stdio)

## Tasks

### Task 1 -- Open commit + PHASE_32.md scaffold

This file. Update `docs/README.md` to show Phase 32 as Open.
Update `docs/ROADMAP.md` with Phase 32 active pointer.

### Task 2 -- Transport trait extraction

Extract `McpTransport` trait from `McpServerBridge`. Refactor
`McpServerBridge` to be generic over transport. Move the
existing stdio I/O into a `StdioTransport` struct. All
existing tests must still pass with zero behavior change.

### Task 3 -- SSE transport implementation

Implement `SseTransport` using `reqwest` for HTTP and a
minimal SSE parser for the response stream. The transport:
- GETs the SSE endpoint on connect
- Parses the `endpoint` event to discover the POST URL
- POSTs JSON-RPC requests to the endpoint URL
- Reads JSON-RPC responses from `message` SSE events

### Task 4 -- Config + binary wiring

Extend `McpServerConfig` with `transport` and `url` fields.
Wire SSE transport into the binary's MCP startup path. Add
`--mcp-sse` CLI flag or extend `--mcp-server` syntax.

### Task 5 -- Integration tests

Tests exercising the SSE transport against a mock HTTP
server. Prove initialize + tools/list + tools/call works
over SSE the same as over stdio.

### Task 6 -- Exit freeze + docs

Exit criteria checklist, prediction-vs-reality table, streak
report. Update PRODUCT_ROADMAP.md to mark MCP Integration
milestone as delivered.
