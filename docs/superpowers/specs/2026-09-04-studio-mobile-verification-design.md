# Phase 189 — Studio mobile-responsive verification pass — design

**Status:** Approved, ready for planning.

## Motivation

`docs/ROADMAP.md`'s Chapter I "Expected phases" list ends at Phase 188 —
unlike Phases 186/187/188, there was no pre-existing roadmap wording naming
Phase 189. Phase 188's own retrospective (`docs/archive/phases/PHASE_188.md`)
named "mobile-responsive" as a deferred item, describing it as "confirmed
genuinely partial... no dedicated mobile nav despite a stray CSS comment
casually referencing one that doesn't actually exist."

**That description is wrong, and grounding against the real code this
session found why.** Commit `689fcf793` (`feat(web): responsive layout —
drawer sidebar + grouped nav`, 2026-06-23, on `main`) already shipped a
complete mobile shell:

- An off-canvas drawer sidebar below 860px, toggled by a real topbar
  hamburger, with a tap-to-dismiss backdrop (`crates/aivyx-web/assets/
  stitch.css:911–946`, `crates/aivyx-web/src/main.rs:895,1171,1343,1531`).
- Grouped, data-driven nav (Workspace/Knowledge/Agent/System).
- Three responsive breakpoints (860/600/440px) that already stack every
  fixed grid found in the stylesheet: `.app`, `.dash-grid`, `.mem`,
  `.field-row` (+ its `.agents`/`.proposal-card` variants), `.kv-grid`,
  `.guide`. The `auto-fill, minmax(...)` grids (`.skills-grid`, `.mcp-grid`,
  `.tools-grid`, `.gallery-grid`, `.mission-picker`) are responsive by
  construction and need no media query.
- A real `<meta name="viewport">` tag (via Dioxus's default HTML shell).
- The two newest screens (Loop, Reminders — added in Phase 188, months
  after this commit) are built entirely on primitives this shell already
  covers (`.field-row`, `.mcp-grid`, `.settings` flex column, `.panel-head`
  flex) — no new gap introduced there.

**The real, unclosed gap is verification, not implementation.** The June
commit's own message states: "Browser-resize visual confirmation isn't
possible in-sandbox" — i.e. the responsive CSS was only ever confirmed
*present in the built bundle*, never actually watched render at a narrow
width, in any browser, by anyone. Five months and roughly ten new screens
later, that remains true. Two small, concretely identified gaps surfaced by
direct inspection during this scoping pass:

- `.stat-row-5` (`stitch.css:1050-1051`) stacks 5→2 columns at 900px but
  never to 1 column, unlike `.stat-row`'s 4→2→1 progression.
- Rows combining `white-space: nowrap` segments with body text (e.g.
  `.audit-row .when`/`.seq`, `stitch.css:407-408`) have no overflow guard
  at very narrow widths — unverified whether this actually breaks anything
  in practice.

This phase is a **verification-and-fix pass**, not new responsive
infrastructure: watch all 24 screens actually render at real narrow
viewports, fix whatever's genuinely broken, and produce the visual proof
that was never captured the first time.

## Approach

A local disposable daemon (`./scripts/dev-run.sh` — local Ollama backend,
throwaway `.dev-run/` state, exactly what this script exists for) serves a
freshly-built Studio bundle. `npx playwright` (resolves locally to v1.62.1;
no fresh browser download needed — point it at the already-installed system
browser via an explicit `executablePath: '/usr/bin/google-chrome-stable'`,
since Playwright's `channel: 'chrome'` auto-detection looks for specific
known binary names and isn't guaranteed to find the `-stable`-suffixed
package on this machine) drives real navigation and screenshot capture.

There is no URL-based routing in `aivyx-web` (`View` is pure Dioxus-signal
state, no `dioxus_router`) — every screen is reached by clicking its sidebar
entry. Below 860px the sidebar is an off-canvas drawer, so the automation
must open it via the hamburger (`.nav-toggle`) before each click at narrow
widths, then let the tap-to-dismiss backdrop close it (matching real user
behavior) before screenshotting.

**Capture matrix:** 23 of the 24 `View::ALL` screens — every one except
`View::Onboarding`, which isn't reachable from the sidebar once an agent
profile exists (confirmed during plan-writing:
`crates/aivyx-web/src/main.rs:1358-1371`'s `agent_group` only includes the
"Create" entry when `!genesis_done`) — × {1280px desktop baseline, 860px,
600px, 440px} = 92 screenshots, saved to
`docs/superpowers/artifacts/phase-189-mobile/<slug>-<width>.png` (one
directory, one file per screen×breakpoint, named after `View::slug()` so
every artifact traces back to a specific screen and width — not a
free-floating claim).

Real, populated content matters more than empty states here: screens should
be exercised with actual data where reasonably easy to arrange (a mission in
progress, a few reminders, some audit entries) rather than only their empty
skeletons, since overflow/clipping bugs are a function of real content
length, not placeholder text.

## Bug bar — what's worth fixing

A screenshot is a **finding** if it shows:

- Horizontal page overflow — anything forcing the body to scroll sideways,
  or content wider than the viewport.
- Clipped or overlapping text/controls.
- A tap target smaller than ~40px at 440px width (buttons, nav items,
  close/dismiss controls).
- Either of the two known suspects actually manifesting (`.stat-row-5`
  staying 2-wide and looking cramped/broken at 440px; `.audit-row` nowrap
  segments crowding out or truncating the message).

**Not a finding:** a screen that is merely dense, tight, or requires
scrolling vertically — this pass fixes breakage, not aesthetics or
information density. No new features, no redesigning a screen's layout
concept, no touching desktop (≥861px) rendering except where a fix cannot
be scoped to a media query without one.

## Review process

Findings are recorded as a table (screen, breakpoint, screenshot filename,
issue, proposed fix) — not fixed silently. After fixes land, the same
screens/breakpoints are re-captured to the same artifact directory (fixed
files replace the originals; the before-state stays recoverable via git
history on the artifacts directory) so the "after" is also a real
screenshot, not a claim the diff looks right. This mirrors the standing
lesson from Phases 186–188: an independent pass that actually re-observes
the result catches what the fixer's own eyes miss.

## Testing

This is a CSS/markup verification pass, not new logic — there is no
meaningful unit test for "does this render correctly." Verification *is*
the screenshot capture itself: a clean run (96 baseline screenshots, then
96 post-fix screenshots for any screen that had a finding) is the pass/fail
signal, not `cargo test`. The existing `cargo test -p aivyx-web` /
`just check-web` suites still run as a regression guard (nothing in this
phase should touch Rust logic, only markup/CSS), but they cannot detect
what this phase exists to catch.

## Out of scope

- Any new responsive infrastructure (drawer, hamburger, breakpoint
  structure) — already built; this phase verifies and patches it.
- Desktop (≥861px) layout changes, except where unavoidable to fix a
  narrow-width bug.
- Screen redesigns, information-density changes, or new features.
- Tablet-specific intermediate breakpoints beyond the three that already
  exist (860/600/440) — not requested, no evidence they're needed.
- `docs/FRONTEND.md`'s stale screen-inventory table (a pre-existing,
  unrelated documentation debt item noted in Phase 188's retrospective) —
  a separate cleanup, not part of this verification pass.
