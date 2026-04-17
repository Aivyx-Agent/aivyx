# Phase 26 — Scheduled Execution: Timer Primitives

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Deliver the first half of **PRODUCT.md G5 — Autonomous and
scheduled execution**. Add cron-style timer triggers to the
daemon so missions can run unattended on a schedule. An
operator writes a `[[schedule]]` entry in `aivyx.toml` or an
agent calls `schedule.create`, and the daemon fires turns at
the scheduled time without operator interaction.

## Why now

1. **The daemon substrate is delivered.** Phase 16-17 shipped
   the Unix-socket daemon with session management, IPC framing,
   and multi-frontend support. The scheduler lives inside the
   daemon process — no new process model needed.

2. **Missions survive restarts.** Phase 20-21 shipped the
   mission state machine with persistence, gates, and
   completion. A scheduled turn that creates a mission gets
   the full gate/audit lifecycle for free.

3. **G5 is the last unstarted goal with all prerequisites
   met.** The daemon (G5 prereq 1) and missions (G5 prereq 2)
   both exist. Timer primitives are the missing piece.

4. **Scheduled Execution is the execution substrate** that
   Web UI Channel, Reflection Layer, and future milestones
   all consume. Building it first means every future phase
   benefits from timer/trigger primitives existing.

## Streak predictions

- **DESIGN.md** — Medium risk. The scheduler may need a new
  amendment section documenting the timer storage format and
  the trigger-to-turn flow. Prediction: **one amendment** for
  the scheduler architecture.

- **PRODUCT.md** — Low risk. G5 is already committed; this
  phase delivers against it, not amending it.

- **Production-core `aivyx-core/src/lib.rs`** — Low risk.
  The scheduler operates at the daemon level, not the core
  turn loop. Prediction: streak **extends to seventeen**.

## Open questions

**Q1 — Do scheduled triggers create missions or bounded
tasks?** Leaning (a) missions — the mission state machine
already handles gates, persistence, and audit attribution.
A "bounded task" would be a mission with no gates that
auto-completes. The mission primitive is the right
abstraction.

**Q2 — Should the scheduler persist across daemon restarts?**
Leaning (a) yes — schedules stored in `aivyx-storage` (or
TOML config) should survive `daemon stop` / `daemon run`
cycles. A volatile-only scheduler would lose operator
schedules on restart, which violates the "missions survive
restarts" property.

**Q3 — What cron syntax subset?** Leaning (a) standard
5-field cron (`minute hour day-of-month month day-of-week`)
with `@daily`, `@hourly` shorthand. Full cron is well-
understood and tooling exists. Custom interval-only syntax
(`every 30m`) is simpler but less expressive.

## Tasks

### Task 1 — Open commit + PHASE_26.md scaffold

This file. Update `docs/README.md` to show Phase 26 as Open.

### Task 2 — Schedule storage + data model

Add a `Schedule` struct (id, cron expression, role, prompt
template, enabled flag) and persistence layer. Store in
`aivyx-storage` under a new `KeyDomain::Schedules` or in
the TOML config as `[[schedule]]` entries. Parse cron
expressions at load time.

**Ship record — Task 2**

| Artefact | What changed |
|---|---|
| `Cargo.toml` (workspace) | Added `cron = "0.16"` and `chrono = { version = "0.4", … }` workspace deps |
| `crates/aivyx-storage/src/lib.rs` | 7th `KeyDomain::Schedules` — subkey derivation, table name `aivyx_schedules_v1`, `ALL` array bump to 7 |
| `crates/aivyx-channel/src/schedule.rs` | **New.** `ScheduleRecord` struct, cron validation via `cron` crate (7-field expressions), `next_fire_time` / `next_fire_time_after`, full CRUD (`create_schedule`, `get_schedule`, `update_schedule`, `list_schedules`, `delete_schedule`), 7 tests |
| `crates/aivyx-channel/src/lib.rs` | `pub mod schedule;` |
| `crates/aivyx-channel/Cargo.toml` | Added `cron` and `chrono` workspace deps |
| `crates/aivyx-config/src/lib.rs` | `ScheduleConfig` struct, `RawSchedule`, `[[schedule]]` TOML surface, `schedules` field on `AivyxConfig` |
| `crates/aivyx-channel/src/bin/aivyx.rs` | Binary destructure updated for `schedules: _schedules` |

Answers to open questions surfaced during implementation:

- **Q3 resolved — 7-field cron.** The `cron` crate uses 7-field
  expressions (`sec min hour dom month dow year`), not standard
  5-field. `@daily` shorthand support is crate-version-dependent
  and not relied upon. This is documented in the test suite.

Streak check:

- **Production-core** — `d8ab203f…` — streak **extends to
  seventeen**. Scheduler touches storage and channel, not core.
- **DESIGN.md** — Untouched this task (amendment deferred to
  Task 3 or later if the scheduler architecture warrants it).
- **PRODUCT.md** — Untouched this task.

Test delta: 643 pass, 0 fail. +7 new schedule tests.

### Task 3 — Daemon scheduler loop

