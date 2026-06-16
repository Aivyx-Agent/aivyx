# Phase 95 — Reflection Cadence Learning (Skip-When-Idle) (Phase 71's reflection-cadence deferral, closed)

Phase 71 introduced the persistent reflection cron with
`[[reflection_schedule]]` blocks that fire on operator-
configured cron intervals — but listed "reflection cadence
learning" in its deferrals from day one. Today the cron
fires unconditionally on its schedule. On idle days (long
weekends, vacation, slow project phases) it pays the LLM
cost for Phase 87 phrasing + Phase 91 judgment + Phase 92
supersession passes that find nothing actionable. That cost
compounds across the four reflection passes + every
operator-configured schedule.

Phase 95 closes the cadence-learning deferral with the
simplest leverage shape: **skip-when-idle**. The
reflection-scheduler reads the audit-chain growth since the
last *fired* cycle for that schedule; if growth is below
the operator-configured `min_audit_entries_to_fire`
threshold AND `skip_when_idle = true`, the scheduler skips
the cycle entirely (no LLM calls, no proposal filing, just
a log line + a counter bump). The operator's cron remains
the **upper bound** on firing rate — cadence learning is
monotonic-slower-only, never faster. With both knobs at
their defaults, behaviour is byte-identical to pre-Phase-95.

## Why this, why now

- Phase 71 named "reflection cadence learning" in its
  deferrals list; the deferral has been carried 24 phases.
- The reflection-cron arc is now end-to-end built: Phase
  71 (cron), Phase 80 (proactive pass), Phase 81 (persona
  lifecycle pass), Phase 87 (consolidation pass), Phase
  91 (LLM judgment pass), Phase 92 (supersession), Phase
  93 (judgment-driven feedback). Idleness on a fixed cron
  is now a concrete cost — the operator pays LLM tokens
  for four passes per cycle. Skip-when-idle directly
  shrinks that cost.
- Surface area is tiny. Two new optional fields on the
  existing `[[reflection_schedule]]` block; a pure
  helper deciding fire-vs-skip; one integration point
  inside `run_reflection_scheduler` between the cron
  trigger and the per-pass dispatch.
- The change is **purely additive**. With
  `skip_when_idle = false` (the default), the scheduler
  fires every cron tick exactly as pre-Phase-95. With
  `true`, the operator's cron is still the upper bound;
  skip events are observable via daemon log + the
  `aivyx learning` surface.
- Reuse is total. The audit log substrate already
  exposes `len()` and per-entry queries; the
  `ReflectionScheduleConfig` shape already accepts new
  optional fields via the established `Option<T>` +
  `#[serde(default)]` pattern (Phase 71 / 84 / 91 / 92).

## Streak predictions

- **DESIGN.md** — **Will hold.** Skip-when-idle is a
  rate-limiting decision over an existing trigger; it
  touches no locked technical-contract decision. The
  audit chain is read-only here; the reflection cron's
  per-pass contracts are unchanged. Hash at entry:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Prediction: streak **extends to forty-two** (currently
  41).

- **PRODUCT.md** — **Will hold.** P15 (the self-learning
  loop) is delivered; this is a cost-and-noise refinement
  on its cron. No commitment changed; the operator-
  facing contract is *strengthened* (the loop no longer
  burns LLM tokens on idle days when the operator opts
  in). Hash at entry:
  `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.
  Prediction: streak **extends to thirty-five** (currently
  34).

- **Production-core `aivyx-core/src/lib.rs`** — **Will
  hold, by design.** The helper lives in
  `aivyx-channel::reflection_scheduler`; the config
  fields live in `aivyx-config`. No `aivyx-core` touch;
  no new `AuditTag`; no new `KeyDomain`. Hash at entry:
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.
  Prediction: streak **extends to forty-three**
  consecutive phases (new project record, beats Phase
  94's 42).

- **New workspace deps** — Zero. Pure arithmetic over
  existing audit-log accessors.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_95.md` + `docs/README.md` status row.

