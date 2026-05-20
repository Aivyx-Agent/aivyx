# Phase 91 — LLM-Judged Recall Usefulness (the missing half of the learning loop)

For 90 phases the recall-feedback signal (Phase 77) has been
**structural**: a recall is judged helpful iff the turn it
participated in completed cleanly without an immediate
operator re-ask, and unhelpful iff the turn failed or timed
out. Every downstream accumulator — the Phase 82 helpfulness
ledger, the Phase 83 co-occurrence ledger, Phase 85's
helpfulness-driven Persona decay, Phase 87's pattern-driven
proposals, Phase 88's pattern-driven decay — consumes this
proxy. The proxy works, but it operates at **turn-level
granularity** (every recall in a successful turn inherits
`+1`; every recall in a failed turn inherits `-1`) and
cannot distinguish "the recall was actively used by the
model" from "the recall was irrelevant but the turn
succeeded anyway."

Phase 91 closes the longest-running feedback-side deferral
(Phase 77, carried forward 14 phases through 78-90) by
adding an **LLM-judged per-recall classification** alongside
the existing structural signal. After Phases 86 / 89 / 90
sharpened input quality, Phase 91 sharpens the symmetric
half — feedback quality.

|                                                  | Layer that's sharpened |
|--------------------------------------------------|------------------------|
| **Phase 86** — Conversational window             | *What* gets embedded   |
| **Phase 89** — Topic canonicalization            | *How* signals key      |
| **Phase 90** — Heuristic recall gate             | *When* recall fires    |
| **Phase 91** — LLM-judged recall usefulness (this) | *Whether* it actually helped |

The learning loop's quality picture is complete after Phase
91: sharper input + sharper feedback.

## Why this, why now

- It is the longest-running feedback-side deferral
  (Phase 77, 14 phases old). Every recall consumer has
  paid the turn-level-proxy tax silently.
- The seam is established. Phase 87 introduced the first
  LLM-on-reflection-path call (`LlmPairPhraser` for
  consolidation proposals); Phase 91 adds a second
  reflection-time LLM call (`RecallJudge` for recall
  classification) using the same reflection-cron cadence
  + the same shared LLM provider on `DaemonConfig`.
- The change is **byte-identical-additive** (Q3a). The new
  per-recall classification is recorded on the recall log
  as a new field; every existing accumulator (Phase 82 / 83
  / 85 / 87 / 88) stays unchanged in v1. A future phase
  consumes the new signal (A/B compare against the
  structural proxy or switch). Maximum safety: shipping
  Phase 91 changes no existing behavior.
- Cost is bounded by Q1a: one LLM call per reflection cron
  tick, batched across every recall event in the window.
  Same shape Phase 87 already pays.

## Streak predictions

- **DESIGN.md** — **Will hold.** Adding a new optional
  field on the existing `RecallHit` IPC type + a new
  optional pass to the reflection scheduler touches no
  locked technical-contract decision. The recall log
  already evolves with `#[serde(default)]`-tolerant
  wire-compat (Phase 84 added `cluster: bool` the same
  way). Hash at entry:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Prediction: streak **extends to thirty-eight** (currently
  37).

- **PRODUCT.md** — **Will hold.** P14 (Persona) +
  G3 (recall) already shipped; this adds a higher-quality
  feedback signal that future phases will consume.
  No new commitment, none weakened; the operator-facing
  contract is *strengthened* (the learning loop will
  eventually produce sharper helpfulness scores, sharper
  co-occurrence affinities, sharper decay decisions). No
  commitment-text edit. Hash at entry:
  `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.
  Prediction: streak **extends to thirty-one** (currently
  30).

- **Production-core `aivyx-core/src/lib.rs`** — **Will
  hold, by design.** The `RecallEvent` / `RecallHit`
  types live in `aivyx-channel/src/recall_log.rs`, not
  `aivyx-core`. The new `RecallJudge` trait + `LlmRecallJudge`
  adapter + the reflection-scheduler integration all
  live in `aivyx-channel`. The new config block is in
  `aivyx-config`. No `aivyx-core` touch; no new `AuditTag`
  (the judgment is recorded on the existing recall log,
  not on the audit chain). Hash at entry:
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.
  Prediction: streak **extends to thirty-nine** consecutive
  phases (new project record, beats Phase 90's 38).

- **New workspace deps** — Zero. The 3-way classification
  decode is a tiny serde enum or hand-rolled string
  parser; the LLM call reuses `aivyx_llm::LlmProvider` the
  same way Phase 87's `LlmPairPhraser` does.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_91.md` + `docs/README.md` status row.

### Task 2 — `[recall_judgment]` config block

`aivyx-config`:

- New top-level `RecallJudgmentConfig`:
  - `enabled: bool` (default `false` — opt-in, the Phase
    80/81/84/87 actuator-block precedent).
  - `max_recalls_per_cycle: u32` (default `30`) — hard
    upper bound on how many recall events the batched LLM
    call may judge in one cron tick. Past this cap, the
    oldest unjudged recalls in the window are skipped (a
    record on the stat surface notes the skip; nothing
    fails).
