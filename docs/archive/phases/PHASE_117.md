# Phase 117 — Phase 116 Deferral Closeout (live-prompt pipe + per-skill tracking)

Closes both Phase-116-internal deferrals in one phase per
Q1b sign-off. After Phase 117, the Phase 116 tool/skill
relevance substrate reaches the LLM in live turns AND
records per-skill outcomes accurately — the gaps the open
doc named at Phase 116 exit get filled.

**Why bundle both:** the operator picked the non-
Recommended Q1b at sign-off — closing both deferrals
together rather than serial focused phases. The bet is on
"one phase, both gaps closed" outweighing the cleaner
focused-phase shape. Phase 113's Operator-Surface Polish
precedent: batching small named deferrals into one cleanup
phase is cheaper than carrying them across more substrate
work.

**This is not a substrate-novelty phase.** Phase 117 ships
focused integration work that takes the Phase 116
substrate-substrate-substrate-substrate (Tasks 2-6) and
plumbs it into live use. Standalone phase past Chapter E's
arc rather than Chapter E #4 — the last named Chapter E
axis (outcome-driven Profile/Role refinement) opens
operator-pressure-shaped after Phase 117.

## Why this, why now

- **Phase 116 ships substrate but doesn't yet augment live
  turns.** The relevance ledger records outcomes after each
  turn; the renderer can build the system-prompt section
  from an entry; but the planner's system prompt is fixed
  at construction time so the rendered section never
  reaches the LLM. Phase 117 closes that gap.
- **Per-skill tracking gap.** Phase 116's recording path
  records `skills.invoke` as a single tool identifier
  rather than per-skill rows. The ledger schema's
  `RelevanceSurfaceKind::Skill` variant is in place; Phase
  117 wires the actual skill names through.
- **Operator value depends on closing both.** With just
  the live-prompt pipe, the relevance section shows all
  skill invocations bundled under `skills.invoke`. With
  just per-skill tracking, the section doesn't reach the
  LLM. Both gaps need to close for Phase 116's intended
  operator-value to land.

## Scope (Q-block sign-off)

- **Q1 — Bundle scope:** (b) **Both deferrals together**
  (non-Recommended; operator picked over focused option).
  Live-prompt pipe + per-skill tracking ship in the same
  phase. Larger scope; cleaner ledger + prompt surface
  immediately.

## Streak predictions

After Phase 116's two-positive-surprise streak (DESIGN.md
and PRODUCT.md held against predicted break), my
predictions for Phase 117 are calibrated more carefully —
the substrate often "fits inside" existing envelopes more
neatly than the open doc anticipates.

- **DESIGN.md** — **Will probably hold.** The new
  `DynamicSystemPromptBuilder` trait lives in
  `aivyx-core/src/agent.rs` alongside the existing Agent
  trait; the `SkillInvocation` audit-event variant is an
  additive enum extension Phase 67's
  `AutoNotifyDispatched` precedent already established.
  Neither touches DESIGN.md proper. Hash at entry:
  `c2be6d51def04207ab0527b4f335edf7c726920b4f102185336eded01b1b995b`.
  Prediction: streak **extends to eight** (was 7 after
  Phase 116).

- **PRODUCT.md** — **Will hold.** Same P8 envelope. Phase
  117 is integration work on substrate that already shipped
  inside P8/P10. Hash at entry:
  `6e840cef2cfcee4c483c2b4769c29cc7062daeb4bf3802c53f48e72ebfe57fb8`.
  Prediction: streak **extends to eight** (was 7).

- **Production-core `aivyx-core/src/lib.rs`** —
  **Probably holds.** The new trait lives inside the
  existing `pub mod agent` boundary; if I'm careful with
  re-exports, `lib.rs` stays byte-identical. Hash at entry:
  `b1169a1ec58779bf30353784d2910ff3cd1e19647c2d707fcfb5c4f691ebdd91`.
  Prediction: streak **extends to two** (was 1 after
  Phase 116). Could break if a re-export turns out
  necessary; honest 70/30 split.

- **New workspace deps** — Zero.

- **Test count** — Substrate-heavy with the new trait +
  audit event variant + per-skill capture path + daemon
  wiring + e2e. Prediction: **+30 to +60**.

## Tasks

Seven sub-tasks plus exit + backfill:

### Task 1 — Open (this commit)

`docs/PHASE_117.md` + `docs/ROADMAP.md` Phase 117 entry +
`docs/README.md` status row.

