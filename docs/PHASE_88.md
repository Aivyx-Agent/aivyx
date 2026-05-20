# Phase 88 — Pattern-Driven Persona Decay (the decay-side of the Phase 87 arc)

Phase 87 made the **co-occurrence ledger** drive Persona
*construction*: a durable affined pair of helpful topics
proposes a new `learned_context` facet. Phase 88 closes the
symmetric arc by making the **same ledger** drive Persona
*decay*: when the pair underlying an already-applied
`consolidate-pair:{A}+{B}` facet has demonstrably weakened in
the ledger, the facet's justification is gone — propose to
retire it.

The opposite move also lands: when a facet's pair is *still*
durable, age-decay no longer touches it (the relationship
still applies, so the identity still applies). This is the
exact symmetric protection Phase 85 added on the helpfulness
side, just keyed on the new provenance arm.

After Phase 88, every durable learning signal is consumed
**on both sides** of the Soul lifecycle:

|                                | Construction (proposes new facets) | Decay (retires applied facets) |
|--------------------------------|------------------------------------|-------------------------------|
| Phase 82 helpfulness ledger    | Phase 77 (warm-only retention)     | **Phase 85** (sustained-negative → decay; sustained-positive → protect from age) |
| Phase 83 co-occurrence ledger  | **Phase 87** (durable pair → propose facet) | **Phase 88** (this phase — pair weakens → decay; still strong → protect from age) |

The Phase 81 detector's age-only baseline remains for facets
without provenance to either signal. Every existing safety
property carries: propose-only, operator-gated, `Revert`-able,
core-protected.

## Why this, why now

- The deliberate Phase 87 deferral. Phase 87 shipped exactly
  half of the symmetric arc — the construction half. The
  decay half is its natural complement; shipping them in
  sequence keeps the surface coherent.
- Reuse is near-total. The Phase 85 provenance-gated
  detector + protection arm is the *exact* template; Phase
  88 adds a second provenance arm (`consolidate-pair:`)
  alongside the existing `recall-fb:` arm and a parallel
  helpfulness-vs-affinity read. The fold site already
  threads ledger reads + helpfulness hints; the
  co-occurrence ledger handle is already on `DaemonConfig`
  (Phase 83/84/87).
- The seam is proven. `LifecycleFacet` already carries
  optional `recall_topic` + `helpfulness` for Phase 85's
  detector arm. Phase 88 adds `pair: Option<(String, String)>`
  + `pair_affinity: Option<PairAffinityHint>` in the same
  shape; the detector branches on whichever provenance is
  present.

## Streak predictions

- **DESIGN.md** — **Will hold.** A second provenance arm on
  an existing detector touches no locked technical-contract
  decision. The Phase 85 precedent shipped a symmetric
  helpfulness arm without DESIGN.md touch; this is the same
  shape on a different ledger. Hash at entry:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Prediction: streak **extends to thirty-five** (currently
  34).

- **PRODUCT.md** — **Will hold.** P14 (Persona) is already
  delivered; this strengthens its decay pipeline. No new
  product commitment, none weakened; the operator-facing
  contract is *strengthened* (the Soul now retires identity
  whose underlying *relationship* dissolved, not only
  identity whose underlying topic stopped helping). No
  commitment-text edit. Hash at entry:
  `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.
  Prediction: streak **extends to twenty-eight** (currently
  27).

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold,
  by design.** Detector extension + config knob + fold-site
  read all live in `aivyx-channel` / `aivyx-config`. The
  decay proposals land through the existing
  `PersistentPersonaProposalLog::append` API (no new chain
  operation, no new `AuditTag` — the Phase 85 precedent).
  Hash at entry:
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.
  Prediction: streak **extends to thirty-six** consecutive
  phases (new project record, beats Phase 87's 35).

- **New workspace deps** — Zero. Reuses the Phase 83 ledger,
  the Phase 81/85 detector + lifecycle pass, the Phase 70
  proposal chain.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_88.md` + `docs/README.md` status row.

### Task 2 — `[persona_lifecycle].decay_pair_below_affinity` knob

`aivyx-config` (extend the existing `PersonaLifecycleConfig`
— the Phase 85 field-addition precedent):

