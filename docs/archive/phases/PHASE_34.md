# Phase 34 — Ollama Foundation

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../../../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../../../PRODUCT.md).

## Goal

Make local LLM usage via Ollama a first-class experience,
equally smooth as cloud providers. Phase 25 shipped the
OpenAI-compatible adapter; this phase removes the friction
that makes local deployment feel like an afterthought.

## Why now

1. **Growing user interest in local-only agents.** Privacy,
   cost, latency, and offline operation are strong motivators.
   Ollama is the dominant local LLM runtime and exposes an
   OpenAI-compatible API — but several paper-cuts in Aivyx's
   current wiring make it harder to use than it should be.

2. **The plumbing exists.** `OpenAiProvider` already targets
   `/v1/chat/completions` and handles streaming + tool calls.
   The work is removing friction, not building a new provider.

3. **Small, high-impact phase.** Each change is independently
   useful and testable. No architecture changes expected.

## Friction points this phase addresses

1. **API key required when not needed.** Ollama ignores API
   keys, but `validate()` demands one when `provider = openai`.
   Users must set a dummy `api_key = "ollama"`.

2. **No `provider = "ollama"` sugar.** Users must know to set
   `provider = "openai"` + `base_url = "http://localhost:11434"`
   — not discoverable.

3. **`stream_options.include_usage` may fail.** Older Ollama
   versions don't support this field. If the server rejects
   unknown fields, the entire request fails.

4. **No connection health check.** Cloud APIs are always up.
   A local Ollama instance might not be running, might be
   loading a model, or might be mid-download. The error
   surface is a raw `reqwest` transport error with no
   actionable guidance.

5. **Banner doesn't show Ollama context.** When running
   against Ollama, the startup banner should show the
   effective base URL and provider clearly so operators
   know they're hitting their local instance.

## Streak predictions

- **DESIGN.md** -- Very low risk. No architecture change.
  Prediction: **untouched**.

- **PRODUCT.md** -- Low risk. Local LLM support is an
  enhancement to the existing provider surface, not a new
  product commitment. Prediction: **untouched** (streak
  at 3 from Phase 33).

- **Production-core `aivyx-core/src/lib.rs`** -- Very low
  risk. All changes are in `aivyx-config`, `aivyx-llm`, and
  the binary. Prediction: **untouched** (streak at 2 from
  Phase 33).

## Tasks

### Task 1 -- Open commit + PHASE_34.md scaffold

This file. Update `docs/README.md` to show Phase 34 as Open.
Update `docs/ROADMAP.md` with Phase 34 active pointer.

### Task 2 -- ProviderKind::Ollama + optional API key

Add `ProviderKind::Ollama` as a config-level variant that
maps to the OpenAI-compatible provider with Ollama-specific
defaults:
- `base_url` defaults to `http://localhost:11434` (not the
  OpenAI default)
- API key is optional (not required by `validate()`)
- `--provider ollama` CLI flag, `AIVYX_PROVIDER=ollama` env

Under the hood, `Ollama` still constructs an `OpenAiProvider`
— the variant is config-level sugar, not a new provider impl.
The `OpenAiConfig` gains an `api_key_required: bool` field (or
the API key becomes `Option`) so the provider can skip the
`Authorization` header when no key is set.

### Task 3 -- Graceful stream_options handling

Make the `stream_options.include_usage` field conditional:
- For `ProviderKind::OpenAi` (cloud): always send it (cloud
  APIs support it).
- For `ProviderKind::Ollama`: omit it by default. Add an
  opt-in config field if needed.

This prevents request failures on older Ollama versions
that reject unknown fields.

### Task 4 -- Connection health check + actionable errors

When `provider = ollama`, attempt a lightweight health check
(GET to the base URL, which Ollama responds to with
`"Ollama is running"`) before the first LLM request. On
failure, emit an actionable error:
- "Ollama is not running at http://localhost:11434 — start
  it with `ollama serve`"
- "Ollama is running but model X is not available — pull it
  with `ollama pull X`"

This replaces the raw `reqwest` transport error with
guidance the user can act on.

### Task 5 -- Banner + worked example

- Update `print_config_banner` to show Ollama-specific
  context when `provider = ollama`.
