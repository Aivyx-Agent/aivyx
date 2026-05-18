# Phase 83 — Cross-Session Pattern Learning (the durable co-occurrence ledger)

Phase 77 made the assistant learn *which topics help*. Phase 82
made that signal durable and longitudinal. But the learning is
still **per-topic and independent** — it never notices that two
topics *travel together*: that whenever it recalls "deploy
runbook" it also recalls "rollback steps," and turns where both
surface go well. That higher-order structure is exactly what
Phase 77 deferred as "cross-session pattern learning," and the
Phase 82 durable-ledger model is the proven way to capture it.

Phase 83 adds a **persistent, time-decayed co-occurrence
ledger**: on the existing reflection cadence it folds in which
topic *pairs* were recalled together and whether those turns
helped, accumulating a durable cross-session affinity signal
that self-decays and self-prunes. It is **zero-config,
behaviour-neutral, and surface-only** — it learns and shows the
patterns; *consuming* them (cluster-aware recall,
pattern-driven proposals) is the explicit next phase, exactly
the conservative substrate-then-consumption discipline Phase 82
just validated.

## Why this, why now

- It is Phase 77's headline deferral, and Phase 82 just built
  the durability model that unblocks it. Per-topic helpfulness
  is shallow; the relationships *between* topics are where
  cross-session structure lives.
- Every precondition is proven. A single `RecallEvent` already
  carries multiple `hits` (the co-occurrence source); the
  Phase 77 per-event signal is uniform across those hits (so
  "this co-recalled set landed in a helpful turn" is directly
  observable); `SessionId` is stable and persisted (genuine
  cross-*session* aggregation); and Phase 82's
  `PersistentHelpfulnessLedger` is the exact EWMA-decay +
  self-prune + zero-config + fold-on-reflection-cadence
  pattern to mirror.
- Reuse is near-total: a fold of already-correlated recall
  data into a new isolated store on the existing reflection
  cron. No new scheduler, no LLM, no behaviour change, no new
  `AuditTag`.

## Streak predictions

