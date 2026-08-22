# Mission Control — Piece 2: Pause/Resume Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** an operator can pause a running Nonagon mission at its next wave
boundary — distinct from abort, this landing is resumable — and later
resume it, continuing the DAG walk from exactly where it paused.

**Architecture:** a new non-terminal `TeamMissionPhase::Paused` variant.
Pause reuses the exact wave-boundary-halt mechanism abort already uses
(`should_halt`, checked by the engine at each wave boundary) via a second
flag mirroring the existing abort flag shape exactly, with a 3-way
priority order (abort > pause > budget cap) when more than one is armed.
Resume reuses the exact "flip persisted phase, then spawn a fresh drive"
shape the existing gate-approval flow (`prepare_gate_resolution`/
`resolve`) already uses — no changes to `aivyx-team`'s engine are needed
at all; `drive()` already holds the very flag it armed and can inspect it
directly after the run completes, so no new `MissionStatus` variant or
`aivyx-team` change is required either.

**Tech Stack:** Rust, `std::sync::atomic::AtomicBool` (mirroring the
existing abort-flag primitive), `serde` wire framing.

## Global Constraints

- `Paused` is explicitly **non-terminal**: `TeamMissionPhase::is_terminal()`
  must NOT list it (it already won't, by simple omission — no code change
  needed there, only the new variant addition).
- The wave-boundary priority order when abort, pause, and a tripped budget
  cap could all be pending at the same check: **abort > pause > budget
  cap** — abort is the existing, most-decisive operator intent and must
  never be silently downgraded to a resumable pause; the budget cap is a
  system-imposed stop, lowest priority against either explicit operator
  action. (This is the same order the approved design doc already
  specifies.)
- No `halt_reason` is set when landing in `Paused` — that field's own
  existing contract is "set iff phase == Halted" (see its doc comment in
  `crates/aivyx-ipc/src/team_mission.rs`); a paused mission isn't halted,
  so this field stays `None` for it, exactly like every other non-`Halted`
  phase already leaves it `None`.
- `SharedMissionState::put`'s broadcast (Piece 1's own change — it now
  calls `self.broadcast_live_view(&id)` on every successful write) already
  covers every phase transition this plan introduces automatically — no
  new broadcast wiring is needed anywhere in this plan; every `put(...)`
  call already pushes a live update.
- This plan does **not** touch anything from Piece 3 (a new Mission
  Control nav view in `aivyx-web`) — no new UI surface, though Task 1
  keeps the *existing* phase-label/color render sites (already broken by
  adding an enum variant to a non-`#[non_exhaustive]` public enum matched
  in three crates) compiling and displaying something sensible.
- This plan does not add a new `MissionStatus` variant to `aivyx-team`'s
  engine (`crates/aivyx-team/src/runtime.rs`) — confirmed unnecessary:
  `drive()` already holds a local reference to the exact flag it arms, so
  it can distinguish "halted because of a pause request" from "halted for
  any other reason" itself, purely within `aivyx-channel`, without the
  engine needing any new vocabulary.

---

## Task 1: `TeamMissionPhase::Paused` + every cross-crate render site

**Files:**
- Modify: `crates/aivyx-ipc/src/team_mission.rs`
- Modify: `crates/aivyx-tui/src/model.rs`
- Modify: `crates/aivyx-tui/src/render.rs`
- Modify: `crates/aivyx-web/src/main.rs`
- Modify: `crates/aivyx-cli/src/bin/aivyx_modules/team_cli.rs`

**Interfaces:**
- Produces: `TeamMissionPhase::Paused` (new enum variant). `aivyx-tui`'s own `MissionPhase::Paused` (new, local, sibling enum — no relation to Piece 1's `StepState`).

**Verified before writing this task** (do not re-derive — these are the
complete, exhaustive lists, found via `grep -rn "TeamMissionPhase::Halted"`
across the whole tree): adding a variant to `TeamMissionPhase` (a public
enum with no `#[non_exhaustive]`) breaks exactly 5 exhaustive matches
across 3 crates. All 5 are listed below with their exact current bodies —
this task's whole job is adding one arm to each, nothing more.

- [ ] **Step 1: Add the new variant**

In `crates/aivyx-ipc/src/team_mission.rs`, find:

```rust
pub enum TeamMissionPhase {
    /// Assembled but not yet executing (transient, pre-first-step).
    Planning,
    /// Walking the DAG.
    Executing,
    /// Paused at a human-approval gate, awaiting an operator decision.
    AwaitingApproval,
    /// Finished — every step ran and any gates passed.
    Done,
    /// Ended by a gate: an auto gate's FAIL verdict, or a human reject.
    Rejected,
    /// Chapter Ballast (Opp D) — halted at a wave boundary because a
    /// per-mission budget cap tripped. Terminal; completed-step outputs are
    /// preserved. Distinct from `Rejected` so the operator sees *why* it ended.
    Halted,
}
```

Change to:

```rust
pub enum TeamMissionPhase {
    /// Assembled but not yet executing (transient, pre-first-step).
    Planning,
    /// Walking the DAG.
    Executing,
    /// Paused at a human-approval gate, awaiting an operator decision.
    AwaitingApproval,
    /// Chapter Mission Control — paused at a wave boundary by an explicit
    /// operator pause request (distinct from `AwaitingApproval`, which is
    /// a *plan-defined* human gate; this is an operator interrupting an
    /// otherwise-unattended run). **Non-terminal** — `is_terminal()`
    /// deliberately omits it. Completed-step outputs are preserved exactly
    /// like `Halted`'s are; resume continues the DAG walk from them. Never
    /// carries a `halt_reason` (that field's own contract is "set iff
    /// phase == Halted").
    Paused,
    /// Finished — every step ran and any gates passed.
    Done,
    /// Ended by a gate: an auto gate's FAIL verdict, or a human reject.
    Rejected,
    /// Chapter Ballast (Opp D) — halted at a wave boundary because a
    /// per-mission budget cap tripped. Terminal; completed-step outputs are
    /// preserved. Distinct from `Rejected` so the operator sees *why* it ended.
    Halted,
}
```

(`is_terminal()`, just below, needs no change — it already lists exactly
`Done | Rejected | Halted`, and `Paused` is correctly non-terminal simply
by not appearing there.)

Add a test to this file's existing `mod tests` (near any other simple
enum-behavior test — check the file's own test module for its established
style) proving this:

```rust
    #[test]
    fn paused_is_not_terminal() {
        assert!(!TeamMissionPhase::Paused.is_terminal());
    }
```

- [ ] **Step 2: Fix `aivyx-cli`'s phase-label match**

In `crates/aivyx-cli/src/bin/aivyx_modules/team_cli.rs`, find (around line
151):

```rust
    match phase {
        TeamMissionPhase::Planning => "planning",
        TeamMissionPhase::Executing => "executing",
        TeamMissionPhase::AwaitingApproval => "awaiting approval",
        TeamMissionPhase::Done => "done",
        TeamMissionPhase::Rejected => "rejected",
        TeamMissionPhase::Halted => "halted",
```

