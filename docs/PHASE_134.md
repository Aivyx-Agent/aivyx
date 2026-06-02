# Phase 134 — Direction B: Embedded Rust-Native Inference (`mistral.rs`)

**Largest architectural phase since Phase 121** (which
gave Ollama a native `/api/chat` adapter and ended
the "everything routes through OpenAI-compat" era).
Phase 134 ships **embedded inference** — Aivyx can now
run a local LLM **inside its own process**, with no
separate runtime server, by linking against
`mistralrs` as a Rust dependency.

## Why this, why now

- **Phase 133 set up the question.** Aivyx now ships
  three first-class out-of-process providers (Ollama,
  llama-server, Jan). The natural next question:
  "Could Aivyx ship one batteries-included Rust-native
  engine and collapse the install story to a single
  binary?" Phase 133 flagged this as Direction B;
  Phase 134 ships it.

- **Aivyx is a Rust workspace already.** Embedding a
  Rust LLM engine has zero cross-language seam to
  manage. `mistralrs` exposes a clean
  `GgufModelBuilder` / `ModelBuilder` /
  `stream_chat_request` API that maps near-identically
  onto Aivyx's existing `LlmProvider` trait surface.

- **Privacy posture, end state.** The Phase 133
  INSTALL.md section documented Ollama's unclear
  telemetry and the January 2026 175K-exposed-hosts
  incident. An embedded engine has **zero outbound
  network calls during inference** — the operator
  loads a local GGUF file and the model runs in the
  Aivyx process. This is the strongest possible
  privacy posture short of air-gapping the machine.

- **One-binary install for new users.** The
  `recommended-providers` meta-feature (Q1c) bundles
  the embedded provider into a single
  `cargo install --features recommended-providers
  aivyx` invocation. New users who don't already have
  Ollama installed get a working local-agent
  experience in one command.

## Q-block sign-off (1 Recommended + 2 non-Recommended)

- **Q1c — Opt-in but bundled in a `recommended-providers`
  meta-feature** (non-Recommended; operator-picked
  over Q1a's pure opt-in or Q1b's default-on).

  Rationale operators may want to revisit later:
  the meta-feature is a UX win for new users but
  means the recommended install pulls in a heavy
  dependency (compile time + binary size
  measurably grow). Operators who care about lean
  builds use `--no-default-features --features
  provider-ollama` explicitly. Aivyx's release CI
  ships two artifacts: slim (Ollama-only) and
  batteries-included (everything).

- **Q2c — CPU + Metal + CUDA in one phase** (non-
  Recommended; operator-picked over Q2a's
  CPU-only or Q2b's CPU+Metal).

  Three backends in one phase triples the test
  matrix and the documentation surface. The
  honest framing: this is a big phase. CUDA
  requires the CUDA toolkit on the build machine
  (operators on plain Linux laptops can't build
  the CUDA variant locally); Metal requires
  macOS. Each backend gates behind its own
  Cargo feature (`provider-mistral-rs-cuda`,
  `provider-mistral-rs-metal`,
  `provider-mistral-rs-accelerate`). CI ships
  per-OS artifacts. The Q2a CPU-only escape
  hatch is preserved via `--features
  provider-mistral-rs` without any backend
  feature.

- **Q3a — Document recommended GGUF models;
  don't bundle** (Recommended).

  INSTALL.md points operators at concrete
  HuggingFace downloads (Qwen3-4B,
  Llama-3.2-3B-Instruct, Phi-4-mini, SmolLM2)
  with per-model size + RAM + use-case guidance.
  Aivyx never silently downloads a model; the
  operator chooses, downloads via `wget` / `curl`
  / HF CLI, and supplies the path in config.
  Strongest privacy posture; smallest release
  artifact; clearest operator consent.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment. The embedded provider plugs into the
  existing `LlmProvider` trait seam unchanged; D1
  (turn loop) doesn't move; D4 (capability
  taxonomy) doesn't move. Current hash:
  `62dabbdd…`. Prediction: streak **extends to 25**.

- **PRODUCT.md** — **Will hold.** Direction B is
  the natural extension of "AI personal assistant,
  privacy-first, local-LLM capable" framing. No
  product principle moves. Current hash:
  `467ba59a…`. Prediction: streak **extends to 25**.

- **`aivyx-core/src/lib.rs`** — **Will hold.** All
  Phase 134 work lives in `aivyx-llm`
  (`MistralRsProvider` impl) plus `aivyx-config`
  (new `ProviderKind` variant + config section)
  plus `aivyx-channel/src/bin/aivyx.rs` (dispatch).
  Core untouched. Current hash: `4f9b8c81…`.
  Prediction: streak **extends from 7 to 8**.

## Tasks

1. **Open doc + ROADMAP + README** — this doc + the
   roadmap section + the README row. Backfill
   Phase 133 hash to `0b8811c`.

