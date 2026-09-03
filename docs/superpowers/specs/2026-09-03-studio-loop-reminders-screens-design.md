# Phase 188 — Studio Loop + Reminders screens — design

**Status:** Approved, ready for planning.

## Motivation

`docs/ROADMAP.md`'s Chapter I "Expected phases" list names Phase 188+ as:
"Dependency-free: bring the localhost SPA up to Chapter F–H (loop,
reminders, skills, connect, identity) + polish + mobile-responsive.
Broadens reach for non-terminal users."

**Grounding against the real code found a mixed picture, the same class
of staleness Phases 186/187 both hit — but only for two of the five
named items.** Direct inspection of `crates/aivyx-web/src/main.rs`'s
`View` enum and its screens:

- **skills, connect, identity are already fully or substantially done.**
  `View::Skills` (Chapter Repertoire) and `View::Mcp` (Chapter Lantern)
  are complete, shipped screens. `View::Agents` (Chapter V) already has
  a Profile editor (V.3) plus a Persona display
  (`EffectivePersonaSummary`) — covers "Profile vs. Persona/Soul" per
  `aivyx-website`'s own `identity.astro` framing. None of these are open
  work.
- **loop and reminders are genuinely missing.** Zero references to
  `LoopStatus` or any loop-related content anywhere in
  `crates/aivyx-web/src/main.rs`. Zero references to reminders either —
  and this one is freshly confirmed, not inferred: `GetReminders` was
  *built this session* as part of Phase 186 (for the TUI Dashboard) and
  is sitting completely unused by Studio, explicitly named as a known
  follow-up in `docs/archive/phases/PHASE_186.md`'s own closing notes.
- **The backend is asymmetric between the two.** Loop already has full
  read+write IPC — `QueryPayload::{LoopAdd, LoopList, LoopStart,
  LoopStop, LoopStatus, LoopLog, LoopSkip}` (`crates/aivyx-ipc/src/
  protocol.rs`), built for the CLI's own `aivyx loop` subcommands.
  Reminders has only `QueryPayload::GetReminders` (read-only), built for
  the TUI Dashboard — no set/cancel query exists for any frontend yet.

**"polish" and "mobile-responsive" are explicitly out of this spec's
scope** — the operator chose to scope only the two concretely-missing
screens now and defer the rest (see below). "polish" is too vague to
check against code without a specific example; a skim of
`docs/VITRINE.md` found mostly model-quality/agent-behavior findings,
not UI parity gaps, and what little is UI-shaped there looks already
absorbed by this session's own `docs/POLISH_WAVES.md` work.
"mobile-responsive" is confirmed genuinely partial (a real viewport
meta tag, two screens — Dashboard, Memory — with their own `@media`
breakpoints) but not systematic (no dedicated mobile nav despite a
stray CSS comment casually referencing one that doesn't actually
exist) — a different *kind* of work (a broad CSS/UX pass across 22
screens) than adding two new screens, deferred to its own future
scoping pass.

## A. Loop screen

New `View::Loop`, placed in the sidebar's existing "System" group
(alongside `Schedules`/`Notifications` — its closest existing
analogs). **Correction from an earlier draft of this spec**: this
group is a hardcoded `groups: Vec<NavGroup>` local variable in the
sidebar component, not a match on `View::ALL` with a "screens"
catch-all as first assumed — that catch-all turned out to belong to a
different, unrelated mechanism (the topbar help button's
`guide_page_for` lookup). Found and fixed during plan-writing.

**Status section** — one query, already shipped:

```rust
fn loop_status_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "loop-status".to_string(),
        payload: QueryPayload::LoopStatus,
    }
}
```

