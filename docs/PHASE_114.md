# Phase 114 — Persona Auto-Proposer Generalization (Chapter E opener)

Extends Phase 112's auto-proposer substrate from
`LearnedSkill` to **every** `PersonaDeltaCategory` variant.
After this phase, the agent self-learns at the entire
Persona layer — not just at the skill axis. Operator picks
the configuration shape per-category per Q1b sign-off.

**Project vision continuation:** Phase 112 closed the
self-learning loop for skills. Phase 113 made it
operator-configurable. Phase 114 broadens the loop to the
full 11-category Persona substrate (Phase 56-110's
identity layer), opening **Chapter E — Self-Improvement
Loop Deepening**. The framing chosen by the operator at
sign-off: "Self-improvement beyond skills" — the
self-learning critical-path piece that turns the skill-
specific Phase 112 framework into a general Persona-axis
self-learner.

The phase opens Chapter E. Mirrors how Chapter D opened at
Phase 105 — a thematic arc that closes a specific
substrate gap. Chapter E's lens is the **self-improvement
loop**; first phase generalizes the auto-proposer to all
PersonaDeltaCategory variants.

## Why this, why now

- **Project-vision critical path.** The Phase 112 substrate
  is skill-shaped. The 10 other PersonaDeltaCategory
  variants (BehavioralPreferences, LearnedContext,
  CommunicationAdaptations, etc.) carry the agent's
  evolving character; today they only land via cron-
  fired reflection or operator-side proposals. Bringing
  the inline-at-turn-boundary auto-proposer to them
  closes the symmetric half of the self-learning loop.
- **Substrate is hot and tested.** Phase 112's pipeline +
  Phase 113's TOML/inspection surface are still warm.
  Generalizing now reuses the freshly-shipped
  `run_auto_propose_pipeline` orchestration and the
  `[skills.auto_propose]` TOML loader patterns without
  re-learning either.
- **Backward-compatible audit chain.** The existing
  `AuditEvent::SkillAutoProposal` variant takes one new
  optional field (`category`) with `#[serde(default,
  skip_serializing_if = "Option::is_none")]`, so existing
  audit chains continue to verify byte-identically while
  new entries carry the category. Phase 92's
  `supersedes_proposal_id` precedent for wire-compatible
  audit-chain extensions.
- **Q-block at sign-off (one Recommended, two non-
  Recommended):**
  - **Q1b — Per-category config block** (non-Recommended;
    operator picked over the uniform single-config
    Recommended). Per-category TOML thresholds + enable
    flags. More flexible; more substrate.
  - **Q2a — Reuse the four Phase 112 signals**
    (Recommended). Same heuristic gate; the LLM judge
    handles per-category signal detection from the turn
    summary.
  - **Q3a — Reuse `persona.propose` for non-skill,
    `skills.propose` for skill** (Recommended). No new
    scope base; KNOWN_BASES stays at 49.

## Scope (Q-block sign-off)

- **Q1 — Per-category config shape:** (b) **Per-category
  TOML config block** (non-Recommended; operator picked
  over Q1a uniform). New `[persona.auto_propose]` section
  with per-category sub-sections — each category carries
  its own `enabled` + `auto_accept_confidence_threshold`.
  More flexible. Implication: aivyx-config gains a
  substantially larger TOML schema than Phase 113's
  `[skills.auto_propose]`.

- **Q2 — Heuristic signals:** (a) **Reuse the four
  Phase 112 signals.** Tool-call count, distinct tool-id
  count, duration, gate-resolve. Cheap, uniform, the
  judge handles per-category specifics from the turn
  summary. Per-category specialized signals (e.g. tone
  detection for `CommunicationAdaptations`) is a
  deferral if operator pressure surfaces.

- **Q3 — Scope-base picture:** (a) **Reuse existing
  bases.** `skills.propose` for `LearnedSkill`;
  `persona.propose` for the other 10 categories. The
  auto-proposer dispatches on the judge-returned category
  to pick the right base. No A3 amendment needed.

## Streak predictions

- **DESIGN.md** — **Will hold.** No D-section touch. The
  auto-proposer is inside the Phase 110 D4-skills section
  envelope; generalization to other categories is inside
  the Phase 59 D-section Persona envelope. Hash at entry:
  `c2be6d51def04207ab0527b4f335edf7c726920b4f102185336eded01b1b995b`.
  Prediction: streak **extends to five** (was 4 after
  Phase 113).

