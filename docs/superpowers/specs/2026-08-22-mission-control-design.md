# Mission Control — Live Nonagon Visualization Design

**Status:** Piece 1 (Live mission state) shipped 2026-08-22 — plan at
`docs/superpowers/plans/2026-08-22-mission-control-live-state.md`, merged
to `main` at `2989a043`. Pieces 2 (Pause/Resume) and 3 (the Mission
Control nav view) are not yet planned or implemented.

## Motivation

`aivyx-desktop` was created as a thin native shell (tray icon, notifications,
daemon lifecycle, a system-webview window hosting the local Studio) — its own
`Cargo.toml` says so explicitly: "A thin native chrome over the existing web
UI, not a UI rewrite." All the real interface lives in `aivyx-web` ("Chapter
M — Mission Control"), a ~6,500-line Dioxus app.

Since `aivyx-desktop` was created, the ecosystem has grown a real
multi-specialist team system (Nonagon: a LEAD delegating to up to 8
specialists over a DAG of steps, gates, and budget-aware halts) — but
`aivyx-web`'s own "Teams" surface is roster **configuration** (who's on the
team, capability scopes, templates), and its "Missions" surface is a flat,
poll-refreshed list with coarse per-step status. Neither shows a mission
*executing* — which specialist is working right now, what just handed off to
what, or lets an operator pause/resume/abort what they're watching happen.

This was raised as "should we build a brand-new Ecosystem Frontend instead of
just the Aivyx PA's own UI." Investigation (recorded in full in the
brainstorming conversation this doc's approval came out of) found:

