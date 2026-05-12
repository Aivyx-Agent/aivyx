# Phase 60 — Persona Visualization (closes P14)

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Close the **Persona** milestone (P14) and the entire **Profile +
Persona forward arc** (Phases 56–60). Phase 59 shipped the
Persona substrate (storage + chain + propose + apply + assembly).
Phase 60 ships the operator-facing surfaces that complete the
milestone:

1. **`aivyx persona show` / `list` CLI subcommands** — labeled
   inspection of the effective Persona and the delta log,
   mirroring Phase 58's `aivyx profile show`.
2. **Per-turn planner-factory refresh** — closes the Phase 59
   Q5(a) deferral. Approved Persona deltas take effect on the
   next turn without daemon restart, matching the Phase 30
   `role_overrides` precedent.
3. **Web UI Persona pane** — read-only timeline view of the
   delta log served via new `Query::ListPersonaDeltas` and
   `Query::GetEffectivePersona` IPC envelopes.
4. **Revert flow** — operator-driven mechanism to undo a prior
   approved delta. Honors P14 commit 4 "operator-reversible"
   via a structured revert delta that nullifies a previous
   approved delta by id.
5. **PRODUCT.md Delivery Status refresh** — P14 moves from
   *Forward* to *Fully Delivered*. After this phase, **every
   P1–P14 commitment is fully shipped** and the
   forward-commitment ledger closes.

**Identity export/import** (flagged as optional in P14 and in
ROADMAP.md Phase 60) is deferred to a future micro-phase if
operator feedback surfaces real pressure. Phase 60 ships the
inspection, edit-by-proposal, and revert surfaces — the
substrate is complete with those.

## Why now

1. **P14 is the last forward commitment.** Phase 59 shipped
   its substrate; Phase 60 ships its operator surface. Closing
   it here closes the entire forward-commitment ledger
   (P1–P14 fully delivered) and ends the Profile + Persona
   forward arc.

2. **The Phase 59 Q5(a) deferral is well-bounded.** Per-turn
   planner refresh has a clean precedent (`role_overrides` from
   Phase 30) and the substrate (`SharedEffectivePersona`) is
   already in place. The refresh is a small focused change to
   the four planner-factory call sites, not new architecture.

3. **No load-bearing dependency on a future phase.** Phase 60
   needs no contract amendment, no new substrate. Every piece
   layers on top of what Phases 47, 56–59 already shipped.

## Streak predictions

- **DESIGN.md** — **Will hold.** Phase 60 ships CLI + Web UI +
  IPC variants + revert delta extension. No D-deliverable
  reshape. Prediction: streak **extends to seven** consecutive
  phases (currently at 6).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will break (intentional)** at Task 6's
  Delivery Status refresh (P14 → Fully Delivered). Same shape
  as Phase 58 Task 5. Prediction: streak **ends at two**
  consecutive phases (Phases 57 + 58 break + 59 untouched
  recovered to 1, this phase extends to 2 before the break).
  Hash at entry: `6bd91519f28370d72b382f9a87044230d7073f276e83d79c1dfb413235c54977`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold.**
  Phase 60's surface lives in `aivyx-channel` (CLI + Web UI
  HTML + IPC variants + apply-side revert handling) and never
  touches `aivyx-core`. Prediction: streak **extends to
  eight** consecutive phases (currently at 7, matching Phase
  59 exit).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

## Tasks

### Task 1 — Open commit + PHASE_60.md scaffold

This file. Update `docs/README.md` to show Phase 60 as Open.
Commit Q-block resolutions before proceeding to Task 2.

### Task 2 — `aivyx persona show` / `list` CLI subcommands

New `CliMode::Persona(PersonaSubcommand)` variant per Q1.
`show` renders the effective Persona (folded chain state) in
labeled banner format. `list` enumerates the delta log with
ids, timestamps, categories, and ops. Both subcommands open
the storage layer with relaxed validation (no API key required,
same shape as `aivyx profile show`).

Tests: parser tests (parses, missing-subcommand, unknown
subcommand, extra-args), render tests against fixture
chains.

### Task 3 — Per-turn planner-factory refresh (closes Phase 59 Q5(a))

Move the `assemble_session_prompt` call from session-build into
the per-turn planner factory closures. The four sites:
parent `run_session` path, `daemon run` path, Telegram
in-process fallback, role-switch child factory. Each captures
`profile`, `active_role_name`, `role_system_prompt`, and
`SharedEffectivePersona` into the closure; per-turn reads the
shared state under the read lock and rebuilds the prompt.

After this task, an approved Persona delta takes effect on the
very next turn without daemon restart — matching the
`role_overrides` precedent and the Q5(a) intent.

The repetition across four sites is the cost of the design;
a future refactor can lift a `SessionPromptBuilder` helper if
the pattern grows further.

Tests: focused unit test that reading the shared state and
re-assembling produces a different prompt after a delta is
applied.

### Task 4 — `Query::ListPersonaDeltas` + `Query::GetEffectivePersona` IPC

