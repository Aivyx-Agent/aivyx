# Phase 125 — Personal Assistant Tool Bundle (Chapter G #1)

**Chapter G opener.** First phase under the operator-facing-
personal-assistant-capabilities chapter, opened after the
Phase 124 exit framing left the operator-tooling-coverage
gap as the load-bearing question for whether channels
matter at all.

## Chapter G context (this is the chapter opener)

After Chapter F #1 (Phase 123 Gmail) shipped and Phase 124
exited with the local-LLM-rehab axis exhausted, the
operator framed the next direction:

> "I dont see the point of Channel Activation until the
> TOOLING has been fully implemented and fully functional.
> There is little point giving the Aivyx Agent Channels if
> its still unable to actually do jobs that a Personal
> Assistant shoulod be able to do."

That framing is load-bearing correct on the personal-
assistant value proposition: chat-surface routing without
working capability is putting the cart before the horse.

The operator's pasted "tool list" was actually gemma4's
hallucinated enumeration from the Phase 122/124 transcripts
— diagnostic itself. Sorting it against Aivyx's real
registered tools surfaced three gaps:

| Gap                 | Real today?  | Phase 125 covers |
|---------------------|--------------|------------------|
| Web search          | ❌ (only `web.fetch`) | ✅ `web.search` via Brave |
| Lightweight TODO    | ❌ (`mission.*` heavyweight) | ✅ `task.*` (4 tools) |
| Health monitoring   | ❌            | ✅ `health.check.*` (3 tools) |

Chapter G is the chapter that fills these gaps. Different
from Chapter F (Gmail / Calendar / etc are external service
integrations targeting specific APIs) — Chapter G is broader
operator capability for the personal-assistant role: web
search, task tracking, monitoring, future tools like
calendar reminders, expense tracking, etc.

Per P10 + P11 + P12, every Chapter G tool ships as a
third-party tool process (same pattern as Chapter F).
Aivyx core stays at the thirteen-tools-forever cap.
Chapter G integrations DO NOT touch substrate; they live
in separate binaries operators install and wire via
`[[tool_process]]`.

## Why bundle all three in one phase

Operator-picked at the Phase 125 direction question:
**bundle, not sequence.** Three tools through one binary
(`aivyx-toolkit`) shares HTTP client, config loader, and
the multi-tool harness pattern proven in Phase 123. Single
operator-install step gets all three tools.

Honest scope risk acknowledged: 8 tools registered (1 web
+ 4 task + 3 health) is the largest single-phase tool
surface to date. Phase 123 was 4 Gmail tools through 8
tasks; Phase 125 is 8 tools through 7 tasks — denser per-
task work but each tool category is smaller in substrate
than Gmail's OAuth flow. Tractable.

## Pre-open architectural picks (already operator-resolved)

- **Scope shape (Phase 125 direction Q):** Bundle three
  tools in one phase.
- **First tool if sequencing (resolved as N/A by bundling
  pick):** web.search via Brave Search API.
