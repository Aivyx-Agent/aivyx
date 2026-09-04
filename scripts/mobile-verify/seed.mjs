#!/usr/bin/env node
// scripts/mobile-verify/seed.mjs
//
// Phase 189 — drives real content into a live aivyx dev daemon through its
// own Studio UI, so the screenshot pass exercises real markup (a chat
// transcript, a reminder, a running mission) instead of only empty states.
// Run once against a fresh daemon before capture.mjs's baseline pass.
// Loop backlog is seeded separately via the CLI (see plan Task 3 Step 1) —
// it needs no LLM call, unlike everything here.
//
// Usage: node seed.mjs --base-url http://127.0.0.1:7843

import { chromium } from 'playwright';

function parseArgs(argv) {
  const args = { baseUrl: 'http://127.0.0.1:7843' };
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === '--base-url') args.baseUrl = argv[++i];
  }
  return args;
}

async function clickNav(page, label) {
  const escaped = label.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  await page
    .locator('.sidebar button.nav-item')
    .filter({ hasText: new RegExp(`^${escaped}$`) })
    .click();
}

async function main() {
  const { baseUrl } = parseArgs(process.argv.slice(2));
  const browser = await chromium.launch({
    executablePath: '/usr/bin/google-chrome-stable',
    headless: true,
  });
  const page = await browser.newPage();
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto(baseUrl, { waitUntil: 'networkidle' });
  await page.waitForTimeout(1000);

  // Chat: two real turns — the second asks the agent to set a reminder,
  // since Reminders is agent-set-via-chat only (no CLI path exists).
  await clickNav(page, 'Chat');
  const chatInput = page.getByLabel('Message');
  await chatInput.fill('What can you help me with? Answer in one short paragraph.');
  await chatInput.press('Enter');
  await page.waitForTimeout(25000); // real model turn
  await chatInput.fill('Remind me in 10 minutes to check the oven.');
  await chatInput.press('Enter');
  await page.waitForTimeout(25000);

  // Missions: start one real mission via the "start a new mission" bar.
  await clickNav(page, 'Missions');
  const missionInput = page.getByLabel('New mission goal');
  await missionInput.fill('List the top 3 available tools and what each is for.');
  await missionInput.press('Enter');
  await page.waitForTimeout(35000); // let the mission make real progress

  await browser.close();
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
