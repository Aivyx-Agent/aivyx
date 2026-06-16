# Phase 27 — Scheduled Execution Phase 2: Webhook Triggers + File Watchers

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../../../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../../../PRODUCT.md).

## Goal

Complete **PRODUCT.md G5 — Autonomous and scheduled execution**
by adding the two remaining trigger types (webhooks and file-
change watchers) and closing the Phase 26 automatic-mission-
wrapping deferral. Phase 26 delivered cron timer primitives;
this phase adds event-driven triggers and unifies all trigger
sources under a shared dispatch abstraction.

After this phase, G5 is fully delivered: the daemon can fire
agent turns in response to cron schedules (Phase 26), HTTP
webhook requests, and filesystem changes — all attributed to
the operator's identity per P6.

## Why now

1. **Phase 26's cron scheduler is the pattern.** The
   `daemon_scheduler.rs` loop, `ScheduleRecord` storage, and
   `fire_schedule` dispatch path are proven. Webhooks and file
   watchers are additional trigger sources that follow the
   same pattern: event → turn → audit.

2. **Completing G5 closes the last open goal.** G3 (memory
   reflection), G4 (sub-agent orchestration), and G6 (local
   execution / privacy) are all delivered. G5 is the last
   forward commitment with undelivered pieces.

3. **The automatic-mission-wrapping deferral from Phase 26**
   naturally belongs with the trigger unification — all
   trigger types benefit from the same opt-in mission wrapping.

4. **hyper is already a transitive dependency** (via reqwest /
   tokio ecosystem). Using it directly for the webhook listener
   avoids adding a new workspace dependency.

## Streak predictions

- **DESIGN.md** — Low risk. Webhooks and file watchers are
  daemon-internal trigger sources, not new architectural
  primitives. The existing daemon IPC spec covers the pattern.
  Prediction: **untouched**.

- **PRODUCT.md** — Low risk. G5 is already committed; this
  phase delivers the remainder. The Delivery Status section's
  G5 entry will update at exit freeze but that's a docs
  change inside PRODUCT.md, which may break the streak.
  Prediction: **likely untouched** (status update can go in
  the phase doc instead).

- **Production-core `aivyx-core/src/lib.rs`** — Low risk.
  Triggers operate at the daemon/channel level, not the core
  turn loop. Prediction: streak **extends to eighteen**.

## Open questions

**Q1 — HTTP framework for webhook listener?** (a) Direct
`hyper` usage — it's already a transitive dependency, so no
new workspace dep. A minimal `hyper::server::conn::http1`
listener on `127.0.0.1` is ~50 LOC. (b) Higher-level
framework like `axum` — more ergonomic but adds a new dep.
Leaning **(a)** to preserve zero-new-dep.

**Q2 — File-watch dependency?** (a) The `notify` crate —
well-maintained, cross-platform (inotify on Linux, kqueue
on macOS, ReadDirectoryChanges on Windows). (b) Hand-rolled
inotify via `libc` — zero deps but Linux-only. Leaning
**(a)** — `notify` is the ecosystem standard and Aivyx
already targets multiple platforms. This would add one new
workspace dependency, breaking the zero-new-dep streak.

**Q3 — Should mission wrapping be opt-in or default?**
(a) Opt-in via `wrap_mission = true` on trigger configs —
fire-and-forget is a valid use case for simple triggers.
(b) Default-on — every triggered turn gets a mission for
audit lifecycle. Leaning **(a)** — opt-in keeps the simple
case simple while making the audited case available.

## Tasks

### Task 1 — Open commit + PHASE_27.md scaffold

This file. Update `docs/README.md` to show Phase 27 as Open.
Update `docs/ROADMAP.md` to show Phase 27 as Active.

### Task 2 — Trigger abstraction layer

Generalize Phase 26's `fire_schedule` into a unified trigger
dispatch path. Introduce a `TriggerSource` enum (`Cron`,
`Webhook`, `FileWatch`) so all trigger types share turn
serialization, dedup, operator attribution, and (later)
optional mission wrapping. The cron scheduler continues to
work exactly as before — this task refactors the dispatch
path, not the cron logic.

### Task 3 — Webhook trigger

Localhost-only HTTP listener inside the daemon. A `POST
/trigger/<id>` endpoint fires the associated trigger's prompt
through the agent. Config surface: `[[webhook]]` TOML entries
with `WebhookConfig` struct. Agent tools: `webhook.create`,
`webhook.list`, `webhook.delete` — all `CEILING_TRUSTED`.
Persistence: `KeyDomain::Webhooks` (8th encrypted storage
domain) or reuse of schedules domain with a discriminator.

