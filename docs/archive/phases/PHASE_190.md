# Phase 190 — Tap-Target Sizing Fix

**Chapter I, phase 6 — [SHIPPED] 2026-09-04.**

## Goal (carried from the roadmap entry)

Like Phase 189, Phase 190 had nothing pre-named to ground against —
Chapter I's "Expected phases" list still ended at 188. The candidate came
straight from Phase 189's own "Known follow-ups": a real, measured
tap-target violation (`.btn-xs` ~22.5px, `.icon-btn` 34×34px, both under
the ~40px mobile bug bar) found by that phase's final review but
deliberately left unfixed there, since both are global classes shared with
desktop and resizing them was outside that phase's CSS-only-mobile scope.

## Where this actually stood going in

Unusually for this run, there was nothing to correct here — Phase 189's
own measurements were exact and the fix was already fully scoped by the
time this phase started: bump both classes to 40px, `.icon-btn` globally
(only 4 usage sites, one already mobile-only), `.btn-xs` only at ≤860px
via a new media query (48 usage sites — a global change would be a real,
unreviewed desktop density change).

## What shipped

- **`.icon-btn`** (`crates/aivyx-web/assets/stitch.css:248-256`) bumped
  `34px`→`40px` on both `width` and `height`, globally, no media query.
- **`.btn-xs`** (`stitch.css:668`) gained a new, additive
  `@media (max-width: 860px)` block setting `min-height: 40px; display:
  inline-flex; align-items: center;` — the desktop rule stayed
  byte-for-byte unchanged.
- **A new measurement tool**, `scripts/mobile-verify/measure.mjs`
  (companion to Phase 189's `capture.mjs`/`seed.mjs`), reading real
  `getBoundingClientRect()` values off the live daemon — proving the fix
  numerically (40×40, height ≥40 at narrow widths, ~22.5px unchanged at
  desktop) rather than trusting the CSS "should" compute to 40px.
- **A real build-tooling bug found and fixed mid-task**: `just build-web`
  run a second time in the same session silently reused a stale
  `target/dx/aivyx-web/` cache, producing a wrong first measurement
  (34×34 instead of 40×40) with a clean exit code and no error. Caught
  because the implementer cross-checked the actual built CSS bytes rather
  than trusting the rebuild's exit status; fixed with a targeted
  `rm -rf target/dx/aivyx-web`. Documented in
  `docs/superpowers/artifacts/phase-189-mobile/README.md` so it doesn't
  cost a future phase the same debugging cycle — this is now the third
  build footgun that runbook records, alongside the worktree-path-leakage
  and non-content-derived-hash gotchas Phase 189 already found.
- **A real cross-session risk caught by the task reviewer and independently
  verified by the controller**: the implementer had temporarily copied the
  edited CSS into the shared main checkout (required by the rebuild
  discipline — `just build-web` must run there, never a worktree) and
  claimed to have restored it byte-identical afterward. The task reviewer
  correctly flagged this specific claim as unverifiable from its own
  worktree-isolated sandbox rather than either accepting or rejecting it
  blind; the controller then independently confirmed the main checkout's
  `git status` was genuinely clean before merging.

## The result

Chapter I's sixth phase closes a small, precisely-scoped gap Phase 189
found and measured but correctly declined to fix inline. The Opus final
review hit a session usage limit mid-run (not a transient overload) —
handled the same way this session has handled every prior Opus
unavailability: asked before falling back, proceeded on Sonnet with
explicit approval once given. That review still found one real, if minor,
gap (the stale-build-cache footgun going undocumented despite already
recurring once) and verified the shipped CSS independently at multiple
points — screenshot comparison against the pre-fix baseline, specificity
conflict checks across all 52 usage sites, and direct inspection of the
committed `dist/` bytes.

## Known follow-ups (not done here, logged for whenever they matter)

- **"Polish"** — still completely unscoped. Named in Phase 188's roadmap
  wording, never grounded against a concrete example since; would need
  the operator's own input on what they have in mind.
- **`docs/FRONTEND.md`'s Studio screen inventory table is stale** — noted
  again (third phase in a row to flag it), still nobody's actual
  responsibility yet.
