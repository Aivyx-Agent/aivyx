# Phase 92 — Pattern-Driven Supersession (the longest-running Persona-actuator deferral, closed)

After Phase 87 (pattern-driven construction) and Phase 88
(pattern-driven decay), the Soul accumulates and retires
`consolidate-pair:` facets independently. But a real
operator workflow shifts continuously: the pair
`(auth, jwt)` might dominate one quarter, then
`(auth, sessions)` the next. Today the actuator handles this
as **two independent decisions**:

- Phase 88 proposes decay of `consolidate-pair:auth+jwt`
  (the old pair weakened).
- Phase 87 proposes a new `consolidate-pair:auth+sessions`
  facet (the new pair strengthened).

The operator sees two proposals to review separately —
nothing tells them these are logically linked. The Soul
gets cluttered with overlapping facets while the operator
decides.

Phase 92 closes the longest-running Persona-actuator
deferral (Phase 70, carried forward 22 phases through
71-91) with **pattern-driven supersession**: when an
existing facet's pair weakens AND a new candidate pair
sharing one endpoint strengthens, the consolidation pass
files the `RemoveList` + `AppendList` proposals **linked by
metadata** so the operator-facing surface presents them as a
single supersession decision. Each proposal is still
independently `Revert`-able; the linkage is operator-visible
context, not a chain-level atomic primitive.

## Why this, why now

- It is the longest-running Persona-actuator deferral
  (Phase 70, 22 phases old). Phases 87 + 88 separately
  deferred it; the symmetry-closing arc made supersession
  the natural cleanup move.
- After the input-quality arc (86 + 89 + 90) and the
  feedback-quality move (91), the Soul actuator is the
  remaining place where focused UX work moves the
  experience forward. Operators with active Phase 87
  consolidation see overlapping facets pile up; Phase 92
  groups them.
- The change is **purely additive**. With the new
  `enable_supersession: bool` knob off (the default), the
  Phase 87 / Phase 88 flow is byte-identical to
  pre-Phase-92. The new metadata field carries through
  the proposal chain via `#[serde(default,
  skip_serializing_if = "Option::is_none")]` — the Phase
  84 / Phase 91 wire-compat shape.
- Reuse is near-total: the structural detector lives in
  `aivyx-channel::persona_consolidation`; the
  `PairPhraser` LLM seam is unchanged; the proposal chain
  gains one optional field (no schema migration).

## Streak predictions

- **DESIGN.md** — **Will hold.** Linking two existing
  proposals by metadata touches no locked technical-contract
  decision. The Phase 70 proposal chain already supports
  multi-proposal flows (the consolidation pass files
  multiple per cycle); Phase 92 just tags some of them as
  belonging together. Hash at entry:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Prediction: streak **extends to thirty-nine** (currently
  38).

- **PRODUCT.md** — **Will hold.** P14 (Persona) is
  delivered; this is a UX refinement on its proposal
  pipeline. No new commitment, none weakened; the
  operator-facing contract is *strengthened* (the Soul's
  proposal flow groups linked decisions; the operator
  reviews supersession as one logical action). Hash at
  entry:
  `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.
  Prediction: streak **extends to thirty-two** (currently
  31).

- **Production-core `aivyx-core/src/lib.rs`** — **Will
  hold, by design.** The detector + the proposal-chain
  field + the reflection-scheduler integration + the
  config knob all live in `aivyx-channel` /
  `aivyx-config`. The proposal chain's `PersonaProposal`
  type is in `aivyx-channel` (not aivyx-core); adding the
  optional `supersedes_proposal_id` field is a local
  change. No `aivyx-core` touch; no new `AuditTag`. Hash
  at entry:
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.
  Prediction: streak **extends to forty** consecutive
  phases (new project record, beats Phase 91's 39).

- **New workspace deps** — Zero. Reuses Phase 87's
  `PairPhraser` + the existing Phase 70 proposal chain.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_92.md` + `docs/README.md` status row.

### Task 2 — `[persona_consolidation].enable_supersession` knob

`aivyx-config`:

- `PersonaConsolidationConfig` gains
  `enable_supersession: bool` (default `false`). With
  `false` the supersession-detection branch never runs —
  byte-identical to pre-Phase-92 (Phase 87 proposals + Phase
  88 decay proposals continue to file independently).
