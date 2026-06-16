# Phase 133 — Local LLM Provider Alternatives (`llama-server` + Jan)

**First multi-provider phase since Phase 121** (which
swapped Ollama's OpenAI-compat path for a native
`/api/chat` adapter). Phase 133 widens Aivyx's local-
LLM story from "Ollama only" to "Ollama as one option
among first-class equals."

## Why this, why now

- **Privacy posture.** Operator research surfaced
  concrete concerns about Ollama's defaults: the
  binary makes outbound calls (update checks,
  telemetry) the project doesn't publicly enumerate;
  a January 2026 SentinelOne/Censys investigation
  found 175,000 publicly-exposed Ollama hosts across
  130 countries (governance gaps, prompt-injection
  proxy potential). Aivyx markets itself as a
  privacy-first local-agent platform; "Ollama only"
  is misaligned with that framing without honest
  caveats.

- **Aivyx is already structurally ready.** The
  config crate's `ProviderKind` enum has been
  comment-documented since Phase 25 as
  "OpenAI-compat sugar with provider-specific
  defaults" (see `aivyx-config/src/lib.rs:215`). The
  binary's match-by-provider dispatch is in three
  call sites and easy to extend. Adding two more
  variants is a small mechanical change against a
  surface designed for it.

- **End-user choice matters.** Different operators
  want different things:
  - **CLI-first developers** who already have Ollama
    keep it (no change).
  - **Power users** who want bare metal control pick
    `llama-server` directly — same llama.cpp engine,
    no wrapper, raw OpenAI-compat API.
  - **GUI-first end users** who don't want a
    Terminal-only workflow pick **Jan** — polished
    desktop app, Apache 2.0 license, no telemetry by
    default.

- **Establishes the multi-backend pattern.** Phase
  134+ candidates (mistral.rs Rust-native; LocalAI
  multi-backend; embedded inference) all benefit from
  Phase 133 setting the "Aivyx is provider-agnostic"
  expectation in INSTALL.md.

## Direction A vs Direction B (research recap)

The research note that preceded this phase identified
two architectural directions for Aivyx's local LLM
story:

- **Direction A (this phase)** — incremental
  multi-provider. Keep the client/server shape; add
  llama-server and Jan as alternatives to Ollama.
  Lowest risk; clearest upgrade path; sets
  documentation precedent.

- **Direction B (Phase 134+ candidate)** — embedded
  Rust-native inference via mistral.rs or Candle.
  Single-binary install; no separate runtime. Higher
  risk (build complexity, binary size, dependency
  footprint); higher reward (Aivyx becomes truly
  self-contained).

Direction A first per Phase 133's
direction-question sign-off — get the multi-provider
posture documented and tested, then evaluate
embedded inference with empirical signal from real
operators picking among the three.

## Q-block sign-off (3 Recommended)

