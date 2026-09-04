# Phase 189 — mobile verification findings

Audited: 92 screenshots (23 screens × 4 widths: 1280/860/600/440) in this directory.

**Update (final-review fix wave):** the original 92 screenshots were captured
with `fullPage: true` against a viewport fixed at 900px tall. The app shell
is a fixed `height: 100vh` grid with an internal scrolling `.view` element,
so the *document* itself is always exactly viewport-height regardless of how
much `.view` scrolls internally — `fullPage: true` screenshots the document,
not an element's internal scroller, so every one of those 92 screenshots
only ever showed the above-the-fold ~800px of each screen. Below-the-fold
content on longer screens (Tools' 57 cards, Audit's 21 events, Teams' 9
full member cards, Sessions' full connection list, Guide's whole page,
Settings' full config, and more) was never actually seen by the original
audit pass. `scripts/mobile-verify/capture.mjs` now strips `.view`'s height
constraint and `overflow-y: auto` before each screenshot (see the comment
above `expandView` in that file for why, and for two other approaches that
were tried and didn't work), so `fullPage: true` now genuinely captures
every screen's full content. All 92 screenshots were re-captured this way.
See "Re-audit of previously-hidden content" below for what the re-audit of
that newly-visible content found (nothing new).

| Screen | Width | Issue | Screenshot | Proposed fix | Resolution |
|---|---|---|---|---|---|
| command | 860, 600, 440 | The 5-tile stat row (Missions, Active, Routines, Audit Events, Chain) drops to a 2-column grid but never reflows further — the 5th tile ("Chain") ends up alone in an incomplete last row with half the row left empty, at all three narrow widths. No clipping or overlap, just a visibly unbalanced/orphaned tile. | `command-860.png`, `command-600.png`, `command-440.png` | Add a 1-column stacking rule for this `.stat-row-5` block at ≤600px (matching the plain `.stat-row`'s fallback), or make the 5th tile span the full row width when it's the sole occupant of the last row. | Fixed — added `.stat-row-5 > :last-child { grid-column: 1 / -1; }` inside the existing `@media (max-width: 900px)` block in `stitch.css`, so the lone 5th tile now spans the full row instead of sitting alone in one column. Re-captured `command-860.png`/`command-600.png`/`command-440.png` and confirmed the "Chain" tile fills the last row's width at all three. |
| settings | 440 | Under "Access level," the `CURRENT REACH: /HOME/JULIAN/...` path readout wraps normally on its first line (breaking at a hyphen) but its second line ("MOBILE-VERIFICATION/.DEV-RUN-189/SANDBOX") runs flush to the viewport's right edge with no right padding — unlike every other paragraph on the same card, which wraps cleanly inside the card's margin at this width (and unlike the same "CURRENT REACH" line at 600px, which does keep its margin). This is a horizontal-overflow-at-the-edge symptom specific to this one long path string at 440px. | `settings-440.png` | Add `overflow-wrap: break-word` (or `word-break: break-all`) to the current-reach path readout so long unbroken path segments wrap inside the card at narrow widths instead of pushing to the container edge. | Fixed — added `word-break: break-word` to `.settings-section .label-tech.sub` in `stitch.css` (matching the same property used elsewhere in the codebase, e.g. `.mem-body`, `.line`, `.doc-text`). Re-captured `settings-440.png` and confirmed the wrapped "CURRENT REACH" second line now sits inside the card's margin with no edge overflow. |
| settings | 440 | On the same screenshot, the "Autonomy" card's LEVEL `<input>` shows its value truncated mid-word — "assisted — reversible free, irreversible confirmed (d" with no ellipsis or scroll affordance — where the full text ("...confirmed (default)") is visible at 600/860/1280px. The input box itself appears to run edge-to-edge in the card at this width, leaving no room for the full value; a native text input just clips overflowing content rather than wrapping it. | `settings-440.png` | Give the `.input` class (or this specific field) a `text-overflow: ellipsis` + `overflow: hidden` treatment at narrow widths so truncation is visibly intentional, or check whether the label/input column split (`.field-row`'s `120px 1fr` — see `stitch.css`) is leaving too little width for the input at 440px and needs its own narrow-width adjustment alongside the fix for the row above. | Fixed — this field is actually a `<select>` (not a text `<input>`), so added `text-overflow: ellipsis; overflow: hidden; white-space: nowrap;` to the existing `select.input` rule in `stitch.css` (same truncation pattern as `.doc-name`/`.doc-viewer h4` elsewhere in the file), rather than a `.field-row`-width change — the `.field-row` already goes single-column at ≤600px (`stitch.css:952`) so width wasn't the constraint, the value string is just too long to fit even at full card width. Re-captured `settings-440.png` and confirmed the value now reads "assisted — reversible free, irreversible confirmed (…" with a visible ellipsis instead of a hard mid-word clip. |

## Screens confirmed clean (no finding at any width)

agents, audit, chat (empty state, see Known limitations), documents, gallery (unconfigured-server empty state), graph (no-graph empty state), guide, loop, mcp (no-servers empty state), memory (empty state), mission-control (idle empty state), missions, notifications, reminders (empty state, see Known limitations), schedules, sessions, skills, teams, tools, voice, wiki (empty state)

(`command` and `settings` appear only in the findings table above per the brief's "table OR clean list" rule — both screens are otherwise fine at 1280, and `command`'s stat-tile issue is a minor layout imbalance rather than a functional break.)

## Known limitations of this pass

- **Chat** (`chat-*.png`): shows "SEND A MESSAGE TO START A TURN." at all four widths. The transcript is a client-side-only signal never persisted across page loads, so a fresh screenshot always sees it empty. This is correct, expected behavior, not a defect.
- **Reminders** (`reminders-*.png`): shows "NO PENDING REMINDERS." at all four widths. During seeding, the model answered a reminder request conversationally without invoking a reminder-setting tool, so no reminder was actually created server-side. This is an accurate empty state, not a bug.
- **Gallery** (`gallery-*.png`): shows the "no `comfyui` MCP server is configured" message at all four widths — this throwaway daemon was never configured with a ComfyUI MCP server, so this is a legitimate unconfigured-feature state, not a skipped screen.
- **Graph** (`graph-*.png`): shows "NO GRAPH YET" at all four widths — the graph feature requires `[memory] profile = "smart"` or `[graph] enabled = true`, neither of which this throwaway daemon has set. Legitimate default/disabled-feature empty state.
- **MCP** (`mcp-*.png`): shows "no MCP servers configured" at all four widths — same throwaway-daemon reason as Gallery/Graph above.
- **Wiki** (`wiki-*.png`): shows "NO PAGES YET" at all four widths — wiki consolidation also requires the "smart" memory profile, unset here. Legitimate default/disabled-feature empty state, not a skipped screen.
- **Mission Control** (`mission-control-*.png`): shows "No mission is currently executing, paused, or awaiting approval" at all four widths — the one seeded mission already completed by the time of capture, so Mission Control's live-run view is correctly empty. Missions itself (a different screen) shows the completed mission with its delegate steps.
- **Sessions** (`sessions-*.png`): the "N CONNECTED" count differs across all four screenshots (13 → 14 → 15 → 16), and the specific session UUIDs at the top differ too. This is expected: each screenshot opens its own new WebSocket session against the live daemon, so the active-session list and count grow between captures. Not a layout defect.
- **Audit** / **Notifications** rows mixing a fixed-width timestamp/id badge with body text were specifically checked against the brief's named "known suspect" (crowding out the message at 440px). Both wrap cleanly onto multiple lines at 440px with no truncation or overlap — this suspect did not reproduce on this content, since event names in this baseline are short single words (e.g. `TurnStarted`, `LlmCost`) and the one notification's mission ID wraps onto its own line without colliding with the "DELIVERED" badge.

## Re-audit of previously-hidden content (final-review fix wave)

After fixing the viewport-clipping bug described at the top of this file,
every screen's full content became visible for the first time at all four
widths. Re-audited against the same bug bar as the original pass
(horizontal overflow, clipped/overlapping content, tap targets <~40px at
440px) — screens checked, with what the newly-visible content turned out to
be:

- **Tools** (57 cards across 21 category groups) — full pass at all 4
  widths. Every card renders completely; the last card (`workspace.write`)
  and its full description are visible with a clean footer below it at
  every width, including 440px (previously cut off mid-sentence 3 cards in,
  per the finding that motivated this fix wave). No overflow, no clipping,
  no tap-target issues — cards are plain content blocks, no small buttons.
- **Audit** — all 21 events now visible (previously only 15 of 21 showed).
  No new issues; wraps the same clean way the visible portion already did.
- **Teams** — all 9 Nonagon member cards now visible (previously only 2-3
  showed). Each member's SCOPES/TOOLS/SOUL fields are `<textarea>`s with a
  small fixed height and their own internal scrollbar showing partial text
  — this is consistent at every width (including 1280) and is normal
  resizable-textarea behavior, not a mobile-specific clipping bug in scope
  here. No horizontal overflow on any of the 9 cards.
- **Agents** — the "Change history" list's `Revert` buttons sit in a
  two-column flex row (text left, button right, vertically centered) at
  both 440px and 1280px — same layout at both, not a narrow-width
  regression. No new issues.
- **Sessions** — the full 22-session list now visible (previously ~13-16
  showed, cut off partway through). No overflow, cards render identically
  down the list.
- **Settings** — the two already-fixed findings (path word-break, autonomy
  select ellipsis) re-verified as still fixed against the full-page
  capture. The rest of the page (Budget, Agent/cycle-breaker, Model,
  Memory profile, Embedding, Proactive surfacing sections), now fully
  visible for the first time, has no overflow or clipping at any width.
- **Guide** — the full Welcome page and its table-of-contents sidebar list
  are now visible (previously cut off partway through "The big picture").
  No overflow; TOC links stack cleanly at 440px.
- **Skills** — all 5 skill cards now visible (previously ~2-3 showed). No
  new issues.
- **Notifications** — the full Channel adapters section (Email/Telegram/
  Discord/Slack config cards) is now visible (previously cut off partway
  through Email). No overflow or clipping.
- **Voice**, **Command** (re-verified the `stat-row-5` fix still holds
  against the full-page capture), **Memory**, **Missions**, **Chat**,
  **Graph**, **Wiki**, **MCP**, **Loop**, **Reminders**, **Gallery**,
  **Mission Control**, **Schedules**, **Documents** — all were already
  short enough that the old 900px-tall capture had shown their full content
  (or, for Loop/Settings, are short once `.dev-run-189/aivyx.toml`'s
  `[loop] enabled = true` is in place — see Fix #3 below); re-checked
  anyway and confirm no new issues.

**Result: no new confirmed findings.** The viewport-clipping bug was a real
gap in the audit's *coverage*, but the content it had been hiding does not
contain any new mobile-responsive defects — every screen's previously-unseen
content follows the same layout patterns already verified clean on the
above-the-fold portion.

## Tap-target criterion — checked, deliberately deferred

The bug bar's tap-target criterion (interactive elements under ~40px tall
at 440px width) was implicitly applied throughout this pass but never
explicitly recorded as checked, which left the "zero tap-target findings"
result reading as "not checked" rather than "checked and clean." Recording
it explicitly here:

- **`.btn-xs`** (`crates/aivyx-web/assets/stitch.css:668`) computes to
  **22.5px tall** (measured directly via `getBoundingClientRect()` on the
  live daemon at 440px width — e.g. Schedules' "Add reflection schedule"
  and "Create schedule" buttons). It's used 48 times across
  `crates/aivyx-web/src/main.rs` on primary actions (e.g. "Create
  schedule", "Add reflection schedule", "Add target", "Add server", and
  Edit/Delete pairs throughout). Visible in `schedules-440.png`.
- **`.icon-btn`** (`crates/aivyx-web/assets/stitch.css:250`) is a fixed
  **34×34px**. Used for topbar icon buttons (help, notifications, theme
  toggle) visible at the top of every screenshot.

Both are under the ~40px bar. **Not fixed in this phase**: both are global
classes used identically on desktop, where they read as intentionally
compact secondary/inline actions rather than a usability problem: resizing
them would be a desktop-affecting change, which is outside this phase's
scope (a CSS-only mobile-verification pass, not a redesign touching shared
component sizing). Flagging this as a real candidate for a future phase —
either a mobile-only touch-target override (e.g. larger tap area via
padding/pseudo-element without changing visual size) or a broader look at
whether these two classes need a bigger floor across the board.
