# Phase 81 — Persona Lifecycle (the Soul that refines itself)

For 80 phases the Persona has only ever **grown**. The
reflection loop (Phase 70/71), the recall-feedback proposals
(Phase 77), and adaptive selection (Phase 79) all add or
surface facets — none of them ever consolidates a redundant
one, retires a stale one, or lets an unreinforced one fade. A
Soul that only accretes, over months, dilutes its own signal
and eventually contradicts itself; Phase 80 raised the stakes
again, because that same bloated Soul now also drives
unprompted proactive sends.

Phase 81 closes the open half of the identity arc: it gives
the Persona a **lifecycle**. On its existing reflection
cadence the assistant notices near-duplicate facets and
long-unreinforced facets and **proposes** consolidation or
decay — never editing identity itself, always through the
existing operator-gated proposal path, and never touching the
always-on core. This is the "self-**improving**" half of the
self-learning vision: the character does not just remember
more, it gets *sharper*.

## Why this, why now

- The 79 arc's open deferral. Phase 79 explicitly deferred
  "Persona consolidation/supersession/decay"; Phase 80 made an
  only-growing Soul materially more consequential (it now
  feeds proactive outbound). This is the natural, highest
  vision-alignment completion of the identity layer.
- Every precondition exists and is proven. The Persona chain
  is append-only but **fully reversible** (`Revert`, Phase 60)
  and every mutation is already a `RemoveList` / `AppendList`
  / `Revert` op flowing through the operator-gated
  `PersonaProposal` path (Phase 59/60). Consolidation and
  decay are expressible *entirely* in those existing ops — no
  schema migration, no new `KeyDomain`, no new delta op.
- The embedding seam (Phase 79 `PersonaContextRefiner`) and
  the reflection-piggyback `Deps` pattern (Phase 77
  `RecallFeedbackDeps`, Phase 80 `ProactiveDeps`) are directly
  reusable. Reuse is near-total: a lifecycle action is a
  normal Pending `PersonaProposal` with a specific shape. No
  new scheduler, no LLM call, no agent turn.

## Streak predictions

