# Phase 173 — Autonomous Loop Foundation (the Aivyx Ralph Loop)

**The orchestration push the Agent Review asked for.** The
review closed with *"the substrate's there; the orchestration
is what remains."* Phase 173 ships the first slice of a
flagship orchestration: an **autonomous, self-re-arming task
loop** — Aivyx's native answer to Geoffrey Huntley's "Ralph"
technique (snarktank/ralph). The operator stocks a backlog of
stories; the loop fires a **fresh-context agent turn per
iteration**, each picking the next story, implementing it,
running gates, committing, and marking it done — re-arming
until the backlog is empty or a hard cap is hit.

The Ralph insight is that a *dumb loop over a fresh agent with
persistent external state* beats a single long context: state
lives in git + a backlog file + a progress log, not in the
model's window. Aivyx already has every piece of that pattern
as a first-class primitive — this phase wires them into a
loop.

## How Ralph maps onto Aivyx primitives

| Ralph piece | Aivyx primitive |
| --- | --- |
| `ralph.sh` (`while` spawning fresh agents) | A **loop driver** background task (sibling of `run_reflection_scheduler`) that re-arms `TriggerDispatch::fire` |
| "fresh AI instance per iteration" | `TriggerDispatch::fire` already mints a fresh `SessionId` + turn |
| `prd.json` (ordered stories + `passes` flags) | **New HMAC-chained backlog substrate** (this phase) |
| `progress.txt` (cross-iteration learnings) | The `aivyx-memory` substrate (deferred auto-wiring → Phase 174) |
| typecheck/tests quality gates | The core `shell` tool (already shipped) |
| commit per iteration | The core `git` tool (already shipped) |
| "mark story complete" | **New `loop.complete` agent tool** (this phase) |
| max-iterations / termination | The driver's re-arm + cap logic (this phase) |
| guardrails | Capability gating + the audit chain (every iteration is a `TriggerSource::Loop` turn in the chain) |

The agent already has `git` + `shell` in the thirteen-tool
core (amendment A12), so it can commit and run gates today.
The only new agent-facing surface is **backlog interaction**
(`loop.next` / `loop.complete`), which lands at the channel
tier alongside `mission.*` / `reflection.*` — **the
thirteen-tool core is untouched, no DESIGN.md A12 amendment.**

## Decisions locked at phase entry

- **Backlog store: HMAC-chained substrate.** Aivyx-native,
  capability-gated, in the audit chain, queryable via tools +
  the `aivyx loop` CLI, surviving independent of the working
  repo. Mirrors `PersistentPersonaProposalLog` (Phase 70).
- **Autonomy: fully autonomous.** Once started, a run executes
  to backlog-completion or a hard cap (max-iterations this
  phase; token-budget + wall-clock are Phase 174) with no
  per-iteration operator gate. The caps + capability gating +
  audit chain are the guardrails. Starting a run is itself the
  operator's explicit, audited action.
- **Scope: foundation.** The loop works end-to-end on the
  minimal path. Progress-log auto-injection, budget wiring,
  wall-clock cap, driver-side gate verification, and a Web UI
  surface are explicit Phase 174+ follow-ons.

## Tasks

1. **Open doc + README.** This doc, README Active row,
   backfill Phase 172's frozen hash (`b0ca1f0`).

2. **Loop backlog substrate.** New `loop_backlog` module
   mirroring `persona_proposal` (Phase 70): an HMAC-chained,
   append-only log of **stories** (`id`, `priority`, `title`,
   `body`, status `Pending | Done | Skipped`) over a new
   HKDF-isolated `KeyDomain::LoopBacklog`. Status transitions
   append new rows (the chain only signs immutable bytes).
   Ops: `add_story`, `next_pending` (lowest `priority` number
   first, then insertion order), `mark_done`, `mark_skipped`,
   `list`, `remaining_count`. Chain-verified at load.

3. **Backlog agent tools.** New `loop_tool` module:
   - `loop.next` — returns the current highest-priority
     pending story (id + title + body) or a clean
     "backlog empty" signal.
   - `loop.complete { story_id }` — marks a story `Done`.
   Capability-gated under a new `loop.write` / `loop.read`
   scope; channel-tier (registered in `bin/aivyx` next to
   `mission.*`). The thirteen-tool core is untouched.

