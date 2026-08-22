# Team-Mission Triggers — Scheduling + Channel Integration Design

**Status:** Design approved, not yet planned or implemented.

## Motivation

Found during an end-user-deployment audit (2026-08-23, prompted by "how would
an end user deploy their Aivyx Agentic Team?"): team missions (Nonagon) have
exactly two entry points today — the CLI (`aivyx team run`) and the Studio
web GUI — both requiring a human at the keyboard in the moment. Two concrete
consequences, both verified against real code, not inferred:

1. **Scheduled/recurring team missions don't exist.** `docs/NONAGON.md`'s own
   worked example is framed as *"Mission (overnight loop): 'Run end-of-day
   BOH close.'"* — but the `[[schedule]]` primitive
   (`crates/aivyx-channel/src/daemon_scheduler.rs`'s `fire_schedule`) has
   exactly two execution paths: a deterministic digest builder (one specific
   routine only) or `dispatch.fire(...)`, which always fires `sched.prompt`
   as a **single-agent LLM turn** via `TriggerDispatch::fire`
   (`crates/aivyx-channel/src/trigger.rs`). No team-mission dispatch path
   exists. `TriggerSource` has no team-mission-*originating* variant — its
   only team-adjacent variant, `Mission` (Chapter Herald), fires *after* a
   mission finishes (a notification), never *to start* one. An operator has
   to run `aivyx team run` by hand every night, or build an external cron
   wrapper around the CLI.
2. **Channel adapters have zero team-mission integration.** Grepped
   `aivyx-telegram`/`aivyx-discord`/`aivyx-slack` for any `team run`/
   `TeamRun` reference — zero hits in all three. An operator who talks to
   their single-agent assistant via a channel has no way to trigger,
   monitor, or approve/reject a team-mission gate from that same channel.

Neither gap is documented anywhere as a known/accepted limitation.

## What already exists (verified, changes the shape of the fix)

- **`team.run` already exists as a real `Tool`** (`TeamRunTool`,
  `crates/aivyx-channel/src/team_mission_driver.rs:1639`), registered into
  the daemon's shared `tool_list`
  (`crates/aivyx-cli/src/bin/aivyx.rs:7276`) — the same tool list every
  daemon-hosted turn draws from (CLI, TUI, web, autonomous-loop iterations,
  *and* scheduled turns). Its `required_scope` is `team.run`, which
  `crates/aivyx-capability/src/lib.rs`'s `TrustTier::Trusted` default
  ceiling grants (confirmed via the crate's own tests, lines 1992/1996) and
  `Untrusted`/`SemiTrusted` do not.
  - **Consequence:** a scheduled team mission is technically reachable
    *today*, indirectly — a `[[schedule]]` entry whose role holds `team.run`
    and whose prompt asks the model to call it. This is why the "gap" isn't
    a hard wall; it's a *reliability* gap (the LLM must correctly decide to
    call the tool) and a *traceability* gap (no link between "this schedule"
    and "the mission it spawned" — it shows up as an ordinary
    single-agent-turn `MissionRecord`, not a `TeamMissionRecord`).
- **Channel adapters are hardcoded to `TrustTier::SemiTrusted`**
  (`crates/aivyx-telegram/src/telegram_channel.rs:228`, a constant, not
  config-driven) — which structurally cannot reach `team.run`
  (`Trusted`-only). This is a deliberate security boundary (see the doc
  comment's reference to "D4"), not an oversight.
- **Team-mission *control* is a different, less-restricted trust
  boundary.** `daemon_server.rs`'s `ResolveTeamGate`/`AbortTeamMission`/
  `PauseTeamMission`/`ResumeTeamMission` handlers (lines 4250–4294) have
  **no capability-scope check at all** — they're plain daemon-IPC handlers,
  reachable by anything with a daemon connection (the CLI, the web Studio's
  WebSocket). A channel adapter's own code runs *inside* the same trusted
  daemon process, so it can call these directly without touching the
  SemiTrusted ceiling.
- **No channel today has any native command parser.** Every message is
  currently forwarded as an LLM-turn prompt (confirmed: no slash-command or
  message-routing precedent found in `aivyx-telegram`/`aivyx-channel`).
- **`TeamMissionService::start_from_goal`
  (`crates/aivyx-channel/src/team_mission_driver.rs:1009`) already supports
  pinning a specific vertical-pack team** (`config: Option<TeamConfig>`) —
  reused directly by this design, not extended.
- **Team missions already support multiple concurrent missions and durable
  gate-parking** (`AwaitingApproval` persists indefinitely until resolved —
  the whole point of Chapter L's checkpoint/resume design). This design
  relies on that rather than inventing a headless/auto-reject mode: a
  scheduled or channel-triggered mission that hits a human gate simply
  parks, exactly like a manually-run one, discoverable via Mission Control.

## Architecture

A new concept: a **team-mission trigger** — anything that can start,
monitor, or control a team mission without a human directly using the CLI
or Studio. This design adds two: a cron schedule, and an authorized channel
command. Three independently-shippable pieces (same shape as the Mission
Control initiative's own 3-piece split):

- **Piece A** — scheduled team missions (`aivyx-channel`'s scheduler).
- **Piece B** — channel-triggered monitoring/control (all 3 channel
  adapters: `aivyx-telegram`, `aivyx-discord`, `aivyx-slack`).
- **Piece C** — channel-triggered new mission starts (same 3 adapters).

Piece B ships no new capability surface (reuses existing unscoped IPC
handlers). Piece C is the only piece that opens new attack surface, and is
scoped, gated, and rate-limited accordingly.

## Piece A — Scheduled team missions

**Data model.** `ScheduleRecord` (`crates/aivyx-channel/src/schedule.rs`)
gains an optional team-mission target:

```rust
/// Chapter Muster — a schedule targets EITHER a single-agent turn
/// (role_name + prompt, today's shape) OR a team mission (this field).
/// Mutually exclusive with role_name/prompt -- validated in
/// ScheduleRecord::new / a new constructor, never both set.
#[serde(default)]
pub team_mission: Option<ScheduledTeamMission>,

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTeamMission {
    pub goal: String,
    /// None => the daemon's default team/roster.
    pub pack_config: Option<PathBuf>,
}
```

`#[serde(default)]` so every pre-existing record (all of which are
single-agent) deserializes unchanged, matching this file's own established
pattern for every other additive field on this struct.

**Dispatch.** `fire_schedule` (`daemon_scheduler.rs:209`) gains a new
branch, checked before the existing `report_kind == "digest"` /
`dispatch.fire(...)` paths: if `sched.team_mission` is `Some`, call
`TeamMissionService::start_from_goal(&goal, pack_config)` directly —
deterministic, no LLM tool-call in the loop, no `TriggerDispatch`
involvement at all (this bypasses the single-agent turn machinery
entirely, it doesn't extend it). The resulting `TeamMissionRecord` is
tagged with the originating `schedule_id` (new field on
`TeamMissionRecord`, threaded through `start_from_goal` or set
immediately after) for traceability — Mission Control and `aivyx team
status` can then show "started by schedule: nightly-boh-close."

**Notification.** Extends the schedule's existing `notify_targets`/
`notify_when` (already present on `ScheduleRecord`, unused by this new
path today): a lightweight observer on the spawned mission fires the
schedule's configured notify targets when the mission reaches
`AwaitingApproval` (so a gate doesn't sit unattended indefinitely with no
signal) **or** a terminal phase (`Done`/`Rejected`/`Halted`). This is new
logic — the existing notify path is keyed off single-agent `TurnOutcome`
and doesn't apply to team-mission phase transitions.

**Surfaces.** `aivyx schedule` CLI and the Studio's schedule editor
(wherever `[[schedule]]` entries are authored/edited) gain the option to
target a team mission instead of a role+prompt — exact UI/CLI shape is an
implementation-plan-level decision, not fixed here.

## Piece B — Channel-triggered monitoring/control

**No new capability.** Verified: `ResolveTeamGate`/`AbortTeamMission`/
`PauseTeamMission`/`ResumeTeamMission` have no capability-scope check in
`daemon_server.rs` — plain daemon-internal calls. A channel adapter's own
message-handling code already runs inside the trusted daemon process and
can call the same functions those IPC handlers call, directly, in-process.

**New: a native command layer**, one per channel adapter (Telegram,
Discord, Slack — same pass, per the approved scope). Recognizes a fixed
command set **before** falling through to the normal chat-turn path (today
every message becomes an LLM-turn prompt with no exception):

- `/team status [<id>]` — render the mission snapshot(s), mirroring the
  CLI's own render functions where they exist (e.g. `loop_cli`'s render
  pattern for `loop status`).
- `/team approve <id> <step>` / `/team reject <id> <step>` →
  `ResolveTeamGate`.
- `/team pause <id>` / `/team resume <id>` → `PauseTeamMission` /
  `ResumeTeamMission`.
- `/team abort <id>` → `AbortTeamMission`.

Each channel's own message-ingress point gains a parse-command-first check;
unrecognized text still falls through to the existing chat-turn path
unchanged.

## Piece C — Channel-triggered new mission starts

**New capability base: `team.run.channel`.** Added to
`crates/aivyx-capability/src/lib.rs`'s `KNOWN_BASES`, but — unlike most
bases — **not part of any trust tier's default ceiling**, `Trusted`
included. Purely operator opt-in, granted only via a channel's own
`capability_scopes` config (the same mechanism that already scopes which
tools a given channel connection can reach today — SemiTrusted channels
already declare an explicit scope list). This keeps `team.run` (the
Trusted-tier, LLM-invoked tool) and `team.run.channel` (this new,
narrow, native-command-only capability) as two genuinely separate grants —
opting a channel into this never grants it anything else Trusted-tier
implies.

**Command.** `/team run <goal>` — same native-command pattern as Piece B
(deterministic, not LLM-tool-mediated, for the same reliability reason as
Piece A), checked directly against the channel's own `CapabilitySet` for
`team.run.channel` (not routed through `Tool::execute` — there's no LLM
decision to gate here, but it's still a real, auditable, revocable grant,
visible wherever channel capability scopes are already surfaced, e.g. the
Studio's Settings screen).

**Confirm-first.** On a `/team run <goal>` from an authorized channel, the
agent replies asking for confirmation (*"Start '<goal>' on the default
team? Reply yes/no."*) rather than starting immediately. A short-lived
pending-trigger record (`goal`, `pack_config`, originating channel/chat id,
`expires_at`) is held (in-memory or store-backed — implementation-plan
decision) until confirmed or it times out (a fixed window, e.g. 5 minutes).
Only on explicit confirmation does the command actually call
`start_from_goal`.

**Rate limiting.** A **new, dedicated** per-channel config knob (e.g.
`[telegram] team_trigger_rate_limit`), independent of the existing
`[rate_limit]` tool-call machinery — this path bypasses that machinery
entirely, since it's a native command, not a `Tool::execute` call. Checked
in the native command handler before even sending the confirm-first
prompt, so a rate-limited channel gets a clear "too many mission-start
requests, try again in N minutes" reply rather than silence. Every
resulting mission still gets `[budget]`'s existing unconditional
per-mission cap (Ballast) regardless of trigger source — that safety net
needs no change.

## Error handling

- `start_from_goal` failure (bad goal, decomposition failure, no team
  configured) → surfaced via the schedule's `notify_targets` (Piece A) or
  as a channel reply (Piece C); never silently swallowed.
- Channel command from an unauthorized channel / missing capability → a
  clear, standard capability-denial reply (matching the existing pattern
  for other channel-tier-denied requests), not silence.
- Confirm-first timeout → pending trigger discarded; a late "yes" gets a
  clear "that request expired, ask again" reply, not a stale/confusing
  start.
- Rate-limited trigger attempt → clear reply naming the limit, not silence.
- Concurrent missions from multiple trigger sources (a schedule fires while
  a channel-triggered mission is already running, etc.) → no new
  constraint needed; `TeamMissionService` already supports multiple
  concurrent missions (Mission Control's own mission-picker UI exists
  specifically for this).

## Testing

- **Piece A**: pure-function tests on `ScheduleRecord`'s new mutual-
  exclusivity validation; pure-function tests on the new notify-on-
  `AwaitingApproval`/terminal-phase logic, decoupled from the scheduler
  loop itself where possible (matching this repo's established preference
  for pure, Signal/IO-free helpers over integration-only coverage).
- **Piece B**: per-channel command-parsing tests (recognized vs.
  unrecognized, malformed args, case sensitivity) + a dispatch test per
  command confirming it reaches the correct daemon function — same shape
  as the existing `daemon_server.rs` IPC-handler test conventions.
- **Piece C**: capability-check tests (channel with/without
  `team.run.channel`); confirm-first state-machine tests (confirm, reject,
  timeout, a stale confirm after expiry); rate-limit tests (under limit,
  at limit, reset after the window).

## Out of scope (explicitly, not forgotten)

- **No new headless/auto-reject gate mode.** Every trigger in this design
  uses the existing interactive model — a scheduled or channel-triggered
  mission that hits a gate parks in `AwaitingApproval` exactly like a
  manually-run one. `run_goal_blocking`'s headless `GatePolicy` (built for
  the autonomous loop) is a related but distinct mechanism, not reused
  here.
- **No LLM-mediated triggering anywhere in this design.** Every trigger
  (cron fire, channel command) is deterministic dispatch — never "the
  model decided to call a tool." This is the core property that makes this
  design actually solve the reliability gap that motivated it, as opposed
  to the already-possible-but-unreliable tool-call path.
- **Exact CLI/Studio UI for authoring a scheduled team mission** (Piece A)
  and the exact confirm-first pending-trigger storage mechanism (Piece C)
  are implementation-plan-level decisions, not fixed here.
- **`team.run`'s existing behavior, scope, and default ceiling are
  untouched.** This design adds a second, narrower, channel-specific
  capability (`team.run.channel`) rather than changing who can reach the
  existing one.