- The other ecosystem repos don't actually support a *cross-repo* dashboard
  today — `aivyx-coder` is a deliberately separate, unrelated CLI tool
  (per this workspace's own root `CLAUDE.md`); `aivyx-kvcache` /
  `aivyx-confine` / `aivyx-checkpoint` are pure infrastructure libraries with
  no end-user surface at all. A dashboard spanning them would mostly be
  static links, not a real product surface.
- The ecosystem's own glossary already names the actual "front door to
  everything" concept — **Aivyx-Hub** — as a planned, not-yet-started, hosted
  layer, explicitly gated on `aivyx` reaching a later `VISION.md`
  growth-chain rung. Building a parallel concept now would preempt and
  probably conflict with that.
- What current 2026 multi-agent UI practice (LangGraph Studio, AutoGen
  Studio, CrewAI Studio) converges on — a live graph/org-chart of agents,
  handoffs, and per-agent drill-in, with a lead-and-workers hierarchy — maps
  directly onto Nonagon's own LEAD/specialist shape, and is a real gap in
  the *existing* `aivyx-web`, not a reason to start a new app.

**Decision: evolve `aivyx-web`/`aivyx-desktop`, not build a new frontend.**
Any genuine cross-repo "front door" stays deferred to future Aivyx-Hub work,
consistent with the ecosystem's own existing sequencing.

## Current state (verified against real code, not assumed)

- `aivyx-web`'s `TeamsPanel` component (`crates/aivyx-web/src/main.rs`) is
  roster config: edit the team, capability scopes, Nonagon Templates
  draft-from-role. It reads `TeamsState`, not live mission execution.
- The poll response (`QueryResponsePayload::TeamMissionList`) actually
  sends raw `Vec<TeamMissionRecord>` over the wire — the frontend calls
  `.to_view()` **client-side**, in wasm, on each record it receives
  (`crates/aivyx-web/src/main.rs`'s WebSocket read loop). `TeamMissionView`
  (`crates/aivyx-ipc/src/team_mission.rs`) is a pure, stateless projection
  of a record's checkpoint: `TeamStepState` is one of `Pending | Done |
  Awaiting | Rejected`. There is **no "running right now" state** — a step
  in flight looks identical to one that hasn't started. This matters for
  Piece 1's design below: the running-step signal lives only in the
  driver's in-memory state, never in the persisted `TeamMissionRecord`, so
  a client computing `.to_view()` locally from a record it already has can
  never learn it — the broadcast **must** carry an already-projected
  `TeamMissionView` (built daemon-side, where the live signal is in scope),
  not a bare record for the client to project itself.
- The frontend refreshes missions by **polling** (a source comment reads:
  "Routines panel polls with the missions + audit feed") — there is no
  live push for mission state today.
- **Abort already exists end-to-end**: `aivyx team abort <id>` CLI, the
  `AbortTeamMission` query (`crates/aivyx-ipc/src/protocol.rs`), and
  `MissionDriver::abort`/`abort_mission`
  (`crates/aivyx-channel/src/team_mission_driver.rs`, "Chapter Belay") —
  halts gracefully at the next wave boundary, preserving the checkpoint.
  This lands in `TeamMissionPhase::Halted`, which `is_terminal()` treats as
  terminal — there is no resume path from it today, even though the
  checkpoint data needed to resume is already on disk.
- A daemon → Web-UI **broadcast** mechanism already exists:
  `WebUiBroadcaster` (`crates/aivyx-channel/src/notify_webui.rs`) is a
  `tokio::sync::broadcast` channel with `subscribe()`/`broadcast()`,
  currently used only for `DaemonMessage::DesktopNotification`
  ("Phase 69 — broadcast-style Web UI desktop notification... relayed onto
  every connected Web UI WebSocket"). This is the exact primitive Piece 1
  extends — not a new mechanism.
- Nothing in `aivyx-desktop` needs to change — it hosts whatever `aivyx-web`
  serves and has no opinion about what's inside.

## Scope decomposition

Same shape as this session's kvcache-adoption initiative: one design, three
independently planned and executed pieces, because they have different risk
profiles and mostly-separate touch surfaces.

1. **Piece 1 — Live mission state.** Backend: a new in-flight step signal
   and a broadcast push path, extending `WebUiBroadcaster`.
2. **Piece 2 — Pause/Resume.** Backend: a new non-terminal
   `TeamMissionPhase::Paused`, new IPC messages, new driver methods, CLI
   parity with `abort`.
3. **Piece 3 — Mission Control view.** Frontend: a new nav destination in
   `aivyx-web` with a live graph/org-chart visualization and controls,
   consuming Piece 1's broadcast and Piece 2's new messages.

Piece 3 depends on both Piece 1 and Piece 2 being in place to be fully live
and fully interactive, but Piece 1 and Piece 2 are independent of each
other and can be built and merged in either order.

## Piece 1 — Live mission state

**Goal:** a connected Web UI client sees a mission's step transition to
"running" the moment the driver starts it, without polling.

**Wire additions** (`crates/aivyx-ipc/src/team_mission.rs`):
- `TeamStepState` gains a `Running` variant, inserted between `Pending` and
  `Done` in the enum declaration (the existing order is not itself
  semantically load-bearing — no code matches on ordinal position — this
  is purely for a reader scanning the definition top to bottom).
- `TeamMissionView` is otherwise unchanged — `to_view()`'s projection still
  derives `Pending`/`Done`/`Awaiting`/`Rejected` from the checkpoint exactly
  as today; `Running` is layered on by the **driver**, not the record
  projection, because "running" is transient in-memory state that must
  never be persisted (a step that was running when the daemon crashed is
  simply not running after a restart — the checkpoint has no opinion, and
  should have none).

**Driver-side tracking** (`crates/aivyx-channel/src/team_mission_driver.rs`):
- The driver already knows, synchronously, which step it is about to
  execute in `drive`/`drive_registered` (it iterates the DAG wave by wave).
  Add an in-memory `Arc<Mutex<HashMap<String /* mission id */, String /*
  running step id */>>>` (or equivalent) owned by the driver/registry,
  set immediately before a step's delegate call starts and cleared the
  moment it completes (success, rejection, or halt). This is the same
  "ephemeral, not persisted, keyed by mission id" shape `pending_gate`
  would have if it weren't durable-by-requirement (it *is* persisted,
  because an approval gate must survive a restart; a running-step marker
  must not, so it deliberately does not reuse that field or its storage).
- A new `to_view_with_live_state(&self, running_step: Option<&str>) ->
  TeamMissionView` (or the existing `to_view` gains an optional parameter)
  overlays `Running` onto the one step id currently in flight, leaving the
  checkpoint-derived states for every other step untouched.

**Broadcast** (`crates/aivyx-channel/src/notify_webui.rs` +
`crates/aivyx-ipc/src/protocol.rs`):
- `DaemonMessage` gains a new broadcast variant, e.g.
  `TeamMissionUpdated { view: TeamMissionView }`, following
  `DesktopNotification`'s exact precedent (broadcast, no session
  correlation).
- The driver calls `WebUiBroadcaster::broadcast(...)` at each of: a step
  starting (now `Running`), a step completing (checkpoint write), a phase
  transition (`Executing → AwaitingApproval`, `→ Paused`, `→ Halted`, `→
  Done`, `→ Rejected`). Reuses the existing broadcaster instance already
  wired into the daemon — no new connection/transport plumbing.

**Frontend** (`crates/aivyx-web/src/main.rs`):
- The existing `missions: Signal<Vec<TeamMissionView>>` context gains a
  new branch in the WebSocket message-handling `match` for
  `DaemonMessage::TeamMissionUpdated`, updating (or inserting) that one
  mission's entry in place.
- The existing poll (on the Routines/dashboard panels) stays as a
  reconciliation fallback for reconnect/missed-broadcast cases — it is not
  removed, only no longer the primary source of truth for an open Mission
  Control view.

**Testing:** driver-side unit tests proving (a) the running-step marker is
set/cleared at the right points and never leaks across missions or survives
a completed step; (b) `to_view_with_live_state` overlays exactly one
`Running` step and leaves the rest identical to plain `to_view()`; (c) a
broadcast fires (using `WebUiBroadcaster`'s own existing
`broadcast_reaches_single_subscriber`-style test harness) at each of the
listed transition points, not fewer, not more. Wire round-trip
(de)serialization test for the new `TeamStepState::Running` variant and
`DaemonMessage::TeamMissionUpdated`, matching this codebase's existing
per-message serde test convention.

## Piece 2 — Pause/Resume

**Goal:** an operator can stop a mission after its current wave finishes,
in a way that is later reversible — distinct from abort, which is not.

**New phase** (`crates/aivyx-ipc/src/team_mission.rs`):
- `TeamMissionPhase::Paused` — a new variant, inserted logically next to
  `AwaitingApproval` (another non-terminal, operator-driven pause point).
  `is_terminal()` returns `false` for it, matching its real semantics
  (unlike `Halted`, which stays terminal and unchanged).

**New IPC messages** (`crates/aivyx-ipc/src/protocol.rs`), mirroring
`AbortTeamMission`'s exact existing shape:
```rust
PauseTeamMission { mission_id: String },
// Responds with QueryResponsePayload::TeamMissionPaused.
ResumeTeamMission { mission_id: String },
// Responds with QueryResponsePayload::TeamMissionResumed.
```

**Driver methods** (`crates/aivyx-channel/src/team_mission_driver.rs`),
mirroring `abort`/`abort_mission`'s existing shape:
- `MissionDriver::pause(&self, id: &str) -> Result<String,
  MissionDriverError>` — sets a request-pause flag, checked at the same
  wave-boundary point `should_halt` already checks the abort flag and the
  Ballast budget cap; on trip, the mission's phase transitions to `Paused`
  (not `Halted`) and the checkpoint is preserved exactly as an abort's is.
  Reuses the existing wave-boundary-halt code path — the only new logic is
  *which phase it lands in*, driven by *why* it stopped. Three independent
  requests can be pending at the same wave-boundary check (abort, pause,
  the Ballast budget cap); the check must have one fixed, explicit priority
  order rather than depending on evaluation order falling out incidentally
  from how the code happens to be written. Priority, highest first: **abort
  > pause > budget cap** — abort is the existing, most-decisive operator
  intent and must never be silently downgraded to a resumable pause by a
  pause request that also happened to be pending; the budget cap is a
  system-imposed stop, lowest priority against either explicit operator
  action.
- `MissionDriver::resume(&self, id: &str) -> Result<String,
  MissionDriverError>` — only valid when `phase == Paused` (any other phase
  is a `MissionDriverError`, mirroring how `approve`/`reject` already
  reject a call against a mission not in `AwaitingApproval`); transitions
  back to `Executing` and re-enters the DAG walk from the existing
  checkpoint via the same `run_until_pause`-shaped resume entry point the
  approval-gate flow already uses (`docs/DAEMON_TEAMS.md`'s stated "resume
  idempotence" property — never re-runs a completed step — applies
  identically here, since resume-from-pause and resume-from-gate-approval
  are the same underlying "continue the DAG walk from a checkpoint"
  operation with a different trigger).

**CLI parity** (`crates/aivyx-cli/src/bin/aivyx_modules/team_cli.rs` or
wherever `team abort` lives): `aivyx team pause <id>` / `aivyx team resume
<id>`, matching `abort`'s existing command shape and help text
conventions.

**Testing:** driver unit tests for (a) pause at a wave boundary lands in
`Paused`, not `Halted`, with the checkpoint intact; (b) resume from
`Paused` continues the DAG and completes remaining steps, with a
discriminating assertion that a step already in the checkpoint before pause
is never re-run (mirroring the existing
`budget_halt_stops_at_wave_boundary_preserving_outputs`-style test's
"wave 2 never ran" assertion pattern, adapted to "wave 2 runs *after*
resume, wave 1's output is untouched"); (c) resume against a non-`Paused`
mission is rejected with a clear error, not a silent no-op; (d) pause and
abort requested concurrently on the same mission — exactly one outcome
wins, deterministically, no partial/torn state.

## Piece 3 — Mission Control view

**Goal:** a dedicated nav destination in `aivyx-web` that shows one active
mission's live specialist graph, matching the LangGraph/AutoGen/CrewAI
Studio pattern this design's research phase converged on — the LEAD at the
center, specialists around it, live handoffs, click-to-drill-in.

**Navigation**: a new `View` variant (alongside the existing `Teams` /
`Missions` / `Dashboard` entries in `crates/aivyx-web/src/main.rs`'s `View`
enum), its own sidebar `NavItem`, its own icon asset under
`crates/aivyx-web/assets/icons/`.

**Mission selection**: if more than one mission is `Executing` /
`AwaitingApproval` / `Paused` at once, a lightweight selector (not a
simultaneous multi-mission graph — deliberately out of scope, see below)
lets the operator pick which one Mission Control is currently watching.

**Graph visualization**: LEAD node at the center; one node per specialist
on the current team roster (reusing `TeamConfig`'s member list, already
fetched for `TeamsPanel`); edges reflect the mission plan's DAG
dependencies (`MissionPlan.steps`, already available via
`TeamMissionView.steps`, augmented with the `Running` state from Piece 1).
The step currently `Running` highlights/animates its specialist node.
Completed steps show their specialist's edge as done; a step in
`Awaiting` highlights the gate reviewer.

**Drill-in**: clicking a specialist node opens a detail panel — role,
current step (if any), capability scopes, and the NT-02 "inert" hint
`TeamsPanel` already computes for scopes the LEAD hasn't also declared
(reuse that exact logic, don't reimplement it).

**Controls**: gate approve/reject (existing `ResolveTeamGate`, already
wired elsewhere — reuse, don't duplicate), abort (existing
`AbortTeamMission`, currently has no UI affordance at all — this is its
first), pause/resume (Piece 2's new messages).

**Styling**: drawn from the existing `aivyx-web/assets/stitch.css` design
tokens — no new visual language. Dark-mode-first (already the ecosystem's
own convention per `aivyx-brand`'s Stitch tokens), consistent with 2026
developer-tool UI norms this design's research surfaced.

**Testing**: Dioxus component-level tests where the existing `aivyx-web`
test suite already has a precedent to match (check what exists before
assuming a pattern — `aivyx-web` compiles only for `wasm32-unknown-unknown`
per its own `Cargo.toml`, which constrains what test harness is realistic;
the implementation plan for this piece must verify what's actually testable
before committing to a test shape, rather than assuming a native-target
pattern applies). At minimum: a pure, non-Dioxus unit test for whatever
function maps a `TeamMissionView` + roster into the graph's node/edge data
structure, decoupled from rendering.

## Explicitly out of scope for this initiative

- Any cross-repo dashboard surfacing `aivyx-coder`, `aivyx-recall`,
  `aivyx-kvcache`, etc. — deferred to future Aivyx-Hub work, per the
  Motivation section above.
- A simultaneous multi-mission "fleet" graph view — v1 watches one mission
  at a time.
- Any change to `aivyx-desktop` — it hosts whatever `aivyx-web` serves and
  needs no changes for this work.
- Any change to `aivyx-tui`'s own team-mission surface (out of this
  design's scope; if it wants parity later, that's separate follow-on
  work).
- Interrupting a specialist genuinely *mid-step* (mid-tool-call) — pause
  and abort both stop at the next wave boundary, matching the existing,
  already-proven Ballast/Belay halt semantics. True mid-step interruption
  would require canceling an in-flight LLM/tool call, a materially
  different and riskier problem not requested here.

## Sequencing recommendation

Piece 1 and Piece 2 first (either order — independent), Piece 3 last, since
it is the only piece that consumes both. This also means Piece 3's own
implementation plan can be written with full knowledge of Piece 1/2's real,
shipped interfaces rather than speculating about them.
