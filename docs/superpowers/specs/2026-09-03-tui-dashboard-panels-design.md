# Phase 186 — TUI Dashboard panels — design

**Status:** Approved, ready for planning.

## Motivation

`docs/archive/phases/PHASE_186.md` scaffolded this phase from `docs/
ROADMAP.md`'s Chapter I entry: "side/overlay views the IPC already serves —
mission, loop status, reminders, recent audit — so the agent's state is
visible in-terminal, not just chat." That wording predates Phase 185
actually shipping — Missions, Audit, and Tools are already real, IPC-fed
tabs in `crates/aivyx-tui` today, not placeholders. `View::Dashboard`
exists but its body (`dashboard_lines`, `crates/aivyx-tui/src/render.rs`)
is a literal stub: role/daemon/status/session-line-count, then placeholder
text naming this phase.

**Grounding against the real code found the remaining scope is uneven, not
uniform.** The roadmap wording implies four equal pieces; they aren't:

- **Loop status is nearly free.** `QueryPayload::LoopStatus` →
  `QueryResponsePayload::LoopStatus` already exists
  (`crates/aivyx-ipc/src/protocol.rs`) and returns a full `LoopRunState`
  (active, iteration/max_iterations, tokens_used, spent_cents,
  consecutive_idle, last_stop_reason) plus backlog remaining count, armed,
  and gate/budget config. This is the same "wire up an existing query"
  shape of work Audit and Tools already were.
