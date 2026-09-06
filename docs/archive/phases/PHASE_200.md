# Phase 200 — Chapter Picket Finding 3: Closing the Team-Mission Gap

**Chapter Picket follow-up — [SHIPPED] 2026-09-06.**

## Goal (carried from Phase 199's known follow-up)

Phase 199 shipped `[agent] injection_scan_enabled`/`injection_scan_exempt`
but its final review found team missions build agents through a separate
path (`aivyx-team`'s `SpecialistFactory::build`) the branch never touched —
fail-safe (scan stays always-on) but a real coverage gap. Phase 199's own
retrospective estimated the fix at "`SpecialistFactory::new` constructed
from four separate files across `aivyx-team`." Phase 200 was scoped to
close that gap.

## What shipped

- **The Phase 199 retrospective's own estimate was corrected before any
  code was written.** Exhaustive re-grounding found 3 of those 4 files
  (`pool.rs`, `runtime.rs`, `testutil.rs`) are test-only fixtures inside
  `#[cfg(test)]`/`mod tests` blocks, not production code. The real
  production propagation was exactly **3** previously-uncovered
  construction paths: `build_agent_stack`'s two `AgentStackSpec` sites
  (`crates/aivyx-channel/src/session.rs`, `crates/aivyx-cli/src/bin/aivyx.rs`),
  the CLI's `aivyx team run` lead agent
  (`crates/aivyx-cli/src/bin/aivyx_modules/team.rs`), and every team
  specialist (`crates/aivyx-team/src/factory.rs`'s `SpecialistFactory::build`,
  reached from both the CLI's team path and the daemon's team-mission
  driver via `TeamAssembly::build()`).
- **Extended `TurnSafety`** (`crates/aivyx-core/src/agent.rs`) — the
  codebase's own existing choke point for per-turn safety knobs — to also
  carry `injection_scan_enabled: bool`/`injection_scan_exempt:
  BTreeSet<String>`. Both `interactive(...)` and `autonomous(...)` gained
  the two new parameters; `apply()` now calls the two `ConcreteAgent`
  builders Phase 199 added, alongside its existing cycle-detection and
  turn-timeout wiring.
- **Per the user's explicit choice, Phase 199's own two already-covered
  sites (`daemon_agent`, `child_agent`) were also converted** to route
  through the extended `TurnSafety` constructors, removing their
  now-redundant direct `.with_injection_scan_enabled`/
  `.with_injection_scan_exempt` calls — so all 6 real
  `TurnSafety::interactive`/`autonomous` call sites in the system
  (exhaustively grepped, not estimated) end up going through the exact
  same mechanism, no special cases.
- **`SpecialistFactory` gained the same two fields and builder methods**,
  mirroring its existing `checkpointer`/`with_checkpointer` pattern
  exactly, threaded through `TeamAssembly::build()` (which already took
  `checkpointer` as a direct parameter — the template for this change),
  `run_mission()` in the CLI's `team.rs`, and `TeamRunDeps` in the
  daemon's `team_mission_driver.rs` (which already had a `checkpointer`
  field to mirror).
- **The final whole-branch review (Opus) found a real, Critical security
  regression the branch itself introduced.** `TurnSafety` derived
  `Default`, and `bool::default()` is `false` — so `TurnSafety::default()`
  had `injection_scan_enabled: false`. Because this branch changed
  `apply()` to *unconditionally* write `injection_scan_enabled`, any code
  path constructing `TurnSafety::default()` directly (rather than via
  `.interactive()`/`.autonomous()`) now silently turned Chapter Picket's
  active scan **off**. Four live production paths do exactly this and
  were newly exposed by `apply()`'s changed semantics: the standalone
  Telegram (x2), Discord, and Slack channel-session construction sites —
  correctly out of this phase's stated scope (they never call
  `.interactive()`/`.autonomous()` at all, so they were never candidates
  for receiving the operator's config), but never audited against
  `apply()`'s new unconditional-write behavior either.
- **This was directly caused by a factually wrong claim in the approved
  design spec itself** — a spec-level miss, not an implementer error. The
  spec dismissed the derived-`Default` risk with "only relevant to any
  test construction that uses `TurnSafety::default()` directly … no such
  production call exists today." That claim was never checked against the
  channel-session crates and was wrong. Fixed with a hand-written
  `impl Default for TurnSafety` preserving `injection_scan_enabled: true`
  (and empty exempt set), with an in-code comment naming exactly which
  four production call sites depend on this default staying scan-on.
  Independently re-verified against all four call sites by reading their
  source directly, not by trusting the fix's own test claims.
- **Three Minor findings from the same review, fixed in the same commit**:
  `TurnSafety`'s struct-level doc comment described only 2 of its 4 knobs;
  a stale comment in `aivyx.rs` referenced a `.with_injection_scan_exempt`
  call site Task 2 of this phase had already deleted; three `aivyx-team`
  test fixtures used the non-default-preserving `false` instead of `true`
  for the two new values, inconsistent with `team_mission_driver.rs`'s own
  test fixtures.
- Full `cargo test` (120 result blocks across default-members, 0 failures)
  and `cargo clippy --all-targets -- -D warnings` (clean) re-verified on
  the branch after the fix, and again independently on `main` after the
  merge.

## The result

All 6 real `TurnSafety::interactive`/`autonomous` call sites in the system
— the daemon, the role-switch child, both channel-session construction
sites, the CLI's team-run lead, and every team specialist — now receive
the operator's `injection_scan_enabled`/`injection_scan_exempt` config
identically, through one mechanism. Chapter Picket's Finding 3 is now
fully closed. The phase also closed a self-introduced Critical regression
before merge (the silent scan-disable via `TurnSafety::default()`) that
would otherwise have shipped a real weakening of the standalone
Telegram/Discord/Slack channel paths.

## Known follow-ups (not done here, logged for whenever they matter)

- **The standalone Telegram/Discord/Slack channel-session paths never
  call `.interactive()`/`.autonomous()` at all** — they construct agents
  via `TurnSafety::default()` directly. This phase's fix keeps that
  default scan-on (preserving current behavior), but it means those four
  paths never receive *any* of the operator's `[agent]` config —
  `cycle_detection`/`turn_timeout_secs` too, not just the two injection-
  scan knobs. This predates this whole session's work and is out of
  scope for what this phase set out to do (propagate to the 6 real
  `.interactive()`/`.autonomous()` call sites), but it's a real gap:
  those four paths are presently un-configurable in ways an operator
  might reasonably expect to control.
- **No operator-facing documentation update for this phase's change** —
  `examples/aivyx.toml`/`CHANGELOG.md` still only reflect Phase 199's
  original two knobs; nothing about this phase changes their meaning for
  an operator, so no new doc debt was added, but the pre-existing gap
  logged in Phase 199 remains open.
