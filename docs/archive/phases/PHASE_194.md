# Phase 194 — Chapter Picket, Phase 1: Create the aivyx-injection-guard Repo

**Chapter Picket, phase 1 — [SHIPPED] 2026-09-05.**

## Goal (carried from the design)

While investigating end-user deployment options, this session asked a
broader question: given `aivyx-confine`/`aivyx-checkpoint`/`aivyx-kvcache`
were all extracted from `aivyx-coder` and adopted by `aivyx`, are there
other similar frameworks worth considering? A fresh survey (real code
opened in both repos, not inferred) found one strong, concrete candidate:
`aivyx-coder`'s `aivyx-sandbox/src/injection_scan.rs` — a phrase-list
prompt-injection tripwire, real and extensively wired into five call sites
(editor context, repo map, user/project `AGENTS.md`, generic tool output)
via `ConfirmationGate` and the autonomous loop. `aivyx` has only a passive
defense in this space (Bulwark's `fence_untrusted_output`, which labels
untrusted content for the model but never actively scans for known
injection phrasings or halts execution). This gap matters specifically
because of `aivyx`'s autonomy dial — at `Autonomous`/`Unleashed` tiers the
agent runs fully unattended, exactly the scenario an undetected injection
could exploit with nobody watching.

Grounding also found `TurnOutcome::Escalated` and the `ApprovalGate`/
`HeadlessRefusal` split (`escalation_parks`) already exist in `aivyx` and
already correctly branch on attended-vs-unattended — nothing currently
triggers `Escalated` from an injection match, but the plumbing needs zero
new code once something does.

Phase 194 is Chapter Picket's first phase: extracting the tripwire into a
new, standalone, public repo — the same `aivyx-confine`/`aivyx-checkpoint`/
`aivyx-kvcache` extraction pattern, applied a fourth time.

## What shipped

- **A new, public GitHub repo: `Aivyx-Agent/aivyx-injection-guard`.**
  Single crate, no workspace, zero dependencies (the ported code only uses
  `std::sync::{Arc, Mutex}`), `MIT OR Apache-2.0` license (license files
  copied verbatim from `aivyx-confine`, not retyped).
- **A verbatim port** of `aivyx-coder`'s `injection_scan.rs`: the
  `INJECTION_MARKERS` phrase list, `scan_for_injection_markers`,
  `InjectionFinding`, `InjectionTaint`, and all 10 existing tests —
  confirmed byte-identical to the real source file via direct `diff`, not
  assumed. Public API names preserved exactly; later phases (the
  `aivyx-coder` migration, the `aivyx` adoption) depend on them unchanged.
- **A real, independently-verified fix wave, twice.** The task reviewer
  caught one Minor issue a task-level implementer working from a brief
  can genuinely miss: the new crate's top-of-file doc comment paraphrased
  rather than used the brief's literally-specified wording. Fixed directly
  (not worth a full fix-subagent dispatch for one doc-comment paragraph),
  then re-verified the byte-identical-body constraint still held at the
  new, shifted line offset the fix introduced — confirming the fix itself
  didn't quietly break the very property the task existed to guarantee.
- **The repo's public visibility verified twice independently**: once by
  the controller directly (`gh repo view`, `git ls-remote` over HTTPS with
  `credential.helper=` cleared), once by a separate read-only verification
  subagent re-running the same checks from scratch plus confirming the
  real `Cargo.toml` content via the GitHub Contents API — matching the
  verification standard Phase 192 established for the other three sibling
  repos.

## A deliberate process deviation, and why

This phase's final whole-branch review and merge step were skipped, unlike
every code-touching phase this session. The reason: the entire deliverable
is one commit in a brand-new repo, already fully reviewed at the task
level (byte-identical-port verification, build/test/clippy re-run
clean-room, license-file diff, public-API-name check) — Task 2 published
that exact commit with zero additional code. A separate "final whole-branch
review" would have re-reviewed the identical diff a second time for no new
integration surface, and there is no `aivyx`-repo-local branch to merge
this into (the new repo is standalone). Documented here explicitly rather
than silently skipped, matching this session's own standard: adapt process
to what a phase's shape actually needs, don't run a step that can't catch
anything a prior step didn't already cover.

## The result

The tripwire logic now has a real, public, independently-owned home. Two
phases remain in Chapter Picket: migrating `aivyx-coder` onto this crate
(removing its local copy, and correcting that repo's own stale
"indirect prompt injection... re-enters context untagged" README/CLAUDE.md
line — confirmed stale this session, since the scanner is real and live),
and adopting it into `aivyx` at Bulwark's existing call site, converting a
match into the existing `TurnOutcome::Escalated` outcome.

## Known follow-ups (not done here, logged for whenever they matter)

- **`aivyx-coder`'s migration onto this crate** — not started. `aivyx-coder`
  still has its own local copy of `injection_scan.rs`; nothing about this
  phase changed that repo.
- **`aivyx`'s adoption + Bulwark-site integration** — not started. `aivyx`
  does not yet depend on `aivyx-injection-guard` at all.
- **`aivyx-coder`'s stale doc line** — its own README/CLAUDE.md still say
  "indirect prompt injection (file/command content re-enters context
  untagged)" as a known limitation, which is no longer accurate (confirmed
  this session: 5 real call sites, `ConfirmationGate` consumption, TUI-level
  pause behavior). Correcting this belongs to the migration phase, since it
  requires editing that repo's own files.
