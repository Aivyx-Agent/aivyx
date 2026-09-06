# Phase 204 — Expanding INJECTION_MARKERS

**Chapter Picket follow-up — [SHIPPED] 2026-09-07.**

## Goal

`aivyx-injection-guard`'s `INJECTION_MARKERS` phrase list (Chapter Picket's
prompt-injection tripwire) had just 9 entries, ported verbatim from
`aivyx-coder`'s original extraction and never revisited since. No real-usage
miss had been observed; the user chose to proactively expand it anyway,
reasoning that these are well-documented, widely-known injection/jailbreak
phrasings rather than speculative guesses. A second open item from the same
brainstorming session — Chapter I's "polish" placeholder — was deliberately
scoped separately and is not part of this phase.

## What shipped

- **`aivyx-injection-guard`'s marker list grew from 9 to 22 entries**,
  grouped into 4 categories (instruction override/reset, system-prompt
  extraction, role/persona jailbreak, safety-guideline bypass), appended
  strictly after the original 9 to preserve
  `scan_breaks_ties_by_marker_list_order_not_by_position_in_text`'s
  list-order tie-break guarantee. 5 new tests: one match per new category
  plus one near-miss check. Shipped via 2 commits directly to that repo's
  `master` (no CHANGELOG/version-bump convention there — confirmed via
  `aivyx-confine` precedent — consumed purely by pinned git `rev`).
- **`aivyx`'s pin bumped** to the new commit; `Cargo.lock` refreshed via
  `cargo update -p aivyx-injection-guard`. No `aivyx`-side code changes —
  the existing injection-scan tests in `aivyx-core` don't hardcode the
  list's length or full contents, only that a known phrase escalates, so
  they passed unchanged.
- **The final whole-branch review (Opus) found the list itself needed a
  second pass before merge — not a mechanics problem, a content one.**
  Two of the 14 new markers were materially more false-positive-prone than
  the rest: `"developer mode"` is an ordinary technical noun phrase (the
  literal name of a real setting in Chrome/Edge extensions, Android, iOS,
  Windows, VS Code — unlike every other marker's second-person-imperative
  phrasing aimed at an AI), and `"you are no longer"` collides with routine
  machine-generated notification footers ("You are no longer subscribed to
  this thread"). Given a match here doesn't just add a warning envelope —
  `check_for_injection` sets `LoopOutcome::Escalated` and hard-stops the
  turn — a false positive on either phrase would have been a real,
  user-visible degraded-UX cost on entirely ordinary `web.fetch`/`fs.read`/
  `gmail.search` content. `"developer mode"` was dropped outright;
  `"you are no longer"` was narrowed to `"you are no longer bound by"`,
  keeping the jailbreak-framing intent while cutting the collision. Final
  marker count: 22, not the originally-proposed 23.
- **The same review caught a doc comment in `aivyx-core`'s
  `check_for_injection` that this phase's own change had made stale**:
  it claimed the marker list was "ported verbatim from aivyx-coder" (no
  longer true — the two consumers' pins have now diverged in length) and
  "no config knob to disable the tripwire" (already false before this
  phase — `injection_scan_enabled`/`injection_scan_exempt` already existed
  on the struct; the comment had simply never been updated when those
  fields were added). Corrected in the same fix pass.
- **A process gap in this phase's own execution was also caught and
  fixed**: the implementation plan document was written but never
  committed — a break from every prior phase's convention of committing
  the spec and plan as paired commits. Added retroactively in the same
  fix.
- Both per-task reviews (the content change in `aivyx-injection-guard`;
  the pin bump in `aivyx`) came back clean with zero findings — everything
  above came from the final whole-branch review looking at both repos'
  diffs together, which a task-scoped review structurally couldn't have
  caught (the false-positive risk only shows up against `aivyx-core`'s
  real call sites; the doc-comment staleness only shows up by reading a
  file neither task touched).
- Every fix was independently re-verified by the controller directly
  against real files and real test runs, not from subagent reports:
  `aivyx-injection-guard`'s diff read byte-for-byte, `Cargo.lock`/
  `Cargo.toml` pin values grepped directly, the corrected doc comment read
  in full, the plan file's copy diffed byte-identical to the original.
  `cargo test`/`cargo clippy` re-run personally in both repos after the
  fix: `aivyx-injection-guard` 15/15 passing, clippy clean; `aivyx` 120/120
  test-result blocks (default-members — this repo's own convention;
  `--workspace` pulls in `aivyx-desktop`, which fails locally on an
  unrelated missing system `webkit2gtk` package, not on anything this
  phase touched), clippy clean.
- One transient failure surfaced during final verification
  (`team_mission_driver::tests::pause_requested_mid_drive_lands_the_mission_in_paused_not_halted`,
  a timing-sensitive test in `aivyx-channel`, a crate this phase never
  touched) — confirmed a pre-existing parallel-execution flake, not a
  regression: it passed 5/5 in isolation, and a full-suite re-run came back
  120/120 clean.

## The result

`aivyx`'s prompt-injection tripwire now recognizes 22 markers across 5
categories instead of 9 in one undifferentiated list, with the 2
highest-false-positive-risk candidates caught and fixed before merge rather
than after a real user hit one. `check_for_injection`'s doc comment
accurately reflects both the list's provenance and the config knobs that
already exist to work around a false positive. The implementation plan is
committed alongside its spec, matching every other phase's paired-commit
convention.

## Known follow-ups (not done here, logged for whenever they matter)

- **`aivyx-coder`'s own separate pin of `aivyx-injection-guard`** was
  deliberately left untouched (a different, unrelated active project) and
  will now diverge from `aivyx`'s pin (9 markers vs. 22). Bumping it too
  would be a reasonable, low-risk pickup for whoever next touches that
  project — not acted on here without being asked.
- **The remaining 20 markers (the original 9 plus 11 of the 14 new ones)
  were not individually re-audited for false-positive risk against real
  Gmail/Calendar/MCP-tool-shaped content** the way the 2 removed/narrowed
  ones were — the final review flagged 3 second-tier candidates
  (`"without any restrictions"`, `"from now on you will"`, `"do anything
  now"`) as plausible but meaningfully lower-risk, and the user's own
  earlier framing (a documented, accepted cost of a phrase-list tripwire)
  was judged to already cover them. Worth revisiting if any of the three
  ever produces a real false positive in practice.
- **Chapter I's "polish" placeholder** — the second open item from the
  brainstorming session this phase's scoping came out of — remains
  unstarted. There's an open, unresolved question from that same session
  about whether live-backend test infrastructure (a GPU rig + Playwright,
  the setup Phase 189 used) is actually available for that investigation,
  which needs resolving before any concrete scoping work begins.
