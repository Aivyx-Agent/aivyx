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

**Ship record:** `17cce6b`. Added `content_type` extraction from
response headers in `web_fetch.rs`. Output JSON now includes a
`"content_type"` field (nullable string). The status code was
already present in the output from Phase 12. This closes the
remaining Q3 half — the doc comment was updated from "audit-log-
only" to "content-type surfaced." Production-core `lib.rs`
untouched (the tool lives in `tools/web_fetch.rs`).

### Task 3 -- Provider-reported token usage in turn outcome

Surface token usage (input/output counts) reported by the LLM
provider in the turn outcome or audit event. This closes the
Phase 25 deferral.

**Ship record:** `fc4310f`. Added `TokenUsage` struct to
`aivyx-core/src/lib.rs` with `From<LlmUsage>` conversion.
`LlmPlanner` accumulates per-step usage in a new
`accumulated_usage` field; `TurnPlanner::turn_usage()` trait
method exposes it (defaults to zero for `VecPlanner`).
`AuditTag::TurnEnded` and `AuditEvent::TurnEnded` gain a `usage:
TokenUsage` field. The agent reads `planner.turn_usage()` at
turn exit and threads it into the audit event. Both providers
(Anthropic, OpenAI) already populated `LlmUsage` on
`LlmStepEnd`; the planner was previously ignoring it.

Files touched: `aivyx-core/src/lib.rs` (TokenUsage struct +
AuditTag), `aivyx-core/src/planner.rs` (trait method),
`aivyx-core/src/llm_planner.rs` (accumulation + impl),
`aivyx-core/src/agent.rs` (threading), `aivyx-audit/src/lib.rs`
(AuditEvent + bridge + tests), `aivyx-audit/src/persistent.rs`
(test helper), `aivyx-channel/src/turn_history_tool.rs`
(destructure update). 710 → 710 tests (no new tests; all
existing pass with the new field).

### Task 4 -- Second regression channel for role primitive

Add a minimal second channel adapter to the integration test
suite as a regression surface for the role-switching behavior.
This closes the Phase 11 Q6 deferral.

**Ship record:** `dcc7111`. Added three tests to
`aivyx-core/src/agent.rs` exercising the role-allowlist under a
`SemiTrusted` channel (`FakeChannel` with `ChannelPlatform::
Telegram` and `TrustTier::SemiTrusted`):

1. `semitrusted_channel_role_allowlist_permits_ceiling_included_tool`
   — `memory.read` (in SemiTrusted ceiling) succeeds through
   SemiTrusted channel with role allowlist.
2. `semitrusted_channel_ceiling_denies_role_allowed_tool` —
   `shell.exec` (not in SemiTrusted ceiling) is denied by the
   capability gate even when the role allowlist includes it.
3. `semitrusted_channel_records_narrowed_effective_caps_in_audit`
   — `TurnStarted` audit event reports SemiTrusted tier and
   narrowed effective capabilities after ceiling intersection.

Test delta: 710 → 713 (+3).

### Task 5 -- Stretch goals (if time permits)

MCP SSE transport or multi-level nesting, depending on which
feels more natural after Tasks 2–4.

**Outcome:** Not attempted. All Tier 1 items closed cleanly;
Tier 2 items are better scoped as their own phase.

### Task 6 -- Exit freeze + docs

Exit criteria checklist, prediction-vs-reality table, streak
report. Record deferral backlog delta.

## Exit criteria

- [x] Task 1 shipped: Phase 31 scaffold, README + ROADMAP
      updated.
- [x] Task 2 shipped at `17cce6b`: content-type in web.fetch
      output. Phase 12 Q3 deferral closed.
- [x] Task 3 shipped at `fc4310f`: TokenUsage in TurnEnded
      audit event. Phase 25 deferral closed.
- [x] Task 4 shipped at `dcc7111`: SemiTrusted regression
      tests for role primitive. Phase 11 Q6 deferral closed.
- [x] Task 5 skipped: stretch goals not attempted.
- [x] Task 6: this section.
- [x] 713 tests, 0 failures.
- [x] `cargo check` clean (only pre-existing MCP warnings).
- [x] DESIGN.md untouched.
- [x] PRODUCT.md untouched.

## Prediction vs reality

| Prediction | Reality | Notes |
|---|---|---|
| DESIGN.md untouched | Untouched | Correct |
| PRODUCT.md untouched | Untouched | Correct — streak extends to 1 |
| Production-core may break (streak 2) | **Broken** at Task 3 | `TokenUsage` + `AuditTag::TurnEnded.usage` added. Streak resets to 0 |

## Streak report

| Target | Streak at entry | This phase | Streak at exit |
|---|---|---|---|
| DESIGN.md | extends | untouched | extends |
| PRODUCT.md | 0 (reset Phase 30) | untouched | 1 |
| Production-core `lib.rs` | 2 | **broke** (Task 3) | 0 |

## Rolling deferrals at Phase 31 exit (8 items, -3 closed)

**Closed this phase:**
- Response headers in web.fetch output (Phase 12 Q3) — Task 2
- Provider-reported token usage (Phase 25) — Task 3
- Second regression channel for role primitive (Phase 11 Q6)
  — Task 4

**Remaining (8 items):**
- **Non-GET verbs (POST/PUT/PATCH/DELETE)** — Phase 12 Q1.
  Deferred indefinitely.
- **Redirect following with per-hop scope re-check** —
  Phase 12 Q5. Deferred indefinitely.
- **Binary response bodies / non-UTF-8** — Deferred
  indefinitely.
- **Per-chunk Telegram rendering** — Phase 12 Task 1.
  Deferred reactively.
- **Multi-level sub-agent nesting** — Phase 14 Task 3.
  Untouched.
- **LocalChannel regression-test rewrite over IPC** —
  Phase 17 Q6->(c+). Tagged: reactive.
- **Telegram-specific protocol extensions** — Phase 19.
  Untouched.
- **MCP SSE transport** — Phase 23. Untouched.
