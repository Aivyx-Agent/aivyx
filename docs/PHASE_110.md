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

## Prediction vs. reality

**Three predicted streak breaks; PRODUCT.md broke against
the prediction (honest surprise — the P8 delivery-status
refresh changed byte identity even though the commitment
text itself stayed inside the envelope).**

- **DESIGN.md — broke as predicted** (2bd52e61 →
  c2be6d51). D4's base table gained a `skills` section
  with the three new scope bases (skills.propose,
  skills.list, skills.invoke). A Phase 110 blockquote
  documents the no-amendment-needed rationale (skills.*
  tools are infrastructure per P10's
  substrate/infrastructure/third-party taxonomy). Streak
  ends at 1.
- **PRODUCT.md — broke against the prediction**
  (3fba1078 → 6e840cef). The open doc said PRODUCT.md
  would hold because P8's commitment text is unchanged
  (the LearnedSkill extension fits inside the existing
  outcome-driven-audited-reflection envelope). The
  delivery-status entry needed the Phase 110 extension
  acknowledged for traceability, and that edit changed
  byte identity. The prediction's spirit — no amendment
  to P8's commitment shape — is honored; the commitment
  text itself is unchanged. The streak break is on the
  delivery-status refresh, not on an amendment. Honest
  break per the Phase 6 Q5 convention. Streak ends at 1.
- **`aivyx-core/src/lib.rs` — broke as predicted**
  (90104b1f → ab0e425d). The `SkillReader`,
  `SkillsListTool`, `SkillsInvokeTool` re-exports + the
  matching `pub use` block in `tools/mod.rs` are the
  load-bearing edits. Streak ends at 1.

**Zero new workspace deps — held.** Pure substrate work
plus tool surface; no external crates pulled in.

**A3 amendment addendum — not filed**, deferred to a
future docs-sweep phase. `KNOWN_BASES` grows by three
(skills.propose, skills.list, skills.invoke) on top of
Phase 109's `git.read`; A3's last refresh at Phase 54
catalogued 43 bases, and the post-Phase-54 additions are
worth batching into one A3 addendum when a focused docs
phase opens for that.

**Test count — `+12`** (workspace `1938 → 1950`). **Below
the predicted `+25` to `+40` band by about half** —
honest surprise. The skills tools test exhaustively (12
tests in `tools/skills.rs` cover both happy and error
paths for both tools), but the Persona-extension and
prompt-rendering work was covered by the existing
profile_prompt / persona test surface plus the integration
tests that were already in place — no new test files
needed beyond `skills.rs`. The prediction was uncalibrated
against the test-surface-already-exists reality of
extending well-tested substrate.

**Second consecutive triple-streak-reset.** Phase 109 was
the first since Phase 56; Phase 110 makes it back-to-back.
The substrate-design-heavy tail of Chapter D necessarily
touches both contract docs and the core re-export surface.
Phases 105–108 all held both streaks; Phases 109–110 both
broke them. The honest framing is that Chapter D's
**lighter half** (read-only export, MCP recipes, channel
adapters) held the streaks, and the **heavier half**
(tool surface growth + skills substrate) broke them. Both
halves shipped honest work.

**Scope — six tasks shipped as planned**, with Tasks 2-6
combined into one commit per the Phase 105/106/108
multi-task-batch pattern.

**End-to-end notes.** The skills substrate is fully wired
through to the agent today. An operator with
`reflection.propose` + `skills.propose` + appropriate
review tooling can:
1. Have an agent draft a `LearnedSkill` proposal through
   `reflection.propose` with a `persona_deltas` array
   containing a `LearnedSkill` category delta.
2. Review the proposal through the Phase 70 surface
   (`aivyx persona proposals list/show/approve/reject`).
3. See the approved skill appear in the agent's system
   prompt as `## Learned skills` on the next turn.
4. Have the agent invoke `skills.list` to enumerate
   approved skills or `skills.invoke` to read a specific
   skill's full procedure body.

The agent-side auto-proposer heuristic (fire reflection-
cron-style after complex turns and draft skill proposals
automatically) **stays a deliberate Phase-110-internal
deferral** for a focused follow-on phase. Phase 71
(reflection cadence) is the precedent for cron-fired
reflection, but the "what counts as a complex turn worth
a proposal" design question needs its own pass.

---

## Chapter D — Substrate Breadth (Phases 105–110) [COMPLETE]

Phase 110 closes Chapter D. The six-phase arc was opened
at Phase 105 in response to the Phase 104 Hermes Agent
comparison, which named five out-of-the-box-surface gaps
relative to the closest public reference for a personal-AI-
agent substrate. Chapter D extended that to a six-phase
arc by adding Phase 105 as a low-risk opener.

| Phase | Status | Headline |
|---|---|---|
| 105 | ✓ shipped | Trajectory Logging — `aivyx audit export` JSONL emitter |
| 106 | ✓ shipped | MCP Server Breadth — `aivyx mcp recipes` + 12-recipe catalog |
| 107 | ✓ shipped | Discord Channel Adapter — `aivyx-discord` crate, twilight-rs SDK |
| 108 | ✓ shipped | Slack Channel Adapter — `aivyx-slack` crate, slack-morphism SDK |
| 109 | ✓ shipped | Tool Breadth + Amendment A12 — `git.status` + `git.diff` + `net.dns` |
| 110 | ✓ shipped | Skills Auto-Creation — `LearnedSkill` PersonaDeltaCategory + skills.* tools |

