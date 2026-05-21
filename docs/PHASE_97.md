# Phase 97 — Token-Budget Context Sizing (Phase 76's + Phase 86's longest-running content deferral, closed)

Phase 76 introduced auto-recall and the `rag_top_k` cap on
how many memories may inject per turn. Phase 79 introduced
adaptive Persona selection with its own per-turn facet cap.
Phase 86 introduced the conversational window with
`recall_window_turns`. All three caps are **count-based**:
"at most K entries / facets / messages." Count is a proxy
for token cost; it's not the cost itself. A single memory
body with a 4 KB blob silently displaces multiple shorter
memories from the same budget; a Persona facet that grew
from one sentence to ten paragraphs eats turn after turn of
input. Phase 76 + Phase 86 both listed "token-budget
context sizing" as deferrals; carried 21 phases and 11
phases respectively.

Phase 97 closes both deferrals with an additive
**recall_token_budget** cap. The existing `rag_top_k` and
Persona-facet counts become **soft hints**; the token
budget is a hard cap applied after rank-ordering. When the
post-rank selection exceeds the budget, the
**lowest-ranked items are dropped** (recall: by cosine
score; Persona: by selection priority) until the
injection fits. No mid-item truncation; no partial-content
delivery — operators get full items or nothing.

With `recall_token_budget = 0` (the default), behaviour is
byte-identical to pre-Phase-97 (count-based only). The
token estimator is hand-rolled at `chars/4` with a small
fudge factor for whitespace + punctuation; ±20% accuracy is
adequate for "is this injection over the budget the
operator set." Sub-token accuracy isn't worth a new
workspace dependency.

## Why this, why now

- Phase 76 and Phase 86 both listed "token-budget context
  sizing" in their deferrals; both deferrals are unique
  remaining content-quality items in the recall pipeline.
- After Phase 96's ANN index, query-time perf is the
  shipped story; **prompt-side cost** is now the next
  pinch point. Long memory bodies + large Personas
  silently inflate every turn's input cost without
  reaching the model's actual context limit until things
  break.
- Surface area is tight. One pure `estimate_tokens(text)
  -> u32` helper; one pure
  `apply_token_budget(items, budget) -> Vec<...>`
  helper; two integration points (the recall formatter
  + the Persona-selection step). One opt-in config knob.
- The change is **purely additive**. With
  `recall_token_budget = 0`, every code path is
  byte-identical to pre-Phase-97. With `>= 1`, the
  budget runs **after** the existing rank-and-filter
  step, dropping the lowest-ranked items until the
  injection fits.
- Reuse is total. No new ledger, no new substrate, no
  new IPC contract. The token-budget helper composes
  over the existing `Vec<(MemoryEntry, f32)>` recall
  output and the existing facet-priority list.

## Streak predictions

- **DESIGN.md** — **Will hold.** Token-budget enforcement
  is a post-rank trim over existing structures; touches
  no locked technical-contract decision. The recall and
  Persona ordering rules are unchanged; the budget just
  truncates the tail. Hash at entry:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Prediction: streak **extends to forty-four** (currently
  43).

- **PRODUCT.md** — **Will hold.** P9 (recall) + P14
  (Persona) commitments are unchanged; the operator-
  facing contract is *strengthened* (the loop won't
  silently inflate context cost when entries grow long).
  Hash at entry:
  `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.
  Prediction: streak **extends to thirty-seven**
  (currently 36).

- **Production-core `aivyx-core/src/lib.rs`** — **Will
  hold, by design.** The token estimator + budget helper
  live in `aivyx-channel` (or a new small `token_budget`
  module within); the integration is at the recall
  formatter + Persona-selection call sites; the config
  knob lives in `aivyx-config`. No `aivyx-core` touch.
  Hash at entry:
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.
  Prediction: streak **extends to forty-five**
  consecutive phases (new project record, beats Phase
  96's 44).

- **New workspace deps** — Zero. Hand-rolled `chars/4`
  estimator; no tokenizer crate.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_97.md` + `docs/README.md` status row.

