# Phase 82 — Persistent Helpfulness Ledger (durable, longitudinal self-learning)

For 81 phases the assistant's "did recalling this actually
help" signal has been **ephemeral**. Phase 77 computes a
`HelpfulnessTally` each reflection cycle over a lookback
window, uses it to bias retention + file proposals, and then
**throws it away**. Nothing accumulates. That single design
choice is why Phase 81 had to make decay age-only (no
sustained-helpfulness signal to consult), why Phase 77
deferred cross-session pattern learning, and why Phase 78
deferred "history beyond the recall-log window."

Phase 82 makes the learning **durable and longitudinal**: a
persisted, per-topic, time-decayed helpfulness ledger that the
existing reflection cron folds each window's tally into, that
naturally ages stale signal away, and that the Phase 78 trust
surface exposes as the assistant's accumulated picture of what
has consistently helped. This is the "make self-learning
*remember its own learning*" step — the substrate that
retroactively unblocks three prior deferrals and is the
prerequisite for the next arc.

## Why this, why now

- It is the common cause behind three separate deferrals
  (Phase 77 cross-session patterns, Phase 78 longitudinal
  history, Phase 81 helpfulness-driven decay). Fixing the
  ephemerality is higher-leverage than any one of them.
- Every precondition is proven. `correlate()` already produces
  a clean per-cycle tally (Phase 77); the reflection cron
  already runs that pass (Phase 70/77); `PersistentRecallLog`
  is the exact persistence pattern to mirror
  (KeyDomain + time-clamped GC); the Phase 78 surface is the
  proven legibility extension point.