- **DESIGN.md** — **Will hold.** Persona lifecycle reuses the
  reflection cadence + the existing proposal/persona chains +
  the Phase 79 embedding seam; no locked technical-contract
  decision is touched. Prediction: streak **extends to
  twenty-eight** consecutive phases (currently 27).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** Refining *how* the
  already-delivered P14 Persona maintains itself introduces no
  new product commitment and weakens none; the operator-facing
  contract is in fact *strengthened* (a structurally-enforced
  guarantee that the always-on core is never consolidated or
  decayed, plus full reversibility of every lifecycle action).
  No commitment-text edit. Prediction: streak **extends to
  twenty-one** consecutive phases.
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold,
  by design.** Lifecycle actions are normal `PersonaProposal`s
  appended through the *existing* proposal log and resolved
  through the *existing* `resolve_persona_proposal` path —
  Persona mutations are already structural in the
  proposal/persona chains, not separate `AuditTag` events, so
  there is no new audit variant and no `aivyx-core` type
  change (the Phase 76/77/80 streak lesson, continued). The
  detector, the pass, the config, and the observability all
  live in `aivyx-channel` / `aivyx-config`. Prediction: streak
  **extends to twenty-nine** consecutive phases (new project
  record, beats Phase 80's 28).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

- **New workspace deps** — Zero. Reuses the 70/71 cadence, the
  59/60 proposal+persona chains + `Revert`, the 79 embedding
  seam, and the 78 surface.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_81.md` + `docs/README.md` status row.

### Task 2 — `[persona_lifecycle]` config section

`aivyx-config` (mirrors the Phase 80 `[proactive]` pattern):

- `PersonaLifecycleConfig { enabled: bool /* default false */,
  consolidation_similarity: f32 /* cosine near-duplicate
  threshold */, decay_max_age_secs: u64 /* unreinforced-age
  horizon */, min_soft_facets: u32 /* never act on a Soul
  smaller than this */, signals: PersonaLifecycleSignals }`
  where `PersonaLifecycleSignals` toggles the action classes
  (`consolidate`, `decay`).
- Absent section → `None` → lifecycle **off** (pre-Phase-81
  behaviour, the common case). Validation when enabled:
  `consolidation_similarity` in `(0.0, 1.0]`,
  `decay_max_age_secs >= 1`, `min_soft_facets >= 1`, at least
  one signal class on. `AivyxConfig.persona_lifecycle:
  Option<PersonaLifecycleConfig>`. Tests for defaults,
  validation, off-when-absent.

### Task 3 — Structural persona-lifecycle detector

`aivyx-channel`, pure + heavily unit-tested (the crux):

- Input: the **six soft-list** facet categories *only* (each
  facet carrying its originating delta age, derived from the
  persona log), the Phase 79 embedding provider, the
  `[persona_lifecycle]` toggles, and `now`. The scalar
  identity (`assistant_name`, `operator_profile`,
  `communication_style`) and **every `behavioral_constraint`**
  are *never passed in* — the Phase 79 always-on-core
  invariant extended structurally to the lifecycle layer:
  lifecycle can only ever touch the six soft lists.
- Output: zero or more `PersonaLifecycleAction { kind:
  Consolidate { originals: Vec<String>, merged: String } |
  Decay { value: String }, category, reason }` — each with a
  concrete, human-readable `reason` (provenance). Deterministic
  ordering for stable cross-cycle dedup.
- Action classes (no LLM, conservative): `Consolidate`
  (pairwise cosine over a category's facets, cluster
  near-duplicates strictly above `consolidation_similarity`,
  propose a deterministic merged facet); `Decay` (a facet
  whose originating delta is older than `decay_max_age_secs`
  with no later reinforcing delta in its category). Each
  toggled by config; never act below `min_soft_facets`.
  Thorough per-class + empty + **core-protection invariant**
  tests (the scalars/`behavioral_constraints` can never be
  produced as an action, proven the way Phase 79 proves
  `reduce_persona`).

### Task 4 — Lifecycle pass → proposals (piggyback the reflection cron)

`aivyx-channel`, mirroring the Phase 80 `ProactiveDeps`
wiring:

- The reflection scheduler's existing on-cron pass also: reads
  the effective Persona soft-list facets + their ages from the
  `PersistentPersonaLog`, runs the detector, drops any action
  that already has a matching Pending/Approved proposal
  (cross-cycle dedup — the assistant must not re-file the same
  identity change every cycle), and **files each surviving
  action as a Pending `PersonaProposal`** through the existing
  `PersistentPersonaProposalLog` (a `Consolidate` proposal
  carries the `RemoveList`×N + `AppendList` recipe; a `Decay`
  proposal carries the `RemoveList`). The loop never resolves
  proposals — the operator approves/rejects in the existing
  `aivyx persona` CLI + Web UI, and `Revert` makes every
  resolved action reversible. No `[persona_lifecycle]` /
  disabled / no schedule → the pass is a complete no-op
  (pre-Phase-81 behaviour). Deps threaded via `DaemonConfig`
  (`ProactiveDeps` precedent). Integration test over a real
  store: detect → file → dedup-next-cycle.

### Task 5 — Observability (Phase 78-consistent)

- Per-cycle stderr breadcrumb: `aivyx persona-lifecycle:
  proposed N (C consolidate, D decay — S suppressed:
  deduped/below-floor/disabled)`.
- Extend the Phase 78 `GetLearningInsights` /
  `LearningInsights` with a lifecycle summary (last proposed
  actions + their `reason` provenance + dedup/suppressed
  counts), rendered in the `aivyx learning` CLI and the Web UI
  Learning pane, parity with the Phase 79 `persona_selection`
  and Phase 80 `proactive` lines. Shared last-cycle stat
  handle (the Phase 79/80 `Arc<RwLock<Option<…>>>` pattern).
  HTML smoke + IPC round-trip updated.

### Task 6 — Tests + docs + exit

- Tests: config, detector per action class + empty +
  core-protection invariant, pass integration (detect → file
  → cross-cycle dedup over a real store), observability, IPC
  round-trip, CLI/Web UI render.
- Docs: `docs/INSTALL.md` "Persona lifecycle (Phase 81)"
  (opt-in, the structural gate, core protection, full
  reversibility via the proposal+`Revert` chain, where it's
  surfaced, how to turn it off); `examples/aivyx.toml`
  `[persona_lifecycle]` block with the off-by-default note.
- Exit: ROADMAP + PRODUCT_ROADMAP frozen entries, docs/README
  status flip, prediction-vs-reality, hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Cadence:** (a) Piggyback the reflection cron pass.
  Zero new scheduler; `PersonaLifecycleDeps` threaded into
  `run_reflection_scheduler` exactly like Phase 77/80. A
  standalone `[[persona_lifecycle_schedule]]` is a deferral.
- **Q2 — Agency:** (a) Propose-only, operator-gated. The pass
  files normal Pending `PersonaProposal`s
  (`RemoveList`/`AppendList`/`Revert` ops); the operator
  approves/rejects in the existing surface. The loop never
  mutates identity itself — the Phase 77/79 no-self-mutation
  ethos applied to the highest-stakes layer; full
  reversibility via the existing `Revert`.
- **Q3 — Detection:** (a) Structural + embedding, no LLM.
  Consolidation reuses the Phase 79 cosine seam (pairwise
  near-duplicate clustering above a high threshold); decay is
  purely age-based structural. No new persistence, no new
  deps, deterministic. Helpfulness-driven decay (needs a
  persisted Phase-77 tally) and contradiction-based
  supersession defer.
- **Q4 — Control + legibility:** (a) Opt-in (off by default)
  + scalars & every `behavioral_constraint` structurally
  excluded (the Phase 79 always-on-core invariant extended) +
  every action a reversible proposal + full provenance on the
  Phase 78 learning surface. Default-off, core-protected,
  fully reversible, explainable.

## Deferrals

**Rolling deferrals carried into Phase 81:**

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
  LLM-judged recall usefulness, cross-session pattern
  learning).
- Phase 78 deferrals (per-memory-entry drill-down, history
  beyond the recall-log window, Web UI live refresh,
  actionable insights).
- Phase 79 deferrals (`[persona]` tuning block,
  conversational-window selection, behavioural Persona).
- Phase 80 deferrals (standalone `[[proactive_schedule]]`,
  LLM-composed proactive prose, conversational/interactive
  proactive, additional proactive signal classes).

**Likely Phase 81 deferrals:**

- **Helpfulness-driven decay.** v1 decay is age-based; decay
  keyed on a sustained-low Phase-77 helpfulness signal needs a
  persisted tally (a new domain) and defers.
- **Contradiction-based supersession.** v1 retires duplicates
  and stale facets; detecting that a newer facet *semantically
  contradicts/overrides* an older one (vs merely near-dup)
  defers.
- **Standalone `[[persona_lifecycle_schedule]]`.** v1
  piggybacks the reflection cadence; an independent timer
  defers.
- **Facet-scoped one-click revert.** v1 reuses the existing
  per-delta `Revert`; a single "undo this consolidation"
  affordance over the multi-delta recipe defers.

## Prediction vs. reality

**Streak — all three predictions correct (the headline).**
DESIGN.md, PRODUCT.md, and `aivyx-core/src/lib.rs` are all
byte-identical to their entry hashes:

- DESIGN.md `89dc8903…` unchanged → streak **28** (predicted
  "extends to twenty-eight" — exact).
- PRODUCT.md `cd60c4f9…` unchanged → streak **21** (predicted
  "extends to twenty-one" — exact).
- `aivyx-core/src/lib.rs` `69fb9af1…` unchanged → streak
  **29**, a new project record beating Phase 80's 28
  (predicted "extends to twenty-nine (new record)" — exact).

The Phase 76/77/80 lesson held again: lifecycle actions are
existing-shape `RemoveList` `PersonaProposal`s flowing through
the *existing* proposal/persona chains, so there was no new
`KeyDomain` and no new `AuditTag` — nothing required an
`aivyx-core` type change. Detector, pass, config, and the
whole observability surface lived in `aivyx-channel` /
`aivyx-config`, exactly as predicted at open.

**Test delta — +17 (1507 → 1524), IN BAND, prediction
correct.** Predicted "~+16-22 … slightly lighter than Phase
80's +20 as there is no new KeyDomain." Actual +17 lands in
the lower half of that band — the **second consecutive in-band
landing** (Phase 80 was the first, after four small misses),
and the "slightly lighter than +20" call was right: Phase 80's
new `ProactiveLog` KeyDomain carried ~3-4 isolation/GC/dedup
tests that Phase 81 (reusing the persona + proposal chains)
did not need; the embedding-clustering detector +
core-protection invariant + `to_proposals` + config + the
integration pass carried the rest. The converged calibration
is now validated across three regimes: reuse-heavy ≈ +10-15,
new-surface-with-new-domain ≈ +20, new-surface-no-new-domain
≈ +17.

**Deviations — one scoped simplification, recorded honestly
(net safer).** The plan (Task 4 / Q2a) described a
`Consolidate` proposal as carrying "the `RemoveList`×N +
`AppendList` recipe." In implementation the deterministic
merge picks the **longest existing member** as the canonical
facet — which is, by construction, already present in the
soft list. The `AppendList(merged)` would therefore be an
idempotent no-op (per `PersonaDeltaOp::AppendList`'s
documented "duplicate appends are idempotent"). Emitting a
no-op proposal the operator must review is pure noise, so the
pass emits **only** the `RemoveList`s for the non-canonical
near-duplicates and no `AppendList`. This is strictly safer
and simpler than the planned recipe: each removal is an
independent, individually-reviewable, individually-`Revert`-able
proposal, and the kept facet is never touched at all (so a
consolidation can never transiently drop the canonical text).
No behaviour the plan promised is lost — consolidation still
dedupes a soft list down to its canonical member, operator-
gated and reversible — but the "+ AppendList" half of the
recipe was correctly identified as unnecessary and dropped.
Every other planned surface shipped exactly as scoped.

**Core-protection invariant — delivered structurally, not by
runtime check.** `soft_facets_of` is total over the six soft
lists and has no arm for the scalar identity or
`behavioral_constraints`; `SoftCategory` has no variant for
them; `to_delta_category` is total over the same six. A
scalar or guardrail therefore *cannot* be expressed as a
lifecycle action — the Phase 79 always-on-core invariant
extended to this layer by construction, proven by the
`core_protection_soft_facets_of_excludes_core` test rather
than asserted at runtime.

No clippy warnings. No new workspace deps.

## Exit criteria

- [x] `[persona_lifecycle]` config + validation +
  off-when-absent — Task 2.
- [x] Structural detector, pure, per-class + empty +
  core-protection invariant tested — Task 3.
- [x] Lifecycle pass piggybacked on reflection; no-op when
  off/absent; files Pending proposals + cross-cycle dedup;
  never resolves — Task 4.
- [x] Breadcrumb + Phase 78 surface extended (CLI + Web UI) —
  Task 5.
- [x] Tests across config, detector, pass integration,
  observability, IPC, CLI/Web UI — Task 6.
- [x] `docs/INSTALL.md` + `examples/aivyx.toml` updated —
  Task 6.
- [x] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 6.
- [x] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [x] DESIGN.md streak extends to twenty-eight.
- [x] PRODUCT.md streak extends to twenty-one.
- [x] Production-core streak extends to twenty-nine (new
  record) — `lib.rs` byte-identical.
- [x] Test count delta: positive — **+17 (1507 → 1524)**, in
  band (predicted ~+16-22), second consecutive in-band
  landing; lighter than Phase 80's +20 as predicted (no new
  KeyDomain).
- [x] Zero clippy warnings.
- [x] Zero new workspace deps.
- [x] Prediction-vs-reality block filled.
