# Phase 93 — Recall-Feedback Switches to LLM-Judgment Signal (closing the Phase 91 deferral)

Phase 91 introduced `LlmRecallJudge` and added the per-hit
`judgment: Option<RecallJudgment>` field to `RecallHit`. The
judge populates it via the `run_recall_judgment_pass`
reflection-cron pass. But the field was scoped as **v1
augment, not replace** — the comment on the field's
definition states the policy explicitly: *"No existing
accumulator (Phase 82 / 83 / 85 / 87 / 88) consumes this
field in v1."* The judgments flow into the audit chain and
the Phase 78 surface; no actuator reads them.

Phase 93 closes that loop. The Phase 91 deferral
documented in `docs/PHASE_92.md` line-for-line ("actuator-
side switch from structural proxy to the new judgment
signal") is shipped: `correlate_detailed` — the single
source of truth that produces the `HelpfulnessTally` both
the memory-promotion (Phase 77 Task 6) and Persona-
proposal (Phase 77 Task 7) actuators read — now consults
the per-hit `judgment` field where present and falls back
to the existing turn-level structural proxy where absent.

With the new `[recall_feedback].use_judgment_signal = false`
knob (the default), `correlate_detailed` is byte-identical
to pre-Phase-93. With the knob on, **per-hit overrides
turn-uniform**: a hit judged `Used` contributes `+WEIGHT`
regardless of the turn's structural signal, a hit judged
`Hurt` contributes `-WEIGHT`, a hit judged `Irrelevant`
contributes `0`, and any un-judged hit (`None`) keeps the
turn-level structural signal it already had. The downstream
actuators read the same `HelpfulnessTally` shape; only the
signal source changes.

## Why this, why now

- The Phase 91 deferral is exactly one phase old and was
  named verbatim in the Phase 92 open doc's "rolling
  deferrals" list. The judge produces verdicts that
  currently flow only to visibility surfaces (audit chain
  + Phase 78 insights); no runtime actuator consumes
  them. Closing this is the natural symmetry move.
- Surface area is tiny. One knob, one function-body
  change inside `correlate_detailed`, no new module, no
  new IPC contract, no new `KeyDomain`. The
  `HelpfulnessTally` shape is unchanged; downstream
  actuators read it identically.
- The change is **purely additive**. With
  `use_judgment_signal = false` (the default), the
  correlator's behaviour is byte-identical to Phase 77.
  Operators who haven't enabled the Phase 91 judge see no
  judgments to consume; operators who have enabled the
  judge for visibility-only see no actuator behaviour
  change unless they also flip this new knob.
- Reuse is total. The judgment field already exists; the
  reflection-cron pass already populates it; the
  `correlate_detailed` call sites already exist; the
  config block already exists. Phase 93 just connects two
  already-built pieces.

## Streak predictions

- **DESIGN.md** — **Will hold.** Switching the per-hit
  signal source from a structural proxy to an LLM
  classification touches no locked technical-contract
  decision. The signal magnitude (`±WEIGHT`), the tally
  shape, the actuator contracts, the audit-chain shape —
  all preserved. Hash at entry:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Prediction: streak **extends to forty** (currently 39).

- **PRODUCT.md** — **Will hold.** P9 (recall) and the
  Persona-actuator commitment are unchanged; the
  operator-facing behaviour with the knob off is byte-
  identical, and with the knob on the only observable
  change is "memory promotion and Persona proposals
  reflect the LLM's per-hit assessment rather than the
  structural turn-level proxy" — a *strengthening*, not a
  weakening, of the existing commitment. Hash at entry:
  `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.
  Prediction: streak **extends to thirty-three** (currently
  32).

- **Production-core `aivyx-core/src/lib.rs`** — **Will
  hold, by design.** All changes land in `aivyx-channel`
  (`recall_feedback.rs` augmentation + recall-log already
  has the field) and `aivyx-config` (the new knob). No
  `aivyx-core` touch; no new `AuditTag`; no new
  `KeyDomain`. Hash at entry:
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.
  Prediction: streak **extends to forty-one** consecutive
  phases (new project record, beats Phase 92's 40).

- **New workspace deps** — Zero. The augmentation is pure
  arithmetic over the `RecallJudgment` enum.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_93.md` + `docs/README.md` status row.

### Task 2 — `[recall_feedback].use_judgment_signal` knob

`aivyx-config`:

- New `RecallFeedbackConfig` block (or extension of the
  existing one if present) with
  `use_judgment_signal: bool` (default `false`). The
  block goes under `[recall_feedback]`; the producer
  (`[recall_judgment]`) and consumer (`[recall_feedback]`)
  knobs stay cleanly separated.
- `RawRecallFeedback` + the build path. Validation:
  trivial (boolean).
- Tests: default false; explicit true wins; absent block
  builds None / honors default; any-field-set builds
  Some.

### Task 3 — Augment `correlate_detailed` (the crux)

`aivyx-channel/src/recall_feedback.rs`:

- `correlate_detailed` (and its thin wrapper `correlate`)
  gains a `use_judgment_signal: bool` parameter. The
  per-hit signal derivation becomes:
  1. If `use_judgment_signal = true` AND
     `hit.judgment.is_some()`: derive the hit's signal
     from the verdict per **Q2**'s mapping —
     `Used → +WEIGHT`, `Hurt → -WEIGHT`, `Irrelevant → 0`
     (no signal accumulated).
  2. Otherwise: fall back to the turn-level structural
     signal (the existing `turn_signal(outcome, all)`
     contribution applied uniformly to the hit).
- The `RecallContribution` struct gains a per-hit
  signal-source breakdown — a new field
  `signal_source: SignalSource { Structural, Judgment }`
  per hit (or carried on the contribution if all hits
  agree). The Phase 78 insights surface reads this so
  the operator can see *why* a hit got the signal it
  did. Wire-compat: `#[serde(default)]` if the contrib
  is serialized (it isn't currently — pure
  in-process); marker only.
- Unit tests on the augmented logic — **single mixed
  fixture per Q4a**:
  - Recall with three hits — `Used` / `Hurt` / un-judged
    — on a turn whose structural signal is `+WEIGHT`.
    With `use_judgment_signal = false`: all three
    accumulate `+WEIGHT`. With `true`: `Used` accumulates
    `+WEIGHT` (from verdict), `Hurt` accumulates
    `-WEIGHT` (from verdict overriding structural),
    un-judged accumulates `+WEIGHT` (from structural
    fallback).
  - Single-hit case: `Irrelevant` with knob on
    accumulates `0` (no contribution); knob off
    accumulates the structural `+WEIGHT`.
  - Knob-on with a turn whose structural signal is `None`
    (escalated/cancelled): un-judged hits still
    accumulate nothing; `Used` / `Hurt` judgments still
    fire (judgment runs even when structural is silent).
  - Knob-off back-compat: byte-identical to pre-Phase-93
    behaviour (one or two existing-shape regression tests
    pinned).

### Task 4 — Threading + integration

`aivyx-channel`:

- Every call site of `correlate_detailed` /
  `correlate` is updated to thread the boolean from
  `[recall_feedback]` config. Call sites:
  - `reflection_scheduler` (the recall-feedback-pass that
    drives the actuators).
  - `recall_insights` (the Phase 78 surface — reads the
    same correlation so the operator sees consistent
    numbers).
  - `recall_feedback` internal tests (pin the existing
    behaviour explicitly).
- `daemon_server.rs` threads the config bool into the
  reflection-scheduler deps.
- Integration test: a reflection-cron cycle with three
  recall events whose hits carry mixed judgments;
  assert the resulting `HelpfulnessTally` matches the
  per-hit verdict-driven sum (not the turn-level
  structural sum). One test, mirroring the Phase 91/92
  integration-test shape.

### Task 5 — Surface + tests + docs + exit

- `aivyx learning` render block: extend the existing
  recall-feedback section with the source split
  (e.g., `N entries scored: J judgment-driven, S
  structural`). When the knob is off the line falls back
  to the existing `N entries scored` shape.
- `docs/INSTALL.md` — new "Judgment-driven recall
  feedback (Phase 93)" subsection under the existing
  Phase 91 `[recall_judgment]` section: explains the
  augment semantics, the `Used`/`Hurt`/`Irrelevant`
  mapping, the structural fallback for un-judged hits,
  the opt-in knob, the consequence on memory-promotion
  + Persona-proposal actuators.
- `examples/aivyx.toml` — document
  `[recall_feedback].use_judgment_signal = true` (with
  the canonical default-off comment).
- Exit: ROADMAP + PRODUCT_ROADMAP frozen entries,
  docs/README status flip, prediction-vs-reality, hash
  backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Posture:** (a) **Augment.** Per-hit `Some(judgment)`
  overrides the turn-level structural signal for that
  specific hit; un-judged (`None`) hits fall back to the
  turn-level proxy. Smooth migration — every un-judged
  hit continues to work exactly as before; judgments
  take effect incrementally as the Phase 91 cron
  processes them. Matches the Phase 91 field doc's own
  framing ("v1 augment, not replace").
- **Q2 — Mapping:** (a) `Used → +WEIGHT`,
  `Hurt → -WEIGHT`, `Irrelevant → 0` (no contribution).
  Symmetric with the existing structural mapping; single
  source of magnitude; irrelevant hits are dead weight,
  not negative signal (the entry isn't punished for being
  surfaced on a topic-tangential turn).
- **Q3 — Knob:** (a) New `[recall_feedback].use_judgment_signal:
  bool`, default `false`. Matches the Phase 87/88/91/92
  actuator opt-in pattern. With the knob off the
  correlator is byte-identical to pre-Phase-93. The knob
  lives on the consumer side (`[recall_feedback]`), not
  the producer side (`[recall_judgment]`), preserving the
  clean producer/consumer separation.
- **Q4 — Test:** (a) Single mixed-fixture test against
  `correlate_detailed` — three hits (Used / Hurt / un-
  judged) on a turn with `+WEIGHT` structural signal,
  exercising knob-off / knob-on / fallback / Irrelevant
  cases in one fixture. Matches the Phase 91/92
  integration-test shape.

## Deferrals

**Rolling deferrals carried into Phase 93** (Phase 91's
actuator-switch deferral is **THIS PHASE**):

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
- Phase 77 deferrals (`[recall_feedback]` tuning knob —
  **THIS PHASE introduces the first knob to that block**).
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
  gate, adaptive thresholds, token-budget context sizing).
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

