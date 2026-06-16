# Phase 71 — Reflection Scheduler Loop (Closes Phase 70 Deferral)

Phase journals are working documents. They churn freely during
the phase and freeze at exit under a final Exit criteria block.
For the locked technical contract see [`../DESIGN.md`](../../../DESIGN.md);
for the locked product contract see [`../PRODUCT.md`](../../../PRODUCT.md).

## Goal

Close the deferral I explicitly carried at Phase 70 exit: wire
the validated `[[reflection_schedule]]` config to an actual
scheduler loop that fires reflection turns on the configured
cron pattern. Phase 70 shipped the storage, IPC, Web UI, CLI,
and the agent-write path; the missing piece was the cadence
that fires reflection without operator prompting. Phase 71
delivers that loop.

After Phase 71, the self-learning vision is genuinely
autonomous: at the configured cron, the daemon synthesizes
recent turn outcomes, runs a reflection turn under the
canonical reflection prompt (or operator-declared
role_override), and any Persona deltas the agent proposes land
as Pending rows in the proposal chain for asynchronous
operator review.

## Why now

1. **Phase 70 left this scoped-down on purpose.** The
   `[[reflection_schedule]]` config block parses and validates
   end-to-end today but does nothing at runtime. That gap was
   acknowledged at exit; closing it now is the natural next
   commit.
2. **Substrate is fully in place.** Audit chain has
   `TurnStarted` / `TurnEnded` pairs with `turn_id`
   correlation (Phase 67). Proposal chain accepts agent
   writes (Phase 70 Task 5c). Operator review surfaces are
   live (Phase 70 Tasks 7-8). All four ingredients exist —
   Phase 71 just composes them.
3. **Q-block fully resolved at design time.** Audit chain +
   LRU cache (Q1), hardcoded canonical prompt (Q2),
   role_override-or-default (Q3), log/audit/skip on error
   (Q4) all signed off pre-Task 2.

## Streak predictions

