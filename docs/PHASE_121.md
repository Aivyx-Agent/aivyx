# Phase 121 — Native Ollama Tool-Calling Path (Local-LLM Rehab #3)

Second-named phase against the audit's local-LLM
rehabilitation axis. Phase 120 closed the tool-name
hallucination failure mode at the substrate boundary;
Phase 121 ships a native Ollama adapter that bypasses
the OpenAI-compat translation layer entirely, giving
Ollama-specific options (`num_ctx`, `num_predict`,
streaming fidelity) first-class treatment.

**Tenth consecutive substrate/polish phase picked over
the Channel Activation Milestone.** Honest tracking.
Audit ranking unchanged: the verification pass remains
the highest-information-value direction; the gap keeps
growing. Phase 121 opens against the audit's #3 axis at
operator pick.

**Honest scope caveat (carried forward from the
Phase 121 sign-off question):** the local-model
hallucination patterns Phase 120 addresses are model-
shaped, not protocol-shaped. A native Ollama adapter
will NOT make qwen3.6:27b stop emitting `fs_read`. What
it WILL give: per-model Ollama options, native
streaming, and no OpenAI-compat translation layer to
debug. Phase 6 Q5 honesty up front so the exit-doc
reality matches anticipation.

## Why this, why now

- **G6 (Local execution, privacy non-negotiable) is the
  project's distinguishing claim.** Phase 34 brought
  Ollama to first-class status via the OpenAI-compat
  path. That path treats Ollama as "OpenAI-shaped with
  different defaults" — which is true for the request
  body but loses fidelity for Ollama-specific options
  (`num_ctx`, `num_predict`, `num_thread`, `mirostat`,
  etc.) and for the JSONL streaming protocol Ollama
  uses natively.

- **The OpenAI-compat path is a translation layer.**
  Translation layers accumulate bugs over time; Phase
  120 caught one (the tool-name handling assumed
  cloud-LLM behavior). A dedicated native adapter
  surfaces Ollama-specific bugs against Ollama, not
  against the OpenAI proxy of Ollama.

- **Phase 116's substrate is local-LLM-friendly.** The
  Phase 116 relevance ledger + Phase 117's live-prompt
  pipe + Phase 120's recovery substrate all benefit
  from a provider that exposes Ollama's actual
  capabilities directly.

## Scope (Q-block sign-off)

- **Q1 — Adapter shape:** (a) **New dedicated
  `OllamaProvider`** (Recommended). Lives in a new
  `aivyx-llm/src/ollama/` module mirroring
  `aivyx-llm/src/openai/`. Implements `LlmProvider`
  directly; talks Ollama's `/api/chat` natively. The
  existing OpenAI-compat path stays untouched for
  explicit `ProviderKind::OpenAi` (cloud OpenAI or
  non-Ollama OpenAI-compat services).

- **Q2 — Streaming posture:** (a) **Full streaming —
  implement Ollama's JSONL stream protocol** (non-
  Recommended; picked over the pragmatic "non-
  streaming for tool calls" Recommended). Larger
  substrate but cleanest UX: text deltas stream through
  for chat-only turns; tool calls arrive in the final
  `done: true` chunk's `tool_calls` array. Honest
  scope-expansion call at sign-off.

- **Q3 — Coexistence with existing OpenAI-compat
  Ollama path:** (a) **`ProviderKind::Ollama` switches
  to native by default** (Recommended). `provider =
  "ollama"` in `aivyx.toml` routes to the new adapter.
  The existing OpenAI-compat code stays untouched for
  `provider = "openai"`. Operators using Ollama get
  native benefits transparently.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract amendment.
  Phase 121 ships a new LlmProvider implementation;
  the trait shape is unchanged. Hash at entry:
  `c2be6d51def04207ab0527b4f335edf7c726920b4f102185336eded01b1b995b`.
  Prediction: streak **extends to twelve** (was 11
  after Phase 120).

- **PRODUCT.md** — **Will hold.** G6 (Local execution,
  privacy non-negotiable) covers Ollama first-class
  support; Phase 121 strengthens that commitment by
  removing the OpenAI-compat translation indirection.
  No contract amendment. Hash at entry:
  `6e840cef2cfcee4c483c2b4769c29cc7062daeb4bf3802c53f48e72ebfe57fb8`.
  Prediction: streak **extends to twelve** (was 11).

