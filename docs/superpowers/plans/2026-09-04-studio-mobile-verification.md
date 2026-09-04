# Phase 189 — Studio Mobile-Responsive Verification Pass Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove — with real screenshots, not a claim — that Studio's already-built mobile-responsive shell (drawer sidebar, 3 breakpoints) actually renders correctly across all 23 sidebar-reachable screens, and fix whatever's genuinely broken.

**Architecture:** A live `aivyx` dev daemon (real LLM backend via the GPU rig's `llama-server`, tunneled over SSH) serves the Studio web UI on `127.0.0.1:7843`. Node + Playwright drives a real, already-installed system Chrome (`/usr/bin/google-chrome-stable`) to seed real content through the UI itself (chat, a mission, a reminder), then screenshots every reachable screen at 4 widths. A visual audit (reading the screenshots directly) produces a findings table against a fixed bug bar; confirmed findings get fixed in `stitch.css`/`main.rs`, the web bundle gets rebuilt, and the affected screens get re-captured to prove the fix.

**Tech Stack:** Rust/Dioxus (`aivyx-web`), the existing `stitch.css` responsive shell, Node.js + Playwright (driving system Chrome, no browser download), the `just build-web` bundle pipeline.

## Global Constraints

- Reachable screen set: all of `View::ALL` **except `View::Onboarding`** — 23 screens, reached only by clicking their sidebar `button.nav-item` (no URL routing exists). Exact slug/label pairs are listed in Task 2.
- Breakpoints: **1280px** (desktop baseline), **860px**, **600px**, **440px** — the three that already exist in `crates/aivyx-web/assets/stitch.css` plus one desktop baseline for comparison. No new breakpoints.
- Below **860px** the sidebar is an off-canvas drawer (`.app.nav-open`), opened only via `button.nav-toggle` (the hamburger) — every nav-item click closes it again (`crates/aivyx-web/src/main.rs:1424`), so the drawer must be re-opened before each click at those widths.
- Artifacts (screenshots + findings) live under `docs/superpowers/artifacts/phase-189-mobile/`, one PNG per `<slug>-<width>.png`, plus `findings.md` and `README.md` (the environment runbook).
- Bug bar — a screenshot is a **finding** only if it shows: horizontal page overflow, clipped/overlapping text or controls, a tap target under ~40px at 440px width, or either known suspect (`.stat-row-5` not stacking to 1 column at 440px; `.audit-row`'s `white-space: nowrap` segments crowding out the message). A screen that is merely dense or requires vertical scrolling is **not** a finding.
- Out of scope: new responsive infrastructure, desktop (≥861px) layout changes except where unavoidable to fix a narrow-width bug, screen redesigns, new breakpoints, `docs/FRONTEND.md` staleness (unrelated, pre-existing).
- The web bundle is embedded in the `aivyx` binary at compile time from the committed `crates/aivyx-web/dist/` — any CSS/markup fix requires `just build-web` (needs `dx` on `PATH`, e.g. `export PATH="$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$PATH"`) **then** `cargo build --bin aivyx` before it takes effect in a running daemon.
- GPU rig for the live LLM backend: `julian@10.80.80.148`, `llama-server` at `/usr/bin/llama-server`, model `/home/julian/models/Qwen3.5-9B-Q4_K_M.gguf`.

---

### Task 1: Stand up the live-backend dev environment

**Files:**
- Create: `docs/superpowers/artifacts/phase-189-mobile/README.md`
- Create: `docs/superpowers/artifacts/phase-189-mobile/_smoke-command-1280.png` (binary, produced by a command below — do not hand-author)

**Interfaces:**
- Produces: a running `aivyx` daemon reachable at `http://127.0.0.1:7843`, backed by a real LLM, that Tasks 2–5 depend on staying up. If it's down when a later task starts, that task's implementer re-runs the commands in `README.md` to bring it back up before proceeding.

- [ ] **Step 1: Start `llama-server` on the GPU rig**

Run this on the rig over SSH, backgrounded so it survives the SSH session:

```bash
ssh julian@10.80.80.148 \
  'nohup /usr/bin/llama-server --model /home/julian/models/Qwen3.5-9B-Q4_K_M.gguf \
     --host 127.0.0.1 --port 8080 --parallel 1 --ctx-size 8192 -ngl 99 \
     > /tmp/llama-server-phase189.log 2>&1 < /dev/null &'
```

Verify it's actually listening before moving on:

```bash
ssh julian@10.80.80.148 'for i in $(seq 1 30); do curl -fsS http://127.0.0.1:8080/health && exit 0; sleep 2; done; echo TIMEOUT; tail -40 /tmp/llama-server-phase189.log; exit 1'
```

Expected: prints `{"status":"ok"}` (or similar JSON with `"status"`) before `TIMEOUT`. If it times out, read the tailed log for the real error (wrong model path, port in use) and fix before continuing — do not proceed on a guess.

- [ ] **Step 2: Tunnel the rig's port to this machine**

Run this with your Bash tool's background option (`run_in_background: true`) so it keeps running after this step returns — Tasks 2–5 need it alive:

```bash
ssh -N -L 8080:127.0.0.1:8080 julian@10.80.80.148
```

Verify the tunnel works from this machine:

```bash
curl -fsS http://127.0.0.1:8080/health
```

Expected: same JSON as Step 1's check, now reachable locally.

- [ ] **Step 3: Build the Studio web bundle and the `aivyx` binary**

```bash
export PATH="$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$PATH"
cd /home/julian/Projects/Rust/aivyx
just build-web
cargo build --bin aivyx
```

Expected: both commands exit 0. `just build-web`'s last line is `bundle → crates/aivyx-web/dist/; rebuild the daemon to embed it:`.

- [ ] **Step 4: Launch the dev daemon against the tunneled backend**

```bash
mkdir -p .dev-run-189/sandbox
cd .dev-run-189
```

Run this with your Bash tool's background option (`run_in_background: true`):

```bash
env AIVYX_PROVIDER=llamacpp \
    AIVYX_OPENAI_BASE_URL=http://127.0.0.1:8080 \
    AIVYX_MODEL=qwen3.5-9b-q4_k_m \
    AIVYX_FS_ROOT="$PWD/sandbox" \
    AIVYX_STORAGE_PATH="$PWD/store.redb" \
    AIVYX_PASSPHRASE=aivyx-dev-throwaway \
    ../target/debug/aivyx daemon run --web-ui
```

Then, from the repo root, verify:

```bash
cd /home/julian/Projects/Rust/aivyx
curl -fsS http://127.0.0.1:7843/ | head -c 200
```

Expected: the start of an HTML document (Dioxus's `index.html` shell — look for `<!doctype html>` or `<html`). If it prints nothing or connection-refused, the daemon didn't start — check whatever the background command's output shows for the real error (a common one: the store already has a passphrase that doesn't match `AIVYX_PASSPHRASE` from a previous run — fix with `rm -rf .dev-run-189` and repeat Step 4 from a clean directory).

- [ ] **Step 5: Smoke-test with a real screenshot**

```bash
mkdir -p docs/superpowers/artifacts/phase-189-mobile
cd /tmp && npm init -y >/dev/null 2>&1 || true
PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm install --no-save playwright@1.62.1 --prefix /tmp/pw-smoke
node -e '
const { chromium } = require("/tmp/pw-smoke/node_modules/playwright");
(async () => {
  const browser = await chromium.launch({ executablePath: "/usr/bin/google-chrome-stable", headless: true });
  const page = await browser.newPage();
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.goto("http://127.0.0.1:7843", { waitUntil: "networkidle" });
  await page.waitForTimeout(1000);
  await page.screenshot({ path: "/home/julian/Projects/Rust/aivyx/docs/superpowers/artifacts/phase-189-mobile/_smoke-command-1280.png", fullPage: true });
  await browser.close();
})();
'
```

Expected: exits 0, and `docs/superpowers/artifacts/phase-189-mobile/_smoke-command-1280.png` exists and is larger than 10KB (`ls -la` it — a near-empty file means the page didn't actually render, e.g. a JS error or the WS never connected).

- [ ] **Step 6: Write the runbook and commit**

Create `docs/superpowers/artifacts/phase-189-mobile/README.md`:

```markdown
# Phase 189 mobile-verification environment runbook

If the dev daemon isn't reachable at `http://127.0.0.1:7843`, bring it back
up with these steps (see `docs/superpowers/plans/2026-09-04-studio-mobile-verification.md`
Task 1 for full detail and expected output at each step):

1. Start `llama-server` on the rig (if not already running):
   `ssh julian@10.80.80.148 'nohup /usr/bin/llama-server --model /home/julian/models/Qwen3.5-9B-Q4_K_M.gguf --host 127.0.0.1 --port 8080 --parallel 1 --ctx-size 8192 -ngl 99 > /tmp/llama-server-phase189.log 2>&1 < /dev/null &'`
2. Tunnel it locally (background): `ssh -N -L 8080:127.0.0.1:8080 julian@10.80.80.148`
3. Rebuild if CSS/markup changed: `just build-web && cargo build --bin aivyx`
   (needs `dx` on `PATH`: `export PATH="$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$PATH"`)
4. Launch the daemon (background, from `.dev-run-189/`):
   `env AIVYX_PROVIDER=llamacpp AIVYX_OPENAI_BASE_URL=http://127.0.0.1:8080 AIVYX_MODEL=qwen3.5-9b-q4_k_m AIVYX_FS_ROOT="$PWD/sandbox" AIVYX_STORAGE_PATH="$PWD/store.redb" AIVYX_PASSPHRASE=aivyx-dev-throwaway ../target/debug/aivyx daemon run --web-ui`
5. Verify: `curl -fsS http://127.0.0.1:7843/ | head -c 200` should print HTML.

`.dev-run-189/` is disposable scratch state (git-ignored), same convention
as `.dev-run/` from `scripts/dev-run.sh` — safe to `rm -rf` and restart from
Step 4 if the store gets into a bad state.
```

```bash
echo ".dev-run-189/" >> .gitignore
git add docs/superpowers/artifacts/phase-189-mobile/README.md \
        docs/superpowers/artifacts/phase-189-mobile/_smoke-command-1280.png \
        .gitignore
git commit -m "docs(phase-189): live-backend dev environment runbook + smoke screenshot"
```

Expected: commit succeeds; `git status` is clean.

---

### Task 2: Playwright capture tool

**Files:**
- Create: `scripts/mobile-verify/package.json`
- Create: `scripts/mobile-verify/.gitignore`
- Create: `scripts/mobile-verify/capture.mjs`

**Interfaces:**
- Consumes: the live daemon from Task 1 at `http://127.0.0.1:7843` (verify it's still up first with `curl -fsS http://127.0.0.1:7843/ | head -c 200`; if not, follow `docs/superpowers/artifacts/phase-189-mobile/README.md`).
- Produces: a CLI, `node capture.mjs --base-url <url> --out <dir> [--screens slug1,slug2,...] [--widths w1,w2,...]`, that screenshots each requested screen at each requested width to `<out>/<slug>-<width>.png`. Task 3 runs it with no `--screens` filter (all 23) and the default widths; Task 5 re-runs it scoped to just the fixed screens.

- [ ] **Step 1: Set up the script's own dependency**

```bash
mkdir -p scripts/mobile-verify
cat > scripts/mobile-verify/package.json <<'EOF'
{
  "name": "aivyx-mobile-verify",
  "private": true,
  "type": "module",
  "devDependencies": {
    "playwright": "1.62.1"
  }
}
EOF
cat > scripts/mobile-verify/.gitignore <<'EOF'
node_modules/
EOF
cd scripts/mobile-verify
PLAYWRIGHT_SKIP_BROWSER_DOWNLOAD=1 npm install
cd ../..
```

Expected: `npm install` exits 0 and creates `scripts/mobile-verify/node_modules/playwright`.

- [ ] **Step 2: Write `capture.mjs`**

```javascript
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

  for (const width of args.widths) {
    await page.setViewportSize({ width, height: 900 });
    await page.goto(args.baseUrl, { waitUntil: 'networkidle' });
    await page.waitForTimeout(500); // first WS snapshot
    for (const [slug, label] of screens) {
      if (width <= 860) {
        // Off-canvas drawer below 860px (stitch.css:926-943).
        await page.click('button.nav-toggle');
      }
      const escaped = label.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
      await page
        .locator('.sidebar button.nav-item')
        .filter({ hasText: new RegExp(`^${escaped}$`) })
        .click();
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
```

- [ ] **Step 3: Verify it runs end to end against Task 1's live daemon**

```bash
curl -fsS http://127.0.0.1:7843/ | head -c 200   # confirm daemon still up; see README.md if not
cd scripts/mobile-verify
mkdir -p /tmp/capture-smoke
node capture.mjs --base-url http://127.0.0.1:7843 --out /tmp/capture-smoke \
  --screens command,loop --widths 1280,440
ls -la /tmp/capture-smoke
cd ../..
```

Expected: prints `captured .../command-1280.png`, `captured .../loop-1280.png`, `captured .../command-440.png`, `captured .../loop-440.png` (4 lines); `ls -la` shows all 4 files, each larger than 5KB.

- [ ] **Step 4: Commit**

```bash
git add scripts/mobile-verify/package.json scripts/mobile-verify/.gitignore \
        scripts/mobile-verify/capture.mjs
git commit -m "test(phase-189): Playwright capture tool for the mobile-responsive pass"
```

Note: `package-lock.json` is intentionally not committed here — `npm install` is re-run fresh by whoever needs the tool next, matching this being throwaway verification tooling, not a shipped dependency.

---

### Task 3: Seed real content and capture the baseline

**Files:**
- Create: `scripts/mobile-verify/seed.mjs`
- Create: `docs/superpowers/artifacts/phase-189-mobile/*.png` (92 files: 23 screens × 4 widths, produced by a command below — do not hand-author)

**Interfaces:**
- Consumes: `scripts/mobile-verify/capture.mjs` from Task 2 (same CLI contract); the live daemon from Task 1.
- Produces: 92 baseline screenshots under `docs/superpowers/artifacts/phase-189-mobile/` that Task 4 reads for the visual audit.

- [ ] **Step 1: Seed the Loop backlog (no LLM call needed)**

```bash
cd /home/julian/Projects/Rust/aivyx/.dev-run-189
env AIVYX_STORAGE_PATH="$PWD/store.redb" AIVYX_PASSPHRASE=aivyx-dev-throwaway \
  ../target/debug/aivyx loop add "Audit the README for broken links" \
  "Check every internal doc link still resolves" --priority 1
env AIVYX_STORAGE_PATH="$PWD/store.redb" AIVYX_PASSPHRASE=aivyx-dev-throwaway \
  ../target/debug/aivyx loop add "Summarize open TODOs" \
  "Grep the workspace for TODO/FIXME and summarize" --priority 2
cd ../..
```

Expected: both commands print a confirmation (a story ID or "added"); no error.

- [ ] **Step 2: Write `seed.mjs`**

```javascript
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
```

- [ ] **Step 3: Run the seed pass**

```bash
curl -fsS http://127.0.0.1:7843/ | head -c 200   # confirm daemon still up
cd scripts/mobile-verify
node seed.mjs --base-url http://127.0.0.1:7843
cd ../..
```

Expected: exits 0 with no error (it prints nothing on success — the `main().catch` only fires on failure). Takes roughly 90 seconds (the `waitForTimeout` calls).

- [ ] **Step 4: Confirm the seed actually landed**

```bash
curl -fsS http://127.0.0.1:7843/ | head -c 200   # still up
```

Then, as a spot-check, run one capture of the Chat screen and read it:

```bash
cd scripts/mobile-verify
node capture.mjs --base-url http://127.0.0.1:7843 \
  --out /tmp/seed-check --screens chat,missions,reminders --widths 1280
cd ../..
```

Read `/tmp/seed-check/chat-1280.png`, `/tmp/seed-check/missions-1280.png`, and `/tmp/seed-check/reminders-1280.png` with your Read tool. Expected: Chat shows two operator messages and at least one agent reply (not an empty composer); Missions shows one mission entry (not "no missions yet"); Reminders shows at least one pending reminder (not "no pending reminders"). If any of these still look empty, wait another 30 seconds and re-run this step once — a slow first model load on the rig can push the real response past the `seed.mjs` timeouts. If still empty after that, note it plainly in Task 4's findings as an accepted limitation (not a defect) rather than silently proceeding as if it worked.

- [ ] **Step 5: Run the full baseline capture — all 23 screens, all 4 widths**

```bash
curl -fsS http://127.0.0.1:7843/ | head -c 200   # confirm daemon still up
cd scripts/mobile-verify
node capture.mjs --base-url http://127.0.0.1:7843 \
  --out ../../docs/superpowers/artifacts/phase-189-mobile
cd ../..
ls docs/superpowers/artifacts/phase-189-mobile/*.png | wc -l
```

Expected: 92 `captured ...` lines printed (23 screens × 4 widths), plus the smoke screenshot from Task 1 — `wc -l` reports `93`.

- [ ] **Step 6: Commit**

```bash
git add scripts/mobile-verify/seed.mjs \
        docs/superpowers/artifacts/phase-189-mobile/*.png
git commit -m "test(phase-189): seed real content + capture the mobile baseline (92 screenshots)"
```

---

### Task 4: Visual audit

**Files:**
- Create: `docs/superpowers/artifacts/phase-189-mobile/findings.md`

**Interfaces:**
- Consumes: the 92 PNGs from Task 3.
- Produces: `findings.md`, a table Task 5 works from.

- [ ] **Step 1: Read every screenshot and apply the bug bar**

For each of the 92 files in `docs/superpowers/artifacts/phase-189-mobile/` (pattern `<slug>-<width>.png`), use your Read tool to view it. For each, check against the Global Constraints' bug bar exactly:

- Horizontal page overflow (content or a scrollbar extending past the viewport edge).
- Clipped or overlapping text/controls.
- A tap target that looks smaller than ~40px at 440px width.
- The two known suspects: does `.stat-row-5` (visible on any screen showing 5 stats side by side) still show 2 columns — cramped or with a stray incomplete row — at 440px? Does any row mixing a fixed-width timestamp/id with body text (e.g. Audit's rows) crowd out or truncate the message at 440px?

It is fastest and most reliable to compare each screen's 1280px shot against its 860/600/440px shots side by side conceptually — the desktop shot tells you what "correct" looks like for that screen's content, so a narrow-width regression is easy to spot by contrast.

- [ ] **Step 2: Write `findings.md`**

Use this exact structure (fill in your real findings — the example rows below are illustrative only, not a prediction of what you'll find, and must not be copied verbatim):

```markdown
# Phase 189 — mobile verification findings

Audited: 92 screenshots (23 screens × 4 widths: 1280/860/600/440) in this directory.

| Screen | Width | Issue | Screenshot | Proposed fix |
|---|---|---|---|---|
| loop | 440 | `.stat-row-5`-based stat block stays at 2 columns and the 5th tile clips against the right edge | `loop-440.png` | Add a 440px stacking rule for `.stat-row-5` matching `.stat-row`'s 1-column fallback |
| audit | 440 | `.audit-row`'s timestamp + seq crowd the message onto one truncated line | `audit-440.png` | Add a wrap rule for `.audit-row` at ≤600px |

## Screens confirmed clean (no finding at any width)

command, chat (empty/populated as landed by seeding), missions, ...

## Known limitations of this pass

(e.g., "Wiki/Lattice never accumulated content within seed.mjs's two chat
turns — checked in their default/empty state only, which is a legitimate
empty-state render, not a skipped screen.")
```

List every one of the 23 screens somewhere in the document (either in the findings table or the "confirmed clean" list) — a screen missing from both is a gap in the audit, not a clean result.

- [ ] **Step 3: Commit**

```bash
git add docs/superpowers/artifacts/phase-189-mobile/findings.md
git commit -m "docs(phase-189): mobile verification findings"
```

---

### Task 5: Fix findings and re-verify

**Files:**
- Modify: `crates/aivyx-web/assets/stitch.css` (and `crates/aivyx-web/src/main.rs` only if a finding needs a markup change, not just CSS)
- Modify: `docs/superpowers/artifacts/phase-189-mobile/findings.md`
- Create/Replace: the specific `docs/superpowers/artifacts/phase-189-mobile/<slug>-<width>.png` files for every screen that had a finding

**Interfaces:**
- Consumes: `findings.md` from Task 4; `capture.mjs`'s `--screens`/`--widths` filters from Task 2.

- [ ] **Step 1: Read `findings.md` and fix each row**

For each finding, make the smallest CSS (preferably) or markup change that fixes it, matching the codebase's existing conventions (e.g. look at how `.stat-row`'s own `@media (max-width: 440px)` rule at `stitch.css:959-961` is written before adding an equivalent one for `.stat-row-5`). Do not touch anything not named in `findings.md` — this task fixes confirmed findings, not a general cleanup pass.

- [ ] **Step 2: Rebuild the bundle and the daemon**

```bash
export PATH="$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$PATH"
cd /home/julian/Projects/Rust/aivyx
just build-web
cargo build --bin aivyx
```

Expected: both exit 0.

- [ ] **Step 3: Restart the daemon**

Stop the previous `daemon run --web-ui` background process (find it with `pgrep -fa 'aivyx daemon run'` and stop it), then repeat Task 1 Step 4 to relaunch it with the freshly built binary — no rig/tunnel changes needed, they're unaffected by a web-bundle rebuild. Verify with `curl -fsS http://127.0.0.1:7843/ | head -c 200`.

- [ ] **Step 4: Re-capture exactly the screens/widths that had findings**

```bash
cd scripts/mobile-verify
node capture.mjs --base-url http://127.0.0.1:7843 \
  --out ../../docs/superpowers/artifacts/phase-189-mobile \
  --screens loop,audit \
  --widths 440
cd ../..
```

(Replace `--screens`/`--widths` with the actual set of `<slug>`/`<width>` pairs from your findings — include every width a given screen had a finding at, not just 440 if others also had one.)

- [ ] **Step 5: Confirm each fix with the Read tool**

Read each re-captured PNG and confirm the specific issue from `findings.md` is gone. If it isn't, go back to Step 1 for that finding — do not mark it fixed on a guess.

- [ ] **Step 6: Update `findings.md`**

For each row, append a `Resolution` column value: `Fixed` (with a one-line note on what changed) or, if on reflection during re-verification a "finding" turns out not to have been a real bug, `No change needed` with the reasoning — do not silently delete a row.

- [ ] **Step 7: Run the regression guard**

```bash
just check-web
cargo test -p aivyx-web
```

Expected: both pass (0 failures). These don't verify the visual fix — the re-captured screenshots do — but they confirm the CSS/markup edits didn't break the wasm build or any existing pure-function test.

- [ ] **Step 8: Commit**

```bash
git add crates/aivyx-web/assets/stitch.css crates/aivyx-web/src/main.rs \
        docs/superpowers/artifacts/phase-189-mobile/findings.md \
        docs/superpowers/artifacts/phase-189-mobile/*.png
git commit -m "fix(web): mobile-responsive verification findings — Phase 189"
```

(Omit `crates/aivyx-web/src/main.rs` from the `add` if no finding needed a markup change — check `git status` first rather than adding a file with no real changes.)
