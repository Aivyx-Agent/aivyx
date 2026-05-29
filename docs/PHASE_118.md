# Phase 118 — Outcome-Driven Profile/Role Refinement (Chapter E #4 — closer)

The last named Chapter E axis. After Phase 118, Chapter E
closes: the agent's self-learning surface covers Persona
(Phase 110 + 112 + 114), self-correction (Phase 115),
tool/skill relevance (Phase 116 + 117), AND now operator-
declared Profile attributes + entirely new Role definitions
— each observed from real-turn outcomes and surfaced as
operator-staged proposals.

**Contract-sensitive boundary.** Phase 118 touches the
operator-declared Profile (P13) and the operator-curated
Role config (P9). The Q-block sign-off locks the
contract-preserving posture: **always-staged for both**.
Auto-accept is not on the table this phase; the operator
owns Profile + Role and reviews every proposal before any
state changes.

**Q1 — broader scope than Recommended.** The operator
picked the (c) BOTH option over the (a) Profile-attribute-
only Recommended. Larger phase shape; closes the Chapter
E axis in one phase rather than splitting Profile-hint
substrate from Role-suggestion substrate.

## Why this, why now

- **Chapter E's last named axis.** After Phase 117, the
  post-Phase-117 deferral ledger was empty and the only
  remaining Chapter E direction named at exit was
  outcome-driven Profile/Role refinement. Phase 118
  ships against it.
- **Phase 114 already covers Profile-mirror Persona
  deltas — but not Profile config itself.** Phase 114's
  generalization let the auto-proposer fire for the six
  Profile-mirror PersonaDeltaCategory variants
  (AssistantName, OperatorProfile, CommunicationStyle,
  PrimaryUseCases, BehavioralPreferences,
  BehavioralConstraints). Those land in the Persona
  chain — they refine the agent's *learned* Profile
  mirror, not the operator-declared Profile config in
  `aivyx.toml`. Phase 118 introduces a distinct surface
  for proposing aivyx.toml Profile-config refinements
  (always-staged, operator copies into aivyx.toml on
  approval). The two surfaces co-exist; the existing
  Profile-mirror categories are untouched.
- **Roles are entirely unaddressed by auto-proposers.**
  Roles in `aivyx-config` carry `system_prompt`,
  `tool_allowlist`, `inheritance` parent chains, etc.
  No phase has shipped a proposer for new Role drafts;
  Phase 118 is the first.

## Scope (Q-block sign-off)

- **Q1 — Refinement scope:** (c) **Both Profile
  attributes AND new Role definitions** (non-Recommended;
  picked over the Profile-only Recommended). Larger
  substrate surface; both proposer paths land in one
  phase. Closes Chapter E #4 in full.
- **Q2 — Acceptance posture:** (a) **Always-staged for
  operator approval** (Recommended). Preserves the P13
  Profile-is-operator-owned contract and the P9 Role-
  config operator-curated boundary. No auto-accept
  regardless of judge confidence.
- **Q3 — Substrate pattern:** (a) **Extend Phase 114
  auto-proposer with new categories** (Recommended).
  Add `ProfileHint` and `RoleDefinitionSuggestion`
  variants to `PersonaDeltaCategory`. Reuse the
  propose/judge/route/audit pipeline; reuse the
  proposals chain + operator-review CLI. Maximum
  substrate reuse.

## Streak predictions

After Phase 117's outcome — 2-of-3 streak predictions
correct (DESIGN.md and PRODUCT.md held, lib.rs broke at
the new `AuditTag::SkillInvocation` variant) — Phase 118
calibrates against the contract-preservation posture:

- **DESIGN.md** — **Will hold.** Always-staged for both
  surfaces preserves P13 (Profile-operator-owned) and P9
  (Role-config operator-curated). No contract amendments
  this phase. Hash at entry:
  `c2be6d51def04207ab0527b4f335edf7c726920b4f102185336eded01b1b995b`.
  Prediction: streak **extends to nine** (was 8 after
  Phase 117).

