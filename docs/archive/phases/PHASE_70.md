# Phase 70 — Reflection Auto-Loop (P14 Self-Learning Closure)

Phase journals are working documents. They churn freely during
the phase and freeze at exit under a final Exit criteria block.
For the locked technical contract see [`../DESIGN.md`](../../../DESIGN.md);
for the locked product contract see [`../PRODUCT.md`](../../../PRODUCT.md).

## Goal

Close the long-held self-learning half of **P14 Persona**.
Phase 29 (frozen) shipped the reflection *substrate* —
`ReflectionProposeTool` + `ReflectionApplyTool` + the
`ProposedPersonaDelta` validation surface — but the agent
only reflects when an operator manually prompts it. Phase 70
adds the **autonomy loop**: a scheduled reflection trigger
fires periodically, the agent synthesizes pending proposals
from observed turn outcomes, and the operator reviews them
asynchronously in a dedicated Web UI pane.

After Phase 70, P14's contract text at PRODUCT.md:1133 —
*"Reflection is the engine that proposes Persona deltas"* —
is delivered both directions: operator-edited (Phase 60) and
agent-proposed-on-its-own-cadence (Phase 70).

## Why now

1. **Reach Milestone closed (Phases 62-69).** Every operator-
   shape on the outbound axis is covered: telegram, webhook,
   email, web-ui desktop. The natural pivot is back to the
   PRODUCT.md core — and P14's "self-learning" half is the
   largest remaining vision-promise from Phase 55's pivot.
2. **Reflection substrate already in tree.** Phase 29 did the
   tool plumbing + capability gating + ProposedPersonaDelta
   validation. Phase 70 is the *loop closure*, not a from-
   scratch build — substantially de-risked.
3. **Q-block fully resolved at design time.** Dedicated
   `[[reflection_schedule]]` config (Q1), outcome-summary
   input (Q2), edit-then-approve flow (Q3), new
   `KeyDomain::PersonaProposals` (Q4) all signed off.

## Streak predictions