- `RawRecallJudgment` deserialize target; validation when
  `enabled = true`: `max_recalls_per_cycle >= 1`.
- Tests: absent → `None`; staged-disabled allowed partial;
  enabled-valid; `max_recalls_per_cycle = 0` rejects.

### Task 3 — `RecallJudge` trait + judgment shape (the crux)

`aivyx-channel`:

- New `recall_judgment` module:
  - `pub enum RecallJudgment { Used, Irrelevant, Hurt }`
    — serde-derived for IPC + recall-log persistence.
    Stable string labels (`"used"`, `"irrelevant"`,
    `"hurt"`) for the JSON wire format.
  - `RecallHit` gains an optional field:
    `judgment: Option<RecallJudgment>` with
    `#[serde(default, skip_serializing_if = "Option::is_none")]`
    so the existing recall log decodes unchanged AND old
    `RecallEvent` JSON blobs stay valid. The `cluster:
    bool` Phase 84 added is the exact wire-compat
    precedent.
  - `pub trait RecallJudge: Send + Sync` with one
    method:
    ```rust
    async fn judge(
        &self,
        recalls: &[RecallJudgeInput],
    ) -> Vec<Option<RecallJudgment>>;
    ```
    where `RecallJudgeInput { recalled_topic, recalled_body,
    response_text }`. Returns one judgment per input in
    order; `None` for an individual recall on per-item
    parse failure (cycle-wide failure → all `None`).
  - `LlmRecallJudge` production adapter (mirrors
    `LlmPairPhraser` from Phase 87): holds
    `Arc<dyn LlmProvider>` + a model name; constructs a
    batched prompt asking the LLM to classify each
    `(topic, body, response)` triple into the 3-way
    enum; parses the response back into the
    `Vec<Option<RecallJudgment>>` shape.
- Unit tests on the parser + the trait shape: each
  enum variant round-trips through serde; the
  `Option<RecallJudgment>` recall-log field
  deserializes from old (missing-field) JSON; the
  `LlmRecallJudge` adapter calls its inner provider once
  per `judge(…)` call regardless of batch size (one LLM
  call per cycle).

### Task 4 — Reflection-scheduler integration

`aivyx-channel/src/reflection_scheduler.rs`:

- New `RecallJudgmentDeps` struct bundling:
  - `config: RecallJudgmentConfig`
  - `recall_log: Arc<PersistentRecallLog>` (already in
    the daemon)
  - `audit_log: Arc<PersistentAuditLog>` (for the model
    response text — pulled from the matching
    `TurnEnded.final_message` if available, otherwise
    skipped)
  - `memory: Arc<dyn Memory>` (for the recalled body —
    `get_recent(topic, n)` to recover the entry the
    recall event referenced; best-effort)
  - `judge: Arc<dyn RecallJudge>`
  - `stat: Option<SharedRecallJudgmentStat>` (the
    Phase 78 surface stat — see Task 5)
- New `run_recall_judgment_pass(deps, sched, now_ms)`
  alongside the existing
  `run_recall_feedback_pass` / `run_proactive_pass` /
  `run_persona_lifecycle_pass` /
  `run_persona_consolidation_pass`:
  1. Read every recall event in the lookback window
     whose `judgment` field is currently `None`.
  2. Cap to `max_recalls_per_cycle` (oldest-first; the
     rest are skipped this cycle).
  3. For each, recover the model response text from
     audit + the recall body from memory. Skip rows
     where either lookup fails (best-effort; no
     judgment recorded for that recall).
  4. Call the batched `RecallJudge::judge(...)` once.
  5. Append per-recall judgments back into the recall
     log via a new `PersistentRecallLog::patch_judgment(
     seq, judgment)` method (or rewrite the row;
     implementation-detail in Task 4).
- Wire into `run_reflection_scheduler` + `fire_reflection`
  alongside the existing five passes.
- Binary wiring: `bin/aivyx` builds a `RecallJudgmentDeps`
  iff the section is enabled, the recall log + audit log
  + memory all exist, and an `LlmRecallJudge` is
  constructable (LlmProvider available); else `None`.

### Task 5 — Phase 78 surface + tests + docs + exit

