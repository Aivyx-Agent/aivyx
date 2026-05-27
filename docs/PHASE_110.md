# Phase 110 — Skills Auto-Creation (Reflection Staging)

The **last Chapter D item** — the substrate-design piece the
ROADMAP flagged with "highest amendment risk of the
Chapter D set." Extends the existing reflection layer
(Phase 29 propose / apply, Phase 30 role-mutation, Phase 59
Persona propose, Phase 60 Persona revert) with a **skills
primitive**: procedural patterns the agent drafts after
complex turns, staged as Persona-style deltas the operator
approves via the existing persona-proposal surface, then
rendered into the agent's system prompt alongside Persona
content on every subsequent turn.

Mid-ground between Hermes's autonomous skill creation (the
agent creates and uses skills with no operator gate) and
Aivyx's current per-action reflection propose/apply
(skills are not first-class today). Stays inside
PRODUCT.md P8's "outcome-driven audited reflection"
envelope — the agent doesn't apply skills autonomously;
it proposes, the operator approves through the Phase 70
mission-gate proposal surface, the chain records both
halves.

## Why this, why now

- **Chapter D's last item.** Phase 105 (audit export), Phase
  106 (MCP recipes), Phase 107 (Discord), Phase 108
  (Slack), Phase 109 (tool breadth + A12) closed five of
  the six Hermes-comparison-driven gaps. Phase 110 closes
  the sixth.
- **The substrate is genuinely ready.** Phase 59's Persona
  delta machinery (10-variant `PersonaDeltaCategory` enum,
  HMAC chain log, proposal flow) is the right shape for
  skills to ride on. Phase 70 added the operator-facing
  proposal review surface (`aivyx persona proposals
  list/show/approve/reject`); Phase 60 added the revert
  primitive. Adding `LearnedSkill` as an 11th category
  reuses every one of those substrate pieces without
  re-implementing them.
- **Q3c's both-render-AND-tool-surface is the right
  trade.** The Q3 sign-off asked operators to choose
  between rendering skills into the system prompt (passive
  injection, the Persona precedent) vs. a callable
  `skills.list` / `skills.invoke` tool surface (active
  reference). The operator picked **both** — passive
  prompt injection means an LLM that doesn't realize it
  has a relevant skill still benefits from being reminded
  every turn; the tool surface means an LLM that wants to
  enumerate or re-read a specific skill has substrate-level
  access. The two layers compose without overlap.
- **Streaks already at zero post-Phase-109.** All three
  byte-identity streaks reset at Phase 109's triple-break.
  Phase 110's predicted breaks (DESIGN.md for the new
  scope base; `aivyx-core/src/lib.rs` for the new tool
  re-exports) compound less awkwardly when the prior
  phase already cleared the slate.

## Scope (Q-block sign-off)

- **Q1 — Storage shape:** (a) **11th `PersonaDeltaCategory`
  variant `LearnedSkill`.** Skills land in the existing
  Persona HMAC chain alongside other identity deltas. No
  new `KeyDomain` needed; the entire Phase 59/60/70
  substrate (chain log, proposal flow, revert primitive,
  operator review surface) carries over.
- **Q2 — Scope base:** (b) **New `skills.propose` scope
  base** (operator-picked over Q2a "extend persona.propose"
  recommendation). Roles that should propose Persona
  refinements but not skill drafts (or vice versa) get the
  granularity. Adds one base to `KNOWN_BASES`.
  `persona.propose` and `skills.propose` are sibling
  capability gates; the proposal-flow plumbing routes both
  to the same chain but the dispatch checks the right base
  for each proposal's `PersonaDeltaCategory`.