- **Production-core `aivyx-core/src/lib.rs`** —
  **Will hold.** The new adapter lives in
  `aivyx-llm`, not `aivyx-core`. ProviderKind
  dispatch routing lives in the binary (aivyx-channel).
  Q1a's "dedicated module" choice deliberately keeps
  the changes off lib.rs. Hash at entry:
  `b420405bf9a5576ecb10f6ea04a965f7ad3a1a92ae9bd8f4abf46e22ef4d3c16`.
  Prediction: streak **re-establishes to two** (was 1
  after Phase 120 broke it). Honest 80/20 hold — the
  20% case fires only if aivyx-config needs a new
  Ollama-native options field that the binary
  destructures (which would touch the binary's
  AivyxConfig destructure but NOT lib.rs); even that
  case is borderline since `aivyx-config` and `aivyx-channel`
  aren't `aivyx-core`.

- **New workspace deps** — Zero anticipated. JSONL
  parsing reuses existing `serde_json` + the
  byte-stream transport already in `aivyx-llm`.

- **Test count** — Substrate-heavy: new request body
  builder + new JSONL line reader + new stream-builder
  + new provider impl + integration tests against
  scripted Ollama-shaped responses. Prediction:
  **+40 to +70**. Honest scope expansion from the
  Q2a non-Recommended pick.

## Tasks

Roughly seven sub-tasks plus exit + backfill:

### Task 1 — Open (this commit)

`docs/PHASE_121.md` + `docs/ROADMAP.md` Phase 121 entry +
`docs/README.md` status row.

### Task 2 — `OllamaProvider` skeleton + request body

- New `aivyx-llm/src/ollama/` module with `mod.rs`
  declaring the public types and `provider.rs` holding
  the implementation.
- `OllamaProvider` struct + `OllamaConfig` with fields:
  - `base_url` (defaults to `http://localhost:11434`)
  - `model` (required; the operator's model name)
  - Reuses the existing `Transport` for HTTP
- `build_request_body(request, options)` pure function
  produces Ollama's `/api/chat` JSON shape:
  - `{ model, messages, tools?, stream: true, options:
    {num_ctx, num_predict, ...} }`
  - `messages` format: same content-block treatment as
    the OpenAI path; tool_calls / tool_call_id arrays
    when present
  - `tools` format matches OpenAI's
    `[{type: "function", function: {name, description,
    parameters}}]`
- `OllamaOptions` struct carrying the Ollama-specific
  options (`num_ctx: Option<u32>`,
  `num_predict: Option<u32>`, etc.). Operator-
  configurable via aivyx-config in Task 6.
- Tests: request body round-trips through serde_json;
  tools array shape matches Ollama's documented
  format; options block omits unset fields.

### Task 3 — JSONL streaming line reader

- New `aivyx-llm/src/ollama/jsonl.rs` (or in `provider.rs`)
  with a `JsonlReader` wrapping `ByteStream`. Reads
  newline-delimited JSON objects:
  - Buffer incoming bytes
  - Emit one JSON value per `\n` boundary
  - Tolerate partial trailing lines across reads
  - Return None on stream EOF
- Pure-ish (state machine over ByteStream); no Ollama-
  specific knowledge.
- Tests: single-line emit; multi-line in one read;
  one line spanning two reads; trailing-no-newline EOF
  case.

### Task 4 — Stream state machine + chunk parser

- `OllamaStream` implementing `LlmStream`:
  - `next_event` consumes JSONL chunks via JsonlReader;
    each chunk has `{ message: { content?, tool_calls?
    }, done: bool, prompt_eval_count?,
    eval_count?, ... }`
  - Text chunks (`message.content` populated, `done: false`)
    emit `LlmStreamEvent::TextChunk`
  - Usage chunks (only on `done: true`) accumulate
    into `LlmStepEnd`'s `LlmUsage` (Ollama reports
    `prompt_eval_count` + `eval_count` as token counts)
  - Tool-call chunks: Ollama's `done: true` chunk
    typically carries the full `tool_calls` array;
    parse into `Vec<ToolCallEnd>` (with
    `NameResolution` populated via the same Phase 120
    Task 3 logic — validate against the request's
    advertised tool set)
- `finish` returns the terminal `LlmStepEnd`
- Tests: streamed text-only response; streamed
  text + final tool-call response; usage extraction;
  scripted JSONL stream produces the right
  `LlmStepEnd` shape.

### Task 5 — OllamaProvider::chat_stream integration