- `RecallJudgmentStat { ts_secs, judged, used, irrelevant,
  hurt, skipped, llm_unavailable, pairs: Vec<(topic,
  RecallJudgment)> }` — the per-cycle digest. Shared
  handle on DaemonConfig + handle_query + IPC wire format
  (same shape as Phase 87's `PersonaConsolidationStat`).
  `aivyx learning` render block; daemon-log breadcrumb.
- Integration test: a recall event without judgment +
  a mock judge that returns `Used` → cycle 1 records
  the judgment; cycle 2 is idempotent (the same event
  is no longer in the unjudged set). Disabled config →
  no-op. LLM unavailable → `llm_unavailable = true`
  flag flips, all recalls remain unjudged.
- `docs/INSTALL.md` — "LLM-judged recall usefulness
  (Phase 91)" section under the existing Phase 77
  recall-feedback section.
- `examples/aivyx.toml` — document the new
  `[recall_judgment]` block.
- Exit: ROADMAP + PRODUCT_ROADMAP frozen entries,
  docs/README status flip, prediction-vs-reality, hash
  backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Cadence:** (a) Reflection cron, batched per
  cycle. One LLM call per cron tick judging every recall
  in the window. Bounded cost; reuses Phase 71 cron
  infrastructure; the Phase 87 LLM-on-reflection-path
  precedent.
- **Q2 — Output:** (a) 3-way structured classification
  per recall: `USED` / `IRRELEVANT` / `HURT`. Easy to
  aggregate to existing `+1 / 0 / -1` helpfulness shape;
  deterministic decode; trivially mockable in tests.
- **Q3 — Combine:** (a) Augment — record the LLM
  judgment as a new optional field on `RecallHit`. Every
  existing accumulator (Phase 82 / 83 / 85 / 87 / 88)
  stays byte-identical in v1; a future phase consumes
  the new signal. Maximum safety: shipping Phase 91
  changes no existing behavior; the new data is
  captured for future use + inspection on the Phase 78
  surface.
- **Q4 — Default:** (a) Opt-in via
  `[recall_judgment].enabled`, default `false`. The
  LLM call has real cost; the operator opts into
  paying it. Matches the 90-phase
  behaviour-change-is-opt-in (and now cost-is-opt-in)
  discipline.

## Deferrals

**Rolling deferrals carried into Phase 91:**

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
- Phase 76 deferrals (token-budget context sizing).
- Phase 77 deferrals (`[recall_feedback]` tuning knob;
  **LLM-judged recall usefulness — THIS PHASE**).
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
- Phase 90 deferrals (pattern-based stoplist, LLM-judged
  gate, adaptive thresholds, token-budget context
  sizing).

**Likely Phase 91 deferrals:**

- **Actuator-side switch to LLM-judged signal.** Phase 91
  ships Q3a (augment) — every existing accumulator stays
  byte-identical. A future phase consumes the new
  judgment field (replace, or weighted-average with the
  structural proxy, or confirm-only). Deferred pending
  operator validation that the LLM judgment quality is
  good enough to drive identity actions.
- **Per-recall LLM critique.** Q2a chose structured 3-way;
  the free-form text critique (Q2b) defers as a Phase 78
  surface enrichment.
- **Adaptive batch size.** `max_recalls_per_cycle` is a
  fixed config knob; tuning from observed LLM latency /
  cost / quality defers.
- **Multi-model ensembling.** A consensus of two LLM
  judges (different models) is more robust than one.
  Defers — significant cost multiplier; the structural
  proxy is already the cross-check.

## Prediction vs. reality

To be filled in at phase exit.

## Exit criteria

- [ ] `[recall_judgment]` config block: `enabled` (default
  `false`), `max_recalls_per_cycle` (default `30`);
  validation when armed — Task 2.
- [ ] `RecallJudgment { Used, Irrelevant, Hurt }` enum
  with stable serde labels — Task 3.
- [ ] `RecallHit` gains
  `judgment: Option<RecallJudgment>` with
  `#[serde(default, skip_serializing_if = "Option::is_none")]`;
  old recall log decodes unchanged — Task 3.
- [ ] `RecallJudge` trait + `LlmRecallJudge` adapter
  (one batched LLM call per `judge(…)`) — Task 3.
- [ ] Reflection-scheduler pass:
  `run_recall_judgment_pass` reads unjudged events in
  the window (oldest-first up to
  `max_recalls_per_cycle`), recovers response + body
  text, calls the judge once, patches each judgment
  back to the recall log — Task 4.
- [ ] `bin/aivyx` builds `RecallJudgmentDeps` iff
  the section is enabled, every substrate is present,
  and an `LlmRecallJudge` is constructable — Task 4.
- [ ] Phase 78 surface (`RecallJudgmentStat` + shared
  handle + IPC wire field + `aivyx learning` render +
  daemon-log breadcrumb) — Task 5.
- [ ] Integration test (idempotent across cycles;
  disabled config no-op; LLM-unavailable flag flips) —
  Task 5.
- [ ] `docs/INSTALL.md` + `examples/aivyx.toml` updated —
  Task 5.
- [ ] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 5.
- [ ] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [ ] DESIGN.md streak extends to thirty-eight.
- [ ] PRODUCT.md streak extends to thirty-one.
- [ ] Production-core streak extends to thirty-nine (new
  record) — `lib.rs` byte-identical.
- [ ] Test count delta: positive (~+9-15; per the
  converged calibration law — new config section
  (≈ +5-6) + new pure module (≈ +3-4) + reflection-pass
  integration (≈ +1-2) + IPC wire-compat assertion
  (≈ +1); no new `KeyDomain` (the recall log
  already owns this domain).
- [ ] Zero clippy warnings.
- [ ] Zero new workspace deps.
- [ ] Prediction-vs-reality block filled.
