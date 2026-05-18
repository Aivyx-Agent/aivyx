# Phase 84 — Cluster-Aware Co-Recall (the first consumption phase)

Phases 82 and 83 built durable learning substrate — the
per-topic helpfulness ledger and the cross-session
co-occurrence ledger — and deliberately shipped both
*surface-only*: the assistant learned the patterns but never
acted on them. Phase 84 is the first phase that **consumes**
that substrate, and it consumes the freshest piece: the
Phase 83 co-occurrence ledger, which was opened with
"consumption is the explicit next phase."

Today Phase 76 auto-recall surfaces only the memories whose
text the turn's query semantically matched. Phase 84 makes
recall **associative**: when a topic A is recalled, the
durable siblings B that have *consistently helped alongside A
across sessions* are also surfaced — even when the literal
query never retrieved them. The assistant stops bringing only
what you asked about and starts bringing what actually goes
with it.

Because this is the first time the assistant *acts* on the
learned signal and changes what the model sees on the hot
path, it ships **opt-in, hard-bounded, budget-neutral, and
self-policing**: the expansion can never reinforce edges it
created itself, and a bad expansion is organically penalised
by the very feedback loop that fed it.

## Why this, why now

- It is the explicit Phase 83 deferral, and the substrate it
  needs (a durable, decayed, self-pruning co-occurrence
  ledger) shipped last phase. Two phases of build-the-
  substrate earn one phase of consume-it.
- Every precondition is proven. Phase 76's
  `SemanticMemoryContext::recall` is the single seam that
  produces the recalled set and logs the `RecallEvent`; the
  Phase 83 ledger already holds the affinities; the Phase
  77/82 feedback loop already measures whatever lands in a
  turn (so a cluster sibling injected into a turn is
  automatically scored — the self-policing comes for free).
- Reuse is high: a bounded post-step on the existing recall
  path + one new per-topic ledger query + a serde-safe marker
  field. No new scheduler, no new `KeyDomain`, no LLM, no new
  `AuditTag`.

## Streak predictions

- **DESIGN.md** — **Will hold.** Consuming an existing
  durable signal inside the existing Phase 76 recall seam
  touches no locked technical-contract decision. Prediction:
  streak **extends to thirty-one** consecutive phases
  (currently 30).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** Making recall associative
  is a quality deepening of the already-delivered G3 recall
  substrate; it introduces no new product commitment and
  weakens none. The operator-facing contract is *strengthened*
  (opt-in, bounded, budget-neutral, fully legible). No
  commitment-text edit. Prediction: streak **extends to
  twenty-four** consecutive phases.
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold,
  by design.** The recall provider is the existing
  `ContextProvider` whose trait already lives in
  `llm_planner.rs` (the Phase 76/79 seam, *not* `lib.rs`);
  the config, the ledger query, the marker field, the
  expansion, and the surface all live in `aivyx-channel` /
  `aivyx-config`. No new audit event, no `aivyx-core` type
  change (the Phase 76/77/80/81/82/83 streak lesson,
  continued). Prediction: streak **extends to thirty-two**
  consecutive phases (new project record, beats Phase 83's
  31).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