### Task 2 — `[embedding].recall_token_budget` knob

`aivyx-config`:

- `EmbeddingConfig` gains
  `recall_token_budget: u32` (default `0`). With `0`,
  the existing count-based caps (`rag_top_k`, Persona
  K-facets) are the only constraint — byte-identical
  to pre-Phase-97. With `>= 1`, the budget is an
  additional hard cap; the lowest-ranked items are
  dropped until the injection fits.
- `RawEmbedding` + the build path. No validation
  bounds: `0` is the meaningful disabled value, and
  large values are the operator's call (a `100_000`
  budget effectively disables enforcement for any
  realistic recall, which is fine).
- Tests: default; explicit value wins; absent
  section honored (build path's `any_set` predicate
  picks up the new field).

### Task 3 — `estimate_tokens` + `apply_token_budget` pure helpers (the crux)

`aivyx-channel`:

- New `token_budget` module exposing two pure
  functions:
  - `pub fn estimate_tokens(text: &str) -> u32` —
    hand-rolled `chars/4` baseline with a small
    boundary fudge (`+1` per leading/trailing
    whitespace transition). Returns at minimum `1`
    for any non-empty string (an empty body shouldn't
    eat the entire budget on a phantom token; a
    non-empty body costs at least one token).
  - `pub fn apply_token_budget<F>(items: Vec<T>,
    budget: u32, cost_of: F) -> Vec<T>` where
    `F: Fn(&T) -> u32`. Walks items in order
    (caller's pre-ranked sequence), accumulating
    cost. Drops items at the tail once the running
    cost would exceed the budget — the first dropped
    item is the lowest-ranked one over the budget.
    Defensive: a single item costing more than the
    whole budget is dropped (we never partial-include
    an over-budget item).
- Unit tests on the pure helpers:
  - `estimate_tokens`: empty → 0; "a" → 1; "hello
    world" → ~3 (12 chars + 1 boundary); long body
    pinned at the chars/4 magnitude; Unicode counted
    by `chars()`, not bytes.
  - `apply_token_budget`: empty input → empty
    output; budget = 0 → empty output; under-budget
    → all items returned; over-budget → tail
    dropped; over-budget single item → all dropped;
    interior items can't be skipped (the caller
    pre-ranked, the budget respects that order).

### Task 4 — Recall + Persona injection integration

`aivyx-channel`:

- `SemanticMemoryContext::recall` (in `memory_recall.rs`):
  AFTER the existing rank + `rag_min_similarity` +
  cluster-expansion steps but BEFORE `format_block`,
  apply the token budget. The hits are already in
  rank order; the budget trims the tail.
- `PersonaContextRefiner::refine` (in `persona_context.rs`):
  AFTER the adaptive Persona selection ranks facets by
  cosine relevance and applies its K-facet cap, apply
  the same budget. Facets are already in selection
  order; the budget trims the tail.
- Both call sites read `recall_token_budget` from the
  passed-in `EmbeddingConfig`. With `0`, the budget
  helper is a no-op.
- Integration tests (pure, against the helper-shaped
  inputs):
  - Recall: 5 hits of varying body length, budget set
    such that only the top-3 fit → exactly the top-3
    return.
  - Persona: 5 facets of varying length, budget set
    such that only the top-2 fit → exactly the top-2
    return.
  - Budget = 0 → all hits/facets pass through
    (regression pin).

### Task 5 — Surface + docs + exit

- `aivyx learning` surface: the existing recall block
  already shows `recalls: N total, S scored`. Phase 97
  adds no new field; the budget's effect is
  intrinsically visible (fewer items in the injected
  block) without needing a surface line. **Surface
  unchanged** for v1; if operators want a "K dropped
  by budget" surface stat, that defers as a follow-up
  observability phase.