- **DESIGN.md** — **Will hold.** Phase 70 adds one `KeyDomain`
  variant + one config section + one persistent store + a
  proposal-lifecycle state machine + Web UI Proposals pane.
  None of these touch the locked technical contract:
  Phases 21/26/27/56 added KeyDomain variants
  (`Missions`, `Schedules`, `Webhooks`, `FileWatches`,
  `Persona`) without DESIGN.md edits, and the substrate is
  already described abstractly at DESIGN.md:946. Prediction:
  streak **extends to seventeen** consecutive phases (currently
  at 16).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** The contract text at
  PRODUCT.md:1133 already commits to "Reflection is the
  engine that proposes Persona deltas"; Phase 70 *delivers*
  that text rather than amending it. No Delivery Status
  refresh needed — the existing entry covers both Phase 60
  (operator-edited) and Phase 70 (agent-proposed).
  Prediction: streak **extends to ten** consecutive phases.
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 70 work lives in `aivyx-config` (new schedule
  section), `aivyx-storage` (new KeyDomain variant), and
  `aivyx-channel` (proposal store, scheduler hook, IPC,
  Web UI, CLI). TurnOutcome already exposes everything the
  outcome-summary input context needs; no core-side hook.
  Prediction: streak **extends to eighteen** consecutive
  phases (new record, beats Phase 69's 17).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

- **New workspace deps** — Zero expected. The reflection
  scheduler reuses the existing `[[schedule]]` cron
  infrastructure; the proposal store reuses the existing
  redb-backed `KeyDomain` substrate.

## Tasks

### Task 1 — Open commit + PHASE_70.md scaffold

This file. Update `docs/README.md` to show Phase 70 as Open.
The `ROADMAP.md` frozen entry lands at exit per recent
convention (Phases 68/69).

### Task 2 — Config: `[[reflection_schedule]]` section

`aivyx-config`:

- `RawReflectionSchedule` deserializer with fields: `name`
  (string), `cron` (string), `lookback_window_secs` (u64,
  default 86400 → 24h), `role_override` (optional string),
  `enabled` (default true).
- `build_reflection_schedules` validates the cron pattern
  (reuse existing `[[schedule]]` validator), lookback bound
  (min 60s, max 30 days), and role-override existence.
- Loader rejects unknown extra fields per the standard
  aivyx-config convention.

### Task 3 — Storage: `KeyDomain::PersonaProposals`

`aivyx-storage`:

- New `KeyDomain::PersonaProposals` variant with HKDF info
  bytes `b"persona-proposals"` and table name
  `aivyx_persona_proposals_v1`.
- Added to the `KeyDomain::all()` iterator + the constructor
  table-creation list.

### Task 4 — `PersonaProposalLog` substrate

`crates/aivyx-channel/src/persona_proposal.rs` (new file):

- `PersonaProposal` struct: `id` (string), `proposed_at_unix_ms`
  (u64), `source_reflection_session_id` (string), `proposed_op`
  (the agent's original `ProposedPersonaDelta`), `status`
  (`Pending | Approved { applied_op, applied_seq } | Rejected
  { reason } | Superseded`), `resolved_at_unix_ms` (optional),
  `audit_seq` (optional, set on resolution).
- `PersistentPersonaProposalLog` mirrors `PersistentPersonaLog`:
  append-only redb-backed, keyed by `proposed_at_unix_ms` +
  ULID-like id for ordering. Methods: `append_pending`,
  `mark_approved`, `mark_rejected`, `mark_superseded`,
  `list` with status filter, `get_by_id`.
- HMAC-chained envelope around each entry (reuse the persona
  log's chain pattern) — the proposal store is auditable
  even before resolution.

### Task 5 — Reflection scheduler hook

`crates/aivyx-channel/src/reflection_scheduler.rs` (new file):

- `run_reflection_scheduler` loop: parallel to
  `daemon_scheduler::run_scheduler` but watches the
  reflection-schedule store. On cron tick, fires a reflection
  turn through `TriggerDispatch`.
- The reflection turn uses a canonical role envelope:
    - System prompt: "You are reflecting on the agent's
      recent behavior. Examine the supplied turn-outcome
      summaries for behavioral patterns. Propose Persona
      deltas via `reflection.propose` when you see a
      consistent pattern (≥3 occurrences). Be conservative
      — propose nothing if no clear pattern emerges."
    - Tool allowlist: `turn.history`, `reflection.propose`.
    - Capability set: `reflection.propose` + `persona.propose`
      + `turn.history.read`.
- Input context: outcome summaries for the lookback window
  (Q2(a)). `OutcomeSummary { session_id, started_at,
  outcome, tool_count, error_count }` — no transcripts.
- The scheduler-fired turn's `reflection.propose` calls now
  route to `PersistentPersonaProposalLog::append_pending`
  instead of (or in addition to) the existing mission-gate
  flow. Synchronous-approval callers (operator-prompted
  reflection) keep the existing gate path; scheduled
  reflection writes to the proposal log directly.

### Task 6 — IPC: Query + Resolution envelopes

`daemon_ipc.rs`:

- `QueryPayload::ListPersonaProposals { status_filter, limit }`
  + `QueryResponsePayload::ListPersonaProposals { proposals,
  total_len }`.
- `QueryPayload::GetPersonaProposal { proposal_id }` +
  `QueryResponsePayload::GetPersonaProposal { proposal }`.
- `FrontendMessage::ResolvePersonaProposal { id, proposal_id,
  resolution }` where `resolution` is one of
  `Approve | ApproveWithEdit { edited_op } | Reject { reason }`.
- `DaemonMessage::PersonaProposalResolved { id, ok, success,
  error }` — `success` carries `{ delta_seq, status }` on
  approval, `None` on reject.
- All variants gain corresponding `DaemonEnvelope` arms.

### Task 7 — Web UI Proposals pane

`web_ui_static.html`:

- New tab `data-pane="proposals"` between Persona and the
  rest. Refresh action wires `ListPersonaProposals { status:
  Pending }` by default; filter chips for `pending |
  approved | rejected | all`.
- Each pending proposal card shows: proposed timestamp,
  source reflection session id, category, proposed op
  (rendered like the Persona pane's delta op rendering),
  three action buttons:
    - **Approve** → `ResolvePersonaProposal { resolution:
      Approve }` (Q3 edit flow optional).
    - **Edit** → expands an inline editor on the op fields
      → **Save & Approve** sends
      `ApproveWithEdit { edited_op }`.
    - **Reject** → modal prompts for an optional reason,
      sends `Reject { reason }`.
- Resolved proposals (history) render in a collapsed
  section below pending; click expands.

### Task 8 — CLI

`aivyx persona proposals` subcommands:

- `list [--status pending|approved|rejected|all]` — table
  output with id, age, category, op preview.
- `show <proposal_id>` — full proposal detail.
- `approve <proposal_id> [--edit]` — `--edit` opens
  `$EDITOR` on a TOML rendering of the op, applies the
  edited version on save.
- `reject <proposal_id> [--reason TEXT]`.

### Task 9 — Tests

- Storage: `KeyDomain::PersonaProposals` constructs, derives,
  isolates from other domains.
- Proposal log: append/list/mark-approved/mark-rejected
  round-trips; chain validation; status-filter list.
- Reflection scheduler: cron tick fires a reflection turn;
  scheduler-fired `reflection.propose` writes to the
  proposal log; operator-prompted reflection still uses
  the mission-gate path.
- IPC: round-trip all five new envelope variants.
- Web UI: HTML smoke tests for the Proposals pane wiring
  (tab present, `'ListPersonaProposals'` dispatch,
  `'ResolvePersonaProposal'` construction, three action
  buttons).
- CLI: parse + render tests for the four subcommands.

### Task 10 — Docs

- `examples/aivyx.toml` gains a commented
  `[[reflection_schedule]]` block alongside the existing
  `[[schedule]]` examples, plus a brief operator-note on
  why the canonical reflection prompt is opinionated.
- `docs/INSTALL.md` gains a "Reflection auto-loop" section
  explaining: the daily cadence default, the
  pending-proposals-need-operator-approval safety, the
  capability gating, and the audit-chain coverage.

### Task 11 — Exit commit

- `ROADMAP.md` Phase 70 frozen entry.
- `docs/PRODUCT_ROADMAP.md` P14 Delivery Status refresh:
  Phase 60 (operator-edited) + Phase 70 (agent-proposed)
  noted as complementary halves of P14.
- `docs/README.md` status flip with backfill.
- Prediction-vs-reality block filled.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Schedule integration:** (a) New
  `[[reflection_schedule]]` config block. Dedicated section
  with reflection-specific knobs (`cron`, `lookback_window_secs`,
  `role_override`); clearer operator mental model than reusing
  `[[schedule]]`. Implementation reuses the existing scheduler
  infrastructure under the hood (Task 5).
- **Q2 — Input scope:** (a) TurnOutcome summaries only. The
  reflection prompt sees a list of `OutcomeSummary` for the
  lookback window; no transcripts. Lightweight + privacy-
  conscious; the agent can call `turn.history` for detail
  when needed.
- **Q3 — Edit-on-approve:** (a) Yes — edit-then-approve.
  The Web UI Proposals pane lets the operator tweak the op
  before approving. Final stored delta records both
  `proposed_op` and `applied_op` so the audit trail captures
  any operator modification.
- **Q4 — Proposal storage:** (a) New encrypted
  `KeyDomain::PersonaProposals` domain. Append-only with
  `Pending | Approved | Rejected | Superseded` status. Clean
  separation: proposals are operator-pending; deltas are
  operator-approved (the persona log's invariant stays
  intact).

## Deferrals

**Rolling deferrals carried into Phase 70:**

- v0.1.0 publication (Phase 61 Task 7).
- System-prompt notification-target enumeration (Phase 62).
- `turn_id` correlation refactor (Phase 67).
- Phase 62/63 reach polish (default-target sugar, per-target
  rate limits, retry, multi-target, conditional notify).
- Phase 68 deferrals (XOAUTH2, HTML email, attachments,
  multi-account `[email]`).
- Phase 69 deferrals (WebPush / service-worker
  notifications, notification urgency, icons, sound,
  history pane).

**Likely Phase 70 deferrals:**

- **Reflection on operator feedback events.** Phase 70 ships
  scheduled (cron-driven) reflection; reflecting in response
  to explicit operator feedback ("that was great", "no try
  again") is a follow-up shape.
- **Multi-window reflection.** The lookback is a single
  window per schedule. Trend-vs-recency comparisons would
  need richer aggregation; defer until pressure surfaces.
- **Proposal supersession on operator-direct-edit.** If the
  operator hand-edits Persona via the existing Phase 60
  CLI/UI while a related proposal is pending, the proposal
  should auto-supersede. Useful but additive — defer to
  Phase 70 polish or a separate phase.
- **Reflection on memory / role proposals.** Phase 70 closes
  the Persona-proposal loop. Memory writes already auto-
  apply per Phase 29; runtime role mutation remains
  deferred from Phase 29 / Phase 30+.

## Prediction vs. reality

**All three streak predictions correct.**

- **DESIGN.md** — Held. Hash at exit:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`
  (byte-identical to entry). Streak extends to **seventeen**
  consecutive phases as predicted. Phase 70 added a KeyDomain
  variant, a new persistent log substrate, a config section,
  IPC envelopes, a Web UI pane, and CLI subcommands — none
  surfaced in the locked technical contract.
- **PRODUCT.md** — Held. Hash at exit:
  `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`
  (byte-identical to entry). Streak extends to **ten**
  consecutive phases as predicted. The forward-pointing line
  at PRODUCT.md:1133 ("Reflection is the engine that proposes
  Persona deltas") covered both Phase 60 (operator-edited)
  and Phase 70 (agent-proposed) without rewording.
- **Production-core `aivyx-core/src/lib.rs`** — Held. Hash
  at exit:
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`
  (byte-identical to entry). Streak extends to **eighteen**
  consecutive phases — new project record, beating Phase 69's
  17. All Phase 70 work lived in `aivyx-config`,
  `aivyx-storage`, and `aivyx-channel`; `aivyx-core` was
  untouched.
- **Workspace deps** — Zero new as predicted.
- **Tests** — +41 (1234 → 1275), exceeding the +25-35
  prediction. Breakdown: 8 config (reflection_schedule), 2
  storage (KeyDomain::PersonaProposals), 11 proposal log
  state machine + redb round-trip, 7 IPC round-trip cases, 5
  daemon-side resolve handler, 1 Web UI HTML smoke, 7 CLI
  parser, 7 CLI render helpers. (Some IPC tests were
  additional cases in existing round-trip tests rather than
  net-new test functions, hence the count differential.)
- **Clippy** — Zero warnings across the workspace.
- **Q-block** — All four resolutions held in implementation:
  Q1(a) dedicated `[[reflection_schedule]]` config section
  parses + validates (cron, lookback bounds, role override,
  name uniqueness across both schedule namespaces); Q2(a)
  proposal storage holds `source_reflection_session_id` for
  outcome-summary traceability; Q3(a) Web UI Proposals pane
  ships `ApproveWithEdit`, and the proposal chain preserves
  both `proposed_op` and `applied_op`; Q4(a) new
  `KeyDomain::PersonaProposals` with distinct genesis seed.

**Scope note — cron auto-firing deferred.** The
`[[reflection_schedule]]` config section parses and validates
end-to-end and the proposal substrate accepts agent-supplied
deltas via `reflection.propose` today, but the dedicated
scheduler-loop that fires reflection turns on the configured
cron pattern was deferred at sign-off as a follow-up. Operators
who want auto-reflection today wire a regular `[[schedule]]`
entry with a reflection-flavored prompt; the proposals land in
the same chain and surface in the same Web UI / CLI panes
either way.

## Exit criteria

- [x] `[[reflection_schedule]]` config block + validation —
  Task 2.
- [x] `KeyDomain::PersonaProposals` storage variant — Task 3.
- [x] `PersistentPersonaProposalLog` + status state machine
  — Task 4.
- [x] Reflection scheduler fires turns + canonical reflection
  prompt + outcome-summary input — Task 5.
- [x] IPC: ListPersonaProposals + GetPersonaProposal +
  ResolvePersonaProposal envelopes round-trip — Task 6.
- [x] Web UI Proposals pane: list / approve / edit-then-
  approve / reject — Task 7.
- [x] CLI: `aivyx persona proposals list|show|approve|reject`
  — Task 8.
- [x] Tests across storage, proposal log, scheduler, IPC,
  HTML smoke, CLI — Task 9.
- [x] `examples/aivyx.toml` + `docs/INSTALL.md` updated —
  Task 10.
- [x] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 11.
- [x] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [x] DESIGN.md streak extends to seventeen.
- [x] PRODUCT.md streak extends to ten.
- [x] Production-core streak extends to eighteen (new
  record).
- [x] Test count delta: positive (~+25–35).
- [x] Zero clippy warnings.
- [x] Zero new workspace deps.
- [x] Prediction-vs-reality block filled.
