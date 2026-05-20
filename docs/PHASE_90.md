# Phase 90 — Heuristic Recall Gate (the third move in the input-quality arc)

For 89 phases auto-recall (Phase 76) and adaptive Persona
selection (Phase 79) have fired on **every conversational
turn**, including turns where the user message is a one- or
two-token acknowledgment (`ok` / `thanks` / `yes` / `cool`)
that cannot meaningfully steer recall or facet selection.
The bare-message embed on those turns is essentially a random
vector that pollutes the ranker — and the recall block /
adaptive Persona block injected on top of that ranker is
noise the planner has to defend against.

Phase 90 closes the longest-standing recall-side deferral
(Phase 76, carried forward 14 phases through 77-89) with the
smallest possible gate: a **length-based heuristic** at the
top of both relevance hooks that skips the embed (and
everything downstream) on turns whose trimmed user message is
shorter than `recall_gate_min_chars`. Both consumers (Phase
76 auto-recall + Phase 79 adaptive Persona) share the same
gate and the same opt-in knob, exactly as Phase 86's window
work shipped both consumers under one switch.

This is the third move in the input-quality arc:

|                                                | Layer that's sharpened |
|------------------------------------------------|------------------------|
| **Phase 86** — Conversational window           | *What* gets embedded   |
| **Phase 89** — Topic canonicalization          | *How* signals key      |
| **Phase 90** — Heuristic recall gate (this)    | *When* recall fires    |

After Phase 90, the recall pipeline runs only when there is
plausibly something to recall *for*.

## Why this, why now

- It is the longest-running recall-side deferral (Phase 76,
  14 phases old). Every subsequent recall consumer (Phase 77
  feedback, Phase 82/83 ledgers, Phase 84 cluster expansion,
  Phase 86 windowed embed, Phase 89 canonical keys) paid the
  noise-turn cost silently.
- The fix is contained: one new pure helper, one config
  knob, two short-circuit guards (one per consumer). Zero
  new workspace deps; zero `aivyx-core/src/lib.rs` touch.
- It is **strictly cost-saving + signal-cleaning** when
  engaged: a gated turn skips the embed call *and* skips
  the memory walk *and* skips the facet ranking. No new
  behaviour is introduced; existing behaviour is just not
  fired in cases where it could not help.
- Symmetric design with Phase 86: same shared knob drives
  both consumers; both consumers short-circuit the same way
  (return `None`, which is exactly the existing
  "best-effort fallback" code path both providers already
  honour).

## Streak predictions

- **DESIGN.md** — **Will hold.** A length-based short-circuit
  at the top of an existing best-effort hook touches no
  locked technical-contract decision. The hooks have always
  been allowed to return `None` (the planner's `recall: Option`
  contract); Phase 90 just adds one more reason to return it.
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Prediction: streak **extends to thirty-seven** (currently
  36).

