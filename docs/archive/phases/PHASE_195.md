# Phase 195 — Chapter Picket, Phase 2: Migrate aivyx-coder onto aivyx-injection-guard

**Chapter Picket, phase 2 — [SHIPPED] 2026-09-05.**

## Goal (carried from the design)

Phase 194 extracted `aivyx-coder`'s real, production-wired prompt-injection
phrase-list tripwire into a new, standalone, public repo —
`Aivyx-Agent/aivyx-injection-guard` — but `aivyx-coder` itself still had its
own local copy (`crates/aivyx-sandbox/src/injection_scan.rs`). Phase 195
closes that: `aivyx-sandbox` now depends on the external crate instead,
the same relationship it already has with `aivyx-confine` for process
confinement, and one real stale documentation claim found alongside the
original survey gets corrected.

## What shipped

- **`aivyx-sandbox` now depends on `aivyx-injection-guard`** (pinned-rev
  git dependency, `Aivyx-Agent/aivyx-injection-guard`, matching the
  existing `aivyx-confine`/`aivyx-checkpoint`/`aivyx-kvcache`/`aivyx-recall`
  pattern in the workspace root `Cargo.toml`). `lib.rs`'s re-export
  (`pub use aivyx_injection_guard::{InjectionFinding, InjectionTaint,
  scan_for_injection_markers}`) replaced the local `mod injection_scan;`
  — confirmed via `grep` before making the change, and independently
  re-confirmed by both the task reviewer and the final review, that every
  other real call site in the repo (`agent/mod.rs`, `delegate.rs`,
  `confirmation.rs`, `agent_builder.rs`, `aivyx-tui/src/app.rs`, and their
  tests) imports through this re-export rather than the submodule path
  directly — so none of them needed any change.
- **The local `injection_scan.rs` file (260 lines, 10 tests) deleted.**
  Those 10 tests now live, byte-identical, in `aivyx-injection-guard`'s own
  repo (verified in Phase 194) — `aivyx-coder`'s own test count dropping
  from 682 to exactly 672 is expected, not a regression, and was called out
  explicitly in the plan so no reviewer would flag it as one.
- **One real stale doc claim corrected**, found during Phase 194's own
  survey: `CLAUDE.md`'s "Known, deliberately-undefended limitations"
  section said prompt-injection content "re-enters context untagged" — no
  longer true, since the scanner actively tags a match via `InjectionTaint`.
  `README.md`'s own, separate "Known limitations" section was read in full
  and confirmed **already accurate** (it correctly describes the real
  scan-and-pause mechanism, its heuristic/pattern-based nature, and that it
  only runs in autonomous mode) — left untouched, narrowing what the
  original survey assumed might be a two-file fix down to the one file that
  actually needed it.
- **A second real, independently-caught fix**: the final whole-branch
  review found that `CLAUDE.md`'s own architecture table documents the
  `aivyx-confine` and `aivyx-checkpoint` extractions on their respective
  crate rows, but the same `aivyx-sandbox` row said nothing about the new
  `aivyx-injection-guard` dependency this phase just added — a maintainer
  reading only the table would wrongly conclude the injection scanner was
  still implemented locally. This is exactly the kind of gap a per-task
  review, looking at one diff at a time, has no way to catch (Task 1
  changed what the table *describes*; Task 2 edited a *different* section
  of the same file) — fixed directly in a follow-up commit rather than
  carried as debt.
- **Full, independently-reproduced verification**: the final review
  re-ran `cargo test --workspace` (672/672, matching the exact expected
  count) and `cargo clippy --workspace --all-targets -- -D warnings` (zero
  warnings) itself rather than trusting either task's report, confirmed
  the pinned rev against the live GitHub remote, confirmed
  `cargo check -p aivyx-sandbox --no-default-features` still works (the new
  dependency is feature-free, so it can't perturb the non-default Landlock
  build path), and confirmed `Cargo.lock`'s new entry introduces zero
  transitive dependencies.

## The result

`aivyx-coder` and `aivyx` (once Phase 196 lands) will share the exact same
prompt-injection tripwire implementation, the same way they already share
process confinement (`aivyx-confine`) and git-ref checkpointing
(`aivyx-checkpoint`). One phase remains in Chapter Picket: adopting
`aivyx-injection-guard` into `aivyx` itself, at Bulwark's existing
untrusted-tool-output call site, converting a match into the already-
existing `TurnOutcome::Escalated` outcome.

## Known follow-ups (not done here, logged for whenever they matter)

- **`aivyx`'s own adoption** — not started (Phase 196, expected next in
  this chapter).
- **No other known gaps.** Unlike prior phases this session, this one
  closed with zero deferred findings beyond the next planned phase itself
  — both findings the final review surfaced (the architecture-table gap,
  and a Minor accuracy nit about "doesn't run in interactive mode at all"
  slightly understating that the *scan* itself is ungated, only the
  *response* is autonomous-only) were resolved or explicitly judged
  correct-as-is (the wording is copied verbatim from `README.md`'s own,
  already-declared-authoritative text, so diverging the two documents
  would be worse than the slight imprecision both now share equally).