- **DESIGN.md** — **Will hold.** A durable derived signal
  store folded in on the existing reflection cadence touches
  no locked technical-contract decision; it mirrors the
  Phase 82 `PersistentHelpfulnessLedger` shape exactly.
  Prediction: streak **extends to thirty** consecutive phases
  (currently 29).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** A second-order learning
  signal over already-delivered G3/P8 substrate introduces no
  new product commitment and weakens none; the operator-facing
  contract is *strengthened* (a cross-session pattern view
  that did not exist). No commitment-text edit. Prediction:
  streak **extends to twenty-three** consecutive phases.
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold,
  by design.** The new persisted store is a `KeyDomain`
  variant in `aivyx-storage` (the Phase 80/82 precedent); the
  ledger, the fold-in, and the surface extension live in
  `aivyx-channel`; the fold reuses the *existing*
  recall-feedback pass, so no new audit event and no
  `aivyx-core` type change (the Phase 76/77/80/81/82 streak
  lesson, continued). Prediction: streak **extends to
  thirty-one** consecutive phases (new project record, beats
  Phase 82's 30).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

- **New workspace deps** — Zero. Reuses the 70/77 cadence, the
  77 `correlate`, the Phase 82 persistence/decay model, and
  the 78 surface.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_83.md` + `docs/README.md` status row.

### Task 2 — `KeyDomain::CooccurrenceLedger` + `PersistentCooccurrenceLedger`

`aivyx-storage` (the full Phase 82 KeyDomain checklist —
15 → 16 variants) + `aivyx-channel`, **zero-config** (the
Phase 77/82 passive-signal precedent — auto-initialised, no
`[…]` block):

- Encrypted, HKDF-isolated `CooccurrenceLedger` domain
  (variant + `as_bytes` + `table_name` + `ALL`/`subkeys` +
  `subkeys` struct array size + `derive_all_subkeys` +
  `subkey_for` + tripwire comment/match + isolation test).
- `PersistentCooccurrenceLedger` over the domain. Key = the
  **canonical topic pair** (the two topic strings sorted, then
  a collision-safe join — escape the delimiter so
  `("a|","b") ≠ ("a","|b")`). Value = `{ ewma_score: f32,
  samples: u32, last_update_secs: u64 }` (the Phase 82
  `LedgerEntry` shape). API mirrors Phase 82:
  - `record_window(pair_net: &[((String, String), f32)],
    now_secs)` — per pair: decay then add the window net,
    bump `samples`, stamp `last_update`.
  - `pair_score(a, b, now_secs)` — point lookup, decayed.
  - `ranked(now_secs)` — all pairs, decayed, score-sorted.
  - `top_affinities(now_secs, top_n)` — the surface view
    (`CooccurrencePatterns { top_pairs: Vec<PairScore> }`).
  - `prune(now_secs)` — drop pairs decayed sub-epsilon AND
    untouched past the horizon.
  - Constants `COOCCURRENCE_HALF_LIFE_SECS` (~60d),
    `COOCCURRENCE_PRUNE_EPSILON`, `COOCCURRENCE_PRUNE_HORIZON_SECS`
    (code constants — tuning deferred, the Phase 82
    precedent). Tests: isolation, canonical-key (order- and
    delimiter-safe), EWMA fold, decay, prune, empty.

### Task 3 — Fold-in on the reflection recall-feedback pass

`aivyx-channel`, threaded via the existing `RecallFeedbackDeps`
(no new scheduler/pass — the Phase 77 site, Phase 82
precedent):

- `RecallFeedbackDeps` gains
  `cooccurrence_ledger: Option<Arc<PersistentCooccurrenceLedger>>`.
  In `run_recall_feedback_pass`, **after** the actuators *and*
  the Phase 82 helpfulness-ledger fold (so recall-feedback and
  Phase 82 are byte-identical), re-derive per recall event the
  set of distinct topics, **bound to the top-K
  highest-scoring hits** per event (the deterministic O(n²)
  cap, Q4a), enumerate the distinct unordered topic pairs,
  attribute the event's Phase 77 ±signal uniformly to each
  pair, aggregate per-window net per pair, then
  `ledger.record_window(&pair_net, now_secs)` and
  `ledger.prune(now_secs)` + a per-cycle breadcrumb
  `aivyx cooccurrence: folded N pair(s), pruned M`. Absent
  ledger → complete no-op (passive add-on; recall-feedback
  unaffected). Threaded via `DaemonConfig` + built zero-config
  in the binary under the same condition as the recall log.
  Integration test over a real store: two cycles → pair
  accumulation + cross-cycle decay + prune; a pair seen across
  distinct sessions aggregates.

### Task 4 — Phase 78 longitudinal surface

`aivyx-channel`, mirroring the Phase 79/80/81/82 surface
extensions exactly:

- The read-only `GetLearningInsights` response gains
  `cooccurrence: Option<CooccurrencePatterns>` (the top
  durable affined topic pairs + decayed joint score + sample
  count). `#[serde(default)]` round-trip back-compat; resolved
  best-effort from the ledger in the handler (error → `None`,
  empty → `None`). Rendered in the `aivyx learning` CLI and
  the Web UI Learning pane ("topics that consistently help
  together") with parity to the Phase 82 accumulated block.
  `daemon_client` return tuple grows to a 7-tuple. HTML smoke
  + IPC round-trip updated.

### Task 5 — Tests + docs + exit

- Tests: KeyDomain isolation, ledger canonical-key + EWMA fold
  + decay + prune + empty, fold-in pass integration (two
  cycles + cross-session over a real store), surface IPC
  round-trip, CLI/Web UI render.
- Docs: `docs/INSTALL.md` "Cross-session pattern learning
  (Phase 83)" (zero-config, what a pair is, the decay, the
  top-K bound, that it changes no behaviour on its own, where
  it surfaces); `examples/aivyx.toml` a short
  automatic-and-unconfigured note (no block, the Phase 77/82
  precedent).
- Exit: ROADMAP + PRODUCT_ROADMAP frozen entries, docs/README
  status flip, prediction-vs-reality, hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Primitive:** (a) Co-occurrence topic *pairs* —
  unordered {A,B} recalled in the same event, durable
  EWMA-decayed joint-helpfulness. Structural, no LLM,
  deterministic, boundable. Sequential and n-ary clusters
  defer (the conservative-v1 discipline of Phases 80–82).
- **Q2 — Durability:** (a) A new durable, EWMA-decayed,
  self-pruning co-occurrence ledger (new HKDF-isolated
  `KeyDomain`, 16th) — the raw recall log GC's at 30 days, so
  a durable derived store is the only way to learn genuinely
  cross-session patterns. Exactly the Phase 82 model.
- **Q3 — Scope:** (a) Surface-only (passive). Detect +
  persist + show on the Phase 78 surface; change no
  behaviour. Consuming patterns (cluster-aware co-recall,
  pattern-driven proposals) is the explicit next phase.
- **Q4 — Control + bound + legibility:** (a) Zero-config (the
  Phase 77/82 passive-signal precedent); bound the O(n²)
  blowup by folding only pairs among the top-K highest-scoring
  hits per event (deterministic cap); EWMA-decay + self-prune;
  surfaced read-only on the Phase 78 surface + breadcrumb;
  K/half-life/prune are code constants (tuning deferred).

## Deferrals

**Rolling deferrals carried into Phase 83:**

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

**Likely Phase 83 deferrals:**

- **Pattern consumption.** This phase learns + surfaces; using
  patterns (cluster-aware co-recall when one sibling is
  recalled; pattern-driven Persona proposals) is the explicit
  next phase.
- **Sequential / temporal patterns.** v1 is unordered pairs;
  ordered "B follows A" sequence mining defers.
- **N-ary clusters.** v1 is pairs (which compose transitively
  for a fraction of the cost); first-class 3+-topic clusters
  defer.
- **Operator-tunable top-K / half-life.** Code constants in
  v1 (the Phase 82 precedent); a `[pattern_learning]` tuning
  block defers.

## Prediction vs. reality

**Streak — all three predictions correct (the headline).**
DESIGN.md, PRODUCT.md, and `aivyx-core/src/lib.rs` are all
byte-identical to their entry hashes:

- DESIGN.md `89dc8903…` unchanged → streak **30** (predicted
  "extends to thirty" — exact).
- PRODUCT.md `cd60c4f9…` unchanged → streak **23** (predicted
  "extends to twenty-three" — exact).
- `aivyx-core/src/lib.rs` `69fb9af1…` unchanged → streak
  **31**, a new project record beating Phase 82's 30
  (predicted "extends to thirty-one (new record)" — exact).

The Phase 76/77/80/81/82 lesson held again: the new persisted
store is a `KeyDomain` variant in `aivyx-storage`; the ledger,
the fold-in, and the surface live in `aivyx-channel`; the
fold reuses the *existing* recall-feedback pass, so no new
audit event and no `aivyx-core` change.

**Test delta — +10 (1534 → 1544): a small miss vs the
phase-specific ~+12-16 refinement, but in the recalibrated
≈ +8-12 band.** Phase 82 (the first instance of this regime)
landed +10 and was recalibrated to "zero-config/surface-only
+ new KeyDomain ≈ +8-12." For Phase 83 the prediction *raised*
that to ~+12-16, betting the pair detector's canonical-key +
top-K-bound + co-occurrence enumeration would carry "more
pure-logic tests than Phase 82's per-topic EWMA." Reality:
Phase 83 landed at **exactly Phase 82's +10**. The bet was
wrong, instructively: the cooccurrence ledger had 6 unit
tests vs the helpfulness ledger's 5 (+1 for the canonical-key
test), but the rest of the test surface is *fixed
scaffolding* that doesn't scale with detector complexity —
KeyDomain isolation (×2), ledger CRUD/decay/prune (×~6),
fold-in integration (×1), CLI render (×1). The empirical
constant for this regime is **≈ +10**, and "the store keys on
pairs not topics" does not move it. Recalibration: do **not**
inflate the band for detector/key sophistication within the
zero-config/surface-only/one-KeyDomain regime — it is ≈ +10,
full stop. (Bands now: reuse-only ≈ +10-15;
zero-config/surface-only+KeyDomain ≈ +10; new-surface-no-new-
domain ≈ +17; config+detector(+domain) ≈ +20.)

