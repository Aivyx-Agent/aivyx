# Phase 33 — Multi-Level Sub-Agent Nesting

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../../../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../../../PRODUCT.md).

## Goal

Close the Phase 14 Task 3 deferral: enable multi-level
sub-agent nesting so a child agent can invoke `role.switch`
recursively, spawning a grandchild (and deeper). The
architecture already supports this — the factory closure
passes the same `ToolRegistry` (containing `RoleSwitchTool`
with its installed factory) to every child. The blocker is
purely config-level: no child role currently declares
`role.switch` in its `capability_scopes`.

## Why now

1. **Oldest active deferral.** Multi-level nesting was
   deferred in Phase 14 Task 3 (April 16) with the note
   "deferred until a concrete use case surfaces." The
   `examples/aivyx.toml` hierarchy already contains the
   natural chain: `default` → `coder` → `researcher` →
   `junior_researcher`. Adding `role.switch:junior_researcher`
   to `researcher` demonstrates the full depth.

2. **Zero architecture changes needed.** The recursive
   factory reuse was validated at Phase 14 Task 3: the child
   `ConcreteAgent` receives `Arc::clone(&tools_for_factory)`,
   which is the same `ToolRegistry` the parent uses. The
   `RoleSwitchTool` inside it has the factory already installed
   via `OnceLock`. No new code paths are required in the
   binary, the tool, or the capability system.

3. **Testing + config is the work.** The phase is primarily
   about: (a) updating `examples/aivyx.toml` to grant
   `researcher` the `role.switch:junior_researcher` scope,
   (b) writing integration tests that prove 2-level nesting
   works, (c) testing the negative case (grandchild cannot
   nest further), and (d) verifying the audit chain shows
   correct role/turn identifiers at each depth.

## Streak predictions

- **DESIGN.md** -- Very low risk. No architecture change.
  Prediction: **untouched**.

- **PRODUCT.md** -- Very low risk. P1 already covers
  sub-agent role-switching; multi-level nesting is a
  fulfillment, not a new commitment. Prediction:
  **untouched** (streak at 2 from Phase 32).

- **Production-core `aivyx-core/src/lib.rs`** -- Very low
  risk. `RoleSwitchTool` is the mechanism and it lives in
  `aivyx-core/src/tools/role_switch.rs`, not `lib.rs`. The
  change is config + tests. Prediction: **untouched**
  (streak at 1 from Phase 32).

## Architecture

No architecture changes. The recursive nesting is a
consequence of existing design decisions:

1. **Factory reuse**: `child_factory` closure captures
   `Arc::clone(&tools_for_factory)`. The `ToolRegistry`
   contains `RoleSwitchTool`. The `RoleSwitchTool` has the
   factory installed via `OnceLock::set`. Therefore every
   child agent built by the factory has access to
   `role.switch`.

2. **Capability-bounded recursion**: A child can only
   invoke `role.switch` if its effective envelope contains a
   `role.switch:<target>` scope. Each level attenuates via
   `assemble_role_envelope` intersection — the capability
   set can only narrow, never widen. Recursion terminates
   when a role's envelope has no `role.switch` scope.

3. **Audit chain**: Each `child.turn()` emits its own
   `TurnStarted`/`TurnEnded` audit events with a distinct
   `TurnId` and `AgentId`. A 3-level chain produces three
   nested turn pairs: parent → child → grandchild →
   grandchild-end → child-end → parent-continues.

## Tasks

### Task 1 -- Open commit + PHASE_33.md scaffold

This file. Update `docs/README.md` to show Phase 33 as Open.
Update `docs/ROADMAP.md` to mark Phase 32 as Frozen and add
Phase 33 active pointer.

### Task 2 -- Config: grant researcher role.switch:junior_researcher

Update `examples/aivyx.toml` to add `role.switch:junior_researcher`
to `researcher`'s `capability_scopes` and `role.switch` to its
`tool_allowlist`. Update existing tests that pin `researcher`'s
envelope to include the new scope.

**Ship record:** `075a00b`. Added `role.switch:junior_researcher`
to `researcher`'s `capability_scopes` and `role.switch` to its
`tool_allowlist`. Updated four test sites that pin researcher's
envelope: binary-internal, cross-crate envelope, cross-crate
render (envelope contents + reachable targets section). The
`--print-role researcher` render test now asserts
`junior_researcher` is listed as a reachable target (was:
`<none>`).

### Task 3 -- Integration tests for multi-level nesting

Write integration tests in `crates/aivyx-channel/tests/` that
exercise the 2-level nesting chain through the capability
system.

**Ship record:** `bb4ea33`. Four new integration tests in
`multilevel_nesting_e2e.rs`:
- `nesting_chain_role_switch_narrows_at_each_level`: walks
  coder → researcher → junior_researcher, verifying
  `role.switch` narrows at each depth and terminates at leaf.
- `inheritance_chain_attenuation_researcher_to_junior`: proves
  junior's envelope is strictly narrower than researcher's
  (scope count and base coverage).
- `backcompat_floor_does_not_contain_role_switch`: pins that
  the floor cannot leak nesting ability to empty-child roles.
- `default_unqualified_role_switch_reaches_all_chain_members`:
  verifies root can reach any role including the deepest leaf.

### Task 4 -- Exit freeze + docs

Exit criteria checklist, prediction-vs-reality table, streak
report. Close the Phase 14 Task 3 deferral in the rolling
backlog.

## Exit criteria

- [x] Task 1 shipped: Phase 33 scaffold, README + ROADMAP
      updated.
- [x] Task 2 shipped at `075a00b`: researcher granted
      `role.switch:junior_researcher`, existing tests updated.
- [x] Task 3 shipped at `bb4ea33`: 4 new integration tests.
- [x] Task 4: this section.
- [x] 740 tests, 0 failures.
- [x] `cargo check` clean (only pre-existing MCP warnings).
- [x] DESIGN.md untouched.
- [x] PRODUCT.md untouched.
- [x] Production-core `aivyx-core/src/lib.rs` untouched.

## Prediction vs reality

| Prediction | Reality | Notes |
|---|---|---|
| DESIGN.md untouched | Untouched | Correct |
| PRODUCT.md untouched | Untouched | Correct — streak extends to 3 |
| Production-core untouched (streak 1) | Untouched | Correct — streak extends to 2 |

## Streak report

| Target | Streak at entry | This phase | Streak at exit |
|---|---|---|---|
| DESIGN.md | extends | untouched | extends |
| PRODUCT.md | 2 | untouched | 3 |
| Production-core `lib.rs` | 1 | untouched | 2 |

## Rolling deferrals at Phase 33 exit (6 items, -1 closed)

**Closed this phase:**
- Multi-level sub-agent nesting (Phase 14 Task 3) — Tasks 2–3

**Remaining (6 items):**
- **Non-GET verbs (POST/PUT/PATCH/DELETE)** — Phase 12 Q1.
  Deferred indefinitely.
- **Redirect following with per-hop scope re-check** —
  Phase 12 Q5. Deferred indefinitely.
- **Binary response bodies / non-UTF-8** — Deferred
  indefinitely.
- **Per-chunk Telegram rendering** — Phase 12 Task 1.
  Deferred reactively.
- **LocalChannel regression-test rewrite over IPC** —
  Phase 17 Q6->(c+). Tagged: reactive.
- **Telegram-specific protocol extensions** — Phase 19.
  Untouched.