- `docs/INSTALL.md` — new "Token-budget context sizing
  (Phase 97)" subsection under the existing Phase 76
  auto-recall section: the chars/4 estimator, the
  combined recall + Persona scope, the drop-lowest-
  ranked eviction posture, the opt-in default.
- `examples/aivyx.toml` — document the new
  `recall_token_budget` knob alongside the existing
  `rag_top_k`, `rag_min_similarity`, etc.
- Exit: ROADMAP + PRODUCT_ROADMAP frozen entries,
  docs/README status flip, prediction-vs-reality, hash
  backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Estimator:** (a) Hand-rolled approximate
  counter (`chars/4` baseline + small fudge factor).
  Preserves the project's zero-new-deps streak; ~20
  lines of code; accuracy ~±20% for English. Sub-token
  accuracy isn't worth a new tokenizer dep.
- **Q2 — Scope:** (a) Combined recall + Persona
  injection. Both displace the model's input budget;
  one knob caps both. `rag_top_k` and Persona K-facets
  become soft hints; the token budget is the hard cap.
  One coherent operator-facing number.
- **Q3 — Eviction:** (a) Drop lowest-ranked items
  until under budget. Recall: lowest cosine score
  first. Persona: lowest selection priority first.
  Conservative — no mid-item truncation; operators get
  full items or nothing.
- **Q4 — Knob:** (a) New
  `[embedding].recall_token_budget: u32` (default
  `0` = disabled). Matches the Phase 87 / 88 / 91 /
  92 / 93 / 95 / 96 actuator opt-in pattern. With the
  knob off, behaviour is byte-identical to
  pre-Phase-97.

## Deferrals

**Rolling deferrals carried into Phase 97** (Phase 76 +
Phase 86's "token-budget context sizing" deferrals are
**THIS PHASE**):

- v0.1.0 publication (Phase 61 Task 7).
- System-prompt notification-target enumeration (Phase 62).
- `turn_id` correlation refactor (Phase 67).
- Phase 68 deferrals (XOAUTH2, HTML email, attachments,
  multi-account `[email]`).
- Phase 69 deferrals (WebPush / service-worker, notification
  urgency, icons, sound).
- Phase 70 deferrals (multi-window reflection).
- Phase 71 deferrals (per-fire role override, operator-
  customizable reflection prompt — reflection cadence
  learning closed by Phase 95).
- Phase 73 deferrals (persisted rate-limit buckets,
  operator-configurable retry-on list).
- Phase 74 deferrals (fuzzy match, edit-content Web UI,
  per-topic eviction-strategy override).
- Phase 75 deferrals (`aivyx memory reembed`, hybrid
  keyword+semantic fusion, query-embedding cache;
  ANN index closed by Phase 96).
- Phase 76 deferrals (**token-budget context sizing —
  THIS PHASE**).
- Phase 77 deferrals — closed by Phase 93.
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
- Phase 84 deferrals (affinity re-ranking, operator-
  tunable affinity policy).
- Phase 85 deferrals (reflection-facet decay via fuzzy
  embedding).
- Phase 86 deferrals (**token-budget context sizing —
  THIS PHASE**, embed-each-and-pool windows, persisted
  windows).
- Phase 87 deferrals (n-ary cluster proposals, operator-
  tunable LLM prompt).
- Phase 88 deferrals (n-ary cluster decay, pair-affinity
  hysteresis).
- Phase 89 deferrals (operator-tunable `[[topic_alias]]`
  mappings, topic-by-topic exception list, one-time
  migration of existing fragmented data, non-ASCII /
  Unicode stemming).
- Phase 90 deferrals (pattern-based stoplist, LLM-judged
  gate, adaptive thresholds).
- Phase 91 deferrals (per-recall LLM critique, adaptive
  batch size, multi-model ensembling, response-text
  recovery).
- Phase 92 deferrals (atomic chain-level supersession
  primitive, n-ary cluster supersession, semantic-
  similarity supersession; Web UI grouping closed by
  Phase 94).