- **PRODUCT.md** — **Will hold.** No new product commitment,
  none weakened; the operator-facing contract is
  *strengthened* (G3 recall + P14 Persona no longer waste
  embed cost or pollute their rankers on noise turns).
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.
  Prediction: streak **extends to thirty** (currently 29).

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold,
  by design.** The gate function lives in `aivyx-channel`
  (alongside the existing `conversation_window` module —
  same provider-side substrate layer); the config knob is a
  new field on the existing `EmbeddingConfig` in
  `aivyx-config`; the two short-circuits are in
  `memory_recall.rs` and `persona_context.rs`. No
  `aivyx-core` touch.
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.
  Prediction: streak **extends to thirty-eight** consecutive
  phases (new project record, beats Phase 89's 37).

- **New workspace deps** — Zero. A length check is `<= a
  dozen lines` of pure Rust.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_90.md` + `docs/README.md` status row.

### Task 2 — `[embedding].recall_gate_min_chars` knob

`aivyx-config`:

- `EmbeddingConfig` gains
  `recall_gate_min_chars: usize`, default `0` (gate
  disabled = byte-identical to pre-Phase-90). Sourced via
  the existing `[embedding]` block's TOML deserializer.
  Validation when `[embedding]` is configured: any value is
  legal (zero = off; large values gate aggressively — the
  operator's call). The existing `DEFAULT_RECALL_WINDOW_TURNS
  = 1` pattern is the precedent.
- Tests: default (`0`); explicit (`6`); explicit zero
  honoured.

### Task 3 — `should_gate_recall` (the crux)

`aivyx-channel`:

- New `pub fn should_gate_recall(text: &str, min_chars:
  usize) -> bool` in a small `recall_gate` module (or as a
  pub fn on the existing `memory_recall` module — module
  choice in the implementation diff). The rule:
  - `min_chars == 0` → never gate (the opt-out / pre-
    Phase-90 default).
  - Otherwise, trim the input; if the trimmed character
    count (Unicode chars, not bytes — `text.trim().chars().
    count()`) is **strictly less than** `min_chars`, gate.
- Unit tests: opt-out (`min_chars = 0`); exact-threshold
  boundary (`min_chars = 4`, input `"ok"` gated, input
  `"hello"` ungated); whitespace-only input gates; empty
  input gates; Unicode characters counted correctly
  (`"héllo"` is 5 chars, not the 6 bytes of UTF-8); the
  trim is whitespace-only (`"  ok  "` gates same as
  `"ok"`).

### Task 4 — Provider integration

`aivyx-channel`:

- `SemanticMemoryContext` gains a `recall_gate_min_chars:
  usize` field (default `0`); the existing
  `with_conversation_windows` builder is the precedent for
  threading new opt-in knobs through. Add a parallel
  `with_recall_gate(min_chars)` builder.
- `SemanticMemoryContext::recall` short-circuits to `None`
  at the top of the method when
  `should_gate_recall(user_message, self.recall_gate_min_chars)`
  is true. Returns before the embed call, before the memory
  walk, before any ledger / cluster-expansion logic.
- `PersonaContextRefiner` gains the same field + builder;
  `refine` short-circuits to `None` at the same point. The
  planner uses the full Persona base prompt on gated turns —
  exactly the existing "Soul too small" / "embed failure"
  fallback path.
- `bin/aivyx` reads `config.embedding.recall_gate_min_chars`
  and threads it to both providers via the new builders.
- Integration tests: a short user message (`"ok"`) with
  `recall_gate_min_chars = 4` produces no recall block AND
  no adaptive Persona selection (both providers return
  `None`); a longer message (`"how do I deploy"`) is
  unaffected; `recall_gate_min_chars = 0` (default) is
  byte-identical to pre-Phase-90 on both providers.

### Task 5 — Docs + exit

- `docs/INSTALL.md` — "Heuristic recall gate (Phase 90)"
  subsection under the Phase 76 auto-recall section: what a
  noise turn is, the operator-visible cost saving, the
  opt-in knob, the byte-identical default, the symmetric
  application to both consumers.
- `examples/aivyx.toml` — document the new
  `recall_gate_min_chars` knob alongside the existing
  `[embedding]` knobs.
- Exit: ROADMAP + PRODUCT_ROADMAP frozen entries,
  docs/README status flip, prediction-vs-reality, hash
  backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Gate signal:** (a) Length-based. Trimmed-char
  count below `recall_gate_min_chars` → gate. Simple,
  deterministic, language-agnostic; covers the dominant
  noise-turn shape (single-token acknowledgments) without
  brittle stoplists or wasted embed calls.
- **Q2 — Default posture:** (a) Opt-in
  `[embedding].recall_gate_min_chars`, default `0` (gate
  disabled). Matches the project's 89-phase
  behaviour-change-is-opt-in discipline; symmetric with the
  Phase 86 `recall_window_turns` default-`1` pattern.
- **Q3 — Surface scope:** (a) Both consumers — auto-recall
  AND adaptive Persona — under the same shared knob.
  Symmetric with the Phase 86 design (one knob, both
  consumers); the two providers ask the same "is this turn
  worth ranking" question, so splitting them is over-
  engineered.
- **Q4 — Gate behavior:** (a) Skip the embed entirely;
  both providers return `None`. Lowest cost (no embed
  call); cleanest semantics; uses the existing best-effort
  fallback contract both providers already honour.

## Deferrals

**Rolling deferrals carried into Phase 90:**

- v0.1.0 publication (Phase 61 Task 7).
- System-prompt notification-target enumeration (Phase 62).
- `turn_id` correlation refactor (Phase 67).
- Phase 68 deferrals (XOAUTH2, HTML email, attachments,
  multi-account `[email]`).
- Phase 69 deferrals (WebPush / service-worker, notification
  urgency, icons, sound).
- Phase 70 deferrals (proposal supersession, multi-window
  reflection).
- Phase 71 deferrals (per-fire role override, operator-
  customizable reflection prompt, reflection cadence
  learning).
- Phase 73 deferrals (persisted rate-limit buckets,
  operator-configurable retry-on list).
- Phase 74 deferrals (fuzzy match, edit-content Web UI,
  per-topic eviction-strategy override).
- Phase 75 deferrals (ANN index, `aivyx memory reembed`,
  hybrid keyword+semantic fusion, query-embedding cache).
- Phase 76 deferrals (**heuristic recall gate — THIS PHASE**,
  token-budget context sizing).
- Phase 77 deferrals (`[recall_feedback]` tuning knob,
  LLM-judged recall usefulness).
- Phase 78 deferrals (per-memory-entry drill-down, Web UI
  live refresh, actionable insights).
- Phase 79 deferrals (`[persona]` tuning block, behavioural
  Persona).
- Phase 80 deferrals (standalone `[[proactive_schedule]]`,
  LLM-composed proactive prose, conversational/interactive
  proactive, additional proactive signal classes).
- Phase 81 deferrals (contradiction-based supersession,
  standalone `[[persona_lifecycle_schedule]]`, facet-scoped
  one-click revert).
- Phase 82 deferrals (operator-tunable half-life/retention).
- Phase 83 deferrals (sequential/temporal patterns, n-ary
  clusters, operator-tunable top-K/half-life).
- Phase 84 deferrals (affinity re-ranking of existing
  candidates, operator-tunable affinity policy).
- Phase 85 deferrals (reflection-facet decay via fuzzy
  embedding).
- Phase 86 deferrals (token-budget context sizing,
  embed-each-and-pool windows, persisted windows).
- Phase 87 deferrals (n-ary cluster proposals, pattern-
  driven supersession, operator-tunable LLM prompt).
- Phase 88 deferrals (pattern-driven supersession, n-ary
  cluster decay, pair-affinity hysteresis).
- Phase 89 deferrals (operator-tunable `[[topic_alias]]`
  mappings, topic-by-topic exception list, one-time
  migration of existing fragmented data, non-ASCII /
  Unicode stemming).

**Likely Phase 90 deferrals:**

- **Pattern-based stoplist.** A configurable
  `[[recall_gate.noise_token]]` list of exact phrases that
  should also gate (`"got it"`, `"sounds good"`). Defers
  pending operator feedback; the length heuristic catches
  the dominant cases.
- **LLM-judged gate.** An LLM judging whether the turn
  warrants a recall, replacing the length heuristic.
  Defers; introduces LLM-on-recall-path cost; the cheap
  length check covers the bulk of the value.
- **Adaptive threshold.** The gate threshold tuned from the
  operator's own message-length distribution. Defers as a
  Phase 91+ self-learning extension.
- **Token-budget context sizing.** Still deferred from
  Phase 76/86. A tokenizer-aware recall context cap is a
  different problem (output sizing, not input gating).

## Prediction vs. reality

To be filled in at phase exit.

## Exit criteria

- [ ] `[embedding].recall_gate_min_chars: usize` (default
  `0` = disabled) — Task 2.
- [ ] `should_gate_recall(text, min_chars) -> bool` pure
  function in `aivyx-channel`; trimmed-char count rule
  with the `min_chars == 0` opt-out short-circuit;
  Unicode-char-counted (not byte-counted) — Task 3.
- [ ] Unit tests on the gate function: opt-out boundary,
  exact-threshold boundary, whitespace-only / empty
  inputs, Unicode-char counting, trim-then-measure
  semantics — Task 3.
- [ ] `SemanticMemoryContext::recall` and
  `PersonaContextRefiner::refine` short-circuit to `None`
  before any embed call when the gate fires; both gain
  the `with_recall_gate(min_chars)` builder — Task 4.
- [ ] `bin/aivyx` threads
  `config.embedding.recall_gate_min_chars` to both
  providers — Task 4.
- [ ] Integration tests: gated turn (short input)
  produces `None` from both providers; ungated turn
  (longer input) is unaffected; `min_chars = 0` is
  byte-identical to pre-Phase-90 — Task 4.
- [ ] `docs/INSTALL.md` + `examples/aivyx.toml` updated —
  Task 5.
- [ ] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 5.
- [ ] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [ ] DESIGN.md streak extends to thirty-seven.
- [ ] PRODUCT.md streak extends to thirty.
- [ ] Production-core streak extends to thirty-eight (new
  record) — `lib.rs` byte-identical.
- [ ] Test count delta: positive (~+6-10; per the
  converged calibration law — knob on an existing block
  (≈ +1-2) + new pure helper (≈ +3-4) + two-provider
  integration (≈ +2-4); no new module, no new
  `KeyDomain`).
- [ ] Zero clippy warnings.
- [ ] Zero new workspace deps.
- [ ] Prediction-vs-reality block filled.
