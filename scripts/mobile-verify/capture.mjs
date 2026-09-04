#!/usr/bin/env node
// scripts/mobile-verify/capture.mjs
//
// Phase 189 — Studio mobile-responsive verification pass.
// Drives the system Chrome (no Playwright browser download — the daemon
// under test only needs a real rendering engine, not Playwright's own
// bundled one) against a live aivyx daemon's Studio web UI: clicking
// through the sidebar (opening the off-canvas drawer first below 860px,
// since every nav-item click closes it again) and screenshotting every
// reachable screen at each requested width.
//
// Usage:
//   node capture.mjs --base-url http://127.0.0.1:7843 \
//     --out ../../docs/superpowers/artifacts/phase-189-mobile \
//     [--screens command,chat,...] [--widths 1280,860,600,440]

import { chromium } from 'playwright';
import { mkdir } from 'node:fs/promises';
import path from 'node:path';

// slug (matches View::slug()) -> sidebar label (matches View::label()).
// View::Onboarding ("create") is deliberately excluded — it's not
// sidebar-reachable once an agent profile exists (see plan Task 2 notes).
const SCREENS = [
  ['command', 'Command'],
  ['chat', 'Chat'],
  ['missions', 'Missions'],
  ['mission-control', 'Mission Control'],
  ['schedules', 'Schedules'],
  ['memory', 'Memory'],
  ['wiki', 'Wiki'],
  ['graph', 'Graph'],
  ['agents', 'Agents'],
  ['skills', 'Skills'],
  ['teams', 'Teams'],
  ['documents', 'Documents'],
  ['audit', 'Audit'],
  ['sessions', 'Sessions'],
  ['gallery', 'Gallery'],
  ['notifications', 'Notifications'],
  ['loop', 'Loop'],
  ['reminders', 'Reminders'],
  ['mcp', 'MCP'],
  ['tools', 'Tools'],
  ['voice', 'Voice'],
  ['settings', 'Settings'],
  ['guide', 'Guide'],
];