- Phase 93 deferrals (per-domain/per-topic verdict-mapping
  weights, replace mode, asymmetric Hurt penalty, sum
  mode, on-disk buffer).
- Phase 94 deferrals (atomic transaction IPC for "Approve
  both", backend-side grouping enrichment, drag UI
  affordances, n-ary group rendering).
- Phase 95 deferrals (backoff-multiplier mode, adaptive
  interval, time-of-day pattern learning, persisted
  cadence stat, LLM-based signal-density classifier,
  per-pass skip granularity).
- Phase 96 deferrals (HNSW-quality recall, iterative
  k-means refinement, persisted ANN index across daemon
  restarts, incremental updates, topic-aware centroid
  seeding, `aivyx learning` ANN backend surface line,
  ANN for `aivyx memory search`).

**Likely Phase 97 deferrals:**

- **Exact-tokenizer integration.** The hand-rolled
  estimator is accurate to ~±20%. Operators running at
  very tight model-context margins may want exact
  counts. A future phase could integrate a tokenizer
  crate (`tiktoken-rs` or model-specific) when the
  benefit justifies breaking the zero-deps streak.
- **Per-category budgets.** Q2c's separate recall /
  persona / window budgets defer — v1's combined
  budget is the simplest coherent shape. A future
  phase splits if operators report uneven crowding.
- **Auto-derive from model context window.** Q4c's
  "read max_context from the LLM provider" defers —
  provider metadata isn't always reliable and the
  operator can compute their own budget once.
- **Conversational-window budget.** Phase 86's
  conversational window (`recall_window_turns`)
  remains count-based — Phase 97 budgets recall +
  Persona injection but not the embed-query window.
  A future phase could extend.
- **Mid-item truncation strategy.** Q3b's "snip
  longest item" defers — v1's drop-lowest-ranked
  preserves coherent per-item content; a future
  phase could add intelligent truncation with
  ellipsis-aware boundaries.
- **Surface line for dropped-by-budget count.** The
  `aivyx learning` recall block stays as-is for v1.
  Adding "K dropped by budget last turn" is the
  natural observability follow-up.

## Prediction vs. reality

To be filled in at phase exit.

## Exit criteria

- [ ] `[embedding].recall_token_budget: u32` (default
  `0`) — Task 2.
- [ ] `estimate_tokens` + `apply_token_budget` pure
  helpers in `aivyx-channel` — Task 3.
- [ ] Unit tests on the pure helpers: estimator
  accuracy on common shapes; budget enforcement on
  under / at / over-budget inputs; empty input;
  zero-budget — Task 3.
- [ ] `SemanticMemoryContext::recall` applies the
  budget AFTER rank + `rag_min_similarity` + cluster
  expansion — Task 4.
- [ ] `PersonaContextRefiner::refine` applies the
  budget AFTER adaptive selection's K-facet cap —
  Task 4.
- [ ] Integration test: recall + Persona with mixed-
  length items, budget tight enough to drop the
  bottom-ranked items — Task 4.
- [ ] `docs/INSTALL.md` + `examples/aivyx.toml`
  updated — Task 5.
- [ ] ROADMAP + PRODUCT_ROADMAP + docs/README
  refreshed — Task 5.
- [ ] All four Q-block questions resolved with
  operator sign-off pre-Task 2.
- [ ] DESIGN.md streak extends to forty-four.
- [ ] PRODUCT.md streak extends to thirty-seven.
- [ ] Production-core streak extends to forty-five
  (new record) — `lib.rs` byte-identical.
- [ ] Test count delta: positive (~+10-15; config
  knob ≈ +3-4; pure helpers ≈ +5-7 with multiple
  boundary cases; integration ≈ +2-4).
- [ ] Zero clippy warnings.
- [ ] Zero new workspace deps.
- [ ] Prediction-vs-reality block filled.
