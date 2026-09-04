# Tap-Target Sizing Fix Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring `.icon-btn` and `.btn-xs` up to a 40px minimum tap target — globally for `.icon-btn` (4 sites, low risk), mobile-only for `.btn-xs` (48 sites, avoid a desktop density change) — and prove it with real, measured screenshots against a live daemon, not just CSS that "should" compute to 40px.

**Architecture:** Two small, adjacent CSS edits in `crates/aivyx-web/assets/stitch.css`, verified by reusing Phase 189's live-backend test environment and `capture.mjs` tool (both already on `main`), plus a new small measurement script that reads real `getBoundingClientRect()` values from the running app — the same real-measurement method Phase 189's final review used to establish the 22.5px/34×34px baseline this phase fixes.

**Tech Stack:** CSS (`stitch.css`), Node + Playwright (reusing the `scripts/mobile-verify/` tooling from Phase 189).

## Global Constraints

- Target size: **40px minimum** (Phase 189's own established "~40px" bug-bar threshold).
- `.icon-btn` (`crates/aivyx-web/assets/stitch.css:248-256`): bump **globally**, no media query. Currently `width: 34px; height: 34px`.
- `.btn-xs` (`crates/aivyx-web/assets/stitch.css:668`): bump **only** at `≤860px` (the app's existing mobile-shell breakpoint, `stitch.css:926`) via a new `@media (max-width: 860px)` block. Desktop (>860px) must be pixel-identical to today: `padding: 3px 8px; font-size: 11px`, unchanged.
- The web bundle is embedded in the `aivyx` binary at compile time from committed `crates/aivyx-web/dist/`. Any CSS fix requires `just build-web` run from the **main checkout** (`/home/julian/Projects/Rust/aivyx`), never from a worktree (building from a worktree bakes a worktree-specific absolute path into the emitted JS — see `docs/superpowers/artifacts/phase-189-mobile/README.md` for the full story), then `cargo build --bin aivyx`.
- Live-backend environment: same one Phase 189 built. If `curl -fsS http://127.0.0.1:7843/ | head -c 200` doesn't return HTML, follow `docs/superpowers/artifacts/phase-189-mobile/README.md`'s recovery steps (rig `llama-server` should already be running independently; only the local SSH tunnel and local daemon typically need restarting).
- Out of scope: any other CSS class, any desktop change to `.btn-xs`, "polish", any new breakpoint, re-auditing the rest of the Phase 189 baseline.

---

### Task 1: Bump both tap targets and verify with measured screenshots

**Files:**
- Modify: `crates/aivyx-web/assets/stitch.css:248-256` (`.icon-btn`)
- Modify: `crates/aivyx-web/assets/stitch.css:668` (`.btn-xs`)
- Create: `scripts/mobile-verify/measure.mjs`

**Interfaces:**
- Consumes: the live daemon at `http://127.0.0.1:7843` (Phase 189, Task 1); `scripts/mobile-verify/capture.mjs`'s existing screen-navigation convention (`.sidebar button.nav-item` selector, `button.nav-toggle` drawer handling below 860px) — `measure.mjs` reimplements a small `clickNav` helper matching that same pattern (already duplicated once in `seed.mjs`; a third small copy for a purpose-built tool is consistent with this directory's existing convention over sharing a module).
- Produces: nothing later tasks depend on — this is the only task in the plan.

- [ ] **Step 1: Confirm the live daemon is up**

```bash
curl -fsS http://127.0.0.1:7843/ | head -c 200
```

Expected: the start of an HTML document. If it fails, follow
`docs/superpowers/artifacts/phase-189-mobile/README.md`'s recovery steps
before continuing — do not proceed against a dead daemon.

- [ ] **Step 2: Note the "before" baseline**

Before touching any CSS, the starting point is already measured and
recorded: `.btn-xs` at **22.5px** tall and `.icon-btn` at **34×34px**,
both established via `getBoundingClientRect()` during Phase 189's final
review (`docs/archive/phases/PHASE_189.md`'s "Known follow-ups" section).
No need to re-measure before the fix — Step 9's 1280px measurement (which
should closely match 22.5px, since desktop `.btn-xs` is untouched) is
the cross-check that this baseline is still accurate.

- [ ] **Step 3: Edit `.icon-btn`**

In `crates/aivyx-web/assets/stitch.css`, find:

```css
.icon-btn {
  display: inline-flex; align-items: center; justify-content: center;
  width: 34px; height: 34px; border-radius: 8px;
  background: transparent; border: 1px solid var(--color-border-ghost);
  color: var(--color-text-secondary); cursor: pointer;
  transition: all 0.15s var(--ease-smooth);
}
```

Change `width: 34px; height: 34px;` to `width: 40px; height: 40px;`:

```css
.icon-btn {
  display: inline-flex; align-items: center; justify-content: center;
  width: 40px; height: 40px; border-radius: 8px;
  background: transparent; border: 1px solid var(--color-border-ghost);
  color: var(--color-text-secondary); cursor: pointer;
  transition: all 0.15s var(--ease-smooth);
}
```

Leave `.icon-btn:hover` and `.icon-btn svg` (the next two lines) untouched.

- [ ] **Step 4: Edit `.btn-xs`**

In `crates/aivyx-web/assets/stitch.css`, find the single line:

```css
.btn-xs { align-self: flex-start; padding: 3px 8px; font-size: 11px; }
```

Leave it exactly as-is (this is the desktop rule — must stay unchanged),
and add a new media-query block immediately after it:

```css
.btn-xs { align-self: flex-start; padding: 3px 8px; font-size: 11px; }
@media (max-width: 860px) {
  .btn-xs { min-height: 40px; display: inline-flex; align-items: center; }
}
.member-detail { display: flex; flex-direction: column; gap: 8px; }
```

(The last line, `.member-detail`, is the existing next rule — shown so
you can see exactly where the new block goes; don't duplicate it.)

- [ ] **Step 5: Rebuild the bundle from the main checkout**

```bash
export PATH="$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$HOME/.cargo/bin:$PATH"
cd /home/julian/Projects/Rust/aivyx
just build-web
```

Expected: exits 0, last line `bundle → crates/aivyx-web/dist/; rebuild the
daemon to embed it: cargo build -p aivyx-cli --bin aivyx --release`. If
`just` isn't found, install it first: `cargo install just --locked` (see
`docs/superpowers/artifacts/phase-189-mobile/README.md` for the full
context on why `~/.cargo/bin` needs to be on `PATH` here).

If you're working from a worktree rather than directly in the main
checkout, `rsync -a --delete` the resulting `crates/aivyx-web/dist/` into
your worktree's copy before committing there — never run `just build-web`
itself from inside a worktree (see the Global Constraints note above).

```bash
cargo build --bin aivyx
```

Expected: exits 0.

- [ ] **Step 6: Restart the daemon**

Stop the previous `aivyx daemon run --web-ui` process
(`pgrep -fa 'aivyx daemon run'` to find it), then relaunch it exactly as
`docs/superpowers/artifacts/phase-189-mobile/README.md`'s daemon-launch
step describes (same env vars, same `.dev-run-189/` working directory —
including its `aivyx.toml`, needed for the Loop/Settings screens, though
not touched by this task). Verify:

```bash
curl -fsS http://127.0.0.1:7843/ | head -c 200
```

Expected: HTML.

- [ ] **Step 7: Write `measure.mjs`**

```javascript
#!/usr/bin/env node
// scripts/mobile-verify/measure.mjs
//
// Phase 190 — measures the real rendered size of specific elements against
// a live aivyx daemon, to prove a CSS tap-target fix actually changed the
// rendered box rather than trusting that the CSS "should" compute to a
// given size. Companion to capture.mjs (screenshots) and seed.mjs (content
// seeding) from Phase 189 — same navigation convention, purpose-built tool.
//
// Usage:
//   node measure.mjs --base-url http://127.0.0.1:7843 --width 440 \
//     --screen schedules --selector ".btn-xs"

import { chromium } from 'playwright';

const SCREEN_LABELS = {
  command: 'Command',
  schedules: 'Schedules',
};

function parseArgs(argv) {
  const args = { baseUrl: 'http://127.0.0.1:7843', width: 1280, screen: null, selector: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--base-url') args.baseUrl = argv[++i];
    else if (a === '--width') args.width = Number(argv[++i]);
    else if (a === '--screen') args.screen = argv[++i];
    else if (a === '--selector') args.selector = argv[++i];
    else throw new Error(`unrecognized argument: ${a}`);
  }
  if (!args.screen || !args.selector) {
    throw new Error('--screen and --selector are required');
  }
  if (!SCREEN_LABELS[args.screen]) {
    throw new Error(`unknown --screen "${args.screen}" (known: ${Object.keys(SCREEN_LABELS).join(', ')})`);
  }
  return args;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const label = SCREEN_LABELS[args.screen];

  const browser = await chromium.launch({
    executablePath: '/usr/bin/google-chrome-stable',
    headless: true,
  });
  const page = await browser.newPage();
  await page.setViewportSize({ width: args.width, height: 900 });
  await page.goto(args.baseUrl, { waitUntil: 'networkidle' });
  await page.waitForTimeout(2000); // first WS snapshot

  // Same drawer-handling convention as capture.mjs/seed.mjs: below 860px
  // the sidebar is an off-canvas drawer opened via the hamburger.
  if (args.width <= 860) {
    await page.click('button.nav-toggle');
  }
  const escaped = label.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  await page
    .locator('.sidebar button.nav-item')
    .filter({ hasText: new RegExp(`^${escaped}$`) })
    .click();
  await page.waitForTimeout(600);

  const sizes = await page.$$eval(args.selector, (els) =>
    els.map((el) => {
      const r = el.getBoundingClientRect();
      return { text: el.textContent.trim().slice(0, 40), width: r.width, height: r.height };
    })
  );

  console.log(JSON.stringify(sizes, null, 2));
  await browser.close();
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
```

- [ ] **Step 8: Measure `.icon-btn` at a narrow width and at desktop**

```bash
cd scripts/mobile-verify
node measure.mjs --base-url http://127.0.0.1:7843 --width 440 --screen command --selector ".icon-btn"
node measure.mjs --base-url http://127.0.0.1:7843 --width 1280 --screen command --selector ".icon-btn"
```

Expected: both print a JSON array of **4** entries — the 3 topbar icons
(help, notifications bell, theme toggle) plus the drawer hamburger
(`.nav-toggle`, which is also `.icon-btn`; it's always in the DOM, just
CSS-hidden via `display: none` above 860px — `stitch.css:915`). At
**440px**, all 4 entries show `"width": 40, "height": 40` (box-sizing is
`border-box` globally — `crates/aivyx-web/assets/stitch.css:148` — so the
1px border is included in that 40, not added on top). At **1280px**, the
3 topbar icons show `"width": 40, "height": 40` and the hamburger entry
shows `"width": 0, "height": 0` (a `display: none` element's
`getBoundingClientRect()` is all-zero — that's the hamburger correctly
being hidden on desktop, not a bug).

- [ ] **Step 9: Measure `.btn-xs` at mobile widths and at desktop**

```bash
node measure.mjs --base-url http://127.0.0.1:7843 --width 440 --screen schedules --selector ".btn-xs"
node measure.mjs --base-url http://127.0.0.1:7843 --width 600 --screen schedules --selector ".btn-xs"
node measure.mjs --base-url http://127.0.0.1:7843 --width 860 --screen schedules --selector ".btn-xs"
node measure.mjs --base-url http://127.0.0.1:7843 --width 1280 --screen schedules --selector ".btn-xs"
cd ../..
```

Expected: at 440/600/860, every entry's `"height"` is `>= 40`. At 1280,
every entry's `"height"` is close to the original ~22.5px (the desktop
rule is untouched, so this should match Phase 189's originally-measured
baseline almost exactly — flag it if it doesn't, since that would mean
the media query leaked into desktop).