### Task 2 — `skip_when_idle` + `min_audit_entries_to_fire` on `[[reflection_schedule]]`

`aivyx-config`:

- `ReflectionScheduleConfig` gains two optional fields:
  - `skip_when_idle: bool` (default `false`). Master
    switch per schedule. With `false`, the cycle always
    fires (pre-Phase-95 behaviour).
  - `min_audit_entries_to_fire: u32` (default `1`). The
    threshold the audit-chain growth must clear to fire.
    `1` means "any new audit entry triggers the cycle";
    higher values raise the bar (`100` = "wait until
    there's a hundred new entries since last fire").
- `RawReflectionSchedule` + the build path. Validation:
  `min_audit_entries_to_fire >= 1` when
  `skip_when_idle = true` (defended; zero would skip
  every cycle including unconditionally-active ones).
- Tests: defaults; explicit values win; staged-config
  posture (knob set but `skip_when_idle = false`) honored
  without validation.

### Task 3 — `should_fire_cycle` pure helper + state tracking (the crux)

`aivyx-channel/src/reflection_scheduler.rs`:

- New `pub fn should_fire_cycle(audit_growth: u64,
  min_to_fire: u32, skip_when_idle: bool) -> bool`. Pure
  arithmetic; trivially testable. Returns `false` only
  when `skip_when_idle && audit_growth < min_to_fire`.
- New `LastFiredCursor` in the scheduler's per-schedule
  state: tracks `last_fired_audit_len: Option<u64>` per
  schedule. Initialized to `None` (first fire after
  daemon boot is unconditional — no prior baseline to
  compare against). Updated to `Some(current_len)` after
  every actual fire (regardless of which passes ran).
- New `RecentReflectionStat { fired: u32, skipped: u32 }`
  shared per-schedule (rolling — accumulates across the
  daemon lifetime). Wire-compat via `#[serde(default)]`
  on each field for IPC round-trip.
- Unit tests on the pure helper: skip-when-idle off →
  always fires; skip-when-idle on + growth below
  threshold → skips; skip-when-idle on + growth at
  threshold (boundary) → fires; skip-when-idle on +
  growth above threshold → fires; skip-when-idle on +
  threshold zero → defended-default fires (the config
  validation makes zero impossible, but the helper
  defends anyway).

### Task 4 — Reflection-scheduler integration

`aivyx-channel/src/reflection_scheduler.rs`:

- Inside `run_reflection_scheduler`'s per-schedule
  dispatch loop, BEFORE running the per-pass dispatchers
  (audit pass + proactive pass + persona-lifecycle pass
  + consolidation pass + judgment pass + recall-feedback
  pass), check `should_fire_cycle`. If false, log
  `aivyx reflection: schedule "X" — skipped (audit-
  growth K below threshold M)` and increment the
  `skipped` counter on the schedule's `RecentReflectionStat`;
  return without running any pass. If true, run all
  passes as before, then update `last_fired_audit_len`
  AND increment `fired`.
- The audit-growth computation reads `audit_log.len()`
  AT cycle entry (cheap — the audit log's len is a
  cached `u64`). The skip path costs essentially zero;
  the fire path pays a single subtraction.
- Integration test: build a fixture daemon with one
  `[[reflection_schedule]]` configured with
  `skip_when_idle = true, min_audit_entries_to_fire = 5`.
  Audit log starts empty. Trigger cycle 1 → fires
  unconditionally (no baseline). Append 3 audit entries;
  trigger cycle 2 → skips (growth 3 < threshold 5);
  `fired = 1, skipped = 1` on the stat. Append 5 more
  audit entries (total growth since last fire: 8);
  trigger cycle 3 → fires; `fired = 2, skipped = 1`.

### Task 5 — Surface + docs + exit

- `aivyx learning` render block: extend with a new
  `cadence: K fired, S skipped (last cycle: <fired|skipped>)`
  line per schedule. When `RecentReflectionStat` is
  absent / both counts are zero, the line is omitted
  (operators not opted into Phase 95 see no surface
  change).
- `docs/INSTALL.md` — new "Reflection cadence learning
  (Phase 95)" subsection under the existing Phase 71
  reflection-scheduler section: the audit-growth signal,
  the per-schedule opt-in, the operator's cron as upper
  bound on firing rate.
- `examples/aivyx.toml` — document the two new
  `[[reflection_schedule]]` keys with an example showing
  a high-volume schedule (low threshold) vs. an idle-
  friendly schedule (high threshold).
- Exit: ROADMAP + PRODUCT_ROADMAP frozen entries,
  docs/README status flip, prediction-vs-reality, hash
  backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Signal:** (a) Audit-chain growth since the last
  *fired* cycle for that schedule. Simple, deterministic,
  available everywhere, naturally captures "real work
  happened." No new substrate; no LLM cost; no
  behavioral guessing.
- **Q2 — Shape:** (a) Skip-when-idle (binary). Either
  fire or skip; the operator's cron is the upper bound
  on firing rate. Cadence learning is monotonic-slower-
  only — no surprises where the system fires more often
  than configured. One threshold knob; simplest
  semantics.
- **Q3 — Knob:** (a) Per-schedule `skip_when_idle: bool`
  + `min_audit_entries_to_fire: u32` on the existing
  `[[reflection_schedule]]` block. Wire-compat via the
  established `Option<T>` + `#[serde(default)]` pattern.
  Different schedules may have different idleness
  tolerances (a daily housekeeping schedule wants a
  high threshold; an hourly responsive schedule wants
  a low one).
- **Q4 — Surface:** (a) Daemon log on skip
  (`aivyx reflection: schedule "X" — skipped (audit-
  growth K below threshold M)`) + `aivyx learning`
  surface stat
  (`cadence: K fired, S skipped (last cycle: ...)`).
  Real-time visibility (log) + aggregate (stat); symmetric
  with the existing per-pass observability the
  reflection cron's other passes provide.

## Deferrals

**Rolling deferrals carried into Phase 95** (Phase 71's
reflection-cadence-learning deferral is **THIS PHASE**):

- v0.1.0 publication (Phase 61 Task 7).
- System-prompt notification-target enumeration (Phase 62).
- `turn_id` correlation refactor (Phase 67).
- Phase 68 deferrals (XOAUTH2, HTML email, attachments,
  multi-account `[email]`).
- Phase 69 deferrals (WebPush / service-worker, notification
  urgency, icons, sound).
- Phase 70 deferrals (multi-window reflection).
- Phase 71 deferrals (per-fire role override, operator-
  customizable reflection prompt, **reflection cadence
  learning — THIS PHASE**).
- Phase 73 deferrals (persisted rate-limit buckets,
  operator-configurable retry-on list).
- Phase 74 deferrals (fuzzy match, edit-content Web UI,
  per-topic eviction-strategy override).
- Phase 75 deferrals (ANN index, `aivyx memory reembed`,
  hybrid keyword+semantic fusion, query-embedding cache).
- Phase 76 deferrals (token-budget context sizing).
- Phase 77 deferrals — closed by Phase 93.
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
- Phase 84 deferrals (affinity re-ranking, operator-
  tunable affinity policy).
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
  gate, adaptive thresholds).
