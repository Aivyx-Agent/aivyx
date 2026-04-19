# Phase 36 — Ollama Model Management

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Give the agent the ability to inspect and manage local Ollama
models. Three new tools — `ollama.list`, `ollama.show`,
`ollama.pull` — let the agent discover available models, inspect
model metadata, and pull new models without requiring the
operator to switch to the Ollama CLI. Completes the local LLM
story started in Phase 34.

## Why now

1. **Phase 34 made Ollama first-class.** `ProviderKind::Ollama`
   with health check, optional API key, worked example. But the
   agent can't inspect or manage its own models — the operator
   must use `ollama pull` and `ollama list` out-of-band.

2. **Self-sufficient local agents.** An agent that can discover
   what models are available and pull new ones when needed is
   genuinely useful for autonomous workflows. A scheduled turn
   could check model availability before starting work.

3. **Clean, self-contained scope.** Three tools, three
   capability bases, no architecture changes. The `HttpTransport`
   seam from Phase 34 (with `get_text`) provides the GET path;
   a new `post_json` method handles the POST path for
   `/api/pull`.

## Ollama API endpoints

Ollama exposes a REST API at its base URL (default
`http://localhost:11434`):

- `GET /api/tags` — list locally available models. Returns
  `{ "models": [{ "name", "size", "modified_at", ... }] }`.
- `POST /api/show` — show model metadata. Body:
  `{ "name": "llama3.1" }`. Returns modelfile, parameters,
  template, license, etc.
- `POST /api/pull` — pull a model from the Ollama registry.
  Body: `{ "name": "llama3.1", "stream": false }`. With
  `stream: false`, blocks until complete and returns a single
  status object.

## Design decisions

- **Tools live in `aivyx-channel`**, not `aivyx-llm`. They are
  agent-facing tools (like mission or schedule tools), not
  provider internals. They use `reqwest` directly for Ollama
  API calls — they don't go through the `LlmProvider` or
  `HttpTransport` trait, because these aren't LLM inference
  requests.

- **Capability tier: Trusted only.** `ollama.list` and
  `ollama.show` are read-only but expose local system state.
  `ollama.pull` downloads data to the local machine. All three
  are gated to `CEILING_TRUSTED` (and `CEILING_KERNEL`).
  SemiTrusted/Untrusted channels cannot manage models.

- **`ollama.pull` uses `stream: false`.** Streaming pull
  progress is complex (chunked JSON with download percentages)
  and not useful for the LLM planner. A blocking pull with a
  generous timeout is simpler and sufficient. The tool reports
  success/failure, not progress.

- **Base URL from config.** The tools need to know where
  Ollama is running. When `provider = "ollama"`, they use the
  configured `base_url` (or default `localhost:11434`). When
  the provider is not Ollama, the tools are not registered.

## Streak predictions

- **DESIGN.md** -- Very low risk. No architecture change.
  Prediction: **untouched** (streak at 6 from Phase 35).

- **PRODUCT.md** -- Low risk. No product commitment change.
  Prediction: **untouched** (streak restarts at 1).

- **Production-core `aivyx-core/src/lib.rs`** -- Very low
  risk. Tools are in `aivyx-channel`. No core type changes.
  Prediction: **untouched** (streak at 4 from Phase 35).

## Tasks

### Task 1 -- Open commit + PHASE_36.md scaffold

This file. Update `docs/README.md` to show Phase 36 as Open.
Update `docs/ROADMAP.md` with Phase 36 active pointer.

### Task 2 -- Capability scopes: ollama.list, ollama.show, ollama.pull

Add three new scope bases to `KNOWN_BASES` in
`aivyx-capability`. Add them to `CEILING_TRUSTED` (and
`CEILING_KERNEL` gets them automatically). Omit from
`CEILING_SEMITRUSTED` and `CEILING_UNTRUSTED`. Add parsing
and ceiling tests.

### Task 3 -- OllamaListTool + OllamaShowTool

Create `crates/aivyx-channel/src/ollama_tools.rs`:

- `OllamaListTool`: `GET /api/tags`, returns model list.
  Scope: `ollama.list`. No input parameters.
- `OllamaShowTool`: `POST /api/show` with `{ "name": input }`,
  returns model metadata. Scope: `ollama.show`.

Both tools construct a `reqwest::Client` internally (same
pattern as `WebFetchTool`). Base URL passed at construction
time.

### Task 4 -- OllamaPullTool

`POST /api/pull` with `{ "name": input, "stream": false }`.
Scope: `ollama.pull`. Uses a longer timeout (5 minutes) since
model downloads can be large. Returns success/failure status.

### Task 5 -- Binary registration + conditional wiring

Register all three tools in `aivyx.rs` **only when
`provider = "ollama"`**. Pass the Ollama base URL to each
tool's constructor. Add the tools to the tool list after
the MCP tools block.

### Task 6 -- Tests

- Capability tests: scope parsing, ceiling grants/denies.
- Tool unit tests with mock HTTP responses (same pattern as
  `FakeTransport` in the LLM provider tests).
- Binary test: tools registered when provider is Ollama,
  not registered otherwise.

### Task 7 -- Exit freeze + docs

Exit criteria checklist, prediction-vs-reality table,
streak report.
