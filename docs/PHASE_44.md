# Phase 44 — `aivyx init` Interactive First-Run Wizard

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Provide a user-friendly interactive setup experience for
non-technical end users. `aivyx init` detects whether Ollama
is running locally and defaults to it (zero API key required),
walks the user through provider and model selection, and writes
a ready-to-use `aivyx.toml` config file.

## Why now

1. **Barrier to entry.** The current launch requires manual
   environment variable setup or TOML editing. Non-technical
   users encounter cryptic errors about missing API keys.

2. **Ollama as default.** Local LLM support landed in Phase 34.
   Making it the default provider for new users gives a
   zero-config path that doesn't require cloud API accounts.

3. **Phase 43 context management.** With context-window pruning
   shipped, the agent handles long sessions gracefully. A
   smoother onboarding experience lets new users benefit from
   this immediately.

## Entry baseline

- Tests: 857
- Clippy warnings: 0
- Deferral backlog: 0
- DESIGN.md streak: 2 phases (untouched since Phase 41)
- PRODUCT.md streak: 7 phases (untouched since Phase 38)
- lib.rs streak: 0 phases (touched in Phase 43 Task 5)

## Streak predictions

| Streak target | Predicted | Notes |
|---|---|---|
| DESIGN.md | untouched (3) | No protocol or contract changes |
| PRODUCT.md | untouched (8) | Init wizard is not a product commitment |
| lib.rs | untouched (1) | All changes in binary + companion module |

## Tasks

### Task 1 — Phase open + CliMode::Init + CLI parsing

Scaffold this file. Add `Init` variant to `CliMode`. Parse
`"init"` as a positional subcommand. Dispatch in `run()`.

### Task 2 — Ollama detection + model listing

Create `init.rs` with `detect_ollama()` and
`list_ollama_models()` via `reqwest` + `serde_json::Value`.

### Task 3 — Interactive prompt helpers

`prompt_line`, `prompt_choice`, `prompt_secret`, `prompt_yes_no`
with `BufRead`/`Write` injection for testability.

### Task 4 — TOML generation

`InitConfig` struct + `render_toml()` with `format!()` templates
per provider (Ollama, Anthropic, OpenAI).

### Task 5 — Wire up `run_init_wizard()` end-to-end

Connect detection, prompts, TOML generation. TTY check,
overwrite guard, 0600 permissions, success message.

### Task 6 — Exit freeze

## Exit criteria

- [x] `aivyx init` subcommand parsed and dispatched.
- [x] Ollama detection via health check.
- [x] Model listing via `/api/tags`.
- [x] Interactive provider/model/path prompts.
- [x] TOML generation for all 3 providers.
- [x] File written with 0600 permissions (Unix).
- [x] All tests pass with net-positive delta.
- [x] Zero clippy warnings.
- [x] DESIGN.md untouched (streak -> 3).
- [x] PRODUCT.md untouched (streak -> 8).

## Ship record

| Metric | Value |
|---|---|
| Tests at entry | 857 |
| Tests at exit | 876 |
| Test delta | +19 |
| Clippy warnings | 0 |
| DESIGN.md | untouched (streak → 3) |
| PRODUCT.md | untouched (streak → 8) |
| lib.rs | untouched (streak → 1) |
| Deferral backlog | 0 |

### Commits

| Commit | Description |
|---|---|
| `09e4337` | Task 1: `aivyx init` subcommand + CLI parsing |
| `83fbf72` | Task 2: Ollama detection and model listing |
| `effd764` | Task 3: interactive prompt helpers |
| `5c8cd33` | Task 4: TOML generation |
| `6f54020` | Task 5: wire up `run_init_wizard()` end-to-end |

### Streak predictions vs actuals

| Streak | Predicted | Actual |
|---|---|---|
| DESIGN.md | untouched (3) | untouched (3) ✓ |
| PRODUCT.md | untouched (8) | untouched (8) ✓ |
| lib.rs | untouched (1) | untouched (1) ✓ |