If any measurement doesn't match, don't guess why — read the actual
computed styles in a browser devtools sense (or re-check the CSS edit
against Step 3/4 verbatim) before proceeding.

- [ ] **Step 10: Capture screenshots as visual corroboration**

```bash
cd scripts/mobile-verify
node capture.mjs --base-url http://127.0.0.1:7843 \
  --out ../../docs/superpowers/artifacts/phase-189-mobile \
  --screens command,schedules --widths 1280,860,600,440
cd ../..
```

Expected: 8 `captured ...` lines. Read the 4 new `schedules-*.png` files
and the 4 new `command-*.png` files with your Read tool — confirm the
Edit/Delete/"Add reflection schedule"/"Create schedule" buttons visibly
grew at 440/600/860 versus 1280, and the topbar icons (help, bell, theme
toggle) look modestly larger at every width versus their pre-fix
appearance in the existing committed screenshots (diff `git log -p
docs/superpowers/artifacts/phase-189-mobile/command-1280.png` isn't
meaningful for a binary — just eyeball the size against what you already
measured numerically in Steps 8-9, this is corroboration, not the primary
proof).

- [ ] **Step 11: Regression guard**

```bash
just check-web
cargo test -p aivyx-web
```

Expected: both clean (0 errors, 0 failures — matches Phase 189's own
61-test baseline; this task shouldn't have changed that count since it
touches only CSS).

- [ ] **Step 12: Commit**

```bash
git add crates/aivyx-web/assets/stitch.css \
        crates/aivyx-web/dist/ \
        scripts/mobile-verify/measure.mjs \
        docs/superpowers/artifacts/phase-189-mobile/command-1280.png \
        docs/superpowers/artifacts/phase-189-mobile/command-440.png \
        docs/superpowers/artifacts/phase-189-mobile/command-600.png \
        docs/superpowers/artifacts/phase-189-mobile/command-860.png \
        docs/superpowers/artifacts/phase-189-mobile/schedules-1280.png \
        docs/superpowers/artifacts/phase-189-mobile/schedules-440.png \
        docs/superpowers/artifacts/phase-189-mobile/schedules-600.png \
        docs/superpowers/artifacts/phase-189-mobile/schedules-860.png
git commit -m "fix(web): bump .icon-btn/.btn-xs to a 40px tap target — Phase 190"
```

Expected: commit succeeds; `git status` is clean.