- Phase 91 deferrals (per-recall LLM critique, adaptive
  batch size, multi-model ensembling, response-text
  recovery).
- Phase 92 deferrals (atomic chain-level supersession
  primitive, n-ary cluster supersession, semantic-
  similarity supersession; Web UI grouping closed by
  Phase 94).
- Phase 93 deferrals (per-domain/per-topic verdict-mapping
  weights, replace mode, asymmetric Hurt penalty, sum
  mode, on-disk buffer).
- Phase 94 deferrals (atomic transaction IPC for "Approve
  both", backend-side grouping enrichment, drag UI
  affordances, n-ary group rendering).

**Likely Phase 95 deferrals:**

- **Backoff multiplier mode.** Q2b's geometric backoff
  defers — skip-when-idle achieves the same end (less
  LLM cost when idle) with simpler semantics and one
  knob instead of two.
- **Adaptive interval / time-of-day learning.** Q2c +
  Q2d defer indefinitely — ambitious prediction is hard
  to validate and the operator's cron is already a
  pretty good prior.
- **Persisted skip stat across daemon restarts.** The
  `RecentReflectionStat` lives in-memory only. Survives
  reads but not restarts. A future phase could persist
  it to the audit chain (or a dedicated `KeyDomain`)
  if operators want long-running cadence visibility.
