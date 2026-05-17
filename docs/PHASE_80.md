# Phase 80 — Proactive Surfacing (the assistant brings things to you)

For 79 phases the assistant has only ever acted when prompted
— a turn, a cron job, a webhook. The 75–79 arc built
everything needed to change that safely: relevance ranking
(75–76), a signal that knows what actually *helps* (77),
legibility and operator trust (78), an adaptive identity (79).
Phase 80 is the capstone: on its existing cadence the
assistant notices a concrete, high-confidence reason to reach
out and **proactively surfaces it** — "you noted X 29 days
ago, it expires tomorrow"; "your `deploy/` notes keep helping,
here's the cluster." This is the single biggest step from "a
sophisticated tool you query" to "a personal assistant that
brings things to you," and it ships **off by default,
hard-capped, and fully explainable** — because an unprompted
*outbound* message is the highest-trust-stakes thing the
assistant can do, and the only honest way to earn it is a
conservative, no-LLM gate plus the noise guard already proven
in Phase 73.

## Why this, why now

- Every precondition exists and is proven: the reflection
  cadence (70/71, also hosting the 77 recall-feedback pass),
  the `NotifyDispatcher` + per-target rate-limit (62/73), the
  Phase 78 trust surface, the Phase 77 structural signal.
- The risk that deferred this at 78 and 79 — unprompted noise
  — is retired structurally: a no-LLM gate that must point to
  a concrete reason, a hard volume cap on top of Phase 73's
  per-target limit, opt-in only, and every send recorded with
  provenance.
- Reuse is near-total: a proactive surfacing is an auto-notify
  with a specific shape. No new scheduler, no LLM call, no
  agent turn.

## Streak predictions