### Task 4 — File-watch trigger

File-system change watcher using the `notify` crate. Config
surface: `[[file_watch]]` TOML entries with `FileWatchConfig`
struct (path patterns, debounce interval). Agent tools:
`file_watch.create`, `file_watch.list`, `file_watch.delete`
— all `CEILING_TRUSTED`. Debounce logic to prevent rapid
re-fires on editor save storms.

### Task 5 — Automatic mission wrapping

Triggered turns (cron, webhook, file-watch) optionally create
a mission for gate/audit lifecycle. An `wrap_mission = true`
field on trigger configs (schedule, webhook, file_watch)
causes `fire_trigger` to create a mission via the existing
`mission::create_mission` path before dispatching the turn.
Closes the Phase 26 deferral: "automatic mission wrapping
for scheduled turns."

### Task 6 — Exit freeze + docs

Exit criteria checklist, prediction-vs-reality table, streak
report, ROADMAP.md update, README.md status update.

## Ship records

### Task 1 — Open commit (2026-04-18)

Commit `a8ce418`. Scaffolded `docs/PHASE_27.md` with goal, why-
now, streak predictions, three open questions, and six-task
breakdown. Updated `docs/README.md` phase-status table (Phase 27
Open). Updated `docs/ROADMAP.md` with Phase 27 active pointer
and Phase 28 placeholder. Updated `docs/PRODUCT_ROADMAP.md` with
Phase 27 active entry under Scheduled Execution milestone.

### Task 2 — Trigger abstraction layer (2026-04-18)

Commit `54e31a6`. Introduced `TriggerSource` enum (`Cron`,
`Webhook`, `FileWatch`) and `TriggerDispatch` struct — shared
turn-dispatch context with `Mutex<()>` serialization. Refactored
`daemon_scheduler.rs` to use `TriggerDispatch` instead of raw
agent + channel factory references. Two tests.

Streak check:
- **Production-core** — `d8ab203f…` — streak holds at seventeen.
- **DESIGN.md** — Untouched.
- **PRODUCT.md** — Untouched.

### Task 3 — Webhook trigger (2026-04-18)

Commit `28cf3f2`. Localhost-only HTTP listener via hyper on
`127.0.0.1:7842`. Three new modules: `webhook.rs` (WebhookRecord
CRUD under `KeyDomain::Webhooks`, 8th storage domain),
`webhook_listener.rs` (hyper HTTP/1.1 server, `POST /trigger/<id>`
fires turn, `GET /health` returns OK, async 202 Accepted),
`webhook_tool.rs` (`webhook.create`, `.list`, `.delete` — OnceLock
pattern, `CEILING_TRUSTED`). Config surface: `WebhookConfig` +
`[[webhook]]` TOML entries. Inline config sync in binary.
httpdate enters Cargo.lock as transitive dep via hyper[server].
Six tests.

Streak check:
- **Production-core** — `d8ab203f…` — streak holds at seventeen.
- **DESIGN.md** — Untouched.
- **PRODUCT.md** — Untouched.

### Task 4 — File-watch trigger (2026-04-18)

Commit `afaa1d7`. Filesystem-change-triggered execution via
`notify` crate. Three new modules: `file_watch.rs` (FileWatchRecord
CRUD under `KeyDomain::FileWatches`, 9th storage domain, with
`debounce_ms` default 2000ms), `file_watcher.rs` (daemon loop using
`notify::RecommendedWatcher`, 60s reload interval, path→watch_id
lookup with canonicalization, `sync_config_file_watches` +
`config_to_records`), `file_watch_tool.rs` (`file_watch.create`,
`.list`, `.delete` — OnceLock pattern, `CEILING_TRUSTED`). Config
surface: `FileWatchConfig` with optional `debounce_ms` +
`[[file_watch]]` TOML entries. `notify` enters Cargo.lock as new
direct dep. Fifteen tests.

Streak check:
- **Production-core** — `d8ab203f…` — streak holds at seventeen.
- **DESIGN.md** — Untouched.
- **PRODUCT.md** — Untouched.

### Task 5 — Automatic mission wrapping (2026-04-18)

Commit `331a299`. Opt-in `wrap_mission = true` field on all
trigger configs (`ScheduleConfig`, `WebhookConfig`,
`FileWatchConfig`) and their storage records (`ScheduleRecord`,
`WebhookRecord`, `FileWatchRecord`). `TriggerDispatch::fire()`
extended with `wrap_mission: bool` parameter: when true and a
mission store is configured (via `with_mission_store()`), creates
a MissionRecord (Created → Running) before the turn and completes
or cancels it after based on `TurnOutcome`. Escalated outcomes
left in Running for the normal gate path. Mission store threaded
from `daemon_server.rs` into the dispatch. `#[serde(default)]` on
TOML raw structs ensures backward compatibility with existing
configs. Closes the Phase 26 deferral: "automatic mission wrapping
for scheduled turns." Two tests.

