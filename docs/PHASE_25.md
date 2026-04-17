# Phase 25 — Multi-Provider Support

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Add an OpenAI-compatible `LlmProvider` adapter so operators
can use GPT-4, Ollama, or any OpenAI-API-compatible endpoint
alongside the Anthropic provider. The `LlmProvider` trait is
already provider-agnostic; this phase delivers the second
concrete implementation.

## Why now

1. **The LlmProvider trait is settled.** It has been stable
   since Phase 1 and unchanged through 24 phases. The trait
   surface (`stream_turn`) is the only shape this phase needs
   to implement against.

2. **Multi-provider is the highest-leverage integration
   after MCP.** The PRODUCT_ROADMAP milestone identifies it
   as a quick win: the trait shape is settled, the adapter is
   a single crate feature, and one implementation unlocks
   GPT-4 + Ollama + every OpenAI-compatible endpoint.

3. **The MCP milestone is shipped (stdio).** Phase 23 + 24
   delivered the MCP adapter end-to-end. Multi-provider is
   the natural next integration surface.

## Streak predictions

- **DESIGN.md** — Low risk. The provider is a new feature
  flag in an existing crate, not a new crate or architectural
  decision.

- **PRODUCT.md** — Not at risk. No product commitment edits
  expected.

- **Production-core `aivyx-core/src/lib.rs`** — Medium risk.
  The `LlmProvider` trait lives in `aivyx-llm`, not
  `aivyx-core`. But if the OpenAI tool-calling shape requires
  a trait extension (e.g., different tool result format), the
  core types may need a change. Prediction: streak **extends
  to fifteen** if the existing trait composes, or **breaks**
  if tool-calling differences require core type changes.

## Tasks

### Task 1 — Open commit + PHASE_25.md scaffold

This file. Update `docs/README.md` to show Phase 25 as Open.

### Task 2 — OpenAI-compatible provider in `aivyx-llm`

Add `provider-openai` feature to `aivyx-llm` with an
`OpenAiProvider` implementing `LlmProvider::stream_turn`.
Target: OpenAI chat completions API with streaming
(`/v1/chat/completions`). Fields: `api_key`, `model`,
`base_url` (for Ollama/custom endpoints).

**Task 2 ship record**

- `provider-openai` feature added to `crates/aivyx-llm/Cargo.toml`
  with the same dependency set as `provider-anthropic`.
- `OpenAiProvider` implements `LlmProvider::stream_turn` targeting
  `/v1/chat/completions` with streaming.
- `OpenAiConfig { api_key, base_url }` — base_url defaults to
  `https://api.openai.com`, overridable for Ollama/local endpoints.
- Wire-format mapping: system prompt as first message (not top-level
  field), tools wrapped in `{"type":"function","function":{…}}`,
  `stream_options.include_usage` for token counts.
- Data-only SSE parser: no `event:` lines in OpenAI format; `[DONE]`
  sentinel terminates the stream.
- Tool call argument accumulation: incremental string concat across
  delta chunks, assembled into `LlmStepEnd::ToolCall` on
  `finish_reason: "tool_calls"`.
- `transport.rs` lifted from `anthropic/` to crate root with
  `#[cfg(any(feature = "provider-anthropic", feature = "provider-openai"))]`
  gating — shared `HttpTransport` seam for both providers.
- 7 new tests (text streaming, tool call assembly, request body
  structure, tool result mapping, assistant round-trip).
- Workspace: 617 tests pass, 0 failures. Production-core streak
  extends to **fifteen** (hash `d8ab203f…`).
- Q1 resolved: **No core type changes needed.** The provider
  translates between OpenAI and internal formats internally.

### Task 3 — Config + CLI wiring

`[provider]` section in `aivyx.toml` or `--provider` CLI
flag to select between `anthropic` (default) and `openai`.
`AIVYX_OPENAI_API_KEY` env var and `[openai] api_key` TOML
field.

### Task 4+ — Scope TBD at Task 3 exit

Candidates: tool-calling translation layer (if OpenAI's
tool format differs), `--provider` flag for daemon mode,
provider-specific token counting.

## Deferrals

**Rolling deferrals carried from Phase 24 (12 items):**

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

## Open questions

**Q1 — Does the OpenAI tool-calling response format require
changes to `aivyx-core` types?** Leaning (b) no — the
provider adapter should translate between formats internally.
But if the `ToolCall` / `ToolResult` types in `aivyx-core`
are too Anthropic-specific, this may require core changes.

**Q2 — Should the provider be selectable per-role or only
globally?** Leaning (a) globally — per-role providers add
complexity for a rare use case. Global selection via config
or CLI flag.
