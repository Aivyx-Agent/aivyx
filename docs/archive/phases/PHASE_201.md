# Phase 201 — Chapter Picket Follow-Up: Channel-Session Config Threading

**Chapter Picket follow-up — [SHIPPED] 2026-09-06.**

## Goal (carried from Phase 200's known follow-up)

Phase 200's own final review found that the standalone Telegram/Discord/
Slack channel-session paths — the in-process fallback used when
`--no-daemon` is passed or no daemon socket is reachable — construct
every agent via `TurnSafety::default()` directly and never call
`.interactive()`/`.autonomous()` at all. That means they receive none of
the operator's `[agent]` config: not just `injection_scan_enabled`/
`injection_scan_exempt` (the two knobs Phase 200 propagated everywhere
else), but `cycle_detection`/`turn_timeout_secs` too, which predate this
whole Chapter Picket arc. Phase 201 was scoped to close that gap.

## What shipped

- **Grounding found the real gap was narrower and better-bounded than it
  sounded.** Each of the 3 channel crates has two dispatch modes: a
  daemon-mode path (a thin IPC client relaying to an already-running
  daemon process — the daemon itself constructs `daemon_agent`, already
  correctly wired since Phase 199/200) and an in-process fallback. The
  daemon-mode path was never at risk — it never constructs a
  `ConcreteAgent` client-side at all. Only the in-process fallback ever
  called `TurnSafety::default()` directly, and that was the entire
  surface this phase touched.
- Added 4 fields — `turn_timeout_secs: Option<u64>`, `cycle_detection:
  Option<bool>`, `injection_scan_enabled: bool`, `injection_scan_exempt:
  BTreeSet<String>` — to `TelegramSessionConfig`, `DiscordSessionConfig`,
  and `SlackSessionConfig`, mirroring the existing `tool_allowlist`/
  `memory_topic_prefix` fields on the same structs (the user's explicit
  choice over a separate-parameter shape like `checkpointer`'s, since
  these are session config, not a shared resource handle). Changed all 4
  real `TurnSafety::default()` call sites (Telegram×2, Discord×1,
  Slack×1) to `TurnSafety::interactive(...)`, populated from the same
  `turn_timeout_secs`/`cycle_detection`/`injection_scan_enabled`/
  `injection_scan_exempt` locals `daemon_agent`/`child_agent`/
  `build_agent_stack` already use — no new config plumbing anywhere.
- **A real plan defect was found and fixed mid-execution, not an
  implementer error.** The plan's own new-test code asserted the
  channel's final reply text matched `"done"` exactly. Task 1's
  implementer correctly found this failing and flagged it rather than
  forcing it green: all 3 channels unconditionally render a `"→
  tool_name"` / `"← tool_name ..."` progress line for every dispatched
  tool call, regardless of injection scanning, so the reply legitimately
  contains more than `"done"` even when the scan is correctly disabled.
  Fixed by correcting the plan for all 3 tasks before Discord/Slack were
  dispatched: the assertion now checks for the *absence* of the
  escalation footer (`"⏸ escalation:"`) instead of an exact match — the
  signal that actually distinguishes "scanned and escalated" from "not
  scanned." 3 of 4 task reviews independently mutation-tested this
  corrected assertion (reverting the fix, confirming the test fails with
  the real escalation-footer text present, then restoring) — non-vacuous
  by direct proof, not inference.
- **The final whole-branch review (Opus) re-verified every load-bearing
  grounding claim from scratch** rather than trusting the plan or task
  reports: re-ran the "exactly 4 real `TurnSafety::default()` call sites
  system-wide" grep and confirmed it true both before and after (zero
  production callers remain post-branch); confirmed no other bypass site
  exists anywhere in the codebase (every real `ConcreteAgent::new` site
  routes through `TurnSafety`); confirmed `TurnSafety::interactive(...)`'s
  real signature matches all 4 new call sites in argument order with no
  coercion; confirmed all 3 crates were implemented identically with no
  drift (byte-identical doc comments, field order, and a uniform choice
  to move rather than clone `injection_scan_exempt` at every site);
  confirmed none of the 3 new structs derive `Default` (so this branch
  introduces no new instance of the Phase 200 regression class at the
  struct level).
- **The review found one more instance of that same regression class,
  one level up.** `TurnSafety::default()`'s own hand-written `Default`
  impl — Phase 200's fix for a real, shipped security regression — had a
  doc comment naming the exact Telegram/Discord/Slack callers this
  branch just moved off of `TurnSafety::default()` entirely. With zero
  test coverage of the invariant, a future contributor reading the
  now-stale justification ("this only matters for these 3 callers, and
  they don't exist anymore") could reasonably conclude the hand-written
  impl is no longer needed and derive `Default` instead — silently
  re-opening the identical regression at the choke point itself, this
  time undetectable by any test. Fixed: reworded the doc comment to
  state the invariant without naming now-nonexistent callers, and added
  a dedicated test, `turn_safety_default_preserves_injection_scan_enabled`,
  pinning `TurnSafety::default().injection_scan_enabled == true` so a
  future `#[derive(Default)]` regression fails CI immediately instead of
  shipping silently.
- One Minor finding from the same review, fixed in the same commit: a
  missing `CHANGELOG.md` `[Unreleased]` entry for this phase's
  operator-visible behavior change (a configured `injection_scan_enabled
  = false` now actually takes effect on the 3 standalone channel paths,
  where it previously had no effect at all).
- Two further findings were reviewed and explicitly left as-is: the
  doc-comment wording nit Task 3's own review logged (`"at each
  construction site below"` reads as plural where Discord/Slack each
  have one site) was judged not worth trading the 3 structs' deliberate
  byte-identical-doc-comment consistency for; and Telegram's untested
  single-chat path (`run_telegram_session_with_transport`, fixed
  identically to the tested mailbox path) was confirmed to have zero
  real production caller, so the coverage asymmetry is a recorded,
  harmless decision rather than an accident.
- Full `cargo test` (120 result blocks across default-members, 0
  failures) and `cargo clippy --all-targets -- -D warnings` (clean)
  independently re-verified by the controller both after the final-review
  fix and again on merged `main`.

## The result

All 4 real `TurnSafety::default()` call sites in the system are gone.
Every real agent-construction path — daemon, role-switch child, both
channel-session construction sites, the CLI's team-run lead, every team
specialist, and now the 3 standalone channel-session fallback paths —
receives the operator's full `[agent]` config through the same
`TurnSafety::interactive`/`autonomous` mechanism. The Phase 200 regression
class (a derived `Default` colliding with `apply()`'s unconditional write)
is now guarded by an executable test at its own choke point, not just a
doc comment.

## Known follow-ups (not done here, logged for whenever they matter)

- None specific to this phase. Chapter Picket's own remaining, previously
  logged follow-up — expanding `INJECTION_MARKERS` beyond its
  verbatim-ported starting set — remains separately open, untouched by
  this phase.