2. **Workspace deps + feature flags.** Add
   `mistralrs = "0.8"` as **optional** dep in
   `aivyx-llm`. New features:
   - `provider-mistral-rs` (CPU baseline).
   - `provider-mistral-rs-cuda` (forwards to
     mistralrs's `cuda` feature).
   - `provider-mistral-rs-metal` (forwards to
     mistralrs's `metal` feature).
   - `provider-mistral-rs-accelerate` (forwards to
     mistralrs's `accelerate` feature).
   - Workspace-level `recommended-providers`
     meta-feature that enables provider-ollama +
     provider-mistral-rs (CPU).

3. **`MistralRsProvider` impl.** New module
   `aivyx-llm/src/mistral_rs/`. Bridges Aivyx's
   `LlmMessage` → `TextMessages`; streams via
   `model.stream_chat_request(...)`; surfaces
   tool calling via `RequestBuilder::set_tools`
   and the `tool_calls` field on the response.
   Returns Aivyx's `LlmStreamEvent` shape so the
   planner is unaware which provider it's
   talking to.

4. **`ProviderKind::MistralRs` + binary
   dispatch.** `aivyx-config` gets a new
   variant (`default_context_window` 8000 same
   as other local providers; `is_openai_compatible`
   returns `false` because the wire protocol is
   in-process, not HTTP). New `[mistralrs]` config
   section: `model_path` (required absolute path
   to a GGUF directory or file), `isq_bits`
   (optional, default `Eight`), `paged_attention`
   (bool, default true on supported platforms).
   `aivyx-channel/src/bin/aivyx.rs` gains a
   dispatch arm constructing
   `MistralRsProvider::from_config(...)`.

5. **Provider regression tests** —
   `ProviderKind::MistralRs` parse + validate +
   display + context window. Where feasible,
   gated tests for each backend feature.

6. **INSTALL.md** — "Embedded Rust-native
   inference (Phase 134)" subsection joins the
   Local LLM providers section. Recommended
   GGUF models matrix; backend selection guide
   (CPU / Metal / CUDA tradeoffs); honest build
   complexity + binary size warning; one-line
   install commands.

7. **Exit doc + prediction-vs-reality** —
   populate prediction-vs-reality, flip Frozen,
   surface Phase 135+ candidates (Channel
   Activation Milestone gets the strongest
   bump yet).

## Exit criteria

- [ ] `docs/PHASE_134.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `mistralrs` optional dep + four feature
  flags + `recommended-providers` meta-feature —
  Task 2.
- [ ] `MistralRsProvider` with `LlmProvider`
  impl — Task 3.
- [ ] `ProviderKind::MistralRs` end-to-end
  through config + binary dispatch — Task 4.
- [ ] Tests covering parse, validate, display,
  defaults — Task 5.
- [ ] INSTALL.md embedded provider docs —
  Task 6.
- [ ] DESIGN.md / PRODUCT.md / lib.rs all HOLD
  as predicted.
- [ ] **One new workspace dependency** —
  `mistralrs` itself (substrate-tier, behind
  opt-in feature gate). All other transitive
  deps flow through it.
- [ ] Zero clippy warnings (with the new
  features enabled in the workspace's clippy
  run).
- [ ] Test count delta: `+15` to `+35`.
- [ ] **Honest measurements:** compile time
  and release binary size deltas, with and
  without the embedded feature.

## Honest scope risks at sign-off

- **Compile time will balloon.** `mistralrs` is
  a substantial crate that pulls in `candle`
  plus tokenizers plus `hf-hub`. Initial build
  with `--features provider-mistral-rs` likely
  takes 5-10 minutes. Operators on
  `--features provider-ollama` see no change.
  Documented honestly in the exit doc.

- **Release binary size will measurably grow.**
  Pure-Rust CPU build estimated at 100-200MB
  added on top of the existing Aivyx binary.
  CUDA variant adds the cuBLAS / cuDNN
  runtime weight. Operators who care pick the
  slim build. Documented honestly.

- **The CUDA backend has build-machine
  prerequisites.** Operators trying to build
  the CUDA variant need the CUDA toolkit
  installed; the Metal variant requires
  macOS. Documented in the per-backend INSTALL
  section.

- **mistralrs is on version 0.8 — pre-1.0
  semver.** API may churn between minor
  versions. We pin `mistralrs = "=0.8.*"`
  and document the upgrade-by-Aivyx-version
  contract.

- **The textual tool-call extractor (Phase 127)
  is provider-specific.** Models with
  non-standard tool-call formats (qwen3.x's
  XML wrappers, phi4-mini's `<tool_call>`
  blocks) routed through `mistralrs`'s
  built-in extraction may behave differently
  than the same model under Ollama. Per-model
  empirical validation in Phase 135+ if
  operators surface mismatches.

- **`ollama.list/show/pull` agent tools have
  no mistralrs equivalent.** Operators on the
  embedded provider don't get autonomous model
  download from the agent. Same posture as
  Phase 133's documented gap for llama-server
  and Jan.

- **Twenty-third consecutive deferral of the
  Channel Activation Milestone** if Phase 134
  ships without taking it. The deferral count
  keeps growing; the signal-strength is
  approaching "we should genuinely consider
  this next."

## Direction after Phase 134

After Phase 134, the candidates for Phase 135:

1. **Channel Activation Milestone.** 23rd
   consecutive deferral. With the local-LLM
   story now ranging from "use Ollama" to
   "Aivyx is a self-contained binary," the
   milestone work is the standout missing
   piece.
2. **Empirical signal collection.** Three
   out-of-process providers + one embedded
   provider. Operators using each in real
   workflows will surface preference data; a
   Phase 135 could codify "Aivyx's official
   recommended local-LLM story" based on the
   signal.
3. **Provider-specific model family probes**
   (Phase 133 follow-on, now lower priority
   given embedded eliminates the need for
   external `/api/show`).
4. **`provider-localai`** — the third
   out-of-process provider in the multi-backend
   lineup.

## Prediction vs reality

**Three-of-three streak predictions correct.**

- **DESIGN.md** — HELD as predicted (`62dabbdd…`
  unchanged). The embedded provider plugged into the
  existing `LlmProvider` trait seam unchanged; D1
  (turn loop) didn't move; D4 (capability
  taxonomy) didn't move. Streak: 24 → **25**.
- **PRODUCT.md** — HELD as predicted (`467ba59a…`
  unchanged). Direction B is the natural extension
  of "privacy-first, local-LLM capable." Streak:
  24 → **25**.
- **`aivyx-core/src/lib.rs`** — HELD as predicted
  (`4f9b8c81…` unchanged). All Phase 134 work in
  `aivyx-llm/mistral_rs/`, `aivyx-config`, and the
  binary dispatch. Core untouched. Streak:
  7 → **8**.

**Test count delta: +16 — within predicted `+15` to
`+35` range.** Workspace lib tests 2965 → 2981. Per-
crate: aivyx-llm +8 (5 conversion-layer tests, 3
stream-emission tests); aivyx-config +8 (env+TOML
parse for the three aliases, validate model_path
required, full section round-trip, is_in_process /
is_openai_compatible distinction, default context
window, Display).

**One new workspace dependency** as predicted:
`mistralrs = "=0.8.*"`, optional, gated behind
`provider-mistral-rs`. All other transitive deps
(candle, hf-hub, tokenizers, aws-lc-rs, …) flow
through it.

**Zero clippy warnings** with default features and
with `--features aivyx-channel/provider-mistral-rs`.
`--all-features` fails in the upstream `objc2` crate
under the Metal backend feature — platform-gated by
design, expected, not regression.

### What landed cleanly + what bent

**Cleanly:** feature flag scaffolding through both
aivyx-llm and aivyx-channel;
`recommended-providers` meta-feature;
`ProviderKind::MistralRs` end-to-end through config +
binary dispatch + tests; conversion-layer unit
tests; build verification on cached deps
(~30s check, ~60s full).

**Bent honestly:**
- **Non-streaming MVP.** mistralrs 0.8.1's
  `Stream<'a>` borrows from the Model with a
  lifetime, which doesn't satisfy Aivyx's
  `LlmStream` contract (`Send + 'static`) without
  a new dep (`ouroboros`) or architectural work
  (mpsc-forwarding spawned task). Phase 134 ships
  `send_chat_request` (full response in one shot)
  and surfaces it as one TextChunk + one StepEnd.
  Streaming-text-deltas is Phase 135 work.
- **API-shape mismatches between mistralrs's
  master-branch examples and 0.8.1.** Three
  differences caught at compile time:
  `Function` has no `strict` field;
  `ToolCallResponse` requires an `index` field;
  `CalledFunction`'s name/arguments are direct
  `String` (not Option). All fixed in convert.rs.
  Honest signal: pre-1.0 mistralrs APIs do churn;
  the `=0.8.*` pin is load-bearing.
- **`--all-features` workspace clippy fails in
  upstream objc2.** Platform-gated by design.

### TLS-stack collision flagged honestly

mistralrs's transitive deps pull in `aws-lc-rs`
alongside the workspace's `rustls`. The workspace
had a documented rustls-only policy (see the
jsonschema feature-disable comment in root
Cargo.toml). Phase 134's embedded provider relaxes
that for opt-in builds: the slim Ollama-only build
keeps rustls-only; the recommended-providers build
carries both stacks. Operators who care about lean
TLS posture use `cargo install aivyx-channel`
(no features) unchanged.

### Direction after Phase 134

Candidates for Phase 135:

1. **Channel Activation Milestone.** 23rd
   consecutive deferral. Four local-LLM providers
   shipped + Direction B done → the milestone work
   is the standout missing piece. Strongest
   signal yet.
2. **Streaming text deltas for the embedded
   provider.** Build the mpsc-forwarding spawned-
   task shim that closes the `Stream<'a>` →
   `LlmStream + Send + 'static` gap. Focused
   substrate; small scope; high operator UX
   payoff.
3. **End-to-end hardware validation.** Operator
   coordination across CPU/Metal/CUDA. Codify
   reported empirical signal — which models +
   backends actually deliver the agentic-quality
   bar Aivyx targets.
4. **Multimodal bridging.** Wire
   `ContentBlock::ImageBase64` through to
   mistralrs's native image content blocks.

**Twenty-third consecutive deferral of the Channel
Activation Milestone.** The signal-strength has
crossed the "genuinely should consider next"
threshold.

