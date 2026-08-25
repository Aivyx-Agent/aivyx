# Recursive-Scheduling Guard — design

**Status:** Approved, ready for planning.

## Motivation

The 2026-08-25 Hermes Agent comparison research
(`aivyx-ecosystem/docs/research/2026-08-25-hermes-agent-comparison-
research.md`) flagged an open question: does Aivyx's own scheduled-run
system prevent a scheduled run from recursively creating more scheduled
runs, the way Hermes Agent explicitly does ("cron-run sessions cannot
recursively create more cron jobs")? This wasn't checked at the time.
This is the second of two chapters the user picked directly off that
comparison's findings — the first, Ecosystem Security Policy, shipped
earlier the same day.

## Research findings (grounded in the real, current code)

**`schedule.create`/`schedule.update`/`schedule.delete` are
unconditionally part of the default capability floor.**
`compute_backcompat_floor` (`aivyx-cli/src/bin/aivyx.rs`) includes all
three scopes with an explicit comment reasoning that the tools' own
in-tool guardrails (growth gradient, frequency floor, count cap) make
blanket-granting them "governance-safe at every level." That floor —
`deps.lead_scopes`, computed once at daemon startup — is reused
identically by every mission-start path via the one shared
`assemble_runtime`/`bind_lead_scopes` chain
(`aivyx-channel/src/team_mission_driver.rs`): `start_from_goal`
(interactive `team.run`), `start_from_goal_for_schedule` (cron/team-
mission schedules), and `start_from_goal_for_channel_trigger`
(Telegram/Discord/Slack `/team run`). There is no origin-based
narrowing anywhere in this chain.

**For prompt-based (non-team-mission) schedules**, `ScheduleRecord
.role_name` is captured on creation but was found to be consumed
nowhere outside `schedule.rs`/`schedule_tool.rs` — every cron-fired
prompt turn runs under whatever the daemon's single active role
currently is, identical capability set to any interactive turn under
that role. (A separate, latent finding — `role_name` appears
effectively vestigial for the fired turn's own authority. Noted here,
not fixed as part of this piece; see "Out of scope.")

**`ToolContext` (what every `Tool::execute` receives) currently has no
field carrying trigger origin.** Its fields are `agent_id`,
`session_id`, `turn_id`, `channel`, `audit`, `cancellation` — nothing
that would let a tool distinguish "this call originated from within an
already-triggered run" from an interactive one.

**A promising existing foundation, however**: `Message` already
carries an `origin: MessageOrigin` field (`Operator` vs `System`), and
`TriggerDispatch::fire()` — the *one* shared function that `Cron`
(`daemon_scheduler.rs`), `Webhook` (`webhook_listener.rs`),
`FileWatch` (`file_watcher.rs`), `Reflection`
(`workspace_journal.rs`, `reflection_scheduler.rs`), and `Loop`
(`loop_driver.rs`) — 5 of the 6 `TriggerSource` variants — all funnel
through, already tags every fired message `.system_originated()`
(`MessageOrigin::System`) unconditionally. This signal already exists
and is already correctly set; it just never reaches `ToolContext`.

**The team-mission path (goal-based schedules, plus the 6th
`TriggerSource::Mission` variant) does not go through `Message`/
`fire()` at all.** `start_from_goal_for_schedule` calls
`aivyx_team::decompose_goal` directly, then
`register_mission_for_schedule`, then `spawn_drive` — no `Message` is
ever constructed. It does, however, already tag the resulting
`MissionRecord.triggered_by` with the schedule's ID
(`.with_triggered_by(schedule_id)`), and the channel-trigger path
tags its own missions the same way with a channel tag (e.g.
`"channel:telegram"`, via `.with_triggered_by(trigger_tag)`) — real,
existing provenance this design builds on rather than inventing from
scratch.

**Existing mitigations that reduce, but do not close, the gap:** the
growth gradient's default (`GrowthAdoption::ProposeOnly`, "the safe
default") lands newly agent-created schedules disabled pending
operator approval — genuinely protective by default. An operator who
opts into `PolicyAuto`/`BroadAuto` — a real, supported configuration
for wanting useful self-scheduling — loses that protection entirely:
new schedules arm directly, with zero origin-awareness. The frequency
floor (15 minutes) bounds how often one schedule can fire, not how
many can be created. The one real backstop against unbounded growth is
`MAX_AGENT_SCHEDULES = 10`, a shared global cap across all
agent-created schedules regardless of provenance — real, but not
recursion-aware, and exhausting it also blocks legitimate future
interactive self-scheduling as a side effect.

**Verdict: this is a real, confirmed, live gap**, not a
false alarm and not something already sufficiently covered by
adjacent mechanisms. Under a real, supported operator configuration
(`PolicyAuto`/`BroadAuto`), a triggered run recursively creating and
auto-arming more schedules is possible today, with no equivalent of
Hermes' named guard anywhere in the stack.

## Scope

All decisions below were made explicitly by the user during
brainstorming, not assumed:

- **Guard mechanism**: an explicit recursion guard at the tool level
  (`ScheduleCreateTool`/`ScheduleUpdateTool`/`ScheduleDeleteTool::
  execute` refuse outright), matching Hermes' own precedent directly
  — not a capability-floor narrowing, not a softer "force-disabled"
  mitigation.
- **Tools covered**: `schedule.create`, `schedule.update`, **and**
  `schedule.delete` (the most conservative of the three options
  offered — not just the "create" vector Hermes' own guard names).
- **Run shapes covered**: both the prompt-turn scheduled path *and*
  the team-mission scheduled path — leaving either open would repeat
  this session's own recurring "asymmetric fix" failure mode, and
  both paths hold the same `schedule.*` capability today.
- **Trigger sources covered**: all 6 `TriggerSource` variants (Cron,
  Webhook, FileWatch, Reflection, Loop, Mission) — not just Cron. The
  same unattended-propagation risk applies regardless of which
  non-interactive mechanism fired the run.
- **Explicitly excluded**: channel-triggered missions
  (`start_from_goal_for_channel_trigger` — Telegram/Discord/Slack
  `/team run`). A real person typed the command and was authenticated
  via the existing sender-allowlist (Piece B's own follow-on chapter)
  — meaningfully different from a fully unattended trigger firing
  itself. Treated as operator-initiated for this guard's purposes.

## Architecture

Two mechanisms, one for each run shape, both converging on the same
kind of check at the tool level.

**Prompt-turn path.** Add a `message_origin: MessageOrigin` field to
`ToolContext` (`aivyx-core/src/lib.rs`), populated from the turn's own
`Message.origin` at the single `ToolContext { .. }` construction site
in `agent.rs`'s turn loop (currently ~line 1253). Because `Cron`,
`Webhook`, `FileWatch`, `Reflection`, and `Loop` already funnel
through the one `TriggerDispatch::fire()` function that already sets
`.system_originated()` on every message it sends, this single change
covers all 5 of those trigger sources with no further per-trigger-
source work — the signal already exists and is already correctly set
today, it just never reached `ToolContext`.

**Team-mission path.** Classify each mission's `triggered_by`
provenance at mission-assembly time (`assemble_runtime` or nearby) into
one of: interactive (`triggered_by: None`), trigger-originated (a
schedule ID or the `TriggerSource::Mission` shape), or channel-
originated (a channel tag, e.g. `"channel:telegram"` — excluded per
scope above). Thread that classification down into each specialist/
lead's own `ToolContext` during mission execution. The exact call-site
shape of how a specialist's `ToolContext` gets constructed today (does
it reuse `Agent::turn()`'s own single construction site, or does the
team-mission driver have a separate path?) needs to be re-confirmed
fresh during planning — this design states the intent and the
provenance signal to key off, not a line-by-line implementation,
consistent with how much of this investigation already had to correct
assumptions made without reading the real code first.

**The guard.** In each of the three tools' `execute()`, check the
new signal; if it indicates a covered non-interactive origin, refuse
with a clear, tool-appropriate error message in the same style this
file already uses for its other guardrails (e.g. the existing "the
autonomy level does not permit self-scheduling" / "the agent-created
schedule cap is reached" messages) — something like: `"schedule.
create: cannot be called from within a triggered/scheduled run (this
turn originated from <source>)"`.

## Testing

Mutation-proof tests for both paths: a prompt-turn test that
constructs a `System`-origin `ToolContext` and asserts
`ScheduleCreateTool::execute` refuses (and a companion test that an
`Operator`-origin context still succeeds, so the guard doesn't
over-block); an equivalent pair for the team-mission path using a
trigger-originated vs. interactive `triggered_by`. Each new test must
be shown to genuinely fail if the guard code is reverted — not merely
asserted to fail, per this session's own established mutation-proof
discipline.

## Out of scope

- The `ScheduleRecord.role_name`-is-effectively-unused finding — real,
  but a separate bug in a different part of the system, not part of
  closing the recursive-scheduling gap. Worth logging to
  `aivyx-ecosystem/ROADMAP.md`'s backlog once this piece ships.
- Lowering `MAX_AGENT_SCHEDULES` or otherwise tuning the existing
  count-cap/frequency-floor/growth-gradient mitigations — this design
  closes the gap at its root (no more recursive creation at all from
  covered origins) rather than tuning the existing backstops that
  were never meant to be the primary defense.
- Channel-triggered missions (explicitly excluded per Scope above).
- Any change to `TriggerSource`'s own definition, `MessageOrigin`'s
  own definition, or the audit/notify machinery around triggers —
  this piece consumes those existing types, it doesn't restructure
  them.