Add, in the same relative position as the enum declaration (right after
`AwaitingApproval`'s arm):

```rust
        TeamMissionPhase::Paused => "paused",
```

- [ ] **Step 3: Fix `aivyx-web`'s phase-label match, and improve its color match**

In `crates/aivyx-web/src/main.rs`, find `phase_label` (around line 5091):

```rust
fn phase_label(p: TeamMissionPhase) -> &'static str {
    match p {
        TeamMissionPhase::Planning => "planning",
        TeamMissionPhase::Executing => "executing",
        TeamMissionPhase::AwaitingApproval => "awaiting approval",
        TeamMissionPhase::Done => "done",
        TeamMissionPhase::Rejected => "rejected",
        TeamMissionPhase::Halted => "halted",
    }
}
```

Add the new arm (this match has no wildcard, so it genuinely won't
compile without it):

```rust
fn phase_label(p: TeamMissionPhase) -> &'static str {
    match p {
        TeamMissionPhase::Planning => "planning",
        TeamMissionPhase::Executing => "executing",
        TeamMissionPhase::AwaitingApproval => "awaiting approval",
        TeamMissionPhase::Paused => "paused",
        TeamMissionPhase::Done => "done",
        TeamMissionPhase::Rejected => "rejected",
        TeamMissionPhase::Halted => "halted",
    }
}
```

Just below it, `phase_class` already has a `_ => ""` wildcard, so it
compiles unchanged — but leaving `Paused` falling through to `""` (no
color) is a worse operator experience than the other operator-attention
phases get. Add an explicit arm reusing the same "amber" tone
`AwaitingApproval` already gets (both are operator-pause points):

```rust
fn phase_class(p: TeamMissionPhase) -> &'static str {
    match p {
        TeamMissionPhase::AwaitingApproval => "amber",
        TeamMissionPhase::Paused => "amber",
        TeamMissionPhase::Done => "sage",
        TeamMissionPhase::Rejected => "error",
        TeamMissionPhase::Halted => "error",
        _ => "",
    }
}
```

- [ ] **Step 4: Add `MissionPhase::Paused` to `aivyx-tui`, and fix its 3 exhaustive matches**

`aivyx-tui` has its own local `MissionPhase` enum (distinct from
`TeamMissionPhase` — it's the TUI's own view-model type), and it does
**not** already have an unused `Paused` variant the way `StepState`
already had an unused `Running` before Piece 1 — this one needs a genuine
new variant, not just a new mapping arm.

In `crates/aivyx-tui/src/model.rs`, find:

```rust
pub enum MissionPhase {
    /// Decomposed, not yet running.
    Planning,
    /// At least one step is running.
    Executing,
    /// Blocked on an operator approval gate.
    AwaitingApproval,
    /// Every step completed.
    Done,
    /// A quality gate rejected the work (`MissionStatus::GateRejected`).
    Rejected,
    /// Chapter Ballast — halted by a per-mission budget cap.
    Halted,
```

(Confirm the exact closing of this enum block before editing — read the
file around this point rather than assuming the exact brace position, since
this plan was written from a snapshot and the file may have shifted
slightly.) Add a new variant, in the same relative position (right after
`AwaitingApproval`):

```rust
    /// Chapter Mission Control — paused at a wave boundary by an explicit
    /// operator pause request. Non-terminal; resumable.
    Paused,
```

Now fix the three exhaustive matches on `MissionPhase`/`TeamMissionPhase`
this file and `render.rs` contain:

1. `model.rs`'s `.label()` method (around line 146):
```rust
            MissionPhase::Planning => "planning",
            MissionPhase::Executing => "executing",
            MissionPhase::AwaitingApproval => "approval",
            MissionPhase::Done => "done",
            MissionPhase::Rejected => "rejected",
            MissionPhase::Halted => "halted",
```
Add, after `AwaitingApproval`'s arm: `MissionPhase::Paused => "paused",`

2. `model.rs`'s `TeamMissionPhase → MissionPhase` projection (around line
   259 — the function this plan's Task 1 exists to keep compiling):
```rust
        TeamMissionPhase::Planning => MissionPhase::Planning,
        TeamMissionPhase::Executing => MissionPhase::Executing,
        TeamMissionPhase::AwaitingApproval => MissionPhase::AwaitingApproval,
        TeamMissionPhase::Done => MissionPhase::Done,
        TeamMissionPhase::Rejected => MissionPhase::Rejected,
        TeamMissionPhase::Halted => MissionPhase::Halted,
```
Add, after `AwaitingApproval`'s arm: `TeamMissionPhase::Paused => MissionPhase::Paused,`

3. `crates/aivyx-tui/src/render.rs`'s `(icon, color)` match (around line
   190):
```rust
        MissionPhase::Executing => ("● executing", palette::OK),
        MissionPhase::AwaitingApproval => ("⚑ approval", palette::AMBER),
        MissionPhase::Planning => ("◦ planning", palette::DIM),
        MissionPhase::Done => ("✓ done", palette::DIMMER),
        MissionPhase::Rejected => ("✗ rejected", palette::ERR),
        MissionPhase::Halted => ("⊘ halted", palette::ERR),
```
Add, in the same file (position within the match doesn't matter
functionally, but place it near `AwaitingApproval`'s arm for a reader
scanning top to bottom): `MissionPhase::Paused => ("⏸ paused", palette::AMBER),`
(matching `aivyx-web`'s own choice of amber for the same "operator-pause"
semantic).

Add a test in `model.rs`'s existing test module proving the projection
(mirror the existing `assert_eq!(rows[0].phase, MissionPhase::Rejected);`-
style test near line 1008 — find its exact fixture-building pattern and
match it):

```rust
    #[test]
    fn paused_team_mission_phase_projects_to_paused_mission_phase() {
        let mut rec = sample(TeamMissionPhase::Paused, None);
        let rows = missions_with_phase(MissionPhase::Paused, Some(&mut rec));
        // Adapt this assertion to whatever this test module's own
        // established fixture/assertion shape actually is once you read
        // it -- the point is: prove TeamMissionPhase::Paused really does
        // become MissionPhase::Paused through the real projection
        // function, not a re-implementation of it.
    }
```

(This last test's exact shape depends on `sample`/`missions_with_phase`'s
real signatures in this file — read them first; if `sample` doesn't
accept a bare `TeamMissionPhase` the way shown, adapt to whatever it
really takes. The assertion that matters is: a record with
`TeamMissionPhase::Paused` produces a row with `MissionPhase::Paused`.)

- [ ] **Step 5: Run the tests to verify everything compiles and passes**

Run: `cargo build --workspace --exclude aivyx-desktop --exclude aivyx-web`
Expected: clean — confirms all 5 exhaustive matches now compile.

Run: `cargo test -p aivyx-ipc -p aivyx-tui -- --test-threads=1`
Expected: all pass, including the new `paused_is_not_terminal` and
`paused_team_mission_phase_projects_to_paused_mission_phase` tests.