- **PRODUCT.md** — **Will hold.** Same P8 (audited
  reflection envelope) covers the new proposer paths.
  Phase 118 is substrate work inside the existing
  envelope. Hash at entry:
  `6e840cef2cfcee4c483c2b4769c29cc7062daeb4bf3802c53f48e72ebfe57fb8`.
  Prediction: streak **extends to nine** (was 8).

- **Production-core `aivyx-core/src/lib.rs`** —
  **Probably holds.** No new `AuditTag` variant needed
  (the existing `SkillAutoProposal` event in
  `aivyx-audit` already carries the `category` field as
  a string from Phase 114). New draft types
  (`ProfileFieldHint`, `RoleDraft`) live inside the
  existing `pub mod skill_proposer` boundary in core;
  the module declaration is already on lib.rs:38 from
  prior phases. PersonaDeltaCategory itself lives in
  `aivyx-channel/src/persona.rs`, not core. Hash at
  entry: `d1d4373bcf54b0390b1e2c15efec4dfa50dd29f74f57aa7ca6950752603f4ae7`.
  Prediction: streak **extends to two** (was 1 after
  Phase 117's break). Honest 60/40 hold — if any new
  draft type needs top-level re-export from core, the
  40% case fires.

- **New workspace deps** — Zero.

- **Test count** — Two new proposer paths × (heuristic
  signals + LLM-judge prompts + always-staged routing
  forcing + chain round-trip + CLI rendering + audit-
  event labeling). Prediction: **+25 to +50**.

## Tasks

Roughly seven tasks plus exit + backfill:

### Task 1 — Open (this commit)

`docs/PHASE_118.md` + `docs/ROADMAP.md` Phase 118 entry +
`docs/README.md` status row.

### Task 2 — New `PersonaDeltaCategory` variants + draft payloads

- Add `ProfileHint` and `RoleDefinitionSuggestion`
  variants to the `PersonaDeltaCategory` enum in
  `crates/aivyx-channel/src/persona.rs`.
- Both list-shaped (each list entry's `value` carries
  a JSON-serialized draft payload — same pattern as
  Phase 110's `LearnedSkill`).
- New draft types in
  `crates/aivyx-core/src/skill_proposer/profile_proposer.rs`:
  - `ProfileFieldHint { field: ProfileField, suggested_value: String, rationale: String }`
  - `RoleDraft { name: String, parent: Option<String>, system_prompt_addendum: String, tool_allowlist_additions: Vec<String>, rationale: String }`
- `ProfileField` enum names the six aivyx.toml Profile-
  config fields explicitly (mirrors the Profile-config
  shape; not the Persona-chain category enum).
- Tests: enum round-trip via serde-jcs; draft-payload
  serialization stability; PersonaDeltaCategory
  variants flow through existing list-category
  validation.

### Task 3 — Heuristic signals for the new proposer paths

- Extend the existing `HeuristicSignalsMatched` struct
  in `aivyx-channel/src/skill_auto_proposer.rs` with two
  new boolean flags:
  - `profile_pattern_repeated` — true when the
    relevance-ledger (Phase 116) shows a consistent
    operator-shape signal (e.g. operator repeatedly
    asks for terse replies; same tool combos invoked
    across N+ turns).
  - `role_shape_recurring` — true when a turn outcome
    suggests the current role's tool_allowlist /
    system_prompt doesn't fit the operator's request
    pattern (e.g. repeated scope-denied events for the
    same tool).
- Signal computation reads the per-session relevance
  ledger + recent audit chain. Phase 116's substrate
  already shipped both surfaces.
- Tests: signal-firing on synthetic ledger + audit
  state; no-signal on baseline state.

### Task 4 — LLM-judge prompt extensions

- Extend the judge prompt template (in
  `aivyx-channel/src/skill_auto_proposer.rs`) to cover
  the two new categories:
  - For `ProfileHint`: judge picks one of the six
    `ProfileField` values + a suggested string + a
    short rationale. Confidence-thousandths reported
    as today.
  - For `RoleDefinitionSuggestion`: judge drafts a
    full `RoleDraft` payload (name kebab-case;
    optional parent role; system_prompt addendum;
    tool_allowlist additions).
- Judge contract documented inline — "your output is
  reviewed by the operator before any state changes;
  err on the side of explicit rationales."
- Tests: judge-response parsing for both new
  categories; malformed-response handling matches
  Phase 114's tolerance.

### Task 5 — Always-staged routing override

- In the auto-proposer's `decide_routing` step (where
  Phase 112/114 confidence-threshold gating happens):
  for `ProfileHint` and `RoleDefinitionSuggestion`,
  ALWAYS route to `Staged` regardless of confidence
  vs threshold. No auto-accept path.
- The override is hard-coded at the category level —
  not operator-configurable. The P13/P9 contract is
  the source of truth; this is contract preservation,
  not policy.
- Audit event records the routing outcome accurately
  (`SkillAutoProposalOutcomeSummary::Staged` with
  `category: Some("ProfileHint")` or
  `Some("RoleDefinitionSuggestion")`).
- Tests: high-confidence judge response still routes
  to Staged for these two categories; the override is
  unconditional.

### Task 6 — Proposals-chain integration + CLI surfacing

- Both new categories land in the existing proposals
  chain via the standard PersonaDeltaProposal substrate
  (Phase 60). No new chain shape.
- `aivyx persona proposals list` includes the new
  categories with appropriate labels.
- `aivyx persona proposals show <id>` renders the
  JSON-serialized draft payload with a human-readable
  formatter for each draft type.
- `aivyx persona proposals approve <id>` works
  identically — the approved delta enters the Persona
  chain as a List append. The operator copies the
  rendered payload into `aivyx.toml` themselves (Phase
  118 does NOT auto-mutate `aivyx.toml`).
- INSTALL.md gets a section explaining the workflow:
  "approve the proposal → review the rendered draft
  → copy into aivyx.toml → restart the daemon".
- Tests: CLI rendering for both new categories; chain
  round-trip via the persona chain primitives;
  approval-side flow lands the entry in folded state.

### Task 7 — Scripted e2e + INSTALL.md sweep + exit

- Scripted e2e: build a synthetic session with the
  signals that fire each new proposer path; verify the
  proposer fires + judge runs + routing forces Staged
  + chain entry round-trips + CLI renders the draft.
- INSTALL.md: new "Profile/Role refinement (Phase 118)"
  section covering the operator workflow (chain →
  review → copy to aivyx.toml).
- Exit: PHASE_118.md prediction-vs-reality + ROADMAP
  freeze + README flip + hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Refinement scope:** (c) **Both Profile
  attributes AND new Role definitions** (non-
  Recommended; broader than the Profile-only
  Recommended).
- **Q2 — Acceptance posture:** (a) **Always-staged for
  operator approval** (Recommended). Preserves P13 +
  P9.
- **Q3 — Substrate pattern:** (a) **Extend Phase 114
  auto-proposer with new categories** (Recommended).
  Reuse existing pipeline.

## Exit criteria

- [x] `docs/PHASE_118.md` + ROADMAP Phase 118 entry +
  docs/README status row — Task 1 (`bd35941`).
- [x] `ProfileHint` + `RoleDefinitionSuggestion`
  PersonaDeltaCategory variants + draft payloads —
  Task 2 (`a22cf2d`).
- [x] Heuristic signal extensions — Task 3 (`f4b8278`).
- [x] LLM-judge prompt extensions for both new
  categories — Task 4 (`d63dfef`).
- [x] Always-staged routing override + per-category
  integration — Task 5 (`006b54e`).
- [x] Proposals-chain integration + CLI rendering for
  both new categories — Task 6 (`1454158`).
- [x] Scripted e2e + INSTALL.md sweep — Task 7 (this commit).
- [x] Q1/Q2/Q3 resolved with operator sign-off pre-Task
  2 (Q1c non-Recommended, Q2a/Q3a Recommended).
- [x] DESIGN.md streak — **HELD as predicted**.
  `c2be6d51…` unchanged. Streak extends 8 → 9.
- [x] PRODUCT.md streak — **HELD as predicted**.
  `6e840cef…` unchanged. Streak extends 8 → 9.
- [x] `aivyx-core/src/lib.rs` streak — **HELD as
  predicted** (60/40 hold; the 60% case fired).
  `d1d4373b…` unchanged. Streak extends 1 → 2.
- [x] Zero new workspace dependencies.
- [ ] Test count delta `+60` (2188 → 2248) — **above**
  the predicted `+25 to +50` range. Honest scope
  reporting: Q1c's broader-than-Recommended scope (both
  Profile attributes AND new Role definitions) plus the
  load-bearing per-category integration in Task 5
  produced more test surface than I anticipated at
  sign-off. Each test pins one concrete behavior; none
  are redundant. Phase 6 Q5 honesty.
