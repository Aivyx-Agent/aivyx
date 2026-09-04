# Phase 189 — Studio Mobile-Responsive Verification Pass

**Chapter I, phase 5 — [SHIPPED] 2026-09-04.**

## Goal (carried from the roadmap entry)

Unlike Phases 186/187/188, Chapter I's "Expected phases" list had nothing
pre-named for Phase 189 — it ends at (now-shipped) Phase 188, followed only
by "subsequent phases are chosen at each exit." Phase 188's own
retrospective had named "mobile-responsive" as a deferred item, so that was
the natural candidate — but scoping it required grounding first, same
discipline as every prior phase this run.

## Where this actually stood going in

Grounding against the real code found the premise wrong. Phase 188's
retrospective described mobile-responsive as "confirmed genuinely
partial... no dedicated mobile nav despite a stray CSS comment casually
referencing one that doesn't actually exist." Direct inspection found a
complete, working mobile shell already shipped months earlier — commit
`689fcf793` (`feat(web): responsive layout — drawer sidebar + grouped nav`,
2026-06-23): an off-canvas drawer sidebar below 860px with a real
hamburger toggle and tap-to-dismiss backdrop, grouped nav, and three
responsive breakpoints (860/600/440px) already stacking every fixed grid in
the stylesheet.

The real, unclosed gap was verification, not implementation: that June
commit's own message admitted "browser-resize visual confirmation isn't
possible in-sandbox" — the CSS had only ever been confirmed present in the
built bundle, never actually watched render at a narrow width, by anyone,
in the ~2 months since. This reframed the phase from "build mobile support"
(mostly already built) to "prove the existing shell actually works, and fix
whatever's genuinely broken."

## What shipped

- **A real live-backend test environment**, not a mock: a disposable
  `aivyx` dev daemon backed by a real LLM (the GPU rig's `llama-server`
  over an SSH tunnel), driving real chat turns, a real completed mission,
  and real audit/notification activity — because CSS bugs at narrow
  widths are a function of real content length, not empty-state
  placeholders.
- **A Playwright capture tool** (`scripts/mobile-verify/capture.mjs`)
  driving the system's actual Chrome browser (no headless-browser mock),
  screenshotting all 23 sidebar-reachable screens (`View::Onboarding` is
  the 24th `View` variant but isn't sidebar-reachable once an agent
  profile exists) at 4 widths — 92 real screenshots, committed as
  artifacts under `docs/superpowers/artifacts/phase-189-mobile/`.
- **A visual audit** (`findings.md`) applying a fixed bug bar (horizontal
  overflow, clipped/overlapping content, sub-40px tap targets, two named
  known suspects) across all 92 screenshots, finding 3 real, narrow,
  fixable CSS bugs — not the sweeping redesign "mobile-responsive" sounded
  like going in.
- **3 confirmed findings, all fixed and re-verified with fresh
  screenshots:** a 5-tile stat row (`.stat-row-5`, Command Center) leaving
  its last tile orphaned alone in an incomplete row at narrow widths, now
  spanning the full row; a long file-path readout overflowing its card's
  right margin at 440px, now wrapping inside it; a `<select>` truncating
  its value mid-word with no ellipsis at 440px, now truncating cleanly.
- **Two real, load-bearing discoveries made and fixed mid-execution, not
  originally in the plan:** the throwaway dev daemon needed a minimal
  `aivyx.toml` to un-gate the Loop and Settings screens (otherwise stuck
  on "not configured"/an infinite loading skeleton — an environment
  config gap, not a CSS bug); and the capture tool needed a warm-up
  navigation sequence to avoid a real timing race against a genuine
  one-shot onboarding-redirect in the app itself, which could otherwise
  hijack the very first `command-*.png` capture.
- **A significant correction found by the final whole-branch review,
  fixed the harder and better way rather than caveated:** the capture
  tool's `fullPage: true` screenshots were silently viewport-clipped, not
  actually full-page — the app shell is a fixed `100vh` layout with an
  internal scrolling region, so every narrow-width screenshot only ever
  showed the above-the-fold ~800px. Fixed by stripping the shell's height
  constraint via inline style right before each screenshot (documented
  in-file, including two earlier approaches that were tried and failed),
  followed by a full 92-screenshot re-capture and a genuine re-audit of
  every screen whose content grew — which turned up zero new bugs, stated
  explicitly rather than left as a silent "probably fine."
- **A second finding from that same review, handled honestly rather than
  fixed:** the bug bar's tap-target criterion (~40px minimum) had never
  been explicitly recorded as checked. Two global classes (`.btn-xs` at
  ~22.5px, `.icon-btn` at 34×34px) measure under it — but they're shared
  with desktop, so resizing them would be a desktop-affecting change
  outside this phase's CSS-only scope. Documented as checked, real, and
  deliberately deferred, rather than silently passed over or wrongly
  "fixed" out of scope.

## The result

Chapter I's fifth phase closes with real, screenshotted proof that
Studio's mobile-responsive shell — built two months before this session
even started — actually works, plus 3 small, genuine bugs found and fixed
that no one had ever looked closely enough to see. This session's now
well-established pattern held on its first "from-scratch scoping" phase
too: the premise carried into the phase (from Phase 188's own retrospective)
was wrong, grounding against real code found the actual gap was narrower
and different in kind (verification, not construction) than assumed, and
the final whole-branch review still found something real (viewport
clipping) that every task-level review and the phase's own thorough
self-audit had missed.

## Known follow-ups (not done here, logged for whenever they matter)

- **Tap-target sizing on `.btn-xs`/`.icon-btn`** — real, measured, ~22.5px
  and 34×34px against a ~40px bar. Global classes shared with desktop; a
  real candidate for a future phase, not addressed here (out of this
  phase's CSS-only-mobile scope).
- **`docs/FRONTEND.md`'s Studio screen inventory table is stale** — noted
  again (first flagged in Phase 188's own follow-ups), still not this
  branch's responsibility, still worth a documentation pass eventually.
- **The capture tool's "expanded" screenshots trade one blind spot for a
  smaller one** — stripping the app shell's fixed height to capture full
  content means any element that relied on flex-fill sizing (e.g. Chat's
  composer, normally pinned to the bottom) instead sizes to its content in
  the capture. Harmless for this pass (documented in-file), but means the
  tool can't, by construction, catch a "flex-fill region overflows its
  real constrained height" class of bug — a different capture mode would
  be needed for that specific check, if it's ever wanted.
