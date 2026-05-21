# Phase 94 — Web UI Grouping for Linked Supersession Proposals (Phase 92's first deferral, closed)

Phase 92 closed the longest-running Persona-actuator deferral
with shared-endpoint supersession: when an applied
`consolidate-pair:` facet's pair decays AND a new pair
sharing one endpoint strengthens, the consolidation pass
files two linked proposals — a `RemoveList` half + an
`AppendList` half — carrying cross-referenced
`supersedes_proposal_id` on `ProposedPersonaDelta`. v1
surfaced the linkage **textually only**, via the
`reason` field on each half (`supersedes proposal
consolidate-pair:auth+jwt` on one side, `superseded by
proposal consolidate-pair:auth+sessions` on the other).
The structured field is on the wire but the surface
doesn't visually group the pair.

Phase 94 closes Phase 92's first deferral: the CLI
(`aivyx persona proposals`) and the Web UI Persona-pane
Proposals list both render linked pairs as **one grouped
unit** with a single primary "approve both" action and a
split menu for the partial cases. The cross-link is
visible at a glance; the common case is one click; the
operator who wants to approve only one half (Phase 92's
explicit `each half independently Revert-able` guarantee)
gets there through the `⋮` menu.

## Why this, why now

- Phase 92 named this exact follow-up first in its
  "likely deferrals" list: *"Web UI visual grouping of
  linked supersession proposals."* The structured
  `supersedes_proposal_id` field already exists on the
  wire; only the surface needs to consume it.
- Surface area is tiny. One pure helper in `aivyx-channel`
  that consumes a flat `Vec<PersonaProposal>` and emits a
  grouped representation; the CLI + Web UI renderers
  both call it. No IPC contract change, no daemon-side
  enrichment, no new chain primitives.
- The change is **purely additive**. The existing flat
  list IPC is preserved; the grouping is a client-side
  rendering decision derived from data already on the
  wire. Operators on older CLI / Web UI versions see the
  flat list with `reason`-text linkage exactly as
  Phase 92 shipped. No protocol bump.
- Reuse is total. Phase 60 created the Web UI Persona
  pane; Phase 70 added the Proposals subsection;
  multiple phases (84, 87, 88, 91, 92) iterated on it.
  Phase 94 just adds a grouping layer on top.

## Streak predictions

- **DESIGN.md** — **Will hold.** Client-side surface
  grouping over a structured field that's already on the
  wire touches no locked technical-contract decision. No
  chain-shape change; no IPC contract change; no new
  `KeyDomain`. Hash at entry:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Prediction: streak **extends to forty-one** (currently
  40).

- **PRODUCT.md** — **Will hold.** P14 (Persona) is
  delivered; this is a UX refinement on its Phase 70
  proposal pipeline + Phase 92 supersession surface.
  No commitment changed; the operator-facing contract is
  *strengthened* (a confusing two-decision flow becomes
  one ergonomic decision). Hash at entry:
  `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.
  Prediction: streak **extends to thirty-four**
  (currently 33).

- **Production-core `aivyx-core/src/lib.rs`** — **Will
  hold, by design.** The grouping helper lives in
  `aivyx-channel`; the CLI render lives in the
  `aivyx_modules` binary tree; the Web UI render lives
  under `web_ui/` (the static asset + JS). No
  `aivyx-core` touch; no new `AuditTag`. Hash at entry:
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.
  Prediction: streak **extends to forty-two** consecutive
  phases (new project record, beats Phase 93's 41).

- **New workspace deps** — Zero. The grouping helper is
  pure `std`.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_94.md` + `docs/README.md` status row.

### Task 2 — `group_supersession_pairs` pure helper (the crux)

`aivyx-channel`:

- New `proposal_grouping` module exposing
  `pub fn group_supersession_pairs(proposals:
  &[PersonaProposal]) -> Vec<ProposalRendering>` where
  `ProposalRendering` is the enum:
  ```rust
  pub enum ProposalRendering<'a> {
      Linked { remove_side: &'a PersonaProposal,
               append_side: &'a PersonaProposal },
      Unlinked(&'a PersonaProposal),
  }
  ```