- `PersonaLifecycleConfig` gains
  `decay_pair_below_affinity: f32` (default `1.0`, mirrors
  the Phase 87 `[persona_consolidation].min_affinity`
  default — a pair must be ≥ `1.0` to propose a facet, and
  staying ≥ `1.0` keeps the facet). Validation when both
  `enabled` and `signal_decay` are on:
  `decay_pair_below_affinity.is_finite()` AND `>= 0.0`.
- `RawPersonaLifecycle` + the build path. Tests: default;
  explicit override; non-finite reject; negative reject.

### Task 3 — Detector extension (the crux)

`aivyx-channel/src/persona_lifecycle.rs`:

- New types: `PairAffinityHint { affinity: f32, samples: u32 }`,
  shaped exactly like the existing `HelpfulnessHint`.
- `LifecycleFacet` gains two new optional fields:
  - `pair: Option<(String, String)>` — `Some((lo, hi))` only
    when the origin delta's `proposal_id` starts with
    `consolidate-pair:` (the canonical sorted form).
  - `pair_affinity: Option<PairAffinityHint>` — resolved at
    fold time by the lifecycle pass when a `pair` exists.
- The detector's decay arm gains a `consolidate-pair`
  signal branch: when `pair` is set AND `pair_affinity` is
  set AND `pair_affinity.affinity < decay_pair_below_affinity`
  → propose `RemoveList` with a `reason` citing the pair +
  the decayed affinity ("co-occurrence pair `A` + `B` —
  decayed affinity 0.4 (below floor 1.0); relationship no
  longer durable"). The proposal id is
  `pl:decay:learned_context:{value}` exactly as the existing
  decay arm (the Phase 85 dedup invariant carries — same
  shape, different reason).
- The detector's age-protection arm gains a parallel
  consolidate-pair branch: when `pair_affinity.affinity >=
  decay_pair_below_affinity` the facet is protected from
  age-decay (mirrors the existing helpfulness protection).
- Unit tests on the pure detector: pair-affinity decay
  fires; pair-affinity protection blocks age-decay; absent
  `pair_affinity` falls back to age-only (byte-identical to
  Phase 85); a facet with both `recall_topic` AND `pair`
  set follows whichever provenance is non-`None` (no
  collision, since a single delta has exactly one
  `proposal_id`).

### Task 4 — Fold-site read + reflection-scheduler wiring

`aivyx-channel/src/reflection_scheduler.rs`:

- The lifecycle pass already builds per-facet provenance
  including `recall_topic` + `helpfulness`. Add: when the
  origin delta's `proposal_id` starts with
  `consolidate-pair:`, parse the `{lo}+{hi}` pair from the
  suffix and resolve its decayed `pair_score` against the
  Phase 83 co-occurrence ledger handle (already on
  `PersonaLifecycleDeps`? — if not, thread it).
- Add `cooccurrence_ledger: Option<Arc<…>>` to
  `PersonaLifecycleDeps` if not already present (Phase 85
  added `helpfulness_ledger: Option<Arc<…>>`; this is the
  symmetric extension). The binary wires the same handle
  already in `DaemonConfig` (Phase 83). `None` → no
  pair-affinity hint → the detector falls back to age-only
  for `consolidate-pair:` facets (byte-identical to
  pre-Phase-88).
- Integration test in `reflection_scheduler::tests`:
  - Two facets on the persona chain — one
    `consolidate-pair:A+B` (durable pair, score 5.0) and
    one `consolidate-pair:X+Y` (drifted pair, score 0.3).
    With `decay_pair_below_affinity = 1.0`, only the
    drifted facet decays; the durable one is protected
    from age-decay even at large ages.
  - Disabled config (`signal_decay = false` or no ledger)
    → byte-identical to Phase 85 (no pair-driven decay).

### Task 5 — Tests + docs + exit

- `docs/INSTALL.md` — extend the Phase 85 "Helpfulness-driven
  decay" section (or add a sibling) with a "Pattern-driven
  decay (Phase 88)" subsection: the symmetric arc, the new
  knob, the byte-identical-fallback behaviour, the
  propose-only safety carrying over.
- `examples/aivyx.toml` — document the new
  `decay_pair_below_affinity` knob on the existing
  `[persona_lifecycle]` block.
- Exit: ROADMAP + PRODUCT_ROADMAP frozen entries,
  docs/README status flip, prediction-vs-reality, hash
  backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Signal floor:** (a) Decayed affinity below a
  configurable floor. The `consolidate-pair:` facet's
  justification IS the pair's durability; once the pair
  falls below the floor, the facet no longer applies.
  Simple, symmetric with Phase 87's `min_affinity` (default
  `1.0`), and easy to tune.