4. **Loop driver + canonical prompt.** New `loop_driver`
   module (sibling of `reflection_scheduler`):
   `run_loop_driver` background task. While a run is active it
   (a) checks termination — backlog empty, or
   `iterations >= max_iterations`; (b) fires one
   `TriggerDispatch::fire(TriggerSource::Loop, …)` with the
   canonical `LOOP_SYSTEM_PROMPT`; (c) waits for the turn to
   settle, increments the iteration counter, and re-arms.
   New `TriggerSource::Loop` variant (audit-distinct, like
   `Reflection`). The prompt instructs: call `loop.next`; if
   empty, stop; else implement the story, run gates with
   `shell`, commit with `git`, then `loop.complete` and write
   one learning line to memory. A shared run-state handle
   (`active`, `iteration`, `started_at`) gives the CLI its
   status + stop control.

5. **Config + CLI + INSTALL + exit + Frozen.** A `[loop]`
   config block (`enabled`, `max_iterations`, default
   `priority`). CLI: `aivyx loop add <title> [--body …]
   [--priority N]`, `aivyx loop list`, `aivyx loop start`,
   `aivyx loop status`, `aivyx loop stop`. INSTALL section,
   exit doc with prediction-vs-reality, README/ROADMAP Frozen
   flip.

## Streak predictions

- **DESIGN.md** — **Will hold.** Streak: 9 → **10**. The
  thirteen-tool core is untouched (loop tools are channel-tier
  like `mission.*`); a new `KeyDomain` variant + a new
  concrete capability scope within the existing taxonomy (A3)
  are not contract changes — `calendar.write` / `drive.read`
  were added the same way without amendments. **This is the
  phase's main streak risk** — an autonomous code-committing
  loop is a notable capability — but it is *orchestration of
  existing substrate*, not a new substrate contract.
- **PRODUCT.md** — **Will hold.** Streak: 63 → **64**. The
  loop is built from existing primitives (triggers, fresh-turn
  dispatch, tools, audit) exactly as the reflection scheduler
  (Phase 71) was — a new orchestration, not a new product
  commitment.
- **`aivyx-core/src/lib.rs`** — **Will hold.** Streak: 9 →
  **10**. All work lands in `aivyx-channel`, `aivyx-config`,
  `aivyx-capability`, and `aivyx-storage`; `aivyx-core` is
  untouched (it already ships `git` + `shell`).

## Exit criteria

- [ ] `docs/PHASE_173.md` + README row + Phase 172 backfill —
  Task 1.
- [ ] `loop_backlog` substrate + `KeyDomain::LoopBacklog`;
  HMAC-chained, add/next/done/list, chain-verified — Task 2.
- [ ] `loop.next` + `loop.complete` capability-gated channel
  tools, registered — Task 3.
- [ ] `run_loop_driver` re-arms `TriggerSource::Loop` turns
  until backlog empty or `max_iterations`; canonical prompt —
  Task 4.
- [ ] `[loop]` config + `aivyx loop {add,list,start,status,
  stop}` — Task 5.
