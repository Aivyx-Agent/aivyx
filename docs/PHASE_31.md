# Phase 31 — Deferral Cleanup

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Reduce the rolling deferral backlog (currently 11 items). This
is a non-product-shape cleanup phase (same category as Phase 15
and Phase 20). No new forward commitments; focus is on closing
existing items that have clear scope and no external blockers.

## Why now

1. **The backlog has been at 11 since Phase 28.** Phases 28–30
   focused on the Reflection Layer and added no new deferrals
   but also closed none. The backlog has been growing steadily
   since Phase 21 (10 → 12 → 13 → 14 → 13 → 11) and a focused
   cleanup pass is overdue.

2. **No product-shape keystones are blocked.** P8 (Reflection)
   just shipped. The remaining milestones (Web UI, Channel SDK,
   Tool Process IPC, SDK Docs) are new work, not deferral debt.
   Cleaning up now avoids carrying stale items into those phases.

## Streak predictions

- **DESIGN.md** -- Low risk. Cleanup items are implementation,
  not architecture. Prediction: **untouched**.

- **PRODUCT.md** -- Low risk. No product-shape changes expected.
  Prediction: **untouched** (streak begins at 0 after Phase 30
  reset).

- **Production-core `aivyx-core/src/lib.rs`** -- Moderate risk.
  Response headers in audit or token counting may touch core
  types. Prediction: streak **may break** (currently at 2).

## Target items

### Tier 1 — Readily closable

1. **Response headers in audit payload** (Phase 12 Q3 half).
   Add response status code and selected headers to the
   `web.fetch` tool's audit event or output.

2. **Provider-specific token counting** (Phase 25). The
   `LlmProvider` trait currently has no token-counting method.
   Anthropic and OpenAI count tokens differently. Add a
   `count_tokens` or similar method, or at minimum surface
   the provider's reported usage in the turn outcome.

3. **Second regression channel for the role primitive** (Phase
   11 Q6). Add a second test channel adapter (beyond
   `LocalChannel`) to the integration test suite as a regression
   surface for role-switching behavior.

### Tier 2 — Moderate effort (stretch goals)

4. **MCP SSE transport** (Phase 23). HTTP/SSE transport for
   remote MCP servers, complementing the existing stdio
   transport.

5. **Multi-level sub-agent nesting** (Phase 14 Task 3).
   Recursive `role.switch` — a child sub-session can itself
   invoke `role.switch` if its role declares the scope.

### Not targeted this phase

- Non-GET verbs (POST/PUT/PATCH/DELETE) — deferred indefinitely
- Redirect following with per-hop scope re-check — deferred
  indefinitely
- Binary response bodies / non-UTF-8 — deferred indefinitely
- Per-chunk Telegram rendering — reactive
- LocalChannel regression-test rewrite over IPC — reactive
- Telegram-specific protocol extensions — no use case yet

## Tasks

### Task 1 -- Open commit + PHASE_31.md scaffold

This file. Update `docs/README.md` to show Phase 31 as Open.
Update `docs/ROADMAP.md` with Phase 31 active pointer.

### Task 2 -- Response headers in web.fetch output

Surface HTTP response status code and content-type header in
the `web.fetch` tool's output JSON. This closes the Phase 12
Q3 deferral.

### Task 3 -- Provider-reported token usage in turn outcome

Surface token usage (input/output counts) reported by the LLM
provider in the turn outcome or audit event. This closes the
Phase 25 deferral.

### Task 4 -- Second regression channel for role primitive

Add a minimal second channel adapter to the integration test
suite as a regression surface for the role-switching behavior.
This closes the Phase 11 Q6 deferral.

### Task 5 -- Stretch goals (if time permits)

MCP SSE transport or multi-level nesting, depending on which
feels more natural after Tasks 2–4.

### Task 6 -- Exit freeze + docs

Exit criteria checklist, prediction-vs-reality table, streak
report. Record deferral backlog delta.
