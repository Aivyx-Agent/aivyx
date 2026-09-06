# Phase 199 — Chapter Picket Finding 3: the Config-Knob Half

**Chapter Picket follow-up — [SHIPPED] 2026-09-06.**

## Goal (carried from Phase 198's deferred follow-up)

Phase 198 closed Finding 3's coverage half (28 productivity-integration
tools flagged as untrusted) and explicitly deferred the other half:
"no operator-facing config knob to disable or tune the tripwire,
independent of what content gets scanned." Phase 199 was scoped to
close that remaining half.

## What shipped

- **`[agent] injection_scan_enabled`** (bool, default `true`) — global
  on/off for Chapter Picket's active scan (`check_for_injection`).
  **`[agent] injection_scan_exempt`** (list of tool names, default
  empty) — per-tool exemption, for an operator who wants the scan on
  everywhere except one noisy tool rather than turning it off
  entirely. Brainstorming surfaced this richer shape over a plain
  boolean — the user's explicit choice, since the original finding
  said "disable *or tune*."
- Threaded through three layers, each mirroring a real, existing
  precedent exactly: `Sourced<bool>` + plain `Vec<String>` on
  `AivyxConfig` (matching `require_enforcement`/`allow_sensitive_paths`);
  two plain fields + builder methods on `ConcreteAgent` (matching
  `tool_allowlist`/`checkpointer`); two local unwraps threaded into the
  daemon's `ConcreteAgent::new` construction sites (matching how
  `require_enforcement`/`guard_sensitive_paths` already reach that
  scope). Only `check_for_injection` is gated — Chapter Bulwark's
  passive fencing (`fence_untrusted_output`) stays unconditional in
  every case, confirmed by the final review tracing it to its one real
  call site and finding the diff never touches it.
- **The final whole-branch review (Opus) independently re-derived a
  manual config-load trace** (a realistic `[agent]` TOML block, walked
  by hand through the loader code) rather than trusting the new unit
  tests' own assertions, and confirmed the exact type chain across all
  three crate boundaries (`Sourced<bool>` → `bool`, `Vec<String>` →
  `BTreeSet<String>`, exactly once) has no silent mismatch anywhere.
- **The review also found the design spec's own premise was factually
  wrong**: it stated "two real `ConcreteAgent::new(...)` construction
  sites exist in this binary," but team missions — both the CLI's
  `aivyx team run` and the daemon's team-mission driver — build agents
  through a completely separate path (`aivyx-team`'s
  `SpecialistFactory::build`) that this branch never touched. The two
  new knobs currently do not apply to team missions at all. This is
  fail-safe (uncovered agents simply keep the scan always-on — nothing
  is weakened), but it means an operator who sets
  `injection_scan_exempt` to quiet a noisy productivity-integration
  tool would find it silently doesn't apply once that same tool is
  called from inside a team mission.
- **Further grounding after the review revealed the real fix is a
  multi-crate propagation, not a quick patch.** The codebase already
  has a purpose-built answer to exactly this class of bug —
  `TurnSafety` (`crates/aivyx-core/src/agent.rs`), whose own doc
  comment says it exists because "that ad-hoc duplication is exactly
  what previously left the daemon, the role-switch child, and the team
  agents unprotected." Moving the two new knobs onto `TurnSafety`
  instead of individual per-site builder calls would close this gap
  structurally, for every current and future construction site at
  once — but `TurnSafety::autonomous()` (the posture team agents use)
  is currently called with zero parameters from two sites
  (`crates/aivyx-cli/src/bin/aivyx_modules/team.rs:308` and
  `crates/aivyx-team/src/factory.rs:214`), and `SpecialistFactory::new`
  (which owns that second call) is itself constructed from four
  separate files across `aivyx-team` (`assembly.rs`, `pool.rs`,
  `runtime.rs`, `testutil.rs`), each needing the two new values
  propagated through its own constructor chain back to `aivyx-cli`'s
  `team.rs` and the daemon's `team_mission_driver.rs`. Given the size
  this turned out to have, the user explicitly chose to scope it as
  its own separate follow-up phase rather than pull it into this
  branch.
- Four Minor findings from the same review were accepted as-is,
  genuinely cosmetic/documentation-only with no correctness impact:
  no operator-facing docs for either new key in `examples/aivyx.toml`
  or `CHANGELOG.md`; one `docs/THREAT_MODEL.md` line now slightly
  overstates the scan as unconditional rather than default-on;
  `.with_injection_scan_exempt(injection_scan_exempt.clone())`'s
  `.clone()` at its last use site is redundant (mirrors the adjacent
  `checkpointer.clone()` style, so left as-is); and one new test's name
  (`injection_scan_still_fires_for_non_exempt_tools_when_others_are_exempt`)
  promises slightly more than its own body verifies (the "others are
  exempt" half is covered by a sibling test, not itself).

## The result

An operator can now turn Chapter Picket's active scan off globally, or
exempt specific noisy tools by name, without touching Bulwark's
passive fencing — for every REPL, daemon, and role-switch-child agent.
Chapter Picket's Finding 3 is now fully closed for that surface;
what's left is a real, precisely-scoped gap for team missions
specifically, logged below rather than guessed at.

## Known follow-ups (not done here, logged for whenever they matter)

- **Extend `TurnSafety` to carry `injection_scan_enabled`/
  `injection_scan_exempt` and thread them through every
  `SpecialistFactory` construction path**, closing the team-mission gap
  described above. Concretely: add the two fields to `TurnSafety`,
  change `interactive(...)` and `autonomous(...)` to accept them, update
  `apply()` to call the two `ConcreteAgent` builders, and propagate the
  two config values through `SpecialistFactory::new`'s four call sites
  (`aivyx-team/src/assembly.rs`, `pool.rs`, `runtime.rs`, `testutil.rs`)
  back to `aivyx-cli/src/bin/aivyx_modules/team.rs:308` and the daemon's
  `crates/aivyx-channel/src/team_mission_driver.rs`. Real, multi-crate
  work — deserves its own brainstorm→spec→plan cycle, not a quick patch.
  Fail-safe in the meantime: team missions simply keep scanning
  everything.
- **No operator-facing documentation for either new key** —
  `examples/aivyx.toml` (which documents `require_enforcement` and
  other comparable knobs in full prose) and `CHANGELOG.md`'s
  `[Unreleased]` section both currently say nothing about
  `injection_scan_enabled`/`injection_scan_exempt`. Cheap to add
  whenever picked up; mixed precedent exists (`cycle_detection`/
  `turn_timeout_secs` are likewise undocumented in `examples/aivyx.toml`
  today).
- **`docs/THREAT_MODEL.md`'s §5.3** now reads as though the active scan
  always fires; a one-clause update noting the new knob (and that
  Bulwark's fencing specifically is never disableable) would keep it
  exactly accurate.
- **The config-knob half's own scope was intentionally narrow**: a
  plain on/off plus a name-exact exemption list, no marker-list
  editing. The marker-list-expansion follow-up already logged in
  Chapter Picket's own section of `docs/ROADMAP.md` remains separately
  open and untouched by this phase.