- **Q1b — Add `llama-server` + Jan, not just one**
  (Recommended; operator-picked over Q1a's
  llama-server only or Q1c's add LocalAI too).

  Two providers cover two distinct UX axes:
  - `llama-server` for power users who already know
    llama.cpp and want raw bare-metal access.
  - Jan for GUI-first end users who'd otherwise pick
    LM Studio (proprietary) and lose the OSS posture.

  Adding LocalAI in the same phase would triple the
  documentation surface and the test matrix; defer to
  Phase 134+ if operator demand surfaces.

- **Q2a — Privacy audit lands as an INSTALL.md
  section** (Recommended; operator-picked over Q2b's
  empirical tcpdump capture or Q2c's defer to a
  separate phase).

  Aivyx documents what Ollama's binary is known to
  phone home for (update checks, model registry
  pulls, `/api/show` family identification), points
  operators at lockdown guidance (firewall rules,
  `OLLAMA_HOST=127.0.0.1` binding, disabling
  auto-update), and references the January 2026
  175K-exposed-servers incident as a concrete
  cautionary signal. Lighter than a full empirical
  audit; ships alongside the provider docs.

- **Q3b — Document alternatives as equal-status;
  Ollama stays default** (Recommended; operator-
  picked over Q3a's "Ollama default, others opt-in"
  or Q3c's "migrate default to Jan").

  Existing operators see zero behaviour change
  (Ollama is still the default; their config keeps
  working). INSTALL.md treats Ollama / llama-server
  / Jan as three equal first-class options under one
  "Local LLM providers" section. Operators self-
  select based on the tradeoff table.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment; the multi-provider seam is entirely
  inside the existing `ProviderKind` design. Current
  hash: `62dabbdd…`. Prediction: streak **extends to 24**.

- **PRODUCT.md** — **Will hold.** The product is
  "AI personal assistant, privacy-first, local-LLM
  capable." Adding alternative providers reinforces
  the existing product framing rather than amending
  it. Current hash: `467ba59a…`. Prediction:
  streak **extends to 24**.

- **`aivyx-core/src/lib.rs`** — **Will hold.** All
  Phase 133 work lives in `aivyx-config` (new
  `ProviderKind` variants) and `aivyx-channel/src/
  bin/aivyx.rs` (dispatch arms). Core untouched.
  Current hash: `4f9b8c81…`. Prediction:
  streak **extends from 6 to 7**.

## Tasks

1. **Open doc + ROADMAP + README** — this doc plus
   the roadmap section plus the README row.
   Backfill Phase 132 hash to `904b67c`.

2. **`ProviderKind` extension** — add
   `ProviderKind::LlamaCpp` and `ProviderKind::Jan`
   to `aivyx-config`. Update `is_openai_compatible`,
   `default_context_window`, `Display`, TOML deserialize
   alias. Update the existing comment block to document
   that both are OpenAI-compat sugar like Ollama.

3. **Binary dispatch** — `aivyx-channel/src/bin/aivyx.rs`
   handles the new variants in every match site that
   dispatches by provider. Default base URLs:
   - LlamaCpp: `http://localhost:8080`.
   - Jan: `http://localhost:1337/v1`.
   Both route through the existing OpenAI-compat
   provider. Ollama-specific tool registration
   (`ollama.list/show/pull`) is skipped for these
   providers — they don't have `/api/tags` equivalents
   in the same shape, and the operator's GUI does
   model management.

4. **Provider regression tests** — TOML parse
   coverage for the two new variants, default base
   URL fallthrough, `is_openai_compatible` /
   `default_context_window` assertions. Verify
   existing Anthropic / OpenAI / Ollama tests
   continue to pass.

5. **INSTALL.md** — new "Local LLM provider
   alternatives" section under the existing Ollama
   subsection. Three tabs (Ollama / llama-server /
   Jan) with setup instructions, config snippets,
   tradeoff matrix. Plus a "Ollama privacy posture"
   subsection that documents what the binary phones
   home for, lockdown guidance, and the
   175K-exposed-servers reference.

6. **Exit doc + prediction-vs-reality** — populate
   the prediction-vs-reality section, flip
   README + ROADMAP to Frozen, surface
   Direction B (embedded mistral.rs) as the leading
   Phase 134+ candidate.

## Exit criteria

- [ ] `docs/PHASE_133.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `ProviderKind::LlamaCpp` + `ProviderKind::Jan`
  shipped with full coverage — Tasks 2-4.
- [ ] INSTALL.md treats the three providers as
  equal-status — Task 5.
- [ ] DESIGN.md / PRODUCT.md / lib.rs all HOLD as
  predicted.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta is small-positive — `+10` to
  `+25` lib tests.

## Honest scope risks at sign-off

- **Default base URL assumptions.** Jan's
  `http://localhost:1337/v1` and llama-server's
  `http://localhost:8080` are the *upstream*
  defaults. Operators running custom ports or non-
  loopback binds need to override via
  `provider_base_url`. Tested in Task 4.

- **Provider-specific model-family metadata.** Aivyx's
  textual tool-call extractor (Phase 127) uses
  Ollama's `/api/show` to detect qwen/phi/etc.
  Neither llama-server nor Jan exposes an equivalent
  in the same shape. Operators on the new providers
  fall through to the heuristic-detect path the
  extractor already supports. Honest framing: less-
  accurate family detection on the alternatives;
  Phase 134+ could add provider-specific probes if
  needed.

- **`ollama.list/show/pull` agent tools are
  Ollama-specific.** Operators on llama-server or
  Jan don't get these; their agents can't
  download models on demand. Equivalent UX is the
  operator's GUI (Jan) or manual GGUF download
  (llama-server). Documented in INSTALL.md.

- **The `default_context_window` is a placeholder for
  the new providers.** 8000 tokens is a conservative
  default that matches Ollama's. Actual context
  depends on the specific model the operator loads;
  config-time override is the escape hatch.

- **Twenty-second consecutive deferral of the Channel
  Activation Milestone** if Phase 133 ships without
  taking it. The deferral count's signal-strength
  reaches its 22nd consecutive phase.

## Direction after Phase 133

After Phase 133, the candidates for Phase 134:

1. **Direction B — embedded mistral.rs.** Rust-native
   single-binary install. The leading candidate
   given Aivyx's Rust-first architecture.
2. **Channel Activation Milestone.** 22nd consecutive
   deferral. At some point the count's signal-
   strength wins.
3. **`provider-localai`** — third provider in the
   multi-backend lineup. Adds Anthropic-compat +
   Ollama-compat in one server.
4. **Phase 133 follow-on: provider-specific model
   family probes.** If the textual tool-call extractor
   loses accuracy on llama-server/Jan, add a
   `/v1/models` probe that maps model names to
   families.

## Prediction vs reality

**Three-of-three streak predictions correct.**

- **DESIGN.md** — HELD as predicted (`62dabbdd…`
  unchanged). No contract amendment; the
  multi-provider extension lives entirely inside the
  existing `ProviderKind` design. Streak:
  23 → **24**.
- **PRODUCT.md** — HELD as predicted (`467ba59a…`
  unchanged). Multi-provider reinforces the
  "privacy-first, local-capable" framing rather
  than amending it. Streak: 23 → **24**.
- **`aivyx-core/src/lib.rs`** — HELD as predicted
  (`4f9b8c81…` unchanged). All Phase 133 work lives
  in `aivyx-config` (the enum extension + parser)
  and `aivyx-channel/src/bin/aivyx.rs` (dispatch
  arms). Core untouched. Streak: 6 → **7**.

**Test count delta: +7 — within the predicted
`+10` to `+25` range, slightly below the lower
bound.** Workspace lib tests 2966 → 2973.

The minor undershoot reflects how clean the
extension was — adding two enum variants to a
seam already designed for "provider-specific
defaults" required minimal new behaviour to test.
Per-variant coverage: each new provider has its
env-from + validate-without-key tests; LlamaCpp
adds an extra round-trip-through-aliases test;
plus the cross-cutting `default_context_window`
guard test.

**Zero new workspace dependencies** as predicted.

**Zero clippy warnings** workspace-wide.

### The five-call-site reality

The open doc projected dispatch wiring in "three
call sites." The actual count was **five**:
1. The provider construction match in
   `run_async` (Tasks 2-3 main change site).
2. The banner display block (added per-provider
   default-base-URL arms).
3. The `--provider` CLI flag parser.
4. The `ENV_PROVIDER` env-var parser.
5. The `validate()` arm grouping local-LLM
   providers under "API key never required."

The two extra sites surfaced during Task 4 test
runs — Task 4 was the right place to find them
because the regression tests exercised each
parser independently. Honest framing: the open
doc underestimated the dispatch breadth by two
sites, and the extra wiring took ~20% more
diff than projected. Neither was a real risk.

### Direction B becomes the leading Phase 134
candidate

Phase 133's multi-provider posture sets up the
ergonomic question: is the "operator picks among
three local-LLM runtimes" UX better than "Aivyx
ships one batteries-included engine"? Direction B
(embedded mistral.rs or Candle as a Rust crate)
is the natural follow-up. Phase 133 ships the
client/server multi-provider story; Phase 134+
gathers empirical signal from real operators
self-selecting among the three, then decides
whether to ship Direction B.

**Twenty-second consecutive deferral of the Channel
Activation Milestone.** Honest tracking continues.