- [x] Zero clippy warnings.
- [x] **Chapter E closed.** All four named Chapter E
  axes shipped: Persona auto-proposer generalization
  (Phase 114), self-correction loop on failed turns
  (Phase 115), tool/skill selection learning from
  outcomes (Phase 116 + 117), and outcome-driven
  Profile/Role refinement (Phase 118).

## Prediction vs reality

**All three streak predictions correct.** First Phase
since Phase 92 to land 3-of-3 on streak predictions; the
contract-preservation posture (always-staged + reuse
existing categories) kept the surface inside its
envelope.

- **DESIGN.md** — HELD as predicted (`c2be6d51…`
  unchanged). The always-staged Q2(a) sign-off preserved
  P13 + P9 without contract amendments; new
  PersonaDeltaCategory variants live in aivyx-channel,
  not in the contract. Streak: 8 → 9.
- **PRODUCT.md** — HELD as predicted (`6e840cef…`
  unchanged). P8 envelope. Streak: 8 → 9.
- **`aivyx-core/src/lib.rs`** — HELD as predicted
  (60/40 hold case fired). `d1d4373b…` unchanged. The
  new ProfileField + ProfileFieldHint + RoleDraft types
  fit inside the existing `pub mod skill_proposer`
  boundary already declared on lib.rs:38; no new
  AuditTag variant needed since `SkillAutoProposal`'s
  `category` field is already `Option<String>` from
  Phase 114. Streak: 1 → 2 (first re-established
  lib.rs streak since Phase 117's break).

**Test count `+60` is ABOVE the predicted `+25 to +50`
range.** Phase 6 Q5 honesty: Q1c's broader scope (both
Profile attributes AND Role suggestions, vs the
Recommended Profile-only) doubled the proposer-path test
surface, and Task 5's per-category integration added
both runtime AND aivyx-config TOML test surface (the
load-bearing PerCategoryConfigSet extension hit both
sides). Each test pins one concrete behavior; none are
redundant. The overshoot is honest reflection of the
broader scope, not test-count inflation.

