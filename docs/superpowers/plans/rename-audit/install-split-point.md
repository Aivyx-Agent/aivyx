# `docs/INSTALL.md` split point — confirmed

**Confirmed line:** `3212`
**Confirmed heading text:** `## Vertical packs — install a signed pack (Chapter Freight)`
**Total file length:** `6678` lines (unchanged from the earlier grounding)

## Verification

Re-ran `grep -n "^## " docs/INSTALL.md` against the current worktree. The
heading `## Vertical packs — install a signed pack (Chapter Freight)` is
still at line 3212, and the file is still exactly 6678 lines long — zero
drift since this plan's own earlier grounding. No adjustment needed.

## Split shape

- **Part 1** (Task 8): lines 1–3211 (the split heading itself starts Part 2)
  — covers "Current install state" through "Connecting a productivity tool
  — `aivyx connect` (Phase 182)", i.e. everything up to and including the
  `[sandbox]` tool-process-sandboxing section. 3211 lines.
- **Part 2** (Task 9): lines 3212–6678, starting at "## Vertical packs —
  install a signed pack (Chapter Freight)" through "## Uninstall" at the
  end of the file. 3467 lines.

## Note on exact midpoint (informational, not actionable)

The file's precise numeric midpoint is line 3339 (6678 / 2). The next
heading after the confirmed split point, `## External productivity
integrations (Chapter F)` at line 3258, is arithmetically 46 lines closer
to that exact midpoint than line 3212 is (81 lines away vs. 127 lines
away). Both are "near the midpoint" in the sense the original plan meant.
Per the step 3 instructions ("if the line has moved, pick the nearest
`##`-level heading to the new midpoint instead") — the line has *not*
moved, so no reassignment is warranted; recorded here only so Tasks 8/9
aren't surprised if they independently re-derive the midpoint and notice
3258 is numerically closer. Line 3212 is still a clean, sensible boundary:
it falls right before the vertical-packs / productivity-integrations /
personal-assistant-capabilities run of chapters that make up the back half
of the doc's content, keeping "core install + first-run + service +
channel" material together in Part 1.