- **Q2 — Endpoint helpfulness:** (a) No additional gate.
  The facet was proposed because the *relationship* was
  durable; conditioning decay on individual topic
  helpfulness would tie two distinct signals and risk
  leaving drifted-but-still-warm pair facets in place
  forever — the exact case Phase 88 is meant to handle.
- **Q3 — Symmetric protection:** (a) Yes, mirroring Phase
  85: a `consolidate-pair:` facet whose pair's decayed
  affinity is still ≥ the floor is protected from
  age-decay. The assistant never retires identity that's
  still demonstrably in use.
- **Q4 — Config surface:** (a) New knob on the existing
  `[persona_lifecycle]` block, gated by the existing
  `signal_decay`. All Persona-decay knobs live in one
  cohesive block; one switch arms the whole decay system.

## Deferrals

**Rolling deferrals carried into Phase 88:**

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
- Phase 85 deferrals (helpfulness-driven *consolidation*
  (closed in Phase 87), reflection-facet decay via fuzzy
  embedding).
- Phase 86 deferrals (token-budget context sizing, heuristic
  recall gate, embed-each-and-pool windows, persisted
  windows).
- Phase 87 deferrals (pattern-driven Persona *decay* — **THIS
  PHASE**, n-ary cluster proposals, pattern-driven
  supersession, operator-tunable LLM prompt).

**Likely Phase 88 deferrals:**

- **Pattern-driven supersession.** When a *related* pair
  strengthens enough to subsume an older facet's pair, the
  cleanest move is supersession (edit-and-replace), not
  decay-then-propose. Defers with the Phase 70/87
  supersession deferral.
- **N-ary clusters in the decay path.** v1 only acts on
  2-ary pairs (the Phase 83 ledger's unit); triples /
  larger clusters defer with the broader Phase 83 n-ary
  deferral.
- **Pair-affinity hysteresis.** A pair oscillating around
  the floor can re-propose / re-decay across cycles. v1
  trusts the EWMA half-life to smooth this; an explicit
  hysteresis band (decay below `floor - delta`, protect
  above `floor + delta`) defers pending operator feedback.
- **Topic canonicalization.** Still deferred from Phase 82.
  `deploy` / `deployment` / `deploys` are still separate
  endpoints across both ledgers.

## Prediction vs. reality

To be filled in at phase exit.

## Exit criteria

- [ ] `[persona_lifecycle].decay_pair_below_affinity`
  (default `1.0`, validated finite ≥ 0.0 when armed) —
  Task 2.
- [ ] `LifecycleFacet` gains `pair: Option<(String,
  String)>` + `pair_affinity: Option<PairAffinityHint>`;
  detector's decay arm gains the consolidate-pair branch
  (decay-trigger and age-protection both) — Task 3.
- [ ] Lifecycle pass resolves the pair's decayed affinity
  from the co-occurrence ledger when present and threads it
  to the detector; `None` → byte-identical to pre-Phase-88
  for `consolidate-pair:` facets — Task 4.
- [ ] Pure detector unit tests: pair-decay-triggers,
  pair-protects-from-age, absent-pair-affinity-falls-back,
  collision-free with the `recall-fb:` arm — Task 3.
- [ ] Integration test: durable pair protected,
  drifted pair decays, disabled config no-op, ledger-absent
  no-op — Task 4.
- [ ] `docs/INSTALL.md` + `examples/aivyx.toml` updated —
  Task 5.
- [ ] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 5.
- [ ] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [ ] DESIGN.md streak extends to thirty-five.
- [ ] PRODUCT.md streak extends to twenty-eight.
- [ ] Production-core streak extends to thirty-six (new
  record) — `lib.rs` byte-identical.
- [ ] Test count delta: positive (~+3-7; per the converged
  calibration law — knob on an existing block (≈ +1) +
  detector extension (≈ +3-4 unit tests) + integration
  (≈ +1); no new module, no new `KeyDomain`, no new config
  section).
- [ ] Zero clippy warnings.
- [ ] Zero new workspace deps.
- [ ] Prediction-vs-reality block filled.
