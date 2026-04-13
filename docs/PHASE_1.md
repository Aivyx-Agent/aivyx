# Phase 1 — First Turn Loop (ACTIVE)

**Status:** Active — opened 2026-04-13
**Predecessor:** [PHASE_0.md](PHASE_0.md) (exit commit `1b4f271`)
**Contract:** [`../DESIGN.md`](../DESIGN.md) (Deliverables 1–8, all LOCKED)

This is the active working doc for Phase 1. It is edited freely
during the phase and frozen at phase exit. Contract changes go
through the amendment process described in [README.md](README.md).

## Goal

Produce the **first compiling turn-loop skeleton** — the code path
that takes a `Message` in from a `ChannelContext` and returns a
`TurnOutcome`, with capability checks enforced, even if no real
tools or LLM providers exist yet. This is the structural proof that
the Phase 0 contract can carry real execution.

Phase 1 is **not** about shipping a runnable agent. It is about
wiring `aivyx-core`, `aivyx-capability`, `aivyx-channel`, and
`aivyx-audit` together until a test can drive a fake agent through
a fake channel and observe the correct `TurnOutcome` and audit trail.

## Entry criteria (inherited from Phase 0 exit)

- [x] All 8 Phase 0 deliverables LOCKED
- [x] 9-crate workspace compiles green
- [x] Exit commit `1b4f271` on `main`
- [x] No contract changes pending from Phase 0

## Known refinements queued from Phase 0

These are extensions Phase 0 flagged but didn't make. Not contract
breaks — implementation details that become concrete in Phase 1.

### R1 — `Tool::required_scope` takes input

**Current (D3):** `fn required_scope(&self) -> Scope`
**Target:** `fn required_scope(&self, input: &Value) -> Scope`

Rationale: static scope declaration can't express input-dependent
requirements. `memory.read` needs `memory.read.session:<id>` where
`<id>` comes from the tool input. Same for `fs.read` — the scope is
`fs.read:<path>`, which depends on the path argument. Making the
signature input-aware lets the scope check fire on *derived* scopes,
not just the tool's nominal scope.

**When to land:** before writing the first `Tool` impl in Phase 1,
so no tool is ever written against the old signature.

### R2 — Window-control scopes stay Reserved

D1's Scenario 2 (close terminal window) implies `display.window_close`
and friends, which D4 left in the Reserved section. Phase 1 does
*not* promote them yet — the tool crate that implements window
control is downstream work, and it will propose the concrete scope
taxonomy then.

**When to land:** when a window-control tool is first implemented.
Not in Phase 1 unless the turn-loop test requires it (it should not).

## Phase 1 task list

*Filled in as the phase opens. Empty entries are intentional —
tasks added only when the preceding ones are concrete.*

- [ ] Stand up `aivyx-capability` real types (`Scope`, `CapabilitySet`,
      `TrustTier`) replacing the D8 stubs, with prefix-attenuation
      intersection logic + unit tests
- [ ] Stand up `aivyx-audit` real types (`AuditEvent`, `AuditLog` trait,
      HMAC chain impl) with the 5 event variants from D4
- [ ] Land R1: change `Tool::required_scope` signature in
      `aivyx-core`
- [ ] Wire `aivyx-core` turn loop against capability + audit, with a
      fake `Tool` impl and a fake `ChannelContext` impl for testing
- [ ] Write the first end-to-end test: fake channel sends a message,
      turn loop runs, correct `TurnOutcome` + correct audit entries
- [ ] (Exit) Document Phase 1 outcome + known issues in this file,
      freeze it, open `PHASE_2.md`

## Open questions

*Questions that come up during implementation land here until
they're answered. An open question at phase exit is either
answered, deferred to a later phase with a clear owner, or promoted
to a DESIGN.md amendment.*

- None yet.

## Exit criteria (to be firmed up)

Draft — will be revised once the task list stabilizes:

- [ ] `aivyx-core` turn loop compiles against real
      `aivyx-capability` and `aivyx-audit` (not stubs)
- [ ] End-to-end test: fake channel → turn loop → fake tool →
      `TurnOutcome::Success` with audit trail verified
- [ ] End-to-end test: scope-denied case returns
      `TurnOutcome::ScopeDenied` and emits the right audit event
- [ ] All Phase 0 contract shapes still compile without modification
      (no silent drift — verified by `git diff DESIGN.md` being
      empty, or by an amendment file existing if not)