function parseArgs(argv) {
  const args = {
    baseUrl: 'http://127.0.0.1:7843',
    out: '.',
    screens: null,
    widths: [1280, 860, 600, 440],
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === '--base-url') args.baseUrl = argv[++i];
    else if (a === '--out') args.out = argv[++i];
    else if (a === '--screens') args.screens = argv[++i].split(',');
    else if (a === '--widths') args.widths = argv[++i].split(',').map(Number);
    else throw new Error(`unrecognized argument: ${a}`);
  }
  return args;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const screens = args.screens
    ? SCREENS.filter(([slug]) => args.screens.includes(slug))
    : SCREENS;
  if (screens.length === 0) {
    throw new Error('no matching screens for --screens filter');
  }

  await mkdir(args.out, { recursive: true });

  const browser = await chromium.launch({
    executablePath: '/usr/bin/google-chrome-stable',
    headless: true,
  });
  const page = await browser.newPage();
  await page.emulateMedia({ reducedMotion: 'reduce' });

  // Click a sidebar nav item by its exact label, opening the off-canvas
  // drawer first below 860px (stitch.css:926-943) since every nav-item click
  // closes it again.
  const clickNav = async (label, width) => {
    if (width <= 860) {
      await page.click('button.nav-toggle');
    }
    const escaped = label.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    await page
      .locator('.sidebar button.nav-item')
      .filter({ hasText: new RegExp(`^${escaped}$`) })
      .click();
  };

  // Make the current screen's full content part of the *document* before
  // screenshotting, instead of guessing a viewport height. Why this is
  // needed at all: the app shell (`.app`, stitch.css) is a fixed
  // `height: 100vh` grid with an internal scrolling `.view` element, and at
  // <=860px the sidebar goes `position: fixed` (out of flow) — so the
  // *document* height is normally exactly the viewport height, no matter
  // how much content `.view` scrolls internally. `page.screenshot({
  // fullPage: true })` only screenshots the document, not an element's
  // internal scroller, so a realistic viewport height (e.g. 900) only ever
  // captured the above-the-fold slice of every screen and silently missed
  // below-the-fold clipping/overlap bugs.
  //
  // Two guess-a-number approaches were tried and both broke on real
  // content:
  //   1. A single oversized constant viewport height (4000px) — too short
  //      for the Tools screen (57 cards, several thousand px tall at
  //      440px), since screens vary too much in content length for one
  //      guessed number to cover all of them.
  //   2. Measuring `.view.scrollHeight` and resizing the viewport to fit —
  //      unreliable both ways: (a) if the viewport was still the oversized
  //      one a *previous* screen's resize left it at, `.view` (flex:1)
  //      stays stretched to that leftover size, and since `scrollHeight`
  //      for a non-overflowing box just equals `clientHeight`, short/empty
  //      screens (e.g. Reminders' "no pending reminders" right after Loop's
  //      long list) measured to nearly the *previous* screen's height
  //      instead of their own; (b) even measured from a small, freshly-reset
  //      baseline, `.view`'s `clientHeight` can itself already exceed
  //      `window.innerHeight` — CSS Grid's `1fr` row (`.app`'s
  //      `grid-template-rows: 1fr var(--status-height)`) has an implicit
  //      `min-height: auto` equal to its content's min-content size, so a
  //      tall-enough screen partially overflows `.app`'s own `100vh` even
  //      before any manual resizing, making "viewport height minus `.view`
  //      clientHeight" an unreliable stand-in for the surrounding chrome's
  //      real, constant size (it went negative in testing on the Tools
  //      screen) and silently undercounted the true content height by
  //      thousands of px.
  //
  // So: skip height arithmetic entirely. Directly strip the height
  // constraint and `overflow-y: auto` from `.view` (and let `.app` size to
  // its content instead of a fixed `100vh`) via inline style overrides right
  // before each screenshot. With nothing left to clip it, `.view`'s full
  // content becomes part of the normal document flow, `.app` (and the
  // document) grow to match, and `fullPage: true` captures everything with
  // no guessing. Do not replace this with a fixed or computed viewport
  // height again — both were tried and both silently clipped long screens.
  //
  // Tradeoff, not a free lunch: `.view` going `flex: none; height: auto`
  // means any child that relied on `.view` having a definite height now
  // sizes to its content instead of filling it — e.g. Chat's composer is
  // normally pinned to the bottom via flex-fill, but in an expanded capture
  // it just sits under a short transcript with empty space below. So this
  // method proves there's no *clipped* content, but by construction it
  // can't catch a "flex-fill region overflows its real constrained height"
  // bug — that class of bug needs a screenshot taken at the real, unexpanded
  // layout instead.
  const expandView = async () => {
    await page.evaluate(() => {
      const view = document.querySelector('.view');
      const app = document.querySelector('.app');
      view.style.overflow = 'visible';
      view.style.height = 'auto';
      view.style.flex = 'none';
      app.style.height = 'auto';
      app.style.minHeight = '100vh';
    });
    await page.waitForTimeout(80); // let the reflow settle before capture
  };

  for (const width of args.widths) {
    await page.setViewportSize({ width, height: 900 });
    await page.goto(args.baseUrl, { waitUntil: 'networkidle' });
    await page.waitForTimeout(2000); // first WS snapshot
    // Warm the Agent profile snapshot (main.rs AgentsPanel's `mc-agents-get`
    // query) before capturing anything: it's fetched lazily on that panel's
    // first mount, and it gates the sidebar's "Create" item (genesis_done in
    // main.rs Sidebar/CommandPalette). Left cold, every screen visited before
    // "Agents" in SCREENS order would show a different sidebar than every
    // screen visited after it — not a render race, a fetch that plain hasn't
    // happened yet. Visiting Agents once here (profile persists on
    // AgentsState for the rest of this page load) makes the sidebar identical
    // across all screens at this width.
    await clickNav('Agents', width);
    await page.waitForTimeout(600);
    // This daemon's throwaway profile is pre-genesis (assistant_name_source
    // != "toml"), which arms a one-shot redirect in main.rs (~line 1008-1023):
    // the *next* time the app is sitting on Command after that snapshot has
    // landed, it force-navigates to the Onboarding wizard once. Since Command
    // is the default view, that one-shot would otherwise hijack the real
    // `command-*.png` capture below. Deliberately spend it here — landing on
    // Onboarding is expected and thrown away — then return to Command a
    // second time, which is a no-op nav-wise and doesn't re-arm the redirect.
    await clickNav('Command', width);
    await page.waitForTimeout(400);
    await clickNav('Command', width);
    await page.waitForTimeout(400);
    for (const [slug, label] of screens) {
      await clickNav(label, width);
      await page.waitForTimeout(400); // re-render + any query round-trip
      await expandView();
      const file = path.join(args.out, `${slug}-${width}.png`);
      await page.screenshot({ path: file, fullPage: true });
      console.log(`captured ${file}`);
    }
  }

  await browser.close();
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