- Algorithm (Q1a — pure structural):
  1. Build a `HashMap<proposal_id, &PersonaProposal>` from
     the input list.
  2. For each proposal, follow its `supersedes_proposal_id`
     to the partner; if the partner exists AND points
     back at this proposal, emit a single `Linked { ... }`
     entry for the pair (deterministic ordering:
     RemoveList side first by category inspection).
  3. Each proposal appears in the output exactly once
     (the second time we encounter the partner of a
     paired proposal, skip — it was already emitted).
  4. Dangling references (an `AppendList` whose
     `supersedes_proposal_id` points at a proposal that
     was rejected/removed and is no longer in the list)
     degrade gracefully to `Unlinked`.
  5. Stable ordering: pairs emit at the position of the
     first half encountered in the input order; the
     RemoveList side renders before the AppendList side
     within a pair.
- Unit tests on the pure helper (Q4a — single mixed
  fixture + dedicated edge-case tests):
  - Mixed fixture: two linked-pair proposals + one
    orphan-link proposal + one unlinked
    recall-fb proposal + one unlinked consolidate-pair
    proposal → exactly one `Linked`, three `Unlinked`
    (the orphan degrades).
  - Self-reference: a proposal pointing at itself →
    treated as `Unlinked` (defensive).
  - Asymmetric link (A points at B, B doesn't point
    back) → both `Unlinked` (defensive; only mutual
    references group).
  - Determinism: shuffled input → same output up to the
    input-order stability rule.
  - Empty list → empty output.

### Task 3 — CLI rendering (`aivyx persona proposals`)

`aivyx-channel/src/bin/aivyx_modules/persona.rs` (or the
equivalent proposals renderer):

- Call the new helper before rendering the list.
- For each `ProposalRendering::Linked { remove_side,
  append_side }`, render two rows with a `└─ supersedes:
  <other_id>` indicator under each half, visually
  grouped via consistent indentation. Example:
  ```
  ▶ consolidate-pair:auth+sessions  [pending]  AppendList
    └─ supersedes: supersede-remove:consolidate-pair:auth+jwt
  ▶ supersede-remove:consolidate-pair:auth+jwt  [pending]  RemoveList
    └─ superseded by: consolidate-pair:auth+sessions
  ```
- For `Unlinked` rendering stays exactly as
  pre-Phase-94 (byte-identical regression).
- +2-3 tests in the existing `persona` CLI module:
  linked-pair rendering shows the indicator; orphan
  degrades to flat; unlinked is unchanged.

### Task 4 — Web UI Persona-pane rendering

`crates/aivyx-channel/src/web_ui/` (static JS / template):

- Web UI ProposalsList consumes the same grouping —
  either via a small JS helper that mirrors the
  Rust helper's logic, OR by piping the grouped result
  through the existing `GetLearningInsights` /
  `ListPersonaProposals` IPC payload (the simpler path:
  the WebSocket payload already carries the
  `supersedes_proposal_id` field — the JS just runs the
  same grouping algorithm client-side).
- Linked pairs render as a single visual card with:
  - The two halves stacked, connected by a visible
    "↔ supersedes" arrow.
  - One primary "Approve both" button that fires two
    sequential `ResolvePersonaProposal` IPC calls
    (RemoveList first, then AppendList).
  - A `⋮` menu with: "Approve RemoveList only", "Approve
    AppendList only", "Reject both", "Reject one"
    options.
  - The split actions invoke single
    `ResolvePersonaProposal` calls and respect Phase 92's
    `each half independently Revert-able` guarantee.
- Orphan / Unlinked cards render exactly as
  pre-Phase-94 (regression).
- v1 Q2a posture: if the first call in an "approve
  both" succeeds and the second fails, the operator is
  left in a half-approved state — the UI surfaces the
  error and the operator finishes via the next refresh.
  No client-side rollback. The Phase 70 chain's
  individual-proposal-state primitive is what's being
  composed; no new chain primitive is added.

