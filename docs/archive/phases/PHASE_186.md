# Phase 186 — TUI State Panels

**Chapter I, phase 2** — status: **open, design pending.**

Scaffolded from `docs/ROADMAP.md`'s Chapter I entry on 2026-09-03,
right after cutting `v0.9.0` (the Interface Polish phase capstone —
Phase 185's Terminal TUI foundation shipped as part of that release).
This file will be filled in and frozen at phase exit, per this
repo's own phase-journal convention (`docs/README.md`).

## Goal (carried from the roadmap entry)

Side/overlay views the IPC already serves — mission, loop status,
reminders, recent audit — so the agent's state is visible
in-terminal, not just chat. May have folded into 185 if scoped
tightly; it didn't (185 shipped mission/audit/tools as their own
tabs — see "Where this actually stands" below), so 186 stands on
its own.

## Where this actually stands going in

Phase 185 and this session's own `docs/POLISH_WAVES.md` sub-project
8 (tool/server call-stat observability) already shipped more of this
than the roadmap entry assumed when it was written:

- `View::Missions`, `View::Audit`, and `View::Tools` are real,
  IPC-fed tabs in `crates/aivyx-tui` today (`crates/aivyx-tui/src/model.rs`'s
  `View` enum) — not placeholders.
- `View::Dashboard` exists but its body
  (`dashboard_lines` in `crates/aivyx-tui/src/render.rs`) is a literal
  stub: role/daemon/status/session line count, then

  > "mission · loop · reminders · recent-audit panels land in Phase 186,
  > wired to the live daemon state the IPC already serves."

So the real remaining scope is narrower than the original roadmap
one-liner: an at-a-glance **Dashboard** view carrying **loop status**
and **reminders** — the two states with no TUI surface at all yet —
plus deciding whether Dashboard should also *summarize* (not
duplicate) Missions/Audit/Tools, or stay a distinct fourth thing.
That scoping question is for the brainstorming pass, not this
scaffold.

## Next step

Brainstorm the actual design (`superpowers:brainstorming`) before
any implementation — this file only records that the phase is open
and what it inherits from Phase 185 and the polish backlog.