- **PRODUCT.md** — **Will hold.** P8 (Outcome-Driven
  Audited Reflection) is the natural envelope; Phase
  112 + 113 already extended P8 inline-fired delivery
  without contract amendment. Phase 114 broadens the
  set of delta categories the auto-proposer can write
  into, all inside P8's "approved deltas land in chain"
  shape. Hash at entry:
  `6e840cef2cfcee4c483c2b4769c29cc7062daeb4bf3802c53f48e72ebfe57fb8`.
  Prediction: streak **extends to five** (was 4 after
  Phase 113).

- **Production-core `aivyx-core/src/lib.rs`** — **Will
  hold.** `skill_proposer` module is already `pub mod`'d
  in `lib.rs`; Phase 114 extends `judge.rs` (category-
  picking) and adds chain-write dispatch in
  `aivyx-channel`, but `lib.rs` itself stays unchanged.
  Hash at entry:
  `deab80d8dded7a59746771a3eb883aad2a55c364ca3743b9bd73b3c7c8837ece`.
  Prediction: streak **extends to three** (was 2 after
  Phase 113).

- **New workspace deps** — Zero. Generalization only.

- **Test count** — Positive. Per-category TOML parser
  tests, judge category-picking tests, chain-write
  dispatch tests per category, audit-event variant
  extension tests, scripted e2e covering at least three
  different categories. Prediction: **+30 to +50**.

## Tasks

Seven sub-tasks plus exit + backfill, comparable to
Phase 112's substrate-heavy shape:

### Task 1 — Open (this commit)

`docs/PHASE_114.md` + `docs/ROADMAP.md` Chapter E + Phase
114 entry + `docs/README.md` status row.

### Task 2 — Judge prompt generalization

- `aivyx-core/src/skill_proposer/judge.rs` —
  `JudgeRequest.existing_skills` becomes
  `JudgeRequest.existing_persona_summary` (snapshot of
  current Persona state across all categories; lets the
  judge consider any category for proposal).
- `JudgeResponse` gains
  `category: Option<String>` carrying the category
  label (e.g. `"BehavioralPreferences"`,
  `"LearnedSkill"`). `None` means the judge declined to
  pick a category (treat as not-worth-proposing).
- `proposed_skill: Option<SkillDraft>` extended to
  `proposed_op: Option<SerializedProposedOp>` carrying
  the JSON-serialized `PersonaDeltaOp` the judge
  drafted. For `LearnedSkill`, the op wraps a
  `SkillDraft`; for `BehavioralPreferences`-and-similar
  (list categories), it wraps an `AppendList` value;
  for scalar categories (e.g. `AssistantName`), it
  wraps a `SetScalar` value.
- Judge system prompt updated: explains the full
  category enum, criteria for each, and asks the judge
  to pick the right category.
- Tests: each category's worth-proposing branch parses;
  each category's draft shape parses; backward-compat
  parser still accepts the old `proposed_skill` shape
  for chain replay.

### Task 3 — Per-category TOML config block (`[persona.auto_propose]`)

- New `[persona.auto_propose]` section in `aivyx-config`
  with per-category sub-sections:
  ```toml
  [persona.auto_propose]
  enabled = true
  judge_model = "claude-haiku-4-5"
  judge_max_tokens = 800
  fuzzy_match_threshold = 0.80

  [persona.auto_propose.heuristic]
  # ... Phase 112's four signals ...

  [persona.auto_propose.learned_skill]
  enabled = true
  auto_accept_confidence_threshold = 0.85

  [persona.auto_propose.behavioral_preferences]
  enabled = true
  auto_accept_confidence_threshold = 0.90

  [persona.auto_propose.assistant_name]
  enabled = false  # scalar; big effect; default off
  auto_accept_confidence_threshold = 0.99
  ```
- Per-category defaults are *deliberately conservative*
  for the high-impact scalar categories
  (AssistantName, OperatorProfile, CommunicationStyle —
  default `enabled = false` for these; the operator must
  explicitly opt in). List categories default
  `enabled = true` since they're additive.
- Backward compatibility: the existing
  `[skills.auto_propose]` section stays a valid alias
  for `[persona.auto_propose.learned_skill]` so Phase 113
  configs keep working without rewrite.