- **New workspace deps** — Zero. Reuses the Phase 76 recall
  seam, the Phase 83 ledger, the 77/82 feedback loop, and the
  78 surface.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_84.md` + `docs/README.md` status row.

### Task 2 — `[recall_cluster]` config section

`aivyx-config` (mirrors the Phase 80 `[proactive]` /
Phase 81 `[persona_lifecycle]` pattern):

- `RecallClusterConfig { enabled: bool /* default false */,
  max_siblings: u32 /* hard per-turn cap */, min_affinity:
  f32 /* the ledger score a sibling must clear */ }`. Absent
  section → `None` → cluster expansion **off** (pre-Phase-84
  behaviour, the common case). Validation when enabled:
  `max_siblings >= 1`, `min_affinity > 0.0`.
  `AivyxConfig.recall_cluster: Option<RecallClusterConfig>` +
  `RawRecallCluster` + `build_recall_cluster_config` + loader
  wiring. Tests: defaults, validation, off-when-absent.

### Task 3 — `siblings_of` ledger query + `RecallHit` origin marker

`aivyx-channel`:

- `PersistentCooccurrenceLedger::siblings_of(topic,
  now_secs, top_n, min_affinity) -> Vec<PairScore>` — one
  scan, keep pairs containing `topic` whose decayed score
  clears `min_affinity`, return the other side, score-sorted,
  capped at `top_n`. (The recall path makes exactly one such
  scan per turn — the ledger self-prunes so it stays
  bounded.)
- `RecallHit` gains `#[serde(default)] cluster: bool` (false =
  primary keyword/semantic hit, true = cluster-injected
  sibling). `#[serde(default)]` keeps the recall-log + IPC
  round-trip back-compatible; Phase 77 `correlate_detailed`
  reads only `(topic, seq)` so it is unaffected (asserted).
  Tests: `siblings_of` (order, threshold, cap, both pair
  positions), marker round-trip, Phase 77 unaffected.

### Task 4 — Cluster-aware recall + the self-policing exclusion

`aivyx-channel`, the behaviour change (the crux):

- Thread a `cooccurrence_ledger` handle into
  `SemanticMemoryContext` (a `with_cooccurrence_ledger`
  builder + `recall_cluster` config; binary wiring — the
  ledger is already built, currently passed only to the
  reflection deps). When `[recall_cluster]` is enabled: after
  the Phase 76 base recall, for the recalled topics call
  `siblings_of`, pick the single best not-already-recalled
  memory under each high-affinity sibling topic, and inject up
  to `max_siblings` — **sharing the existing `rag_top_k`
  budget** (siblings displace the weakest primary hits, so
  zero context-size growth and zero token-cost regression).
  Injected hits are recorded in the `RecallEvent` with
  `cluster = true`.
- **Self-policing (Q3a):** the Phase 83 co-occurrence fold in
  `run_recall_feedback_pass` **excludes** `cluster`-marked
  hits when enumerating pairs — the ledger only ever learns
  from organic keyword/semantic co-recall, never from its own
  expansion (no runaway self-reinforcement). Cluster hits are
  *not* excluded from the Phase 77/82 helpfulness signal, so a
  bad expansion organically lands in worse turns and the
  driving edge decays. Integration tests: bounded +
  budget-neutral + marked injection; the fold ignores
  cluster-marked hits.

### Task 5 — Observability (Phase 78-consistent)

`aivyx-channel`, mirroring the Phase 79 `persona_selection`
shared-stat pattern (recall runs per-turn in the planner, not
per reflection cycle):

- A shared last-recall-cluster stat
  (`Arc<RwLock<Option<…>>>`) the recall provider writes and
  `GetLearningInsights` reads: last turn's injected sibling
  count + the pairs. `#[serde(default)]` field on the
  `LearningInsights` response; resolved best-effort in the
  handler; rendered in the `aivyx learning` CLI and the Web
  UI Learning pane ("cluster co-recall: N siblings injected
  last turn"); per-turn stderr breadcrumb. HTML smoke + IPC
  round-trip updated.

### Task 6 — Tests + docs + exit

- Tests: config, `siblings_of` + marker, recall expansion
  (bounded/budget-neutral/marked) + fold-exclusion
  integration, observability, IPC round-trip, CLI/Web UI
  render.
- Docs: `docs/INSTALL.md` "Cluster-aware co-recall
  (Phase 84)" (opt-in, what a sibling is, the cap +
  affinity floor, budget-neutral, self-policing, where it
  surfaces, how to turn off); `examples/aivyx.toml`
  `[recall_cluster]` block with the off-by-default note.
- Exit: ROADMAP + PRODUCT_ROADMAP frozen entries, docs/README
  status flip, prediction-vs-reality, hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Mechanism:** (a) Bounded sibling injection. Surface
  the affined siblings the literal query missed (the actual
  value); re-rank-only cannot. Tightly bounded.
