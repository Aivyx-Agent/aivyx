# Phase 115 — Self-Correction Loop on Failed Turns (Chapter E #2)

Closes the symmetric negative-feedback half of Phase 114's
auto-proposer. Phase 114 ships the positive-pattern path
("complex turns worth saving as Persona refinements");
Phase 115 ships the negative-pattern path ("failed turns
worth learning from"). After this phase, the agent
self-corrects across the same 11-category PersonaDelta
surface it self-learns over.

**Project vision continuation:** "Self-Learning,
Self-Improving AI Personal Assistant." Phase 114 closed
the self-learning loop at the Persona layer; Phase 115
closes the self-correcting loop. The agent observes
failures and proposes targeted Persona refinements
(BehavioralConstraints, LearnedContext, etc.) to avoid
recurrence. Same pipeline, broader trigger — maximum
substrate reuse per Q3a sign-off.

**Q1 was non-Recommended (broadest failure scope).** The
operator picked the maximum-signal posture: all non-
Completed `TurnOutcome` variants fire the self-correction
pipeline. Substrate cost is roughly the same as the
narrow option; the bet is on the judge + per-category
threshold gating handling the higher candidate volume
cleanly.

## Why this, why now

- **Phase 114's substrate is still hot.** The polymorphic
  judge, per-category routing, chain-write dispatch, and
  audit-event surface all just shipped. Phase 115 mostly
  flips one knob: the post-finalize trigger fires on more
  outcomes, the judge prompt handles failure context.
- **Negative feedback closes the loop.** Phase 112-114
  shipped the propose-judge-accept-render loop for
  positive patterns. The agent self-learns from what
  WORKS. Phase 115 makes it self-correct from what
  DOESN'T — the symmetric half that closes the bigger
  loop.
- **Q-block at sign-off:**
  - **Q1c — All non-Completed outcomes** (non-
    Recommended; operator picked over the narrower
    options). Failed + Escalated + Cancelled + TimedOut.
  - **Q2a — Let judge pick from full surface**
    (Recommended). Same polymorphic surface as Phase 114;
    BehavioralConstraints is the natural fit for many
    failures but the judge picks per-failure.
  - **Q3a — Extend Phase 114 substrate** (Recommended).
    Maximum reuse; new `from_failed_turns: bool` config
    knob; audit-event distinguishes completion-source
    from failure-source proposals.

## Scope (Q-block sign-off)

- **Q1 — Failure scope:** (c) **All non-Completed outcomes**
  (non-Recommended). The post-finalize hook fires the
  self-correction pipeline on:
  - `TurnOutcome::Failed(error)` — system or planner
    failure.
  - `TurnOutcome::Cancelled` — operator cancelled (via
    `/cancel` or equivalent).
  - `TurnOutcome::TimedOut` — agent exceeded its turn
    budget.
  - `TurnOutcome::Escalated` — the agent escalated; the
    operator hasn't resolved yet. The pipeline fires
    immediately at escalation time (treating the
    escalation itself as a learnable event); operator-
    resolve (/reject) integration is a Phase-115-internal
    deferral if it surfaces (would require hooking the
    persona-proposal-resolve site separately from the
    turn-finalize hook).

- **Q2 — Category scope:** (a) **Let judge pick from full
  surface** (Recommended). The judge prompt is extended to
  handle failure context but the category enumeration
  stays the same 11 variants. Per-category enable flags
  (Phase 114) apply uniformly — operator can disable
  specific categories for self-correction the same way
  they do for self-learning.

- **Q3 — Pipeline shape:** (a) **Extend Phase 114
  substrate** (Recommended). Same
  `auto_propose_for_turn` orchestration; broader trigger;
  new `from_failed_turns: bool` config knob; audit-event
  carries a new `source: ProposalSource` field
  (`CompletedTurn` | `FailedTurn`) with serde-skip-if-none
  for backward compatibility. Phase 92 / Phase 114
  precedent.

## Streak predictions

- **DESIGN.md** — **Will hold.** No D-section touch.
  Phase 115 extends the auto-proposer's trigger
  surface inside the D4/D5 envelope Phase 114 left
  intact. Hash at entry:
  `c2be6d51def04207ab0527b4f335edf7c726920b4f102185336eded01b1b995b`.
  Prediction: streak **extends to six** (was 5 after
  Phase 114).

- **PRODUCT.md** — **Will hold.** P8 (Outcome-Driven
  Audited Reflection) is the natural envelope; Phase
  115 broadens the *trigger surface* the auto-proposer
  fires from, all inside P8's "approved deltas land in
  chain" commit. Hash at entry:
  `6e840cef2cfcee4c483c2b4769c29cc7062daeb4bf3802c53f48e72ebfe57fb8`.
  Prediction: streak **extends to six** (was 5).

- **Production-core `aivyx-core/src/lib.rs`** — **Will
  hold.** The `skill_proposer` module is already
  `pub mod`-exposed; Phase 115 extends `judge.rs` (new
  failure-context fields + prompt extensions) and
  potentially adds a `ProposalSource` enum, but
  `lib.rs` itself stays unchanged. Hash at entry:
  `deab80d8dded7a59746771a3eb883aad2a55c364ca3743b9bd73b3c7c8837ece`.
  Prediction: streak **extends to four** (was 3).

- **New workspace deps** — Zero. Substrate extension only.

- **Test count** — Positive but smaller than Phase 114
  (Phase 114 added the per-category infrastructure; 115
  extends it). Failure-outcome heuristic tests,
  judge-prompt extension tests, audit-event source-field
  tests, scripted e2e covering each failure type.
  Prediction: **+20 to +35**.

## Tasks

Seven sub-tasks plus exit + backfill, matching the
Chapter E phase shape:

### Task 1 — Open (this commit)

`docs/PHASE_115.md` + `docs/ROADMAP.md` Phase 115 entry +
`docs/README.md` status row.

### Task 2 — Failure-outcome heuristic

- Extend `aivyx-core/src/skill_proposer/heuristic.rs`
  with a new `is_failure_candidate(outcome)` function
  that returns `true` for the four failure-outcome
  variants (Failed, Cancelled, TimedOut, Escalated).
- Add a parallel `FailureHeuristicConfig` (or extend
  `HeuristicConfig`) with per-failure-outcome enable
  flags so the operator can tune which failure types
  fire the pipeline.
- Tests: each failure-outcome variant fires the
  heuristic; Completed never fires the failure
  heuristic (Phase 114's path handles Completed).

### Task 3 — Judge prompt + JudgeRequest extension

- `aivyx-core/src/skill_proposer/judge.rs` —
  `JudgeRequest` gains a `proposal_source:
  ProposalSource` field (new enum: `CompletedTurn`
  | `FailedTurn { failure_summary: String }`).
- Judge system prompt extended: a `## Failed-turn
  correction context` block when the source is
  `FailedTurn`. The judge sees the failure summary
  and is instructed to draft a refinement that would
  prevent recurrence (BehavioralConstraint, learned
  context, etc.).
- `JudgeResponse` carries the same shape — no new
  fields. The `is_worth_proposing` boolean still
  controls whether anything lands.
- Tests: prompt-shape coverage for both source
  variants; ProposedDraft round-trip; serde round-trip
  for the new ProposalSource enum.

### Task 4 — Daemon post-finalize hook broadening

- Currently the hook fires only for
  `TurnOutcome::Completed`. Extend it to fire for any
  `TurnOutcome` variant when
  `config.from_failed_turns == true`.
- The hook constructs a `failure_summary` string
  describing the outcome (e.g. "agent failed with
  error: ...", "operator cancelled after N tool
  calls").
- Build `TurnSignals` from the failure context (some
  fields are degenerate — e.g. `tool_calls_made` may
  be 0 for an early failure; the heuristic gate
  handles this).
- The auto-proposer's `auto_propose_for_turn` is
  extended to take a `ProposalSource` arg and route
  to the appropriate judge prompt.

### Task 5 — Audit event source-field extension

- `aivyx-audit::AuditEvent::SkillAutoProposal` gains an
  optional `source: Option<ProposalSourceSummary>`
  field with `#[serde(default, skip_serializing_if =
  "Option::is_none")]`. Existing chains verify byte-
  identically (Phase 92 / Phase 114 precedent).
- `ProposalSourceSummary` enum: `CompletedTurn`
  (default for backward compat) | `FailedTurn`.
- Tests: backward-compat (pre-Phase-115 entries
  decode with source=None); new entries carry the
  source; round-trip through serde-jcs.

### Task 6 — TOML config: `from_failed_turns` + per-failure-type enable

- `[persona.auto_propose]` gains a
  `from_failed_turns: bool` top-level field. Default
  `false` (Phase 114 behavior preserved); operator
  opts in.
- Optional `[persona.auto_propose.failure_outcomes]`
  sub-section with `failed: bool`, `cancelled: bool`,
  `timed_out: bool`, `escalated: bool` per-variant
  enables. Defaults: `failed = true`, `cancelled =
  false`, `timed_out = true`, `escalated = false`
  (operator-conservative; the most common
  cancellation is the operator killing a turn they
  no longer want, not a learnable signal).
- Tests: parse cases for the new fields; defaults;
  per-variant override.

### Task 7 — Scripted e2e + INSTALL.md sweep + exit

- Scripted e2e covering three failure types:
  - `TurnOutcome::Failed` with the judge returning a
    BehavioralConstraint proposal → auto-accept lands.
  - `TurnOutcome::TimedOut` with the judge declining
    to propose (the failure isn't actionable).
  - `TurnOutcome::Cancelled` with `cancelled = false`
    in config → no pipeline fire (the heuristic
    short-circuits).
- INSTALL.md: extend the Phase 112-114 auto-proposer
  section with a "Self-correction loop (Phase 115)"
  paragraph covering the `from_failed_turns` knob and
  the per-failure-outcome enable flags.
- Exit: PHASE_115.md prediction-vs-reality + ROADMAP
  freeze + README flip + hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Failure scope:** (c) **All non-Completed
  outcomes** (non-Recommended). Maximum-signal posture;
  the judge's confidence threshold + per-category enable
  gating handles candidate volume.
- **Q2 — Category scope:** (a) **Let judge pick from
  full surface** (Recommended). Polymorphic judge from
  Phase 114; per-category enables apply uniformly to
  failure-driven proposals.
- **Q3 — Pipeline shape:** (a) **Extend Phase 114
  substrate** (Recommended). Same orchestration; new
  `from_failed_turns` knob; new `source` field on
  audit event for forensic distinction.

## Exit criteria

- [x] `docs/PHASE_115.md` + ROADMAP Phase 115 entry +
  docs/README status row — Task 1 (`258262f`).
- [x] Failure-outcome heuristic + per-variant enable
  config — Task 2 (`9ee8834`).
- [x] `JudgeRequest.source` field + failure-context
  prompt block — Task 3 (`6a98289`).
- [x] Daemon post-finalize hook fires on non-Completed
  outcomes when configured — Task 4 (`d83e7f0`).
- [x] `AuditEvent::SkillAutoProposal.source` backward-
  compat extension — Task 5 (`c1ec4cd`).
- [x] TOML config `from_failed_turns` + failure-outcomes
  sub-section — Task 6 (`6e0f5d1`).
- [x] Scripted e2e covering Failed + TimedOut + Phase
  113-alias paths — Task 7.
- [x] All three Q-block questions resolved with
  operator sign-off pre-Task 2 (Q1c, Q2a, Q3a
  recorded above).
- [x] DESIGN.md streak extends to six — **HELD as
  predicted**. `c2be6d51…` unchanged.
- [x] PRODUCT.md streak extends to six — **HELD as
  predicted**. `6e840cef…` unchanged.
- [x] `aivyx-core/src/lib.rs` streak extends to four —
  **HELD as predicted**. `deab80d8…` unchanged.
- [x] Zero new workspace dependencies.
- [x] Test count delta positive — `+24` (2105 → 2129),
  inside the `+20 to +35` predicted range.
- [x] Zero clippy warnings.
- [x] **The self-correction loop closes alongside the
  self-learning loop.** Chapter E's negative-feedback
  half ships next to the positive-feedback half from
  Phase 114; the same pipeline handles both via the
  `ProposalSource` discriminator.

## Prediction vs reality

**Three streak predictions; all three held — fourth
all-hold result in a row.** Phases 112, 113, 114, and 115
each saw every prediction hit. This is the longest streak-
hold run in project history. Substrate is genuinely
mature: Chapter E's pipeline extensions are all reuse,
not new substrate cost.

- **DESIGN.md** — Held as predicted. `c2be6d51…`
  unchanged. The failure-feedback path slots into the
  existing D4/D5 envelope; no new section needed.
  Streak: 5 → 6.
- **PRODUCT.md** — Held as predicted. `6e840cef…`
  unchanged. P8 (Outcome-Driven Audited Reflection)
  covers failure-driven reflection identically to
  completion-driven; broadening the trigger doesn't
  touch the contract. Streak: 5 → 6.
- **`aivyx-core/src/lib.rs`** — Held as predicted.
  `deab80d8…` unchanged. The `skill_proposer` module's
  internal extensions (FailureKind enum, ProposalSource
  enum, judge prompt branching) all live inside the
  existing `pub mod skill_proposer` boundary. Streak:
  3 → 4.

**Test count `+24` is inside the `+20 to +35` band.**
Breakdown:
- `aivyx-core/src/skill_proposer/heuristic.rs` — +7
  tests (FailureKind labels, FailureHeuristicConfig
  per-variant + defaults + all-enabled/disabled + serde
  round-trip).
- `aivyx-core/src/skill_proposer/judge.rs` — +6 tests
  (ProposalSource default + label; system prompt
  mentions both source modes; user prompt branches per
  source variant).
- `aivyx-audit` — +4 tests (source-field round-trip;
  source=None absent-field serialization; pre-Phase-115
  decoder compat; ProposalSourceSummary tagged-enum
  shape).
- `aivyx-config` — +4 tests (failure defaults;
  from_failed_turns explicit; per-outcome override;
  failure_outcomes sub-section arms section alone).
- `aivyx-channel/tests/skill_auto_proposer_e2e.rs` —
  +3 tests (failed_turn → BehavioralConstraints auto-
  accept; timed_out + judge-decline → drop;
  Phase-113-alias does-not-fire-failure-path pin).

**Q-block resolved exactly as signed off.** Q1c (broadest
failure scope) was the operator's non-Recommended pick;
the implementation cost roughly the same as the narrower
options because the per-failure-outcome enable defaults
keep the noise from Cancelled and Escalated turns off by
default.

## Phase-115-internal deferral named at sign-off

- **Operator-resolve escalation (/reject) integration.**
  Phase 115 fires the failure pipeline on
  `TurnOutcome::Escalated` at finalize time (when the
  agent escalates), not on the later
  `operator-/reject` resolution. Capturing the
  `/reject` half would require hooking the persona-
  proposal-resolve site separately from the turn-
  finalize hook — deferred unless operator pressure
  surfaces in real use.

## Chapter E direction after Phase 115

After Phase 115, two named Chapter E axes remain (subject
to operator-pressure-shaped reordering at each phase
exit):
- Tool/skill selection learning from outcomes.
- Outcome-driven Profile/Role refinement proposals.

The post-Phase-115 ledger has one named deferral (the
escalation /reject hook above). The next Chapter E phase
opens against whichever axis surfaces real-use value
first.
