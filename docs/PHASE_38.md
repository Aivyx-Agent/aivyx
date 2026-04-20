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
1. Rendering parity (terminal vs Telegram formatting)
2. Integration test infra (E2E harness for full turns)
3. Protocol versioning (MCP/SSE format negotiation)

For each: is it still relevant? Should it be reframed? Can
it be closed as "won't do" or "already addressed"?

### Task 6 — Exit freeze + docs

Exit criteria checklist, prediction-vs-reality table,
streak report.

## Streak predictions

- **DESIGN.md** — Low risk. No architecture change expected.
  Prediction: **untouched** (streak at 14 from Phase 38).

- **PRODUCT.md** — **Will be touched** in Tasks 2 and 4
  (P10 amendment + delivery status refresh).
  Prediction: **touched** (streak resets to 0).

- **Production-core `aivyx-core/src/lib.rs`** — Very low risk.
  Clippy warnings are in other crates. No tool changes.
  Prediction: **untouched** (streak at 1 from Phase 38).
