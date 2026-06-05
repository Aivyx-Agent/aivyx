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

| Prediction | Reality | Held? |
| --- | --- | --- |
| DESIGN.md HOLD → 11 | Untouched (gate config fields + a driver-internal runner; the gate runs as operator config, no new capability scope, no A3 amendment) | ✅ |
| PRODUCT.md HOLD → 65 | Untouched | ✅ |
| `aivyx-core/src/lib.rs` HOLD → 11 | Untouched | ✅ |
| Zero new workspace deps | `tokio::process` was already compiled in via feature unification (aivyx-core requires it) — no Cargo.toml change | ✅ |
| Zero clippy warnings | `cargo clippy --workspace --all-targets -- -D warnings` clean | ✅ |
| Test count delta `+20` to `+40` | **`+16`** (gate 9 + driver +4 + config +3); workspace ~4,072 → ~4,088 | ❌ **below band** |

**Honest miss on the test count.** Predicted `+20..+40`,
landed `+16`. Two reasons, both real: (a) `decide()` was
*extended in place* rather than re-implemented, so the
wall-clock cases reused the existing decision-matrix scaffold
instead of a fresh test module; (b) the headline behaviour —
`run_loop_driver` actually stopping a run on a red gate — is
**integration glue over the concrete `TriggerDispatch`**, which
can't be unit-tested without faking the dispatch, so it is
covered transitively (the `GateRunner` outcomes,
`gate_stop_reason`, `decide()`, and `finish_run` are each unit-
tested; their composition in the driver is build- + behaviour-
verified, not unit-asserted). A tighter estimate for a
hardening phase whose value is in glue would have been
`+12..+20`.

The hardening closed end-to-end:

1. **Gate config knobs** (Task 2). `[loop].gate_command`,
   `gate_timeout_secs`, `working_dir`, `max_run_secs` with
   armed-section validation.
2. **`GateRunner` + `ShellGateRunner`** (Task 3). `sh -c` in
   `working_dir` with a timeout (kill_on_drop + start_kill);
   `GateOutcome` where only `Passed` is green. Real `sh`
   tests: true/false/exit-code/timeout/working-dir/shell-
   features/bad-dir.
3. **Driver verification** (Task 4). `decide()` gained the
   wall-clock cap (`StopWallClock`); `run_loop_driver` runs the
   gate **pre-flight** (refuse to start on red) + **after every
   iteration** (stop on red, non-destructive). The gate is
   built from the armed config at the daemon spawn site.
4. **Surface** (Task 5). `aivyx loop status` now shows gate +
   wall-clock config (`gate_enabled` / `max_run_secs` on the
   IPC); INSTALL rewritten with the new knobs + the
   stop-on-red, daemon-privilege, non-destructive posture.

### Honest-debt status carried forward (Phase 175)

- **Token-budget per-run cap** + **progress-log
  auto-injection** — the remaining loop-capability items.
- **Grandchildren on gate timeout may linger** — `start_kill`
  kills the `sh` child, not a process group; documented.
- **Wall-clock is checked at iteration boundaries**, not
  mid-turn.
- Sixty-third consecutive deferral of the Channel Activation
  Milestone.

### The result

The autonomous loop's single biggest safety gap from Phase 173
— the driver trusting the agent's self-reported completion — is
closed. With `gate_command` set, a run cannot start on a red
tree and stops the instant any iteration breaks it, bounded
additionally by `max_iterations` and an optional wall-clock
cap, all without a single new workspace dependency or a broken
contract streak.