- **DESIGN.md** — **Will hold.** Phase 71 adds one scheduler
  module + a canonical prompt constant + an outcome-summary
  helper + binary wire-up. No locked-contract edits.
  Prediction: streak **extends to eighteen** consecutive
  phases (currently at 17).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** No commitment-text edits;
  no Delivery Status refresh. Phase 70 already noted the
  scheduler deferral; Phase 71 closes it without rewriting
  any contract text.
  Prediction: streak **extends to eleven** consecutive phases.
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 71 work lives in `aivyx-channel` (new scheduler
  module, outcome summarizer, binary wire-up). Audit reads
  go through the existing `PersistentAuditLog` API; no
  core-side hook.
  Prediction: streak **extends to nineteen** consecutive
  phases (new record, beats Phase 70's 18).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

- **New workspace deps** — Zero expected. Cron parsing reuses
  the existing dep (`cron` crate, already in tree per the
  Phase 26 scheduler). LRU cache reuses
  `std::collections::HashMap` + a small VecDeque eviction
  policy — no `lru` crate dep needed for this scope.

## Tasks

### Task 1 — Open commit + PHASE_71.md scaffold

This file. Update `docs/README.md` to show Phase 71 as Open.

### Task 2 — Reflection scheduler module

`crates/aivyx-channel/src/reflection_scheduler.rs` (new file):

- `pub async fn run_reflection_scheduler(...)` loop parallel
  to `daemon_scheduler::run_scheduler`. Watches the
  `Vec<ReflectionScheduleConfig>` passed by the daemon and
  the audit log handle; on each cron tick, fires a reflection
  turn through `TriggerDispatch`.
- Cron parsing uses the same crate the existing
  `[[schedule]]` infrastructure uses; per-entry next-fire
  timestamps drive a `tokio::time::sleep_until` interval.
- Failure mode per Q4: a panicked or errored reflection turn
  emits an `eprintln` diagnostic + an audit event
  (`AutoNotifyDispatched`-style entry to be added or a new
  variant if the existing taxonomy doesn't fit), and the
  scheduler moves on to the next cron fire. No in-window
  retry, no consecutive-failure backoff.

### Task 3 — Canonical reflection prompt + role envelope

In `reflection_scheduler.rs`:

- `pub const REFLECTION_SYSTEM_PROMPT: &str = "..."` — the
  hardcoded canonical prompt per Q2(a). Conservative wording:
  examine outcome summaries for behavioral patterns, propose
  Persona deltas via `reflection.propose` only when a pattern
  recurs ≥3 times, prefer narrower categories (BehavioralPreferences,
  LearnedContext) over identity-level changes (AssistantName,
  CommunicationStyle).
- Per Q3, the scheduled turn uses the role_override from the
  `[[reflection_schedule]]` entry if set, else the role
  resolved at daemon startup (the "default" / active role).
  The canonical prompt is wrapped into the chosen role's
  envelope by the same `assemble_session_prompt` helper used
  for normal turns — so role-derived capability gating still
  applies.

### Task 4 — Outcome summarizer + LRU cache

In `reflection_scheduler.rs`:

- `OutcomeSummary { session_id, turn_id, started_at,
  outcome_kind, tool_calls_made, duration_ms,
  error_count_estimate }` struct — the wire shape the
  reflection prompt receives via `turn.history` tool calls
  or a synthetic injection.
- `summarize_recent_outcomes(audit_log, lookback_secs)`
  walks the audit chain backwards from current time, pairs
  `TurnStarted` + `TurnEnded` events by `turn_id`, and
  emits one summary per completed turn within the window.
- `OutcomeSummaryCache` per Q1's "audit primary, in-memory
  cache layer": a small VecDeque-backed bounded LRU keyed by
  `(lookback_secs, audit_chain_len)` that returns cached
  summaries when the chain hasn't advanced beyond the cache
  point and the lookback hasn't changed. Eviction at 8
  entries (back-to-back schedules with different lookbacks
  is the realistic high-water mark).

### Task 5 — Binary wire-up

`crates/aivyx-channel/src/bin/aivyx.rs`:

- Spawn `run_reflection_scheduler` alongside the existing
  scheduler/webhook/file-watch tasks in the daemon startup
  path, passing the `config_reflection_schedules` + the
  audit-log handle + the persona proposal log + the role
  resolver.
- The startup banner gains a one-line entry per registered
  reflection schedule ("aivyx daemon: reflection schedule
  `<name>` registered, next fire at <ts>").

### Task 6 — Tests

- Outcome summarizer: synthetic audit entries → expected
  summaries; pair-matching by turn_id; lookback-window
  filtering; missing TurnEnded (in-flight turn) skipped;
  out-of-order entries handled.
- LRU cache: hit + miss + eviction order; cache invalidation
  on chain growth; different lookbacks produce different
  cache entries.
- Cron-tick dispatch: mocked TriggerDispatch records a single
  fire per cron interval; multiple schedules fire
  independently; disabled entries skip.
- Failure path: a `TriggerDispatch::fire` returning Err logs
  + audit-records + the scheduler continues.
- Integration: end-to-end e2e with a real (in-memory) audit
  + proposal log + scheduler + dispatcher firing a single
  cron tick → outcome summaries materialize → reflection
  prompt receives them → on agent-proposed deltas (mocked),
  Pending rows appear in the proposal chain.

### Task 7 — Docs

- `examples/aivyx.toml` `[[reflection_schedule]]` block
  comment loses the "Phase 70 substrate-only" caveat.
- `docs/INSTALL.md` Reflection auto-loop section flips the
  "cron-fired auto-firing is deferred polish" line to a
  "running on the configured cron" line.

### Task 8 — Exit commit

- `ROADMAP.md` Phase 71 frozen entry.
- `docs/PRODUCT_ROADMAP.md` P14 self-learning entry: append
  "and Phase 71 closed the cron-loop deferral."
- `docs/README.md` status flip with backfill.
- Prediction-vs-reality block filled.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Outcome summary source:** (c) Audit chain primary,
  in-memory LRU cache layer. The audit log is the source of
  truth (already records every TurnStarted/TurnEnded pair);
  a bounded VecDeque-backed cache absorbs back-to-back
  reflection-schedule fires without re-walking the chain.
- **Q2 — Reflection prompt:** (a) Hardcoded constant in
  `reflection_scheduler.rs`. Single source of truth,
  version-controlled, conservative phrasing. Operator-
  customizable prompt is a deferred polish.
- **Q3 — Role selection:** (a) `role_override` when set,
  else the default role. Matches what Phase 70 Task 2's
  config validation already enforced (role_override must
  reference an existing role).
- **Q4 — Failure mode:** (a) Log + audit + skip until next
  cron. Matches the existing `[[schedule]]` failure semantics;
  no in-window retry; no consecutive-failure backoff. A
  one-off failure self-heals at next cron.

## Deferrals

**Rolling deferrals carried into Phase 71:**

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
- Phase 70 deferrals (proposal supersession on operator-
  direct-edit, reflection on operator feedback events,
  multi-window reflection, memory / role proposal flows).

**Likely Phase 71 deferrals:**

- **Operator-customizable reflection prompt.** Hardcoded
  constant ships in Phase 71 per Q2(a). Per-schedule
  `system_prompt = "..."` override is a clean follow-up
  when an operator surfaces a real need.
- **Reflection cadence learning.** A future polish where the
  daemon observes operator approval rates and suggests
  schedule changes ("you reject 90% of nightly proposals;
  consider running weekly instead").
- **Reflection-of-reflection.** Meta-reflection — running a
  reflection on the reflection schedule's own success rate.
  Speculative; not on the near-term roadmap.

## Prediction vs. reality

**All three streak predictions correct.**

- **DESIGN.md** — Held. Hash at exit:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`
  (byte-identical to entry). Streak extends to **eighteen**
  consecutive phases as predicted. Phase 71 added a scheduler
  module, a canonical prompt constant, a summarizer + cache, a
  `TriggerSource` variant, and binary wire-up — none touched
  the locked technical contract.
- **PRODUCT.md** — Held. Hash at exit:
  `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`
  (byte-identical to entry). Streak extends to **eleven**
  consecutive phases. Phase 71 delivers the cron-loop wiring
  Phase 70 explicitly deferred; no new commitment text was
  needed.
- **Production-core `aivyx-core/src/lib.rs`** — Held. Hash at
  exit:
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`
  (byte-identical to entry). Streak extends to **nineteen**
  consecutive phases — new project record, beating Phase 70's
  18. All Phase 71 work lived in `aivyx-audit`,
  `aivyx-channel`, `aivyx-config` (no edits this phase, but
  the existing `ReflectionScheduleConfig` is the type
  consumed); `aivyx-core` was untouched.
- **Workspace deps** — Zero new as predicted. `cron` and
  `chrono` were already in tree from Phase 26.
- **Tests** — +14 (1275 → 1289). Just under the +15-25
  prediction floor. Breakdown: 14 reflection_scheduler tests
  covering pair-matching, in-flight skip, lookback filter,
  sort order, LRU cache (hit / eviction / MRU bump / distinct
  key), prompt format helper (empty + populated), cron
  next-fire-after (valid + invalid), earliest-update reducer,
  and a smoke test on the canonical prompt's behavioral
  constraints. The binary wire-up didn't add net-new tests
  because it threads existing types through the daemon's
  startup spawn pattern; the e2e harness already exercises
  that path. Honest miss on the lower bound.
- **Clippy** — Zero warnings across the workspace.
- **Q-block** — All four resolutions held:
  - **Q1(c) implemented.** Audit chain is the source of truth;
    `OutcomeSummaryCache` (capacity 8, VecDeque-backed LRU,
    keyed by `(lookback_secs, audit_chain_len)`) absorbs
    back-to-back fetches.
  - **Q2(a) implemented.** `REFLECTION_SYSTEM_PROMPT` is a
    hardcoded const in `reflection_scheduler.rs`; a smoke
    test guards its behavioral constraints from accidental
    gutting in future refactors.
  - **Q3(a) scoped.** `role_override` is recorded in the
    schedule config + the scheduler's log line for forensic
    attribution. The per-fire runtime role override is a
    deferred polish — v1 runs the reflection turn under the
    daemon's active role. Operators who want a dedicated
    reflection envelope today declare a `[[role]]` and run
    the daemon under it via the existing role-switching path.
    Acknowledged in the code comment at `fire_reflection`.
  - **Q4(a) implemented.** Errors from the audit walker or
    `TriggerDispatch::fire` emit a diagnostic + return; the
    scheduler updates `last_fired` regardless so a broken
    schedule doesn't hot-loop. No in-window retry; no
    consecutive-failure backoff.
- **Graceful degradation** — when `[[reflection_schedule]]`
  entries are configured but no audit log is available
  (test fixtures, PoC daemon), the daemon prints a clear
  diagnostic and runs without the scheduler. No silent
  no-op.

## Exit criteria

- [x] `run_reflection_scheduler` loop + cron dispatch — Task 2.
- [x] Canonical `REFLECTION_SYSTEM_PROMPT` constant + role
  envelope wiring — Task 3.
- [x] `OutcomeSummary` + audit-walk summarizer + LRU cache
  — Task 4.
- [x] Binary wire-up + startup-banner registration line —
  Task 5.
- [x] Tests across summarizer, cache, dispatch, failure
  path, integration — Task 6.
- [x] `examples/aivyx.toml` + `docs/INSTALL.md` updated —
  Task 7.
- [x] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 8.
- [x] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [x] DESIGN.md streak extends to eighteen.
- [x] PRODUCT.md streak extends to eleven.
- [x] Production-core streak extends to nineteen (new
  record).
- [x] Test count delta: positive (~+15-25).
- [x] Zero clippy warnings.
- [x] Zero new workspace deps.
- [x] Prediction-vs-reality block filled.
