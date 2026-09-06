# Phase 203 — Closing Phase 202's Three Logged Follow-Ups

**Follow-up cleanup — [SHIPPED] 2026-09-06.**

## Goal (carried from Phase 202's own retrospective)

Phase 202 (first-launch store safety) shipped with 3 follow-ups explicitly
logged in its own "Known follow-ups" section — one real gap and two
cosmetic issues, all found by that phase's own final whole-branch review.
Phase 203 was scoped to close all three.

## What shipped

- **Closed the real gap**: `aivyx --verify-only`, `aivyx audit export`,
  and `aivyx cost` could still create a real, permanent store on a
  completely unconfigured machine. Grounding found the original exclusion
  couldn't simply be removed: these 3 modes never require an API key
  (`LoadOptions.require_api_key` is `false` for them), so
  `config.validate()` was structurally incapable of ever detecting
  "nothing is configured yet" for them — the absence of a store had to
  become the trigger directly. `should_early_validate` simplified to a
  pure, mode-independent store-existence check; a new
  `early_validate_message` owns the mode-aware branching, and both the
  diagnostic modes and the default chat path now route through the exact
  same `decide_unconfigured_first_run` wizard-offer-or-fail mechanism —
  per the user's explicit choice during brainstorming to extend the
  shared mechanism rather than build a separate, simpler check.
- **Fixed the doubled `aivyx:`-prefixed output** on the non-interactive
  failure path, and **routed the passphrase-mismatch retry message**
  through the same testable prompt seam every other prompt in that
  function already uses, removing the last bare `eprintln!` from
  `aivyx-channel`'s passphrase module entirely.
- **A real TDD-ordering flaw was caught and fixed during planning
  itself**, before any code was written: the plan's first draft
  implemented two new pure functions before writing their own tests,
  which would have made a later "verify it fails" step nonsensical
  (the functions would already exist and the tests would already pass).
  Restructured into proper red-green order before the plan was ever
  dispatched.
- **Task 1's own review caught a real bug its own implementer's
  self-report completely missed**: `should_early_validate`'s doc comment
  was left genuinely duplicated by an imperfect find/replace (the old
  4-line paragraph wasn't removed, just prepended-to with new text) —
  found only by reading the real committed file directly, not by
  reading the diff or trusting the "no concerns" self-report. Fixed in
  a follow-up commit within the same task, with the full suite
  re-verified clean afterward.
- **The final whole-branch review found something more serious: Task 1's
  own fix had reintroduced the exact symptom Issue 2 existed to remove.**
  `early_validate_message` baked `"aivyx: "` into the shared message
  string before it was passed through `early_validate_fail_output`, so
  on the non-TTY path the returned `Err` — already prefixed — got
  prefixed a **second** time by `main()`'s generic error handler.
  Confirmed empirically by building and running the real binary:
  `aivyx: aivyx: no store exists yet at "…" — nothing to
  verify/export/report on`. The review also traced that the
  diagnostic-mode hint told operators to run `aivyx init`, which
  (confirmed: zero `RedbStorage`/`StorageConfig`/`derive_master_key`
  references anywhere in `init.rs`) only ever writes `aivyx.toml` and
  never creates a store — a remedy that could never resolve the
  condition it reported, walking an operator into a real dead-end loop
  (wizard runs once, `aivyx.toml` now exists, the wizard's own overwrite
  guard aborts on the second attempt, exits 0, nothing ever gets
  verified). Both fixed: `early_validate_message`'s messages are now
  prefix-free with their own mode-appropriate remedy baked in (the
  diagnostic-mode remedy correctly names running `aivyx` itself, not
  just `aivyx init`, as the thing that actually creates a store), and
  the `"aivyx: "` prefix is now added exactly once, only at the real
  print site.
- **The same review hunted for sibling instances of Task 1's own
  doc-comment-duplication bug class across the whole 3-commit diff** and
  confirmed, via `grep -c` on five distinctive phrases across every
  changed doc-comment region, that none exist — the one instance already
  found and fixed was the only one.
- 3 further Minor findings from the same review were fixed in the same
  commit: a stale gate-body comment still describing the old,
  mode-independent trigger condition; a doc comment on
  `read_interactive_password_with_confirm` that didn't mention the
  retry-prompt-as-output-channel design this phase's own Task 2 had just
  introduced; and a duplicated prompt-text literal between two
  `const`s.
- **Every fix in this phase was independently re-verified against the
  real, built binary, not just the diff or a report** — the controller
  ran `aivyx --verify-only` and `aivyx cost` against a genuinely
  nonexistent store path, non-interactively, and confirmed by hand:
  exactly one `aivyx:` prefix (not two), the corrected actionable hint,
  exit code 1, and zero files created on disk.
- Full `cargo test` (120 result blocks across default-members, 0
  failures) and `cargo clippy --all-targets -- -D warnings` (clean)
  independently re-verified by the controller after the final-review fix
  and again on merged `main`.

## The result

`aivyx --verify-only`/`audit export`/`cost` can no longer create a real
store on an unconfigured machine — the same safety guarantee Phase 202
gave the default chat path now covers every real invocation mode. The
non-interactive failure path prints its message exactly once, with a
hint that actually resolves the condition it reports for every mode. The
passphrase module's only remaining diagnostic message flows through the
one testable seam that function has, leaving zero untestable output
paths in that file. All 3 items Phase 202 logged as open are now closed.

## Known follow-ups (not done here, logged for whenever they matter)

None specific to this phase — every follow-up Phase 202 logged is closed,
and this phase's own final review's findings were all fixed within the
same branch before merge, not deferred.
