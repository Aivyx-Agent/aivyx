# Phase 112 — Skill Auto-Proposer (Phase 110's named follow-on)

Closes the Phase 110 named deferral: *"agent-side
auto-proposer heuristic (fire reflection-cron-style after
complex turns, draft skill proposals automatically)."*
Phase 110 shipped the **propose-approve-render-invoke**
substrate for learned skills; Phase 112 ships the
**autonomous propose-and-accept** loop that closes the
self-learning circuit for the project vision.

**Project vision:** *"Self-Learning, Self-Improving AI
Personal Assistant."* After Phase 112, the agent
genuinely self-learns at the skill layer — complex turns
trigger an LLM-judged proposal, high-confidence proposals
auto-accept into the LearnedSkill chain, and the next
turn's system prompt already carries the new skill. No
operator-in-the-loop required for the steady-state case;
the operator still inspects via `aivyx persona list` and
can revert via the existing Phase 60 revert primitive.

The phase is **not chapter-framed.** Chapter D closed at
Phase 110. Phase 111 closed two Chapter-D-internal
adapter-wiring deferrals. Phase 112 closes the last
Chapter-D-internal substrate deferral — same "follow-on
to a closed chapter" precedent as Phase 111. After Phase
112, all named Chapter D follow-ons are closed.

## Why this, why now

- **Project-vision critical path.** The skills.* substrate
  shipped in Phase 110 is currently operator-driven —
  `skills.propose` is a tool the agent must explicitly
  call, and every proposal is staged for the operator's
  manual approval. The vision wants "self-learning,
  self-improving" — the auto-proposer is the substrate
  piece that turns Phase 110's framework into actual
  self-learning.
- **One specifically-named deferral, fresh substrate.**
  Phase 110 named this exact deferral at exit. The
  propose-approve-render-invoke plumbing (Phase 59 chain,
  Phase 60 revert, Phase 110 skills.* + system-prompt
  injection) is in place and stable. Closing the
  deferral now uses the still-warm substrate without
  re-learning it.
- **Q-block at sign-off (operator-picked, three non-
  Recommended):** ship the more autonomous shape across
  the board — LLM-judge confirms every candidate (Q1b),
  fire inline at turn boundary (Q2b, **non-Recommended**
  — picked over cron-fired), threshold-gated auto-accept
  (Q3b, **non-Recommended** — picked over always-staged),
  and LLM-semantic dedup on top of title fuzzy-match (Q4b,
  **non-Recommended** — picked over title-only). This is a
  deliberate operator-driven aggressiveness; the cost-
  implications are surfaced honestly in the task
  breakdown below.

## Scope (Q-block sign-off)

- **Q1 — Trigger signal:** (b) **Heuristic + LLM-judge.**
  A cheap deterministic gate (tool-call count, distinct
  tool-id count, duration, gate-resolve presence) filters
  to a small candidate set; for each candidate, a small
  LLM call asks "would a learned skill help similar
  turns later?" plus returns a proposal draft if yes.
  Phase 91 (LLM-judgment for recall feedback) is the
  two-stage judgment precedent; Phase 95 (skip-when-idle)
  is the cheap-deterministic-signal precedent.
- **Q2 — Fire timing:** (b) **Inline at turn boundary**
  (operator-picked over Q2a Recommended cron-fired).
  Fires immediately when each turn ends. Implication: the
  candidate-judge LLM call runs in a **background task
  spawned after `finalize`**, NOT on the turn's critical
  path. The user never waits for the auto-proposer; the
  next turn can begin before the proposal lands. Cost
  posture: per-candidate-turn LLM cost, not per-turn LLM
  cost (the cheap heuristic gates first).
- **Q3 — Acceptance:** (b) **Threshold-gated auto-accept**
  (operator-picked over Q3a Recommended always-staged).
  Operator-configurable TOML threshold; LLM-judge
  confidence ≥ threshold → auto-accept (lands directly
  in the LearnedSkill chain as an approved entry); below
  threshold → staged for operator approval via the
  existing Phase 110 surface. Default threshold leaves
  the auto-accept path enabled — the operator opted into
  the trust window deliberately, so the phase doesn't
  bury the feature behind a disabled-by-default flag.