- `impl LlmProvider for OllamaProvider`:
  - `chat_stream` builds the request body via Task 2's
    helper, posts to `{base_url}/api/chat` via the
    existing `Transport`, wraps the byte stream in
    Task 3's `JsonlReader`, and constructs the
    `OllamaStream` from Task 4.
- Phase 120 Task 3 `NameResolution` populated against
  the request's `tools[].name` set — same belt-and-
  suspenders posture the OpenAI and Anthropic providers
  use.
- Integration tests with scripted JSONL responses
  exercising: chat-only turn, tool-call turn,
  hallucinated-name turn (NameResolution::Unknown
  populated).

### Task 6 — aivyx-config wiring + binary dispatch

- Extend `aivyx-config` with optional `[ollama]` TOML
  sub-section carrying `OllamaOptions` (`num_ctx`,
  `num_predict`, `num_thread`, etc.). Defaults `None`;
  Phase 6 Q5 honesty — Ollama's own defaults apply
  when the operator doesn't override.
- Binary dispatch in `aivyx-channel/src/bin/aivyx.rs`:
  when `provider_kind == ProviderKind::Ollama`,
  construct an `OllamaProvider` instead of falling
  through to the OpenAI-compat path.
- Tests: `[ollama]` section parses; absent section →
  `None`; binary dispatch picks the right provider
  based on `ProviderKind`.

### Task 7 — Scripted e2e + INSTALL.md sweep + exit

- Scripted e2e against the full stack: a mock
  Transport returning a canned JSONL response;
  OllamaProvider runs through chat_stream; assert the
  emitted events + terminal LlmStepEnd match the
  scripted shape.
- INSTALL.md: new "Native Ollama provider (Phase 121)"
  section covering the operator workflow:
  - `provider = "ollama"` now uses the native adapter
  - `[ollama]` options reference
  - When to pick `provider = "openai"` with an Ollama
    `base_url` (rare; operator needs OpenAI-compat
    behavior specifically)
  - How Phase 120 tool-call recovery interacts with
    the native path (it runs identically; provider-
    side NameResolution flagging is uniform across
    providers)
- Exit: PHASE_121.md prediction-vs-reality + ROADMAP
  freeze + README flip + hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Adapter shape:** (a) **New dedicated
  `OllamaProvider`** (Recommended).
- **Q2 — Streaming posture:** (a) **Full streaming —
  JSONL protocol** (non-Recommended; picked over
  pragmatic "non-streaming for tools").
- **Q3 — Coexistence:** (a) **`ProviderKind::Ollama`
  switches to native by default** (Recommended).

## Exit criteria

- [x] `docs/PHASE_121.md` + ROADMAP Phase 121 entry +
  docs/README status row — Task 1 (`97cc7d8`).
- [x] `OllamaProvider` skeleton + request body — Task 2
  (`7cb5566`).
- [x] JSONL streaming line reader — Task 3 (`d0ad4e7`).
- [x] Stream state machine + chunk parser — Task 4
  (`d7d9564`).
- [x] OllamaProvider::chat_stream integration — Task 5
  (`f8bcb55`).
- [x] aivyx-config wiring + binary dispatch — Task 6
  (`e87f574`).
- [x] Scripted e2e + INSTALL.md sweep — Task 7 (this commit).
- [x] Q1/Q2/Q3 resolved with operator sign-off pre-Task
  2 (Q1a + Q2a non-Recommended + Q3a recorded above).
- [x] DESIGN.md streak — **HELD as predicted**.
  `c2be6d51…` unchanged. Streak extends 11 → 12.
- [x] PRODUCT.md streak — **HELD as predicted**.
  `6e840cef…` unchanged. Streak extends 11 → 12.
- [x] `aivyx-core/src/lib.rs` streak — **HELD as
  predicted** (80/20 hold case carried through).
  `b420405b…` unchanged. Streak re-establishes 1 → 2
  after Phase 120's break. The new adapter lives
  entirely in `aivyx-llm`, the OllamaOptions struct in
  `aivyx-config`, and the dispatch routing in
  `aivyx-channel` — Q1a's "dedicated module" pick
  deliberately kept changes off lib.rs and the 80%
  case landed.
- [x] Zero new workspace dependencies. `provider-ollama`
  feature reuses existing transport deps.
- [ ] Test count delta `+54` (2346 → 2400) — within the
  predicted `+40 to +70` band.
- [x] Zero clippy warnings.
- [x] **Ollama users get first-class native treatment.**
  `provider = "ollama"` uses Ollama's `/api/chat`
  directly; the OpenAI-compat translation layer is no
  longer in the request path for local-model
  operators.