Two new variants on `QueryPayload` / `QueryResponsePayload` per
the Phase 47 inspection-query precedent. `ListPersonaDeltas`
returns the delta log with pagination shape matching
`ListAuditEntries` (`from_seq`, `limit`, server-side cap).
`GetEffectivePersona` returns the current snapshot of the
shared state.

`DaemonConfig` gains a `persona_log: Arc<PersistentPersonaLog>`
field and a `shared_persona: SharedEffectivePersona` field
threaded through `ConnectionContext` into `handle_query`. Two
new conversion helpers on the wire side
(`PersonaDeltaSummary`, `EffectivePersonaSummary`).

Tests: IPC round-trip (request + response shapes), handler
unit tests for both variants.

### Task 5 — Web UI Persona pane

New "Persona" tab in `web_ui_static.html` after the existing
Profile tab. Two-section layout:
- "Effective Persona" — current folded state, mirroring the
  `aivyx persona show` output.
- "Delta timeline" — paginated list of approved deltas with
  per-entry click-to-revert per Q3.

Per Q3, the revert click sends a new `FrontendMessage`-side
request (or reuses the reflection.propose path via a
synthetic proposal — Q4 chooses). The simplest path keeps
the substrate symmetric: the operator's revert intent becomes
a proposal that auto-gates with the operator's identity (no
gate prompt since the operator is the proposer).

### Task 6 — Revert delta mechanism

Per Q4, extend `PersonaDeltaOp` (or add a sibling op) to
support reverting a prior approved delta by id. The simplest
shape: append a new delta whose op is `Revert { target_delta_id }`,
and update `apply_delta_to_state` (in `persona.rs`) to apply
the *inverse* of the target's op when folding a revert into the
runtime state. This preserves the append-only chain invariant
while making the operator-reversible commitment (P14 commit 4)
concrete.

Tests: revert-after-SetScalar restores the prior value; revert-
after-AppendList removes the value; revert-of-a-revert restores
the original delta.

### Task 7 — PRODUCT.md Delivery Status refresh (P14 → Delivered)

Move P14 from Forward to Fully Delivered in PRODUCT.md.
Update the Delivery Status header to "as of Phase 60 exit".
Add the Phase 59 + 60 phase references. Add a "**P1–P14
are all fully shipped**" status statement — the
forward-commitment ledger closes here.

This is the streak-breaking edit predicted above.

### Task 8 — Roadmap refresh + Exit freeze

Update `docs/PRODUCT_ROADMAP.md` Persona milestone status to
delivered. Update `docs/ROADMAP.md` Phase 60 entry from
Scheduled to Frozen. Note the Profile + Persona arc closure
and the project's new state: every commitment delivered;
post-arc posture re-established.

Prediction-vs-reality block, exit criteria checkboxes,
README phase row Frozen + exit commit hash backfill.

## Deferrals

**Rolling deferrals carried into Phase 60:** the Phase 59
Task 6 per-turn freshness deferral — closed by this phase's
Task 3.

**Net-new deferrals from Phase 60:**
- **Identity export/import** — operator backup/transfer of
  the Persona chain. P14's contract permits it; ROADMAP.md
  Phase 60 flagged it as optional. Deferred to a future
  micro-phase if operator pressure surfaces. Re-binding the
  HMAC chain on import is the load-bearing decision (the
  delta_ids are deterministic but the chain seeds and key
  diverge across hosts) and warrants its own focused phase.

## Prediction vs. reality

- **DESIGN.md** — Predicted: streak **extends to seven**.
  **Reality: correct.** Hash unchanged at entry and exit:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Phase 60 shipped under existing D-deliverables.

- **PRODUCT.md** — Predicted: streak **ends at two**
  (Task 7 Delivery Status refresh, intentional). **Reality:
  correct.** Hash at entry:
  `6bd91519f28370d72b382f9a87044230d7073f276e83d79c1dfb413235c54977`.
  PRODUCT.md held byte-identical through Tasks 1–6 and broke
  intentionally in Task 7 (`d97bf1b`) when P14 moved from
  Forward to Fully Delivered and the ledger-closure
  language landed.

- **Production-core `aivyx-core/src/lib.rs`** — Predicted:
  streak **extends to eight**. **Reality: correct.** Hash
  unchanged:
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.
  All Phase 60 surface routed through `aivyx-channel` (CLI
  + IPC + Web UI HTML + revert folder) without touching
  `aivyx-core`.

## Exit criteria

- [x] `aivyx persona show` + `aivyx persona list` + `aivyx
  persona revert` CLI subcommands wired through
  `parse_cli_args` + `run` — Task 2, commit `8600c04`.
- [x] Per-turn planner refresh in three planner-factory
  sites (parent / daemon-run / role-switch child) — Task 3,
  commit `25a9bf8`. Telegram in-process fallback deferred
  per the scope note in Task 3 ship record.
- [x] `QueryPayload::ListPersonaDeltas` +
  `QueryPayload::GetEffectivePersona` + handlers + `FrontendMessage::RevertPersonaDelta`
  + `DaemonMessage::PersonaRevertResolved` — Task 4, commit
  `b7f356d`.
- [x] Web UI Persona pane rendering live state + click-to-
  revert — Task 5, commit `879efbc`.