- **Q3 — Activation:** (c) **Both render-in-prompt AND
  callable tool surface** (operator-picked over Q3a
  "render-only" recommendation). The render path extends
  `assemble_session_prompt` with a `## Learned skills`
  section (Phase 59 Q6a precedent — labeled section
  between Persona and active role). The tool surface adds
  `skills.list` (enumerate approved skills) and
  `skills.invoke` (render one specific skill's text into
  the current turn's working context).
- **Q4 — Phase scope:** (a) **Foundation: propose +
  approve + render + tool-surface.** Substrate piece. The
  agent-side auto-proposer heuristic (fire reflection-cron-
  style after complex turns, draft skill proposals
  automatically) stays a deliberate deferral for a
  follow-on phase — Phase 71 (reflection cadence) is the
  precedent for cron-fired reflection and that machinery
  needs its own design pass for the "what counts as a
  complex turn worth a proposal" question.

## Streak predictions

- **DESIGN.md** — **Will break.** Adding `skills.propose`
  to D4's base table is a new substrate base. Same
  break-pattern as Phase 109's `git.read` addition. Streak
  re-established to 1 at Phase 109; predicted to break
  again at Phase 110.

- **PRODUCT.md** — **Will hold.** P8 ("Outcome-Driven
  Audited Reflection") is already shipped at Phase 30 and
  the LearnedSkill extension is **inside P8's envelope** —
  P8's commitment is the propose-approve-apply flow shape,
  not its category-specific schema. No P8 text change
  needed. A Delivery-Status note may land for completeness
  but the commitment itself is not amended. Streak
  re-established to 1 at Phase 109; predicted to extend
  to 2.

- **Production-core `aivyx-core/src/lib.rs`** — **Will
  break.** Two new tools (`skills.list`, `skills.invoke`)
  land in `aivyx-core/src/tools/` per the Phase 4
  substrate convention (Phase 109's load-bearing
  prediction-vs-reality lesson). The re-export block
  edit is the load-bearing break. Streak re-established
  to 1 at Phase 109; predicted to break again at
  Phase 110.

- **A3 amendment addendum** — **Possibly needed.**
  `KNOWN_BASES` grows by one (`skills.propose`); A3 was
  last refreshed at Phase 54 with a 43-base inventory.
  Phase 110 wouldn't strictly require an A3 addendum
  (A3's count isn't pinned by an explicit assertion
  PRODUCT.md or DESIGN.md checks against), but a future
  docs-sweep phase that catches up the post-Phase-54
  inventory should batch `skills.propose` alongside the
  other post-Phase-54 additions (`net.dns`-now-with-tool,
  `git.read`, etc.).

- **New workspace deps** — Zero. All substrate work
  reuses existing crates. The skill-text rendering is
  pure string concatenation; no template engine pulled
  in.

- **Test count** — Positive. Persona-delta extension
  tests + skills tools tests + assemble_session_prompt
  render tests. Rough prediction: **+25 to +40**.

## Tasks

Six sub-tasks plus exit + backfill, comparable to Phase
107's structure (substrate-design phase):

### Task 1 — Open (this commit)

`docs/PHASE_110.md` + `docs/ROADMAP.md` Chapter D Phase
110 entry flip (scheduled → Active) + per-phase `## Phase
110` section + `docs/README.md` status row.

### Task 2 — `LearnedSkill` PersonaDeltaCategory variant + `LearnedSkill` schema

- `aivyx-channel/src/persona.rs` (or wherever
  `PersonaDeltaCategory` lives) gains an 11th variant
  `LearnedSkill`. The variant carries an inner struct or
  string with the skill's payload — name, trigger
  description, procedure text.
- The `PersonaDeltaOp` variants (`SetScalar`, `AppendList`,
  `RemoveList`, `Revert`) must each have a sensible
  meaning for `LearnedSkill`. `AppendList` is the natural
  fit — the operator's approved skill set is a list each
  delta extends.
- Schema validation at proposal-append time: the validator
  rejects a `LearnedSkill` delta whose payload doesn't
  parse into the expected shape.

### Task 3 — `skills.propose` scope base + proposal-flow routing

- `aivyx-capability::KNOWN_BASES` gains `skills.propose`.
- `CEILING_TRUSTED` includes it; tier-ceiling tests cover
  the new base.
- `reflection.propose` flow at the tool layer: when a
  proposal's `persona_deltas` array contains any
  `LearnedSkill` delta, the dispatch upgrades the
  required_scope from `persona.propose` to
  `skills.propose`. A role holding `persona.propose` but
  not `skills.propose` can still propose every other
  category; a role holding `skills.propose` but not
  `persona.propose` is symmetric for the skill-only case.
  Mixed proposals (Persona-mirror + LearnedSkill in one
  array) require both scopes.

### Task 4 — `skills.list` + `skills.invoke` tools

- New `aivyx-core/src/tools/skills.rs` housing both tools.
- `SkillsListTool` — reads approved skills from the
  Persona chain, returns them as a JSON array of
  `{name, trigger, procedure}` records. Read-only; no
  scope qualifier needed beyond a `skills.list` scope
  base (also added to `KNOWN_BASES`).
- `SkillsInvokeTool` — takes a skill name and a current-
  turn context blob; renders the skill's procedure text
  into the agent's working context (via a `ToolOutput`
  StreamEvent carrying the rendered text). Effectively
  the same as the agent reading the skill in its system
  prompt, but on-demand and explicit.
- Per-tool unit tests + integration tests in the binary's
  registration path.

### Task 5 — System-prompt rendering: `## Learned skills` section

- `aivyx-channel::assemble_session_prompt` extension:
  takes the approved skills list, renders a `## Learned
  skills` section between Persona's `## How I have
  learned to communicate` and the active role's `##
  Active role: <name>` section. Same labeled-section
  pattern Phase 59 Q6a established.
- Empty skills list = section omitted entirely (mirrors
  the Persona "empty → drop section" behavior).
- Per-skill rendering: one bullet per skill with
  `name: trigger description (procedure summary)`. Full
  procedure text is reserved for `skills.invoke` to
  avoid bloating every system prompt.

### Task 6 — Binary wiring + PRODUCT.md P8 delivery-status refresh + docs sweep + exit

- `aivyx` binary registers `SkillsListTool` and
  `SkillsInvokeTool` at the same tool-registration site
  as `git.status` / `net.dns` (Phase 109 precedent).
- PRODUCT.md P8 Delivery-Status entry refreshed to
  acknowledge the LearnedSkill extension as inside the
  P8 envelope (not an amendment — P8's commitment text
  stays unchanged).
- `docs/INSTALL.md` First-run checklist gains a Phase
  110 paragraph covering the propose + approve + render
  + invoke flow.
- `examples/aivyx.toml` gains a role declaration showing
  a `skills.propose`-equipped role.
- Exit: PHASE_110.md prediction-vs-reality + ROADMAP
  freeze + Chapter D complete summary + README flip +
  hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Storage:** (a) **11th PersonaDeltaCategory
  variant `LearnedSkill`.** Reuses the entire Phase
  59/60/70 substrate.
- **Q2 — Scope base:** (b) **New `skills.propose`**
  (operator pick over the Q2a "extend persona.propose"
  recommendation). Per-category granularity at the role-
  config layer; sibling capability gate to
  `persona.propose`.
- **Q3 — Activation:** (c) **Both render-in-prompt AND
  callable tool surface** (operator pick over the Q3a
  "render-only" recommendation). System-prompt section
  for passive injection + `skills.list` /
  `skills.invoke` for explicit reference.
- **Q4 — Phase scope:** (a) **Foundation: propose +
  approve + render + tool-surface.** Agent-side
  auto-proposer heuristic deferred to a follow-on
  phase.

## Exit criteria

- [ ] `docs/PHASE_110.md` + ROADMAP Chapter D Phase 110
  entry flip + docs/README status row — Task 1 (this
  commit).
- [ ] `LearnedSkill` PersonaDeltaCategory variant +
  schema + proposal-flow validation — Task 2.
- [ ] `skills.propose` scope base added to `KNOWN_BASES`
  + dispatch-time required-scope upgrade for proposals
  containing `LearnedSkill` deltas — Task 3.
- [ ] `skills.list` + `skills.invoke` tools shipped in
  `aivyx-core/src/tools/skills.rs` — Task 4.
- [ ] `assemble_session_prompt` extended with the
  `## Learned skills` section — Task 5.
- [ ] Binary wiring + PRODUCT.md P8 delivery-status
  refresh + INSTALL.md + examples/aivyx.toml — Task 6.
- [ ] All four Q-block questions resolved with operator
  sign-off pre-Task 2 (recorded above).
- [ ] DESIGN.md streak break predicted at the
  `skills.propose` base-table addition.
- [ ] PRODUCT.md streak predicted to hold (P8 envelope
  unchanged; Delivery-Status note is the only edit
  needed).
- [ ] `aivyx-core/src/lib.rs` streak break predicted at
  the `skills.list` / `skills.invoke` re-exports.
- [ ] Zero new workspace dependencies.
- [ ] Test count delta positive — predicted `+25` to
  `+40`.
- [ ] Zero clippy warnings.
- [ ] **Chapter D complete** — exit doc carries the
  Chapter D retrospective alongside Phase 110's own
  results.
