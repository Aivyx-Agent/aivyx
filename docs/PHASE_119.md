# Phase 119 — Phase 118 Apply-Side Closeout + Tool-Relevance Dump (Operator-Value Polish)

Audit-informed phase opened against the operator-value
polish layer identified during the post-Phase-118
codebase audit. Phase 118 shipped both Profile/Role
auto-proposer categories with always-staged routing, but
the operator's action surface stopped at "approve in
chain." Acting on an approved hint still required
hand-editing `aivyx.toml`. Phase 119 closes that gap and
ships the deferred Phase 116 `aivyx tool-relevance dump`
CLI in the same sweep.

**Q-block — all three Recommended picks.** First Q-block
since Phase 116 to land all-Recommended; consistent with
the audit's "scoped polish layer" framing.

**Audit context — seventh phase in a row of substrate/polish
work picked over the Channel Activation Milestone.** Tracking
this honestly as a pattern, not a problem: Phase 112-118
substrate kept shipping cleanly with predictions held;
Phase 119 continues the substrate-polish posture for one
more focused phase. The Channel Activation Milestone
becomes harder to defer after Phase 119 closes the
operator-value loop mechanically.

## Why this, why now

- **Phase 118 mechanically incomplete on the operator
  side.** `aivyx persona proposals show <id>` renders the
  Phase 118 draft beautifully and ends with "To apply:
  edit aivyx.toml [profile] and update the field above" —
  but the operator still has to translate the rendered
  block into a TOML edit by hand. Phase 119 ships the
  apply-helper CLI commands that close that gap.
