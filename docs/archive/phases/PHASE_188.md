# Phase 188 — Studio Loop + Reminders Screens

**Chapter I, phase 4 — [SHIPPED] 2026-09-04.**

## Goal (carried from the roadmap entry)

"Dependency-free: bring the localhost SPA up to Chapter F–H (loop,
reminders, skills, connect, identity) + polish + mobile-responsive.
Broadens reach for non-terminal users."

## Where this actually stood going in

Grounding against the real code before scoping found a mixed picture —
3 of the 5 named features were already fully or substantially shipped:
`View::Skills` (Chapter Repertoire) and `View::Mcp` (Chapter Lantern)
were complete screens; `View::Agents` (Chapter V) already had a Profile
editor plus a Persona display, covering "identity." Only **loop** and
**reminders** were genuinely missing — and strikingly, both are the
exact same two gaps Phase 186 had just closed for the TUI Dashboard,
with the backend already built and proven from that same phase
(`LoopStatus` already existed for the CLI; `GetReminders` was built
during Phase 186 for the TUI and sat unused by Studio). "polish" and
"mobile-responsive" were explicitly scoped out — too vague/broad to
ground against code without more specific input, deferred to their own
future scoping pass rather than guessed at. Full design reasoning:
`docs/superpowers/specs/2026-09-03-studio-loop-reminders-screens-design.md`.

## What shipped

- **A Loop screen** — status (active/idle/stalled, iteration progress,
  spend, backlog remaining) plus Start/Stop controls, reusing the
  already-shipped `LoopStatus`/`LoopStart`/`LoopStop` queries built for
  the CLI. No backlog add/list/skip UI (deliberately deferred).
- **A Reminders screen** — a read-only pending-reminders list, reusing
  `GetReminders`. No set/cancel UI (reminders stay agent-set via chat,
  matching the actual product shape).
- **Zero new backend/IPC work** for either screen — this phase was
  entirely new frontend consumers of queries that already existed
  before it started.
- **A design-spec correction found and fixed during plan-writing,
  before any implementation**: the sidebar was assumed to be a match on
  `View::ALL` with a generic "screens" catch-all group — it's actually
  a hardcoded `groups: Vec<NavGroup>` local variable, and the catch-all
  that does exist belongs to a different, unrelated mechanism (the
  topbar help button's `guide_page_for` lookup). Corrected in both the
  spec and the plan before dispatching any implementer.
- **A compiler-enforced gap the plan's own brief didn't anticipate,
  found identically by both Task 1 and Task 2's implementers**: a 4th
  exhaustive `match` over `View` (a page-title match, separate from
  `slug`/`label`/render-dispatch) that Rust's own exhaustiveness check
  forced each to handle. Task 3's implementer then found and correctly
  investigated a related plan inaccuracy — a final-sweep grep-count
  check expected 3 sites, should have said 4 — confirmed via direct
  code reading (and a control check against a pre-existing view) rather
  than blind trust, ruling out a duplicate-arm bug.
- **Two real, non-obvious bugs the final whole-branch review caught,
  after all three tasks had already passed their own per-task review
  clean**: neither screen had any live-refresh (fetch-once-on-mount
  only, unlike every comparable screen and unlike the TUI Dashboard's
  own established polling precedent for these exact two queries); and
  Start/Stop never re-fetched loop status, so the success banner and
  the buttons' own disabled state could visibly contradict each other —
  this was literally required by the approved design spec and dropped
  during implementation, missed by Task 1's own per-task review too.
  Both fixed: each panel now polls on its own scoped `use_future` loop
  (cancelled automatically by Dioxus on unmount, so no explicit
  view-active check is needed), and the control buttons re-fetch status
  immediately after sending their command.
- **A regression the first fix wave introduced, caught by a second,
  independent re-review**: fixing a Minor finding (reminder rows using
  a form-field CSS class for read-only content) by switching to
  `glass-card` broke the actual layout — `.glass-card` has no
  `display:flex`/`grid`, so the due-offset and message text rendered as
  one unbroken, unseparated run. Fixed with an inline flex style,
  verified by extracting and grepping both the old and new committed
  `.wasm` blobs directly to prove the fix was genuinely present in one
  and absent from the other.
- **A sustained Opus server-side overload (4 consecutive dispatch
  failures) during the final review**, worked around by falling back to
  Sonnet for that one review with the user's explicit approval — Sonnet
  still caught both real Important findings above. Opus became
  available again in time for both fix-wave re-reviews, including the
  source-level verification (reading `dioxus-hooks`/`dioxus-core`
  directly to confirm `use_future`'s real unmount-cancellation
  behavior, and tracing the actual WS→IPC→daemon→`loop_driver` path to
  confirm message-ordering guarantees) that caught the layout
  regression above.

## The result

Chapter I's fourth phase closes with Studio's Loop and Reminders
screens live, closing the last concretely-scoped gap from the original
Phase 188 roadmap wording. This session's now well-established pattern
held again, twice over on one branch: a final review on the most
capable available model found real issues after every per-task review
had passed clean, and a re-review of the resulting fix wave found a
further, genuinely new regression that fix wave itself introduced —
reinforcing that "the fix passed its own tests" is not the same
guarantee as "an independent pass verified the fix didn't break
something else."

## Known follow-ups (not done here, logged for whenever they matter)

- **"polish" and "mobile-responsive" remain unscoped.** Neither was
  concretely grounded against code this session — "polish" because no
  specific example was identified, "mobile-responsive" because it's
  confirmed genuinely partial (a real viewport tag, two screens with
  their own `@media` breakpoints) but not systematic, and is a
  different *kind* of work (a broad CSS/UX pass across 22 screens) than
  this phase's two new-screen additions. Both are real candidates for a
  future phase, not closed by this one.
- **`docs/FRONTEND.md`'s Studio screen inventory table is stale** for a
  dozen already-shipped screens (Schedules, MCP, Tools, Gallery, Audit,
  Sessions, and now Loop/Reminders too) — confirmed pre-existing debt
  this branch didn't introduce and isn't on the hook for, but worth a
  documentation pass eventually.
- **A closer existing CSS class** (`glass-card mcp-card`, already used
  identically elsewhere for a keyed card inside an `mcp-grid`) would
  have been marginally more consistent than the inline flex style used
  to fix the layout regression — a cosmetic-only Minor the second
  re-review noted but didn't require fixing, given two fix waves had
  already landed on this branch.
