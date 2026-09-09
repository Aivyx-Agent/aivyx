# Daemon-Side Teams — Durable, Interactive Nonagon (Chapter L)

> **Status:** design contract. This is the spec Chapter L scaffolds from.
>
> Chapter J shipped the Nonagon — a lead agent convening up to nine
> attenuated specialists over a mission **DAG** (`aivyx-team`). But it only
> runs **in-process**, one shot, via `aivyx-pa team run "<mission>"`: no daemon
> ownership, no live view, no durability. The J.7 TUI **Missions panel**
> (`MissionsState` / `MissionRow` / `Msg::MissionsUpdated`) is a finished
> view-model that **nothing feeds** — it renders the empty state.
>
> Chapter L makes the **daemon run teams**, streams their progress to a live
> TUI feed, **persists** missions across restarts, and adds **human-approval
> gates** (a mission can pause for the operator to approve/reject a step).
> It is the prerequisite for a web Mission-Control GUI (Ch.M).

---

## 1. What exists today

- **Engine, batch run.** `aivyx-team/src/runtime.rs::TeamRuntime::run(plan,
  lead_channel) -> Result<MissionReport, TeamError>` walks the whole DAG,
  running each ready set concurrently (`join_all`), and returns a
  `MissionReport { goal, outputs: BTreeMap<String,String>, status }` **only at
  the end**. `MissionStatus::{Completed, GateRejected{step,verdict}}`. There is
  **no step-level progress** — the caller is blind until completion.
- **Mission DAG.** `mission.rs`: `Step { id, kind, deps }`,
  `StepKind::{Delegate{specialist,prompt,memory_topic?}, Gate{reviewer,criteria}}`,
  `plan.ready(&completed)`, Kahn cycle detection, `validate()`. Gates are
  judged by `gate_passed()` (a verdict passes unless it begins with `FAIL`).
- **In-process driver.** `aivyx-cli/.../team.rs::run_mission` builds the team
  with `TeamAssembly::build(...)` and runs one turn over a one-shot
  `MissionChannel`. The daemon path **reuses** the assembly + channel.
- **TUI seam (J.7).** `aivyx-tui/src/model.rs`: `MissionsState { rows }`,
  `MissionRow { id, goal, phase, steps, ... }`, `MissionStep { state }`,
  `MissionPhase::{Planning, Executing, AwaitingApproval, Paused, Done,
  Rejected, Halted}` (`Paused` — Chapter Mission Control — and `Halted` —
  Chapter Ballast/Belay — both post-date this table; only `Paused` joins
  `Planning`/`Executing`/`AwaitingApproval` as non-terminal),
  `StepState::{Pending, Running, Done, Gated, Failed}`,
  `Msg::MissionsUpdated(Vec<MissionRow>)` + `MissionSelectNext/Prev`.
  `render.rs::render_missions` is master/detail. **The `AwaitingApproval`
  phase is currently unreachable** — Chapter L lights it up.
- **Pattern to mirror — the autonomous loop.** `SharedLoopState` (an `Arc`
  snapshot the daemon mutates each iteration) + a `LoopStatus` **poll** query
  + a daemon-spawned `run_loop_driver`. See `daemon_server.rs` (loop spawn),
  `daemon_ipc.rs` (`QueryPayload::LoopStatus`), `daemon_client.rs::loop_status`,
  `loop_cli.rs::render_status`.
- **Do NOT conflate.** `KeyDomain::Missions` + `MissionRecord` +
  `ListMissions`/`GetMission` is the **older single-agent** mission lifecycle
  (Phase 21). Chapter L adds a **new** `KeyDomain::TeamMissions` and a new
  team-mission IPC surface; the old surface is left intact.

---

## 2. The crux — checkpoint/resume, not a suspended future

The two operator decisions — **human gates** (a mission pauses for approval)
and **persistence** (a paused mission survives a daemon restart) — together
forbid the obvious implementation. A long-lived `async` future that simply
`.await`s an approval channel **dies when the daemon restarts**, taking the
paused mission with it.

So `TeamRuntime` becomes **checkpoint/resume-based**:

```
run_until_pause(plan, completed, lead_channel, observer) -> RunYield
    where RunYield = Completed(MissionReport)
                   | AwaitingHuman { step, partial: BTreeMap<…> }
                   | Rejected { step, verdict, partial }
```

- It walks ready sets exactly as `run` does, firing the `observer` as steps
  start/finish, **until** it reaches a `Gate { mode: Human, .. }` whose
  upstream is ready — then it **returns `AwaitingHuman`** with the
  accumulated `completed`/`outputs` checkpoint instead of blocking.
- The daemon **persists** that checkpoint (`TeamMissionRecord`, §4) and marks
  the mission `AwaitingApproval`. The in-flight future ends; nothing is held.