This is a *calibration* miss, not a *scope* miss: every
planned surface (16th KeyDomain + isolation, the canonical-
pair ledger with EWMA/decay/prune, the top-K-bounded fold-in
on the existing pass, the Phase 78 cross-session view in
CLI + Web UI, IPC round-trip) shipped exactly as scoped, and
the conservative Q-block answers (pairs not sequences,
surface-only, zero-config) deliberately kept the blast radius
minimal.

**One in-scope clippy resolution, recorded honestly.** Adding
the seventh per-phase read-only field to the
`LearningInsights` payload pushed the `DaemonMessage` /
`DaemonEnvelope` protocol enums past clippy's
`large_enum_variant` threshold (Phase 82's sixth field was
just under). The `LearningInsights` payload *legitimately*
accretes one surface field per learning phase and is the
common `QueryResponse` case; boxing every protocol field for
a non-hot-path control message is churn the next phase
reintroduces. Resolved with a justified
`#[allow(clippy::large_enum_variant)]` on the two envelopes —
the sanctioned escape for an intentionally-large protocol
variant, consistent with the codebase's existing
`#[allow(clippy::too_many_arguments)]` (Phase 81). No wire
change, no behaviour change.

No new clippy warnings. No new workspace deps. Zero behaviour
change to the recall-feedback loop or the Phase 82 ledger
(the co-occurrence fold runs *after* both — verified by their
unchanged pass/fold tests).