- **LLM-based signal-density classifier.** Q1d defers —
  self-referential, expensive, and not obviously better
  than audit-growth at the threshold scales operators
  care about.
- **Per-pass skip granularity.** v1 skips the whole
  cycle; future work could let individual passes (e.g.,
  Phase 91 judgment) skip independently based on
  per-pass signals (e.g., un-judged recall count). v1
  whole-cycle granularity is the simplest leverage shape.

## Prediction vs. reality

**Predictions held — all three streaks correct.**

- **DESIGN.md — held.** Skip-when-idle is a rate-limiting
  decision over an existing trigger; touched no locked
  technical-contract decision. The audit chain is read-
  only here; the reflection cron's per-pass contracts
  are unchanged. Streak: **42 consecutive phases** (was
  41).
- **PRODUCT.md — held.** P15 (the self-learning loop) is
  delivered; this is a cost-and-noise refinement on its
  cron. No commitment changed; the operator-facing
  contract is *strengthened* (operators with idle days
  no longer pay LLM tokens for empty passes when they opt
  in). Streak: **35 consecutive phases** (was 34).
- **`aivyx-core/src/lib.rs` — held, by design.** Helper +
  state + integration all in `aivyx-channel/src/
  reflection_scheduler.rs`; config knobs in
  `aivyx-config`; surface in
  `bin/aivyx_modules/learning.rs`; IPC field on
  `QueryResponsePayload::LearningInsights` (in
  `daemon_ipc.rs`). No `aivyx-core` touch; no new
  `AuditTag`; no new `KeyDomain`. Streak: **43
  consecutive phases** — new project record, beating
  Phase 94's 42.

**Test count — `+12`** (workspace `1675 → 1687`). **Two
over** the predicted `+6-10` band. Breakdown:

- Config knobs `+3` (defaults pinned, explicit values
  win, staged-config posture honored, zero-threshold
  rejection).
- Pure helper + stat `+7` (knob-off always-fires
  regression; knob-on below / at / above threshold;
  defended threshold-zero; `RecentReflectionStat`
  default-zero; serde wire-compat round-trip + decode-
  from-empty).
- Integration `+1` (multi-cycle fire/skip/fire/skip
  scenario over the `decide_cadence_action` +
  `mark_cycle_fired` helpers — exercises first-cycle-
  unconditional, boundary `>=` rule, and cursor-not-
  updated-on-skip invariant in one fixture).
- Surface render `+1` (three-state cadence block: empty
  / all-zero-omitted / mixed-with-only-active-rendered).

The `+2 over` is accounted for by the helper + stat
earning more individual boundary tests than the
calibration law's "new pure module (≈ +4-5)" anticipated.
The stat's serde wire-compat shape earned its own test
because the codebase's IPC-additive-fields pattern now
explicitly pins decode-from-empty as a wire-compat
contract (the test fails if `#[serde(default)]` is
accidentally removed). Within the project's pattern for
"new pure module with multiple boundary cases" this is
on-track with Phase 94's `+11` (helper + edge cases).

