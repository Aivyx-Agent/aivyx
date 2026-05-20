# Phase 93 — Recall-Judge → Gate Calibration Loop (closing the learning loop)

Phase 90 introduced the heuristic recall gate with per-domain
`min_score` thresholds. Phase 91 introduced `LlmRecallJudge`,
which emits a structured `RecallJudgment` per recall hit:
`helpful` / `unhelpful` / `irrelevant`. Today these two
components don't talk to each other. The gate uses static
thresholds the operator picked at config time; the judge
produces judgments that flow into the audit chain but **no
runtime component reads them back**. The loop is open.

Phase 93 closes the loop. A new per-domain rolling buffer
collects `LlmRecallJudge` verdicts as they're produced. A new
reflection-cron pass (`run_recall_calibration_pass`) reads
the buffer per cycle and nudges the gate's `min_score`
threshold: up when recent judgments above the current cutoff
are mostly unhelpful (the gate is letting too much through);
down when judgments **below** the current cutoff would have
been helpful (the gate is gating too aggressively). With the
new `[recall_gate].enable_calibration` knob off (the default),
the gate is byte-identical to pre-Phase-93.

## Why this, why now

- After Phase 91, the `LlmRecallJudge` produces useful
  per-hit judgments **that no other runtime component
  reads**. The audit trail captures them; the operator can
  manually consult them; nothing in the system actuates on
  them. Phase 93 is the natural symmetry move: judgments
  earn their cost by influencing the gate they were
  generated from.
- After Phase 92's Soul-side actuator deferral closed, the
  recall-side learning loop is the next-most-leveraged open
  surface. The Soul has filed → applied → decay →
  consolidate → supersede. Recall has gather → gate →
  recall → judge **with no learning feedback**. Phase 93
  fills that.
- The change is **purely additive**. With
  `enable_calibration = false` the gate's behaviour is
  byte-identical to Phase 90. The new buffer is in-memory,
  daemon-lifetime, lost on restart — no on-disk schema, no
  IPC contract change at the recall-hit boundary (the
  buffer ingests the existing `RecallJudgment` shape from
  Phase 91 unchanged).
- Reuse is near-total. The reflection-cron orchestration
  already runs Phase 87 phrasing + Phase 91 judgment +
  Phase 92 supersession; the new calibration pass slots in
  alongside them with the same `enable_X = false` opt-in
  posture. The Phase 78 `*Stat` surface pattern absorbs
  the new `RecallCalibrationStat`.

## Streak predictions

- **DESIGN.md** — **Will hold.** Tuning a per-domain
  threshold from operator-validated signal touches no locked
  technical-contract decision. The gate's threshold is
  already a runtime value; the calibrator just nudges it
  in response to the judge's verdicts. No new locked
  contract. Hash at entry:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Prediction: streak **extends to forty** (currently 39).

- **PRODUCT.md** — **Will hold.** The recall pipeline is
  delivered; this strengthens its self-improving posture
  without changing any operator-facing commitment. P9
  (recall) and P15 (learning loop, if any commitments
  exist) are not weakened; the per-domain gate continues
  to be operator-tunable via `[recall_gate]` config and now
  *also* self-calibrates when the operator opts in. Hash
  at entry:
  `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.
  Prediction: streak **extends to thirty-three** (currently
  32).

- **Production-core `aivyx-core/src/lib.rs`** — **Will
  hold, by design.** The buffer + the pure `calibrate(...)`
  function + the reflection-scheduler pass + the config
  knob + the recording integration all live in
  `aivyx-channel` / `aivyx-config`. The gate config lives
  in `aivyx-config` already; the `RecallJudgment` type
  lives in `aivyx-channel` (Phase 91). No `aivyx-core`
  touch; no new `AuditTag`; no new `KeyDomain`. Hash at
  entry:
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.
  Prediction: streak **extends to forty-one** consecutive
  phases (new project record, beats Phase 92's 40).

- **New workspace deps** — Zero. The calibrator is pure
  arithmetic over the `RecallJudgment` shape; the buffer
  is a `VecDeque` from `std`.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_93.md` + `docs/README.md` status row.

