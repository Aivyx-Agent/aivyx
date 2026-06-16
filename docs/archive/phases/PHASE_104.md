# Phase 104 — `aivyx init` Polish (Chapter C opener)

Chapter B closed at Phase 103 with the tool layer in good
shape. Phase 104 opens **Chapter C — Operator Onboarding**, the
arc that closes the gap between *fresh install* and *first
successful turn*. The first item is the first thing every new
operator touches: the `aivyx init` wizard.

The wizard has worked since Phase 44, and templates were added
in Phase 66. Driving it end-to-end as a fresh operator surfaces
three concrete papercuts that have accumulated since:

- **Stale provider defaults.** The Anthropic default model is
  `claude-sonnet-4-20250514` — about a year out of date; the
  current Sonnet identifier is `claude-sonnet-4-6`. The OpenAI
  default is `gpt-4o`; the current flagship is `gpt-4.1`. A
  fresh operator who accepts the default gets a config that
  may fail immediately on first turn because the model id no
  longer resolves.
- **No-models guidance.** When `list_ollama_models` returns
  empty, the wizard prints `No local models found. Run "ollama
  pull <model>" first.` and re-prompts for a free-text model
  name. A new operator with no Ollama experience has nothing
  to type. The wizard knows the operator picked Ollama and has
  no models — it should suggest one.
- **Write-then-find-out-later.** The wizard collects
  provider + key + model, then writes `aivyx.toml` and exits
  with `Run aivyx to start the agent.` If the key is wrong or
  the model name is a typo, the operator discovers it on first
  turn — *after* committing to a passphrase, opening the
  daemon, and waiting for the first request to come back as a
  4xx. The wizard should verify the combination before writing
  so the config that lands on disk is known-good.

Phase 104 closes all three. It refreshes the Anthropic/OpenAI
defaults to the current generation, adds a `Try: ollama pull
llama3.2:3b` suggestion on the empty-models path (single
copy-pasteable command, not a menu), and adds a `verify-before-
write` step that hits the provider's `GET /v1/models` with the
supplied key, confirms the chosen model is in the returned
list, and re-prompts on failure so a broken config never lands
on disk.

## Why this, why now

- **It is the highest-leverage onboarding paper-cut.** Every
  new operator's first interaction with the substrate is
  `aivyx init`. The three papercuts compound: stale defaults
  + no verify means a default-accepting operator writes a
  config that fails on first turn with no in-wizard
  indication that anything was wrong. Closing the loop here
  unblocks the rest of Chapter C.
- **It does not depend on the held public-hosting decision.**
  Phase 61's `v0.1.0` publication is still gated on the
  hosting choice in the operator's memory. Init polish ships
  against the existing `cargo install --path` and
  `scripts/dev-run.sh` paths; the polish lands now and pays
  off the moment `v0.1.0` does publish.
- **The verify substrate already exists.** `aivyx-llm` ships
  an `OpenAiProvider` and an `AnthropicProvider`; both
  providers' `/v1/models` endpoints are documented and
  reachable with the same `reqwest::Client` the wizard already
  uses to detect Ollama. Phase 104 reuses the shared
  `HttpTransport` seam that Phase 25 lifted to the crate root.
- **The change is additive on the operator-facing surface.**
  Every prompt the wizard already asks remains; the only new
  prompts are an inline retry on verify-failure. An operator
  who runs `aivyx init` today with a working key gets a
  byte-similar experience plus an extra `Verifying provider…
  ok` line.

## Streak predictions

- **DESIGN.md** — **Will hold.** Init wizard polish is a pure
  operator-side UX change; it touches no locked technical-
  contract decision, no daemon-IPC variant, no capability
  scope. Hash at entry:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Prediction: streak **extends to fifty-one** (was 50 at
  Phase 103 exit — already a project milestone).

- **PRODUCT.md** — **Will hold.** No P-* commitment is being
  amended; the change is entirely below the commitment line
  (P10's enumerated tool list, P11/P12, P13/P14 identity all
  unchanged). Hash at entry:
  `9f0a515c9076544866aa955d6835763ee15beb0d009b4c335f5710b6d9ba61d3`.
  Prediction: streak **extends to four** (was 3).

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold.**
  Every line of Phase 104 lives in `aivyx-channel`'s binary
  (`aivyx_modules/init.rs`) and possibly a small helper added
  to `aivyx-llm` (a `verify_provider_credentials` free
  function callable without a full `LlmProvider`
  construction); `aivyx-core` is not touched. Hash at entry:
  `ab3f9730c692917023239bbdd7c375497459e2a7fb3bbf08c007b5c945c6210d`.
  Prediction: streak **extends to four** (was 3).

- **New workspace deps** — Zero. `reqwest` is already a direct
  dep of `aivyx-llm` and (via `aivyx-llm`) of `aivyx-channel`;
  the wizard already uses it for Ollama detection.

