# Phase 73 — Reach Tier-2 Polish: Retry, Rate Limit, History Pane

Phase journals are working documents. They churn freely during
the phase and freeze at exit under a final Exit criteria block.
For the locked technical contract see [`../DESIGN.md`](../../../DESIGN.md);
for the locked product contract see [`../PRODUCT.md`](../../../PRODUCT.md).

## Goal

Close the Tier-2 polish backlog Phase 72 explicitly deferred —
the per-target behavior knobs that compound on the Tier-1
multi-target / default / conditional bundle:

1. **Per-target retry semantics.** On transient failures
   (Transport / Timeout / Rejected 5xx), retry up to
   `retry_count` times with exponential backoff starting at
   `retry_backoff_ms_start`. Default `retry_count = 0` — no
   behavior change for pre-Phase-73 configs.
2. **Per-target rate limits.** Optional
   `rate_limit_max` + `rate_limit_window_secs` build an
   in-memory token bucket per target. Exhausted bucket
   records the new
   `AutoNotifyOutcomeSummary::SkippedByRateLimit { limit,
   window_secs }` audit variant.
3. **Web UI notification history pane.** New IPC
   `ListNotificationHistory { from_seq, limit, target_filter }`
   walks the audit chain for `AutoNotifyDispatched` events;
   the Web UI renders a paginated, filter-by-target table
   with outcome badges (delivered / failed / skipped-empty /
   skipped-by-condition / skipped-by-rate-limit). CLI parity
   gets `aivyx notify history` as a subcommand.

After Phase 73, the Reach Milestone polish backlog is closed
end-to-end. The remaining notify-shaped deferrals (WebPush /
service-worker for closed-tab delivery, Slack-flavored
webhooks) are operator-feedback-shaped and gated on real
operator pressure.

## Why now

1. **Phase 72 explicitly deferred this.** The Tier-1 trio
   shipped; Tier-2 was acknowledged at exit as "different
   cluster of concerns, focused follow-up." Phase 73 is that
   follow-up.
2. **Substrate is in place.** Audit chain records every
   `AutoNotifyDispatched` event since Phase 67. Concurrent
   fan-out + outcome variants land in Phase 72. Retry + rate
   limit + history all extend existing surfaces.
3. **Q-block fully resolved at design time.** Flat retry
   fields (Q1(b)), retry on Transport+Timeout+Rejected 5xx
   (Q2(b)), in-memory daemon-lifetime token bucket (Q3(a)),
   audit-derived paginated history view (Q4(a)) all signed off
   pre-Task 2.

## Streak predictions

