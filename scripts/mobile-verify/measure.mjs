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