- **DESIGN.md** — **Will hold.** Proactive surfacing reuses
  the reflection cadence + notify dispatcher + structural
  signal; no locked technical-contract decision is touched.
  Prediction: streak **extends to twenty-seven** consecutive
  phases (currently 26).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** A new assistant-initiated
  surface is a quality/vision deepening of already-delivered
  G3 (memory) / P8 (reflection) substrate, delivered
  conservatively (opt-in, capped). It introduces no new
  product *commitment* and weakens none. No commitment-text
  edit. Prediction: streak **extends to twenty** consecutive
  phases.
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold,
  by design.** The proactive pass is a structural,
  no-LLM/no-turn composition dispatched through the existing
  `NotifyDispatcher`, so it naturally produces the **existing**
  `AutoNotifyDispatched` audit event — no new `AuditTag`
  variant (the Phase 76/77 streak lesson, continued). The
  detector, dispatch, config, storage variant, and
  observability all live in `aivyx-channel` / `aivyx-config` /
  `aivyx-storage`. Nothing needs an `aivyx-core` type change.
  Prediction: streak **extends to twenty-eight** consecutive
  phases (new project record, beats Phase 79's 27).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

- **New workspace deps** — Zero. Reuses the 70/71/77 cadence,
  the 62/73 notify + rate-limit, the 77 signal, and the 78
  surface.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_80.md` + `docs/README.md` status row.

### Task 2 — `[proactive]` config section

`aivyx-config` (mirrors the Phase 75 `[embedding]` pattern):

- `ProactiveConfig { enabled: bool /* default false */,
  target: String /* notify target name */, max_per_window:
  u32, window_secs: u64, signals: ProactiveSignals }` where
  `ProactiveSignals` toggles the structural classes
  (`ttl_expiry`, `recall_cluster`, `due_reminder`).
- Absent section → `None` → proactive **off** (pre-Phase-80
  behaviour, the common case). Validation: when enabled,
  `target` non-empty, `max_per_window >= 1`, `window_secs
  >= 1`, at least one signal class on. `AivyxConfig.proactive:
  Option<ProactiveConfig>`. Tests for defaults, validation,
  off-when-absent.

### Task 3 — `KeyDomain::ProactiveLog` + `PersistentProactiveLog`

`aivyx-storage` (the full Phase 75/77 KeyDomain checklist) +
`aivyx-channel`:

- Encrypted, HKDF-isolated `ProactiveLog` domain (variant +
  `as_bytes` + `table_name` + `ALL`/`subkeys` + `subkey_for` +
  tripwire + isolation test).
- `PersistentProactiveLog`: records the deterministic id of
  every surfaced item + its ts; `was_surfaced(id)` for
  cross-cycle dedup (the assistant must never re-surface the
  same thing) + `gc_older_than` clamp on the same cadence.

### Task 4 — Structural proactive detector

`aivyx-channel`, pure + heavily unit-tested (the crux):

- Input: the memory entries (+ the `[proactive]` signal
  toggles + global `memory_ttl_secs`), the Phase 77
  recall-feedback tally, and `now`.
- Output: zero or more `ProactiveItem { id, kind, topic,
  summary, reason }` where each carries a concrete,
  human-readable `reason` (provenance). Deterministic id from
  `(kind, topic, boundary)` so dedup is stable across cycles.
- Signal classes (no LLM, conservative): `TtlExpiry` (an
  entry within a small window of TTL eviction),
  `RecallCluster` (a topic whose Phase-77 net helpfulness is
  strongly positive over a high threshold), `DueReminder`
  (reminder-shaped memory whose due time has arrived). Each
  toggled by config; thorough per-class + empty tests.

### Task 5 — Proactive pass (piggyback the reflection cron)

`aivyx-channel`, mirroring the Phase 77 `RecallFeedbackDeps`
wiring:

- The reflection scheduler's existing on-cron pass also: runs
  the detector, drops any item `was_surfaced`, applies the
  hard `max_per_window` cap (beyond Phase 73's per-target
  limit), dispatches each surviving item through the existing
  `NotifyDispatcher` to `[proactive] target`, and records it
  in `PersistentProactiveLog`. No `[proactive]` / disabled /
  no schedule → the pass is a complete no-op (pre-Phase-80
  behaviour). Deps threaded via `DaemonConfig`
  (`RecallFeedbackDeps` precedent).

### Task 6 — Observability (Phase 78-consistent)

- Per-cycle stderr breadcrumb: `aivyx proactive: surfaced N
  (M suppressed: capped/deduped/disabled)`.
- Extend the Phase 78 `GetLearningInsights` / `LearningInsights`
  with a proactive summary (last surfaced items + their
  `reason` provenance), rendered in the `aivyx learning` CLI
  and the Web UI Learning pane. Notify history already records
  the dispatch itself (`AutoNotifyDispatched`). HTML smoke +
  round-trip updated.

### Task 7 — Tests + docs + exit

- Tests: config, storage isolation + log dedup/GC, detector
  per signal class + empty, cap/dedup/dispatch pass
  (integration over a real store + a fake notify backend),
  observability, IPC round-trip, CLI/Web UI render.
- Docs: `docs/INSTALL.md` "Proactive surfacing (Phase 80)"
  (opt-in, the structural gate, the hard cap, where it's
  recorded, how to turn it off); `examples/aivyx.toml`
  `[proactive]` block with the off-by-default note.
- Exit: ROADMAP + PRODUCT_ROADMAP frozen entries, docs/README
  status flip, prediction-vs-reality, hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Cadence:** (a) Piggyback the reflection cron pass.
  Zero new scheduler; the operator who wants proactive is the
  one running reflection. A standalone `[[proactive_schedule]]`
  is a deferral.
- **Q2 — Relevance gate:** (a) Structural, no extra LLM. The
  assistant interrupts only when it can point to a concrete
  reason (TTL boundary / strong recall cluster / due
  reminder) — the Phase 77 no-self-judgement ethos applied to
  the highest-stakes action.
- **Q3 — Delivery:** (a) Reuse the `NotifyDispatcher`;
  Phase 73's per-target rate-limit is the reused hard spam
  cap, and every send lands in the notify history.
- **Q4 — Control + legibility:** (a) Opt-in (off by default)
  + a hard `max_per_window` cap on top of Phase 73's limit +
  full provenance recorded in the notify history and the
  Phase 78 learning surface. Default-off, capped, explainable.

## Deferrals

**Rolling deferrals carried into Phase 80:**

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
- Phase 78 deferrals (per-memory-entry drill-down, history
  beyond the recall-log window, Web UI live refresh,
  actionable insights).
- Phase 79 deferrals (`[persona]` tuning block, Persona
  consolidation/supersession/decay, behavioural Persona,
  conversational-window selection).

**Likely Phase 80 deferrals:**

- **Standalone `[[proactive_schedule]]`.** v1 piggybacks the
  reflection cadence; an independent timer defers.
- **LLM-composed proactive prose.** v1 surfaces a structural,
  templated message. An LLM rephrasing pass (gated, still
  no-self-judgement on *whether* to surface) defers.
- **Conversational / interactive proactive.** v1 is one-way
  notify. A proactive item the operator can reply to inline
  is a later arc.
- **Additional signal classes** (calendar-shaped memory,
  contradiction detection, etc.) beyond the initial three.

## Prediction vs. reality

To be filled in at phase exit.

## Exit criteria

- [ ] `[proactive]` config + validation + off-when-absent —
  Task 2.
- [ ] `KeyDomain::ProactiveLog` + isolation test +
  `PersistentProactiveLog` (dedup + GC) — Task 3.
- [ ] Structural detector, pure, per-class + empty tested —
  Task 4.
- [ ] Proactive pass piggybacked on reflection; no-op when
  off/absent; hard cap + cross-cycle dedup + dispatch —
  Task 5.
- [ ] Breadcrumb + Phase 78 surface extended (CLI + Web UI) —
  Task 6.
- [ ] Tests across config, storage, detector, pass
  integration, observability, IPC, CLI/Web UI — Task 7.
- [ ] `docs/INSTALL.md` + `examples/aivyx.toml` updated —
  Task 7.
- [ ] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 7.
- [ ] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [ ] DESIGN.md streak extends to twenty-seven.
- [ ] PRODUCT.md streak extends to twenty.
- [ ] Production-core streak extends to twenty-eight (new
  record) — `lib.rs` byte-identical.
- [ ] Test count delta: positive (~+14-22; per the converged
  calibration band, top-of-band — detector + new KeyDomain +
  config carry real new surface like Phase 77's +22).
- [ ] Zero clippy warnings.
- [ ] Zero new workspace deps.
- [ ] Prediction-vs-reality block filled.
