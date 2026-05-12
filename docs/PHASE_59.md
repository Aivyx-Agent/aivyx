# Phase 59 — Persona Foundation (P14)

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

First code phase of the **Persona** half of the Profile +
Persona forward arc (Phases 56–60; P14 amendment A10 filed
Phase 56). Phase 59 lands the Persona substrate:

1. **`PersonaDelta` records** — structured proposals (one
   field-edit per delta per Q1) carrying category, payload,
   timestamp, and an HMAC-chained sequence number.
2. **`KeyDomain::Persona` storage** — parallel HMAC chain
   (per Q2) ordered append-only, audit-verifiable via
   `aivyx --verify-only` once the runtime walker is wired
   (Phase 60 territory; Phase 59 only writes the chain).
3. **Proposal surface** — `reflection.propose` extended (per
   Q4) with a new `persona_deltas` field. Operator approves
   through the existing P2 mission-gate machinery; on
   approval, `reflection.apply` writes the deltas to the
   Persona chain.
4. **Effective Persona** — a shared runtime state computed
   at daemon startup from the chain, then mutated atomically
   on each `reflection.apply` (per Q5 hot-reload semantics —
   same shape as `role_overrides` from Phase 30).
5. **System-prompt extension** — `assemble_session_prompt`
   gets a third labeled section (`## How I have learned to
   communicate`) per Q6, inserted between the Profile
   section and the Active Role section. Empty Persona =
   passthrough (non-invasive default).
6. **`persona.propose` capability scope** — new base in
   `aivyx-capability` controlling proposal authority.

Phase 60 closes the milestone with operator-facing surfaces
(`aivyx persona show` / Web UI pane / revert deltas /
optional export-import). Phase 59 ships the engine; Phase 60
ships the dashboard.

## Why now

1. **The contract is in place.** P14 landed Phase 56
   (amendment A10) and Phase 58 closed P13. P14 is the
   last remaining forward commitment; opening its
   implementation now is the natural sequence.

2. **The substrate Phase 59 needs already exists.** P8's
   reflection layer (`reflection.propose` /
   `reflection.apply`), P2's mission-gate machinery, the
   HMAC chain primitive from `aivyx-audit`, the encrypted-
   redb storage from `aivyx-storage`, and the
   `assemble_session_prompt` helper from Phase 57 are all
   in production. Phase 59 wires them together against a
   new `PersonaDelta` type and a new storage domain.

3. **No load-bearing dependency on Phase 60.** The
   operator-facing surfaces (`aivyx persona show`, Web UI
   pane, revert flow) need the substrate Phase 59 ships;
   Phase 59 does not need them. The phases sequence
   cleanly.

## Streak predictions

- **DESIGN.md** — **Will hold.** Phase 59 ships under
  existing D-deliverables (D2 audit chain extends via a
  new domain, D4 capability taxonomy gains one base, D5
  storage gains one domain). No D-deliverable adds or
  reshapes. Prediction: streak **extends to six**
  consecutive phases (currently at 5 since Phase 54's A3
  addendum break).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** P14 is in the contract;
  Phase 59 implements it. Delivery Status refresh waits
  for Phase 60's milestone closure. Prediction: streak
  **extends to one** consecutive phase (just broke at 2
  in Phase 58 Task 5).
  Hash at entry: `6bd91519f28370d72b382f9a87044230d7073f276e83d79c1dfb413235c54977`.

- **Production-core `aivyx-core/src/lib.rs`** — **At risk.**
  Persona-aware planner mutation per-turn (the hot-reload
  shape per Q5) may need a new error variant or a small
  type addition in `aivyx-core`, similar to how Phase 51
  D6 added `AivyxError::{Storage,Crypto}`. If the design
  cleanly threads through `aivyx-channel` only (matching
  the Phase 30 `role_overrides` precedent), the streak
  extends. If a new `aivyx-core` shape is needed, it
  breaks. Prediction noted as **at risk**; resolution
  surfaces during Task 2.
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

## Tasks

### Task 1 — Open commit + PHASE_59.md scaffold

This file. Update `docs/README.md` to show Phase 59 as Open.
Commit Q-block resolutions before proceeding to Task 2.

