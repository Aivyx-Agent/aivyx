# Phase 186 — TUI Dashboard Panels

**Chapter I, phase 2 — [SHIPPED] 2026-09-03.**

## Goal (carried from the roadmap entry)

Side/overlay views the IPC already serves — mission, loop status,
reminders, recent audit — so the agent's state is visible
in-terminal, not just chat.

## Where this actually stood going in

Phase 185 and `docs/POLISH_WAVES.md` sub-project 8 (tool/server
call-stat observability, shipped the same day as this phase opened)
had already delivered more of the roadmap entry's scope than its own
wording assumed: `View::Missions`, `View::Audit`, and `View::Tools`
were already real, IPC-fed tabs in `crates/aivyx-tui`, not
placeholders. Only `View::Dashboard` remained a stub. Grounding the
brainstorm against the real code (not the roadmap's stale one-liner)
found the remaining scope was uneven, not four equal pieces: loop
status was a near-free wire-up (`QueryPayload::LoopStatus` already
existed); reminders had **zero** frontend IPC surface anywhere,
including Studio (`aivyx-web`) — agent-tool-only since Phase 183;
missions/audit summaries were free reuse of already-live state and
fetch functions. Full design reasoning:
`docs/superpowers/specs/2026-09-03-tui-dashboard-panels-design.md`.

## What shipped

- **`QueryPayload::GetReminders` / `QueryResponsePayload::Reminders`**
  (`crates/aivyx-ipc/src/protocol.rs`) — the first frontend-facing
  surface reminders has ever had. `ReminderView` is a wasm-clean
  field-for-field mirror of `aivyx-channel`'s internal `Reminder`
  type (not a shared type — `aivyx-ipc` cannot depend on
  `aivyx-channel`), converted by a plain function
  (`reminder_to_view`, not a `From` impl — the orphan rule blocks
  that direction since both types are foreign to `aivyx-channel`).
  `daemon_server.rs`'s `DaemonConfig`/`ConnectionContext`/
  `handle_query` all gained a new `reminder_store` field, threaded
  through every one of the 9 real construction/call sites in the
  codebase (verified exhaustively, twice, by two different
  reviewers independently grepping the whole tree).
- **A real Dashboard** (`dashboard_lines`, `crates/aivyx-tui/src/render.rs`):
  loop status (idle/running/stalled, derived from `LoopRunState`),
  next 3 reminders soonest-first, a mission phase count, and an
  audit total + 3 most recent entries — summaries, not full lists;
  the detail stays on each feature's own tab.
- **Dashboard scroll** — a mid-branch finding (not anticipated by
  the design spec) that Dashboard, alone among the panel views, had
  no scroll mechanism and could silently truncate on realistic
  terminal sizes. Fixed by reusing `audit_scroll_offset`'s existing
  pin-to-top pattern, with a `dashboard_scroll_max` clamp formula
  **cross-checked by a test against `dashboard_lines`' real output
  length** — specifically to avoid repeating the exact bug class
  `docs/POLISH_WAVES.md` sub-project 8 found in the Tools view's own
  scroll clamp (a hand-derived formula that silently drifted from
  the real render output). Both the final whole-branch review and
  its own re-review independently re-derived the line-count formula
  from `dashboard_lines`' actual push calls rather than trusting the
  implementer's arithmetic — the verification step that would have
  caught the original bug, done for real this time, twice.
- **A second real gap found by the final review**: `AppState.reminders`
  (a bare `Vec`) couldn't distinguish "never fetched / fetch failed"
  from "genuinely no pending reminders" — both rendered `"none
  pending"`, unlike the adjacent `loop_status: Option<LoopStatusView>`
  field's correct handling. Fixed to `Option<Vec<ReminderView>>`;
  also corrected the design spec's own wording, which had conflated
  the two states.

## The result

Chapter I's second phase closes with Dashboard no longer a stub —
every TUI panel view (Chat, Missions, Dashboard, Audit, Tools) now
shows live daemon state, and reminders gained their first frontend
surface anywhere in the product. Two genuine, non-obvious bugs
surfaced and were fixed inside this one phase, both caught by
independent review rather than by the implementer's own testing:
the scroll-clamp drift hazard (caught at the task-review gate) and
the fetched-vs-empty reminders ambiguity (caught at the final
whole-branch review gate) — reinforcing this session's now
well-established pattern that a final review on the most capable
model finds real issues even when every per-task review passed
clean.

## Known follow-ups (not done here, logged for whenever they matter)

- **Studio (`aivyx-web`) still has no reminders screen.** The new
  `GetReminders` query is free to reuse for one — explicitly out of
  this phase's scope, not forgotten.
- **`docs/DAEMON_IPC.md` has no `GetReminders` addendum.** Only 2 of
  87 `QueryPayload` variants are documented there today (the two
  this design explicitly modeled itself on: `GetToolStats`,
  `GetMcpServerCallStats`) — a weak convention, cheap to extend,
  deferred rather than blocking this phase.
- **`dashboard_scroll_max`'s prose doc comment** restates its
  formula in English, a second, test-unverified source of truth
  alongside the actual cross-checked formula — flagged, deliberately
  left as-is (deleting it would make the magic constant `15`
  unreviewable; the comment already names the test that guards it).
- **Reminder message text isn't truncated** in the render layer
  (the design spec says "truncated message") — ratatui clips at the
  panel edge with no ellipsis on a narrow terminal. Cosmetic only.
- **Dashboard's poll-tick cost**: parked on Dashboard, each
  `MISSION_POLL` tick now does three sequential daemon round trips
  (missions, loop status, reminders) instead of one, and key input
  waits for all three. Negligible at realistic reminder counts over
  a local socket; noted as a real but not currently worth fixing.
- **The `View::Tools`/`View::Audit` scroll clamps remain hand-derived**,
  guarded only indirectly by their own render tests — `Dashboard`'s
  new formula-plus-cross-check-test pattern is stronger and worth
  generalizing to the other two if either's clamp is ever touched
  again.