- Add `examples/aivyx-ollama.toml` as a worked example for
  local LLM setup, mirroring the teaching style of
  `examples/aivyx.toml`.
- Update `examples/aivyx.toml`'s provider section comment
  to mention `provider = "ollama"` alongside `"openai"`.

### Task 6 -- Exit freeze + docs

Exit criteria checklist, prediction-vs-reality table,
streak report.

## Ship records

### Tasks 2–3 — `e805580`

**ProviderKind::Ollama + optional API key + conditional
stream_options.**

- `ProviderKind::Ollama` variant with `is_openai_compatible()`
  helper; validation skips API-key requirement for Ollama.
- `OpenAiConfig.api_key` → `Option<SecretString>`;
  `without_api_key()` ctor omits Authorization header.
- `include_stream_usage` bool gates `stream_options` in
  request body (defaults off for `without_api_key()`).
- `DEFAULT_OLLAMA_BASE_URL` constant
  (`http://localhost:11434`).
- CLI `--provider ollama` flag; banner shows default base URL.
- 14 new tests (6 config, 7 provider, 1 CLI). 754 total.

### Tasks 4–5 — `c076c24`

**Health check + worked example.**

- `HttpTransport::get_text` default method (backward-compatible
  trait widening) + `ReqwestTransport` implementation.
- `OpenAiProvider::health_check()` — GET to base URL, returns
  actionable error with `ollama serve` guidance on failure.
- Binary performs non-fatal health check at Ollama provider
  construction; warns with actionable message if unreachable.
- `examples/aivyx-ollama.toml` — complete worked example for
  local LLM setup (provider, model, role config, optional
  overrides).
- Updated `examples/aivyx.toml` provider comment to document
  all three provider values and cross-reference the Ollama
  example.
- 2 new health-check tests. 756 total.

## Exit criteria

- [x] `ProviderKind::Ollama` parses from TOML, env, CLI flag.
- [x] `validate()` accepts Ollama without API key.
- [x] `OpenAiConfig.api_key` is `Option<SecretString>`;
      Authorization header omitted when `None`.
- [x] `stream_options.include_usage` omitted for Ollama by
      default; opt-in via `with_include_stream_usage(true)`.
- [x] `DEFAULT_OLLAMA_BASE_URL` = `http://localhost:11434`.
- [x] Health check GETs base URL at startup; warns (non-fatal)
      with `ollama serve` guidance on connection failure.
- [x] `examples/aivyx-ollama.toml` worked example ships.
- [x] `examples/aivyx.toml` provider comment documents all
      three values and cross-references Ollama example.
- [x] Banner shows Ollama base URL (default or explicit).
- [x] All existing tests pass; 16 new tests added.
- [x] DESIGN.md untouched.
- [x] PRODUCT.md untouched.
- [x] `aivyx-core/src/lib.rs` untouched.

## Prediction vs reality

| Streak file                  | Predicted   | Actual      |
|------------------------------|-------------|-------------|
| DESIGN.md                    | untouched   | untouched   |
| PRODUCT.md                   | untouched   | untouched   |
| aivyx-core/src/lib.rs        | untouched   | untouched   |

All three predictions confirmed. Streaks extend:
- DESIGN.md: 5 (Phases 30–34)
- PRODUCT.md: 4 (Phases 31–34)
- production-core: 3 (Phases 32–34)

## Streak report

| Streak file             | Last touched | Current run          |
|-------------------------|--------------|----------------------|
| DESIGN.md               | Phase 29     | 5 (Phases 30–34)     |
| PRODUCT.md              | Phase 30     | 4 (Phases 31–34)     |
| aivyx-core/src/lib.rs   | Phase 31     | 3 (Phases 32–34)     |

## Summary

Phase 34 made local LLM usage via Ollama a first-class
experience. `ProviderKind::Ollama` is config-level sugar —
not a new provider implementation — that constructs the same
`OpenAiProvider` with Ollama-appropriate defaults: optional
API key, omitted `stream_options`, and `localhost:11434` as
the default base URL. A non-fatal health check at startup
gives actionable guidance when Ollama is unreachable.

Test delta: +16 (740→756). Deferral backlog unchanged at 6.
