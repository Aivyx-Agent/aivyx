# Phase 28 — Deferral Cleanup + Reflection Foundation

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

A **hybrid phase**: close the three most impactful rolling
deferrals (trimming the backlog from 13 → 10), then lay the
first Reflection Layer primitive — a read-only audit
introspection surface the agent can query to observe its own
recent turn outcomes. This phase advances **PRODUCT.md G3 —
Memory Reflection** by providing the observation substrate the
reflection loop will compose against in a future phase.

## Why now

1. **The deferral backlog has been creeping up.** From 8 items at
   Phase 15 to 13 at Phase 27 exit. The last cleanup pass was
   Phase 20 (closed 6 items, twelve phases ago). Three items are
   genuinely impactful: `mission.list`/`mission.status` tools,
   webhook port configurability, and either MCP SSE transport or
   provider-specific token counting.

2. **G5 is delivered; G3 is the next undelivered goal.** The
   trigger substrate from Phases 26–27 provides the scheduling
   machinery the Reflection Layer needs. The missing piece is an
   audit introspection surface — without it, the agent can only
   write to the audit chain, never read from it.

3. **The audit query primitive is a natural follow-on from the
   trigger work.** A reflection trigger (schedule or file-watch)
   needs something to reflect *on*. The audit chain is the right
   source — it captures every tool call, scope check, and turn
   outcome.

## Streak predictions

- **DESIGN.md** — Low risk. Deferral closures and audit query
  are daemon/channel-level work. Prediction: **untouched**.

- **PRODUCT.md** — Low risk. G3 is already committed; this
  phase begins delivery. Prediction: **untouched**.

- **Production-core `aivyx-core/src/lib.rs`** — Low risk.
  Mission tools follow existing OnceLock patterns. Audit query
  reads from storage, not core. Prediction: streak **extends
  to nineteen**.

## Open questions

**Q1 — Which three deferrals to close?** The three most impactful
from the 13-item backlog:
  (a) `mission.list` / `mission.status` read-only tools — operators
      currently can't inspect missions without raw storage access.
      Small, follows existing OnceLock pattern exactly.
  (b) Webhook port configurability — `[daemon] webhook_port` in
      config. Tiny change, closes a Phase 27 decision item.
  (c) `mission.list` + `mission.status` as one task, webhook port
      config as a second, and a third from: MCP SSE transport
      (Phase 23, larger), provider-specific token counting
      (Phase 25, medium), or forensic `ToolOutcome::NotInRole`
      (Phase 11, small). Leaning **(c)** with the forensic
      variant as the third — it's the oldest deferral and small.

**Q2 — Audit query shape?** (a) A `turn.history` tool that returns
recent `TurnStarted`/`TurnEnded` pairs with outcome summaries —
compact, focused on what the reflection loop needs. (b) A more
general `audit.query` tool that can filter by event kind, time
range, etc. — more powerful but larger surface. Leaning **(a)**
for this phase — the general query can come later if needed.

**Q3 — Scope for audit query tool?** The `audit.read` scope
already exists and is in `CEILING_SEMITRUSTED`. A `turn.history`
tool scoped under `audit.read` fits naturally. Leaning **(a)**
— reuse `audit.read`.

## Tasks

### Task 1 — Open commit + PHASE_28.md scaffold

This file. Update `docs/README.md` to show Phase 28 as Open.
Update `docs/ROADMAP.md` with Phase 28 active pointer.

### Task 2 — `mission.list` and `mission.status` tools

Two new tools following the OnceLock pattern:
- `mission.list` — returns all missions with state, gate count,
  and timestamps. Scoped to `mission.create` (same tier as create).
- `mission.status` — returns a single mission by ID with full
  gate details. Scoped to `mission.create`.

Closes the Phase 21 deferral: "`mission.list` / `mission.status`
read-only tools."

### Task 3 — Webhook port configurability

Add `[daemon] webhook_port` to the config surface. Default
remains 7842. Thread through to the webhook listener startup.
Closes the Phase 27 decision item about hardcoded port.

### Task 4 — Forensic `ToolOutcome::NotInRole` variant

Add a `NotInRole` variant to `ToolOutcomeSummary` in `aivyx-core`
for when a tool call is attempted but the tool isn't in the
active role's allowlist. Currently these are silent drops — the
forensic variant makes them visible in the audit chain.
Closes the Phase 11 Q1 deferral (oldest item in the backlog).

Note: this task will break the production-core streak (modifies
`aivyx-core/src/lib.rs`). Acceptable because the streak is at
eighteen and the change is a single enum variant addition.

### Task 5 — Audit introspection: `turn.history` tool

A read-only tool that queries the persistent audit chain and
returns recent turn outcomes. Input: optional `limit` (default
10) and optional `since_ms` (epoch millis). Output: array of
`{ turn_id, session_id, channel, outcome, tool_calls_made,
duration_ms, started_at }` objects.

Scoped to `audit.read`. Follows the OnceLock pattern with a
`KeyDomain::Audit` handle.

This is the **first Reflection Layer primitive** — the
observation surface the agent needs before it can reflect on
its own behavior.