### Task 2 — `[recall_gate].enable_calibration` knob

`aivyx-config`:

- `RecallGateConfig` gains
  `enable_calibration: bool` (default `false`). With
  `false` the recording side is a no-op and the
  calibration pass short-circuits — byte-identical to
  pre-Phase-93.
- `RawRecallGate` + the build path. Validation: trivial
  (boolean).
- A second knob `calibration_buffer_size: usize` (default
  32) — the rolling buffer's per-domain capacity. Bounded
  in `(0, 1024]` to prevent pathological config.
- A third knob `calibration_nudge_step: f32` (default
  `0.02`) — the maximum threshold delta per pass. Bounded
  in `(0.0, 0.25]`.
- Tests: defaults; explicit values win; buffer-size
  bounds; nudge-step bounds; absent section builds None
  (no calibration); any-field-set builds Some.

### Task 3 — `RecallJudgmentBuffer` + pure `calibrate(...)` (the crux)

`aivyx-channel`:

- New `recall_calibration` module with:
  - `pub struct RecallJudgmentBuffer { per_domain:
    HashMap<KeyDomain, VecDeque<RecordedJudgment>>, cap:
    usize }` where `RecordedJudgment { score: f32,
    verdict: RecallVerdict }` captures the heuristic score
    the hit entered at + the LLM verdict.
  - `pub fn record(&mut self, domain: KeyDomain, score:
    f32, verdict: RecallVerdict)` appends + truncates to
    `cap` from the front.
  - `pub fn calibrate(buffer: &VecDequeView, threshold:
    f32, nudge_step: f32) -> CalibrationOutcome` where
    `CalibrationOutcome { new_threshold: f32, direction:
    Direction, applied: bool }`. **Rule (Q2a):** among
    judgments **above** the current threshold, if
    `unhelpful + irrelevant` ≥ 60% of N, nudge threshold
    *up* by `min(nudge_step, gap_to_lowest_unhelpful)`;
    among judgments **below** the threshold, if `helpful`
    ≥ 60% of N, nudge *down* by `min(nudge_step,
    gap_to_highest_helpful)`. Bounded in `[0.0, 1.0]`. If
    both sides have signal, the larger-evidence side wins
    deterministically.
- Recording integration: the `RecallJudge::judge(...)`
  result path in `recall_feedback.rs` records into the
  buffer when `enable_calibration = true` (gated lookup;
  no-op when off). The recording side carries the
  heuristic score the hit entered the gate with — that's
  already known at recall time.
- Unit tests on the pure function: above-threshold-mostly-
  unhelpful nudges up; below-threshold-mostly-helpful
  nudges down; balanced signal doesn't move; nudge clamps
  at `nudge_step`; nudge clamps at the bounds `[0.0,
  1.0]`; under-N buffer (insufficient evidence) doesn't
  move; identical-buffer second call is idempotent (same
  threshold in → same threshold out → same direction).
- Buffer tests: under-cap appends; at-cap rotates oldest;
  per-domain isolation.

### Task 4 — Reflection-scheduler integration

`aivyx-channel/src/reflection_scheduler.rs`:

- New `run_recall_calibration_pass` that iterates per
  `KeyDomain` in the buffer, calls `calibrate(...)` per
  domain, applies the new threshold to the in-memory
  `RecallGateConfig` snapshot (the gate config is loaded
  per recall call, so the in-memory mutation is visible
  immediately to the next recall), and accumulates
  per-domain outcomes into a new `RecallCalibrationStat
  { calibrated: u32, raised: u32, lowered: u32, held:
  u32 }`.
- `PersonaConsolidationStat`-style IPC wire-compat via
  `#[serde(default)]` on the new fields. The stat surfaces
  through the existing Phase 78 `learning` IPC + CLI
  surface.
- `daemon_server.rs` threads the calibration deps into
  `RecallCalibrationDeps` (buffer handle + gate config
  handle + `nudge_step` + `buffer_size`).
