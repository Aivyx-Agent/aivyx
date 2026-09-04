# Phase 190 — Tap-target sizing fix — design

**Status:** Approved, ready for planning.

## Motivation

Like Phase 189, Phase 190 had no pre-existing roadmap wording to ground
against — `docs/ROADMAP.md`'s Chapter I "Expected phases" list ends at
188, and 189 was documented in prose after that list rather than added to
it. This is again a from-scratch scoping decision.

The candidate came from Phase 189's own final whole-branch review
(`docs/archive/phases/PHASE_189.md`'s "Known follow-ups"), which found the
bug bar's tap-target criterion (~40px minimum at narrow widths) had a real
violation: two global CSS classes measure under it.

- **`.btn-xs`** (`crates/aivyx-web/assets/stitch.css:668` —
  `padding: 3px 8px; font-size: 11px`) measures **~22.5px tall**, verified
  via `getBoundingClientRect()` on the live daemon during Phase 189's
  review. Used **48 times** across `crates/aivyx-web/src/main.rs` on
  primary actions (e.g. "Create schedule," "Add reflection schedule,"
  Edit/Delete pairs — visible in `docs/superpowers/artifacts/
  phase-189-mobile/schedules-440.png`).
- **`.icon-btn`** (`stitch.css:248-256` — `width: 34px; height: 34px`)
  measures **34×34px**, used **4 times**: the mobile drawer's hamburger
  toggle (`main.rs:1528`, already `display: none` above 860px — see
  `stitch.css:915`) plus 3 persistent topbar icons (help, notifications
  bell, theme toggle — `main.rs:1539,1549,1568`) visible at every width.

Phase 189 deliberately left both unfixed: resizing global classes shared
with desktop is a desktop-affecting change, outside that phase's
CSS-only-mobile-verification scope. This phase picks that thread back up
directly.

## Approach

Both classes get bumped to a **40px minimum** — the same "~40px" threshold
Phase 189's own bug bar already established, not a new number. The two
classes get different treatment because their blast radius is different:

- **`.icon-btn` → global bump, no media query.** Only 4 usages, one of
  which (the hamburger) is already mobile-only by virtue of its own
  `display: none` rule above 860px. The other 3 are topbar icons — a
  small, contained surface. There's no real cost to a 40×40 icon button
  looking the same on desktop as on mobile, so this is a straightforward,
  low-risk accessibility upgrade applied everywhere: `width`/`height`
  `34px` → `40px`. The SVG icon itself stays `18px` (a modestly larger
  box around an unchanged icon is a safe visual change); `border-radius:
  8px` is left as-is.
- **`.btn-xs` → mobile-only, via the app's existing 860px breakpoint.**
  48 usages spread across many screens is a much bigger, less-reviewed
  surface — a global bump would meaningfully change desktop row density
  everywhere the class appears, which is exactly the kind of
  desktop-affecting change Phase 189 kept out of scope and this phase
  should too. Instead, add a `@media (max-width: 860px)` override —
  reusing the same breakpoint the app already uses to switch into its
  mobile shell (`stitch.css:926`, the off-canvas drawer), rather than
  inventing a new one. Desktop (>860px) keeps today's compact
  `padding: 3px 8px; font-size: 11px` untouched.

  Rather than reverse-engineering a `padding` value that happens to
  produce a 40px line box (fragile — depends on inherited line-height),
  the mobile override sets an explicit box: `min-height: 40px; display:
  inline-flex; align-items: center;` — matching `.icon-btn`'s own pattern
  of an explicit size with flex-centered content, which is robust
  regardless of font metrics. `padding`/`font-size` stay as they are
  (the flex box handles the height; the existing horizontal padding still
  applies).

## Verification

Reuse `scripts/mobile-verify/capture.mjs` (still on `main` from Phase
189 — no new tooling needed). Screenshot Schedules (the screen that
originally surfaced the `.btn-xs` finding) at 440/600/860px, and the
topbar (visible on every screen) at both 440px and 1280px for
`.icon-btn` — confirming the global bump lands identically at a narrow
width too, not just desktop. Then, against the live daemon, measure
the actual rendered box via `getBoundingClientRect()` — the same
real-measurement method Phase 189's final review used to establish the
22.5px/34×34px baseline in the first place. "The CSS should compute to
40px" is not sufficient evidence; a measured number is.

Also re-capture the same screens at 1280px (desktop) and confirm
`.btn-xs` there is **unchanged** from its pre-fix baseline (`padding: 3px
8px; font-size: 11px`, ~22.5px) — proving the media query didn't leak
into desktop.

## Testing

Same as Phase 189: this is CSS, not logic, so there's no unit test for
"is this button tall enough." `just check-web` and `cargo test -p
aivyx-web` run as a build/regression floor (confirming the CSS/markup
edit doesn't break the wasm build or any existing pure-function test);
the real proof is the measured screenshot evidence above.

## Out of scope

- Any CSS class other than `.btn-xs` and `.icon-btn`.
- Any desktop (>860px) change to `.btn-xs`.
- "Polish" (still unscoped — no concrete example identified).
- Any new mobile breakpoint (reuses the existing 860px one).
- Re-auditing the rest of the Phase 189 baseline for other tap-target
  violations beyond these two named classes — Phase 189's own audit
  already covered the full 23-screen sweep; this phase fixes the one
  named, measured gap it left open.
