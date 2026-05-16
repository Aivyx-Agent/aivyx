# Phase 78 — Learning Observability & Trust Surface

Phases 75–77 made the assistant self-improving: it auto-recalls
memory, learns which recalls help, and on its own self-tunes
retention and files Persona proposals. But the operator's only
window into any of that is `aivyx recall-feedback: …` stderr
breadcrumbs. An autonomous system the operator can't see is an
autonomous system the operator can't trust. Phase 78 makes the
loop **legible**: a first-class, read-only view — in the CLI,
over IPC, and in the Web UI — of *what the assistant has learned
and why*, with each Pending Persona proposal traceable back to
the recalls and turn outcomes that motivated it.

## Why this, why now

- The loop is *closed* (Phase 77) but *opaque*. Legibility is
  the natural and highest-trust next move — and the honest one
  after building three phases of increasingly autonomous
  machinery.
- It pays down the Phase 76/77 Web-UI deferral thread instead
  of opening new behavioral surface.
- Everything it needs is already persisted: the Phase 77
  recall log + the proposal chain's `source_reflection_session_id`
  + the audit chain. No new behavior, no new storage — a pure
  read surface computed from what exists.

## Streak predictions

- **DESIGN.md** — **Will hold.** A read-only inspection query
  + CLI verb + Web UI pane is the established Phase 60/74
  inspection pattern; no locked technical-contract decision is
  touched. Prediction: streak **extends to twenty-five**
  consecutive phases (currently 24).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** Making the self-learning loop
  observable is a transparency/quality improvement to
  already-delivered G3/P8/P14 substrate — it changes nothing
  the agent *does*. No commitment-text edit. Prediction:
  streak **extends to eighteen** consecutive phases.
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold,
  by design.** The IPC `QueryPayload`/`QueryResponsePayload`
  enums live in `aivyx-channel`'s `daemon_ipc.rs` (not a
  streak file — Phase 74 added `SearchMemory` there). The
  digest is computed by reusing `reflection_scheduler::`
  `summarize_recent_outcomes_from_entries` + `recall_feedback::`
  `correlate` + the recall log + the proposal chain — every
  one of those already exists in `aivyx-channel`. Nothing
  needs a new `aivyx-core` type, and the signal deliberately
  reuses the existing `OutcomeSummary` rather than any
  `AuditTag` change (the Phase 76/77 streak discipline,
  continued). Prediction: streak **extends to twenty-six**
  consecutive phases (new project record, beats Phase 77's
  25).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

- **New workspace deps** — Zero. Pure read surface over the
  existing recall / proposal / audit substrate.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_78.md` + `docs/README.md` status row.

### Task 2 — Insight derivation (`recall_insights`)

`aivyx-channel`, pure + unit-tested:

- `LearningDigest { window_secs, recalls_scored, promoted,
  not_promoted, top_helpful: Vec<(topic, score)>,
  top_unhelpful: Vec<(topic, score)>, proposals_in_window }`
  — the per-window operational picture, derived from a
  `HelpfulnessTally` + the recall events + the proposal list.
- `ProposalProvenance { proposal_id, topic, net_score,
  contributing: Vec<ContributingTurn> }` where a
  `ContributingTurn` names the recall timestamp, the matched
  turn outcome, and the hits — reconstructed (Q4a) by matching
  a proposal's `source_reflection_session_id` + topic against
  the recall log. No schema/chain migration.
- Pure functions over already-tested inputs (`correlate` is
  Phase 77). Deterministic ordering for stable rendering.

### Task 3 — Inspection IPC query

`aivyx-channel/daemon_ipc.rs`:

- `QueryPayload::GetLearningInsights { window_secs: Option<u64> }`
  (`None` → a sane default window) +
  `QueryResponsePayload::LearningInsights { digest,
  proposals: Vec<ProposalProvenance> }`. serde-tagged,
  `#[serde(default)]` where round-trip back-compat needs it.
  Round-trip fixtures updated.

### Task 4 — Daemon query handler

`aivyx-channel/daemon_server.rs`:

- Handle `GetLearningInsights`: build `OutcomeSummary`s from
  the audit chain via the existing
  `summarize_recent_outcomes_from_entries`, read
  `recall_log.events_since`, `correlate`, derive the digest +
  per-proposal provenance (proposals from the existing
  proposal-chain handle). Returns an **empty** digest (not an
  error) when no recall log / no `[embedding]` is configured —
  the pre-Phase-78 "nothing learned yet" state is a valid
  answer.

### Task 5 — `daemon_client` + CLI

- `daemon_client::get_learning_insights(...)`.
- `aivyx learning [--window <secs>]` — renders the digest
  (counts + top helpful/unhelpful topics) and, per Pending
  proposal, its provenance. Parser + render unit tests.

### Task 6 — Web UI pane