- On `ResolveTeamGate { approve: true }` the daemon **re-invokes**
  `run_until_pause(plan, completed_from_record, …)`, which records the human
  gate as passed and continues from the next ready set. `approve: false`
  marks the mission `Rejected` (the gate's dependents never run), partial
  outputs preserved.
- The existing `run` / `run_observed` become **thin wrappers** over
  `run_until_pause` with an empty starting checkpoint; a plan with no human
  gates runs straight to `Completed`, byte-for-byte as today.

This keeps the engine pure and the durability concern at the daemon boundary:
the checkpoint is just the `completed`/`outputs` state the DAG walk already
tracks, made serializable.

---

## 3. Gate modes

`StepKind::Gate` gains a mode (serde-defaulted to `Auto` so existing plans and
the kitchen pack are unchanged):

```
enum GateMode { Auto, Human }
StepKind::Gate { reviewer, criteria, mode: GateMode }
```

- **`Auto`** (today's behavior): the `reviewer` specialist judges the upstream
  output (`gate_passed`); FAIL → `GateRejected`, the run continues only on
  PASS. Fully automatic.
- **`Human`**: the runtime pauses (`AwaitingHuman`); the operator approves or
  rejects via the TUI/CLI. The `reviewer`/`criteria` are surfaced as context
  for the operator's decision (an optional advisory auto-review can still run
  and be shown, but the verdict is the human's).

---

## 4. Persistence — `KeyDomain::TeamMissions`

A new encrypted domain (mirrors `loop_backlog.rs::PersistentLoopBacklog`),
one row per mission keyed by mission id:

```
struct TeamMissionRecord {
    id: MissionId,            // ULID/uuid; stable across restart
    goal: String,
    plan: MissionPlan,        // the DAG (serde)
    outputs: BTreeMap<String,String>,   // the checkpoint (completed steps)
    phase: MissionPhase,      // Planning|Executing|AwaitingApproval|Paused|Done|Rejected|Halted
    pending_gate: Option<String>,       // step id when AwaitingApproval
    started_at_unix_ms: u64,
    updated_at_unix_ms: u64,
}
```

- The daemon **saves on every transition** (step complete, pause, resume,
  done/reject).
- On **startup**, the daemon reloads the store; `AwaitingApproval` missions
  are resumable, `Executing` missions interrupted by a crash are re-driven
  from their last checkpoint (idempotent — completed steps aren't re-run).
- Specialist sub-turns already land on the **persistent HMAC audit chain**
  (`KeyDomain::Audit`) — that durability is unchanged; this domain only adds
  the live mission/checkpoint state the audit chain doesn't model.

---

## 5. Daemon execution + IPC

- **`SharedMissionState`** — an in-memory registry (active + recent), keyed by
  id, backed by the store (mirrors `SharedLoopState`). The IPC read path and
  the run task both touch it.
- **`TeamRun { goal, config? }`** — assemble the team (`TeamAssembly::build`
  over the daemon's **real tool list**, as Chapter J's c905c4c established, so
  specialists get their attenuated tools), spawn `run_until_pause` with an
  observer that updates the snapshot + persists, on the shared chain. Returns
  the new `MissionId`.
- **`TeamMissionList` / `TeamMissionStatus { id }`** — poll queries returning
  snapshot(s); the TUI ticks `TeamMissionList`, the CLI renders one.
- **`ResolveTeamGate { mission_id, step, approve }`** — resume (`approve`) or
  abort (`!approve`) a paused mission.

The feed is **poll-based** (consistent with `LoopStatus`); streaming is a
future option that doesn't change this contract.

---

## 6. CLI + TUI + Chat surface

- **CLI** (`aivyx-pa team`, extends `team.rs`):
  - `aivyx-pa team run "<goal>" [--config <pack.toml>]` → **daemon-first**
    (sends `TeamRun`, then polls to render progress), **in-process fallback**
    when no daemon is running (today's path).
  - `aivyx-pa team status [<id>]` / `aivyx-pa team list` → render snapshots
    (pure render fns, mirror `loop_cli::render_status`).
  - `aivyx-pa team approve|reject <id> <step>` → `ResolveTeamGate`.
  - `aivyx-pa team abort <id>` → `AbortTeamMission` (**Chapter Belay**): stop a
    **running** mission. It halts gracefully at its next wave boundary —
    in-flight specialist turns finish, completed outputs are preserved — landing
    in `Halted` (reason "aborted by operator"), the same terminal shape as a
    tripped budget cap. Reuses the runtime's existing `observer.should_halt()`
    hook (no new cancellation plumbing into specialist turns). A mission *paused
    at a human gate* isn't running, so it can't be aborted this way — `reject`
    its gate instead. Completing the operator control surface over autonomous
    missions: budget-halt (Ballast) + gate approve/reject (L) + **abort**.
  - `aivyx-pa team pause <id>` / `aivyx-pa team resume <id>` → `PauseTeamMission` /
    `ResumeTeamMission` (**Chapter Mission Control**): pause requests a
    graceful stop at the mission's next wave boundary — the same mechanism as
    abort (in-flight specialist turns finish, completed outputs are
    preserved) — but lands in the new, **non-terminal** `Paused` phase
    instead of `Halted`. `resume` continues the DAG walk from the preserved
    checkpoint, re-seeding the per-mission budget meter from the mission's
    persisted cumulative spend rather than resetting it, and reusing the
    same resume machinery gate-approval already uses. A mission *paused at a
    human gate* isn't running the same way, so it can't be paused this way
    either — resolve its gate instead.
- **TUI** (`aivyx-tui`):
  - A periodic mission-poll tick in `app.rs` maps `TeamMissionList`
    snapshots → `MissionRow`s → `Msg::MissionsUpdated`. The panel goes live.
  - The `AwaitingApproval` phase renders an approve/reject affordance; a key
    binding sends `ResolveTeamGate`. The TUI stays free of `aivyx-team` types
    (the driver maps snapshots → rows, same seam as J.7).
- **Chat** (Telegram/Discord/Slack daemon-frontends): a `/team ...`
  command set recognized before the normal chat-turn path (`team_command.rs`
  parses; `team_dispatch.rs` dispatches the monitoring/control subcommands —
  `/team status [<id>]`, `/team approve|reject <id> <step>`,
  `/team pause|resume <id>`, `/team abort <id>` — over the same
  daemon-internal call pattern the CLI uses). Same semantics as the CLI
  surface above (a mission paused at a human gate can't be paused/aborted,
  resolve its gate instead); replies are chat-appropriate text, not the
  CLI's fixed-width tables.
  - **Sender allowlist (2026-08-23).** The entire `/team ...` surface above
    — not just `/team run` below — is gated by a per-channel
    `team_command_allowed_senders` list (`aivyx-pa.toml`, `[telegram]`/
    `[discord]`/`[slack]`); an unset/empty list denies every `/team`
    command from every sender (deny-by-default). A bare "yes"/"no" reply
    to a `/team run` confirm prompt is separately bound to the sender who
    staged that trigger (see below), not just gated by this list, since
    "yes"/"no" never parses as a `/team` command and so never reaches this
    check at all. See `docs/INSTALL.md`'s `/team` section for the
    operator-facing config and the upgrade-breaking-change note.
  - **`/team run <goal>`** (Piece C) starts a *new* mission instead of
    controlling an existing one, so it does **not** go through
    `team_dispatch.rs` or the CLI's anonymous `Query` IPC path — that path
    has no per-caller authorization and would let any chat message start a
    mission. It uses its own separate, identity-declaring path instead
    (`daemon_client::run_team_mission_channel`, which does its own
    `StartSession` handshake so the daemon knows which real channel is
    asking) and is confirm-first: the bot replies "Start '<goal>' on the
    default team? Reply yes/no." and only calls
    `run_team_mission_channel` on a bare "yes" within 5 minutes ("no" or a
    stale "yes" cancels instead). The daemon only honors the request if the
    operator opted the channel in via `team_run_channel = true` in
    `aivyx-pa.toml` (default `false` — off for every channel until set); an
    optional `team_trigger_rate_limit` caps confirmed starts per rolling
    hour per chat. See `docs/INSTALL.md` and `docs/ROUTINES.md` for the
    operator-facing config and usage.

---

## 7. Phase plan

| Phase | Deliverable |
|---|---|
| **L.0** | This design contract. |
| **L.1** | Engine: `MissionObserver` trait + `run_observed` (additive progress feed; no behavior change). |
| **L.2** | Engine: `GateMode::{Auto,Human}` + the `run_until_pause` checkpoint/resume refactor (`run`/`run_observed` become wrappers). |
| **L.3** | Persistence: `KeyDomain::TeamMissions` + `TeamMissionRecord` + `PersistentTeamMissionStore` + reload-on-startup. |
| **L.4** | Daemon: `SharedMissionState` + `TeamRun` / `ResolveTeamGate` handlers (assemble over the real tool list, on the shared chain). |
| **L.5** | IPC variants + `daemon_client` helpers + `aivyx-pa team run\|status\|list\|approve\|reject`. |
| **L.6** | TUI poll tick + live `MissionsUpdated` feed + `AwaitingApproval` approve/reject UX. |
| **Ch.M** ✅ | Web **Mission Control** GUI (separate chapter, unblocked by this one) — shipped 2026-08-22/23: live mission state, pause/resume, and the Studio's Mission Control nav view (LEAD/specialist graph, drill-in, abort/pause/resume controls). See `aivyx-ecosystem/ROADMAP.md`'s Mission Control entry for the full 3-piece account. |
| _(deferred)_ | L.7 autonomous-loop ↔ team integration. |

---

## 8. Invariants

- **NT-02 preserved.** Specialists stay attenuated (`declared ∩ lead`); the
  daemon path changes *who drives* the team, never the capability math.
- **One HMAC chain.** Daemon-run specialist sub-turns append to the same
  `KeyDomain::Audit` chain as every other turn — `aivyx-pa audit export` /
  `--verify-only` see them.
- **Resume idempotence.** `run_until_pause` from a checkpoint never re-runs a
  completed step; running a no-human-gate plan through the resume path equals
  running it straight through (a tested equivalence).
- **Old mission surface untouched.** `KeyDomain::Missions` /
  `ListMissions` / `GetMission` keep working; team missions are additive.