- Integration test: seed the per-domain buffer with eight
  unhelpful-above-threshold judgments at scores
  `[0.85..0.95]` against a threshold of `0.80`; run one
  calibration pass; assert the in-memory threshold moved
  up by exactly `nudge_step` (clamped at the lowest
  unhelpful score's gap, which is the smaller of the
  two); `stat.raised == 1, stat.calibrated == 1`. Run a
  second pass against the same buffer; assert byte-
  identical state (same outcome, idempotent because the
  threshold has caught up to the gap).

### Task 5 — Surface + tests + docs + exit

- `aivyx learning` render block extended with the new
  `RecallCalibrationStat`: a line under the existing
  recall section showing `N calibrated last cycle (R
  raised, L lowered, H held)`. When the count is 0 (the
  default-off state for almost every operator at first)
  the line is omitted to avoid noise.
- `docs/INSTALL.md` — new "Recall-gate calibration loop
  (Phase 93)" subsection under the existing Phase 90
  recall-gate section: the recording side, the rolling
  buffer, the threshold-nudging rule, the opt-in knob,
  the bounded nudge step, the in-memory-only buffer
  (lost on restart), the operator-tunable buffer size.
- `examples/aivyx.toml` — document the new
  `enable_calibration`, `calibration_buffer_size`,
  `calibration_nudge_step` keys alongside the existing
  `[recall_gate]` block.
- Exit: ROADMAP + PRODUCT_ROADMAP frozen entries,
  docs/README status flip, prediction-vs-reality, hash
  backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Signal:** (a) Per-domain rolling buffer of last
  N judgments (default N=32, operator-tunable in `(0,
  1024]`). Bounded memory, simple aggregation math, no
  decay-rate hyperparameter, no on-disk schema. The
  in-memory variant can graduate to disk in a later
  phase if operators want survival across daemon
  restart; for v1 the loop converges quickly enough that
  loss-on-restart is acceptable.
- **Q2 — Knob:** (a) The existing per-domain `min_score`
  threshold. Reuses the Phase 90 gate's own knob; no
  new gate-internal parameters. One axis of motion,
  well-understood semantics, easy to revert by clearing
  the buffer or toggling the knob off (the threshold
  snaps back to the configured value on daemon restart).
- **Q3 — Cadence:** (a) Reflection cron + `enable_calibration
  = false` default. Matches the Phase 87 / 88 / 91 / 92
  actuator opt-in pattern. The calibration pass runs
  alongside the other reflection passes (phrasing,
  judging, supersession) on the same cron schedule.
- **Q4 — Test:** (a) One-cycle nudge + idempotency.
  Seed the buffer with mostly-unhelpful judgments above
  threshold; assert the calibrator moves the threshold up
  by the expected amount in one pass; assert a second
  pass against the same buffer produces byte-identical
  state. Matches the Phase 87 / 92 integration-test
  shape; doesn't couple the test to the calibrator's
  precise long-run convergence behaviour.

## Deferrals

**Rolling deferrals carried into Phase 93:**

- v0.1.0 publication (Phase 61 Task 7).
- System-prompt notification-target enumeration (Phase 62).
- `turn_id` correlation refactor (Phase 67).
- Phase 68 deferrals (XOAUTH2, HTML email, attachments,
  multi-account `[email]`).
- Phase 69 deferrals (WebPush / service-worker, notification
  urgency, icons, sound).
- Phase 70 deferrals (multi-window reflection).
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
- Phase 77 deferrals (`[recall_feedback]` tuning knob).
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
- Phase 87 deferrals (n-ary cluster proposals, operator-
  tunable LLM prompt).
- Phase 88 deferrals (n-ary cluster decay, pair-affinity
  hysteresis).
- Phase 89 deferrals (operator-tunable `[[topic_alias]]`
  mappings, topic-by-topic exception list, one-time
  migration of existing fragmented data, non-ASCII /
  Unicode stemming).
- Phase 90 deferrals (pattern-based stoplist, LLM-judged
  gate — **partially addressed THIS PHASE via threshold
  calibration**, adaptive thresholds — **THIS PHASE**,
  token-budget context sizing).