- **Q2 — Gating:** (a) Opt-in `[recall_cluster]` (off by
  default). The first phase that acts on the learned signal
  and changes hot-path context — the Phase 80/81
  behaviour-change discipline.
- **Q3 — Loop safety:** (a) Cluster hits marked on
  `RecallHit` and **excluded from the Phase 83 co-occurrence
  fold** (the ledger never learns from its own expansion) but
  still measured by the Phase 77/82 helpfulness signal (a bad
  expansion self-penalises). Self-policing.
- **Q4 — Bounds + budget + legibility:** (a) Hard
  `max_siblings` cap + `min_affinity` floor + siblings share
  the existing `rag_top_k` budget (budget-neutral, no
  context/token regression) + breadcrumb + Phase 78 surface
  stat.

## Deferrals

**Rolling deferrals carried into Phase 84:**

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
- Phase 76 deferrals (conversational-window query, heuristic
  recall gate, token-budget context sizing).
- Phase 77 deferrals (`[recall_feedback]` tuning knob,
  LLM-judged recall usefulness).
- Phase 78 deferrals (per-memory-entry drill-down, Web UI
  live refresh, actionable insights).
- Phase 79 deferrals (`[persona]` tuning block,
  conversational-window selection, behavioural Persona).
- Phase 80 deferrals (standalone `[[proactive_schedule]]`,
  LLM-composed proactive prose, conversational/interactive
  proactive, additional proactive signal classes).
- Phase 81 deferrals (contradiction-based supersession,
  standalone `[[persona_lifecycle_schedule]]`, facet-scoped
  one-click revert).
- Phase 82 deferrals (helpfulness-driven Persona decay —
  needs a topic→category mapping; operator-tunable
  half-life/retention; topic canonicalization).
- Phase 83 deferrals (sequential/temporal patterns, n-ary
  clusters, operator-tunable top-K/half-life).

**Likely Phase 84 deferrals:**

- **Helpfulness-driven Persona decay.** Still the other open
  consumption (Phase 81+82); needs the topic→category
  mapping. Unaffected by this phase.
- **Re-rank by affinity.** v1 injects missed siblings;
  additionally boosting the *ordering* of existing candidates
  by affinity defers.
- **Operator-tunable affinity/cap beyond enable + 2 knobs.**
  v1 ships `max_siblings` + `min_affinity`; richer policy
  (per-topic overrides, decayed-vs-raw) defers.
- **Cluster-contribution sub-stat on the helpfulness view.**
  v1 surfaces a last-turn cluster stat; folding "how much did
  cluster siblings help vs primary" into the Phase 82
  accumulated view defers.

## Prediction vs. reality

**Streak — all three predictions correct (the headline).**
DESIGN.md, PRODUCT.md, and `aivyx-core/src/lib.rs` are all
byte-identical to their entry hashes:

- DESIGN.md `89dc8903…` unchanged → streak **31** (predicted
  "extends to thirty-one" — exact).
- PRODUCT.md `cd60c4f9…` unchanged → streak **24** (predicted
  "extends to twenty-four" — exact).
- `aivyx-core/src/lib.rs` `69fb9af1…` unchanged → streak
  **32**, a new project record beating Phase 83's 31
  (predicted "extends to thirty-two (new record)" — exact).

The Phase 76/79 seam held once more: the recall provider is
the existing `ContextProvider` whose trait lives in
`llm_planner.rs`, so a hot-path *behaviour* change still
required no `aivyx-core` edit. Config, the ledger query, the
`RecallHit` marker, the expansion, the self-policing fold
exclusion, and the surface all landed in `aivyx-channel` /
`aivyx-config`; no new `AuditTag`.

