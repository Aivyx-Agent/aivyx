# Phase 38 — Foundation Audit

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

After 37 phases of forward implementation, pause and audit
the accumulated codebase (55,860 lines, 11 crates, 788 tests)
for structural health, contract compliance, and accumulated
warnings. No new features — only corrections, classification,
and cleanup.

## Why now

1. **P10 compliance gap.** Phase 37 added `web.post` as a new
   tool alongside the seven substrate tools, but didn't amend
   P10 or classify `web.post` in the substrate/infrastructure/
   third-party taxonomy. By P10's own rules, adding a new
   operator-facing tool to core requires a `PRODUCT.md`
   amendment. The longer this goes unresolved, the more likely
   future phases treat the taxonomy as advisory rather than
   binding.

2. **Clippy warning accumulation.** 13 warnings across 4
   crates — dead code in `aivyx-mcp`, style issues in
   `aivyx-channel`, function argument counts. All minor
   individually but collectively they signal drift from
   "zero-warning codebase."

3. **Delivery status staleness.** `PRODUCT.md`'s Delivery
   Status section is frozen at Phase 35 (2026-04-19). Two
   phases have shipped since, including a new tool (`web.post`)
   that changes the P10 narrative.

4. **Deferral backlog at 3.** All three remaining deferrals
   are stale entries from early phases that may no longer be
   relevant or may need reframing.

## Tasks

### Task 1 — Open commit + PHASE_38.md scaffold

This file. Update `docs/README.md` to show Phase 38 as Open.
Update `docs/ROADMAP.md` with Phase 38 entry.

### Task 2 — P10 amendment: web.post classification

Decide whether `web.post` is substrate (requires P10
amendment to "eight tools") or can be reclassified as
infrastructure. Write the amendment if substrate.

Key arguments for substrate:
- Same shape as `web.fetch` — operator-facing, not self-
  management. The taxonomy says infrastructure is "tools
  the agent uses to manage itself."
- Has its own `Tool::name()`, scope base, and registration.
- An operator configuring role capabilities would expect to
  see `net.post` alongside `net.fetch`.

Key arguments for infrastructure:
- `web.post` is a *complement* to `web.fetch`, not a new
  surface area. The capability is "network access," which
  was already substrate.
- The operator doesn't discover `web.post` independently —
  it ships wherever `web.fetch` ships.

### Task 3 — Clippy warning cleanup

Fix all 13 clippy warnings:
- `aivyx-mcp`: 5 dead-code warnings (deserialized-but-unused
  fields in MCP protocol structs)
- `aivyx-channel`: collapsible if, unnecessary closure,
  too-many-args (x2), owned-for-comparison, field-assignment
  style (x2)
- `aivyx-channel` tests: 2 unused assignments

### Task 4 — PRODUCT.md Delivery Status refresh

Update the Delivery Status section from Phase 35 to Phase 37:
- Record `web.post` addition (per Task 2's resolution)
- Update Phase 37's web-fetch hardening deliverables
- Refresh deferral backlog (6 → 3)

### Task 5 — Deferral backlog review

Reassess the 3 remaining deferrals:

1. **Rendering parity (terminal vs Telegram formatting)**
   — **Closed: resolved by design.** Both channels render
   via `StreamEvent` polymorphism. The asymmetry (Local
   streams `ToolOutput` chunks live; Telegram defers to
   aggregated `ToolCallFinished` to avoid API cost) is
   intentional Phase 12 design, not a bug. Documented in
   `TelegramChannel::stream_event` as trust-tier asymmetry.

2. **Integration test infra (E2E harness for full turns)**
   — **Closed: implemented.** 10 E2E test files in
   `crates/aivyx-channel/tests/` exercise the full turn
   loop with real tool dispatch: `fs_tool_e2e.rs`,
   `memory_tool_e2e.rs`, `daemon_roundtrip_e2e.rs`,
   `multilevel_nesting_e2e.rs`, `role_envelope_e2e.rs`,
   etc. The deferral was overtaken by incremental
   implementation across Phases 17–33.

3. **Protocol versioning (MCP/SSE format negotiation)**
   — **Reframed: keep as sole remaining deferral.** Version
   constants exist (`PROTOCOL_VERSION = "0.1"` in daemon
   IPC, `"2024-11-05"` in MCP) but no negotiation logic
   validates them. Acceptable for single-version deployment;
   becomes relevant when MCP servers evolve or when the
   daemon needs backward compatibility.

**Net result:** Deferral backlog drops from 3 to 1.

### Task 6 — Exit freeze + docs

## Exit criteria

- [x] P10 amendment filed (Amendment A5, `web.post` added
      to substrate tool list, seven → eight).
- [x] Clippy warnings eliminated (13 → 0 across 4 crates).
- [x] PRODUCT.md Delivery Status refreshed (Phase 35 → 38).
- [x] Deferral backlog reviewed and reduced (3 → 1).
- [x] All 788 tests pass (unchanged from Phase 37 exit).
- [x] `DESIGN.md` untouched (streak at 14 from Phase 25).
- [x] `PRODUCT.md` touched in Tasks 2 + 4 (P10 amendment +
      delivery status refresh). Streak resets to 0.
- [x] `aivyx-core/src/lib.rs` untouched (streak at 1 from
      Phase 38).

## Streak predictions + reality

| Streak target | Predicted | Reality | Notes |
|---|---|---|---|
| DESIGN.md | untouched (14) | untouched | continues |
| PRODUCT.md | touched (0) | touched | P10 amendment + delivery status |
| lib.rs | untouched (1) | untouched | continues |

## Ship records

- **Task 1** `27e1c74` — open commit, PHASE_38.md scaffold,
  README.md + ROADMAP.md updates.
- **Task 2** `7798254` — Amendment A5: P10 substrate tool
  count (7 → 8, web.post).
- **Task 3** `90cbbeb` — clippy warning cleanup (13 → 0,
  9 files across 4 crates).
- **Task 4** `54a4c83` — PRODUCT.md Delivery Status refresh
  (Phase 35 → Phase 38).
- **Tasks 5+6** `b596207` — deferral backlog review
  (3 → 1) + exit freeze.