- Tests: each category's parse cases (default, explicit,
  threshold-validation); the `[skills.auto_propose]`
  alias still works; per-category `enabled = false`
  routes to "category disabled" outcome.

### Task 4 — Chain-write dispatch by category

- `aivyx-channel/src/skill_auto_proposer.rs` — split
  `write_auto_accepted_skill` into a category-dispatching
  `write_auto_accepted_delta` that:
  - For `LearnedSkill`: keeps the existing
    `LearnedSkill::to_json_value` wrap + `AppendList`
    delta op.
  - For list categories (BehavioralPreferences,
    LearnedContext, etc.): emits a plain `AppendList
    { value: <judge-drafted text> }` op.
  - For scalar categories (AssistantName,
    OperatorProfile, CommunicationStyle): emits a
    `SetScalar { value: Some(<text>) }` op.
- Same for `write_staged_delta` (the Staged path).
- Tests: dispatch table coverage — each category lands
  with the right `PersonaDeltaOp` variant in the chain.

### Task 5 — Audit event extension (backward-compatible)

- `aivyx-audit::AuditEvent::SkillAutoProposal` gains
  `category: Option<String>` with `#[serde(default,
  skip_serializing_if = "Option::is_none")]`. Existing
  audit chains keep verifying byte-identically (the field
  is absent from existing entries' canonical bytes);
  Phase 114+ entries carry the category. Phase 92's
  `supersedes_proposal_id` precedent.
- The variant name `SkillAutoProposal` stays for chain
  backward compatibility — renaming would break old
  chains. The doc-comment is updated to note "Phase
  114 generalized this to all PersonaDeltaCategory
  variants; category populated for Phase 114+ entries."
- `event_type_label` returns "SkillAutoProposal"
  unchanged.
- Tests: backward-compat round-trip (an entry with no
  `category` field deserializes to `category: None`
  and re-serializes to the original bytes); new
  entries carry the category through serde-jcs.

### Task 6 — Daemon wiring + operator-surface

- Bin wiring: `[persona.auto_propose]` config →
  `SkillAutoProposerContext` construction. Falls back
  to the Phase 113 `[skills.auto_propose]` section if
  it's present and the new section isn't (the alias
  Task 3 ships).
- Operator-surface: extend the existing Phase 113
  `aivyx persona list --auto-only` with an optional
  `--category <name>` filter so operators can scope
  to a specific axis. (Deferral if scope creeps:
  `--category` lands in a follow-on; the
  `delta_id`-prefix + audit-export filter together
  cover the inspection surface today.)

### Task 7 — Scripted e2e + INSTALL.md sweep + exit

- Scripted e2e at least three categories: LearnedSkill
  (backward compat), BehavioralPreferences (the most
  common list category target), AssistantName (scalar
  with default-off; operator-opt-in path).
- INSTALL.md update: rename/extend the Phase 112-113
  "Skill auto-proposer" section to "Persona
  auto-proposer (Phases 112-114)" with per-category
  TOML examples.
- Exit: PHASE_114.md prediction-vs-reality + ROADMAP
  freeze + README flip + hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Per-category config shape:** (b) **Per-category
  config block** (non-Recommended). New
  `[persona.auto_propose]` TOML section with per-category
  sub-sections. More flexible; more substrate; the
  operator's deliberate trade.
- **Q2 — Heuristic signals:** (a) **Reuse Phase 112's
  four signals.** Same gate; the judge handles per-
  category signal detection from the turn summary.
- **Q3 — Scope-base picture:** (a) **Reuse existing
  bases.** `skills.propose` for `LearnedSkill`;
  `persona.propose` for the other 10 categories. No new
  base; KNOWN_BASES stays at 49.

## Exit criteria

- [x] `docs/PHASE_114.md` + ROADMAP Chapter E + Phase
  114 entry + docs/README status row — Task 1 (`e28d3e8`).
- [x] Judge prompt generalized to category-picking;
  `JudgeResponse.category` field + `proposed_draft`
  generalization — Task 2 (`2960391`).
- [x] Per-category `[persona.auto_propose.<category>]`
  TOML config + alias compatibility with Phase 113's
  `[skills.auto_propose]` — Task 3 (`8beb407`).
- [x] Chain-write dispatch by category — Task 4
  (`6206577`).
- [x] Audit-event backward-compatible extension with
  `category: Option<String>` — Task 5 (`8830b4e`).
- [x] Daemon wiring + operator-surface flag — Task 6
  (`b7f09e4`).
- [x] Scripted e2e covering at least three category
  types (skill + list + scalar) — Task 7.
- [x] All three Q-block questions resolved with operator
  sign-off pre-Task 2 (Q1b, Q2a, Q3a recorded above).
- [x] DESIGN.md streak extends to five — **HELD as
  predicted**. `c2be6d51…` unchanged.
- [x] PRODUCT.md streak extends to five — **HELD as
  predicted**. `6e840cef…` unchanged.
- [x] `aivyx-core/src/lib.rs` streak extends to three —
  **HELD as predicted**. `deab80d8…` unchanged.
- [x] Zero new workspace dependencies.
- [x] Test count delta positive — `+37` (2068 → 2105),
  inside the `+30 to +50` predicted range.
- [x] Zero clippy warnings.
- [x] **The self-learning loop generalizes from skill
  to full Persona surface.** Chapter E opens with the
  most natural extension of Phase 112's substrate;
  every PersonaDeltaCategory variant is now reachable
  by the auto-proposer per the operator's per-category
  config.

## Prediction vs reality

**Three streak predictions; all three held — third
all-hold result in a row.** Phase 112, 113, and 114
each saw every prediction hit, the first three-streak-
streak since Phase 56–58 (the original P13/P14 amendment
sequence). Substrate momentum is at its highest point of
the project.

- **DESIGN.md** — Held as predicted. `c2be6d51…`
  unchanged. The Phase 114 generalization slotted into
  the existing D4 (Phase 110 skills section) + D5
  (Phase 59-60 Persona section) envelopes; no D-section
  edit needed. Streak: 4 → 5.
- **PRODUCT.md** — Held as predicted. `6e840cef…`
  unchanged. P8 (Outcome-Driven Audited Reflection)
  covers inline-fired Persona reflection identically
  to cron-fired; generalizing the categories the
  auto-proposer can write into is inside P8's commit.
  Streak: 4 → 5.
- **`aivyx-core/src/lib.rs`** — Held as predicted.
  `deab80d8…` unchanged. The `skill_proposer` module
  was already `pub mod`-exposed at Phase 112; Phase 114
  extended `judge.rs` (new types + helpers) without
  touching `lib.rs` itself. Streak: 2 → 3.

**Test count `+37` is inside the `+30 to +50` band.**
Breakdown:
- `aivyx-core/src/skill_proposer/judge.rs` — +8 tests
  (the existing 21 grew to 29 covering category-picking,
  ProposedDraft variants, ExistingPersonaSnapshot
  round-trip, etc.).
- `aivyx-config` — +7 tests (per-category section
  parse, defaults, override, threshold-validation,
  alias-coexistence, lookup-helper).
- `aivyx-channel/src/skill_auto_proposer.rs` — +12 tests
  (per-category routing branches; From-conversion
  population; build_proposed_op dispatch table;
  parse_persona_delta_category coverage).
- `aivyx-audit` — +3 tests (Phase 114 backward-compat:
  category-set round-trip, category-None serializes as
  absent field, pre-Phase-114 decoder compat).
- `aivyx-channel/tests/skill_auto_proposer_e2e.rs` —
  +4 tests (list_category_auto_accepts,
  scalar_category_disabled_by_default,
  scalar_category_explicitly_enabled,
  operator_profile_disabled).
- Other: +3 from Task 2's prompt-shape extensions and
  helper tests.

**Q-block resolved exactly as signed off.** No mid-phase
shifts. Q1b (per-category TOML) added the most substrate
(new aivyx-config types + per-category routing +
defaults table + alias compatibility) but the cost
landed cleanly inside the predicted test-count band.

## Chapter E direction after Phase 114

Phase 114 is the first phase of Chapter E. The remaining
Chapter E axes named at sign-off (operator-pressure-
shaped, picked at each phase exit):
- Tool/skill selection learning from outcomes.
- Self-correction loop on failed turns.
- Outcome-driven Profile/Role refinement proposals.

The post-Phase-114 ledger has zero named deferrals; the
next Chapter E phase opens against whichever axis the
operator picks against observed value from Phase 114's
auto-proposer in real use.