- A "Learning" tab that dispatches `GetLearningInsights` and
  renders the digest + per-proposal provenance, read-only
  (approve/reject stays in the existing persona-proposals
  surface). HTML smoke assertion.

### Task 7 — Tests + docs + exit

- Tests: digest/provenance derivation (helpful/unhelpful
  ranking, proposal matching, empty/no-op), IPC round-trip,
  CLI parser + render, Web UI HTML smoke, handler integration
  over a real store.
- Docs: `docs/INSTALL.md` "Learning insights (Phase 78)"
  section (what the view shows, the recall-window horizon, why
  it's read-only); `examples/aivyx.toml` note pointer if
  warranted.
- Exit: ROADMAP + PRODUCT_ROADMAP frozen entries, docs/README
  status flip, prediction-vs-reality, hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Data model:** (a) Compute-on-query from the live
  recall log + correlator. Always consistent with what the
  loop actually does; zero new storage/streak surface; bounded
  to the recall-log retention horizon (longer history
  deferred).
- **Q2 — Channels:** (a) Full parity — IPC + CLI + Web UI.
  The headless CLI matters for the VPS-private daemon posture;
  the Web UI is where an operator watches an autonomous
  system. The established inspection pattern; less would
  undercut the phase.
- **Q3 — Granularity:** (b) Digest + proposal provenance.
  The two highest-trust-stakes views (is the loop healthy /
  why did it propose a Persona change). Per-entry drill-down
  deferred — honest scope after two consecutive
  over-ambitious test-count misses.
- **Q4 — Provenance linkage:** (a) Reconstruct on-query from
  the proposal's existing `source_reflection_session_id` +
  the recall log. No schema/HMAC-chain migration; consistent
  with the compute-on-query model.

## Deferrals

**Rolling deferrals carried into Phase 78:**

- v0.1.0 publication (Phase 61 Task 7).
- System-prompt notification-target enumeration (Phase 62).
- `turn_id` correlation refactor (Phase 67).
- Phase 68 deferrals (XOAUTH2, HTML email, attachments,
  multi-account `[email]`).
- Phase 69 deferrals (WebPush / service-worker, notification
  urgency, icons, sound).
- Phase 70 deferrals (proposal supersession, multi-window
  reflection).
- Phase 71 deferrals (per-fire role override, operator-
  customizable reflection prompt, reflection cadence
  learning).
- Phase 73 deferrals (persisted rate-limit buckets,
  operator-configurable retry-on list).
- Phase 74 deferrals (fuzzy match, edit-content Web UI,
  per-topic eviction-strategy override).
- Phase 75 deferrals (ANN index, `aivyx memory reembed`,
  hybrid keyword+semantic fusion, query-embedding cache).
- Phase 76 deferrals (conversational-window query, heuristic
  recall gate, token-budget context sizing).
- Phase 77 deferrals (`[recall_feedback]` tuning knob,
  LLM-judged recall usefulness, cross-session pattern
  learning).

**Likely Phase 78 deferrals:**

- **Per-memory-entry learning drill-down.** v1 ships the
  digest + proposal provenance (Q3b). A per-entry "this note:
  recalled N×, net score, promoted?" view defers until
  operators ask to drill that deep.
- **History beyond the recall-log window.** The view is
  bounded by the ~30-day recall-log retention (Q1a). A
  persisted long-horizon learning history defers.
- **Web UI live refresh.** v1's pane is fetch-on-open /
  manual refresh, matching the other inspection panes. A
  push/streaming "watch it learn" view defers.
- **Actionable insights.** v1 is read-only observability.
  Operator actions driven *from* the learning view (pin a
  memory, mute a topic) defer to a later phase.

## Prediction vs. reality

To be filled in at phase exit.

## Exit criteria

- [ ] `LearningDigest` + `ProposalProvenance` derivation,
  pure + tested — Task 2.
- [ ] `GetLearningInsights` IPC query + response +
  round-trip — Task 3.
- [ ] Daemon handler; empty (not error) when no recall
  substrate — Task 4.
- [ ] `daemon_client` + `aivyx learning` CLI + parser/render
  tests — Task 5.
- [ ] Web UI Learning pane + HTML smoke — Task 6.
- [ ] Tests across derivation, IPC, CLI, Web UI, handler
  integration — Task 7.
- [ ] `docs/INSTALL.md` (+ `examples/aivyx.toml` if
  warranted) updated — Task 7.
- [ ] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 7.
- [ ] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [ ] DESIGN.md streak extends to twenty-five.
- [ ] PRODUCT.md streak extends to eighteen.
- [ ] Production-core streak extends to twenty-six (new
  record) — `lib.rs` byte-identical.
- [ ] Test count delta: positive (~+18-30, calibrated per
  the Phase 77 note: inspection-on-existing-substrate phases
  trend lower).
- [ ] Zero clippy warnings.
- [ ] Zero new workspace deps.
- [ ] Prediction-vs-reality block filled.
