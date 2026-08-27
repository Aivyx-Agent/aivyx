# `/classic` retirement (V09_PLAN row 7 / POLISH_WAVES.md sub-project 2) — design

**Status:** Approved, ready for planning.

## Motivation

`docs/POLISH_WAVES.md` decomposed the v0.9 polish-wave backlog into 7
(now 8) sequenced sub-projects, scoped directly against `docs/
VITRINE.md`'s real, dated operator-walkthrough findings rather than
re-summarized from memory. Sub-project 2, `/classic` retirement, is
next in that sequence. `VITRINE.md` §11's real inventory (not
`V09_PLAN.md`'s own stale "expect: 3 panes" guess) found four `/classic`
panes without a Studio equivalent: audit, sessions, notifications,
learning. Notifications already shipped (Chapter Herald, 2026-07-07).
This chapter closes the remaining three, plus TUI's own parallel
`Audit` gap, then performs the actual retirement.

## Research findings (grounded in the real, current code)

**`/classic` is one 2,879-line static file
(`aivyx-channel/src/web_ui_static.html`) serving 10 panes** (chat,
missions, audit, sessions, profile, persona, proposals, notifications,
memory, learning), embedded via `include_str!` and served at the
`/classic` route. It has a second, easy-to-miss role: `serve_static`'s
own doc comment says `/` serves "the bundle's `index.html` when built,
else the legacy chat page (the fallback)" — the *same* file. Retiring
`/classic` without replacing this fallback role would break every
no-bundle install.