- **Phase 116 deferred-CLI ledger.** Phase 116 Task 7's
  exit notes left `aivyx tool-relevance dump` deferred
  ("until the live-prompt path lands, the prompt section
  IS the inspection surface"). Phase 117 closed the
  live-prompt deferral; Phase 119 closes the
  inspection-CLI deferral. Bundling here keeps the
  deferral ledger tidy.
- **Single audit-event posture.** The audit chain
  carries the operator's approve action through
  `PersonaProposalResolved`; it currently does NOT carry
  the operator's act-on-approval action (Phase 118
  closure stops at "operator copied into aivyx.toml,
  somehow"). Phase 119 adds dedicated `ProfileHintApplied`
  and `RoleDraftImported` audit-event variants so forensic
  walks can answer "the operator approved AND acted on
  this hint" definitively.

## Scope (Q-block sign-off)

- **Q1 — Apply-side scope:** (a) **Both apply-helpers +
  the deferred tool-relevance dump** (Recommended).
  Three CLI commands ship in this phase:
  - `aivyx profile apply-hint <id>`
  - `aivyx role import <id>`
  - `aivyx tool-relevance dump`

- **Q2 — Approval-vs-apply boundary:** (a) **Separate
  commands** (Recommended). The existing
  `aivyx persona proposals approve <id>` stays
  category-agnostic; it lands the chain entry as
  Approved. The new apply-helpers are independent
  operator gestures the operator runs when they're
  ready to act. Mirrors Phase 110 LearnedSkill's
  approval semantics (the runtime picks up approved
  skills from the persona chain; the operator never
  "applies" a learned skill separately).

- **Q3 — Audit-event shape:** (a) **New audit-event
  variants** (Recommended). Two additive `AuditEvent`
  variants in `aivyx-audit`:
  - `ProfileHintApplied { session_id, proposal_id,
    field, applied_value }`
  - `RoleDraftImported { session_id, proposal_id,
    role_name, parent }`
  Both `#[serde(default, skip_serializing_if = ...)]`
  on any optional fields per the Phase 92 / Phase 118
  wire-compat precedent. Audit-chain HMAC integrity
  preserved.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract amendment.
  Phase 119 ships operator-facing CLI surface; the
  P13/P9 always-staged contract from Phase 118 stays
  intact (CLI apply doesn't bypass operator decision —
  the operator chose to run the command). Hash at entry:
  `c2be6d51def04207ab0527b4f335edf7c726920b4f102185336eded01b1b995b`.
  Prediction: streak **extends to ten** (was 9 after
  Phase 118).

- **PRODUCT.md** — **Will hold.** P8 (outcome-driven
  audited reflection) covers the new audit-event
  variants. P13 (Profile operator-declared) and P9
  (Role config operator-curated) both stay intact —
  the CLI is operator-driven by definition. Hash at
  entry: `6e840cef2cfcee4c483c2b4769c29cc7062daeb4bf3802c53f48e72ebfe57fb8`.
  Prediction: streak **extends to ten** (was 9 after
  Phase 118).

- **Production-core `aivyx-core/src/lib.rs`** —
  **Probably holds.** The new audit-event variants land
  in `aivyx-audit`, not core. Most CLI-side work lives
  in `aivyx-channel/src/bin/aivyx_modules/`. The only
  way lib.rs breaks is if the daemon-side wiring needs
  new `AuditTag` variants to bridge AuditWriter calls
  from the CLI request handler — which is the Phase 117
  `AuditTag::SkillInvocation` precedent (broke lib.rs).
  Honest 70/30 hold — if I can keep the apply requests
  flowing through the existing IPC + audit pathways
  without bridging via AuditTag, lib.rs holds. Hash at
  entry: `d1d4373bcf54b0390b1e2c15efec4dfa50dd29f74f57aa7ca6950752603f4ae7`.
  Prediction: streak **extends to three** (was 2 after
  Phase 118).

- **New workspace deps** — Zero.

- **Test count** — Substrate is mostly CLI plumbing +
  atomic TOML edit + audit-event round-trip + e2e
  apply-on-approved-chain-entry. Prediction:
  **+25 to +45**.

## Tasks

Roughly seven sub-tasks plus exit + backfill:

### Task 1 — Open (this commit)

`docs/PHASE_119.md` + `docs/ROADMAP.md` Phase 119 entry +
`docs/README.md` status row.

### Task 2 — Audit-event variants

- New `AuditEvent::ProfileHintApplied { session_id,
  proposal_id, field, applied_value }` variant in
  `aivyx-audit`.
- New `AuditEvent::RoleDraftImported { session_id,
  proposal_id, role_name, parent }` variant.
- `#[serde(default)]` on any fields that might
  later need wire-compat extensions; preserve the
  byte-identical canonical-JSON pattern for pre-Phase-119
  chain entries.
- Tests: round-trip via serde-jcs; HMAC chain
  hash-stable; wire-compat (pre-Phase-119 chains decode
  unchanged).

### Task 3 — Atomic TOML editing primitive

- New `aivyx-channel/src/bin/aivyx_modules/toml_edit.rs`
  module (or similar location) with:
  - `apply_profile_field(toml_path, field,
    value) -> Result<(), Error>` — parses the existing
    `aivyx.toml`, applies the field update under
    `[profile]`, writes atomically (tmp-file + rename),
    preserves comments + formatting via the `toml_edit`
    crate (NB: `toml` crate doesn't preserve comments;
    `toml_edit` does — confirm before adding).
  - `apply_role_section(toml_path, role_draft) ->
    Result<(), Error>` — adds a new `[roles.<name>]`
    section; refuses if a role with the same name
    already exists (or `--force` provided).
- The `toml_edit` crate IS already a dep transitively
  via something? Verify before adding; if not, this is a
  new workspace dep — flag it for Q-block re-sign-off.
- Tests: round-trip with comments preserved; refuses to
  overwrite without --force; atomic write doesn't leave
  a partial file on disk if it fails mid-write.

### Task 4 — `aivyx profile apply-hint <id>`

- New CLI subcommand under `aivyx profile apply-hint
  <proposal-id>`.
- Reads the proposal via daemon IPC
  (`GetPersonaProposal`); refuses if not
  `ProfileHint` category; parses the
  `ProfileFieldHint` payload; confirms with the operator
  (`--yes` to skip); writes to `aivyx.toml` via the
  Task 3 primitive; emits the `ProfileHintApplied`
  audit-event via daemon IPC (new
  `FrontendMessage::ApplyProfileHint` variant +
  matching daemon handler).
- Surfaces "daemon restart required for the new value
  to take effect" guidance.
- Tests: CLI parsing; refuses non-ProfileHint
  proposals; refuses pending proposals (must be
  Approved); end-to-end apply round-trip.

### Task 5 — `aivyx role import <id>`

- New CLI subcommand `aivyx role import
  <proposal-id> [--force]`.
- Reads the proposal; refuses if not
  `RoleDefinitionSuggestion`; parses `RoleDraft`;
  confirms with the operator; writes `[roles.<name>]`
  section; emits `RoleDraftImported` audit-event;
  surfaces daemon-restart guidance.
- Tests: CLI parsing; refuses overwrite without
  `--force`; honors parent inheritance correctly in
  the written section; end-to-end apply round-trip.

### Task 6 — `aivyx tool-relevance dump`

- New CLI subcommand. Reads the
  `KeyDomain::ToolRelevanceLedger` via daemon IPC
  (new `Query::DumpToolRelevance` + matching response
  payload).
- Renders per-keyword-key outcome rows in a human-
  readable table:
  ```
  keyword_key                     surface  identifier        success  failure  last_seen
  research+deploy                 tool     fs.read                7        1   2026-05-28T17:00:00Z
  research+deploy                 skill    summarize-pdf          3        0   2026-05-28T16:42:00Z
  ```
- Optional `--keyword-key <key>` filter.
- Tests: CLI parsing; renders empty ledger cleanly
  (no panic); renders populated ledger in stable
  column order.

### Task 7 — Scripted e2e + INSTALL.md sweep + exit

- Scripted e2e (`tests/phase_119_apply_e2e.rs` or
  extend `skill_auto_proposer_e2e.rs`):
  1. Auto-propose a ProfileHint at confidence 1.0 →
     Staged.
  2. Operator approves via the proposal API.
  3. Operator runs `apply_profile_field` on the
     approved entry (helper directly; CLI invocation
     is its own integration test).
  4. Assert `aivyx.toml` now has the suggested value
     under `[profile]`.
  5. Assert the audit chain has a
     `ProfileHintApplied` event with the right
     proposal_id linkage.
  6. Same shape for `RoleDraftImported`.
- INSTALL.md: extend the Phase 118 section with a
  "Phase 119 — operator-action commands" sub-section.
  Replace the "To apply: edit aivyx.toml manually"
  guidance with `aivyx profile apply-hint <id>` +
  `aivyx role import <id>` examples. Add a new
  "Inspecting the tool-relevance ledger" sub-section
  for the dump command.
- Exit: PHASE_119.md prediction-vs-reality + ROADMAP
  freeze + README flip + hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Apply-side scope:** (a) **Both apply-helpers +
  tool-relevance dump** (Recommended). Three CLI
  commands ship in this phase.
- **Q2 — Approval-vs-apply boundary:** (a) **Separate
  commands** (Recommended). Approve lands chain entry;
  apply (later, separate operator gesture) writes to
  aivyx.toml.
- **Q3 — Audit-event shape:** (a) **New audit-event
  variants** (Recommended). Additive `ProfileHintApplied`
  + `RoleDraftImported` variants in `aivyx-audit` with
  wire-compat serde defaults.

## Exit criteria

- [ ] `docs/PHASE_119.md` + ROADMAP Phase 119 entry +
  docs/README status row — Task 1 (this commit).
- [ ] Audit-event variants — Task 2.
- [ ] Atomic TOML editing primitive — Task 3.
- [ ] `aivyx profile apply-hint <id>` CLI — Task 4.
- [ ] `aivyx role import <id>` CLI — Task 5.
- [ ] `aivyx tool-relevance dump` CLI — Task 6.
- [ ] Scripted e2e + INSTALL.md sweep — Task 7.
- [ ] Q1/Q2/Q3 resolved with operator sign-off pre-Task
  2 (all three Recommended).
- [ ] DESIGN.md streak — predicted HOLD (streak → 10).
- [ ] PRODUCT.md streak — predicted HOLD (streak → 10).
- [ ] `aivyx-core/src/lib.rs` streak — predicted HOLD
  (streak → 3), honest 70/30 hold.
- [ ] Zero new workspace dependencies, OR honest Q-block
  re-sign-off if `toml_edit` crate isn't already
  transitively present.
- [ ] Test count delta within `+25` to `+45`.
- [ ] Zero clippy warnings.
- [ ] Phase 118 operator-value loop **mechanically
  closed**: the operator's gesture from "approve a Phase
  118 proposal" to "live config reflects the approval"
  is a CLI command, not a hand-edit.
- [ ] Phase 116 `aivyx tool-relevance dump` deferral
  closed.

## Direction after Phase 119

After Phase 119, every named substrate / operator-value
deferral from Phases 112-118 is closed. The Channel
Activation Milestone becomes the highest-information-value
direction — the substrate is mechanically complete; the
remaining question is "does any of it work in real use?"

Phase 120 candidates per the audit ranking:

1. **Channel Activation Milestone** — long-deferred
   operator-verification pass. Real-bot Telegram +
   Discord + Slack; cross-channel regression sweep;
   Phase 116/117/118 self-learning loop verification
   against real-protocol traffic. Highest information
   value.
2. **Release prep (v0.1.0 + shell installer)** — the
   substrate is mature enough; Phase 99 deferred this
   pending repo-infrastructure decision.
3. **Local-LLM rehabilitation** — qwen3.6/gemma4
   tool-name hallucination. G6 (privacy non-negotiable)
   distinguishing claim deserves first-class local-LLM
   support.
4. **A new thematic Chapter F** — observability,
   multi-agent shapes, distribution prep — shaped by
   what the Channel Activation Milestone surfaces.

Phase-by-phase decision at Phase 119 exit.