- `RawPersonaConsolidation` + the build path. Validation:
  trivial (a boolean can't be invalid).
- Tests: explicit `true` wins; default `false`; absent
  section honored.

### Task 3 — `supersedes_proposal_id` field + detector (the crux)

`aivyx-channel`:

- `PersonaProposal` (in `persona_proposal.rs`) gains a new
  optional field
  `supersedes_proposal_id: Option<String>` with
  `#[serde(default, skip_serializing_if = "Option::is_none")]`
  — the Phase 84 `cluster: bool` / Phase 91 `judgment:
  Option<…>` wire-compat shape. Old proposal-chain entries
  decode unchanged; fresh `None` proposals serialize without
  the field.
- New `detect_supersession(...)` pure function in
  `persona_consolidation` module: takes the current Persona
  chain's applied `consolidate-pair:` facets, the
  co-occurrence ledger, the helpfulness ledger, the
  consolidation config, and returns a `Vec<SupersessionCandidate>`
  where each candidate is `{ old_proposal_id, old_facet_value,
  new_pair, new_facet_proposal_id }`. The detection rule
  (Q1a): for each applied `consolidate-pair:{A}+{B}` facet
  whose pair affinity has decayed below the Phase 88 floor,
  look in the Phase 83 ledger for a new pair `(A, C)` or
  `(B, C)` whose affinity is above the Phase 87 construction
  floor AND both endpoints are independently helpful
  (Phase 82 ledger ≥ `min_topic_helpfulness`). If found,
  it's a supersession candidate.
- Unit tests on the pure detector: shared-endpoint match
  fires (`(A, B)` weakens, `(A, C)` strengthens → supersede);
  no-overlap pair is NOT a supersession (just two
  independent phases); the new pair's same-endpoint-but-
  unhelpful → does NOT supersede (gate enforced); duplicate
  detection (the same `(A, C)` can't supersede two
  different existing facets in one cycle — pick the strongest
  match deterministically).

### Task 4 — Reflection-scheduler integration

`aivyx-channel/src/reflection_scheduler.rs`:

- `run_persona_consolidation_pass` is extended to call
  `detect_supersession` BEFORE the standard `consolidate(...)`
  selector. When `enable_supersession = true`:
  1. Detect supersession candidates.
  2. For each, file two linked proposals:
     - `RemoveList { value: old_facet_value }` with
       `supersedes_proposal_id = None` AND a `reason` citing
       the supersession.
     - `AppendList { value: <phrased by Phase 87 PairPhraser> }`
       with `supersedes_proposal_id = Some(old_proposal_id)`
       AND a `reason` citing the new pair + the linkage.
     - The `RemoveList`-side records the linkage via the
       `RemoveList`'s `supersedes_proposal_id` field too, so
       the operator-facing surface can resolve both halves
       from either side.
  3. Skip these `(A, B)` and `(A, C)` pairs from the
     standard `consolidate(...)` selector to avoid duplicate
     filing.
- `PersonaConsolidationStat` gains a `superseded: u32`
  counter (with `#[serde(default)]` for IPC wire-compat).
- Integration test: a persona chain with an existing
  applied `consolidate-pair:{A}+{B}` facet, the Phase 83
  ledger with a weakening `(A, B)` pair AND a strengthening
  `(A, C)` pair, the Phase 82 ledger with all three topics
  helpful → cycle 1 files two linked proposals (one
  `RemoveList` + one `AppendList`); the `superseded` count
  on the stat is 1; idempotent on cycle 2 (the dedup against
  the proposal chain catches both halves).

### Task 5 — Surface + tests + docs + exit

- The `aivyx persona proposals` CLI + the Web UI Proposals
  pane already render each proposal with its `reason`.
  Phase 92 changes the operator surface minimally: each
  half of a supersession pair carries the linkage in its
  `reason` text ("supersedes proposal `consolidate-
  pair:auth+jwt`" on the AppendList half;
  "superseded by proposal `consolidate-pair:auth+sessions`"
  on the RemoveList half). The structured
  `supersedes_proposal_id` field is available to future
  Web UI work (grouping the two visually); v1's
  `reason`-based linkage is the minimum-shipping form.
- `aivyx learning` render block already shows
  `PersonaConsolidationStat`; extend it with a
  "superseded N" suffix.
- `docs/INSTALL.md` — extend the existing Phase 87
  "Pattern-driven Persona proposals" section with a
  "Pattern-driven supersession (Phase 92)" subsection: the
  shared-endpoint detection, the LLM-phrased replacement,
  the linked-proposals shape, the opt-in knob, the
  independently-Revert-able guarantee.
- `examples/aivyx.toml` — document the new
  `enable_supersession` knob alongside the existing
  `[persona_consolidation]` block.
- Exit: ROADMAP + PRODUCT_ROADMAP frozen entries,
  docs/README status flip, prediction-vs-reality, hash
  backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Trigger:** (a) Shared-endpoint detection. An
  existing `consolidate-pair:{A}+{B}` facet's pair has
  decayed below the Phase 88 floor AND a new pair `(A, C)`
  sharing one endpoint has strengthened above the Phase 87
  construction floor (both endpoints helpful). Conservative;
  deterministic; only fires when there's a clear
  "replacement" relationship.
- **Q2 — LLM seam:** (a) Reuse Phase 87's `PairPhraser`
  for the new facet's prose. Structural detection is pure;
  prose phrasing reuses the existing seam. Per-candidate
  LLM failure → skip that supersession (the facet stays via
  the standard Phase 87/88 flow).
- **Q3 — Proposal shape:** (a) Two existing-shape proposals
  (`RemoveList` for the old facet + `AppendList` for the
  new facet) linked by a new optional `supersedes_proposal_id`
  field on `PersonaProposal` with `#[serde(default,
  skip_serializing_if = "Option::is_none")]` for wire-compat
  (the Phase 84 / Phase 91 precedent). No new proposal
  kind; no chain-schema migration; each half remains
  independently `Revert`-able.
- **Q4 — Default:** (a) New knob on the existing
  `[persona_consolidation]` block, default `false`. The
  operator already opting into Phase 87 consolidation
  enables supersession with one extra key; with the knob
  off the proposal flow is byte-identical to pre-Phase-92.

## Deferrals

**Rolling deferrals carried into Phase 92:**

- v0.1.0 publication (Phase 61 Task 7).
- System-prompt notification-target enumeration (Phase 62).
- `turn_id` correlation refactor (Phase 67).
- Phase 68 deferrals (XOAUTH2, HTML email, attachments,
  multi-account `[email]`).
- Phase 69 deferrals (WebPush / service-worker, notification
  urgency, icons, sound).
- Phase 70 deferrals (**proposal supersession — THIS PHASE**,
  multi-window reflection).
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
- Phase 87 deferrals (n-ary cluster proposals, **pattern-
  driven supersession — THIS PHASE**, operator-tunable LLM
  prompt).
- Phase 88 deferrals (**pattern-driven supersession — THIS
  PHASE**, n-ary cluster decay, pair-affinity hysteresis).
- Phase 89 deferrals (operator-tunable `[[topic_alias]]`
  mappings, topic-by-topic exception list, one-time
  migration of existing fragmented data, non-ASCII /
  Unicode stemming).
- Phase 90 deferrals (pattern-based stoplist, LLM-judged
  gate, adaptive thresholds, token-budget context sizing).
- Phase 91 deferrals (actuator-side switch from structural
  proxy to the new judgment signal, per-recall LLM
  critique, adaptive batch size, multi-model ensembling,
  response-text recovery via audit-chain extension).

**Likely Phase 92 deferrals:**

- **Web UI visual grouping of linked supersession proposals.**
  v1 surfaces the linkage via the `reason` text on each
  half; a future Web UI Proposals-pane refinement could
  render them as a single visual card with two atomic
  approve/reject actions. Defers as a Phase 78-surface
  enrichment.
- **Atomic chain-level supersession primitive.** Q3b
  (`Supersede` proposal kind) and Q3c (`Compound`
  proposals) defer. The v1 linked-proposals shape is
  sufficient for operator-side semantics; a true chain-
  level atomic primitive would require an HMAC-chain
  schema migration.
- **N-ary cluster supersession.** v1 only handles 2-ary
  pair supersession (`(A, B)` → `(A, C)`); triples /
  larger clusters defer with the broader Phase 83 n-ary
  deferral.
- **Semantic-similarity supersession.** Q1c (using
  embeddings to detect synonym pairs that share zero
  literal endpoints) defers as a Phase 89-`[[topic_alias]]`
  follow-up.

## Prediction vs. reality

**Predictions held — all three streaks correct.**

- **DESIGN.md — held.** Linking two existing proposals by
  metadata touched no locked technical-contract decision.
  The Phase 70 proposal chain already supports multi-
  proposal flows; Phase 92 tags some of them as belonging
  together. Streak: **39 consecutive phases** (was 38).
- **PRODUCT.md — held.** P14 (Persona) is delivered; this
  is a UX refinement on its proposal pipeline. No new
  commitment, none weakened; the operator-facing contract
  was *strengthened* (the Soul's proposal flow groups
  linked decisions). Streak: **32 consecutive phases**
  (was 31).
- **`aivyx-core/src/lib.rs` — held, by design.** The
  detector + the proposal-chain field + the reflection-
  scheduler integration + the config knob all live in
  `aivyx-channel` / `aivyx-config`. The proposal chain's
  `PersonaProposal` + `ProposedPersonaDelta` types are in
  `aivyx-channel`. No new `AuditTag`. Streak: **40
  consecutive phases** — new project record, beating
  Phase 91's 39.

**Test count — `+10`** (workspace `1643 → 1653`). Squarely
inside the predicted `+6-10` band. Breakdown:

- Config knob `+3` (default false even when consolidation
  enabled; explicit-true wins; supersession-only-key
  section builds Some via the any-field-set predicate).
- Field + detector `+6` (`parse_pair_proposal_id`
  round-trip + malformed rejection; `detect_supersession`
  shared-endpoint fires; no-shared-endpoint skipped;
  durable-old-pair skipped; unhelpful-new-endpoint
  skipped; `enable_supersession = false` short-circuits).
- Reflection-scheduler integration `+1` (one multi-cycle
  scenario covering filed → idempotent dedup; both linked
  halves carry cross-referenced `supersedes_proposal_id`;
  `superseded = 1, filed = 2` on the stat).

**Scope — every planned surface shipped exactly as scoped.**
The opt-in knob; the optional `supersedes_proposal_id`
field on `ProposedPersonaDelta` with full HMAC-chain wire-
compat (the `serde_jcs` canonicalization respects
`skip_serializing_if` — absent fields don't appear in the
canonical bytes, so old entries still verify and fresh
`None` round-trips identically to pre-Phase-92); the pure
`detect_supersession` detector with shared-endpoint
detection + Phase 87 double-gate inheritance + Phase 88
floor reuse; the reflection-pass wiring that runs
supersession BEFORE the standard selector and excludes
already-filed canonical ids from the selector's
candidates; the `superseded: u32` counter on the Phase 78
surface stat with IPC wire-compat; the binary's threading
of `persona_log` + `pair_below_affinity` from the
`[persona_lifecycle]` config — all landed as the open-doc
described. The `RemoveList` side uses a distinct
`supersede-remove:<old_id>` id (rather than reusing the
old `consolidate-pair:` id) so it can't collide with a
future re-proposal of the same canonical pair. Both
halves carry the cross-linked `supersedes_proposal_id`
field. Zero clippy warnings. Zero new workspace deps.

## Exit criteria

- [x] `[persona_consolidation].enable_supersession: bool`
  (default `false`) — Task 2 (commit `a5b2abb`).
- [x] `ProposedPersonaDelta` gains
  `supersedes_proposal_id: Option<String>` with
  `#[serde(default, skip_serializing_if = "Option::is_none")]`;
  old proposal-chain entries decode unchanged — Task 3
  (commit `5d36b80`). Field placed on
  `ProposedPersonaDelta` (the chained type) rather than on
  `PersonaProposal` (the derived view) for single source
  of truth; surface reads it via
  `proposed_op.supersedes_proposal_id`.
