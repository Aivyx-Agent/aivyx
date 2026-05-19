# Phase 87 — Pattern-Driven Persona Proposals (the self-improving Soul, the second consumption)

Phase 84 made auto-recall **act** on the Phase 83 co-occurrence
ledger (durable affined siblings injected on the hot path).
Phase 85 made Persona decay **act** on the Phase 82 helpfulness
ledger (sustained-negative topics retire identity; sustained-
positive protects). Phase 87 closes the symmetric arc: the
co-occurrence ledger now acts on the **Soul-construction**
side too. Durable, consistently-co-occurring pairs of *helpful*
topics propose a new `learned_context` facet — the same
propose-only + operator-gated + Revert-able + core-protected
flow the Persona chain already uses, just driven by the second
durable signal that has so far only fed *recall*.

The visible asymmetry going into Phase 87:

|                    | Acts on Phase 82 helpfulness ledger | Acts on Phase 83 co-occurrence ledger |
|--------------------|--------------------------------------|----------------------------------------|
| Recall (hot path)  | (n/a — recall consumes embeddings)   | **Phase 84** (cluster-aware co-recall) |
| Soul (proposals)   | Phase 77 promotions (warm-only)      | **Phase 87** (this phase)              |
| Soul (decay)       | **Phase 85** (helpfulness-driven)    | (deferred — pattern-driven decay)      |

The single missing cell that closes the "every durable signal
feeds back into the Soul" loop. Phase 85 explicitly deferred
this as "pattern-driven Persona proposals."

## Why this, why now

- It is the deliberate Phase 85 deferral, and it sits on a
  ledger (Phase 83) that has now had two phases (84, 85) of
  operational evidence — the durable signal is real.
- The actuator pattern is established and proven: opt-in
  config block (Phase 80/81/84), reflection-cron cadence
  (Phase 71/77/85), per-cycle cap + dedup (Phase 80), proposes
  through the **existing** Persona proposal chain (Phase 70)
  with the **existing** `propose-only + edit-then-approve +
  Revert + core-protected` flow. No new chain, no new
  KeyDomain, no new AuditTag.
- Reuse is near-total: the conservative double-gate (Q1a)
  reuses Phase 82's `topic_score` + Phase 83's
  `siblings_of`; the per-cycle cap + dedup reuses the Phase 80
  precedent; the LLM phrasing call (Q2b) reuses the same
  reflection LLM provider already on `DaemonConfig`; the
  Phase 78 surface gains one more "last cycle" stat — same
  shape as Phase 79/80/81/84.

## Streak predictions

- **DESIGN.md** — **Will hold.** No locked technical-contract
  decision reopens. A new actuator on existing chains using
  existing signals is exactly the post-Phase-86 ratio. Hash at
  entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Prediction: streak **extends to thirty-four** (currently 33).