- **Test count** — Positive. New tests cover: refreshed
  default model strings in `render_toml` output, the empty-
  models hint string, and the verify path against a mock HTTP
  server (success + 401 + model-not-in-list + network-error
  paths). Plus the existing parse_model_names + prompt_choice
  tests remain green. Rough prediction: **+8 to +12**.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_104.md` + `docs/ROADMAP.md` entry (Chapter C
opener section + Phase 104 entry marked `Active`) +
`docs/README.md` status row.

### Task 2 — Substrate refresh + verify-before-write

- **Default model refresh.** `init.rs` line 642
  (`"claude-sonnet-4-20250514"`) → `"claude-sonnet-4-6"`.
  Line 661 (`"gpt-4o"`) → `"gpt-4.1"`. Update the
  corresponding `render_toml_anthropic` / `render_toml_openai`
  test assertions in the same module.
- **Ollama no-models hint.** Replace the existing
  `eprintln!("No local models found. Run \`ollama pull
  <model>\` first.")` with a two-line block that names a
  concrete starter: `No local models found. Try: ollama pull
  llama3.2:3b` (3B is the right tier for a fresh-laptop first
  turn — small enough to download in seconds, capable enough
  to drive a real conversation). The prompt that follows
  stays the same (free-text model name) so the operator can
  still pick whatever they end up pulling.
- **Verify-before-write.** A new `verify_provider_credentials`
  helper somewhere reusable (likely `aivyx-llm` as a
  `pub async fn` since both providers' verify-paths share the
  same shape: `GET /v1/models` with `Authorization: Bearer <key>`
  for OpenAI and `x-api-key: <key>` for Anthropic, parse the
  returned `data: [{id, ...}]` list, return `Ok(Vec<String>)`).
  The wizard calls it between step 4 (key/model collection)
  and step 6 (render + write). On `Err`, print the error,
  loop back to re-prompt for whichever input is implicated
  (key for 401/403, model for `model not found`, both for
  network errors that could be either). The retry loop has
  a hard cap of three attempts before falling through to a
  final `Write anyway? [y/N]` — verify is a guardrail, not a
  lock.
- **Banner.** A single `Verifying provider… ok` line on
  success (matching the existing `Ollama detected at …`
  style) so the operator sees the check happen.

### Task 3 — Tests + docs + exit

- **Defaults tests.** `render_toml_anthropic` and
  `render_toml_openai` assertions updated to the new model
  ids. New `default_model_is_current` style test that pins
  the constant directly.
- **No-models hint test.** A small string-shape assertion
  that the new hint contains the suggested `ollama pull
  llama3.2:3b` command.
- **Verify path tests.** Three new tests against a
  `wiremock`-style local HTTP server (or hand-rolled
  `tokio::net::TcpListener` if a new dep is unwelcome — Phase
  101 took the `jsonschema` dep deliberately, but Phase 104
  is supposed to be deps-free): (1) 200 response with the
  chosen model present → `Ok(())`; (2) 401 → `Err(Auth)`;
  (3) 200 with the chosen model *not* present →
  `Err(ModelNotFound)`. The mock server pattern lives in
  `aivyx-llm`'s existing test infrastructure.
- **`docs/INSTALL.md`** — a one-line note in the existing
  `aivyx init` section that the wizard verifies the
  provider/key/model before writing.
- Exit: ROADMAP frozen entry, docs/README status flip,
  prediction-vs-reality recap, hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Verify mechanism:** (a) **`GET /v1/models` +
  membership check.** For Anthropic + OpenAI, fetch the
  models list with the supplied key and confirm the chosen
  model is in the returned list. Ollama already uses
  `/api/tags` (same pattern). Zero spend, single round-trip,
  catches both auth-failed and model-not-found in one call.
  Chosen over a minimal-tokens completion (real spend, no
  added coverage in practice) and over auth-only HEAD (does
  not catch typo'd model names — half the failure mode the
  verify step exists to prevent).
- **Q2 — Verify-failure handling:** (a) **Re-prompt for
  key/model, retry on the spot.** On failure, print the
  error and loop back to the appropriate prompt — key for
  401/403, model for not-in-list. Operator never gets a
  broken config written. Hard cap of three retries then a
  `Write anyway? [y/N]` so verify is a guardrail, not a
  lock. Chosen over a single `Write anyway?` (ships broken
  state by default if they say yes) and over abort (loses
  the rest of the operator's wizard inputs).
- **Q3 — Default model refresh:** (a) **Sonnet 4.6 /
  GPT-4.1.** Anthropic default `claude-sonnet-4-6`
  (current Sonnet, balanced cost/capability — the natural
  default per the project's environment), OpenAI default
  `gpt-4.1` (current flagship). Chosen over Opus 4.7 + GPT-4.1
  (max cost defaults; fresh operators usually don't want the
  Opus tier on turn 1) and over Haiku 4.5 + GPT-4.1-mini
  (cheap defaults; the substrate is happiest at the
  mid-tier).
- **Q4 — Ollama no-models hint:** (a) **Single hardcoded
  suggestion** — `Try: ollama pull llama3.2:3b`. One
  copy-pasteable command; operator unblocked immediately.
  Chosen over a per-tier shortlist (more choice, more text,
  more decision fatigue at the first-touch moment) and over
  keeping the current behavior (one of the three papercuts
  Phase 104 exists to close).

## Prediction vs. reality

**All three streak predictions held; test-count
prediction over-shot.**

- **DESIGN.md — held.** Init wizard polish is purely
  operator-side; no locked technical-contract decision
  touched, no daemon IPC variant added, no capability
  scope amended. Byte-identical at exit (hash still
  `89dc8903…a94bce`). Streak: **51 consecutive phases** —
  one past the Phase 103 half-hundred milestone.
- **PRODUCT.md — held.** No P-* commitment touched; the
  change lives entirely below the commitment line.
  Byte-identical at exit (hash still `9f0a515c…ba61d3`).
  Streak: **4 consecutive phases** (was 3).
- **`aivyx-core/src/lib.rs` — held.** Every line of Phase
  104 lives in `aivyx-channel/src/bin/aivyx_modules/init.rs`
  and the new `aivyx-llm/src/verify.rs` module; `aivyx-core`
  is untouched. Byte-identical at exit (hash still
  `ab3f9730…c6210d`). Streak: **4 consecutive phases**
  (was 3).

**New workspace deps — zero, as predicted.** `reqwest`
was already a direct dep of `aivyx-channel` and a
feature-gated dep of `aivyx-llm` (the verify helper is
gated on `any(provider-anthropic, provider-openai)`,
matching the existing module convention).

**Test count — `+17`** (workspace `1808 → 1825`),
**outside** the predicted `+8` to `+12` band by 5.
Breakdown:
- 15 tests in `aivyx-llm/src/verify.rs`:
  `parse_models_response × 5` (success, missing-id
  entries, empty data, missing `data` field, invalid
  JSON); `classify_response × 7` (200 ok, 401 → Auth,
  403 → Auth, 500 → Other, model-not-in-list,
  200-unparseable, long-body truncation);
  `summarize_models × 3` (empty list, within cap,
  truncate with overflow count).
- 2 tests in `aivyx-channel/.../init.rs`:
  `default_models_are_current` pins the refreshed
  constants; `ollama_empty_hint_includes_concrete_pull_command`
  pins the empty-models hint string.

The over-shoot is in the verify module's
parse-and-classify coverage. The classifier is the
piece that distinguishes auth vs. model-not-found in
the wizard's retry loop, and a faulty classifier would
silently route the operator to the wrong re-prompt
(re-asking for the model when the key was the problem,
or vice-versa). Exhaustive branch coverage on a
security-adjacent guardrail felt right; the prediction
was simply too tight.

**Scope — all three tasks shipped as planned.** Tasks 2
and 3 merged into one commit per the Phase 103 pattern;
exit is its own commit.

**End-to-end notes.** The wizard's verify path was not
driven against the live Anthropic / OpenAI endpoints
during phase work (would require operator-supplied
credentials and live network). It was verified by unit
tests against the extracted `classify_response` pure
function covering every error branch. The live HTTP
layer in `verify_provider_credentials` is treated as
integration-tested-only — the same posture
`detect_ollama` and `list_ollama_models` have held
since Phase 44.

## Exit criteria

- [x] `docs/PHASE_104.md` + ROADMAP Chapter C section +
  Phase 104 entry + docs/README status row — Task 1
  (commit `f30dc80`).
- [x] Refreshed Anthropic + OpenAI default model strings
  in `init.rs` + matching test updates — Task 2 (commit
  `7855329`).
- [x] Ollama empty-models hint line updated to include a
  concrete `ollama pull llama3.2:3b` suggestion — Task 2
  (commit `7855329`).
- [x] `verify_provider_credentials` helper in `aivyx-llm`
  + wizard wiring with three-retry cap + `Write anyway?`
  fallthrough — Task 2 (commit `7855329`).
- [x] Verify-path tests covering 200-ok / 401 /
  model-not-in-list / 500 / unparseable / long-body
  truncation — Task 3 (commit `7855329`). Tested at the
  pure `classify_response` seam rather than against a
  mock HTTP server (avoids a new dev-dep on `wiremock`
  and matches the wizard's existing
  `detect_ollama` / `parse_model_names` test posture).
- [x] `docs/INSTALL.md` mention — Task 3 (commit
  `7855329`).
- [x] ROADMAP + docs/README refreshed at exit — this
  commit.
- [x] All four Q-block questions resolved with operator
  sign-off pre-Task 2 (recorded above).
- [x] DESIGN.md streak extends to fifty-one.
- [x] PRODUCT.md streak extends to four.
- [x] Production-core `lib.rs` streak extends to four.
- [x] Zero new workspace dependencies.
- [x] Test count delta positive — `+17` (above the
  predicted `+8`–`+12` band; over-shoot called out
  honestly in the prediction-vs-reality section).
- [x] Zero clippy warnings.