**Chapter D retrospective.** Six phases, all shipped.
Three amendments landed (A12 for the substrate tool count;
A4 addenda for `aivyx-discord` and `aivyx-slack` crate
counts; the workspace-layout file traceability table now
carries Phase 107/108 rows). The
`docs/ADAPTER_PATTERN.md` moved from "tentative at two
data points" (Phase 9) to "confirmed at three" (Phase 107
exit) to "confirmed at four" (Phase 108 exit) — the doc's
load-bearing claim that the channel-adapter pattern is
reusable across protocols is now backed by four in-tree
adapters (Local + Telegram + Discord + Slack).

The chapter closed with **two Phase-107/108-internal
deferrals bundled together** for a focused follow-on:
the Discord daemon-frontend variant (Phase 19 Telegram-over-
daemon parallel) and the live Socket Mode wiring for
`SlackMorphismTransport` (callback-state-passing via
`SlackClientEventsUserState`). Both land at the **Channel
Activation Milestone** alongside real-bot smoke testing
across every channel adapter.

Chapter D also closed with **one Phase-110-internal
deferral**: the agent-side auto-proposer heuristic for
skills. The propose-approve-render-invoke substrate is
complete; the "agent automatically drafts skill proposals
after complex turns" heuristic needs its own design pass
that names what counts as a complex turn.

Workspace tests across the chapter: **1843 → 1950
(+107)**. Six phases of net-positive test surface. Zero
clippy warnings throughout.

Streak journey across the chapter:
- DESIGN.md: 53 (Phase 105 entry) → ended Phase 109 at 55,
  re-established to 1 at Phase 109 break, broke again at
  Phase 110.
- PRODUCT.md: 6 (Phase 105 entry) → ended Phase 109 at 8,
  re-established to 1 at Phase 109 break, broke again at
  Phase 110 (delivery-status refresh).
- `aivyx-core/src/lib.rs`: 6 (Phase 105 entry) → ended
  Phase 109 at 8 (broke at the tools re-exports, not at
  the predicted-but-non-existent location), re-established
  to 1 at Phase 109 break, broke again at Phase 110.

Two phases (109, 110) reset all three streaks; four phases
(105, 106, 107, 108) held them. The honest pattern:
substrate-design phases break streaks, adapter / docs /
reader-only phases hold them.

After Phase 110: **Hermes-comparison channel-breadth gap
closed** (5 in-tree adapters: Local + Telegram + Web UI +
Discord + Slack vs. Hermes's 6 — WhatsApp + Signal remain
candidates for a future Reach-style follow-on);
**tool-breadth gap closed** (13 substrate tools post-A12);
**skills-auto-creation gap closed** (LearnedSkill substrate
+ skills.* tool surface). The Hermes-comparison axis is
done; the next named work picks itself based on operator
feedback or as amendment-introduced commitments.

## Exit criteria

- [x] `docs/PHASE_110.md` + ROADMAP Chapter D Phase 110
  entry flip + docs/README status row — Task 1 (commit
  `36cef97`).
- [x] `LearnedSkill` PersonaDeltaCategory variant +
  schema + LearnedSkill struct + EffectivePersona
  field — Task 2 (commit `933c4cf`).
- [x] `skills.propose` + `skills.list` + `skills.invoke`
  scope bases added to `KNOWN_BASES`; CEILING_TRUSTED
  includes all three — Task 3 (commit `933c4cf`).
- [x] `skills.list` + `skills.invoke` tools shipped in
  `aivyx-core/src/tools/skills.rs` with SkillReader
  closure abstraction — Task 4 (commit `933c4cf`).
- [x] `assemble_session_prompt` extended with the
  `## Learned skills` section between Persona and
  active role — Task 5 (commit `933c4cf`).
- [x] Binary wiring (SkillReader closure threading the
  shared_persona read-lock) + PRODUCT.md P8
  delivery-status refresh + DESIGN.md D4 `skills` section
  + INSTALL.md + examples/aivyx.toml — Task 6 (commit
  `933c4cf`).
- [x] All four Q-block questions resolved with operator
  sign-off pre-Task 2 (recorded above).
- [x] DESIGN.md streak broke as predicted at the
  `skills` D4 section addition.
- [x] PRODUCT.md streak **broke against the prediction**
  — the P8 delivery-status refresh edit changed byte
  identity even though the commitment text itself
  stayed inside the envelope. Honest break per the
  Phase 6 Q5 convention.
- [x] `aivyx-core/src/lib.rs` streak broke as predicted
  at the skills.list / skills.invoke + SkillReader
  re-exports.
- [x] Zero new workspace dependencies.
- [x] Test count delta positive — `+12` (below the
  predicted `+25`–`+40` band; the prediction was
  uncalibrated against the test-surface-already-exists
  reality of extending well-tested substrate).
- [x] Zero clippy warnings.
- [x] **Chapter D complete** — retrospective recorded
  above.