### Task 5 — Surface tests + docs + exit

- `docs/INSTALL.md` — under the existing Phase 92
  "Pattern-driven supersession" subsection, add a brief
  note that Phase 94 makes the linkage operator-visible
  via grouped rendering in both CLI and Web UI; mention
  the `⋮` menu for partial actions.
- `examples/aivyx.toml` — no new keys (Phase 94 is a
  pure surface change). A comment cross-reference under
  the Phase 92 supersession block points at Phase 94 as
  the grouping-rendering layer.
- Exit: ROADMAP + PRODUCT_ROADMAP frozen entries,
  docs/README status flip, prediction-vs-reality, hash
  backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Detection:** (a) Pure structural pass on the
  existing proposal list. The grouping helper consumes
  the flat `Vec<PersonaProposal>` already returned by
  the IPC and produces `Vec<ProposalRendering>`. No IPC
  contract change; no daemon-side enrichment; single
  source of truth for the grouping algorithm. Honors
  Phase 92's "chain stays unchanged" commitment by
  keeping the change strictly above the wire.
- **Q2 — Approval:** (a) One-click approves both halves
  via two sequential `ResolvePersonaProposal` IPC calls;
  the `⋮` menu offers approve-just-one / reject-both /
  reject-one for the partial cases the operator may need.
  Honors Phase 92's explicit `each half independently
  Revert-able` guarantee.
- **Q3 — Surface:** (a) Web UI Persona pane AND the
  `aivyx persona proposals` CLI both grouped. Maintains
  the project's CLI/Web UI parity discipline; both
  clients call the same pure helper.
- **Q4 — Test:** (a) Single mixed-fixture test on the
  grouping helper covering linked pairs, orphan links,
  unlinked proposals across two `proposal_id` patterns,
  plus dedicated unit tests for the defensive edge cases
  (self-reference, asymmetric link, empty list,
  determinism). Matches the Phase 91/92/93 integration-
  test shape.

## Deferrals

**Rolling deferrals carried into Phase 94:**

- v0.1.0 publication (Phase 61 Task 7).
- System-prompt notification-target enumeration (Phase 62).
- `turn_id` correlation refactor (Phase 67).
- Phase 68 deferrals (XOAUTH2, HTML email, attachments,
  multi-account `[email]`).
- Phase 69 deferrals (WebPush / service-worker, notification
  urgency, icons, sound).
- Phase 70 deferrals (multi-window reflection).
- Phase 71 deferrals (per-fire role override, operator-
  customizable reflection prompt, reflection cadence
  learning).
- Phase 73 deferrals (persisted rate-limit buckets,
  operator-configurable retry-on list).
- Phase 74 deferrals (fuzzy match, edit-content Web UI,
  per-topic eviction-strategy override).
- Phase 75 deferrals (ANN index, `aivyx memory reembed`,
  hybrid keyword+semantic fusion, query-embedding cache).
- Phase 76 deferrals (token-budget context sizing).
- Phase 77 deferrals (the `[recall_feedback]` block was
  introduced in Phase 93; further per-knob tuning
  defers).
- Phase 78 deferrals (per-memory-entry drill-down, Web UI
  live refresh, actionable insights).
- Phase 79 deferrals (`[persona]` tuning block, behavioural
  Persona).
- Phase 80 deferrals (standalone `[[proactive_schedule]]`,
  LLM-composed proactive prose, conversational/interactive
  proactive, additional proactive signal classes).
- Phase 81 deferrals (contradiction-based supersession,
  standalone `[[persona_lifecycle_schedule]]`, facet-scoped
  one-click revert).
- Phase 82 deferrals (operator-tunable half-life/retention).
- Phase 83 deferrals (sequential/temporal patterns, n-ary
  clusters, operator-tunable top-K/half-life).
- Phase 84 deferrals (affinity re-ranking of existing
  candidates, operator-tunable affinity policy).
- Phase 85 deferrals (reflection-facet decay via fuzzy
  embedding).
- Phase 86 deferrals (token-budget context sizing,
  embed-each-and-pool windows, persisted windows).
- Phase 87 deferrals (n-ary cluster proposals, operator-
  tunable LLM prompt).
- Phase 88 deferrals (n-ary cluster decay, pair-affinity
  hysteresis).
- Phase 89 deferrals (operator-tunable `[[topic_alias]]`
  mappings, topic-by-topic exception list, one-time
  migration of existing fragmented data, non-ASCII /
  Unicode stemming).
- Phase 90 deferrals (pattern-based stoplist, LLM-judged
  gate, adaptive thresholds, token-budget context sizing).
- Phase 91 deferrals (per-recall LLM critique, adaptive
  batch size, multi-model ensembling, response-text
  recovery via audit-chain extension).
- Phase 92 deferrals (**Web UI visual grouping — THIS
  PHASE**, atomic chain-level supersession primitive,
  n-ary cluster supersession, semantic-similarity
  supersession).
- Phase 93 deferrals (on-disk buffer survival, persisted
  threshold writes — n/a since the scope shifted;
  per-domain/per-topic verdict-mapping weights, replace
  mode, asymmetric Hurt penalty, sum mode, on-disk
  judgment buffer).

**Likely Phase 94 deferrals:**

- **Web UI atomic transaction.** v1 fires two sequential
  IPC calls for "Approve both" with no client-side
  rollback if the second fails. A future phase could
  add an atomic `ResolveSupersessionGroup` IPC method
  that the daemon executes transactionally. v1's
  half-approved-state-on-failure is acceptable because
  Phase 92 guarantees individual `Revert`-ability.
- **Backend-side grouping enrichment.** Q1b's
  `linked_with: Vec<String>` on `ProposalSummary`
  defers — useful for hypothetical non-grouping clients
  but unnecessary while CLI + Web UI are the only
  consumers.
- **Drag-to-merge / drag-to-split UI affordances.**
  Operators arranging linked pairs visually beyond the
  default grouping. Speculative; gated on operator
  feedback.
- **N-ary group rendering.** When Phase 83's n-ary
  cluster supersession deferral closes (still pending),
  the grouping helper will need to handle 3+-way
  supersession groups. v1 only handles 2-ary pairs.

## Prediction vs. reality

To be filled in at phase exit.

## Exit criteria

- [ ] `group_supersession_pairs` pure helper in
  `aivyx-channel` returns
  `Vec<ProposalRendering<'_>>` from a flat
  `&[PersonaProposal]` — Task 2.