- **health.check shape:** Scheduled monitoring with state +
  alerts. Tool-process-side polling; alert dispatch
  composed by agent loop via `health.check.recent_changes`
  + `notify.send` rather than tool-process callback to
  core (substrate-minimal; defers the IPC-to-core question
  Phase 124's secondary findings raised).

## Real-use signal driving this phase

Operator pasted gemma4's hallucinated tool list as the
"this is what a Personal Assistant should do" spec. Roughly
60% of gemma4's invented categories mapped to real Aivyx
tools (file system, shell, scheduling, webhooks, etc); the
remaining 40% were gaps. The four-substrate-phase Phase 124
finding ruled out "fix the local-LLM invocation reliability
first"; the operator-actionable answer is to fill the gaps
so the tool surface is at least complete for whichever
provider (cloud, future-better-local) does invoke
reliably.

## Why this, why now

- **The Channel Activation Milestone is now 13× deferred.**
  Phase 125 is the 13th deferral. The operator framing is
  honest: channels without working tools is wrong-order.
  Phase 125 prioritizes the tools.

- **Chapter G's substrate cost is near-zero.** Phase 123
  built the multi-tool harness + per-tool-process config
  + per-tool-process file storage + capability-base
  extension pattern. Phase 125 consumes that substrate
  unchanged. No new architectural primitives — only new
  tool implementations.

- **Local-LLM-rehab axis is honestly exhausted (Phase 124
  exit).** Continuing to substrate against the model wall
  would be diminishing returns. Operator-tooling
  coverage is the substrate axis with clearest value;
  Phase 125 picks it.

## Scope (Q-block sign-off, pre-resolved)

All three picks made in the multi-question round opening
this phase:

- **Q1 — Scope shape:** **Bundle all three in
  `aivyx-toolkit` multi-tool binary** (operator-picked
  over sequence / hybrid).

- **Q2 — First tool focus:** **`web.search` via Brave
  Search API** (resolved as N/A by Q1 bundling, but the
  Q2 pick informs in-phase task ordering — web.search
  ships first within Phase 125).

- **Q3 — `health.check` shape:** **Scheduled monitoring
  with state + alerts** (Recommended; operator-picked
  over one-shot / defer). Tool-process-side polling
  loop; alerts composable via agent invocation of
  `recent_changes` + `notify.send`.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract amendment.
  New binary crate consumes existing SDK substrate. Hash
  at entry:
  `c2be6d51def04207ab0527b4f335edf7c726920b4f102185336eded01b1b995b`.
  Prediction: streak **extends to sixteen** (was 15
  after Phase 124).

- **PRODUCT.md** — **Will hold.** P10 + P11 + P12 already
  cover this case (operator-facing tools beyond the
  thirteen-tool substrate are third-party). G6 covers
  local-execution posture. No contract amendment. Hash:
  `6e840cef2cfcee4c483c2b4769c29cc7062daeb4bf3802c53f48e72ebfe57fb8`.
  Prediction: streak **extends to sixteen**.

- **`aivyx-core/src/lib.rs`** — **Will hold.** Substrate
  work lives entirely in the new `aivyx-toolkit` crate +
  aivyx-capability's KNOWN_BASES/ceilings. Tool trait,
  Scope, ToolContext consumed unchanged. Hash:
  `b420405bf9a5576ecb10f6ea04a965f7ad3a1a92ae9bd8f4abf46e22ef4d3c16`.
  Prediction: streak **extends to six** (was 5). 90/10
  hold.

- **New workspace deps** — Zero anticipated. `reqwest`,
  `serde`, `serde_json`, `tokio`, `thiserror`, `uuid`,
  `base64`, `toml` all already in use by aivyx-gmail
  (which Phase 125 mirrors). No additional crates.

- **Test count** — Substantial: 8 tools × per-tool input
  validation + schema tests, plus polling-loop substrate,
  plus per-tool API/storage round-trips. Prediction:
  **`+80 to +130`** anchored on Phase 123's `+151` for
  4 tools through OAuth (Phase 125 is more tools but
  simpler substrate per tool).

## Tasks

Seven sub-tasks plus exit + backfill:

### Task 1 — Open (this commit)

`docs/PHASE_125.md` + `docs/ROADMAP.md` Chapter G section
+ Phase 125 entry + `docs/README.md` status row. Documents
the operator-pressure framing + the bundle-architectural
pick + the chapter-G chapter framing.

### Task 2 — `aivyx-toolkit` crate skeleton + capability bases

- New `aivyx-toolkit` binary crate at
  `crates/aivyx-toolkit/`. Mirrors `aivyx-gmail`'s
  layout: `lib.rs` + `main.rs` + per-tool modules.
- Reuses Phase 123's multi-tool harness pattern
  (`run_multi_tool_subprocess`) inline (until the
  SDK-validation lift recommendation lands as
  substrate work).
- Per-tool-process config file at
  `~/.aivyx/tool-processes/toolkit/config.toml`.
- New scope bases in `aivyx-capability::KNOWN_BASES`:
  - `web.search` — gates `web.search` tool.
  - `task.read` — gates `task.list`.
  - `task.write` — gates `task.create`, `task.complete`,
    `task.delete`.
  - `health.read` — gates `health.check.list`,
    `health.check.recent_changes`.
  - `health.write` — gates `health.check.add`.
- All five added to `CEILING_TRUSTED` only (Trusted-tier
  default mirrors `shell.exec` / `notify.send` / Gmail
  scope gating per Phase 62 Q2(a) — personal-assistant
  tools shouldn't be reachable from remote channels
  without explicit role grant).
- A3 amendment bumped: 52 → 57 bases. New "Personal
  assistant tool process scopes" category in the
  enumeration.
- Tests: capability count pin update; scope-parse tests
  for each new base.

### Task 3 — `web.search` tool (Brave Search API)

- Operator-provided Brave Search API key in
  `~/.aivyx/tool-processes/toolkit/config.toml` under
  `[brave_search]`. Free tier (2000 queries/month) is
  fine for personal use.
- Tool input: `{q: string (required), count: u32
  (optional, 1-20, default 10)}`.
- Tool output: `{results: [{title, url, description}],
  query, total_estimated_count}`.
- Capability scope: `web.search`.
- Verification: `NotApplicable` (read-only query).
- Tests: input parse + schema bounds; Brave API
  request shape; response decode against canned JSON;
  HTTP error handling.

### Task 4 — `task.*` tools (create / list / complete / delete)

- Lightweight TODO tracking with JSON file storage at
  `~/.aivyx/tool-processes/toolkit/tasks.json` (0600
  perms, atomic write-then-rename — same pattern as
  Gmail tokens).
- Four tools:
  - `task.create` — input: `{title: string,
    notes: string optional, due_date_iso: string
    optional}`; output: `{id, title, status: "open",
    created_at_iso}`. Scope: `task.write`.
  - `task.list` — input: `{status: "open" | "complete"
    | "all" (default "open"), limit: u32 (default 25)}`;
    output: `{tasks: [...], total_count}`. Scope:
    `task.read`.
  - `task.complete` — input: `{id: string,
    completion_note: string optional}`; output:
    `{id, status: "complete", completed_at_iso}`.
    Scope: `task.write`.
  - `task.delete` — input: `{id: string}`; output:
    `{id, deleted: true}`. Scope: `task.write`.
- Tests: per-tool input validation; file
  round-trip; concurrent-write safety (tokio Mutex over
  the file handle); schema constraints.

### Task 5 — `health.check` substrate (polling loop + state)

- Tool-process-side polling loop: background tokio task
  that iterates registered watchers on their configured
  intervals. Watchers stored at
  `~/.aivyx/tool-processes/toolkit/health-watchers.json`.
  Current state + recent transitions stored at
  `~/.aivyx/tool-processes/toolkit/health-state.json`.
- Polling cadence: each watcher's `interval_secs`
  (operator-chosen; minimum 60s to prevent abuse).
- State transitions ("was-ok-now-down" / "was-down-now-ok")
  recorded with timestamp; ring buffer (last 100
  transitions across all watchers; per Phase 6 Q5
  honest scope — full alert history is operator-
  desirable but unbounded growth is risk).
- HTTP probes use the same `reqwest::Client` instance
  as `web.search`; 10-second timeout per probe.
- Tests: state-transition detection (mocked HTTP
  responses); ring-buffer cap; concurrent polling
  safety (multiple watchers hitting different URLs
  in parallel); interval-respect (watcher with 60s
  interval isn't polled twice in 30s).

### Task 6 — `health.check.*` tools

Three operator-facing tools:

- `health.check.add` — input: `{name: string (unique),
  url: string, interval_secs: u32 (60-86400), expect_status:
  u16 optional (default 200)}`; output:
  `{name, url, interval_secs, status: "pending"}`.
  Scope: `health.write`. Validation: name uniqueness,
  URL well-formed, interval in range.
- `health.check.list` — input: `{}`; output:
  `{watchers: [{name, url, interval_secs, last_check_iso,
  last_status, last_ok: bool}]}`. Scope: `health.read`.
- `health.check.recent_changes` — input:
  `{window_minutes: u32 (default 60, max 1440)}`;
  output: `{changes: [{watcher_name, transitioned_at_iso,
  from_state, to_state, status_code}]}`. Scope:
  `health.read`. The substrate that enables agent-side
  alert composition: agent calls this on schedule,
  sees changes, decides to invoke `notify.send`.
- **Defer `health.check.remove` to a follow-on phase.**
  Operators can manually edit the watchers file to
  remove a watcher; the next polling cycle drops it.
  A proper remove tool ships in Phase 126 if value
  is proven.
- Tests: per-tool input validation; integration with
  the polling-loop substrate from Task 5 (add a
  watcher; let it poll; query list; verify state).

### Task 7 — INSTALL.md sweep + exit + hash backfill

Operator-facing documentation:

- New `## Operator-facing personal assistant
  capabilities (Chapter G)` section framing the chapter
  and the bundle-binary choice.
- Sub-section `### Personal assistant tool bundle (Phase
  125)` with the full operator setup:
  - GCP-free setup (no OAuth — Brave API key only).
  - `~/.aivyx/tool-processes/toolkit/config.toml`
    structure.
  - `[[tool_process]]` registration in `aivyx.toml`.
  - Per-role `capability_scopes` examples for each
    tool combination.
  - Alert composition recipe: operator schedules a
    cron via `schedule.create` that fires the agent;
    agent calls `health.check.recent_changes` then
    composes `notify.send` for any state changes.
- PHASE_125.md exit doc: prediction-vs-reality, test
  count delta, any architectural surprises surfaced.
- ROADMAP.md rotation: Phase 125 → Frozen; Chapter G
  remains open.
- README.md status row → Frozen with exit-commit hash
  backfilled in standard second-step commit.

## Exit criteria

- [ ] `docs/PHASE_125.md` + ROADMAP Chapter G + Phase 125
  entry + `docs/README.md` status row — Task 1.
- [ ] `aivyx-toolkit` crate skeleton + 5 new scope bases
  + A3 amendment update — Task 2.
- [ ] `web.search` tool (Brave) — Task 3.
- [ ] `task.*` tools (4 tools) — Task 4.
- [ ] `health.check` substrate (polling + state) — Task 5.
- [ ] `health.check.*` tools (3 tools) — Task 6.
- [ ] INSTALL.md sweep + exit — Task 7.
- [ ] Q1 / Q2 / Q3 resolved pre-Task 2 (bundle + brave +
  scheduled monitoring).
- [ ] DESIGN.md streak — predicted HOLD (streak → 16).
- [ ] PRODUCT.md streak — predicted HOLD (streak → 16).
- [ ] `aivyx-core/src/lib.rs` streak — predicted HOLD
  (streak → 6), honest 90/10 hold.
- [ ] Zero new workspace dependencies.
- [ ] Test count delta within `+80` to `+130`.
- [ ] Zero clippy warnings.
- [ ] Operator-facing INSTALL section enables a
  competent operator to install Brave key + register
  the tool process + use all 8 tools end-to-end
  through a tool-invoking provider (Anthropic /
  OpenAI / future-fixed-local-model).

## Honest scope risks at sign-off

- **8 tools is dense for one phase.** Phase 123 was 4
  tools through 8 tasks; Phase 125 is 8 tools through 7
  tasks. The per-task pressure is higher. Phase 6 Q5
  applies: if any one tool category proves harder than
  estimated, the exit doc reports honestly which got
  cut.

- **Local-LLM invocation reliability is unchanged.** Phase
  124 established that qwen3.6 + gemma4 don't invoke
  tools reliably. Phase 125 ships MORE tools they
  won't invoke. The operator-facing value lands when
  paired with a tool-invoking provider (Anthropic /
  OpenAI / future-fixed-local). Stating this honestly:
  Phase 125 doesn't close the local-LLM gap; it expands
  the surface that works WHEN any invocation works.

- **Alert composition is agent-driven, not automatic.**
  health.check records state transitions; the agent
  reads `recent_changes` on schedule and decides to
  notify. This works when the agent invokes tools
  reliably (cloud providers); under local models that
  don't invoke, the substrate captures changes but
  alerts never go out. A future phase can add a
  daemon-side polling hook to dispatch automatically;
  out of Phase 125 scope.

- **Brave Search free tier limits (2000/month).** Will
  bite a heavy operator. INSTALL.md notes the limit
  and points at paid tiers for production use.

- **Task data is plaintext on disk.** Task notes might
  contain sensitive personal information (operator's
  TODO list). 0600 perms protect from other OS users
  but not from disk-image theft. Honest framing in
  INSTALL.md: task data is no-more-protected than
  Gmail's token file.

## Direction after Phase 125

After Phase 125, Chapter G is open and ongoing. Candidates
for Phase 126:

1. **Chapter G #2 — second tool bundle (e.g.
   calendar.* + reminder.* + budget.*)** — continues
   the operator-tool-surface expansion arc.
2. **Health.check.remove + alert-dispatch IPC** —
   completes the health-check surface and ships the
   automatic alert path Phase 125 deferred.
3. **Textual tool-call extraction substrate** — Phase
   124's secondary finding. Would rescue qwen3
   specifically; expands which models can use the new
   Chapter G tools.
4. **Chapter F #2 — Calendar/Drive/GitHub** — alternative
   chapter direction if operator pressure pivots
   away from Chapter G.
5. **Channel Activation Milestone** — fourteenth
   consecutive deferral if skipped (Phase 125 itself
   is the 13th). Audit's #1 unchanged.
6. **Release prep (v0.1.0 + installer)** — Phase 99
   deferred.
7. **Operator-pressure-driven new direction**.

Phase-by-phase decision at Phase 125 exit, sharpened by
the operator's first-real-use signal once the tool
bundle ships.