- **Per-domain or per-topic weights.** Q2d's operator-
  tunable per-variant weights defer. The fixed
  `±WEIGHT`/`0` mapping is sufficient for v1; tuning the
  magnitudes belongs to a later phase that can also
  defend the chosen values empirically.
- **Replace mode.** Q1b's hard switch defers — the
  augment posture is strictly more conservative and the
  operator can always disable the Phase 91 cron to
  effectively achieve "judgment-only on" + "structural
  off" with one extra knob. A future phase could collapse
  the two configs into a per-mode enum if the operator
  surface demands it.
- **Asymmetric Hurt penalty.** Q2b's `-2*WEIGHT` for
  `Hurt` defers. v1 keeps symmetry with the structural
  mapping; a future phase can revisit if observed Hurt
  rates suggest entries are being insufficiently pruned.
- **Sum mode.** Q1c's stack-both-signals option defers —
  it risks double-counting on agreement and was the
  weakest of the four candidates.

## Prediction vs. reality

To be filled in at phase exit.

## Exit criteria

- [ ] `[recall_feedback].use_judgment_signal: bool`
  (default `false`) — Task 2.
- [ ] `correlate_detailed` augmented with the per-hit
  override rule; structural fallback for un-judged hits;
  byte-identical behaviour with knob off — Task 3.
- [ ] Unit tests on `correlate_detailed`: knob-off
  regression; knob-on with mixed-judgment hits; per-
  verdict mapping (`Used`/`Hurt`/`Irrelevant`);
  structural fallback for `None` — Task 3.
- [ ] All `correlate_detailed` / `correlate` call sites
  thread the knob from config — Task 4.
- [ ] Integration test: reflection-cron cycle with mixed-
  judgment hits → tally matches per-hit verdict-driven
  sum — Task 4.
- [ ] `aivyx learning` surface extended (judgment-vs-
  structural source split) — Task 5.
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
- [ ] Test count delta: positive (~+6-10; per the
  converged calibration law — knob on a new (effectively
  new) block (≈ +3-4) + augmentation logic (≈ +3-5,
  knob-off regression + knob-on per-verdict mapping +
  structural fallback) + integration (≈ +1); no new
  module, no new `KeyDomain`).
- [ ] Zero clippy warnings.
- [ ] Zero new workspace deps.
- [ ] Prediction-vs-reality block filled.