A background task inside the daemon that evaluates schedule
entries against the current time and fires `SubmitInput`-
equivalent turns when a schedule is due. Needs: next-fire-
time computation, deduplication (don't double-fire on slow
turns), and attribution (scheduled turns carry the
operator's identity per P6).

**Ship record — Task 3**

| Artefact | What changed |
|---|---|
| `crates/aivyx-channel/src/daemon_scheduler.rs` | **New.** `run_scheduler` background loop — adaptive tick (sleep until next-fire, capped at 60 s), deduplication via `last_fired_at ≥ fire_time`, `sync_config_schedules` for TOML → storage merge, `config_to_records` converter, turn serialization via `Mutex`. 8 tests |
| `crates/aivyx-channel/src/daemon_server.rs` | `run_daemon` gains `schedule_store: Option<DomainHandle>` param; spawns `run_scheduler` alongside accept loop sharing agent + factory + shutdown token |
| `crates/aivyx-channel/src/lib.rs` | `pub mod daemon_scheduler;` |
| `crates/aivyx-channel/src/bin/aivyx.rs` | Config schedule sync at daemon startup; passes `KeyDomain::Schedules` handle to `run_daemon` |
| `crates/aivyx-channel/tests/daemon_roundtrip_e2e.rs` | 5 call sites updated for new `run_daemon` arity (`schedule_store: None`) |

Design decisions:

- **Adaptive tick cadence**: sleeps until the earliest next-fire-time
  across all enabled schedules, capped at 60 s so dynamically created
  schedules are picked up within one minute.
- **Turn serialization**: a `Mutex` serializes scheduled turns so two
  schedules firing simultaneously don't interleave on the agent.
- **Config-to-storage sync**: TOML `[[schedule]]` entries are merged
  into `KeyDomain::Schedules` on daemon startup. Storage is
  authoritative after first sync — edits via agent tools persist
  independently of the config file.
- **Deduplication**: `last_fired_at ≥ fire_time` prevents double-fire
  if the scheduler ticks again before the turn completes.

Streak check:

- **Production-core** — `d8ab203f…` — streak holds at seventeen.
- **DESIGN.md** — Untouched.
- **PRODUCT.md** — Untouched.

Test delta: 652 pass, 0 fail. +9 new scheduler tests (8 in
`daemon_scheduler.rs`, 0 regressions in e2e suite).

### Task 4 — `schedule.create` / `schedule.list` /
`schedule.delete` tools

Agent-facing tool surface so the LLM can create, list, and
delete schedules within a turn. Capability scope:
`schedule.create`, `schedule.list`, `schedule.delete`.

**Ship record — Task 4**

| Artefact | What changed |
|---|---|
| `crates/aivyx-channel/src/schedule_tool.rs` | **New.** Three tools: `ScheduleCreateTool` (cron + prompt → persisted schedule), `ScheduleListTool` (returns all schedules with next-fire-time), `ScheduleDeleteTool` (by ID with not-found handling). `OnceLock`-factory pattern per `MissionCreateTool`. 6 tests |
| `crates/aivyx-channel/src/lib.rs` | `pub mod schedule_tool;` |
| `crates/aivyx-channel/src/bin/aivyx.rs` | Tool registration + `set_schedule_store` wiring for all three tools |
| `crates/aivyx-capability/src/lib.rs` | Added `schedule.create`, `schedule.list`, `schedule.delete` to `KNOWN_BASES` and `CEILING_TRUSTED` (omitted from `CEILING_SEMITRUSTED` — schedule management is a Trusted-tier operation) |

Design decisions:

- **Trusted-tier only.** Schedule tools are in `CEILING_TRUSTED`
  but not `CEILING_SEMITRUSTED`. Creating/deleting cron schedules
  is an operator-level action — a SemiTrusted channel (Telegram)
  should not be able to create unattended daemon turns.
- **`schedule.delete` returns `deleted: false`** for unknown IDs
  rather than failing, matching the idempotent pattern.
- **`schedule.list` includes `next_fire`** (RFC 3339) so the LLM
  can reason about upcoming fires without parsing cron.

Streak check:

- **Production-core** — `d8ab203f…` — streak holds at seventeen.
- **DESIGN.md** — Untouched.
- **PRODUCT.md** — Untouched.

Test delta: 658 pass, 0 fail. +6 new schedule tool tests.

### Task 5+ — Scope TBD at Task 4 exit

Candidates: `[[schedule]]` TOML config surface (if not done
in Task 2), `--schedule` CLI flag, webhook triggers (Phase
2 of the milestone), file-change watchers.

## Deferrals

**Rolling deferrals carried from Phase 25 (13 items):**

- **Forensic `ToolOutcome::NotInRole` variant** —
  Phase 11 Q1. Untouched.
- **Second regression channel for the role primitive** —
  Phase 11 Q6. Untouched.
- **Response headers in audit payload** — Phase 12 Q3 half.
  Untouched.
- **Non-GET verbs (POST/PUT/PATCH/DELETE)** — Phase 12 Q1.
  Deferred indefinitely.
- **Redirect following with per-hop scope re-check** —
  Phase 12 Q5. Deferred indefinitely.
- **Binary response bodies / non-UTF-8** — Deferred
  indefinitely.
- **Per-chunk Telegram rendering** — Phase 12 Task 1.
  Deferred reactively.
- **Multi-level sub-agent nesting** — Phase 14 Task 3.
  Untouched.
- **LocalChannel regression-test rewrite over IPC** —
  Phase 17 Q6->(c+). Tagged: **reactive.**
- **Telegram-specific protocol extensions (attachment
  delivery, inline keyboards, etc.)** — Phase 19. Untouched.
- **`mission.list` / `mission.status` read-only tools** —
  Phase 21. Untouched.
- **MCP SSE transport** — Phase 23. Untouched.
- **Provider-specific token counting** — Phase 25.
  Untouched.