- [x] `detect_supersession(...)` pure function in
  `persona_consolidation` module: shared-endpoint match,
  pair-floor + helpfulness gates, deterministic ranking
  on ambiguity — Task 3 (commit `5d36b80`).
- [x] Unit tests on the pure detector: shared-endpoint
  fires; no-overlap doesn't; helpfulness gate enforced;
  duplicate-detection deterministic — Task 3 (commit
  `5d36b80`).
- [x] `run_persona_consolidation_pass` files linked
  proposals when `enable_supersession = true`; both
  halves carry the cross-linked `supersedes_proposal_id`;
  the standard `consolidate(...)` selector skips the
  superseded pairs to avoid duplicate filing — Task 4
  (commit `0c7cb1c`).
- [x] `PersonaConsolidationStat` gains
  `superseded: u32` with IPC wire-compat
  (`#[serde(default)]`) — Task 4 (commit `0c7cb1c`).
- [x] Integration test: existing applied facet + weakening
  pair + strengthening shared-endpoint pair → cycle 1
  files two linked proposals; idempotent on cycle 2 —
  Task 4 (commit `0c7cb1c`).
- [x] `docs/INSTALL.md` + `examples/aivyx.toml` updated —
  Task 5 (this commit).
- [x] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 5 (this commit).
- [x] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [x] DESIGN.md streak extends to thirty-nine.
- [x] PRODUCT.md streak extends to thirty-two.
- [x] Production-core streak extends to forty (new
  record) — `lib.rs` byte-identical.
- [x] Test count delta: positive (`+10`, squarely inside
  the predicted `+6-10` band).
- [x] Zero clippy warnings.
- [x] Zero new workspace deps.
- [x] Prediction-vs-reality block filled.