### Task 2 — `PersonaDelta` + `KeyDomain::Persona` storage

Define `PersonaDelta` in a new `aivyx-channel::persona`
module (or extend `aivyx-config` — choice depends on
where Profile naturally couples; Q-block does not pin this).
Fields:

- `delta_id: String` (uuid)
- `proposed_at_unix_ms: u64`
- `approved_at_unix_ms: u64`
- `proposal_id: String` (back-pointer to reflection mission)
- `category: PersonaDeltaCategory` (enum per Q3)
- `payload: PersonaDeltaPayload` (Add / Replace / Append)
- `mac_hex: String` (HMAC over `seq | prev_mac | body`,
  same shape as the audit chain)

Add `KeyDomain::Persona` to `aivyx-storage`. Persist the
chain as one row per delta keyed by sequence number.
Verify-chain semantics piggyback on the existing
HmacChainLog pattern from `aivyx-audit`.

Unit tests:
- Single delta written and read back round-trips through
  the storage layer
- Two deltas chain correctly (`prev_mac` linkage)
- Tampered delta fails verification

### Task 3 — `persona.propose` scope + reflection.propose extension

Add `persona.propose` to `aivyx-capability::KNOWN_BASES` and
to the `CEILING_TRUSTED` set. Extend the JSON schema and
input type of `ReflectionProposeTool` to accept a new field
`persona_deltas: Vec<ProposedPersonaDelta>` alongside the
existing `memory_writes`, `prompt_append`, `allowlist_changes`.

Extend `ProposalRecord` (serialized inside the mission
description) to carry the deltas through gate approval.
The gate prompt rendering gains a "Persona deltas: N"
line so the operator sees what they are approving.

Tests:
- Propose with persona_deltas only — mission created
- Propose with mixed memory + persona deltas — both
  serialize / deserialize through the mission record
- Propose without persona.propose scope when scope is
  required — denied at the existing per-tool gate

### Task 4 — `reflection.apply` writes approved deltas

Extend `ReflectionApplyTool` to, on an approved mission
whose proposal carries persona deltas, append each
delta to the `KeyDomain::Persona` chain (with operator's
approval timestamp captured as the delta's
`approved_at_unix_ms`). The apply tool returns the count
of deltas committed alongside the existing
`writes_executed` count.

Atomic semantics: all deltas of a single approval batch
land at consecutive sequence numbers, with each `prev_mac`
correctly chained. If any append fails mid-batch, the
chain stays at the last-successful sequence (partial
batches are allowed; the caller can re-apply).

### Task 5 — `EffectivePersona` runtime state + planner integration

Build a `compute_effective_persona(chain) -> EffectivePersona`
function that walks the persona chain at daemon startup and
folds approved deltas into a structured runtime state. The
state mirrors the Persona's category enum from Q3.

Wire `Arc<RwLock<EffectivePersona>>` through `DaemonConfig`
(or a sibling channel struct) so the planner factory can
read it per-turn. Match the Phase 30 `role_overrides`
threading pattern. `reflection.apply` writes both the chain
AND mutates the shared state under the lock.

For the in-process (non-daemon) path: cleaner if Phase 59
defers hot-reload there and only loads at startup, since
the in-process path has fewer turns per session and a
restart is cheap.

### Task 6 — `assemble_session_prompt` extends with Persona section

Per Q6, extend the system-prompt assembly to compose three
labeled sections when applicable:

```
## About this assistant      <-- Profile (Phase 57)
...

## How I have learned to communicate    <-- Persona (Phase 59)
...

## Active role: <name>      <-- Role envelope
...
```

Empty Persona → omit the section entirely (passthrough
behavior preserved). Update unit tests in
`profile_prompt.rs` (or new module if substantial) to
cover the three-section layout.

### Task 7 — Tests + exit freeze

Workspace test suite passes; clippy clean. Streak
prediction-vs-reality block, exit criteria checkboxes,
README phase row Frozen + exit commit hash backfill.

## Deferrals

**Rolling deferrals carried into Phase 59:** none.

**Net-new deferrals from Phase 59 (planned for Phase 60):**