**Scope — every planned surface shipped exactly as scoped.**
Two new optional fields on the existing
`[[reflection_schedule]]` block with full wire-compat;
the pure `should_fire_cycle` helper; the
`RecentReflectionStat` + `SharedRecentReflectionStats`
shape; the `decide_cadence_action` /
`mark_cycle_fired` helpers refactored out of the loop
body for testability; the `run_reflection_scheduler`
loop integration that consults the gate BEFORE per-pass
dispatch; the daemon-server IPC plumbing that surfaces
the per-schedule stats via `GetLearningInsights`; the
`aivyx learning` surface block. Zero clippy warnings
after one `large_enum_variant` allow added to
`QueryResponsePayload` (the new optional field tipped a
long-running additive-fields variant past clippy's
threshold; boxing each Option<Stat> would churn serde
wire formats for marginal benefit). Zero new workspace
deps.

**Boundary semantics noted explicitly.** The helper uses
`audit_growth >= min_to_fire` (not strictly greater
than). An operator setting `min_audit_entries_to_fire =
1` gets "any new audit entry fires the cycle" — not
"any entry beyond the first." This is the intuitive
reading; pinned in the
`should_fire_skip_on_at_threshold_fires` test for future
readers.

**Cursor invariant.** `last_fired_audit_len` is updated
ONLY on actual fires — skip cycles preserve the old
baseline. This is the key invariant that makes a long
run of skips eventually accumulate enough growth to
fire. Pinned in the multi-cycle integration test (cycle
2 skips, cycle 3 then sees growth 8 since the cursor
that was last set in cycle 1).

## Exit criteria

- [x] `[[reflection_schedule]].skip_when_idle: bool`
  (default `false`) + `min_audit_entries_to_fire: u32`
  (default `1`, bounded `>= 1` when `skip_when_idle =
  true`) — Task 2 (commit `54fb77e`).
- [x] `should_fire_cycle` pure helper +
  `RecentReflectionStat` + `SharedRecentReflectionStats`
  — Task 3 (commit `ce63091`).
- [x] Unit tests on the pure helper: knob-off always
  fires; knob-on with growth below / at / above
  threshold; defended threshold-zero; stat-default-zero;
  serde wire-compat — Task 3 (commit `ce63091`).
- [x] `run_reflection_scheduler` calls the helper BEFORE
  per-pass dispatch via `decide_cadence_action`; updates
  `last_fired_audit_len` on every actual fire via
  `mark_cycle_fired` — Task 4 (commit `7bcef6c`).
- [x] `RecentReflectionStat { fired, skipped }` IPC
  wire-compat via `#[serde(default)]` — Task 3 +
  surfaced through `GetLearningInsights` in Task 5.
- [x] Integration test: multi-cycle fire/skip/fire/skip
  scenario over the helpers — Task 4 (commit
  `7bcef6c`).
- [x] `aivyx learning` surface extended with cadence
  block — Task 5 (this commit).
- [x] `docs/INSTALL.md` + `examples/aivyx.toml` updated —
  Task 5 (this commit).
- [x] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 5 (this commit).
- [x] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [x] DESIGN.md streak extends to forty-two.
- [x] PRODUCT.md streak extends to thirty-five.
- [x] Production-core streak extends to forty-three (new
  record) — `lib.rs` byte-identical.
- [x] Test count delta: positive (`+12`, two over the
  predicted `+6-10` band — accounted for by the helper
  + stat earning more individual boundary tests than
  the calibration law anticipated, including a serde
  wire-compat test that pins the additive-fields
  contract).
- [x] Zero clippy warnings (one `large_enum_variant`
  allow added to `QueryResponsePayload` since the
  addition tipped a long-running additive-fields
  variant past clippy's threshold).
- [x] Zero new workspace deps.
- [x] Prediction-vs-reality block filled.