Response is `QueryResponsePayload::LoopStatus { state: LoopRunState,
remaining: usize, armed: bool, gate_enabled: bool, max_run_secs:
Option<u64>, max_run_tokens: Option<u64>, max_run_usd: Option<f64>,
max_idle_iterations: u32 }` (`crates/aivyx-ipc/src/protocol.rs`) —
identical shape to what the TUI Dashboard already renders (Phase 186),
so the same field interpretation applies: `state.active` for
running/idle, `state.iteration`/`state.max_iterations` for progress,
`state.spent_cents`/`state.tokens_used` for spend, `state.
consecutive_idle` for stall detection, `state.last_stop_reason` when
idle. Displayed as: active/idle badge, iteration progress, spend,
backlog remaining count, and — new to this screen, not in the TUI
Dashboard's compact summary — whether `[loop]` is armed at all (`armed:
false` means Start is meaningless; see below).

**Controls** — two new query helpers, both wired to already-shipped
backend queries:

```rust
fn loop_start_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "loop-start".to_string(),
        payload: QueryPayload::LoopStart { max_iterations: None },
    }
}

fn loop_stop_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "loop-stop".to_string(),
        payload: QueryPayload::LoopStop,
    }
}
```

Both respond with `QueryResponsePayload::LoopControl { ok: bool,
message: String }`. On `ok: false`, `message` renders as an inline
error banner on the screen (not silently swallowed) — matches this
crate's established pattern for surfacing a failed write (e.g. the MCP
screen's own `set_mcp_server_query`/`delete_mcp_server_query` error
handling). On `ok: true`, re-fetch `loop_status_query()` immediately so
the screen reflects the new state without waiting for any poll.

Button state: **Start** disabled when `!armed` (nothing configured to
start) or `state.active` (already running); **Stop** disabled when
`!state.active`. No confirmation dialog for either — matches Mission
Control's own abort/pause/resume buttons, which fire immediately.

**Explicitly not built**: backlog add/list/skip UI (`LoopAdd`/
`LoopList`/`LoopSkip` stay CLI/TUI-only) — deferred per the approved
scope, not a technical limitation (the backend already supports it).

## B. Reminders screen

New `View::Reminders`, same sidebar group.

**Read-only list** — one query, already shipped this session and
currently unused by any frontend:

```rust
fn reminders_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "reminders".to_string(),
        payload: QueryPayload::GetReminders,
    }
}
```

Response is `QueryResponsePayload::Reminders { reminders:
Vec<ReminderView> }` (`ReminderView { id, due_unix, message,
notify_targets, created_unix }`, already soonest-first from
`ReminderStore::list()` — no extra sort needed, same guarantee the TUI
Dashboard's own render code relies on). Displayed as one row per
reminder: a due-time offset formatted by a new, local function in
`aivyx-web` itself (`aivyx-tui` is a separate, non-wasm crate — nothing
is literally shared across the two). Re-implement the same
plain-integer-second-arithmetic approach `crates/aivyx-tui/src/
render.rs`'s `format_due_offset` already established this session
(no new date/time dependency in this wasm-clean crate either — same
constraint, same solution, independently written) plus the message
text, full list (not
truncated to 3 like the TUI Dashboard's compact summary — this is the
dedicated screen, not a summary panel).

**Empty state**: "no pending reminders" — matches the TUI Dashboard's
own convention for the same data.

**Explicitly not built**: set/cancel UI. No `CancelReminder` or
`SetReminder` query exists yet for any frontend, and none is added by
this plan — reminders stay agent-set via chat (natural-language time
resolution is the actual product shape per Phase 183's own design,
not a manual form), matching the approved scope decision. This is a
genuinely read-only screen, by design, not an interim state.

## Testing

Both screens get the same test treatment already established for
recent screens in this crate (e.g. `McpPanel`) — component-level
render assertions over fetched state (loading / empty / populated).
The Loop screen's button-disabled logic (`!armed || state.active` for
Start, `!state.active` for Stop) is real conditional logic and gets
its own focused tests independent of rendering, covering all four
`(armed, active)` combinations. The Reminders screen has no
comparable logic — its tests are render-only (empty list, one
reminder, multiple reminders in soonest-first order).

## Out of scope

- Loop backlog management (add/list/skip) in Studio.
- Reminder creation/cancellation in Studio (no backend query exists
  for either; not added here).
- "polish" (unscoped — no concrete findings identified against current
  code).
- Mobile-responsive design work (a separate, broader CSS/UX pass;
  confirmed genuinely partial but not addressed by this spec).