- **Revert-delta mechanism** — P14 commit 4 commits to
  operator-reversibility. The mechanism (append a structured
  revert delta) is straightforward but the **UX** (Web UI
  click-to-revert, CLI subcommand, gate prompt rendering for
  reverts) belongs with the Phase 60 dashboard.
- **`aivyx persona show / list` CLI** — operator-facing
  inspection mirrors Phase 58's `aivyx profile show`. Same
  scope: read the chain, render the effective Persona +
  the delta log.
- **Web UI Persona pane** — timeline view of approved
  deltas, current effective Persona, click-to-revert.
- **Identity export/import** — back up / transfer a
  Persona chain. Touches HMAC chain semantics (the import
  re-binds the chain), so it lands with Phase 60's polish.

## Prediction vs. reality

*(Filled at exit.)*

## Exit criteria

*(Filled at exit.)*

- [ ] `PersonaDelta` type + `KeyDomain::Persona` storage
  shipping with HMAC-chained writes (Task 2).
- [ ] `persona.propose` capability scope registered in
  `aivyx-capability::KNOWN_BASES` + `CEILING_TRUSTED`
  (Task 3).
- [ ] `reflection.propose` accepts `persona_deltas` in its
  JSON schema and propagates them through the mission
  record (Task 3).
- [ ] `reflection.apply` appends approved deltas to the
  Persona chain (Task 4).
- [ ] `EffectivePersona` runtime state shared with the
  planner factory; updated atomically on apply (Task 5).
- [ ] `assemble_session_prompt` extended with the
  "How I have learned to communicate" section (Task 6).
- [ ] All six Q-block questions resolved.
- [ ] Streak predictions verified (DESIGN.md → 6,
  PRODUCT.md → 1, lib.rs → resolution per Task 2).
- [ ] Test count delta recorded.
- [ ] Prediction-vs-reality block filled.

## Open questions

**Q1 — Delta granularity.** What does one `PersonaDelta`
represent?

  - **(a)** **One field-edit per delta** — each operator-
    approved delta carries a single mutation: *"append this
    line to behavioral_preferences"*, *"replace
    communication_style with this"*, *"add this entry to
    learned_context"*. Fine-grained; operator approval
    decisions are specific.
  - **(b)** **One approved-batch per delta** — operator
    approves a bundle of changes as a single delta.
    Coarser; one approval gesture covers multiple changes.
    Less granular for revert (reverting one delta undoes
    the whole batch).
  - **(c)** **One Persona snapshot per delta** — each
    delta = entire effective Persona at approval time.
    Smallest mental model (no diff semantics) but largest
    storage cost and unclear revert semantics.

  **Recommendation: (a).** Fine-grained deltas make the
  operator's approval decision specific and the revert
  mechanism (Phase 60) targeted: "revert delta #47" is
  meaningful when one delta = one field-edit. The
  chain-walker code is the same regardless of
  granularity; storage cost is marginal.

**Q2 — Storage shape.** Where do `PersonaDelta` records
live?

  - **(a)** **New `KeyDomain::Persona`** with its own HMAC
    chain (parallel to the audit chain). Independent
    auditability surface; operator can verify Persona log
    integrity without walking the full audit chain. P14
    commit 2 commits to "an append-only HMAC-chained log
    parallel to the existing audit chain" — language
    literally pins (a).
  - **(b)** **Share the existing audit chain** — Persona
    deltas appear as a new `AuditEvent::PersonaDelta`
    variant in the existing chain. Smaller code surface
    (one chain to walk); couples Persona to audit chain
    size.
  - **(c)** **Plain-text file like Profile** (not HMAC-
    chained). Operator can inspect with cat. Violates P14
    commit 2 — not on the table; mentioned only to rule
    out.

  **Recommendation: (a).** P14's contract language pins
  this. Implementation cost is small (the chain primitive
  exists in `aivyx-audit`); the contract clarity is large.