### Task 2 — `DynamicSystemPromptBuilder` trait

- New trait in `aivyx-core/src/agent.rs`:
  ```rust
  #[async_trait]
  pub trait DynamicSystemPromptBuilder: Send + Sync {
      /// Return a per-turn system-prompt addendum keyed off
      /// the just-received user message. Caller concatenates
      /// to the static base prompt before passing to the
      /// planner. Returning the empty string suppresses the
      /// addendum.
      async fn build(&self, user_message: &Message) -> String;
  }
  ```
- `ConcreteAgent` gains
  `dynamic_prompt_builder: Option<Arc<dyn
  DynamicSystemPromptBuilder>>` field.
- Tests: trait shape; ConcreteAgent constructs with /
  without the builder.

### Task 3 — Per-turn prompt assembly in the agent

- `ConcreteAgent::turn(...)`: at entry, if
  `dynamic_prompt_builder` is Some, await `build(&msg)`
  and concatenate the addendum to the planner's static
  prompt before the first step.
- The addendum updates per-step within the turn? No —
  fixed at turn entry to keep prompt-cache friendly.
  Re-assembly is per-turn, not per-step.
- Failure-isolated: a builder panic / future-error
  defaults to no addendum (logged WARN).
- Tests: agent with no builder uses static prompt
  byte-identical to Phase 116; agent with builder
  appends the addendum.

### Task 4 — `RelevancePromptBuilder` implementation

- New `aivyx-channel/src/relevance_prompt_builder.rs`:
  - `pub struct RelevancePromptBuilder { ledger:
    Arc<PersistentToolRelevanceLedger>, config:
    ToolRelevanceConfig }`
  - `impl DynamicSystemPromptBuilder for
    RelevancePromptBuilder` — calls
    `aivyx_core::relevance::keyword_key(text,
    config.max_keywords)`, then
    `render_relevance_section(...)` with the per-turn
    keyword key.
- Tests: builder returns "" for empty keyword key;
  returns rendered section for matched key + populated
  ledger; honors per-section + per-outcome config.

### Task 5 — Per-skill tracking via audit-event extension

- New `AuditEvent::SkillInvocation { turn_id,
  session_id, skill_name }` variant in `aivyx-audit`.
  Additive enum extension (Phase 67 `AutoNotifyDispatched`
  precedent); existing chains decode unchanged.
- `skills.invoke` tool (Phase 110) extended: after the
  skill is rendered into the outcome, the tool's
  `ToolContext::audit_log` gets a `SkillInvocation`
  entry written alongside the regular `ToolCall` entry.
  The skill_name lands in cleartext (operator-readable;
  the rest of the input is the same hashed payload as
  before).
- `record_turn_outcomes` extended: SkillInvocation
  entries get recorded as `RelevanceSurfaceKind::Skill`
  with the skill_name as identifier.
- Tests: audit-event round-trip; skills.invoke emits
  both ToolCall + SkillInvocation; record_turn_outcomes
  populates Skill rows separately from Tool rows.

### Task 6 — Daemon wiring

- Bin: when `[tool_relevance] enabled = true` AND the
  ledger handle is constructed, also construct a
  `RelevancePromptBuilder` and thread it into
  `ConcreteAgent`. The agent type may need a new
  builder method for the dynamic prompt builder; minimal
  surface change.
- Tests: scripted construction that confirms the
  builder is wired when config is on, None when off.

### Task 7 — Scripted e2e + INSTALL.md sweep + exit

- Scripted e2e: build a real ledger with one keyword
  key + outcomes; construct a `RelevancePromptBuilder`;
  call `build(msg)` with a user message containing the
  matching keywords; assert the returned addendum
  contains the expected tool names and counts.
- INSTALL.md: refresh the "Tool/skill relevance hints"
  section to note that Phase 117 closes the live-prompt
  pipe + per-skill tracking deferrals; the substrate
  reaches the LLM in live turns from this phase
  forward.
- Exit: PHASE_117.md prediction-vs-reality + ROADMAP
  freeze + README flip + hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Bundle scope:** (b) **Both deferrals together**
  (non-Recommended). Live-prompt pipe + per-skill
  tracking close in the same phase.

## Exit criteria

- [x] `docs/PHASE_117.md` + ROADMAP Phase 117 entry +
  docs/README status row — Task 1 (`6d9f4d4`).
