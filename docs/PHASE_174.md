# Phase 174 — Loop Hardening: Driver-Side Gate Verification

**The hardening Phase 173 promised.** Phase 173 shipped the
autonomous loop with one honest, foregrounded gap: a run is
fully autonomous and the driver **trusts the agent's
`loop.complete`** — it never independently checks that the tree
is actually green. The only hard guardrail is `max_iterations`.
Phase 174 closes that gap: the driver re-runs the operator's
configured **gate command** (build / tests) and refuses to let
a run charge ahead on a red tree, plus adds a **wall-clock
cap** as a second hard bound.

## The core idea

A loop run is the highest-trust-stakes thing Aivyx does — it
writes code and commits each iteration. Ralph's safety comes
from the gate: *only advance on green*. Phase 173 left gate
enforcement entirely to the agent (via the canonical prompt).
Phase 174 makes the **driver** the enforcer:

- The operator configures `[loop].gate_command` (e.g.
  `"cargo test"`).
- The driver runs it **before the first iteration** (refuse to
  start on a red tree) and **after every iteration**.
- On **green**, the run continues. On **red**, the run stops
  immediately with a `gate failed at iteration N` reason — the
  loop does not pile more changes onto a broken tree.

### Why stop, not roll back (the non-destructive choice)

When the gate goes red the driver **stops the run** — it does
**not** `git reset` the agent's commit or mutate the backlog.
This matches the project's posture: the driver never takes a
destructive, hard-to-reverse action on the operator's repo. A
red tree halting the loop bounds the damage to exactly one
bad iteration, the commit is preserved for the operator to
inspect, and `aivyx loop status` reports the failure. Automatic
rollback is a louder, riskier behaviour deferred unless real
use demands it.

## What this is NOT (still deferred)

- **Token-budget cap** (the Phase 97 / 143-150 arc) and
  **progress-log auto-injection** (the `progress.txt` analog
  over memory) remain Phase 175+ — they are *capability*, not
  *safety*, and the user picked safety first.
- **Web UI loop pane** remains deferred.

## Tasks

1. **Open doc + README.** This doc, README Active row, backfill
   Phase 173's frozen hash (`36610d9`).

2. **Gate config knobs.** Extend `[loop]` with:
   - `gate_command: Option<String>` — the shell command the
     driver runs to verify the tree. Absent → no driver-side
     verification (pre-174 behaviour; `max_iterations` is the
     only cap).
   - `gate_timeout_secs: u64` — kill + treat as red if the gate
     runs longer than this (default 600).
   - `working_dir: Option<String>` — where the gate runs
     (default: the daemon's CWD).
   - `max_run_secs: Option<u64>` — wall-clock cap; a run stops
     once it has run this long.
   Validation on an armed section; tests.

3. **`GateRunner` trait + shell impl.** A `GateRunner` trait
   (`async fn run() -> GateOutcome { Passed | Failed { code } |
   Errored { detail } | TimedOut }`) so the driver's
   verification logic is testable without shelling out. The
   production `ShellGateRunner` runs `gate_command` in
   `working_dir` via the platform shell with the timeout. Tests
   drive fakes + a real trivial command (`true` / `false`).

4. **Driver gate verification + wall-clock.** New
   `LoopDecision::StopGateFailed { iteration }` and
   `StopWallClock`. `decide()` gains the wall-clock check
   (elapsed ≥ `max_run_secs`). `run_loop_driver` runs the gate
   pre-flight + post-iteration; a red gate finishes the run with
   the gate-failed reason. `SharedLoopState` records it via the
   existing `last_stop_reason`. Tests cover the decision matrix
   + the gate-stop transition with a fake runner.

5. **Daemon wiring + INSTALL + exit + Frozen.** Build the
   `ShellGateRunner` in `bin/aivyx` when `gate_command` is set
   and thread it into the driver spawn; `aivyx loop status`
   renders whether gate verification is on. INSTALL section
   (the new knobs + the stop-on-red posture); exit doc; README
   Frozen flip.

## Streak predictions

- **DESIGN.md** — **Will hold.** Streak: 10 → **11**. New
  `[loop]` config fields + a driver-internal gate runner are
  not a contract change; the gate command runs as operator
  config (like a cron), not an agent tool — no new capability
  scope, no A3 amendment, the thirteen-tool core untouched.
- **PRODUCT.md** — **Will hold.** Streak: 64 → **65**.
  Hardening an existing capability, not a new commitment.
- **`aivyx-core/src/lib.rs`** — **Will hold.** Streak: 10 →
  **11**. Work lands in `aivyx-channel` + `aivyx-config`;
  `aivyx-core` is untouched.

## Exit criteria

- [ ] `docs/PHASE_174.md` + README row + Phase 173 backfill —
  Task 1.
- [ ] `[loop]` gate knobs + validation — Task 2.
- [ ] `GateRunner` trait + `ShellGateRunner` (timeout,
  working_dir, exit-status → outcome) — Task 3.
- [ ] `decide()` wall-clock + `run_loop_driver` pre-flight +
  post-iteration gate; red → `StopGateFailed`; idle on
  green — Task 4.
- [ ] `ShellGateRunner` wired into the daemon spawn; `aivyx
  loop status` shows gate state — Task 5.
- [ ] DESIGN.md / PRODUCT.md / `aivyx-core/src/lib.rs` HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+20` to `+40`.

## Honest scope risks at sign-off

- **The gate command runs at daemon privilege.** It is
  operator-configured (like a cron command), executed directly
  by the daemon, not an agent tool — so it is not capability-
  gated. An operator who points `gate_command` at something
  destructive owns that. Documented.
- **Stop-on-red preserves the bad commit.** The driver does
  not roll back; the operator inspects + reverts manually. The
  trade is deliberate (non-destructive); automatic rollback is
  a future consideration.
- **The gate is the operator's whole-tree check, not
  per-story.** It cannot tell which story broke the tree — only
  that the tree is red after iteration N. Per-story gating
  would need a richer contract; out of scope.
- **Wall-clock is checked between iterations, not mid-turn.**
  A single runaway iteration is bounded by the agent's own
  max-steps / the gate timeout, not the wall-clock cap (which
  fires at the next iteration boundary).
- **Sixty-third consecutive deferral of the Channel Activation
  Milestone** — intentional hold.

## Direction after Phase 174

- **Phase 175 — loop capability:** token-budget per-run cap
  (the Phase 97 / 143-150 arc) + progress-log auto-injection
  (carry prior-iteration learnings into each fresh context).
- **Web UI loop pane** (live backlog + iteration counter + gate
  state + stop control).
- The post-172 roster carries forward: LLM-judged correction
  classification, tool/topic surfacing in `OutcomeSummary`,
  cryptographic PRNG, PDF full-compression page count, and the
  Channel Activation Milestone.

## Prediction vs reality

_(Filled at exit.)_