- Phase 91 deferrals (**actuator-side switch from
  structural proxy to the new judgment signal — THIS
  PHASE**, per-recall LLM critique, adaptive batch size,
  multi-model ensembling, response-text recovery via
  audit-chain extension).
- Phase 92 deferrals (Web UI visual grouping of linked
  supersession proposals, atomic chain-level supersession
  primitive, n-ary cluster supersession, semantic-
  similarity supersession).

**Likely Phase 93 deferrals:**

- **On-disk persistence of the per-domain buffer.** v1's
  in-memory buffer is lost on daemon restart; the
  threshold reverts to the configured value and the loop
  warms up again. Operators with long calibration runs
  may want survival across restart — defers as a Phase
  78/79 ledger-style follow-up.
- **Persisted threshold writes.** v1 mutates the in-memory
  gate config only; the persisted `[recall_gate]` config
  on disk is unchanged. The operator's hand-set
  thresholds are preserved on restart; the calibrated
  values are not. A future phase could optionally write
  calibrated values back to disk (with operator opt-in,
  per-domain audit trail).
- **Non-linear nudge rules.** v1's bounded-step linear
  nudge is conservative. A future phase could let the
  step size react to evidence strength (e.g., nudge
  faster when 90% of buffer agrees, slower at 60%).
- **Cross-domain regularization.** v1 calibrates each
  domain independently. Domains with sparse signal could
  borrow strength from related domains — defers as a
  speculative extension.
- **Per-recall LLM critique of the calibration decision.**
  An LLM seam reviewing the calibrator's chosen direction
  before applying it — symmetric to Phase 87's
  `PairPhraser` seam on writes. Defers; the linear rule
  is interpretable enough for v1.

## Prediction vs. reality

To be filled in at phase exit.

## Exit criteria

- [ ] `[recall_gate].enable_calibration: bool` (default
  `false`) + `calibration_buffer_size: usize` (default
  32, bounded `(0, 1024]`) + `calibration_nudge_step: f32`
  (default `0.02`, bounded `(0.0, 0.25]`) — Task 2.
- [ ] `RecallJudgmentBuffer` + pure `calibrate(...)` in
  a new `recall_calibration` module; recording integration
  into the `RecallJudge::judge(...)` result path — Task 3.
- [ ] Unit tests on the pure calibrator: above-threshold-
  mostly-unhelpful nudges up; below-threshold-mostly-
  helpful nudges down; balanced doesn't move; nudge
  clamps at step; nudge clamps at bounds; under-N
  doesn't move; idempotent on identical input — Task 3.
- [ ] Buffer tests: under-cap; at-cap rotation; per-domain
  isolation — Task 3.
- [ ] `run_recall_calibration_pass` runs alongside the
  Phase 87 / 91 / 92 reflection passes; applies threshold
  to in-memory gate config — Task 4.
- [ ] `RecallCalibrationStat { calibrated, raised,
  lowered, held }` with IPC wire-compat — Task 4.
- [ ] Integration test: one-cycle nudge in the expected
  direction by the expected amount; second cycle byte-
  identical (idempotent) — Task 4.
- [ ] `aivyx learning` surface extended — Task 5.
- [ ] `docs/INSTALL.md` + `examples/aivyx.toml` updated —
  Task 5.
- [ ] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 5.
- [ ] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [ ] DESIGN.md streak extends to forty.
- [ ] PRODUCT.md streak extends to thirty-three.
- [ ] Production-core streak extends to forty-one (new
  record) — `lib.rs` byte-identical.
- [ ] Test count delta: positive (~+8-12; per the
  converged calibration law — knob on existing block
  (≈ +3-4, three knobs) + new pure module (≈ +7-9,
  buffer + calibrate + their failure modes) + recording
  integration (≈ +1) + reflection integration (≈ +1);
  no new `KeyDomain`).
- [ ] Zero clippy warnings.
- [ ] Zero new workspace deps.
- [ ] Prediction-vs-reality block filled.