- **DESIGN.md** — **Will hold.** Phase 73 extends config
  fields + dispatch logic + audit variants. No locked-contract
  shapes touched. Prediction: streak **extends to twenty**
  consecutive phases (currently at 19).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** Polish on an existing
  commitment (Reach Milestone); no commitment-text edits.
  Prediction: streak **extends to thirteen** consecutive
  phases.
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 73 work lives in `aivyx-config` (new fields),
  `aivyx-audit` (`SkippedByRateLimit` variant + history
  query response types), and `aivyx-channel` (retry loop,
  rate-bucket, IPC handler, Web UI pane, CLI subcommand).
  Prediction: streak **extends to twenty-one** consecutive
  phases (new record, beats Phase 72's 20).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

- **New workspace deps** — Zero expected. Backoff uses
  `tokio::time::sleep` (already in tree). Token bucket is a
  plain `HashMap<String, VecDeque<u64>>` (sliding-window
  timestamps) under a `tokio::sync::Mutex` (already in tree).

## Tasks

### Task 1 — Open commit + PHASE_73.md scaffold

This file. Update `docs/README.md` to show Phase 73 as Open.

### Task 2 — Config: retry + rate-limit fields

`aivyx-config`:

- `NotifyTargetConfig` gains four new fields:
    - `pub retry_count: u32` (default 0).
    - `pub retry_backoff_ms_start: u64` (default 500).
    - `pub rate_limit_max: Option<u32>` (default `None`).
    - `pub rate_limit_window_secs: Option<u64>` (default
      `None`).
- `RawNotifyTarget` accepts `retry_count`,
  `retry_backoff_ms_start`, `rate_limit_max`,
  `rate_limit_window_secs` per Q1(b)'s flat shape.
- Loader validation:
    - `retry_count` ≤ 10 (a hard ceiling — runaway retries
      are a footgun).
    - `retry_backoff_ms_start` ≥ 100 (don't hammer; allow
      tests to use the floor).
    - `rate_limit_max` and `rate_limit_window_secs` must
      both be set or both be unset (partial config is a
      load-time error naming the field).
    - When both set, `rate_limit_max` ≥ 1 and
      `rate_limit_window_secs` ≥ 1 (zero is meaningless).

### Task 3 — Audit: new outcome variant + history query types

`aivyx-audit`:

- `AutoNotifyOutcomeSummary::SkippedByRateLimit { limit,
  window_secs }` variant. `limit` + `window_secs` carry the
  effective bucket policy at the time of the skip so audit
  forensics can answer "what was the rate limit when this
  was skipped?" without needing the live config.

`aivyx-channel`:

- New IPC `QueryPayload::ListNotificationHistory { from_seq:
  u64, limit: u32, target_filter: Option<String> }`.
- Response `QueryResponsePayload::ListNotificationHistory {
  entries: Vec<NotificationHistoryEntry>, total_len: u64 }`.
- `NotificationHistoryEntry { seq, dispatched_at_unix_ms,
  session_id, trigger_kind, trigger_id, target_name,
  outcome_kind, outcome_detail }` — flat wire shape mirroring
  the existing `AuditEntrySummary` pattern.

### Task 4 — Dispatch: retry loop + rate bucket

`crates/aivyx-channel/src/trigger.rs`:

- New `RateLimitRegistry` (in-memory `HashMap<String,
  VecDeque<u64>>` under a `tokio::sync::Mutex`) tracks
  per-target dispatch timestamps. `check_and_record(target,
  max, window)` returns `Ok(())` when within budget or
  `Err(())` when exhausted; on Ok the call also records the
  timestamp so subsequent calls see it.
- Before each per-target dispatch in the fan-out loop:
    - If the target's rate limit is set, check the bucket.
      Exhausted → audit `SkippedByRateLimit` for that target
      and move on (no retry, no further attempts in this
      fire).
- Retry loop wraps the actual `dispatcher.dispatch(...)`
  call. On `NotifyError::Transport(_)`,
  `NotifyError::Timeout`, or
  `NotifyError::Rejected(status)` where `status >= 500`,
  sleep for `retry_backoff_ms_start * 2^attempt` and retry
  up to `retry_count` total attempts (per Q2(b)). Auth,
  UnknownTarget, and Rejected 4xx never retry. The final
  outcome (after exhausting retries) is what audits.

### Task 5 — Daemon-side history query handler

`crates/aivyx-channel/src/daemon_server.rs`:

- `handle_query` gets a new arm for `ListNotificationHistory`:
  walks the audit chain via `entries_range`, filters for
  `AuditEvent::AutoNotifyDispatched`, applies the
  `target_filter` if set, paginates by the `from_seq` /
  `limit` window (same cap as the audit pane: max 500 per
  page). Renders each match into a
  `NotificationHistoryEntry`.

### Task 6 — Web UI Notifications pane

`crates/aivyx-channel/src/web_ui_static.html`:

- New tab `data-pane="notifications"` between Audit and
  Sessions.
- Filter row: target chip (auto-populated from distinct
  target names in the loaded page) + pagination buttons.
- Per-entry card: target name, timestamp, outcome badge
  (color-coded: delivered = teal, failed = orange,
  skipped-* = amber), trigger source + id, error detail
  collapsible.
- Refresh button issues `ListNotificationHistory` query.
- HTML smoke test guarding tab + query construction + the
  five outcome badges.

### Task 7 — CLI subcommand

`crates/aivyx-channel/src/bin/aivyx.rs` + a new helper
module:

- `aivyx notify history [--target NAME] [--limit N]`.
- Reads the same IPC query the Web UI uses; renders a
  flat-text table with one line per entry: timestamp, target,
  outcome, trigger source + id, error if any.
- Parser tests for the three flag combinations + the
  bare-subcommand form.

### Task 8 — Tests

- Config: retry-count cap rejects, backoff floor rejects,
  rate-limit-partial rejects, sane defaults parse.
- Rate bucket: empty bucket admits N calls then rejects;
  window expiry restores capacity; per-target isolation
  (target A exhaustion doesn't block target B).
- Retry loop: Transport error retries `count` times,
  Timeout retries, Rejected 5xx retries, Rejected 4xx
  doesn't, Auth doesn't, UnknownTarget doesn't. Total
  attempts == count + 1 (initial + retries). Backoff
  doubles per attempt (mocked sleep).
- Audit: skipped-by-rate-limit emits the new variant with
  correct limit/window values.
- IPC: ListNotificationHistory round-trip + paginated
  filter-by-target.
- Web UI: HTML smoke test for the new pane.
- CLI: parser tests + render-helper tests.

### Task 9 — Docs

- `examples/aivyx.toml` gains retry + rate-limit fields on a
  commented `[[notify_target]]` block, with operator-note
  about the failure-mode taxonomy (which errors retry).
- `docs/INSTALL.md` "Multi-target + conditional dispatch"
  section grows a "Retry + rate limit" subsection covering
  defaults, the no-retry-on-auth rule, and the audit-trail
  for skipped-by-rate-limit.

### Task 10 — Exit commit

- `ROADMAP.md` Phase 73 frozen entry.
- `docs/PRODUCT_ROADMAP.md` Reach Milestone status: append
  "Phase 73 closed the Tier-2 polish backlog."
- `docs/README.md` status flip with backfill.
- Prediction-vs-reality block filled.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Retry config shape:** (b) Flat per-target fields:
  `retry_count` + `retry_backoff_ms_start`. Less TOML nesting,
  greppable. Default `retry_count = 0` preserves today's
  behavior. Operators who want broader retry tune per
  target.
- **Q2 — Retry policy:** (b) Transport + Timeout + Rejected
  where HTTP status ≥ 500. The 5xx-only carve-out for
  Rejected matches HTTP server-side transient semantics;
  401/403/4xx-other never retry. Auth + UnknownTarget never
  retry.
- **Q3 — Rate limits:** (a) In-memory per-target token bucket
  with daemon-lifetime persistence. Exhaustion records
  `SkippedByRateLimit { limit, window_secs }` in the audit
  chain. Daemon restart resets the bucket — acceptable for
  v1 since the audit chain remains the canonical record of
  what dispatched.
- **Q4 — History pane source:** (a) Audit-derived paginated
  view. `ListNotificationHistory` IPC walks the chain and
  filters for `AutoNotifyDispatched` events; same
  pagination shape as the audit pane (max 500 per page).
  No new storage layer.

## Deferrals

**Rolling deferrals carried into Phase 73:**

- v0.1.0 publication (Phase 61 Task 7).
- System-prompt notification-target enumeration (Phase 62).
- `turn_id` correlation refactor (Phase 67).
- Phase 68 deferrals (XOAUTH2, HTML email, attachments,
  multi-account `[email]`).
- Phase 69 deferrals (WebPush / service-worker, notification
  urgency, icons, sound).
- Phase 70 deferrals (proposal supersession, reflection on
  feedback events, multi-window reflection, memory/role
  proposal flows).
- Phase 71 deferrals (per-fire role override, operator-
  customizable reflection prompt, reflection cadence
  learning).
- Phase 72 deferrals (fan-out integration tests — partially
  re-opened here in Task 8's bucket).

**Likely Phase 73 deferrals:**

- **Persisted rate-limit buckets.** v1 keeps state in
  memory; daemon restart resets. Persistence is a clean
  follow-up if operators surface real "noisy neighbor"
  problems across restarts.
- **Operator-configurable retry-on list.** Q2(c) — let
  operators declare `retry_on = ["transport", "timeout",
  "rejected_5xx"]` per target. Defers until a real use case
  needs the granularity.
- **WebPush, Slack-shape webhook, XOAUTH2.** Rolling from
  prior phases. Each is operator-feedback-gated.

## Prediction vs. reality

**All three streak predictions correct.**

- **DESIGN.md** — Held. Hash at exit:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`
  (byte-identical to entry). Streak extends to **twenty**
  consecutive phases as predicted. Phase 73 added config
  fields, an audit variant, IPC envelopes, dispatch logic,
  Web UI pane, and CLI subcommand — none touched the locked
  technical contract.
- **PRODUCT.md** — Held. Hash at exit:
  `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`
  (byte-identical to entry). Streak extends to **thirteen**
  consecutive phases. Phase 73 polishes an existing
  commitment (Reach Milestone) without redefining it.
- **Production-core `aivyx-core/src/lib.rs`** — Held. Hash
  at exit:
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`
  (byte-identical to entry). Streak extends to **twenty-one**
  consecutive phases — **new project record**, beating
  Phase 72's 20. All Phase 73 work lived in `aivyx-config`
  (new fields), `aivyx-audit` (`SkippedByRateLimit` variant),
  and `aivyx-channel` (retry loop, rate-bucket registry,
  IPC handler, Web UI pane, CLI subcommand).
- **Workspace deps** — Zero new as predicted. `tokio::time::sleep`
  was already in tree for the backoff; the token bucket uses
  `tokio::sync::Mutex` + `HashMap` + `VecDeque` from std.
- **Tests** — +31 (1302 → 1333), comfortably inside the
  +25-35 prediction. Breakdown:
  - 8 new config tests (defaults, retry-count cap, backoff
    floor, explicit values, partial-rate-limit rejection
    × 2, zero-value rejection, both-set happy path).
  - 6 new `trigger::tests` (transient-failure includes /
    excludes, rate-bucket admit / evict / per-target
    isolation, TargetPolicy round-trip).
  - 5 new `daemon_server::tests` (renderer for the five
    outcome variants).
  - 1 new HTML smoke (Notifications pane wiring).
  - 6 new CLI parser tests (default shape, both flags, zero-
    limit error, non-numeric-limit error, bare-subcommand
    error, unknown-subcommand error).
  - 5 new render-helper tests in the notify module
    (empty/no-filter, empty/with-target, delivered, failed,
    pagination).
- **Clippy** — Zero warnings across the workspace.
- **Q-block** — All four resolutions held in implementation:
  - **Q1(b)** — Flat per-target fields `retry_count` +
    `retry_backoff_ms_start` on `NotifyTargetConfig`. Default
    `retry_count = 0` preserves Phase 62 behavior.
  - **Q2(b)** — `is_transient_failure` classifier: Transport
    + Timeout + Rejected with HTTP status ≥ 500. Auth,
    UnknownTarget, and Rejected 4xx never retry.
    `dispatch_with_retry` is a pure async helper.
  - **Q3(a)** — `RateLimitRegistry` is a per-target
    `VecDeque<u64>` of recent timestamps under a
    `tokio::sync::Mutex`. Daemon-lifetime state; restart
    resets the bucket. Exhausted bucket records the new
    `AutoNotifyOutcomeSummary::SkippedByRateLimit { limit,
    window_secs }` audit variant.
  - **Q4(a)** — `ListNotificationHistory` IPC walks the
    audit chain for `AutoNotifyDispatched` events, filters
    by target name, paginates with the same server-side cap
    (500) as the audit pane. No new storage.

After Phase 73 the Reach Milestone polish backlog is closed
end-to-end. The remaining notify-shaped deferrals (WebPush,
Slack-flavored webhooks, XOAUTH2) are operator-feedback-
gated.

## Exit criteria

- [x] `NotifyTargetConfig` gains four retry/rate-limit
  fields + raw parsing + validation — Task 2.
- [x] `AutoNotifyOutcomeSummary::SkippedByRateLimit` audit
  variant + `ListNotificationHistory` IPC envelopes — Task 3.
- [x] `RateLimitRegistry` + per-target token bucket — Task 4.
- [x] Retry loop wraps `dispatcher.dispatch` with the
  Q2(b) failure-class filter + exponential backoff — Task 4.
- [x] Daemon-side `ListNotificationHistory` handler walks
  the audit chain with target filter + pagination — Task 5.
- [x] Web UI Notifications pane with target chip + outcome
  badges + pagination — Task 6.
- [x] `aivyx notify history [--target ...] [--limit ...]`
  CLI — Task 7.
- [x] Tests across config, rate bucket, retry loop, audit,
  IPC, HTML smoke, CLI parser — Task 8.
- [x] `examples/aivyx.toml` + `docs/INSTALL.md` updated —
  Task 9.
- [x] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 10.
- [x] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [x] DESIGN.md streak extends to twenty.
- [x] PRODUCT.md streak extends to thirteen.
- [x] Production-core streak extends to twenty-one (new
  record).
- [x] Test count delta: positive (~+25-35).
- [x] Zero clippy warnings.
- [x] Zero new workspace deps.
- [x] Prediction-vs-reality block filled.