- [x] Revert delta mechanism (`PersonaDeltaOp::Revert` +
  inverse-apply folder + revert-of-revert) — Task 6, commit
  `65e7951`.
- [x] PRODUCT.md Delivery Status: P14 → Fully Delivered —
  Task 7, commit `d97bf1b`.
- [x] PRODUCT_ROADMAP + ROADMAP refreshed; milestone closed.
- [x] All five Q-block questions resolved (defaults
  signed off pre-Task 6).
- [x] DESIGN.md streak extends to seven.
- [x] PRODUCT.md streak ends at two (Task 7 intentional).
- [x] Production-core streak extends to eight.
- [x] Test count delta: +19 (1052 → 1071 across the
  workspace), zero clippy warnings.
- [x] Prediction-vs-reality block filled.

## Open questions

**Q1 — CLI surface shape.** What does the `persona`
subcommand surface look like?

  - **(a)** `CliMode::Persona(PersonaSubcommand)` mirroring
    Phase 58 with `Show` and `List` variants. `show` prints
    the folded effective state; `list` prints the per-delta
    timeline.
  - **(b)** Single `show` subcommand that prints both the
    effective state and the timeline together.
  - **(c)** Add a `revert` subcommand alongside `show` /
    `list` — operator can revert via CLI as well as Web UI.

  **Recommendation: (c).** The nested enum scales; adding
  `revert` now (rather than later) means the substrate has
  CLI + Web UI revert paths from the start. `revert` takes
  a delta_id and propose-then-apply-and-auto-gate per Q4
  resolution.

**Q2 — Per-turn refresh threading.** Each planner factory
needs `Profile`, `active_role_name`, `role_system_prompt`,
and `SharedEffectivePersona` to rebuild the prompt per turn.
How?

  - **(a)** Capture the four values directly in each factory
    closure. Repetitive across four sites but simple.
  - **(b)** Build a `SessionPromptBuilder` struct that bundles
    them and lift the per-turn assemble into a method.
    Reduces repetition but introduces a new type for a
    short-lived helper.

  **Recommendation: (a).** The repetition is bounded (four
  sites), and each site already manages a different set of
  factory captures. A `SessionPromptBuilder` would be premature
  abstraction. If a fifth site lands, lift then.

**Q3 — Web UI revert UX.** How does the operator revert a
delta from the Web UI?

  - **(a)** Click a "Revert" button next to each delta in the
    timeline. Browser sends a new `FrontendMessage` that the
    daemon handles as an auto-approved revert proposal
    (operator-initiated, no gate needed).
  - **(b)** Read-only Web UI; revert is CLI-only via
    `aivyx persona revert <id>`. Web UI is purely
    inspection. Simpler scope; loses parity with profile/
    mission panes that already have action buttons.

  **Recommendation: (a).** Phase 47's mission gates already
  have Approve/Deny buttons — the precedent exists. The Web
  UI surface is more discoverable for revert actions than
  the CLI. New `FrontendMessage::RevertPersonaDelta { delta_id }`
  variant.

**Q4 — Revert mechanism shape.** How is a revert recorded
in the chain?

  - **(a)** New `PersonaDeltaOp::Revert { target_delta_id }`
    variant. The `apply_delta_to_state` folder looks up the
    target delta and applies its inverse operation.
  - **(b)** Revert is mechanical at the runtime layer (the
    folder consults a "revoked" set when folding); the chain
    records a `Revoke` event rather than a `PersonaDelta`.
    Adds a chain-event-type distinction; complicates the
    HMAC chain shape.
  - **(c)** Reverts compose at the proposal layer: an
    `AppendList` revert becomes a `RemoveList` delta; a
    `SetScalar` revert becomes a `SetScalar { value: <prior> }`
    delta with the prior value embedded. No new op variant
    but the propose-side needs to walk the chain to find
    the prior value.

  **Recommendation: (a).** Cleanest substrate impact — one
  new `PersonaDeltaOp` variant, one folder arm. The
  target_delta_id is the operator-meaningful handle (matches
  what the CLI / Web UI surfaces show). The folder consults
  the running chain (which it already walks) for the
  target's op. (b) reshapes the chain shape unnecessarily;
  (c) front-loads work to the propose side and loses the
  "this entry IS a revert" semantic legibility.

**Q5 — Revert authority.** Who can propose a revert?

  - **(a)** Operator only — reverts come from the CLI
    `persona revert` or Web UI "Revert" button. Agent
    cannot propose a `Revert` op via reflection.propose.
    Auto-gated at apply (no operator-approval prompt needed
    since the operator initiated it).
  - **(b)** Agent can propose reverts via reflection.propose;
    operator approves through the same mission-gate flow as
    any other delta. Adds the agent as a potential
    revert-proposer.

  **Recommendation: (a).** Reverts are operator-initiated
  by their nature — they undo a prior decision the operator
  approved. The agent proposing a revert would be
  contractually odd (why approve something then propose to
  undo it?). Keeping the agent's proposal surface to
  forward-mutation deltas and reserving reverts for the
  operator is cleaner. The Q5 boundary aligns with the P14
  commit 4 framing of operator-reversibility.