- [ ] Unit tests on the pure helper: linked-pair fixture;
  orphan-link degradation; unlinked passthrough; self-
  reference defended; asymmetric-link defended; empty
  list — Task 2.
- [ ] `aivyx persona proposals` CLI renders linked pairs
  with a `└─ supersedes:` indicator; unlinked rows
  unchanged — Task 3.
- [ ] CLI tests: linked-pair rendering; orphan
  degradation; unlinked regression — Task 3.
- [ ] Web UI Persona-pane Proposals renders linked
  pairs as a single visual card with one primary
  "Approve both" + `⋮` split menu — Task 4.
- [ ] One-click approve fires two sequential
  `ResolvePersonaProposal` IPC calls; partial actions
  use the existing single-call path — Task 4.
- [ ] `docs/INSTALL.md` cross-references Phase 94 under
  the existing Phase 92 subsection — Task 5.
- [ ] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 5.
- [ ] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [ ] DESIGN.md streak extends to forty-one.
- [ ] PRODUCT.md streak extends to thirty-four.
- [ ] Production-core streak extends to forty-two (new
  record) — `lib.rs` byte-identical.
- [ ] Test count delta: positive (~+6-10; per the
  converged calibration law — new pure helper module
  (≈ +5-7, mixed fixture + edge cases) + CLI rendering
  (≈ +2-3) + Web UI surface manually tested or +1-2
  rust-side integration; no new module schema, no new
  `KeyDomain`).
- [ ] Zero clippy warnings.
- [ ] Zero new workspace deps.
- [ ] Prediction-vs-reality block filled.