**Q3 — Delta categories.** What kinds of mutations does a
`PersonaDelta` carry?

  - **(a)** **Mirror Profile's six categories** — every
    field of P13 (`assistant_name`, `operator_profile`,
    `communication_style`, `primary_use_cases`,
    `behavioral_preferences`, `behavioral_constraints`) is
    a candidate for Persona mutation. Persona refines what
    Profile declares.
  - **(b)** **Persona-specific categories** per the P14
    amendment commentary: `learned_context`,
    `communication_adaptations`, `character_traits`,
    `relationship_milestones`. Persona adds new identity
    facets that Profile doesn't carry.
  - **(c)** **Both** — Persona can refine Profile fields
    *and* add Persona-specific content. The unified set
    is six P13 categories + four P14 categories = ten.

  **Recommendation: (c).** Profile is the seed; Persona is
  the growth. Both kinds of growth are valuable —
  *refining* communication_style based on observed
  operator preferences AND *adding* learned_context
  entries about the operator's domain. The ten-category
  enum is the cleanest contract surface; the alternative
  (only b) leaves no way to learn that the operator
  prefers terser responses than Profile originally
  declared.

**Q4 — Proposal surface.** How does the agent propose a
Persona delta?

  - **(a)** **Extend the existing `reflection.propose`
    tool** with a new optional field
    `persona_deltas: Vec<ProposedPersonaDelta>`.
    Matches existing precedent (the same tool already
    proposes memory writes, prompt appends, and allowlist
    changes — adding persona is one more category).
  - **(b)** **New dedicated `persona.propose` tool** —
    one tool per proposal category. Keeps each tool's
    JSON schema simple but doubles the tool count for the
    reflection layer.
  - **(c)** **Both** — `reflection.propose` accepts
    persona deltas alongside other categories;
    `persona.propose` is an alias for the persona-only
    case. Maximum flexibility, maximum API surface.

  **Recommendation: (a).** Reflection is the unified
  proposal surface per P8. Adding a category keeps the
  API consistent and aligns the existing gate prompt
  pattern. (b) fragments the reflection surface for no
  upside; (c) is gratuitous duplication.

**Q5 — Effective-state hot-reload.** When the operator
approves a Persona delta, how soon does the next turn see
it?

  - **(a)** **Per-turn freshness via shared state** —
    daemon holds an `Arc<RwLock<EffectivePersona>>`;
    planner factory reads it per-turn; `reflection.apply`
    mutates it under the write lock. Matches Phase 30
    `role_overrides` precedent exactly. Effective on the
    very next turn.
  - **(b)** **Load-time only — restart required** —
    matches Profile (Q5(a) of Phase 58). Operator stops
    and restarts daemon to pick up newly-approved deltas.
    Simpler; misaligned with `role_overrides`.
  - **(c)** **Per-turn freshness via chain re-read** —
    planner walks the Persona chain per-turn (no shared
    state). Always fresh; expensive at chain length
    growth.

  **Recommendation: (a).** Matches the most relevant
  precedent (`role_overrides` from Phase 30) and honors
  P14 commit 3's "effective Persona at turn start"
  language. The shared-state plumbing is mechanical and
  well-understood. (b) would create the surprising UX
  where reflection.apply tells the operator "approved"
  but the next turn shows no effect. (c) is correctness-
  equivalent to (a) but trades latency for state
  simplicity — wrong trade.

**Q6 — Effective-identity assembly layout.** How does the
system prompt compose Profile + Persona + Active Role?

  - **(a)** **Three labeled sections in sequence**:
    ```
    ## About this assistant            <- Profile
    ...

    ## How I have learned to communicate    <- Persona
    ...

    ## Active role: <name>            <- Role envelope
    ...
    ```
    Clear separation. Operator reading the rendered
    prompt can tell what came from where. Matches Phase
    57 Q3(c) precedent.
  - **(b)** **Merge Profile + Persona into one block**
    with sub-sections distinguishing static vs learned.
    Less verbose but blurs the static/dynamic boundary.
  - **(c)** **Persona overwrites Profile fields** — the
    rendered output shows only the merged state, no
    history. Cleanest reading but loses the contract
    distinction.

  **Recommendation: (a).** The contract distinction
  between Profile (operator-declared static) and Persona
  (reflection-written dynamic) matters; the labeled
  layout preserves it. (b) and (c) lose information the
  contract pins.