- **PRODUCT.md** — **Will hold.** P14 (Persona) is delivered;
  this enriches its proposal pipeline. No new commitment, none
  weakened; the operator-facing contract is *strengthened*
  (the assistant now learns cross-topic *relationships* into
  identity, not just per-topic warmth). No commitment-text
  edit. Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.
  Prediction: streak **extends to twenty-seven** (currently 26).

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold,
  by design.** The new pass + config block + Phase 78
  surface stat all live in `aivyx-channel` / `aivyx-config` /
  `bin/aivyx`. The proposal lands through the existing
  `PersistentPersonaProposalLog` API (Phase 70 / 85
  precedent — no new chain operation). No new `AuditTag` (the
  proposal-chain append already audits). Hash at entry:
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.
  Prediction: streak **extends to thirty-five** consecutive
  phases (new project record, beats Phase 86's 34).

- **New workspace deps** — Zero. Reuses the Phase 83 ledger,
  the Phase 82 ledger, the existing reflection-LLM provider on
  `DaemonConfig`, and the Phase 70 proposal chain.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_87.md` + `docs/README.md` status row.

### Task 2 — `[persona_consolidation]` config block

`aivyx-config` (the Phase 80/81/84 actuator-block precedent):

- New top-level `PersonaConsolidationConfig`:
  - `enabled: bool` (default **`false`** — opt-in like every
    other actuator)
  - `min_affinity: f32` (default `1.0`, mirrors Phase 84
    `recall_cluster.min_affinity`) — the decayed Phase 83 pair
    score floor a candidate must clear.
  - `min_samples: u32` (default `3`, mirrors Phase 85
    `decay_min_samples`) — observation-count floor on the
    pair.
  - `min_topic_helpfulness: f32` (default `0.0`) — both
    endpoints must have Phase 82 ledger score `>=` this.
    Default `0.0` enforces "non-negative" (the Q1a "both
    endpoints helpful" gate); raise it for stricter.
  - `max_proposals_per_cycle: u32` (default `3`, mirrors
    Phase 80 `max_per_cycle`) — hard cap on filings per
    reflection cycle.
- `RawPersonaConsolidation` + the build path; validation when
  `enabled = true`: `min_affinity > 0.0`, `min_samples >= 1`,
  `max_proposals_per_cycle >= 1`,
  `min_topic_helpfulness.is_finite()`.
- Tests: defaults; explicit overrides; each validation reject.

### Task 3 — The consolidation pass (the crux)

`aivyx-channel`:

- New `persona_consolidation` module with a pure
  `select_candidates(co_ledger, helpfulness, persona_chain,
  pending, config, now) -> Vec<Candidate>` function:
  - Pull the top-K affined pairs from Phase 83 above
    `min_affinity` + `min_samples` (the Phase 84
    `siblings_of` path generalized over the ledger).
  - For each candidate pair `(A, B)`: drop if either
    endpoint's Phase 82 helpfulness ledger score is below
    `min_topic_helpfulness` (Q1a conservative double-gate;
    "non-negative" by default).
  - Drop if the Persona chain already has an applied facet
    whose provenance is `consolidate-pair:{A}+{B}` (or the
    canonicalized `{B}+{A}`), OR a Pending proposal with the
    same provenance — the dedup invariant of Phase 70/85
    extended.
  - Truncate to `max_proposals_per_cycle` after sorting by
    (decayed-affinity desc, helpfulness-min desc).
- New `consolidate(...)` async impl: per surviving candidate,
  ask the existing reflection LLM provider to phrase a single
  `learned_context` facet (~one sentence, Q2b) — prompt is
  fixed + short, the operator can edit-then-approve in the
  existing UI (the propose-only safety stays intact). LLM
  errors per candidate are skipped (best-effort); a
  cycle-wide LLM failure records "0 filed, LLM unavailable"
  and ends gracefully.
- File each survivor as a `PendingPersonaProposal` through the
  existing `PersistentPersonaProposalLog::append` with
  `proposal_id = "consolidate-pair:{A}+{B}"` (canonicalized
  alphabetic).
- Wire the pass into `reflection_scheduler` alongside the
  Phase 77 / 82 / 83 / 85 passes — same cron, same shared
  handles, same best-effort posture.
- Unit tests on the pure selector: double-gate (both helpful),
  cap, dedup against chain, dedup against Pending, ordering
  stability.

### Task 4 — Phase 78 surface + breadcrumb

`aivyx-channel`:

- New `PersonaConsolidationStat { ts_secs, filed, skipped,
  reason: Option<String> }` + `SharedPersonaConsolidationStat`
  ctor (the Phase 79/80/81/84 Q4a precedent — one shared
  handle the pass writes and `GetLearningInsights` reads).
- Threaded onto `DaemonConfig` + the query dispatcher; the
  `aivyx learning` view + Web UI **Learning** tab render a
  "Pattern-driven Persona proposals (last cycle)" block.
- Daemon log breadcrumb each cycle:
  `aivyx persona-consolidation: filed N (skipped M)`
  (Phase 80 / 84 / 85 stderr convention).

### Task 5 — Tests + docs + exit

- Integration tests across `reflection_scheduler` (the
  established harness shape from Phase 85): real ledgers +
  Persona chain + mocked LLM provider → proposals filed; both
  dedup arms hit; the cap is respected; one-endpoint-unhelpful
  case skipped; disabled config → no firing (byte-identical to
  pre-Phase-87).
- `docs/INSTALL.md` "Pattern-driven Persona proposals
  (Phase 87)" section (what the actuator does, the conservative
  double-gate, the LLM-summarized prose, the opt-in config,
  the propose-only + edit-then-approve safety net carrying
  over from Phase 70).
- `examples/aivyx.toml` documents the new
  `[persona_consolidation]` block.
- Exit: ROADMAP + PRODUCT_ROADMAP frozen entries, docs/README
  status flip, prediction-vs-reality, hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Source signal:** (a) Pair affinity **AND** both
  endpoints helpful — the conservative double-gate. Mirrors
  Phase 85's evidence-floor discipline; a pattern made of
  topics that individually hurt is never proposed.
- **Q2 — Proposal shape:** (b) LLM-summarized natural-
  language statement, phrased by the reflection LLM. Richer
  operator-facing prose; determinism is relaxed but the Phase
  70 propose-only + edit-then-approve flow keeps the
  *actuator contract* intact (the operator is the final
  filter). Per-candidate LLM error → skip that candidate;
  cycle-wide LLM unavailability → "0 filed, LLM unavailable"
  recorded.
- **Q3 — Cadence + dedup:** (a) Reflection cron + per-cycle
  cap (`max_proposals_per_cycle`) + dual dedup (Persona chain
  + Pending queue). The established cadence for every "act on
  durable learning" pass (77, 82, 83, 85); idempotent across
  cycles.
- **Q4 — Default posture + config:** (a) Opt-in
  `[persona_consolidation]` block, `enabled = false` default,
  with the four knobs above. The Phase 80/81/84 actuator
  posture — every actuator that surfaces operator-visible
  artifacts is opt-in. With no block (or `enabled = false`)
  the pass never runs (byte-identical to pre-Phase-87).

## Deferrals

**Rolling deferrals carried into Phase 87:**

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
- Phase 76 deferrals (heuristic recall gate, token-budget
  context sizing).
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
- Phase 82 deferrals (operator-tunable half-life/retention,
  topic canonicalization).
- Phase 83 deferrals (sequential/temporal patterns, n-ary
  clusters, operator-tunable top-K/half-life).
- Phase 84 deferrals (affinity re-ranking of existing
  candidates, operator-tunable affinity policy).
- Phase 85 deferrals (helpfulness-driven *consolidation* —
  **THIS PHASE**, reflection-facet decay via fuzzy
  embedding).
- Phase 86 deferrals (token-budget context sizing, heuristic
  recall gate, embed-each-and-pool windows, persisted
  windows).

**Likely Phase 87 deferrals:**

- **Pattern-driven Persona *decay*.** The symmetric move on
  the decay side — retire an old facet whose `consolidate-
  pair:{A}+{B}` provenance points at a pair that no longer
  co-occurs — defers. The decay-half of the symmetric arc.
- **n-ary cluster proposals.** v1 only proposes from 2-ary
  affined pairs (the Phase 83 ledger's unit). Triples /
  larger clusters defer with the broader Phase 83 n-ary
  deferral.
- **Pattern-driven supersession.** Editing an *existing*
  facet when its pattern strengthens (vs. filing a new one)
  defers — same shape as the Phase 70 supersession deferral.
- **Operator-tunable LLM prompt.** v1 uses a fixed short
  prompt for facet phrasing; operator-customizable
  consolidation prose defers with Phase 71's
  operator-customizable-reflection-prompt deferral.
- **Topic canonicalization.** `deploy` / `deployment` /
  `deploys` are still distinct pair endpoints. Canonical-
  ization defers from Phase 82's deferral.

## Prediction vs. reality

**Predictions held — all three streaks correct.**

- **DESIGN.md — held.** No locked technical-contract decision
  reopened; a new actuator on existing chains using existing
  signals is exactly the post-Phase-86 ratio. Streak: **34
  consecutive phases** (was 33).
- **PRODUCT.md — held.** No new commitment, none weakened;
  the operator-facing contract was *strengthened* (P14
  Persona now also learns cross-topic *relationships*, not
  only per-topic warmth). Streak: **27 consecutive phases**
  (was 26).
- **`aivyx-core/src/lib.rs` — held, by design.** The new
  pass + config + Phase 78 surface stat all live in
  `aivyx-channel` / `aivyx-config` / `bin/aivyx`; proposals
  land through the existing
  `PersistentPersonaProposalLog::append` API (no new chain
  operation, no new `AuditTag`). Streak: **35 consecutive
  phases** — new project record, beating Phase 86's 34.

**Test count — `+15`** (workspace `1573 → 1588`). At the
upper edge of the predicted `+11-15` band, on the nose. The
breakdown matches the converged calibration law exactly:
new config *section* `+6` (absent / staged-disabled-partial /
enabled-valid / each of the three numeric-bound rejects),
new pure pass module `+8` (id canonicalization; the four
selector rejection arms in one sweep; chain dedup; cap +
order; disabled short-circuit; canonical-id filing; cycle-
wide LLM unavailability flag; quiet-empty-input distinct
from LLM-down), reflection_scheduler integration `+1` (one
end-to-end test exercising filed → idempotent-dedup →
disabled-noop → LLM-down-flag in a single multi-phase
scenario). The integration test does more per test than the
nominal `+1-2` band assumed but lands the count on
prediction — the convergent law continues to hold.

**Scope — every planned surface shipped exactly as scoped.**
The config block + the Q1a conservative double-gate selector +
the Q2b LLM-phrased facet (with both per-candidate skip and
cycle-wide `llm_unavailable` flagging) + the Q3a reflection-
cron cadence with dual dedup + cap + the Q4a opt-in posture
all landed as the open-doc described. The reflection
scheduler signature grew by one optional deps parameter;
every `DaemonConfig` fixture (compat + 7 e2e) got the three
new `None` defaults. Daemon-log breadcrumb (`aivyx
persona-consolidation: schedule "X" — filed N`) + the new
`aivyx learning` block ("Pattern-driven Persona proposals
(last cycle, opt-in)") + the Phase 78 surface stat all
shipped on the established Q4a pattern (`#[serde(default)]`
on the new IPC field, wire-compat preserved). Zero clippy
warnings. Zero new workspace deps.

