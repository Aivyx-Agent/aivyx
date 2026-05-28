# Phase 113 — Operator-Surface Polish (deferral cleanup)

Closes the operator-surface deferrals accumulated through
Chapter D and Phase 112. After this phase, every named
operator-surface gap that survived the Phase 111 + 112
substrate work is closed: the auto-proposer is configurable
through TOML, the inspection flags exist, the A3 amendment
inventory is current, and INSTALL.md reflects the
post-Phase-112 surface.

Standalone phase past Chapter D's close — same precedent
as Phases 111 and 112. Not chapter-framed; opened because
the deferral ledger has accumulated enough small named
items that batching them into one focused phase is cheaper
than carrying them across more substrate phases.

**Phase 20 (Daemon Management + Deferral Cleanup)
precedent:** Phase 20 closed five small daemon-side
deferrals in one phase; Phase 113 mirrors that shape for
operator-surface deferrals.

## Why this, why now

- **Five named deferrals bundle naturally.** TOML loader +
  operator-inspection flags both originate at Phase 112
  exit. A3 amendment addendum has been carried since
  Phase 109 (originally Phase 110-deferred). INSTALL.md
  sweep is Phase 112 exit's own deferral. The Channel
  Activation Milestone is unblocked but the operator-
  surface gaps would make it less useful to run.
- **Phase 112's auto-proposer is functionally unreachable
  without TOML wiring.** The substrate ships ready to fire
  but the binary today wires `skill_auto_proposer: None`
  unconditionally — operator can't enable it without a
  source edit. Phase 113 makes the feature actually
  enable-able through the same config-driven path the
  rest of the system uses.
- **A3 amendment addendum is overdue.** `KNOWN_BASES`
  catalog last refreshed at Phase 54 (43 bases). Post-
  Phase-54 additions: `git.read` (Phase 109),
  `skills.propose` + `skills.list` + `skills.invoke`
  (Phase 110) — four new bases putting the live count
  at ~47. The addendum batches them in one DESIGN.md edit
  rather than churning DESIGN.md per phase.
- **Q1a at sign-off — bundle A3 into Phase 113.**
  Operator-picked over the carve-out alternative; clean
  "all operator-surface deferrals in one phase" framing.
  DESIGN.md streak breaks as predicted at the A-amendment
  file edit.

## Scope (Q-block sign-off)

- **Q1 — A3 scoping:** (a) **Bundle A3 into Phase 113**
  (Recommended; operator-picked at sign-off). The A3
  amendment addendum lands as Task 5; DESIGN.md streak
  breaks at the A-amendment file edit.

## Streak predictions

- **DESIGN.md** — **Will break** as predicted at sign-off
  (Q1a). The A3 amendment addendum (Task 5) edits the
  A-amendment file inside DESIGN.md. Hash at entry:
  `c2be6d51def04207ab0527b4f335edf7c726920b4f102185336eded01b1b995b`.
  Prediction: streak **resets to one** (was 3 after
  Phase 112).

- **PRODUCT.md** — **Will hold.** No P-axis touch. The
  Phase 112 auto-proposer + Phase 110 skills substrate
  already landed inside the P8 (Outcome-Driven Audited
  Reflection) and P10 (Substrate Tools) envelopes; Phase
  113 is operator-surface work entirely inside those
  shipped commitments. Hash at entry:
  `6e840cef2cfcee4c483c2b4769c29cc7062daeb4bf3802c53f48e72ebfe57fb8`.
  Prediction: streak **extends to four** (was 3 after
  Phase 112).

- **Production-core `aivyx-core/src/lib.rs`** — **Will
  hold.** All Phase 113 work lives in `aivyx-config`
  (TOML loader), `aivyx-channel/src/bin/aivyx.rs`
  (DaemonConfig auto-construction + persona-list flags +
  audit-export filter), and DESIGN.md (A3 addendum). No
  `aivyx-core` touch. Hash at entry:
  `deab80d8dded7a59746771a3eb883aad2a55c364ca3743b9bd73b3c7c8837ece`.
  Prediction: streak **extends to two** (was 1 after
  Phase 112).

- **New workspace deps** — Zero. TOML parsing already
  in use (Phase 13+). Filter flag wiring uses existing
  CLI scaffolding.

- **Test count** — Positive but modest. TOML parsing
  tests (~10), filter-flag tests (~6), Daemon-construction
  smoke (~3). Prediction: **+15 to +25**. Smaller than
  Phase 112 because this is wiring + flags rather than
  substrate.

## Tasks

Seven sub-tasks plus exit + backfill, mirroring Phase 112's
shape:

### Task 1 — Open (this commit)

`docs/PHASE_113.md` + `docs/ROADMAP.md` entry + `docs/
README.md` status row.

### Task 2 — TOML `[skills.auto_propose]` config loader

- Promote `SkillAutoProposeConfig` shape from `aivyx-
  channel` into `aivyx-config` (or define a parallel
  `Raw` deserialize struct + `build_…` validator that
  produces a `SkillAutoProposeConfig` — Phase 91
  `RecallJudgmentConfig` precedent).
- Wire the `[skills.auto_propose]` section into the
  top-level `AivyxConfig` struct as
  `pub skill_auto_propose: Option<SkillAutoProposeConfig>`.
  `None` = section absent → feature off; `Some(_)` =
  section present → feature configured per the parsed
  values (defaults backfilled for any field the operator
  omits).
