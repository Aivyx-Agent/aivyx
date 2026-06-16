# Phase 25 — Multi-Provider Support

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../../../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../../../PRODUCT.md).

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

**Task 3 ship record**

- `ProviderKind` enum (`Anthropic` | `OpenAi`) added to `aivyx-config`,
  serde-compatible for TOML deserialization.
- New `AivyxConfig` fields: `provider`, `openai_api_key`,
  `openai_base_url`.
- `RawToml` gains `[openai]` section (`api_key`, `base_url`) and
  `[agent] provider` field.
- Env vars: `AIVYX_PROVIDER`, `AIVYX_OPENAI_API_KEY`,
  `AIVYX_OPENAI_BASE_URL`. Standard env > TOML > default precedence.
- `validate()` now checks `openai_api_key` when provider is `OpenAi`,
  `anthropic_api_key` when `Anthropic` — no longer demands the wrong
  key for the active provider.
- `hydrate_secrets_from_store` reads `openai_api_key` from
  `KeyDomain::Secrets` alongside the existing Anthropic path.
- `--provider <anthropic|openai>` CLI flag in the binary, highest
  priority (overrides env and TOML).
- Provider construction in `run_async` dispatches between
  `AnthropicProvider` and `OpenAiProvider` based on `provider_kind`.
- `aivyx-channel/Cargo.toml` now enables `provider-openai` feature.
- Startup banner shows `provider` and conditional `openai_*` fields.
- Example TOML updated with `[agent] provider` and `[openai]` section.
- 7 config tests + 5 CLI tests. Workspace: 635 tests, 0 failures.
- Production-core streak extends to **sixteen** (hash `d8ab203f…`).
- Q2 resolved: provider selection is **global** via config/CLI, not
  per-role.

### Task 4 — Docs update + PRODUCT_ROADMAP delivery record

Mark Multi-Provider milestone as delivered in
`docs/PRODUCT_ROADMAP.md`. Update sequencing notes.
All three Task 4 candidates from the original plan are
resolved or deferred:

- **Tool-calling translation**: handled internally by
  `OpenAiProvider` — no core type changes (Q1 resolved in
  Task 2).
- **`--provider` for daemon mode**: daemon reads
  `AIVYX_PROVIDER` env var or `[agent] provider` TOML,
  which is sufficient — the `daemon run` parser does not
  need a `--provider` flag.
- **Provider-specific token counting**: deferred — both
  providers report usage via `LlmUsage` from the stream;
  model-specific tokenizer libraries are a future concern.

**Task 4 ship record**

- `PRODUCT_ROADMAP.md`: Multi-Provider milestone marked as
  delivered with delivery summary. Sequencing notes updated.
  Milestone section updated from "candidate" to "delivered."
- Remaining Task 4 candidates triaged: tool-calling already
  internal (Task 2), daemon provider already works via env/TOML
  (Task 3), token counting deferred.
- New deferral: **provider-specific token counting** (Phase 25).

## Deferrals

**Rolling deferrals carried from Phase 24 + Phase 25 (13 items):**

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
- **Provider-specific token counting** — Phase 25. Both
  providers report usage via `LlmUsage`; model-specific
  tokenizer libraries for pre-request estimation are a
  future concern.

## Open questions

**Q1 — Does the OpenAI tool-calling response format require
changes to `aivyx-core` types?** ✓ Resolved in Task 2: **no**.
The provider adapter translates between formats internally.

**Q2 — Should the provider be selectable per-role or only
globally?** ✓ Resolved in Task 3: **globally**. Config/CLI/env
selection, not per-role.

## Prediction vs reality

| Streak | Prediction | Outcome |
|--------|-----------|---------|
| DESIGN.md | Low risk — no edits | ✓ Unchanged (`ceb5386…`) |
| PRODUCT.md | Not at risk | ✓ Unchanged (`478cab6…`) |
| Production-core | Extends to 15 if trait composes, breaks if tool-calling needs core changes | ✓ Extends to **sixteen** — no core changes needed. Hash `d8ab203f…` unchanged. |

Phase scope prediction: "1 phase, quick win." Reality: **4 tasks
in 1 phase** — open, provider impl, config+CLI, docs. Clean fit.

## Exit criteria

- [x] `OpenAiProvider` implements `LlmProvider::stream_turn` with
  text streaming and tool-call assembly.
- [x] `provider-openai` feature compiles and passes 7 provider-
  specific tests.
- [x] Config wiring: `ProviderKind` enum, `--provider` CLI,
  `AIVYX_PROVIDER` env, `[agent] provider` + `[openai]` TOML.
- [x] `validate()` checks the correct API key per provider.
- [x] Binary dispatches between `AnthropicProvider` and
  `OpenAiProvider` at startup.
- [x] Shared `HttpTransport` seam (transport lift to crate root).
- [x] `PRODUCT_ROADMAP.md` Multi-Provider milestone delivered.
- [x] Both Q-block questions resolved.
- [x] 635 workspace tests, 0 failures.
- [x] Production-core streak intact (16 phases).
- [x] DESIGN.md unchanged.
- [x] PRODUCT.md unchanged.