- [x] **Operator-facing changes are transparent or
  additive.** Existing `aivyx.toml` files with
  `provider = "ollama"` work unchanged after Phase 121
  ships (now routed through the native path); the
  new `[ollama]` options sub-section is purely
  additive (absent → Ollama's own defaults).

## Prediction vs reality

**Three of three streak predictions correct.** Phase 121
returns to the Phase 119 posture: all three streaks
held, the lib.rs streak re-established to 2 after Phase
120's break. The Q1a "dedicated module" pick paid off
exactly as the open doc anticipated.

- **DESIGN.md** — HELD as predicted (`c2be6d51…`
  unchanged). No contract amendment; Phase 121 ships a
  new LlmProvider implementation inside the existing
  trait shape. Streak: 11 → 12.
- **PRODUCT.md** — HELD as predicted (`6e840cef…`
  unchanged). G6 (Local execution, privacy non-
  negotiable) covers Ollama first-class support;
  Phase 121 strengthens that commitment by removing the
  OpenAI-compat translation indirection without
  contract change. Streak: 11 → 12.
- **`aivyx-core/src/lib.rs`** — HELD as predicted
  (80/20 hold case held). `b420405b…` unchanged. The
  new adapter lives in `aivyx-llm/src/ollama/`; the
  OllamaOptions config struct lives in `aivyx-config`;
  the dispatch routing in `aivyx-channel/src/bin/aivyx.rs`.
  Q1a's deliberate "dedicated module" architectural
  pick made this hold. Streak re-establishes 1 → 2.

**Test count `+54` is within the predicted `+40 to
+70` range.** Per-task breakdown:
- Task 2: 17 tests for the skeleton + request body
  builder (config helpers, endpoint, request shape,
  message translation).
- Task 3: 14 tests for the JSONL line reader (pure
  helper + async stream-driven cases including the
  load-bearing line-spanning-chunks reassembly).
- Task 4: 10 tests for the stream state machine
  (text-only, tool-call all-at-once on done, mixed
  text-then-tool-calls, hallucinated name flagging,
  multiple tool calls in one batch, malformed-chunk
  defensive paths).
- Task 5: 8 chat_stream integration tests (`/api/chat`
  endpoint routing, auth header behavior, text-only,
  tool-call, hallucinated name through provider, wire
  shape capture).
- Task 6: 4 aivyx-config tests for the `[ollama]`
  TOML section.
- Task 7: 1 final e2e test (split-chunk JSONL through
  the full pipeline).

**Q-block went through as operator-picked.** Q1a + Q2a
(non-Recommended full streaming) + Q3a all shipped.
The Q2a non-Recommended pick paid off: full JSONL
streaming supports text-deltas-while-tools shape (text
then tool calls on terminal) cleanly, which the
"non-streaming for tools" pragmatic option would have
sacrificed.

**Phase 120 substrate uniformity preserved.** The
provider-side `NameResolution` validation, the planner's
fuzzy-match recovery, and the audit chain's
`auto_corrected_from` field all flow through the native
Ollama adapter identically to how they flow through the
OpenAI and Anthropic providers. Phase 121 was substrate-
expansion at the LLM layer; Phase 120's substrate-fix at
the planner layer composes on top.

**Honest scope caveat held at exit.** The open doc
flagged that the native adapter does NOT fix model-shaped
hallucination patterns directly. Reality matches: the
hallucination recovery remains Phase 120's substrate
(running uniformly through both adapters); Phase 121's
contribution is native protocol fidelity, operator-
tunable Ollama options, and zero translation-layer
debugging. The honest-up-front sign-off carried through.

## Direction after Phase 121

After Phase 121, the local-LLM rehabilitation axis has
shipped two phases (Phase 120 tool-call recovery +
Phase 121 native Ollama). The third named option (per-
model prompt variants) becomes a candidate for follow-on
if real use exposes that small-context models still
struggle with verbose tool descriptions.

Phase 122 candidates (per audit ranking):

1. **Channel Activation Milestone** — eleventh phase in
   a row of substrate/polish would push this further.
   The verification gap keeps growing; the audit
   ranking stays unchanged.
2. **Per-model prompt variants** (local-LLM rehab #2,
   the still-unshipped #2 named direction).
3. **Release prep (v0.1.0 + shell installer)** —
   Phase 99 deferred publish-infrastructure decision.
4. **A new thematic Chapter F** — shaped by operator
   pressure.

Phase-by-phase decision at Phase 121 exit.