## Exit criteria

- [x] `KeyDomain::CooccurrenceLedger` (16th) + isolation test
  + zero-config `PersistentCooccurrenceLedger` (canonical-pair
  key, EWMA fold + decayed read + prune) — Task 2.
- [x] Fold-in on the reflection recall-feedback pass after the
  actuators + Phase 82 fold; top-K-bounded pair enumeration;
  no-op when absent; per-cycle prune + breadcrumb — Task 3.
- [x] Phase 78 surface extended with the durable affined-pair
  view + sample counts (CLI + Web UI) — Task 4.
- [x] Tests across KeyDomain, ledger math/canonical-key,
  fold-in pass integration (incl. cross-session), IPC
  round-trip, CLI/Web UI render — Task 5.
- [x] `docs/INSTALL.md` + `examples/aivyx.toml` updated —
  Task 5.
- [x] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 5.
- [x] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [x] DESIGN.md streak extends to thirty.
- [x] PRODUCT.md streak extends to twenty-three.
- [x] Production-core streak extends to thirty-one (new
  record) — `lib.rs` byte-identical.
- [~] Test count delta: positive — **+10 (1534 → 1544)**, a
  small miss vs the ~+12-16 refinement but in the
  Phase-82-recalibrated ≈ +8-12 band (landed at exactly
  Phase 82's +10; the regime constant is ≈ +10 — fixed
  scaffolding, not detector complexity, dominates).
  Recalibrated in prediction-vs-reality.
- [x] Zero clippy warnings (one justified
  `#[allow(large_enum_variant)]` on the protocol envelopes).
- [x] Zero new workspace deps.
- [x] Prediction-vs-reality block filled.