**Audit** — the legacy pane's `loadAudit()` sends `{ kind:
'ListAuditEntries', from_seq, limit }` (`AUDIT_PAGE_SIZE`-sized pages;
the daemon caps `limit` server-side at 500 —
`AUDIT_QUERY_MAX_LIMIT` in `daemon_server.rs`) and a "Verify chain"
button sends `{ kind: 'VerifyAuditChain' }`. Both queries already exist
and are fully implemented server-side. This is a pure port.

**Sessions** — `loadSessions()` sends `{ kind: 'ListSessions' }`. The
real backend state behind it, `DaemonState.sessions: Vec<String>`
(`daemon_server.rs`), is genuinely bare — pushed at `StartSession`
(`st.sessions.push(sid.clone())`, ~line 2052) and removed on disconnect
(`st.sessions.retain(|s| s != sid)`, ~line 3502). `SessionSummary`
(the wire type) carries only `session_id`. No channel, trust tier, or
activity timestamp is tracked anywhere today. At `StartSession`, the
handler already has `frontend_type` and constructs the session's
`ChannelContext` (`channel_factory(ft)`) — channel platform and trust
tier are available for free at that exact point.

**Learning** — `loadLearning()` sends `{ kind: 'GetLearningInsights'
}`, a fully-built, already-complete query.

**TUI Audit/Tools** — `aivyx-tui/src/render.rs`'s `render_panel`
hardcodes both as `placeholder_lines(...)` strings; `View::Audit`'s own
comment says "live IPC data ... is the Phase 186 follow-on." `aivyx-tui`
has no query-dispatch wiring for either view today.

**TUI Tools has no backend counterpart at all** — confirmed via a
direct grep for `ListTools`/`GetToolRegistry`/similar across
`aivyx-channel`: nothing exists. Its own placeholder text ("provenance,
capability scope, and call stats") describes a feature that would need
a brand-new registry-enumeration query, and its call-stats half needs
the same audit-chain aggregation the MCP tool-level health signal
finding (`VITRINE.md` §10) also needs. Split out of this chapter into
a new, not-yet-designed `POLISH_WAVES.md` sub-project 8 rather than
building that mechanism twice or under-scoping it here.

**The proven read-only screen recipe** (`aivyx-web/src/main.rs`'s
`NotificationsPanel`, ~100 lines): a `use_context::<Signal<State>>()`
hydrated by a WS query response, a two-column `dash-grid` (`dash-main`
+ `dash-rail`) layout, per-item row components. Every new screen below
follows this shape.

## Scope

Decisions made explicitly with the user during brainstorming, not
assumed:

- **Sessions is enriched, not ported bare.** Add `channel:
  ChannelPlatform`, `trust_tier: TrustTier`, `created_at_ms: u64`, and
  `last_active_at_ms: u64` to a new `SessionRecord` replacing the bare
  `String` in `DaemonState.sessions`. The first three are free at
  `StartSession`; `last_active_at_ms` needs a new update at
  `SubmitInput` (or turn completion) — a real behavior change, not
  just a new screen.
- **Learning gets no new nav entry.** Folds into the existing Command
  Center screen as a new panel/card, per `VITRINE.md`'s own suggested
  candidate — not a 4th new Studio destination this chapter alone.
- **TUI Audit is in scope; TUI Tools is not.** Audit is a real port
  (same `ListAuditEntries` query the web screen uses, just rendered in
  ratatui). Tools has no existing query to port and needs the shared
  aggregator design that also serves the MCP health signal — both now
  live in sub-project 8, out of this chapter.
- **The actual retirement (deleting `/classic`'s panes, replacing the
  no-bundle fallback) is the last piece, gated on A/B/C/E all existing
  and being live-verified.** Not a blind delete — `/classic`'s
  fallback role must be replaced, not just removed.

## Architecture

### A. Web Audit screen

New `View::Audit` nav entry (Command group, alongside Missions/Chat).
New `AuditState { entries: Vec<AuditEntryView>, total_len: u64,
from_seq: u64 }` context. On mount and on prev/next, dispatch
`ListAuditEntries { from_seq, limit: AUDIT_PAGE_SIZE }` and update the
context from the response — same shape `NotificationsPanel` already
uses for its own history query. A "Verify chain" button dispatches the
existing `VerifyAuditChain` query and renders a pass/fail banner
(mirrors the legacy pane's `audit-verify-banner`).

### B. Web Sessions screen, enriched

**Backend** (`aivyx-channel/src/daemon_server.rs`):
- New `SessionRecord { session_id: String, channel: ChannelPlatform,
  trust_tier: TrustTier, created_at_ms: u64, last_active_at_ms: u64 }`
  replaces `DaemonState.sessions: Vec<String>` (becomes
  `Vec<SessionRecord>`, or a `HashMap<String, SessionRecord>` keyed by
  `session_id` if planning finds O(1) lookup genuinely needed at the
  `SubmitInput` update site — a call for the implementer to make with
  real evidence, not asserted here).
- At `StartSession`: capture `channel.platform()`/`channel.trust_tier()`
  from the already-constructed `ChannelContext`, plus
  `SystemTime::now()` for both timestamps at creation.
- At `SubmitInput` (or wherever a turn is known to have started/
  completed for that session): update `last_active_at_ms`.
- `SessionSummary` (the wire type) and `QueryResponsePayload::
  ListSessions`'s handler gain the same fields.
- Every existing `st.sessions` call site (push/retain/the
  `ListSessions` handler, plus the 4 test call sites already in
  `daemon_server.rs`) needs updating for the new element type — sized
  during planning against the real diff, not enumerated here.

**Frontend**: new `View::Sessions` screen, same recipe as A, listing
sessions newest-active-first with channel/trust-tier/age rendered per
row.

### C. Learning → Command Center

No new `View` variant. A new panel/card added to the existing Command
Center screen's component, hydrated via `GetLearningInsights` — same
data the legacy pane's `loadLearning()` already renders, moved into
the existing dashboard layout rather than a standalone screen.

### D. The actual retirement

Gated on A/B/C/E existing and being live-verified by the operator in a
real browser (this phase's established working mode — GUI rendering
can't be checked from this sandbox). Once verified:

- Delete the now-redundant panes from `web_ui_static.html` (all 10:
  chat/missions/profile/persona/proposals/memory are already covered
  by existing Studio screens per `VITRINE.md` §11's own accounting;
  audit/sessions/notifications/learning are covered once A/B/C ship).
- Replace `/`'s no-bundle fallback with a small, dedicated "the Studio
  bundle isn't built — see `just build-web`" page — not the multi-pane
  legacy app.
- Remove the `/classic` route from `serve_static` entirely; confirm it
  404s.

### E. TUI Audit view

`aivyx-tui` gains a real IPC client call for `ListAuditEntries` (the
same query A uses), replacing `View::Audit`'s hardcoded
`placeholder_lines(...)` in `render.rs`. New keybindings for
pagination (prev/next, mirroring the web screen's own), rendered as a
scrollable ratatui list matching the existing panel-block style
(`panel_block`, `palette::AMBER`/`DIM` conventions already used
elsewhere in `render.rs`). `aivyx-tui`'s existing event/model loop
(`event.rs`/`model.rs`) gets a new `Msg` variant + state field for the
fetched page, following the same pattern `View::Missions` already
uses for its own live IPC-fed data.

## Testing

- **B (backend)**: unit tests for `SessionRecord` capture at
  `StartSession` (channel/trust-tier/timestamps match what the
  constructed `ChannelContext` reports) and for `last_active_at_ms`
  updating on `SubmitInput` — mutation-tested (revert the update,
  confirm the test fails), matching this session's established
  discipline.
- **A/B/C (Dioxus)**: cannot be compiled in this sandbox (no `rustup`,
  no `wasm32-unknown-unknown` target, no GPU/live-serve) — static
  review only, then the operator verifies live in a browser and
  reports friction, per this phase's own established "Working mode."
  Any daemon-side logic the screens depend on (the queries themselves)
  is tested directly against the daemon, independent of the frontend.
- **D**: verify `/classic` 404s post-retirement and `/` serves the new
  fallback page when no bundle is built — both are plain HTTP checks
  against the daemon, no wasm needed.
- **E (TUI)**: `aivyx-tui` compiles and runs in this sandbox like any
  other crate — real tests against the new `Msg`/state wiring using
  the existing scripted-IPC test patterns already in `event.rs`/
  `model.rs`'s own test modules.

## Out of scope

- TUI Tools and the MCP tool-level health signal — split into
  `POLISH_WAVES.md` sub-project 8, not yet designed.
- Any change to the audit chain's own format, verification logic, or
  `ListAuditEntries`/`VerifyAuditChain`'s server-side implementation —
  this chapter consumes those existing queries, it doesn't change them.
- Settings-coverage expansion, MCP CRUD, Schedules screen, and the
  other config-write-surface items — `POLISH_WAVES.md` sub-project 7,
  a separate, later chapter.
- UI Modernization (dashboard restyle, graph-view cleanup) —
  `POLISH_WAVES.md` sub-project 6, sequenced after this one
  deliberately so new screens from this chapter get the new visual
  language once rather than being restyled twice.
- Any change to `aivyx-desktop` — out of scope for the whole v0.9
  polish-wave backlog per `V09_PLAN.md`.