Streak check:
- **Production-core** — `d8ab203f…` — streak holds at seventeen.
- **DESIGN.md** — Untouched.
- **PRODUCT.md** — Untouched.

### Task 6 — Exit freeze (2026-04-18)

This task. Docs-only.

### Exit criteria (final)

1. ✅ `TriggerSource` enum with `Cron`, `Webhook`, `FileWatch` variants,
   unified dispatch via `TriggerDispatch::fire()`.
2. ✅ Webhook HTTP listener on `127.0.0.1:7842` (localhost-only per P6),
   `POST /trigger/<id>` fires turn with 202 Accepted, async background
   execution.
3. ✅ File-watch trigger via `notify` crate with per-watch debounce
   (default 2000ms), 60s reload interval, recursive directory watching.
4. ✅ Nine encrypted storage domains (added `Webhooks`, `FileWatches`).
5. ✅ Nine capability bases added: `webhook.create`, `webhook.list`,
   `webhook.delete`, `file_watch.create`, `file_watch.list`,
   `file_watch.delete` — all `CEILING_TRUSTED` only.
6. ✅ Config surface: `[[webhook]]`, `[[file_watch]]` TOML entries with
   config-to-storage sync on daemon startup.
7. ✅ Automatic mission wrapping: opt-in `wrap_mission = true` on all
   trigger configs, creating MissionRecords for gate/audit lifecycle.
8. ✅ 690 tests pass, 0 failures. +30 net-new tests this phase
   (660 → 690).
9. ✅ Production-core `aivyx-core/src/lib.rs` streak extends to
   **eighteen consecutive phases** — `d8ab203f…`.
10. ✅ DESIGN.md untouched — `ceb53860…`.
11. ✅ PRODUCT.md untouched — `478cab6a…`.

### Prediction vs reality

| Prediction | Reality |
|---|---|
| DESIGN.md — low risk, untouched | **Correct.** Untouched. Triggers are daemon-internal concerns, not new architectural primitives. |
| PRODUCT.md — low risk, likely untouched | **Correct.** Untouched. |
| Production-core — low risk, streak extends to eighteen | **Correct.** All trigger work operates at daemon/channel/storage level. |
| Q1 — hyper for webhook listener (leaning direct hyper) | **Direct hyper.** hyper is already a transitive dep; direct usage is ~80 LOC for a minimal HTTP/1.1 listener. httpdate entered Cargo.lock as a transitive dep. |
| Q2 — notify crate for file watching (leaning yes) | **Yes.** `notify` 8.x provides cross-platform filesystem notification. Breaks zero-new-dep at 2 entries for this phase. |
| Q3 — mission wrapping opt-in or default (leaning opt-in) | **Opt-in.** `wrap_mission = true` on trigger configs, defaults to `false`. Keeps simple triggers simple. |

### Decisions made during Phase 27 not in DESIGN.md

1. **Trigger dispatch serialization.** All trigger types share a
   single `Mutex<()>` turn lock. This prevents concurrent triggered
   turns from interleaving, which simplifies the audit chain and
   avoids provider rate-limit contention. If parallel triggers
   become needed, the lock can be replaced with a semaphore.

2. **Webhook port 7842.** Hardcoded localhost-only on
   `127.0.0.1:7842`. No configuration surface yet — a future phase
   can add `[daemon] webhook_port` to the config. The port number
   was chosen to be memorable (aivyx → roughly "ai" + "vyx") and
   unlikely to conflict with common services.

3. **File-watch reconciliation strategy.** The watcher is rebuilt
   from scratch (not incrementally updated) every 60 seconds when
   the watch set changes. This is simpler than incremental
   add/remove and the cost is negligible at the expected scale
   (tens of watches, not thousands).

4. **Mission wrapping outcome mapping.** `TurnOutcome::Completed`
   → mission completed. `Failed`/`Cancelled`/`TimedOut` → mission
   cancelled (not failed — the mission itself didn't hit a gate
   rejection). `Escalated` → left in Running (the gate path
   handles it). This avoids conflating turn-level failure with
   mission-level failure.

5. **New Cargo.lock entries.** `httpdate` (transitive via hyper
   server feature) and `notify` (direct) are the two new Cargo.lock
   entries this phase. Zero-new-dep streak broken at 2.