- **Q4 — De-duplication:** (b) **Title fuzzy-match +
  LLM semantic check** (operator-picked over Q4a
  Recommended title-only). Title fuzzy-match drops
  obvious dups cheaply (`memory.gc` vs `memory_gc`);
  for non-filtered proposals, the LLM-judge call
  (Q1b's judge) **also** carries an
  `is_duplicate_of: Option<existing_skill_name>` field
  so one LLM call covers both worth-proposing AND
  novelty-check. Cost-conscious by piggybacking on the
  Q1b judge call rather than firing a separate dedup
  judge.

## Streak predictions

- **DESIGN.md** — **Will break.** D4 has a "Skills"
  section (added at Phase 110); Phase 112 extends it with
  the auto-propose pipeline shape (background-task
  contract, threshold-config schema, judge-prompt
  shape). Could maybe land as an A-amendment addendum
  to keep DESIGN.md byte-identical (Phase 107 precedent
  for amendment addendum without DESIGN.md edits), but
  the auto-propose pipeline is sufficiently substrate-
  level that a D4 extension reads cleaner. Hash at
  entry:
  `c2be6d51def04207ab0527b4f335edf7c726920b4f102185336eded01b1b995b`.
  Prediction: streak **resets to one** (was 2 after
  Phase 111).

- **PRODUCT.md** — **Will hold.** P8 ("Outcome-Driven
  Audited Reflection") is the natural envelope for the
  auto-proposer — the proposal pipeline is reflection
  fired inline rather than on a cron, but it's still
  reflection in shape (read recent outcomes, judge,
  propose deltas). Phase 110 already extended P8 to
  cover LearnedSkill as a delta category; Phase 112
  extends P8's *firing pattern* (was: cron-fired or
  operator-tool-called; becomes: also inline-after-turn)
  without changing P8's commitment. Hash at entry:
  `6e840cef2cfcee4c483c2b4769c29cc7062daeb4bf3802c53f48e72ebfe57fb8`.
  Prediction: streak **extends to three** (was 2 after
  Phase 111).

- **Production-core `aivyx-core/src/lib.rs`** — **Will
  break.** The judge-invocation surface (a new LLM call
  shape for "judge this turn outcome and return
  proposal + confidence + dup-of") likely lives in
  `aivyx-core/src/llm.rs` or a new
  `aivyx-core/src/skill_proposer.rs`, which means
  `lib.rs` either gets a new `pub mod` or modifies an
  existing re-export. The background-task spawn pattern
  may also need a new public type. Hash at entry:
  `ab0e425d368bb98ceb761187826ea26575b124d3b4f99cffb9e9778e20c543b7`.
  Prediction: streak **resets to one** (was 2 after
  Phase 111).

- **New workspace deps** — Zero predicted. LLM substrate
  is already there (Phase 25 multi-provider, Phase 87
  phrasing, Phase 91 judgment). Background-task spawn
  uses existing tokio. Threshold config uses existing
  TOML surface.

- **Test count** — Positive. The auto-proposer is
  substrate-heavy with multiple interacting pieces.
  Heuristic detection tests + LLM-judge prompt-shape
  tests (with a mock LLM) + threshold-gate tests +
  dedup tests + background-task wiring tests + scripted
  e2e (complex turn → judge → auto-accept → next turn
  has the skill rendered). Prediction: **+30 to +50**.

## Tasks

Seven sub-tasks plus exit + backfill, comparable to
Phase 110's substrate-heavy multi-session shape:

### Task 1 — Open (this commit)

`docs/PHASE_112.md` + `docs/ROADMAP.md` entry
(standalone follow-on past Chapter D's close, matching
Phase 111's framing) + `docs/README.md` status row.

### Task 2 — Heuristic detection (cheap signal)

- New `aivyx-core/src/skill_proposer/heuristic.rs` (or
  similar) defining `TurnComplexity` and `is_candidate(
  turn_outcome, config) -> bool`.
- Signals (all configurable, all default to operator-
  reasonable thresholds): `tool_call_count >= 3`,
  `distinct_tool_id_count >= 2`, `duration_ms >= 5000`,
  `had_successful_gate_resolve == true`, OR any
  combination via `MatchMode::Any | All`.
- Pure-deterministic, zero LLM cost; reads from the
  just-completed `TurnOutcome` and the daemon's audit-
  chain entries for that turn.
- Tests: a fixture turn outcome under-threshold returns
  `false`; over-threshold returns `true`; per-signal
  ablation tests pin each threshold independently.

### Task 3 — LLM-judge prompt + invocation surface

- New `aivyx-core/src/skill_proposer/judge.rs` defining
  `JudgeRequest` (turn outcome summary + existing skill
  list snapshot) and `JudgeResponse` (`{
  is_worth_proposing: bool, confidence: f32,
  proposed_skill: Option<SkillDraft>,
  is_duplicate_of: Option<String> }`).
- Prompt shape: short single-shot prompt that takes the
  turn summary (Phase 71 reflection-summary pattern) +
  existing skill name+summary list (for the dedup
  check) and returns the JudgeResponse as structured
  output (Phase 87 phrasing-via-structured-output
  precedent).
- The judge call uses the operator's configured LLM
  provider via `aivyx-llm` (no provider-specific code);
  same call shape as the Phase 91 judgment / Phase 92
  supersession surfaces.
- Tests: prompt-shape stability test (golden output
  against a recorded fixture); JudgeResponse parse
  test; integration test against a scripted LLM mock
  that returns each branch.

### Task 4 — Background-task wiring (post-finalize fire)

- The auto-proposer fires from a `tokio::spawn` started
  inside the turn-driver after the `finalize` event has
  been sent to the channel. Critical-path latency is
  zero; the proposer runs in the background.
- Shutdown coordination: the daemon's existing
  `CancellationToken` propagates to the spawned task so
  shutdown is clean. The task carries `Arc<...>` clones
  of the audit log, persona chain, LLM client, and
  config it needs.
- Failure isolation: any auto-proposer failure (LLM
  timeout, judge-parse error, write failure on the
  proposal chain) logs at WARN with a structured event
  and **does not affect the turn outcome** — the user
  already got their reply; the auto-proposer is purely
  additive.
- Tests: scripted scenario where the judge mock returns
  an error; turn outcome stays unchanged; WARN log line
  asserted via a captured logger.

### Task 5 — Threshold-gated auto-accept

- New TOML config section `[skills.auto_propose]` with
  fields:
  - `enabled: bool` (default `true` — operator opted
    in via Q3b),
  - `heuristic: { mode: Any|All, tool_call_count_min:
    u32, distinct_tool_id_min: u32, duration_ms_min:
    u64, require_gate_resolve: bool }`,
  - `auto_accept_confidence_threshold: f32` (default
    `0.85` — high enough to require strong judge
    signal, low enough to actually fire),
  - `fuzzy_match_threshold: f32` (default `0.80` for
    the cheap title-similarity pre-filter).
- Auto-accept path: a `JudgeResponse.confidence >=
  threshold` AND `is_duplicate_of.is_none()` AND
  fuzzy-match-clean proposal routes through the
  existing Phase 110 acceptance code path (the same
  function `persona approve` uses), tagged with an
  `auto_accepted: true` audit-event field so the
  operator can list them separately later.
- Staged path: anything else lands in the proposals
  chain as a regular un-approved entry, surfaced by
  `aivyx persona list` exactly as a manual proposal
  would be.
- Tests: confidence above threshold → auto-accept;
  below threshold → staged; dup detected → dropped;
  fuzzy-match drops obvious title dups before the
  judge call (for cost efficiency).

### Task 6 — Audit observability surface

- New audit event variant `SkillAutoProposalEvent`
  carrying: `outcome` (proposed | auto-accepted |
  staged | dup-dropped-fuzzy | dup-dropped-llm |
  judge-rejected | judge-error), `confidence`,
  `proposed_skill_name`, `judge_latency_ms`,
  `heuristic_signals_matched`.
- `aivyx audit export` filter flag
  `--event-type SkillAutoProposalEvent` for forensic
  walks of the auto-proposer's history.
- `aivyx persona list --auto-only` and `--manual-only`
  flags (Phase 60 list precedent) for separating the
  two proposal sources at inspection time.
- Tests: audit-event round-trip through JSONL export;
  the persona list filter flag returns only the
  matching subset.

### Task 7 — Binary wiring + scripted e2e + docs sweep + exit

- The auto-proposer construction lives in
  `aivyx-channel/src/bin/aivyx.rs` daemon-init path so
  it's available to every channel (Local + Telegram +
  Web UI + Discord + Slack — all five adapters get
  auto-proposer for free since it hooks the turn
  finalize event, not the adapter).
- Scripted e2e test: a fixture turn that crosses every
  heuristic threshold + a scripted LLM judge returning
  a high-confidence non-dup proposal → assert the
  skill lands in the LearnedSkill chain as auto-
  accepted → next turn's system prompt renders it →
  the operator-side `aivyx persona list --auto-only`
  shows it.
- `docs/INSTALL.md` Phase 112 paragraph: the auto-
  proposer's default-on posture, the TOML config
  knobs, the audit-export filter, and the
  always-revertable-via-Phase-60 escape hatch.
- `docs/ROADMAP.md` post-Phase-112 ledger updated:
  Phase 110's named follow-on is closed; all Chapter
  D internal deferrals are now closed.
- Exit: PHASE_112.md prediction-vs-reality + ROADMAP
  freeze + README flip + hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Trigger signal:** (b) **Heuristic + LLM-judge.**
  Two-stage filter — cheap heuristic gates first,
  LLM-judge confirms candidates. Phase 91 / Phase 95
  precedent.
- **Q2 — Fire timing:** (b) **Inline at turn boundary**
  (non-Recommended). Background-task spawned post-
  `finalize` keeps critical-path latency at zero.
  Operator deliberately picked the more autonomous
  posture.
- **Q3 — Acceptance:** (b) **Threshold-gated auto-accept**
  (non-Recommended). Operator-configurable TOML
  threshold defaulting `0.85`; high-confidence non-dup
  proposals auto-accept, others stage. Operator
  deliberately opened the auto-accept trust window.
- **Q4 — De-duplication:** (b) **Title fuzzy-match +
  LLM semantic check** (non-Recommended). LLM check
  piggybacks on the Q1b judge call (one LLM round-trip
  covers both judgement axes); fuzzy-match pre-filter
  keeps cost down by avoiding the judge call for
  obvious title-dups.

## Exit criteria

- [ ] `docs/PHASE_112.md` + ROADMAP Phase 112 entry +
  docs/README status row — Task 1 (this commit).
- [ ] Heuristic detection (`aivyx-core/src/skill_proposer/
  heuristic.rs`) + per-signal threshold tests — Task 2.
- [ ] LLM-judge surface (`aivyx-core/src/skill_proposer/
  judge.rs`) + prompt-shape + parse + branch tests —
  Task 3.
- [ ] Background-task wiring with shutdown propagation +
  failure isolation tests — Task 4.
- [ ] `[skills.auto_propose]` TOML config + threshold-
  gated auto-accept routing + dedup pre-filter — Task 5.
- [ ] `SkillAutoProposalEvent` audit-event variant +
  `--auto-only` / `--manual-only` persona-list filters —
  Task 6.
- [ ] Binary wiring + scripted e2e (all five adapters
  inherit auto-proposer via the channel-agnostic
  turn-finalize hook) + docs sweep — Task 7.
- [ ] All four Q-block questions resolved with operator
  sign-off pre-Task 2 (recorded above).
- [ ] DESIGN.md streak break predicted — D4 skills
  section extended with auto-proposer pipeline.
- [ ] PRODUCT.md streak extends to three (P8 envelope).
- [ ] `aivyx-core/src/lib.rs` streak break predicted —
  new `skill_proposer` module.
- [ ] Zero new workspace dependencies.
- [ ] Test count delta positive — predicted `+30` to
  `+50`.
- [ ] Zero clippy warnings.
- [ ] **Phase 110's named follow-on closed.** Skills
  loop is now genuinely self-learning end-to-end.