**Q-block went through fully as operator-picked.** Q1c
non-Recommended bundle of both surfaces shipped together;
Q2a Recommended always-staged contract preservation
hard-coded; Q3a Recommended substrate reuse via existing
Phase 114 pipeline.

## Chapter E closed

All four named Chapter E axes shipped across phases
114-118:

1. **Phase 114** — Persona auto-proposer generalization
   (the eleven-category fan-out + LLM judge picking).
2. **Phase 115** — Self-correction loop on failed turns
   (the negative-feedback path; failure-kind
   classification).
3. **Phase 116 + 117** — Tool/skill selection learning
   from outcomes (relevance ledger + live-prompt pipe +
   per-skill tracking).
4. **Phase 118** — Outcome-driven Profile/Role
   refinement (ProfileHint + RoleDefinitionSuggestion
   always-staged proposals).

The post-Phase-118 deferral ledger is **empty**. No
Phase-118-internal deferrals anticipated; reality
matches anticipation. Chapter E's substrate fully shipped.

## Chapter E direction after Phase 118

After Phase 118, Chapter E closes. The post-Phase-118
deferral ledger SHOULD be empty (no named deferrals
anticipated at open; reality TBD at exit). The next
phase opens against:

- **Channel Activation Milestone** — still has not run.
  Six phases of Chapter D/E substrate + Phase 118
  shipping without real-use signal. Running the
  milestone surfaces whether the self-learning loop
  helps in practice.
- **A new thematic chapter (Chapter F)** —
  observability, deeper memory, multi-agent shapes,
  distribution prep, whatever the operator's pressure
  shapes next.
- **Operator pressure** — Phase-by-phase decision at
  the next sign-off.