- [ ] DESIGN.md / PRODUCT.md / `aivyx-core/src/lib.rs` HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+35` to `+55`.

## Honest scope risks at sign-off

- **Fully autonomous + agent self-marks-done.** The agent
  calls `loop.complete` after it *believes* the gates passed;
  the driver does not independently re-run tests between
  iterations this phase. The canonical prompt enforces the
  "only complete after gates pass + commit" discipline, every
  iteration is audited, and the `max_iterations` cap bounds
  blast radius — but **driver-side gate verification (run the
  test command between iterations, only advance on green) is
  the headline Phase 174 hardening.**
- **Caps are max-iterations only this phase.** Token-budget
  integration (Phase 97 / the Phase 143–150 budget arc) and a
  wall-clock cap are Phase 174. An operator must set
  `max_iterations` sanely; the default is conservative.
- **Progress log is manual.** The agent is *told* to write a
  learning line to memory each iteration, but auto-injection
  of prior learnings into the next iteration's context (the
  `progress.txt` analog) is not yet wired — Phase 174.
- **No mid-run backlog edits surfaced.** Adding/removing
  stories while a run is live is allowed by the substrate but
  not specially coordinated; the driver re-reads
  `next_pending` each iteration, so edits take effect on the
  next turn (documented, not a bug).
- **Single concurrent run.** One active loop run per daemon
  this phase; concurrent runs are out of scope.
- **Sixty-second consecutive deferral of the Channel
  Activation Milestone** — intentional hold.

## Direction after Phase 173

- **Phase 174 — loop hardening:** driver-side gate
  verification, token-budget + wall-clock caps, progress-log
  auto-injection (the `progress.txt` analog over memory),
  per-run audit summary.
- **Web UI loop pane** (live backlog + iteration counter +
  stop control), mirroring the Persona/Learning panes.
- **Operator-gated variants** (per-N-iteration check-ins) for
  operators who want the semi-autonomous posture.
- The post-172 roster carries forward: tool/topic surfacing in
  `OutcomeSummary`, LLM-judged correction classification,
  cryptographic PRNG, PDF full-compression page count, and the
  Channel Activation Milestone.

## Prediction vs reality

| Prediction | Reality | Held? |
| --- | --- | --- |
| DESIGN.md HOLD → 10 | Untouched (the A3 amendment *file* enumeration was bumped 69→71 for the two new scopes — the established process, not a DESIGN.md edit) | ✅ |
| PRODUCT.md HOLD → 64 | Untouched | ✅ |
| `aivyx-core/src/lib.rs` HOLD → 10 | Untouched | ✅ |
| Zero new workspace deps | All work reused existing primitives (`aivyx_storage`, the HMAC-chain template, `TriggerDispatch`, `tokio::sync::Notify`) | ✅ |
| Zero clippy warnings | `cargo clippy --workspace --all-targets -- -D warnings` clean (one `manual Default` + one unused-fn reworded in Task 4) | ✅ |
| Test count delta `+35` to `+55` | `+37` (backlog 12 + tools 5 + driver 11 + cli 5 + config 4); workspace ~4,035 → ~4,072 | ✅ (low end of band) |

The foundation closed end-to-end — the loop works:

1. **Backlog substrate** (Task 2). `loop_backlog`: an
   HMAC-chained, append-only story list mirroring the Phase 70
   persona-proposal chain, over the new
   `KeyDomain::LoopBacklog`. add / next_pending / done /
   skipped / list, chain-verified at load (tamper + wrong-key
   both bail).
2. **Agent tools** (Task 3). `loop.next` / `loop.complete` —
   channel-tier, capability-gated under two new
   Trusted-tier bases (A3 enumeration 69→71). The thirteen-tool
   core (A12) untouched; the agent already has `git` + `shell`.
3. **Driver + canonical prompt** (Task 4). `loop_driver`:
   `run_loop_driver` fires a fresh-context `TriggerSource::Loop`
   turn per iteration; pure `decide()` termination; Notify-woken
   `SharedLoopState`. New `TriggerSource::Loop` +
   `TriggerKindSummary::Loop`.
4. **Config + CLI** (Task 5). `[loop]` block (cap + default
   priority); `aivyx loop add/list/start/stop/status` over five
   new IPC query/response pairs; the daemon spawns the driver
   when armed.

### What landed beyond the open

The IPC + CLI surface touched the usual daemon plumbing
(DaemonConfig / ConnectionContext field accretion, the
`QueryPayload`/`QueryResponsePayload` enums) — expected for a
new operator-facing daemon command, same shape as the Phase
102 tool-observability add.

### Honest-debt status carried forward (Phase 174)

- **Driver-side gate verification** — the driver trusts the
  agent's `loop.complete`; it does not independently re-run
  tests between iterations. Headline hardening.
- **Token-budget + wall-clock caps** — `max_iterations` is the
  only hard cap this phase.
- **Progress-log auto-injection** — the agent is told to write
  learnings to memory, but prior learnings are not yet
  auto-injected into the next iteration (the `progress.txt`
  analog).
- **Single concurrent run; mid-run backlog edits take effect
  next iteration** — both documented, not bugs.
- Sixty-second consecutive deferral of the Channel Activation
  Milestone.

### The orchestration push

The Agent Review closed with *"the substrate's there; the
orchestration is what remains."* Phase 173 is exactly that
push: it composes the existing substrate (fresh-turn dispatch,
the HMAC-chain pattern, capability gating, the audit chain,
core `git`/`shell`) into a flagship autonomous-loop capability,
adding no new workspace dependency and leaving every contract
streak intact.