**Test delta — +10 (1544 → 1554): a second consecutive miss
vs the ~+16-22 prediction.** Phase 83 missed +12-16 (landed
+10); Phase 84 predicted ~+16-22 reasoning "config + hot-path
behaviour ≈ the Phase 80 +20 regime" and again landed **+10**.
Two misses in the same direction is a model error, not noise.
The confirmed calibration law: realized test count is driven
almost entirely by **how many new standalone pure modules
each get a per-branch unit suite** — empirically a detector
module ≈ +7, a new `KeyDomain` ≈ +2 (isolation/metadata), a
config section ≈ +5-6 — and *not* by whether the phase adds
config, changes the hot path, or threads a wide surface.
Phase 84 added a config section (+5) but **no new detector
module and no new `KeyDomain`**: the behaviour change rode the
*existing* `memory_recall` + Phase 83 ledger modules and was
integration-tested (2 tests), the marker/query added +2, the
surface +1 — total +10, the same floor as Phases 82/83.

Recalibration (replacing the earlier band zoo with one rule):
**predict ≈ +10 unless the phase introduces a new
unit-tested pure module; add ≈ +7 per detector-class module
and ≈ +2 per new `KeyDomain` on top.** Phase 84 = +5 (config)
+ ~+5 (query/marker/integration/render) ≈ +10, no detector,
no domain — the rule now retro-fits all of 80–84. This is a
*calibration* miss, not a *scope* miss: every planned surface
(config, `siblings_of`, the marker, bounded budget-neutral
injection, the self-policing fold exclusion, the Phase 78
`cluster_recall` surface, IPC round-trip) shipped exactly as
scoped, and the conservative Q-block answers deliberately
reused existing modules rather than spawning new ones —
which is *why* the count is low and the blast radius small.

**Two in-scope clippy resolutions, recorded honestly.**
(1) `large_enum_variant` on the protocol envelopes was
already suppressed (Phase 83) — the new `cluster_recall`
field needed no further action. (2) `render_insights` crossed
`too_many_arguments` (it accretes one display param per
learning phase: 79/80/81/82/83/84). Resolved with a justified
`#[allow(clippy::too_many_arguments)]` — the sanctioned escape
for an intentionally-wide pure renderer, consistent with the
codebase's existing `#[allow(too_many_arguments)]` (Phase 81
`fire_reflection`). No wire change, no behaviour change.

No new clippy warnings. No new workspace deps. Recall is
byte-identical to pre-Phase-84 when `[recall_cluster]` is
absent/disabled (asserted), and the co-occurrence ledger +
recall-feedback loop are unaffected by the self-policing
exclusion except as intended (asserted).

## Exit criteria

- [x] `[recall_cluster]` config + validation + off-when-absent
  — Task 2.
- [x] `siblings_of` ledger query + `RecallHit` `cluster`
  marker (serde-safe, Phase 77 unaffected) — Task 3.
- [x] Cluster-aware injection on the Phase 76 recall path:
  opt-in, `max_siblings`-capped, `min_affinity`-gated,
  budget-neutral (shares `rag_top_k`), marked; co-occurrence
  fold excludes marked hits (self-policing) — Task 4.
- [x] Breadcrumb + Phase 78 surface extended (CLI + Web UI) —
  Task 5.
- [x] Tests across config, ledger query/marker, recall
  expansion + fold-exclusion integration, observability, IPC
  round-trip, CLI/Web UI render — Task 6.
- [x] `docs/INSTALL.md` + `examples/aivyx.toml` updated —
  Task 6.
- [x] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 6.
- [x] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [x] DESIGN.md streak extends to thirty-one.
- [x] PRODUCT.md streak extends to twenty-four.
- [x] Production-core streak extends to thirty-two (new
  record) — `lib.rs` byte-identical.
- [~] Test count delta: positive — **+10 (1544 → 1554)**, a
  second consecutive miss vs ~+16-22. Confirmed the
  calibration law (count tracks new unit-tested pure modules,
  not config/behaviour breadth): no new detector module / no
  new `KeyDomain` → the ≈ +10 floor. Recalibrated in
  prediction-vs-reality. Scope fully shipped.
- [x] Zero clippy warnings (one new justified
  `#[allow(too_many_arguments)]` on `render_insights`).
- [x] Zero new workspace deps.
- [x] Prediction-vs-reality block filled.