- **Reminders has no frontend surface at all.** Phase 183 shipped a real
  `remind.*` feature (`ReminderStore`, `crates/aivyx-channel/src/
  reminder_store.rs`) with `set`/`list`/`cancel`/`due`, but it is
  **agent-tool-only** — `remind.list` is invoked by the agent during a
  turn via a lazily-set `OnceLock<SharedReminderStore>`
  (`crates/aivyx-channel/src/reminder_tool.rs`). There is no
  `QueryPayload` variant for it, and Studio (`aivyx-web`) has no reminders
  screen either. This is genuinely new backend work, not a wire-up — the
  same shape as `docs/POLISH_WAVES.md` sub-project 8's `GetMcpServerCallStats`
  addition (a new query/response pair + threading a new store handle into
  `daemon_server.rs`'s `handle_query`).
- **Missions and Audit summaries are free reuse, not new work.** Missions
  is polled continuously on a fixed cadence (`MISSION_POLL`) regardless of
  the active view (`crates/aivyx-tui/src/app.rs`'s `run_loop`), so
  `state.missions.rows` is always current — a Dashboard mini-summary reads
  it directly, no new fetch. Audit is fetch-on-switch only
  (`fetch_audit_page`); Dashboard's summary calls that same existing
  function from a new call site rather than adding a new query.

**Two stale comments found during this research, fixed as part of this
work:** `dashboard_lines`'s placeholder text, and `render_panel`'s doc
comment (`crates/aivyx-tui/src/render.rs`, just above the function) which
still claims "Tools' capability/call-stats detail remain the Phase 186
follow-on" — false since sub-project 8 shipped that. Both get corrected to
describe what was actually still open going into this phase (Dashboard
only).

## A. `GetReminders` query

New pair in `crates/aivyx-ipc/src/protocol.rs`:

```rust
QueryPayload::GetReminders,
QueryResponsePayload::Reminders { reminders: Vec<Reminder> },
```

Reuses the existing `Reminder` type (`id`, `due_unix`, `message`,
`notify_targets`, `created_unix`) from `crates/aivyx-channel/src/
reminder_store.rs` — already soonest-first from `ReminderStore::list()`, no
extra sort needed. No request parameters; always "all pending."

`daemon_server.rs`'s `handle_query` gains one new parameter,
`reminder_store: Option<&crate::reminder_tool::SharedReminderStore>`,
threaded from the same daemon-boot call site that already constructs
`reminder_store` today for the `remind.*` tools
(`crates/aivyx-cli/src/bin/aivyx.rs`, near where `remind_list_tool.set_store(...)`
is called). `None` → empty list, matching every other `Option<&...>` arm's
degrade-gracefully convention already established in that function.

## B. Loop status wire-up

No backend change. New TUI-side fetch function (`fetch_loop_status`,
mirroring `fetch_tool_stats`/`fetch_audit_page`'s existing shape) that
sends `QueryPayload::LoopStatus` and pushes the response into a new
`AppState` field (`loop_status: Option<LoopStatusView>`, wrapping the
existing wire fields — see model changes below).

## C. Dashboard content and layout

Keeps the existing role/daemon/status/session block (unchanged), then four
compact sections in `panel_block`'s existing visual style — summaries, not
full lists; the underlying detail stays on each feature's own tab:

- **Loop** — one of: `idle`, `running (iter N/max, $X.XX, K tokens)`, or
  `stalled (N consecutive idle)` (derived from `LoopRunState.active` +
  `consecutive_idle > 0`), plus `last_stop_reason` shown only when not
  currently active.
- **Reminders** — `"not yet fetched"`, `"N pending"`, or `"none pending"`
  (see the three-state breakdown below), then up to 3 next by `due_unix`,
  each as `due offset + truncated message` (e.g. `in 2h — call mom`). Time
  offset is plain integer-second arithmetic against the daemon's reported
  clock — no new date/time dependency, consistent with this crate not
  using one today.
- **Missions** — one line, counted by phase from `state.missions.rows`
  (e.g. `"2 active, 1 gated, 5 done"`).
- **Audit** — `state.audit_total` (if already known) or a value from a
  Dashboard-triggered page-0 fetch, plus up to 3 most recent entries'
  `event_type` (same fields the Audit tab itself already renders).

Each section renders `"—"` / a neutral empty state — never a blank gap or
an error, matching `dashboard_lines`'s existing degrade-gracefully style
for `role`/`daemon`. Critically, "hasn't loaded yet" and "the daemon has
none" are **not** the same state and must render distinctly wherever a
section's data can be fetched independently of whether it's actually
empty (final-review finding: the Reminders panel originally collapsed
both into `Vec::is_empty()`, so a never-fetched or errored-fetch state
was indistinguishable from a genuinely empty one):

- **Loop** already gets this right — `loop_status: Option<LoopStatusView>`
  renders `"idle — not yet fetched"` for `None`, distinct from a `Some`
  reporting an actually-idle loop.
- **Reminders** — `reminders: Option<Vec<ReminderView>>` renders
  `"not yet fetched"` for `None` (never fetched, or the last fetch
  errored), `"none pending"` for `Some(vec![])` (fetched, genuinely no
  pending reminders), and the count + soonest-3 rendering for
  `Some(non-empty)`.
- **Missions** and **Audit** don't need this distinction today: Missions
  is always live-fed (no fetch-pending window to represent), and Audit's
  `audit_total`/`audit_entries` are seeded synchronously before the first
  Dashboard draw is possible.

## D. Refresh mechanism

`Msg::SwitchView(View::Dashboard)` triggers one fetch each for loop-status,
reminders, and an audit page-0 — mirroring the existing `switching_to_audit`
/ `switching_to_tools` gates in `app.rs`. Missions needs no fetch (already
live).

`run_loop`'s existing tick (`tokio::select!` against `MISSION_POLL`, which
today unconditionally calls `poll_missions` every tick regardless of view)
gains one more conditional branch: when `state.view == View::Dashboard`,
the same tick also re-fetches loop-status and reminders. Audit is **not**
re-polled on tick — its Dashboard summary is a snapshot as of switching in,
consistent with the Audit tab's own static-until-paginate behavior. This
keeps loop/reminders visibly live while parked on Dashboard (e.g. watching
a running loop's iteration count climb) without adding a second timer or
polling anything while Dashboard isn't the active view.

## Testing

- `GetReminders` round-trip test in `daemon_server.rs` (mirrors
  `fold_mcp_server_stats`'s test style from sub-project 8): a populated
  `ReminderStore`, query, assert the response's `reminders` matches
  `ReminderStore::list()`'s own output; a `None` store degrades to an empty
  list.
- One pure-function render test per Dashboard section (loop idle / running
  / stalled; reminders empty / populated; missions summary; audit summary)
  in `crates/aivyx-tui/src/render.rs`'s existing test module, following
  `dashboard_lines`' current test style.
- An `app.rs` test confirming the tick only re-fetches loop-status/
  reminders when `state.view == View::Dashboard` — not on every tick
  regardless of view, which would silently waste queries whenever the
  operator is on Chat/Missions/Audit/Tools instead.

## Out of scope

- Any change to Missions/Audit/Tools themselves — Dashboard only reads
  their existing state or calls their existing fetch functions.
- A reminders screen in Studio (`aivyx-web`) — noted as a gap this research
  found, but it's a separate surface with its own scoping; not part of
  this phase.
- Any new date/time formatting dependency — offsets stay plain
  integer-second arithmetic.