### Task 6 — Exit freeze + docs

Exit criteria checklist, prediction-vs-reality table, streak
report, ROADMAP.md update, README.md status update. Record
deferral backlog delta.

---

## Ship records

### Task 1 — Open commit

Opened Phase 28 journal, updated docs/README.md (Phase 28 Open),
docs/ROADMAP.md (Phase 28 active pointer), docs/PRODUCT_ROADMAP.md
(Phase 28 reflection entry).

### Task 2 — `mission.list` and `mission.status` tools

Two new tools in `mission_tool.rs` following the OnceLock pattern.
`mission.list` returns all missions with state/role/gates/timestamps.
`mission.status` returns a single mission by ID with full gate details.
Both scoped separately (`mission.list`, `mission.status`) added to
KNOWN_BASES and CEILING_TRUSTED. Wired in the binary with store
injection. 4 new tests. Closes the Phase 21 deferral.

### Task 3 — Webhook port configurability

Added `[daemon]` TOML section with `webhook_port: Option<u16>`.
New `RawDaemon` struct in config, wired through loader → AivyxConfig →
`run_daemon` → webhook listener spawn with `unwrap_or(DEFAULT_WEBHOOK_PORT)`
fallback. Updated `run_daemon_compat` and all 5 e2e test call sites.
Closes the Phase 27 decision item about hardcoded port 7842.

### Task 4 — Forensic `ToolOutcome::NotInRole` variant

Added `NotInRole { tool_name }` to `ToolOutcome` and `NotInRole` to
`ToolOutcomeSummary` in `aivyx-core`. Updated the `From` impl, the
`tool_outcome_summary_str` renderer, and the `render_tool_result`
function in `llm_planner.rs`. Changed the allowlist gate in the turn
loop from `ToolOutcome::Denied` with synthetic scope to the new
`NotInRole` variant. ScopeDenied audit tag still fires for backward
compatibility. 1 new test. Closes the Phase 11 Q1 deferral — the
oldest item in the backlog (17 phases carried).

**Production-core streak broken at 18.** New streak starts at 0.

### Task 5 — `turn.history` audit introspection tool

First Reflection Layer primitive. New `turn_history_tool.rs` in
`aivyx-channel` — a read-only tool that queries the persistent
audit chain and returns recent turn outcomes. Input: optional
`limit` (default 10) and `since_ms` (epoch millis filter). Output:
array of `{ turn_id, session_id, channel, outcome, tool_calls_made,
duration_ms, started_at }`. Scoped to `audit.read` (already in
CEILING_SEMITRUSTED). Uses `OnceLock<Arc<dyn AuditLog>>`, wired in
the binary by cloning the `Arc<PersistentAuditLog>` before the
`dyn AuditHook` cast. 2 new tests. Advances PRODUCT.md G3.

Streak check:
- DESIGN.md: **untouched** (streak extends)
- PRODUCT.md: **untouched** (streak extends)
- Production-core: **broken at Task 4** (new streak: 0)

## Exit criteria

- [x] All 6 tasks committed.
- [x] `cargo check` clean (warnings only in aivyx-mcp, pre-existing).
- [x] `cargo test` — 697 tests, 0 failures, 1 ignored.
- [x] Three deferrals closed: `ToolOutcome::NotInRole` (Phase 11 Q1),
      `mission.list`/`mission.status` (Phase 21),
      webhook port configurability (Phase 27 decision).
- [x] First Reflection Layer primitive (`turn.history`) shipped.
- [x] DESIGN.md untouched — streak extends.
- [x] PRODUCT.md untouched — streak extends.
- [x] Production-core streak broken (expected, predicted in journal).
- [x] Deferral backlog: 13 → 11 (closed 2 from the formal list;
      webhook port was a decision item, not a numbered deferral).

## Prediction vs reality

| Prediction | Reality |
|---|---|
| DESIGN.md untouched | **Correct.** Untouched. |
| PRODUCT.md untouched | **Correct.** Untouched. |
| Production-core streak extends to 19 | **Wrong.** Broke at Task 4 (NotInRole variant). The journal itself noted this was expected for Task 4. The streak prediction at phase open was optimistic — it said "Low risk" and "extends to nineteen" but then Task 4's own description said "this task will break the production-core streak." |
| Backlog 13 → 10 | **Close.** Actual is 13 → 11. Webhook port config was a Phase 27 decision item, not a numbered deferral, so only 2 formal closures. |

## Deferrals

**Rolling deferrals at Phase 28 exit (11 items, -2 closed):**

- **Second regression channel for the role primitive** —
  Phase 11 Q6. Untouched.
- **Response headers in audit payload** — Phase 12 Q3 half.
  Untouched.
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
  Phase 17 Q6->(c+). Tagged: **reactive.**
- **Telegram-specific protocol extensions (attachment
  delivery, inline keyboards, etc.)** — Phase 19. Untouched.
- **MCP SSE transport** — Phase 23. Untouched.
- **Provider-specific token counting** — Phase 25.
  Untouched.