Run: `cargo test -p aivyx-web mission_control_tests` (native — this crate
genuinely compiles/tests natively, confirmed during Piece 1's own Task 6)
Expected: unaffected, still passes (this task didn't touch any of Piece
1's own pure functions).

Build `aivyx-web` for the real wasm32 target too (same isolated toolchain
Piece 1's implementers used, likely still present at
`/tmp/claude-1000/{rustup-home,cargo-home}` — check there first):
`cargo build -p aivyx-web --target wasm32-unknown-unknown`
Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/aivyx-ipc/src/team_mission.rs crates/aivyx-tui/src/model.rs crates/aivyx-tui/src/render.rs crates/aivyx-web/src/main.rs crates/aivyx-cli/src/bin/aivyx_modules/team_cli.rs
git commit -m "feat: add TeamMissionPhase::Paused + fix all 5 cross-crate render sites

Adding a variant to a public, non-#[non_exhaustive] enum breaks every
exhaustive match on it -- found and fixed all 5 (aivyx-cli's phase_label,
aivyx-web's phase_label + phase_class, aivyx-tui's MissionPhase::label(),
the TeamMissionPhase->MissionPhase projection, and render.rs's icon/color
match) via a full grep sweep before writing this task, rather than
discovering them one at a time. is_terminal() needed no change -- Paused
is non-terminal simply by omission. No wiring to any real pause/resume
mechanism yet -- Tasks 2-5."
```

---

## Task 2: `SharedMissionState` pause-flag tracking (mirrors `abort_flags` exactly)

**Files:**
- Modify: `crates/aivyx-channel/src/team_mission_driver.rs`

**Interfaces:**
- Produces: `SharedMissionState::arm_pause` (private), `disarm_pause` (private), `request_pause` (public) — mirroring `arm_abort`/`disarm_abort`/`request_abort`'s exact existing shapes.

**Verified**: `SharedMissionState`'s current `abort_flags` field + its 3
methods (`arm_abort`, `disarm_abort`, `request_abort`) are the exact
precedent to mirror — read below, copied verbatim from the current file.

- [ ] **Step 1: Write the failing tests**

Find `SharedMissionState`'s test module (the same one Piece 1's Task 3
added `mark_running`/`clear_running` tests to) and add:

```rust
    #[tokio::test]
    async fn request_pause_returns_false_for_an_unarmed_mission() {
        let shared = SharedMissionState::new(team_domain().await);
        assert!(!shared.request_pause("no-such-mission"));
    }
```

(Confirmed: this file's test fixture is the async `team_domain()` helper
— there is no sync variant. `arm_pause`/`disarm_pause` are private, so a
public test can only observe them indirectly through `request_pause`'s
own return value, same as the existing abort tests already do.)

```rust
    #[tokio::test]
    async fn arm_pause_then_request_pause_sets_the_flag() {
        let shared = SharedMissionState::new(team_domain().await);
        // arm_pause is private -- call it via the same test-module access
        // the existing abort tests already use (this test lives inside
        // `mod tests`, which has access to private items in the same file).
        let flag = shared.arm_pause("m1");
        assert!(!flag.load(std::sync::atomic::Ordering::SeqCst));
        assert!(shared.request_pause("m1"));
        assert!(flag.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn disarm_pause_removes_the_flag() {
        let shared = SharedMissionState::new(team_domain().await);
        shared.arm_pause("m1");
        shared.disarm_pause("m1");
        assert!(!shared.request_pause("m1"), "no flag left to set");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p aivyx-channel team_mission_driver -- --test-threads=1`
Expected: FAIL to compile — `arm_pause`/`disarm_pause`/`request_pause`
don't exist yet.

- [ ] **Step 3: Add the field and methods**

Find `SharedMissionState`'s struct definition (has `abort_flags:
Arc<RwLock<HashMap<String, Arc<AtomicBool>>>>` and, after Piece 1,
`running_steps`/`broadcaster` too). Add a new field right after
`abort_flags`:

```rust
    /// Chapter Mission Control — runtime-only pause flags, keyed by
    /// mission id, mirroring `abort_flags` exactly (armed by the drive,
    /// read by the observer at each wave boundary, set by
    /// `request_pause`). Not persisted for the same reason `abort_flags`
    /// isn't — an armed-but-unresolved pause flag is meaningless across a
    /// restart (an interrupted mission simply isn't paused-by-request
    /// after a restart; `reload`'s own zombie-reconciliation already
    /// handles the interrupted-mid-drive case by landing such missions in
    /// `Halted`, not `Paused`).
    pause_flags: Arc<RwLock<std::collections::HashMap<String, Arc<std::sync::atomic::AtomicBool>>>>,
```

Update `SharedMissionState::new` to initialize it (`pause_flags:
Arc::new(RwLock::new(std::collections::HashMap::new()))`, alongside the
existing `abort_flags`/`running_steps`/`broadcaster` initializers).

Add the three methods right after `request_abort`:

```rust
    /// Chapter Mission Control — arm a fresh pause flag for an executing
    /// mission and return it, mirroring `arm_abort` exactly (the drive
    /// hands the clone to the observer's `should_halt`, and keeps its own
    /// copy to check after the run completes, for phase selection).
    fn arm_pause(&self, id: &str) -> Arc<std::sync::atomic::AtomicBool> {
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        self.pause_flags
            .write()
            .expect("pause flags lock")
            .insert(id.to_string(), Arc::clone(&flag));
        flag
    }

    /// Chapter Mission Control — drop a mission's pause flag once its
    /// drive ends, mirroring `disarm_abort` exactly.
    fn disarm_pause(&self, id: &str) {
        self.pause_flags.write().expect("pause flags lock").remove(id);
    }

    /// Chapter Mission Control — request that an executing mission pause
    /// at its next wave boundary, mirroring `request_abort` exactly.
    /// Returns `true` if the mission was running (a flag was armed),
    /// `false` if not (already terminal, paused, or unknown).
    pub fn request_pause(&self, id: &str) -> bool {
        match self.pause_flags.read().expect("pause flags lock").get(id) {
            Some(flag) => {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                true
            }
            None => false,
        }
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p aivyx-channel team_mission_driver -- --test-threads=1`
Expected: PASS — the 3 new tests plus everything pre-existing in this
file (unaffected — nothing calls `arm_pause`/`disarm_pause` yet outside
these new tests).

- [ ] **Step 5: Commit**

```bash
git add crates/aivyx-channel/src/team_mission_driver.rs
git commit -m "feat: SharedMissionState tracks pause flags, mirroring abort_flags

arm_pause/disarm_pause/request_pause are the exact same shape as
arm_abort/disarm_abort/request_abort. Not persisted, same rationale.
Nothing wires this into a real drive yet -- Task 3."
```

---

## Task 3: Wire pause into `should_halt`, `drive()`, and `DriveCleanupGuard`

**Files:**
- Modify: `crates/aivyx-channel/src/team_mission_driver.rs`

**Interfaces:**
- Consumes: `SharedMissionState::arm_pause`/`disarm_pause` (Task 2).
- Produces: nothing new for later tasks — this is where a mission
  actually starts landing in `Paused` for real.

**Verified**: this is a genuine restructuring of `drive()`'s existing
`MissionStatus::Halted { reason } => { ... }` match arm — read the exact
current body below before editing (copied verbatim from the current
file); do not assume it matches any prior summary of this function, since
Piece 1 already restructured it once (adding `DriveCleanupGuard`).

- [ ] **Step 1: Write the failing test**

`RegistryObserver` is private; find its existing test(s) (search for
`RegistryObserver {` — Piece 1's Task 4 added
`on_step_started_marks_running_then_completion_clears_it` here, a good
neighbor). Add:

```rust
    #[tokio::test]
    async fn should_halt_prioritizes_abort_over_pause_over_budget() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let shared = SharedMissionState::new(team_domain().await);
        let (tx, _rx) = mpsc::unbounded_channel();

        // Pause armed and tripped, abort armed but NOT tripped, no budget:
        // pause's own reason wins.
        let pause = Arc::new(AtomicBool::new(true));
        let abort = Arc::new(AtomicBool::new(false));
        let observer = RegistryObserver {
            shared: shared.clone(),
            id: "m1".to_string(),
            tx: tx.clone(),
            budget_guard: None,
            abort: Some(Arc::clone(&abort)),
            pause: Some(Arc::clone(&pause)),
        };
        assert_eq!(observer.should_halt(), Some("paused by operator".to_string()));

        // Now also trip abort -- abort must win over pause, even though
        // pause is still armed and tripped too.
        abort.store(true, Ordering::SeqCst);
        assert_eq!(observer.should_halt(), Some("aborted by operator".to_string()));
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p aivyx-channel should_halt_prioritizes -- --test-threads=1`
Expected: FAIL to compile — `RegistryObserver` has no `pause` field yet.

- [ ] **Step 3: Add the `pause` field and the priority check**

Find `RegistryObserver`'s struct definition:

```rust
struct RegistryObserver {
    shared: SharedMissionState,
    id: String,
    tx: mpsc::UnboundedSender<()>,
    budget_guard: Option<(crate::mission_meter::MissionMeter, aivyx_cost::MissionBudget)>,
    abort: Option<Arc<std::sync::atomic::AtomicBool>>,
}
```

Add a new field:

```rust
struct RegistryObserver {
    shared: SharedMissionState,
    id: String,
    tx: mpsc::UnboundedSender<()>,
    budget_guard: Option<(crate::mission_meter::MissionMeter, aivyx_cost::MissionBudget)>,
    abort: Option<Arc<std::sync::atomic::AtomicBool>>,
    /// Chapter Mission Control — the mission's pause flag, mirroring
    /// `abort` exactly. Read at each wave boundary in `should_halt`, with
    /// lower priority than an abort but higher than the Ballast budget
    /// check.
    pause: Option<Arc<std::sync::atomic::AtomicBool>>,
}
```

Find `should_halt`:

```rust
    fn should_halt(&self) -> Option<String> {
        // Chapter Belay — an operator abort takes priority over the budget check.
        if let Some(flag) = &self.abort {
            if flag.load(std::sync::atomic::Ordering::SeqCst) {
                return Some("aborted by operator".to_string());
            }
        }
        let (meter, budget) = self.budget_guard.as_ref()?;
        budget.breach(meter.tokens(), meter.usd())
    }
```

Change to:

```rust
    fn should_halt(&self) -> Option<String> {
        // Chapter Belay — an operator abort takes priority over pause and
        // the budget check.
        if let Some(flag) = &self.abort {
            if flag.load(std::sync::atomic::Ordering::SeqCst) {
                return Some("aborted by operator".to_string());
            }
        }
        // Chapter Mission Control — a pause request takes priority over
        // the budget check, but never over an abort (checked above,
        // unconditionally, first).
        if let Some(flag) = &self.pause {
            if flag.load(std::sync::atomic::Ordering::SeqCst) {
                return Some("paused by operator".to_string());
            }
        }
        let (meter, budget) = self.budget_guard.as_ref()?;
        budget.breach(meter.tokens(), meter.usd())
    }
```

- [ ] **Step 4: Run the new test to verify it passes, then wire `drive()`**

Run: `cargo test -p aivyx-channel should_halt_prioritizes -- --test-threads=1`
Expected: PASS.

Now find `drive()`. Its current relevant section:

```rust
    // The observer pings on each step completion; the drive runs in a task so
    // that when it ends the observer (and its sender) drop, closing the ping
    // channel and ending the drain loop.
    // Chapter Belay — arm this mission's abort flag; the observer halts the run
    // at the next wave boundary if an operator requests an abort.
    let abort = shared.arm_abort(id);
    // Chapter Mission Control — from here on, EVERY exit path (the two
    // early `?` returns below, a panic, or normal completion) runs
    // cleanup exactly once via Drop, not just the happy path.
    let _cleanup_guard = DriveCleanupGuard { shared, id };
    let (tx, mut rx) = mpsc::unbounded_channel();
    let observer = RegistryObserver {
        shared: shared.clone(),
        id: id.to_string(),
        tx,
        budget_guard,
        abort: Some(abort),
    };
```

Change to:

```rust
    // The observer pings on each step completion; the drive runs in a task so
    // that when it ends the observer (and its sender) drop, closing the ping
    // channel and ending the drain loop.
    // Chapter Belay — arm this mission's abort flag; the observer halts the run
    // at the next wave boundary if an operator requests an abort.
    let abort = shared.arm_abort(id);
    // Chapter Mission Control — arm this mission's pause flag too. Kept as
    // a local binding (not just handed to the observer) because drive()
    // itself checks it AFTER the run completes, to decide whether a
    // MissionStatus::Halted outcome should land in Paused instead.
    let pause = shared.arm_pause(id);
    // Chapter Mission Control — from here on, EVERY exit path (the two
    // early `?` returns below, a panic, or normal completion) runs
    // cleanup exactly once via Drop, not just the happy path.
    let _cleanup_guard = DriveCleanupGuard { shared, id };
    let (tx, mut rx) = mpsc::unbounded_channel();
    let observer = RegistryObserver {
        shared: shared.clone(),
        id: id.to_string(),
        tx,
        budget_guard,
        abort: Some(abort),
        pause: Some(Arc::clone(&pause)),
    };
```

Now find the `MissionStatus::Halted { reason } => { ... }` arm inside
`drive()`'s later `match outcome { RunYield::Done(report) => { ... match
report.status { ... } ... } ... }`:

```rust
                MissionStatus::Halted { reason } => {
                    eprintln!(
                        "aivyx team: mission {id} halted — {reason}"
                    );
                    audit.on_event(AuditTag::HeadlessRefusal {
                        run_id: id.to_string(),
                        step: "<halt>".to_string(),
                        reason: format!("team mission halted: {reason}"),
                    });
                    halted_reason = Some(reason);
                    TeamMissionPhase::Halted
                }
```

Change to:

```rust
                MissionStatus::Halted { reason } => {
                    if pause.load(std::sync::atomic::Ordering::SeqCst) {
                        // Chapter Mission Control — a pause request
                        // tripped this halt, not an abort or budget cap
                        // (should_halt's own priority order guarantees
                        // this branch is only reached when abort is NOT
                        // armed) -- land in the new, non-terminal Paused
                        // phase instead of Halted. No halt_reason is set
                        // (that field's contract: "set iff phase ==
                        // Halted") -- outputs are preserved exactly like
                        // a real Halted landing, ready for resume to
                        // continue from. Still logged (distinctly) for
                        // operator visibility, but NOT sent to the audit
                        // chain -- HeadlessRefusal means "declined," which
                        // an operator-requested pause isn't.
                        eprintln!("aivyx team: mission {id} paused");
                        TeamMissionPhase::Paused
                    } else {
                        eprintln!(
                            "aivyx team: mission {id} halted — {reason}"
                        );
                        audit.on_event(AuditTag::HeadlessRefusal {
                            run_id: id.to_string(),
                            step: "<halt>".to_string(),
                            reason: format!("team mission halted: {reason}"),
                        });
                        halted_reason = Some(reason);
                        TeamMissionPhase::Halted
                    }
                }
```

Finally, find `DriveCleanupGuard`'s `Drop` impl:

```rust
impl Drop for DriveCleanupGuard<'_> {
    fn drop(&mut self) {
        self.shared.disarm_abort(self.id);
        self.shared.clear_all_running(self.id);
    }
}
```

Change to:

```rust
impl Drop for DriveCleanupGuard<'_> {
    fn drop(&mut self) {
        self.shared.disarm_abort(self.id);
        self.shared.disarm_pause(self.id);
        self.shared.clear_all_running(self.id);
    }
}
```

- [ ] **Step 5: Write a genuinely end-to-end test proving pause lands in `Paused`, not `Halted`**

This is the real proof the whole task exists for — go beyond the unit-level
`should_halt` test from Step 1. Find the file's existing full-drive
integration tests (search for ones that call `drive_registered` or
`team_run` directly and assert on the resulting phase — e.g. near the
existing `budget_halt_stops_at_wave_boundary_preserving_outputs`-style
test, if this file has one, or `anchor_observer_halts_when_aborted`) to
find the exact fixture/harness this file's own end-to-end drive tests
already use, and mirror it exactly rather than inventing a new harness.
Write a test that: starts a multi-step mission, requests a pause via
`shared.request_pause(id)` (or `SharedMissionState`'s equivalent public
entry point this file's existing abort-during-drive test already uses to
simulate a mid-run request), lets the drive run to its next wave
boundary, and asserts the resulting record's `phase` is
`TeamMissionPhase::Paused` (not `Halted`), its `halt_reason` is `None`,
and at least one step's output is preserved in the checkpoint (proving
outputs survive the pause the same way they survive a halt).

Match this file's own established pattern precisely for how a mid-drive
abort is already tested (there should be a very close existing precedent
for "request X while a drive is running, from another task/thread, and
assert the drive lands where X implies") — read that test fully before
writing this one, since the exact mechanism for injecting a pause request
*during* an in-flight `drive()` call (not before it starts) needs to
match however the existing abort equivalent does it.

- [ ] **Step 6: Run the full test module**

Run: `cargo test -p aivyx-channel team_mission_driver -- --test-threads=1`
Expected: all pass — the new tests from this task, plus everything
pre-existing (this task touches `should_halt`, `drive()`, and
`DriveCleanupGuard`'s `Drop` — all three have significant existing
coverage; a full, not partial, pass matters here).

- [ ] **Step 7: Commit**

```bash
git add crates/aivyx-channel/src/team_mission_driver.rs
git commit -m "feat: wire pause into should_halt/drive()/DriveCleanupGuard

should_halt's priority order is now abort > pause > budget cap. drive()
arms its own pause flag (kept local, not just handed to the observer) so
it can distinguish a pause-triggered halt from any other MissionStatus::
Halted outcome after the run completes, landing in the new non-terminal
Paused phase with no halt_reason set. DriveCleanupGuard now disarms the
pause flag too, unconditionally, on every exit path -- same rationale as
its existing abort-flag cleanup."
```

---

## Task 4: Service-layer `pause`/`resume` API (mirrors `abort`/`resolve` exactly)

**Files:**
- Modify: `crates/aivyx-channel/src/team_mission_driver.rs`

**Interfaces:**
- Consumes: `SharedMissionState::request_pause` (Task 2), `TeamMissionPhase::Paused` (Task 1).
- Produces: `MissionDriverError::NotPausable(String, String)`, `MissionDriverError::NotResumable(String, TeamMissionPhase)` (new error variants). `pause_mission(shared, id) -> Result<String, MissionDriverError>` (free function, mirrors `abort_mission` exactly). `prepare_pause_resolution(shared, id) -> Result<TeamMissionPhase, MissionDriverError>` (free function, mirrors `prepare_gate_resolution`'s split shape exactly). `TeamMissionService::pause(&self, id: &str) -> Result<String, MissionDriverError>` (mirrors `abort`). `TeamMissionService::resume(&self, id: &str) -> Result<TeamMissionPhase, MissionDriverError>` (mirrors `resolve`, minus its reject branch — resume only ever succeeds into `Executing` or errors).

**Verified**: `abort_mission`/`TeamMissionService::abort` and
`prepare_gate_resolution`/`TeamMissionService::resolve` are the two exact
precedents this task mirrors — both read in full below, copied verbatim
from the current file.

- [ ] **Step 1: Write the failing tests**

Add near this file's existing `abort_mission`/`prepare_gate_resolution`
tests (search for `fn abort_mission_` or similar to find that test
module's own established fixture pattern for building a registered
mission at a known phase, and match it exactly):

```rust
    #[tokio::test]
    async fn pause_mission_requires_executing_phase() {
        let shared = SharedMissionState::new(team_domain().await);
        let plan = MissionPlan::new("goal", vec![Step::delegate("a", "specialist", "do a")]);
        let mut record = TeamMissionRecord::new("m1", "goal", plan);
        record.phase = TeamMissionPhase::Done;
        shared.put(record).await.expect("put");

        let err = pause_mission(&shared, "m1").unwrap_err();
        assert!(matches!(err, MissionDriverError::NotPausable(..)));
    }

    #[tokio::test]
    async fn pause_mission_on_an_unknown_id_is_not_found() {
        let shared = SharedMissionState::new(team_domain().await);
        let err = pause_mission(&shared, "no-such-mission").unwrap_err();
        assert!(matches!(err, MissionDriverError::NotFound(..)));
    }

    #[tokio::test]
    async fn prepare_pause_resolution_requires_paused_phase() {
        let shared = SharedMissionState::new(team_domain().await);
        let plan = MissionPlan::new("goal", vec![Step::delegate("a", "specialist", "do a")]);
        let mut record = TeamMissionRecord::new("m1", "goal", plan);
        record.phase = TeamMissionPhase::Executing;
        shared.put(record).await.expect("put");

        let err = prepare_pause_resolution(&shared, "m1").await.unwrap_err();
        assert!(matches!(err, MissionDriverError::NotResumable(..)));
    }

    #[tokio::test]
    async fn prepare_pause_resolution_flips_paused_to_executing() {
        let shared = SharedMissionState::new(team_domain().await);
        let plan = MissionPlan::new("goal", vec![Step::delegate("a", "specialist", "do a")]);
        let mut record = TeamMissionRecord::new("m1", "goal", plan);
        record.phase = TeamMissionPhase::Paused;
        record.outputs.insert("a".to_string(), "partial output".to_string());
        shared.put(record).await.expect("put");

        let phase = prepare_pause_resolution(&shared, "m1").await.expect("resolve");
        assert_eq!(phase, TeamMissionPhase::Executing);
        let after = shared.snapshot("m1").expect("still present");
        assert_eq!(after.phase, TeamMissionPhase::Executing);
        assert_eq!(
            after.outputs.get("a"),
            Some(&"partial output".to_string()),
            "checkpoint survives the resume"
        );
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p aivyx-channel pause_mission -- --test-threads=1`
Run: `cargo test -p aivyx-channel prepare_pause_resolution -- --test-threads=1`
Expected: FAIL to compile — none of the new items exist yet.

- [ ] **Step 3: Add the error variants**

Find `MissionDriverError`:

```rust
pub enum MissionDriverError {
    /// A storage read/write failed.
    #[error("mission store error: {0}")]
    Store(#[from] StorageError),
    /// Assembling or running the team failed.
    #[error("team error: {0}")]
    Team(#[from] TeamError),
    /// The named mission isn't in the registry.
    #[error("no such mission: {0}")]
    NotFound(String),
    /// `resolve_team_gate` was called on a mission that isn't awaiting one.
    #[error("mission {0} is not awaiting a gate decision (phase {1:?})")]
    NotAwaiting(String, TeamMissionPhase),
    /// `resolve_team_gate` named a step that isn't the pending gate.
    #[error("mission {0} is awaiting gate {1:?}, not {2:?}")]
    WrongGate(String, String, String),
    /// The mission-drive task panicked.
    #[error("mission run task failed: {0}")]
    Join(String),
    /// Chapter Belay — abort was requested on a mission that isn't running.
    #[error("mission {0} cannot be aborted ({1})")]
    NotAbortable(String, String),
}
```

Add two variants right after `NotAbortable`:

```rust
    /// Chapter Mission Control — pause was requested on a mission that
    /// isn't running.
    #[error("mission {0} cannot be paused ({1})")]
    NotPausable(String, String),
    /// Chapter Mission Control — resume was requested on a mission that
    /// isn't paused.
    #[error("mission {0} is not paused (phase {1:?})")]
    NotResumable(String, TeamMissionPhase),
}
```

- [ ] **Step 4: Add `pause_mission`, mirroring `abort_mission` exactly**

Find `abort_mission` (the free function):

```rust
pub fn abort_mission(
    shared: &SharedMissionState,
    id: &str,
) -> Result<String, MissionDriverError> {
    let record = shared
        .snapshot(id)
        .ok_or_else(|| MissionDriverError::NotFound(id.to_string()))?;
    match record.phase {
        TeamMissionPhase::Executing => {
            if shared.request_abort(id) {
                Ok(format!(
                    "abort requested — mission {id} will halt at its next step boundary"
                ))
            } else {
                Err(MissionDriverError::NotAbortable(
                    id.to_string(),
                    "the mission is no longer running".to_string(),
                ))
            }
        }
        TeamMissionPhase::AwaitingApproval => Err(MissionDriverError::NotAbortable(
            id.to_string(),
            "it is paused at a human gate — reject the gate instead".to_string(),
        )),
        other => Err(MissionDriverError::NotAbortable(
            id.to_string(),
            format!("it is not currently running (phase {other:?})"),
        )),
    }
}
```

Add a new function right after it, mirroring its exact shape:

```rust
/// Chapter Mission Control — request that a **running** mission pause.
/// Sets the mission's pause flag; its drive pauses gracefully at the next
/// wave boundary (in-flight specialist turns finish, completed outputs
/// are preserved), landing the mission in the new, non-terminal `Paused`
/// phase — distinct from `abort_mission`, whose landing (`Halted`) is
/// terminal. A mission paused at a human gate isn't running the same way,
/// so it can't be paused this way either — resolve its gate instead.
/// Returns a short status message on success.
pub fn pause_mission(
    shared: &SharedMissionState,
    id: &str,
) -> Result<String, MissionDriverError> {
    let record = shared
        .snapshot(id)
        .ok_or_else(|| MissionDriverError::NotFound(id.to_string()))?;
    match record.phase {
        TeamMissionPhase::Executing => {
            if shared.request_pause(id) {
                Ok(format!(
                    "pause requested — mission {id} will pause at its next step boundary"
                ))
            } else {
                Err(MissionDriverError::NotPausable(
                    id.to_string(),
                    "the mission is no longer running".to_string(),
                ))
            }
        }
        TeamMissionPhase::AwaitingApproval => Err(MissionDriverError::NotPausable(
            id.to_string(),
            "it is paused at a human gate — reject the gate instead".to_string(),
        )),
        other => Err(MissionDriverError::NotPausable(
            id.to_string(),
            format!("it is not currently running (phase {other:?})"),
        )),
    }
}
```

- [ ] **Step 5: Add `prepare_pause_resolution`, mirroring `prepare_gate_resolution`'s split shape**

Find `prepare_gate_resolution` (already read in full during this plan's
own research — its real shape validates the current phase, mutates and
persists via `put`, and returns the new phase for the caller to decide
whether to spawn a drive). Add a new function near it:

```rust
/// Chapter Mission Control — validate `id` is `Paused`, flip it back to
/// `Executing`, and persist (`put` broadcasts the transition
/// automatically). Mirrors `prepare_gate_resolution`'s own split shape:
/// the daemon flips persisted state before spawning the (long) resume
/// drive. Unlike gate resolution, there is no "reject" branch here —
/// resume either succeeds into `Executing` or errors.
pub async fn prepare_pause_resolution(
    shared: &SharedMissionState,
    id: &str,
) -> Result<TeamMissionPhase, MissionDriverError> {
    let mut record = shared
        .snapshot(id)
        .ok_or_else(|| MissionDriverError::NotFound(id.to_string()))?;
    if record.phase != TeamMissionPhase::Paused {
        return Err(MissionDriverError::NotResumable(id.to_string(), record.phase));
    }
    record.phase = TeamMissionPhase::Executing;
    shared.put(record).await?;
    Ok(TeamMissionPhase::Executing)
}
```

- [ ] **Step 6: Add `TeamMissionService::pause`/`resume`**

Find `TeamMissionService::abort`:

```rust
    /// Chapter Belay — request that a running mission halt at its next wave
    /// boundary. Returns a short status message.
    pub fn abort(&self, id: &str) -> Result<String, MissionDriverError> {
        abort_mission(&self.state, id)
    }
```

And `TeamMissionService::resolve` (already read in full — the exact split
"prepare, then conditionally spawn_drive" shape). Add two new methods
near them:

```rust
    /// Chapter Mission Control — request that a running mission pause at
    /// its next wave boundary. Returns a short status message.
    pub fn pause(&self, id: &str) -> Result<String, MissionDriverError> {
        pause_mission(&self.state, id)
    }

    /// Chapter Mission Control — resume a paused mission, spawning the
    /// resume drive. Returns the immediate phase (always `Executing` on
    /// success).
    pub async fn resume(&self, id: &str) -> Result<TeamMissionPhase, MissionDriverError> {
        let phase = prepare_pause_resolution(&self.state, id).await?;
        self.spawn_drive(id.to_string());
        Ok(phase)
    }
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p aivyx-channel team_mission_driver -- --test-threads=1`
Expected: all pass, including the 4 new tests from Step 1.

- [ ] **Step 8: Commit**

```bash
git add crates/aivyx-channel/src/team_mission_driver.rs
git commit -m "feat: TeamMissionService::pause/resume, mirroring abort/resolve exactly

pause_mission/prepare_pause_resolution are free functions mirroring
abort_mission/prepare_gate_resolution's exact shapes. resume() has no
reject branch (unlike resolve()) -- it either succeeds into Executing or
errors. No IPC/CLI wiring yet -- Tasks 5/6."
```

---

## Task 5: IPC wire messages + daemon dispatch + client functions

**Files:**
- Modify: `crates/aivyx-ipc/src/protocol.rs`
- Modify: `crates/aivyx-channel/src/daemon_server.rs`
- Modify: `crates/aivyx-channel/src/daemon_client.rs`

**Interfaces:**
- Consumes: `TeamMissionService::pause`/`resume` (Task 4).
- Produces: `QueryPayload::PauseTeamMission { mission_id: String }` / `QueryPayload::ResumeTeamMission { mission_id: String }`. `QueryResponsePayload::TeamMissionPaused { mission_id: String, message: String }` / `QueryResponsePayload::TeamMissionResumed { mission_id: String, phase: TeamMissionPhase }`. `daemon_client::pause_team_mission(socket_path, mission_id) -> Result<String, DaemonError>` / `daemon_client::resume_team_mission(socket_path, mission_id) -> Result<TeamMissionPhase, DaemonError>`.

**Verified**: `AbortTeamMission`/`TeamMissionAborted` (a short-message
response) and `ResolveTeamGate`/`TeamGateResolved` (a phase response) are
the two exact wire-shape precedents — `PauseTeamMission` mirrors the
first (pause doesn't complete synchronously, just requests a graceful
stop), `ResumeTeamMission` mirrors the second (resume synchronously flips
the phase and starts a background drive, exactly like gate-approve does).

- [ ] **Step 1: Add the `QueryPayload` variants**

In `crates/aivyx-ipc/src/protocol.rs`, find:

```rust
    /// Chapter Belay — request that a running mission halt at its next wave
    /// boundary. Responds with [`QueryResponsePayload::TeamMissionAborted`].
    AbortTeamMission {
        mission_id: String,
    },
```

Add two new variants right after it:

```rust
    /// Chapter Mission Control — request that a running mission pause at
    /// its next wave boundary (resumable, unlike abort). Responds with
    /// [`QueryResponsePayload::TeamMissionPaused`].
    PauseTeamMission {
        mission_id: String,
    },
    /// Chapter Mission Control — resume a paused mission; the resume
    /// drives in the background. Responds with
    /// [`QueryResponsePayload::TeamMissionResumed`].
    ResumeTeamMission {
        mission_id: String,
    },
```

- [ ] **Step 2: Add the `QueryResponsePayload` variants**

Find:

```rust
    /// Chapter Belay — response to [`QueryPayload::AbortTeamMission`]. A short
    /// human-readable status (the mission will halt at its next wave boundary).
    TeamMissionAborted {
        mission_id: String,
        message: String,
    },
```

Add two new variants right after it:

```rust
    /// Chapter Mission Control — response to
    /// [`QueryPayload::PauseTeamMission`]. A short human-readable status
    /// (the mission will pause at its next wave boundary).
    TeamMissionPaused {
        mission_id: String,
        message: String,
    },
    /// Chapter Mission Control — response to
    /// [`QueryPayload::ResumeTeamMission`]. The phase the resume moved the
    /// mission to (always `Executing` — the resume drives in the
    /// background).
    TeamMissionResumed {
        mission_id: String,
        phase: crate::TeamMissionPhase,
    },
```

- [ ] **Step 3: Add a wire round-trip test**

Find this file's existing round-trip test for `AbortTeamMission`/
`TeamMissionAborted` (search for either name in the test module) and add
analogous cases for the two new message pairs, matching that test's exact
style (likely an encode→decode round trip asserting field equality).

- [ ] **Step 4: Run the tests, then wire the daemon dispatch**

Run: `cargo test -p aivyx-ipc -- --test-threads=1`
Expected: pass, including the new round-trip test.

In `crates/aivyx-channel/src/daemon_server.rs`, find:

```rust
        QueryPayload::AbortTeamMission { mission_id } => {
            let Some(svc) = team_missions else {
                return no_team_missions();
            };
            match svc.abort(&mission_id) {
                Ok(message) => QueryResponsePayload::TeamMissionAborted {
                    mission_id,
                    message,
                },
                Err(e) => QueryResponsePayload::QueryError {
                    code: "abort_team_mission_failed".into(),
                    message: e.to_string(),
                },
            }
        }
```

Add two new arms right after it:

```rust
        QueryPayload::PauseTeamMission { mission_id } => {
            let Some(svc) = team_missions else {
                return no_team_missions();
            };
            match svc.pause(&mission_id) {
                Ok(message) => QueryResponsePayload::TeamMissionPaused {
                    mission_id,
                    message,
                },
                Err(e) => QueryResponsePayload::QueryError {
                    code: "pause_team_mission_failed".into(),
                    message: e.to_string(),
                },
            }
        }
        QueryPayload::ResumeTeamMission { mission_id } => {
            let Some(svc) = team_missions else {
                return no_team_missions();
            };
            match svc.resume(&mission_id).await {
                Ok(phase) => QueryResponsePayload::TeamMissionResumed {
                    mission_id,
                    phase,
                },
                Err(e) => QueryResponsePayload::QueryError {
                    code: "resume_team_mission_failed".into(),
                    message: e.to_string(),
                },
            }
        }
```

- [ ] **Step 5: Add the client functions**

In `crates/aivyx-channel/src/daemon_client.rs`, find `abort_team_mission`:

```rust
/// Chapter Belay — request that a running team mission halt. Returns the
/// daemon's status message.
pub async fn abort_team_mission(
    socket_path: &Path,
    mission_id: String,
) -> Result<String, DaemonError> {
    let payload = send_query(
        socket_path,
        "abort-team-mission",
        QueryPayload::AbortTeamMission { mission_id },
    )
    .await?;
    match payload {
        QueryResponsePayload::TeamMissionAborted { message, .. } => Ok(message),
        QueryResponsePayload::QueryError { code, message } => {
            Err(DaemonError::Protocol(format!("{code}: {message}")))
        }
        other => Err(DaemonError::Protocol(format!(
            "expected TeamMissionAborted, got {other:?}"
        ))),
    }
}
```

(Read the rest of this function if the snippet above is cut off in the
real file before editing — the shape above is what this plan's own
research confirmed, but confirm the exact closing braces/match arms in
the real file first.) Add two new functions mirroring it:

```rust
/// Chapter Mission Control — request that a running team mission pause
/// (resumable, unlike abort). Returns the daemon's status message.
pub async fn pause_team_mission(
    socket_path: &Path,
    mission_id: String,
) -> Result<String, DaemonError> {
    let payload = send_query(
        socket_path,
        "pause-team-mission",
        QueryPayload::PauseTeamMission { mission_id },
    )
    .await?;
    match payload {
        QueryResponsePayload::TeamMissionPaused { message, .. } => Ok(message),
        QueryResponsePayload::QueryError { code, message } => {
            Err(DaemonError::Protocol(format!("{code}: {message}")))
        }
        other => Err(DaemonError::Protocol(format!(
            "expected TeamMissionPaused, got {other:?}"
        ))),
    }
}

/// Chapter Mission Control — resume a paused team mission. Returns the
/// phase the resume moved it to (always `Executing`).
pub async fn resume_team_mission(
    socket_path: &Path,
    mission_id: String,
) -> Result<crate::team_mission::TeamMissionPhase, DaemonError> {
    let payload = send_query(
        socket_path,
        "resume-team-mission",
        QueryPayload::ResumeTeamMission { mission_id },
    )
    .await?;
    match payload {
        QueryResponsePayload::TeamMissionResumed { phase, .. } => Ok(phase),
        QueryResponsePayload::QueryError { code, message } => {
            Err(DaemonError::Protocol(format!("{code}: {message}")))
        }
        other => Err(DaemonError::Protocol(format!(
            "expected TeamMissionResumed, got {other:?}"
        ))),
    }
}
```

- [ ] **Step 6: Run the full crate tests, build the workspace**

Run: `cargo test -p aivyx-ipc -p aivyx-channel -- --test-threads=1`
Expected: all pass.

Run: `cargo build --workspace --exclude aivyx-desktop --exclude aivyx-web`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/aivyx-ipc/src/protocol.rs crates/aivyx-channel/src/daemon_server.rs crates/aivyx-channel/src/daemon_client.rs
git commit -m "feat: wire PauseTeamMission/ResumeTeamMission over IPC

PauseTeamMission mirrors AbortTeamMission's shape (a short status
message); ResumeTeamMission mirrors ResolveTeamGate's shape (the new
phase). Daemon dispatch and client-side functions both added, mirroring
their exact abort/resolve precedents. No CLI surface yet -- Task 6."
```

---

## Task 6: CLI (`aivyx team pause|resume <id>`)

**Files:**
- Modify: `crates/aivyx-cli/src/bin/aivyx.rs`
- Modify: `crates/aivyx-cli/src/bin/aivyx_modules/team_cli.rs`

**Interfaces:**
- Consumes: `daemon_client::pause_team_mission`/`resume_team_mission` (Task 5).

**Verified**: `TeamSubcommand::Abort { mission_id: String }`'s argv
parsing, `is_daemon_verb()` matcher, and `team_cli.rs`'s execution arm are
the exact precedents — all read in full during this plan's own research.

- [ ] **Step 1: Add the `TeamSubcommand` variants**

In `crates/aivyx-cli/src/bin/aivyx.rs`, find:

```rust
    /// Chapter Belay — `aivyx team abort <id>`: stop a running mission (it
    /// halts gracefully at its next step boundary, preserving completed work).
    Abort { mission_id: String },
}
```

Add two new variants right before the closing `}`:

```rust
    /// Chapter Belay — `aivyx team abort <id>`: stop a running mission (it
    /// halts gracefully at its next step boundary, preserving completed work).
    Abort { mission_id: String },
    /// Chapter Mission Control — `aivyx team pause <id>`: pause a running
    /// mission at its next step boundary — resumable, unlike abort.
    Pause { mission_id: String },
    /// Chapter Mission Control — `aivyx team resume <id>`: resume a
    /// paused mission from its checkpoint.
    Resume { mission_id: String },
}
```

Update `is_daemon_verb`:

```rust
    fn is_daemon_verb(&self) -> bool {
        matches!(
            self,
            TeamSubcommand::Start { .. }
                | TeamSubcommand::StartGoal { .. }
                | TeamSubcommand::List
                | TeamSubcommand::Status { .. }
                | TeamSubcommand::Approve { .. }
                | TeamSubcommand::Reject { .. }
                | TeamSubcommand::Abort { .. }
                | TeamSubcommand::Pause { .. }
                | TeamSubcommand::Resume { .. }
        )
    }
```

- [ ] **Step 2: Add argv parsing**

Find:

```rust
            "abort" => {
                let mission_id = args.get(2).cloned().ok_or_else(|| {
                    "`aivyx team abort` requires a <mission-id>".to_string()
                })?;
                TeamSubcommand::Abort { mission_id }
            }
            "" => {
                return Err(
                    "`aivyx team` requires a subcommand: roster | init | run | start | \
                     list | status | approve | reject | abort"
                        .to_string(),
                );
            }
            other => {
                return Err(format!(
                    "unknown `aivyx team` subcommand `{other}` (expected: roster | \
                     init | run | start | list | status | approve | reject)"
                ));
            }
```

Change to:

```rust
            "abort" => {
                let mission_id = args.get(2).cloned().ok_or_else(|| {
                    "`aivyx team abort` requires a <mission-id>".to_string()
                })?;
                TeamSubcommand::Abort { mission_id }
            }
            "pause" => {
                let mission_id = args.get(2).cloned().ok_or_else(|| {
                    "`aivyx team pause` requires a <mission-id>".to_string()
                })?;
                TeamSubcommand::Pause { mission_id }
            }
            "resume" => {
                let mission_id = args.get(2).cloned().ok_or_else(|| {
                    "`aivyx team resume` requires a <mission-id>".to_string()
                })?;
                TeamSubcommand::Resume { mission_id }
            }
            "" => {
                return Err(
                    "`aivyx team` requires a subcommand: roster | init | run | start | \
                     list | status | approve | reject | abort | pause | resume"
                        .to_string(),
                );
            }
            other => {
                return Err(format!(
                    "unknown `aivyx team` subcommand `{other}` (expected: roster | \
                     init | run | start | list | status | approve | reject | abort | \
                     pause | resume)"
                ));
            }
```

(Note the two usage strings above are inconsistent with each other in
the *current* file even before this edit — the empty-subcommand message
lists `abort` but the unknown-subcommand message's parenthetical doesn't.
Fix both to the same complete list while you're here, since leaving one
updated and one stale would be a worse inconsistency than either was
before.)

- [ ] **Step 3: Wire execution in `team_cli.rs`**

In `crates/aivyx-cli/src/bin/aivyx_modules/team_cli.rs`, find:

```rust
        TeamSubcommand::Abort { mission_id } => {
            let message =
                aivyx_channel::daemon_client::abort_team_mission(&socket_path, mission_id)
                    .await
                    .map_err(|e| format!("team abort failed: {e}"))?;
            println!("{message}");
            Ok(())
        }
```

Add two new arms right after it:

```rust
        TeamSubcommand::Pause { mission_id } => {
            let message =
                aivyx_channel::daemon_client::pause_team_mission(&socket_path, mission_id)
                    .await
                    .map_err(|e| format!("team pause failed: {e}"))?;
            println!("{message}");
            Ok(())
        }
        TeamSubcommand::Resume { mission_id } => {
            let phase = aivyx_channel::daemon_client::resume_team_mission(
                &socket_path,
                mission_id.clone(),
            )
            .await
            .map_err(|e| format!("team resume failed: {e}"))?;
            println!("mission {mission_id} resumed (now {phase:?})");
            println!("track it with `aivyx team status {mission_id}`");
            Ok(())
        }
```

- [ ] **Step 4: Run the full workspace test suite**

Run: `cargo build --workspace --exclude aivyx-desktop --exclude aivyx-web`
Expected: clean.

Run: `cargo test --workspace --exclude aivyx-desktop --exclude aivyx-web -- --test-threads=1`
Expected: full green. Report the exact total vs. the pre-Piece-2 baseline.

Run: `cargo clippy -p aivyx-ipc -p aivyx-channel -p aivyx-cli -p aivyx-tui --all-targets`
Expected: no new warnings.

Run `aivyx team pause --help`-equivalent sanity check (there's no
`--help` machinery visible in this argv parser — instead, run `aivyx team
""` i.e. no subcommand, or an unknown one, and confirm the printed usage
string now correctly lists `pause`/`resume` in both places).

- [ ] **Step 5: Commit**

```bash
git add crates/aivyx-cli/src/bin/aivyx.rs crates/aivyx-cli/src/bin/aivyx_modules/team_cli.rs
git commit -m "feat: aivyx team pause|resume <id> CLI commands

Mirrors 'aivyx team abort <id>''s exact argv/dispatch shape. Also fixed
a pre-existing inconsistency between the two usage-string error messages
(one already listed abort, the other's parenthetical didn't) while
touching this exact code."
```

---

## Final verification (whole plan)

- [ ] `cargo build --workspace --exclude aivyx-desktop --exclude aivyx-web`
      — clean.
- [ ] `cargo test --workspace --exclude aivyx-desktop --exclude aivyx-web -- --test-threads=1`
      — all green; note the new total vs. the pre-Piece-2 baseline (Piece
      1 shipped at 5607 passed, 0 failed, 2 ignored on this same command).
- [ ] `cargo clippy -p aivyx-ipc -p aivyx-channel -p aivyx-cli -p aivyx-tui --all-targets`
      — no new warnings.
- [ ] `cargo build -p aivyx-web --target wasm32-unknown-unknown` — clean
      (Task 1 touched `aivyx-web`'s two phase-render functions; confirm
      they still compile for the real target, not just natively).
- [ ] Confirm (by reading the diff, not just running tests) that Piece 3
      (a new Mission Control nav view in `aivyx-web`) was **not**
      started — this plan's scope is Piece 2 only.
- [ ] Manually confirm the full operator round trip makes sense end to
      end by reading (not necessarily running) the call chain once more:
      `aivyx team pause <id>` → `pause_team_mission` (client) →
      `PauseTeamMission` (wire) → `daemon_server`'s dispatch →
      `TeamMissionService::pause` → `pause_mission` → `request_pause` →
      (next wave boundary) `should_halt` returns `Some("paused by
      operator")` → `drive()` sees `MissionStatus::Halted`, checks its
      own `pause` flag, lands in `TeamMissionPhase::Paused` →
      `DriveCleanupGuard` disarms both abort and pause flags → `aivyx
      team resume <id>` → `resume_team_mission` (client) →
      `ResumeTeamMission` (wire) → `daemon_server`'s dispatch →
      `TeamMissionService::resume` → `prepare_pause_resolution` flips to
      `Executing` and persists → `spawn_drive` drives from the preserved
      checkpoint.