## Exit criteria

- [x] `[persona_consolidation]` config block: `enabled`
  (default `false`), `min_affinity` (default `1.0`),
  `min_samples` (default `3`), `min_topic_helpfulness`
  (default `0.0`), `max_proposals_per_cycle` (default `3`);
  validation when enabled — Task 2 (commit `8f33288`).
- [x] Pure selector: top-K affined pairs above
  `min_affinity`/`min_samples` with **both endpoints**
  Phase 82-helpful at `>= min_topic_helpfulness`; dual dedup
  against Persona chain + Pending; cap respected — Task 3
  (commit `377dd97`).
- [x] LLM-phrased `learned_context` facet via the existing
  reflection LLM provider; per-candidate LLM error → skip
  that candidate; cycle-wide failure → "0 filed, LLM
  unavailable" — Task 3 (commit `377dd97`).
- [x] Pass wired into `reflection_scheduler` on the existing
  cron; proposals filed via the existing
  `PersistentPersonaProposalLog::append` with `proposal_id =
  "consolidate-pair:{lo}+{hi}"` — Task 3 (commit `377dd97`).
- [x] `PersonaConsolidationStat` + shared handle + Phase 78
  surface ("Pattern-driven Persona proposals (last cycle,
  opt-in)") + daemon-log breadcrumb — Task 4 (commit
  `7c66734`).
- [x] Integration tests: filed, dedup-chain, dedup-pending,
  cap, one-endpoint-unhelpful skip, disabled-byte-identical,
  LLM-unavailable graceful — Task 5 (this commit).
- [x] `docs/INSTALL.md` + `examples/aivyx.toml` updated —
  Task 5 (this commit).
- [x] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 5 (this commit).
- [x] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [x] DESIGN.md streak extends to thirty-four.
- [x] PRODUCT.md streak extends to twenty-seven.
- [x] Production-core streak extends to thirty-five (new
  record) — `lib.rs` byte-identical.
- [x] Test count delta: positive (`+15`, upper edge of the
  predicted `+11-15` band).
- [x] Zero clippy warnings.
- [x] Zero new workspace deps.
- [x] Prediction-vs-reality block filled.
