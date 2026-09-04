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