- Reuse is near-total: the ledger is a fold of an already-
  computed tally into a new isolated store. No new scheduler,
  no LLM, no behaviour change on its own (a passive internal
  signal — exactly Phase 77's zero-config nature).

## Streak predictions

- **DESIGN.md** — **Will hold.** A durable signal store folded
  in on the existing reflection cadence touches no locked
  technical-contract decision; it mirrors the proven
  `PersistentRecallLog` shape. Prediction: streak **extends to
  twenty-nine** consecutive phases (currently 28).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** Making the already-delivered
  G3/P8 self-learning signal durable introduces no new product
  commitment and weakens none; the operator-facing contract is
  *strengthened* (a longitudinal trust view that did not exist
  before). No commitment-text edit. Prediction: streak
  **extends to twenty-two** consecutive phases.
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold,
  by design.** The new persisted store is a `KeyDomain`
  variant in `aivyx-storage` (the Phase 80 `ProactiveLog`
  precedent — `aivyx-storage`, not `aivyx-core`); the fold-in,
  the ledger type, and the surface extension live in
  `aivyx-channel`. No new `AuditTag`, no `aivyx-core` type
  change (the Phase 76/77/80/81 streak lesson, continued).
  Prediction: streak **extends to thirty** consecutive phases
  (new project record, beats Phase 81's 29).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

- **New workspace deps** — Zero. Reuses the 70/77 cadence, the
  77 `correlate`, the recall-log persistence pattern, and the
  78 surface.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_82.md` + `docs/README.md` status row.

### Task 2 — `KeyDomain::HelpfulnessLedger` + `PersistentHelpfulnessLedger`

`aivyx-storage` (the full Phase 80 KeyDomain checklist —
14 → 15 variants) + `aivyx-channel`, **zero-config** (the
Phase 77 / `RecallEvents` precedent — a passive internal
signal, auto-initialised, no `[…]` block):

- Encrypted, HKDF-isolated `HelpfulnessLedger` domain (variant
  + `as_bytes` + `table_name` + `ALL`/`subkeys` +
  `derive_all_subkeys` + `subkey_for` + tripwire + isolation
  test).
- `PersistentHelpfulnessLedger` over the domain. Key = topic
  bytes; value = `{ ewma_score: f32, samples: u32,
  last_update_secs: u64 }`. API:
  - `record_window(net_by_topic: &[(String, f32)], now_secs)`
    — per topic: `ewma = decay(dt)·ewma + window_net`,
    `samples += hits`, `last_update_secs = now`. `decay(dt) =
    0.5^(dt / HELPFULNESS_HALF_LIFE_SECS)` so a topic that
    stopped helping fades on its own.
  - `topic_score(topic) -> Option<LedgerEntry>` (point get,
    decayed-to-now on read).
  - `ranked(now_secs) -> Vec<(String, LedgerEntry)>` (scan,
    decayed, score-sorted) for the surface.
  - `prune(now_secs) -> usize` — drop rows whose decayed
    magnitude < `HELPFULNESS_PRUNE_EPSILON` and
    `last_update` older than `HELPFULNESS_PRUNE_HORIZON_SECS`
    (bounded growth that mirrors the signal's own decay).
  - Constants `HELPFULNESS_HALF_LIFE_SECS` (~60d),
    `HELPFULNESS_PRUNE_EPSILON`, `HELPFULNESS_PRUNE_HORIZON_SECS`
    (code constants — tuning deferred, exactly as
    `RECALL_LOG_RETAIN_SECS` is). Per-class + empty +
    decay-math + prune tests.

### Task 3 — Fold-in on the reflection recall-feedback pass

`aivyx-channel`, threaded via the existing `RecallFeedbackDeps`
(no new scheduler, no new pass — the Phase 77 site):

- `RecallFeedbackDeps` gains
  `helpfulness_ledger: Option<Arc<PersistentHelpfulnessLedger>>`.
  In `run_recall_feedback_pass`, immediately **after**
  `correlate(&recalls, summaries)` (so Actuators A/B are
  untouched): aggregate the `(topic, seq)` tally to a per-topic
  net, `ledger.record_window(&net_by_topic, now_secs)`, then
  `ledger.prune(now_secs)`. Absent ledger / no recall
  substrate → complete no-op (pre-Phase-82 behaviour). A
  per-cycle breadcrumb `aivyx helpfulness-ledger: folded N
  topic(s), pruned M`. Integration test over a real store:
  two cycles → accumulation + cross-cycle decay + prune.

### Task 4 — Phase 78 longitudinal surface

`aivyx-channel`, mirroring the Phase 79/80/81 surface
extensions:

- The read-only `GetLearningInsights` response gains an
  accumulated, decayed top-helpful / top-unhelpful per-topic
  view **with sample counts** (the longitudinal picture Phase
  78 deferred — distinct from the existing windowed
  `top_helpful`/`top_unhelpful`). `#[serde(default)]` for
  round-trip back-compat; resolved from the ledger in the
  handler. Rendered in the `aivyx learning` CLI and the Web UI
  Learning pane ("accumulated helpfulness (all-time, decayed)")
  with parity to the existing windowed block. HTML smoke + IPC
  round-trip updated.

### Task 5 — Tests + docs + exit

- Tests: KeyDomain isolation, ledger EWMA fold + decay + prune
  + empty, fold-in pass integration (two cycles over a real
  store), surface IPC round-trip, CLI/Web UI render.
- Docs: `docs/INSTALL.md` "Persistent helpfulness ledger
  (Phase 82)" (zero-config, what it accumulates, the half-life
  decay, where it surfaces, that it changes no behaviour on
  its own); `examples/aivyx.toml` a short note that it is
  automatic and unconfigured (no block, like the Phase 77
  recall-feedback note).
- Exit: ROADMAP + PRODUCT_ROADMAP frozen entries, docs/README
  status flip, prediction-vs-reality, hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Accumulation:** (a) Time-decayed EWMA + sample count.
  Per topic `{ ewma_score, samples, last_update_secs }`;
  `ewma = 0.5^(dt/half_life)·ewma + window_net`. A topic that
  used to help but stopped fades on its own — the
  self-improving / things-change posture; enables trivial
  bounded pruning.
- **Q2 — Granularity:** (a) Per topic. The seq dimension is
  aggregated away — topics persist across memory eviction
  while entry seqs churn, so per-topic is the durable grain
  for longitudinal trends and the future decay/pattern arc.
- **Q3 — Scope:** (a) Accumulate + surface only. Build the
  durable ledger, fold each cycle, expose the longitudinal
  Phase 78 view. **Do not** rewire Phase 81 decay this phase —
  the topic→Persona-category mapping is a real separate
  problem, explicitly set up as the next phase.
- **Q4 — Control + legibility:** (a) Zero-config (the Phase 77
  / `RecallEvents` precedent — a passive internal signal, no
  outbound action, no behaviour change alone); new
  HKDF-isolated `KeyDomain`; EWMA-decay-then-prune GC on the
  reflection cadence; half-life/horizon are code constants
  (tuning deferred, as `RECALL_LOG_RETAIN_SECS` is); surfaced
  read-only on the Phase 78 surface.

## Deferrals

**Rolling deferrals carried into Phase 82:**

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

**Likely Phase 82 deferrals:**

- **Helpfulness-driven Persona decay.** The ledger makes the
  signal durable; *consuming* it in Phase 81 decay needs a
  topic→Persona-category mapping that does not exist (Q3). The
  immediate next phase.
- **Cross-session pattern learning.** Higher-order patterns
  over the durable ledger (Phase 77's deferral) build on this
  substrate but are their own phase.
- **Operator-tunable half-life / retention.** Code constants
  in v1 (the `RECALL_LOG_RETAIN_SECS` precedent); a
  `[helpfulness_ledger]` tuning block defers.
- **Topic canonicalization / aliasing.** Topics are raw memory
  strings; a normalization/alias layer (so renamed topics keep
  their accumulated signal) defers.

## Prediction vs. reality

To be filled in at phase exit.

## Exit criteria

- [ ] `KeyDomain::HelpfulnessLedger` + isolation test +
  zero-config `PersistentHelpfulnessLedger` (EWMA fold +
  decayed read + prune) — Task 2.
- [ ] Fold-in on the reflection recall-feedback pass after
  `correlate`; no-op when absent; per-cycle prune + breadcrumb
  — Task 3.
- [ ] Phase 78 surface extended with the accumulated decayed
  per-topic view + sample counts (CLI + Web UI) — Task 4.
- [ ] Tests across KeyDomain, ledger math, fold-in pass
  integration, IPC round-trip, CLI/Web UI render — Task 5.
- [ ] `docs/INSTALL.md` + `examples/aivyx.toml` updated —
  Task 5.
- [ ] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 5.
- [ ] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [ ] DESIGN.md streak extends to twenty-nine.
- [ ] PRODUCT.md streak extends to twenty-two.
- [ ] Production-core streak extends to thirty (new record) —
  `lib.rs` byte-identical.
- [ ] Test count delta: positive (~+18-24; per the converged
  calibration band — a new KeyDomain + the EWMA ledger math +
  fold-in integration carry real new surface, comparable to
  Phase 80's +20 which also added a KeyDomain).
- [ ] Zero clippy warnings.
- [ ] Zero new workspace deps.
- [ ] Prediction-vs-reality block filled.