- Validation: thresholds must be in `[0.0, 1.0]`;
  heuristic counts must be sane integers; judge_model
  must be non-empty. Invalid config returns
  `ConfigError::Invalid`.
- Tests: section-absent → None; section-present minimal
  → defaults filled; full section → all fields parsed;
  invalid threshold → error; invalid heuristic count →
  error.

### Task 3 — Daemon auto-construction in bin/aivyx.rs

- When `aivyx_config.skill_auto_propose.is_some()`,
  construct a `SkillAutoProposerContext` from:
  - The parsed `SkillAutoProposeConfig`.
  - The same LLM provider the agent uses (the daemon
    already builds one for the planner).
- Thread the resulting `Arc<SkillAutoProposerContext>`
  through `DaemonConfig::skill_auto_proposer` (replacing
  the unconditional `None` from Phase 112 Task 7).
- The feature stays opt-in via the config section — the
  operator who hasn't configured `[skills.auto_propose]`
  sees no change in behavior. The defaults inside the
  loaded config still match Phase 112's defaults
  (`enabled=true`, threshold `0.85`, etc.).
- Test: a smoke test loading a TOML with the section,
  asserting `DaemonConfig::skill_auto_proposer` is
  `Some(_)`. (Full e2e through the daemon is Phase 112's
  existing `tests/skill_auto_proposer_e2e.rs`; Task 3's
  test just confirms the binary wires the construction
  correctly.)

### Task 4 — Operator-inspection flags

- `aivyx persona list --auto-only` and
  `aivyx persona list --manual-only`:
  - The auto-accepted proposals carry the `pd-auto-`
    `delta_id` prefix (Phase 112 Task 7's
    `write_auto_accepted_skill` synthesizes
    `pd-auto-{proposal_id}`). The filter walks the
    persona list and includes only entries whose
    `delta_id` matches the prefix (`--auto-only`) or
    doesn't (`--manual-only`).
  - Flags are mutually exclusive; specifying both is a
    usage error.
- `aivyx audit export --event-type SkillAutoProposal`:
  - Extends the existing `audit export` filter surface
    (Phase 105). The `event_type` string is matched
    against the chain entry's `event_type` discriminator
    (already populated by the audit-export labelling
    switch, which Phase 112 Task 6 extended to include
    `SkillAutoProposal`).
- Tests: persona-list filter against a fixture chain
  with mixed auto/manual entries; audit-export filter
  against a chain with mixed event variants; mutually-
  exclusive flag rejection.

### Task 5 — A3 amendment addendum

- Edit `DESIGN.md`'s A-amendment file (the
  `## A3 — KNOWN_BASES inventory` section) to add the
  four post-Phase-54 base additions: `git.read`,
  `skills.propose`, `skills.list`, `skills.invoke`.
- Bump the inventory count: `43 → 47`.
- Note the originating phases (109 for `git.read`; 110
  for the three skills bases) so a future auditor can
  trace the additions.
- No test surface; this is documentation.

### Task 6 — INSTALL.md Phase 112 paragraph

- New paragraph covering the `[skills.auto_propose]`
  TOML section: minimal config example, the default-on
  posture (Q3b at Phase 112 sign-off), the threshold
  semantics, the always-revertable-via-Phase-60 escape
  hatch.
- Also link the new inspection flags (`persona list
  --auto-only`, `audit export --event-type
  SkillAutoProposal`) so the operator's mental model is
  complete.

### Task 7 — Exit doc + ROADMAP freeze + README flip + hash backfill

- PHASE_113.md prediction-vs-reality block.
- ROADMAP Phase 113 entry: Active → Frozen.
- docs/README.md status row: Active → Frozen.
- Exit commit hash backfill.

## Exit criteria

- [ ] `docs/PHASE_113.md` + ROADMAP Phase 113 entry +
  docs/README status row — Task 1 (this commit).
- [ ] `[skills.auto_propose]` TOML loader in
  `aivyx-config` + parse/validation tests — Task 2.
- [ ] Binary auto-construction of
  `SkillAutoProposerContext` from the loaded TOML —
  Task 3.
- [ ] `aivyx persona list --auto-only/--manual-only`
  flags + `aivyx audit export --event-type
  SkillAutoProposal` filter — Task 4.
- [ ] A3 amendment addendum in DESIGN.md (`KNOWN_BASES`
  43 → 47) — Task 5.
- [ ] INSTALL.md Phase 112 paragraph — Task 6.
- [ ] Q-block question Q1 resolved with operator sign-off
  pre-Task 2 (recorded above: Q1a — bundle A3 into Phase
  113).
- [ ] DESIGN.md streak break predicted (A3 addendum;
  resets 3 → 1).
- [ ] PRODUCT.md streak extends to four.
- [ ] `aivyx-core/src/lib.rs` streak extends to two.
- [ ] Zero new workspace dependencies.
- [ ] Test count delta positive — predicted `+15` to
  `+25`.
- [ ] Zero clippy warnings.
- [ ] **All five named operator-surface deferrals
  closed.** The auto-proposer is configurable, the
  inspection flags exist, the A3 inventory is current,
  INSTALL.md reflects the post-Phase-112 surface.