- [x] Live-prompt pipe shipped — Task 2 + Task 3
  combined. **Honest scope reduction:** the open doc
  anticipated a new `DynamicSystemPromptBuilder` trait
  but the existing `aivyx_core::llm_planner::
  SystemPromptRefiner` (Phase 79) already covered the
  per-turn dynamic-prompt use case. Phase 117 extended
  the trait with a `base_prompt: &str` parameter so
  extending refiners can compose without rebuilding the
  base from scratch. (`4dff0a1` + `4c0d61b`).
- [x] `RelevancePromptRefiner` implementation — Task 3
  in commit (`4c0d61b`).
- [x] Per-skill tracking — Task 4
  (`AuditEvent::SkillInvocation` variant +
  `skills.invoke` audit emission + record_turn_outcomes
  per-skill recording) (`36b3340`).
- [x] Daemon wiring + chained inner refiner — Task 5
  (`c6317c5`). When the operator has [tool_relevance]
  armed, RelevancePromptRefiner installs as the
  planner's system_prompt_refiner; if Phase 79
  PersonaContextRefiner is ALSO armed, it chains as the
  inner so both refinements ride on the single refiner
  slot.
- [x] INSTALL.md sweep — Task 7 (this commit).
- [x] Q1 resolved with operator sign-off pre-Task 2
  (Q1b recorded above).
- [x] DESIGN.md streak — **HELD as predicted**.
  `c2be6d51…` unchanged. Streak extends 7 → 8.
- [x] PRODUCT.md streak — **HELD as predicted**.
  `6e840cef…` unchanged. Streak extends 7 → 8.
- [x] `aivyx-core/src/lib.rs` streak — **BROKE**
  against predicted hold (70/30 risk acknowledged at
  sign-off). The new `AuditTag::SkillInvocation`
  variant in the core forward-declared audit enum
  touched `lib.rs`. Streak resets 1 → 1 (broken again
  in a row). The 30% case landed.
- [x] Zero new workspace dependencies.
- [ ] Test count delta `+14` (2174 → 2188) — **below**
  the predicted `+30 to +60` range. Honest scope
  reduction in Task 2 (reusing the existing
  SystemPromptRefiner trait rather than introducing a
  new trait + ConcreteAgent surface changes) cut a
  significant amount of test surface. The reduced
  scope reaches the same end state more directly; the
  test surface lives where it lives. Phase 6 Q5
  honesty.
- [x] Zero clippy warnings.
- [x] **Both Phase-116-internal deferrals closed.**
  The Phase 116 relevance substrate now reaches the
  LLM in live turns via the Phase 79
  SystemPromptRefiner slot; per-skill outcomes record
  accurately through `AuditEvent::SkillInvocation`;
  the operator-value Phase 116 aimed at lands in full.

## Prediction vs reality

**Two of three streak predictions correct; one broke
against predicted hold.** The lib.rs break was already
hedged at the open doc (70/30 split); the 30% case
fired.

- **DESIGN.md** — HELD as predicted (`c2be6d51…`
  unchanged). The new SystemPromptRefiner trait
  extension lives inside `llm_planner.rs`; the
  SkillInvocation variant is additive (Phase 67
  precedent). Streak: 7 → 8.
- **PRODUCT.md** — HELD as predicted (`6e840cef…`
  unchanged). P8 envelope. Streak: 7 → 8.
- **`aivyx-core/src/lib.rs`** — BROKE against predicted
  hold. `b1169a1e…` → `d1d4373b…`. The new
  `AuditTag::SkillInvocation` variant touched the
  forward-declared audit enum in lib.rs. The open doc
  acknowledged 30% break risk; honest reality.

**Test count `+14` is BELOW the predicted `+30 to +60`
range.** Phase 6 Q5 honesty: the Task 2 scope reduction
(reusing existing SystemPromptRefiner trait rather than
introducing a new trait + agent surface) cut the test
surface significantly. The substrate end state is the
same; the simpler path got there with fewer test
artifacts. The +14 is honest reflection of the reduced
scope, not a missed test target.

**Q-block went through fully as operator-picked.** Q1b
bundle both deferrals (non-Recommended) — operator
deliberately picked over the focused single-deferral
shape. Both deferrals closed together, as committed.

## Chapter E direction after Phase 117

After Phase 117, the last named Chapter E axis remains:
**outcome-driven Profile/Role refinement**. The
post-Phase-117 deferral ledger is empty — both Phase
116 named deferrals closed. The next phase opens
against the last Chapter E axis OR against operator
pressure — Phase-by-phase decision at the next
sign-off.
