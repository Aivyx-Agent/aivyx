# Missions Polish Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the 5 findings in `docs/POLISH_WAVES.md` sub-project 5 (rejected-mission reason display, gate labels with attempt context, handoff-fidelity prompts, mission topic-naming discipline, and a contradictory-memory badge with full resolve/dismiss).

**Architecture:** Two small wire-type/rendering fixes in `aivyx-ipc`/`aivyx-web` (A, B), a one-string prompt fix in `aivyx-team` (C), a deterministic mission-scoped memory-topic-prefix wired through `aivyx-team`'s specialist factory and `aivyx-channel`'s mission driver (D), and a Studio-side wiring of an already-complete backend loop for memory conflicts (E).

**Tech Stack:** Rust workspace (`cargo test --workspace --exclude aivyx-desktop`, `cargo clippy --workspace --exclude aivyx-desktop --all-targets -- -D warnings`), Dioxus (Studio).

## Global Constraints

- `aivyx-desktop` is excluded from all workspace-wide commands — no system webkit2gtk libs in this sandbox, this repo's own established practice.
- Every new wire-type field (`TeamMissionView.verify_attempts`) uses `#[serde(default)]`, matching `halt_reason`'s own precedent, so old snapshots still deserialize.
- `TeamAssembly::build`'s existing 11-positional-argument signature is not refactored into a builder/config-struct pattern — out of scope, noted as a pre-existing code smell in the design spec.
- No change to Concord's detection logic (`contradiction.rs`) or to Persona/Soul conflicts (Chapter Accord) — item E only surfaces what Concord already detects for memory.
- The Studio `dist/` bundle is tracked; any `aivyx-web` source change ships with a rebuilt, committed bundle in the same commit. If `dx bundle` errors with a version mismatch, `cargo install dioxus-cli --version 0.6.3 --locked --force` first (`--locked` is load-bearing — already hit and fixed once in this workspace).

---

### Task 1: Rejected-mission reason display

**Files:**
- Modify: `crates/aivyx-web/src/main.rs:2427-2450` (`MissionRow`)

**Interfaces:** none new — `mission.halt_reason: Option<String>` already exists on `TeamMissionView`.

- [ ] **Step 1: Make the rendering change**

No dedicated unit test for this task: it's a one-block conditional inside a Dioxus `#[component]` with no branching logic to extract into a testable pure function (matches this workspace's own precedent for prior one-block Studio rendering fixes — verification is compile + clippy + the rebuilt bundle). This task touches only the `row1`/`halt_reason` part of `MissionRow` shown below — leave the `GateControls { ... }` call at the bottom exactly as it is today (unchanged, 2-argument form); Task 2 updates that call site separately. Find `MissionRow` in `crates/aivyx-web/src/main.rs`:

```rust
#[component]
fn MissionRow(mission: TeamMissionView) -> Element {
    let pct = mission.progress.min(100);
    let awaiting = mission.phase == TeamMissionPhase::AwaitingApproval;
    rsx! {
        div { class: "glass-card mission",
            div { class: "row1",
                span { class: "chip {phase_class(mission.phase)}", "{phase_label(mission.phase)}" }
                span { class: "goal", "{mission.goal}" }
                span { class: "lead label-tech", "{mission.lead}" }
            }
            div { class: "progress", div { class: "fill", style: "width: {pct}%;" } }
            div { class: "steps",
                for step in mission.steps.iter() {
                    span { class: "step label-tech", "{step.label}" }
                }
            }
            if awaiting {
                if let Some(gate) = mission.pending_gate.clone() {
                    GateControls { mission_id: mission.id.clone(), step: gate }
                }
            }
        }
    }
}
```

Replace with (adds one conditional block right after `row1`; the `GateControls` call at the bottom gains a `verify_attempts` prop here too, since Task 2 lands in the same file — do both edits in one pass if executing Task 1 and Task 2 together, otherwise Task 2 will update this exact call site again):

```rust
#[component]
fn MissionRow(mission: TeamMissionView) -> Element {
    let pct = mission.progress.min(100);
    let awaiting = mission.phase == TeamMissionPhase::AwaitingApproval;
    rsx! {
        div { class: "glass-card mission",
            div { class: "row1",
                span { class: "chip {phase_class(mission.phase)}", "{phase_label(mission.phase)}" }
                span { class: "goal", "{mission.goal}" }
                span { class: "lead label-tech", "{mission.lead}" }
            }
            // POLISH_WAVES.md sub-project 5, item A — the operator
            // previously saw REJECTED/HALTED with no explanation;
            // halt_reason already carries the judge's precise verdict
            // (or the halt cause) and already flows over the wire.
            if let Some(reason) = mission.halt_reason.as_ref() {
                div { class: "notice err", "{reason}" }
            }
            div { class: "progress", div { class: "fill", style: "width: {pct}%;" } }
            div { class: "steps",
                for step in mission.steps.iter() {
                    span { class: "step label-tech", "{step.label}" }
                }
            }
            if awaiting {
                if let Some(gate) = mission.pending_gate.clone() {
                    GateControls { mission_id: mission.id.clone(), step: gate }
                }
            }
        }
    }
}
```

(The `GateControls { ... }` call is unchanged here — still the 2-argument form Task 2 will later extend. `class: "notice err"` is an existing CSS class already used elsewhere in this file for error-toned banners — `crates/aivyx-web/assets/stitch.css:543`: `.notice.err { background: rgba(196, 85, 62, 0.15); color: var(--color-error); }`. No new CSS needed.)

- [ ] **Step 2: Compile-check and clippy (native, no wasm32 target needed)**

Run: `cargo check -p aivyx-web && cargo clippy -p aivyx-web --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 3: Rebuild the wasm bundle**

```bash
export PATH="$HOME/.rustup/toolchains/stable-x86_64-unknown-linux-gnu/bin:$HOME/.cargo/bin:$PATH"
cd crates/aivyx-web && dx bundle --release --platform web
```

Copy `target/dx/aivyx-web/release/web/public/` over `crates/aivyx-web/dist/` (**replace** the directory contents — `rm -rf crates/aivyx-web/dist && cp -r target/dx/aivyx-web/release/web/public crates/aivyx-web/dist`, not a merging `cp -r src/* dst/`, which has previously left a stale orphaned wasm binary behind), delete any `*.br` files (`find crates/aivyx-web/dist -name '*.br' -delete`), and confirm `git ls-tree -r main -- crates/aivyx-web/dist | grep -c '\.br$'` prints `0` before committing.

Run: `git status --porcelain crates/aivyx-web/dist/` and confirm exactly one `.wasm` file changed (no leftover old one) via `git status --porcelain crates/aivyx-web/dist/assets/ | grep wasm`.

- [ ] **Step 4: Commit**

```bash
git add crates/aivyx-web/src/main.rs crates/aivyx-web/dist/
git commit -m "feat(web): show the rejection/halt reason on a mission row

POLISH_WAVES.md sub-project 5, item A. halt_reason already carried
the judge's precise verdict (or halt cause) and already flowed over
the wire; MissionRow never rendered it. Mission Control's own live
graph deliberately excludes Rejected/Halted missions from its
'watchable' set, so the plain Missions list is where this belongs.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 2: Gate labels with attempt context

**Files:**
- Modify: `crates/aivyx-ipc/src/team_mission.rs` (`TeamMissionView` struct, `to_view()`)
- Modify: `crates/aivyx-web/src/main.rs` (`GateControls`, its two call sites, new `gate_label` helper)
- Test: `crates/aivyx-ipc/src/team_mission.rs`, `crates/aivyx-web/src/main.rs` (existing `#[cfg(test)]` modules)

**Interfaces:**
- Consumes: `TeamMissionRecord.verify_attempts: u32` (pre-existing).
- Produces: `TeamMissionView.verify_attempts: u32` (new field); `fn gate_label(step: &str, verify_attempts: u32) -> String` (new, private, in `main.rs`).

- [ ] **Step 1: Write the failing `to_view()` test**

Add to `crates/aivyx-ipc/src/team_mission.rs`'s existing `#[cfg(test)] mod tests` block, near `to_view_derives_step_states_and_progress` (search for it — it uses a `sample(id)` helper already in this file):

```rust
    #[test]
    fn to_view_carries_verify_attempts() {
        let mut rec = sample("v3");
        rec.verify_attempts = 2;
        let view = rec.to_view();
        assert_eq!(view.verify_attempts, 2);
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p aivyx-ipc to_view_carries_verify_attempts -- --exact`
Expected: FAIL with "no field `verify_attempts` on type `TeamMissionView`".

- [ ] **Step 3: Add the field and thread it through `to_view()`**

In `crates/aivyx-ipc/src/team_mission.rs`, find `TeamMissionView`'s struct definition:

```rust
pub struct TeamMissionView {
    pub id: String,
    pub goal: String,
    /// The team lead's name (the pack's lead, or `coordinator` for the
    /// default Nonagon) — shown in the feed.
    pub lead: String,
    pub phase: TeamMissionPhase,
    /// The step id awaiting an operator decision, when `phase ==
    /// AwaitingApproval` — what `aivyx team approve|reject <id> <step>` /
    /// the approve/reject affordances target.
    pub pending_gate: Option<String>,
    /// Why the mission `Halted` (budget cap detail, or "aborted by operator"),
    /// when `phase == Halted`. Lets a client show *which* cause ended it.
    #[serde(default)]
    pub halt_reason: Option<String>,
    /// Completion percent in `0..=100` (completed steps / total).
    pub progress: u16,
    pub steps: Vec<TeamStepView>,
}
```

Add the new field after `halt_reason`:

```rust
pub struct TeamMissionView {
    pub id: String,
    pub goal: String,
    /// The team lead's name (the pack's lead, or `coordinator` for the
    /// default Nonagon) — shown in the feed.
    pub lead: String,
    pub phase: TeamMissionPhase,
    /// The step id awaiting an operator decision, when `phase ==
    /// AwaitingApproval` — what `aivyx team approve|reject <id> <step>` /
    /// the approve/reject affordances target.
    pub pending_gate: Option<String>,
    /// Why the mission `Halted` (budget cap detail, or "aborted by operator"),
    /// when `phase == Halted`. Lets a client show *which* cause ended it.
    #[serde(default)]
    pub halt_reason: Option<String>,
    /// POLISH_WAVES.md sub-project 5, item B — the mission's current
    /// Chapter Reprise verification-retry attempt count, so a repeated
    /// approval gate (the same `gate_review_brief` re-shown on each
    /// retry) can tell the operator which attempt this is. `0`/`1` both
    /// mean "no retry has happened yet" — same meaning as
    /// `TeamMissionRecord::verify_attempts`'s own default of `0`.
    #[serde(default)]
    pub verify_attempts: u32,
    /// Completion percent in `0..=100` (completed steps / total).
    pub progress: u16,
    pub steps: Vec<TeamStepView>,
}
```

Find `to_view()`'s construction of `TeamMissionView` (search for `halt_reason: self.halt_reason.clone(),`):

```rust
        TeamMissionView {
            id: self.id.clone(),
            goal: self.goal.clone(),
            lead: self
                .config
                .as_ref()
                .map(|c| c.lead.clone())
                .unwrap_or_else(|| "coordinator".to_string()),
            phase: self.phase,
            pending_gate: self.pending_gate.clone(),
            halt_reason: self.halt_reason.clone(),
            progress: ((done * 100) / total) as u16,
            steps,
        }
```

Add `verify_attempts` right after `halt_reason`:

```rust
        TeamMissionView {
            id: self.id.clone(),
            goal: self.goal.clone(),
            lead: self
                .config
                .as_ref()
                .map(|c| c.lead.clone())
                .unwrap_or_else(|| "coordinator".to_string()),
            phase: self.phase,
            pending_gate: self.pending_gate.clone(),
            halt_reason: self.halt_reason.clone(),
            verify_attempts: self.verify_attempts,
            progress: ((done * 100) / total) as u16,
            steps,
        }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p aivyx-ipc to_view_carries_verify_attempts -- --exact`
Expected: PASS.

- [ ] **Step 5: Write the failing `gate_label` tests**

Add to `crates/aivyx-web/src/main.rs`'s existing `#[cfg(test)] mod tests` block (search for `mission_controls_shown_for_each_phase`, add nearby):

```rust
    #[test]
    fn gate_label_omits_attempt_suffix_on_first_attempt() {
        assert_eq!(
            gate_label("gate_review_brief", 0),
            "⚑ awaiting approval — gate_review_brief"
        );
        assert_eq!(
            gate_label("gate_review_brief", 1),
            "⚑ awaiting approval — gate_review_brief"
        );
    }

    #[test]
    fn gate_label_includes_attempt_suffix_on_retry() {
        assert_eq!(
            gate_label("gate_review_brief", 2),
            "⚑ awaiting approval — gate_review_brief (attempt 2)"
        );
    }
```

- [ ] **Step 6: Run tests to verify they fail**

Run: `cargo test -p aivyx-web gate_label -- --exact`
Expected: FAIL with "cannot find function `gate_label`".

- [ ] **Step 7: Add `gate_label` and wire it into `GateControls`**

Add this function near `GateControls` in `crates/aivyx-web/src/main.rs` (e.g. directly above it):

```rust
/// POLISH_WAVES.md sub-project 5, item B — the gate label, with attempt
/// context only when this isn't the first attempt (a first-attempt gate
/// needs no "(attempt 1)" noise). Extracted as a pure function so it's
/// testable without a Dioxus runtime.
fn gate_label(step: &str, verify_attempts: u32) -> String {
    if verify_attempts > 1 {
        format!("⚑ awaiting approval — {step} (attempt {verify_attempts})")
    } else {
        format!("⚑ awaiting approval — {step}")
    }
}
```

Find `GateControls`:

```rust
#[component]
fn GateControls(mission_id: String, step: String) -> Element {
    let ws = use_context::<Sender>();
    let approve = (mission_id.clone(), step.clone());
    let reject = (mission_id.clone(), step.clone());
    let label = step.clone();
    rsx! {
        div { class: "gate",
            span { class: "gate-label", "⚑ awaiting approval — {label}" }
            button {
                class: "btn btn-sage",
                onclick: move |_| ws.send(resolve_team_query(approve.0.clone(), approve.1.clone(), true)),
                "Approve Sequence"
            }
            button {
                class: "btn btn-ghost-danger",
                onclick: move |_| ws.send(resolve_team_query(reject.0.clone(), reject.1.clone(), false)),
                "Reject"
            }
        }
    }
}
```

Replace with:

```rust
#[component]
fn GateControls(mission_id: String, step: String, verify_attempts: u32) -> Element {
    let ws = use_context::<Sender>();
    let approve = (mission_id.clone(), step.clone());
    let reject = (mission_id.clone(), step.clone());
    let label = gate_label(&step, verify_attempts);
    rsx! {
        div { class: "gate",
            span { class: "gate-label", "{label}" }
            button {
                class: "btn btn-sage",
                onclick: move |_| ws.send(resolve_team_query(approve.0.clone(), approve.1.clone(), true)),
                "Approve Sequence"
            }
            button {
                class: "btn btn-ghost-danger",
                onclick: move |_| ws.send(resolve_team_query(reject.0.clone(), reject.1.clone(), false)),
                "Reject"
            }
        }
    }
}
```

- [ ] **Step 8: Update both call sites**

In `MissionRow` (`crates/aivyx-web/src/main.rs`), find:

```rust
            if awaiting {
                if let Some(gate) = mission.pending_gate.clone() {
                    GateControls { mission_id: mission.id.clone(), step: gate }
                }
            }
```

Replace with:

```rust
            if awaiting {
                if let Some(gate) = mission.pending_gate.clone() {
                    GateControls { mission_id: mission.id.clone(), step: gate, verify_attempts: mission.verify_attempts }
                }
            }
```

In `MissionControls` (search for the other `GateControls {` call site), find:

```rust
            if shown.contains(&"gate") {
                if let Some(gate) = mission.pending_gate.clone() {
                    GateControls { mission_id: mission.id.clone(), step: gate }
                }
            }
```

Replace with:

```rust
            if shown.contains(&"gate") {
                if let Some(gate) = mission.pending_gate.clone() {
                    GateControls { mission_id: mission.id.clone(), step: gate, verify_attempts: mission.verify_attempts }
                }
            }
```

- [ ] **Step 9: Run tests to verify they pass**

Run: `cargo test -p aivyx-web gate_label`
Expected: both new tests PASS.

- [ ] **Step 10: Full-crate check, compile-check, clippy, rebuild bundle, and commit**

Run: `cargo test -p aivyx-ipc && cargo clippy -p aivyx-ipc --all-targets -- -D warnings`
Expected: clean.

Run: `cargo check -p aivyx-web && cargo clippy -p aivyx-web --all-targets -- -D warnings`
Expected: clean.

Rebuild the wasm bundle exactly as in Task 1 Step 3 (same commands, same `rm -rf` + `cp -r` replace, same `.br` cleanup, same verification).

```bash
git add crates/aivyx-ipc/src/team_mission.rs crates/aivyx-web/src/main.rs crates/aivyx-web/dist/
git commit -m "feat(mission): show retry-attempt context on repeated approval gates

POLISH_WAVES.md sub-project 5, item B. TeamMissionRecord's own
verify_attempts (persisted specifically so a gated mission's retry
count survives gate-approval re-entries — Chapter Reprise) was never
added to the wire TeamMissionView, so a re-shown gate on a retry
looked identical to the first attempt. GateControls now shows
'(attempt N)' once N > 1.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 3: Handoff-fidelity prompts

**Files:**
- Modify: `crates/aivyx-team/src/runtime.rs:311` (`build_input`)

**Interfaces:** none — a string literal change in a private function.

- [ ] **Step 1: Confirm no test pins the old header text verbatim**

Run: `grep -rn "Context from upstream steps" crates/aivyx-team/src/ crates/aivyx-channel/src/`
Expected: only the one production site (`runtime.rs:311`) matches — no test asserts this exact string. If a test DOES match, read it and update its expected string to match Step 2's new text before proceeding (do not skip this check).

- [ ] **Step 2: Change the header text**

Find in `crates/aivyx-team/src/runtime.rs`'s `build_input`:

```rust
        if step.deps.is_empty() {
            return base;
        }
        let mut ctx = String::from("\n\n--- Context from upstream steps ---");
```

Replace with:

```rust
        if step.deps.is_empty() {
            return base;
        }
        // POLISH_WAVES.md sub-project 5, item C — live mission repro
        // (725be8d8): the writer specialist opened its turn by reading
        // `workspace: …/brief_text.md`, a file no step ever wrote — the
        // real handoff (the researcher's output) was sitting right here
        // in the message. State plainly that this text IS the input.
        let mut ctx = String::from(
            "\n\n--- Context from upstream steps (this IS your real \
             input — nothing is written to a file for you) ---",
        );
```

- [ ] **Step 3: Run the crate's tests**

Run: `cargo test -p aivyx-team`
Expected: all existing tests pass (this is a prompt-text-only change; no test should have depended on the exact old wording beyond what Step 1 already checked).

- [ ] **Step 4: Full-crate check and commit**

Run: `cargo clippy -p aivyx-team --all-targets -- -D warnings`
Expected: clean.

```bash
git add crates/aivyx-team/src/runtime.rs
git commit -m "feat(team): state plainly that upstream context IS the real input

POLISH_WAVES.md sub-project 5, item C. Live mission repro: a
specialist opened its turn by reading a workspace file no step ever
wrote, ignoring the real handoff sitting in its own task message.
build_input is the single shared prompt-assembly function for both
the CLI's 'aivyx team run' and Mission-Control-driven missions, so
one string fixes every mission path.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 4: Mission topic-naming discipline

> **⏳ Reverted at final review (2026-08-31, commit f02f0032) — do not
> re-execute this task as written.** The mechanism below was
> implemented, then found to (1) not actually fix the naming-
> discipline finding it cites and (2) measurably harm Task 5's own
> Concord conflict-detector plus knowledge-wiki synthesis cost plus
> the Memory screen's topic rail. See `docs/POLISH_WAVES.md`'s §5 for
> the full account and the lesson for a future attempt. Left below
> for historical reference only.

**Files:**
- Modify: `crates/aivyx-team/src/factory.rs` (`SpecialistFactory` struct + builder + `build()`)
- Modify: `crates/aivyx-team/src/assembly.rs` (`TeamAssembly::build` signature + its 2 test call sites)
- Modify: `crates/aivyx-channel/src/team_mission_driver.rs` (`assemble_runtime` signature + its 1 call site, + the `TeamAssembly::build` call inside it)
- Modify: `crates/aivyx-team/Cargo.toml` (new dev-dependency)
- Test: `crates/aivyx-team/src/factory.rs` (existing `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `ConcreteAgent::with_memory_topic_prefix(Option<String>) -> Self` (pre-existing, `aivyx-core/src/agent.rs:277`).
- Produces: `SpecialistFactory::with_mission_topic_prefix(Option<String>) -> Self`; `TeamAssembly::build`'s new trailing `mission_topic_prefix: Option<String>` parameter.

- [ ] **Step 1: Add the dev-dependency**

In `crates/aivyx-team/Cargo.toml`, find:

```toml
[dev-dependencies]
# Turn-running tests need a Tokio runtime for the sub-turn; J.4.2's
# concurrency proof needs a Barrier + a timeout guard (sync + time).
tokio = { workspace = true, features = ["macros", "rt", "time", "sync"] }
# Final-review fix wave (remaining-sites plan) — the checkpoint e2e test in
# factory.rs drives a real GitCheckpointer::detect + test_support git
# helpers against a real fs_root repo, mirroring aivyx-core's own agent.rs
# checkpoint tests and the three channel crates' analogous tests.
aivyx-checkpoint = { workspace = true }
tempfile = "3.27.0"
```

Replace with (adds `aivyx-memory`, needed for Step 8's end-to-end test — confirmed no dependency cycle: `aivyx-memory` doesn't depend on `aivyx-team`):

```toml
[dev-dependencies]
# Turn-running tests need a Tokio runtime for the sub-turn; J.4.2's
# concurrency proof needs a Barrier + a timeout guard (sync + time).
tokio = { workspace = true, features = ["macros", "rt", "time", "sync"] }
# Final-review fix wave (remaining-sites plan) — the checkpoint e2e test in
# factory.rs drives a real GitCheckpointer::detect + test_support git
# helpers against a real fs_root repo, mirroring aivyx-core's own agent.rs
# checkpoint tests and the three channel crates' analogous tests.
aivyx-checkpoint = { workspace = true }
tempfile = "3.27.0"
# POLISH_WAVES.md sub-project 5, item D — the mission-topic-prefix e2e
# test drives a real MemoryWriteTool + InMemoryMemory, mirroring the
# checkpointer e2e test's own shape one field over.
aivyx-memory = { path = "../aivyx-memory" }
```

- [ ] **Step 2: Write the failing end-to-end test**

Add to `crates/aivyx-team/src/factory.rs`'s existing `#[cfg(test)] mod tests` block, right after `build_attaches_the_checkpointer_when_configured` (search for it):

```rust
    /// End-to-end proof that `SpecialistFactory::build` actually wires a
    /// mission-scoped topic prefix into
    /// `ConcreteAgent::new(...).with_memory_topic_prefix(...)` — not just
    /// that `build()` still returns `Ok` (which would pass identically
    /// whether the wiring exists or not, since `new` already defaults the
    /// prefix to `None`). Mirrors `build_attaches_the_checkpointer_when_
    /// configured`'s exact shape, one field over: drives a real
    /// `memory.write` tool call through a real `SpecialistFactory::build`-
    /// constructed `ConcreteAgent`, then inspects the real `Memory`
    /// substrate to confirm the entry landed under the PREFIXED physical
    /// topic, not the bare logical one the agent typed.
    #[tokio::test]
    async fn build_attaches_the_mission_topic_prefix_when_configured() {
        use crate::testutil::{FakeLeadChannel, FakeProvider};
        use aivyx_core::{ChannelContext, Message};
        use aivyx_memory::{InMemoryMemory, Memory, MemoryWriteTool};

        let memory: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
        let write_tool: Arc<dyn Tool> = Arc::new(MemoryWriteTool::new(Arc::clone(&memory)));

        // required_scope reads the UNPREFIXED logical topic the agent
        // types (the prefix is applied only at execute-time, inside the
        // memory tool itself) — see aivyx-memory/src/tools.rs's own
        // doc comment on this exact split.
        let write_scope = "memory.write:topic:overall_conditions";
        let lead = CapabilitySet::from_scopes([Scope::parse(write_scope).unwrap()]);
        let m = member("spec", &[write_scope], &["memory.write"]);

        let provider = FakeProvider::tool_call_then_done(
            "memory.write",
            serde_json::json!({ "topic": "overall_conditions", "body": "VFR at all three fields" }),
        );
        let f = SpecialistFactory::new(provider, "test-model", 4096, Arc::new(NullAuditHook), vec![write_tool])
            .with_mission_topic_prefix(Some("m-a1b2c3d4-".to_string()));

        let specialist = f.build(&m, &lead).expect("build");

        let channel = FakeLeadChannel::at(TrustTier::Trusted);
        let message = Message::text(channel.session_id(), "log conditions");
        let _ = specialist.turn(message, &channel).await;

        let prefixed = memory.get_recent("m-a1b2c3d4-overall_conditions", 10).await.unwrap();
        assert_eq!(
            prefixed.len(),
            1,
            "the write must land under the mission-prefixed physical topic"
        );
        assert_eq!(prefixed[0].body, "VFR at all three fields");

        // The bare, unprefixed logical topic must have nothing written
        // under it — proves the prefix was actually applied, not just
        // present alongside an unprefixed duplicate.
        let unprefixed = memory.get_recent("overall_conditions", 10).await.unwrap();
        assert!(unprefixed.is_empty(), "nothing should land under the bare logical topic");
    }
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p aivyx-team build_attaches_the_mission_topic_prefix_when_configured -- --exact`
Expected: FAIL with "no method named `with_mission_topic_prefix`".

- [ ] **Step 4: Add the field and builder to `SpecialistFactory`**

In `crates/aivyx-team/src/factory.rs`, find the `SpecialistFactory` struct's last field:

```rust
    kv_cache_handles: Option<(
        Arc<aivyx_llm::KvSlotPool>,
        Arc<aivyx_kvcache::LlamaServerSlotStore>,
        String,
    )>,
}
```

Add a new field after it (before the closing `}`):

```rust
    kv_cache_handles: Option<(
        Arc<aivyx_llm::KvSlotPool>,
        Arc<aivyx_kvcache::LlamaServerSlotStore>,
        String,
    )>,
    /// POLISH_WAVES.md sub-project 5, item D — attached to every
    /// specialist this factory builds, so every `memory.write` call
    /// during this mission is automatically, deterministically
    /// prefixed with the same mission-scoped string — a real fix for
    /// specialists filing inconsistent topic names within one mission
    /// (bare-ICAO vs `overall_conditions` vs
    /// `overall_conditions_summary`, observed live), not a hopeful
    /// prompt instruction. `None` (the default) preserves pre-existing
    /// behavior.
    mission_topic_prefix: Option<String>,
}
```

Find `SpecialistFactory::new`'s constructor body:

```rust
        SpecialistFactory {
            provider,
            model: model.into(),
            max_tokens,
            audit,
            base_tools,
            dialogue: None,
            member_backends: std::collections::HashMap::new(),
            checkpointer: None,
            kv_cache_handles: None,
        }
```

Add the new field's default:

```rust
        SpecialistFactory {
            provider,
            model: model.into(),
            max_tokens,
            audit,
            base_tools,
            dialogue: None,
            member_backends: std::collections::HashMap::new(),
            checkpointer: None,
            kv_cache_handles: None,
            mission_topic_prefix: None,
        }
```

Add a new builder method near `with_checkpointer` (same style):

```rust
    /// Attach a mission-scoped memory-topic prefix to every specialist
    /// this factory builds. `None` (the default) preserves pre-existing
    /// behavior (bare logical topics, no automatic prefixing).
    pub fn with_mission_topic_prefix(mut self, prefix: Option<String>) -> Self {
        self.mission_topic_prefix = prefix;
        self
    }
```

- [ ] **Step 5: Apply it in `build()`**

Find, in `SpecialistFactory::build`:

```rust
        )
        .with_checkpointer(self.checkpointer.clone());
```

Replace with:

```rust
        )
        .with_checkpointer(self.checkpointer.clone())
        .with_memory_topic_prefix(self.mission_topic_prefix.clone());
```

- [ ] **Step 6: Run test to verify it still fails (one step at a time)**

Run: `cargo test -p aivyx-team build_attaches_the_mission_topic_prefix_when_configured -- --exact`
Expected: still FAIL, now with a compile error naming `TeamAssembly::build`/`assemble_runtime` if either was touched already — if not yet touched, this specific test (which only exercises `SpecialistFactory` directly, not `TeamAssembly`) should now PASS. Confirm:
Expected: PASS. (This test only drives `SpecialistFactory::build` directly, not through `TeamAssembly` — Steps 7-9 below thread the prefix the rest of the way through the real production call chain, verified separately in Step 10.)

- [ ] **Step 7: Thread the parameter through `TeamAssembly::build`**

In `crates/aivyx-team/src/assembly.rs`, find `TeamAssembly::build`'s signature:

```rust
    pub fn build(
        config: TeamConfig,
        provider: Arc<dyn LlmProvider>,
        model: impl Into<String>,
        max_tokens: u32,
        audit: Arc<dyn AuditHook>,
        base_tools: Vec<Arc<dyn Tool>>,
        ceiling: CapabilitySet,
        member_backends: std::collections::HashMap<
            String,
            crate::factory::SpecialistBackend,
        >,
        checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>,
        kv_cache_handles: Option<(
            Arc<aivyx_llm::KvSlotPool>,
            Arc<aivyx_kvcache::LlamaServerSlotStore>,
            String,
        )>,
        message_origin: aivyx_core::MessageOrigin,
    ) -> Result<Self, TeamError> {
```

Replace with (adds one trailing parameter):

```rust
    pub fn build(
        config: TeamConfig,
        provider: Arc<dyn LlmProvider>,
        model: impl Into<String>,
        max_tokens: u32,
        audit: Arc<dyn AuditHook>,
        base_tools: Vec<Arc<dyn Tool>>,
        ceiling: CapabilitySet,
        member_backends: std::collections::HashMap<
            String,
            crate::factory::SpecialistBackend,
        >,
        checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>,
        kv_cache_handles: Option<(
            Arc<aivyx_llm::KvSlotPool>,
            Arc<aivyx_kvcache::LlamaServerSlotStore>,
            String,
        )>,
        message_origin: aivyx_core::MessageOrigin,
        // POLISH_WAVES.md sub-project 5, item D.
        mission_topic_prefix: Option<String>,
    ) -> Result<Self, TeamError> {
```

Find, in the same function's body:

```rust
        let factory = SpecialistFactory::new(provider, model, max_tokens, audit, base_tools)
            .with_dialogue(Arc::clone(&bus), dialogue.clone())
            .with_member_backends(member_backends)
            .with_checkpointer(checkpointer)
            .with_kv_cache(kv_cache_handles);
```

Replace with:

```rust
        let factory = SpecialistFactory::new(provider, model, max_tokens, audit, base_tools)
            .with_dialogue(Arc::clone(&bus), dialogue.clone())
            .with_member_backends(member_backends)
            .with_checkpointer(checkpointer)
            .with_kv_cache(kv_cache_handles)
            .with_mission_topic_prefix(mission_topic_prefix);
```

Update the module's two existing test call sites. Find (the shared `assembly()` test helper):

```rust
    fn assembly(provider: Arc<dyn LlmProvider>) -> TeamAssembly {
        TeamAssembly::build(
            config(),
            provider,
            "test-model",
            4096,
            Arc::new(NullAuditHook),
            vec![],
            lead_caps(),
            std::collections::HashMap::new(),
            None,
            None,
            aivyx_core::MessageOrigin::Operator,
        )
        .expect("valid team")
    }
```

Replace with:

```rust
    fn assembly(provider: Arc<dyn LlmProvider>) -> TeamAssembly {
        TeamAssembly::build(
            config(),
            provider,
            "test-model",
            4096,
            Arc::new(NullAuditHook),
            vec![],
            lead_caps(),
            std::collections::HashMap::new(),
            None,
            None,
            aivyx_core::MessageOrigin::Operator,
            None,
        )
        .expect("valid team")
    }
```

Find the other direct call site (`build_rejects_an_invalid_config`):

```rust
        let result = TeamAssembly::build(
            bad,
            FakeProvider::always("x"),
            "m",
            4096,
            Arc::new(NullAuditHook),
            vec![],
            lead_caps(),
            std::collections::HashMap::new(),
            None,
            None,
            aivyx_core::MessageOrigin::Operator,
        );
```

Replace with:

```rust
        let result = TeamAssembly::build(
            bad,
            FakeProvider::always("x"),
            "m",
            4096,
            Arc::new(NullAuditHook),
            vec![],
            lead_caps(),
            std::collections::HashMap::new(),
            None,
            None,
            aivyx_core::MessageOrigin::Operator,
            None,
        );
```

- [ ] **Step 8: Run the crate's tests**

Run: `cargo test -p aivyx-team`
Expected: all tests pass, including the new `build_attaches_the_mission_topic_prefix_when_configured`.

- [ ] **Step 9: Thread the mission id through `team_mission_driver.rs`**

In `crates/aivyx-channel/src/team_mission_driver.rs`, find `assemble_runtime`'s signature:

```rust
fn assemble_runtime(
    deps: &TeamRunDeps,
    mut config: TeamConfig,
    message_origin: aivyx_core::MessageOrigin,
    // Chapter Mission Control (Fix A) — the mission's cumulative spend so
    // far, read from the persisted record right before this call. Seeded
    // into the fresh `MeteringAuditHook` below so a `[budget]` cap tracks
    // spend across the mission's WHOLE lifetime, not just this one drive
    // invocation (a resume, a gate-approval continuation, and a Chapter
    // Reprise retry all construct a fresh hook via this same function).
    seed_tokens: u64,
    seed_usd: f64,
) -> Result<(Arc<TeamRuntime>, Option<crate::mission_meter::MissionMeter>), MissionDriverError> {
```

Replace with (adds one trailing parameter — `id` is already in scope at the one call site, `drive_registered`, confirmed by reading its own signature):

```rust
fn assemble_runtime(
    deps: &TeamRunDeps,
    mut config: TeamConfig,
    message_origin: aivyx_core::MessageOrigin,
    // Chapter Mission Control (Fix A) — the mission's cumulative spend so
    // far, read from the persisted record right before this call. Seeded
    // into the fresh `MeteringAuditHook` below so a `[budget]` cap tracks
    // spend across the mission's WHOLE lifetime, not just this one drive
    // invocation (a resume, a gate-approval continuation, and a Chapter
    // Reprise retry all construct a fresh hook via this same function).
    seed_tokens: u64,
    seed_usd: f64,
    // POLISH_WAVES.md sub-project 5, item D — the mission's own id, used
    // to derive a mission-scoped memory-topic prefix.
    mission_id: &str,
) -> Result<(Arc<TeamRuntime>, Option<crate::mission_meter::MissionMeter>), MissionDriverError> {
```

Find the call site in `drive_registered`:

```rust
        let (runtime, meter) =
            assemble_runtime(deps, config, message_origin, seed_tokens, seed_usd)?;
```

Replace with:

```rust
        let (runtime, meter) =
            assemble_runtime(deps, config, message_origin, seed_tokens, seed_usd, id)?;
```

Find, inside `assemble_runtime`'s own body, the `TeamAssembly::build` call:

```rust
    let assembly = TeamAssembly::build(
        config,
        Arc::clone(&deps.provider),
        deps.model.clone(),
        deps.max_tokens,
        audit,
        deps.base_tools.clone(),
        ceiling,
        member_backends,
        deps.checkpointer.clone(),
        deps.kv_cache_handles.clone(),
        message_origin,
    )?;
    Ok((assembly.runtime(), meter))
}
```

Replace with (computes a short, readable mission-scoped prefix from the mission id — mission ids are UUID v4 strings, confirmed via the two `uuid::Uuid::new_v4().to_string()` call sites in this same file):

```rust
    // POLISH_WAVES.md sub-project 5, item D — a short, readable
    // mission-scoped prefix. Mission ids are UUID v4 strings; the first
    // 8 hex characters are enough to disambiguate concurrent missions
    // without making every memory topic name unreadably long.
    let mission_topic_prefix = format!("m-{}-", &mission_id[..mission_id.len().min(8)]);
    let assembly = TeamAssembly::build(
        config,
        Arc::clone(&deps.provider),
        deps.model.clone(),
        deps.max_tokens,
        audit,
        deps.base_tools.clone(),
        ceiling,
        member_backends,
        deps.checkpointer.clone(),
        deps.kv_cache_handles.clone(),
        message_origin,
        Some(mission_topic_prefix),
    )?;
    Ok((assembly.runtime(), meter))
}
```

- [ ] **Step 10: Run the crate's tests**

Run: `cargo test -p aivyx-channel`
Expected: all tests pass — this change is additive to an internal function signature with exactly one caller, already updated.

- [ ] **Step 11: Full-workspace check and commit**

Run: `cargo clippy -p aivyx-team -p aivyx-channel --all-targets -- -D warnings`
Expected: clean.

```bash
git add crates/aivyx-team/Cargo.toml crates/aivyx-team/src/factory.rs crates/aivyx-team/src/assembly.rs crates/aivyx-channel/src/team_mission_driver.rs
git commit -m "feat(team): deterministically prefix every mission's memory writes

POLISH_WAVES.md sub-project 5, item D. Live mission repro: one
mission's specialists filed memory under three naming conventions
(bare-ICAO, overall_conditions, overall_conditions_summary). Rather
than a prompt hint, wires up a real, already-built, previously-unused
mechanism: ConcreteAgent::with_memory_topic_prefix is already enforced
on every memory.write call, but was never wired into Nonagon
specialist construction. Every specialist in a mission now
automatically gets memory_topic_prefix = 'm-{first-8-hex-of-mission-
id}-', so a mission's own memory writes land under one consistent,
mission-scoped namespace regardless of what logical topic name each
specialist chooses.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 5: Contradictory-memory badge + resolve/dismiss

**Files:**
- Modify: `crates/aivyx-web/src/main.rs` (`MemoryState`, new `MemoryUi`, `MemoryPanel`, new query builders, new response handlers, `ws_task`/`read_task` threading)

**Interfaces:**
- Consumes: `aivyx_ipc::conflict::{MemoryConflict, ConflictSide}` (pre-existing, wasm-clean), `QueryPayload::GetMemoryConflicts`, `QueryResponsePayload::MemoryConflicts { conflicts }`, `FrontendMessage::ResolveMemoryConflict { id, topic, archive_seq }`, `FrontendMessage::DismissMemoryConflict { id, conflict_id }`, `DaemonEnvelope::MemoryConflictResolved { id, ok, removed, error }`, `DaemonEnvelope::MemoryConflictDismissed { id, ok, error }` (all pre-existing, already used by the CLI's `daemon_client.rs`).
- Produces: `MemoryState.conflicts: Vec<MemoryConflict>`; `MemoryUi { notice: Option<(bool, String)> }` (new, mirrors `SkillsUi`/`SchedulesUi`); `mem_conflicts_query() -> FrontendMessage`.

- [ ] **Step 1: Add `conflicts` to `MemoryState` and the new `MemoryUi` struct**

Find `MemoryState` (`crates/aivyx-web/src/main.rs:243`):

```rust
struct MemoryState {
    topics: Vec<String>,
    entries: Vec<MemoryEntrySummary>,
    /// True when a semantic search was transparently served by the keyword path.
    fell_back: bool,
    /// MG — the knowledge-graph nodes (topics + entry counts).
    graph_nodes: Vec<MemoryGraphNode>,
    /// MG — the weighted co-occurrence edges (empty ⇒ a topic cloud).
    graph_edges: Vec<PairScore>,
    /// `false` until the first entries snapshot arrives — distinguishes "still
    /// loading" from "genuinely no memories yet" so the panel shows a skeleton.
    loaded: bool,
}
```

Replace with:

```rust
struct MemoryState {
    topics: Vec<String>,
    entries: Vec<MemoryEntrySummary>,
    /// True when a semantic search was transparently served by the keyword path.
    fell_back: bool,
    /// MG — the knowledge-graph nodes (topics + entry counts).
    graph_nodes: Vec<MemoryGraphNode>,
    /// MG — the weighted co-occurrence edges (empty ⇒ a topic cloud).
    graph_edges: Vec<PairScore>,
    /// `false` until the first entries snapshot arrives — distinguishes "still
    /// loading" from "genuinely no memories yet" so the panel shows a skeleton.
    loaded: bool,
    /// POLISH_WAVES.md sub-project 5, item E — Concord-detected memory
    /// contradictions, refreshed on view-open and after every
    /// resolve/dismiss ack.
    conflicts: Vec<aivyx_ipc::conflict::MemoryConflict>,
}
```

Add a new struct near `SkillsUi`'s own definition (search for `struct SkillsUi` to find the right spot):

```rust
/// POLISH_WAVES.md sub-project 5, item E — Memory screen UI state
/// (resolve/dismiss action feedback), mirroring `SkillsUi`/`SchedulesUi`'s
/// own minimal shape exactly.
#[derive(Clone, Default, PartialEq)]
struct MemoryUi {
    notice: Option<(bool, String)>,
}
```

- [ ] **Step 2: Register `MemoryUi` in `App`**

Find (search for `let skills_ui = use_signal(SkillsUi::default);`):

```rust
    let skills_ui = use_signal(SkillsUi::default);
```

Add a new line right after it:

```rust
    let skills_ui = use_signal(SkillsUi::default);
    let memory_ui = use_signal(MemoryUi::default);
```

Find (search for `use_context_provider(|| skills_ui);`):

```rust
    use_context_provider(|| skills_ui);
```

Add a new line right after it:

```rust
    use_context_provider(|| skills_ui);
    use_context_provider(|| memory_ui);
```

- [ ] **Step 3: Fetch `memory_ui` from context in `MemoryPanel`, add the query builder, and boot-fetch**

Find `MemoryPanel`'s opening lines:

```rust
fn MemoryPanel() -> Element {
    let ws = use_context::<Sender>();
    let memory = use_context::<Signal<MemoryState>>();
```

Replace with:

```rust
fn MemoryPanel() -> Element {
    let ws = use_context::<Sender>();
    let memory = use_context::<Signal<MemoryState>>();
    let memory_ui = use_context::<Signal<MemoryUi>>();
```

Find `mem_graph_query` (`crates/aivyx-web/src/main.rs`, search for `fn mem_graph_query`):

```rust
fn mem_graph_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-mem-graph".to_string(),
        payload: QueryPayload::GetMemoryGraph { limit: 60 },
    }
}
```

Add a new function right after it:

```rust
fn mem_conflicts_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-mem-conflicts".to_string(),
        payload: QueryPayload::GetMemoryConflicts,
    }
}
```

Find `MemoryPanel`'s boot `use_future` (search for `ws.send(mem_graph_query());`):

```rust
    use_future(move || async move {
        ws.send(mem_topics_query());
        ws.send(mem_search_query(String::new(), false));
        ws.send(mem_graph_query());
    });
```

Replace with:

```rust
    use_future(move || async move {
        ws.send(mem_topics_query());
        ws.send(mem_search_query(String::new(), false));
        ws.send(mem_graph_query());
        ws.send(mem_conflicts_query());
    });
```

- [ ] **Step 4: Add the response handlers**

Find (search for `payload: QueryResponsePayload::GetMemoryTopicEntries { entries },` — the arm immediately follows `ListMemoryTopics`/`GetMemoryGraph`):

```rust
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::GetMemoryTopicEntries { entries },
                    ..
                } => {
                    let mut m = memory.write();
                    m.entries = entries;
                    m.fell_back = false;
                    m.loaded = true;
                }
```

Add a new arm right after `SearchMemory`'s arm (search for `payload: QueryResponsePayload::SearchMemory { matches, fell_back_to_keyword },` to find the exact spot, and insert after its closing `}`):

```rust
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::MemoryConflicts { conflicts },
                    ..
                } => {
                    memory.write().conflicts = conflicts;
                }
                DaemonEnvelope::MemoryConflictResolved { ok, removed, error, .. } => {
                    let msg = if ok {
                        if removed {
                            "Conflict resolved.".to_string()
                        } else {
                            "That entry was already gone.".to_string()
                        }
                    } else {
                        error.unwrap_or_else(|| "Resolve failed.".to_string())
                    };
                    memory_ui.write().notice = Some((ok, msg));
                    if ok {
                        ws.send(mem_conflicts_query());
                    }
                }
                DaemonEnvelope::MemoryConflictDismissed { ok, error, .. } => {
                    let msg = if ok {
                        "Dismissed — kept both.".to_string()
                    } else {
                        error.unwrap_or_else(|| "Dismiss failed.".to_string())
                    };
                    memory_ui.write().notice = Some((ok, msg));
                    if ok {
                        ws.send(mem_conflicts_query());
                    }
                }
```

This response-dispatch match lives inside `read_task` (spawned by `ws_task`), which already receives `ws: Sender` (used above for the re-fetch send) and `memory: Signal<MemoryState>` (already threaded — confirmed by its existing use in the arms right above this one). It needs `memory_ui: Signal<MemoryUi>` threaded in too, at 3 sites — mirror exactly how `skills_ui` is already threaded through all 3:

1. `App`'s own `use_coroutine` call (search for `let ws: Sender = use_coroutine(move |rx| {`):

```rust
    let ws: Sender = use_coroutine(move |rx| {
        ws_task(
            rx, missions, running_overlay, dashboard, memory, wiki, lattice, settings, agents,
            teams, documents, voice, skills, skills_ui, mcp, tools, gallery, schedules_ui,
            notifications, audit_page, sessions_page, connected, session, transcript, streaming,
            gate, mission_ui,
        )
    });
```

Replace with (adds `memory_ui` right after `memory`, matching how `skills_ui` sits right after `skills`):

```rust
    let ws: Sender = use_coroutine(move |rx| {
        ws_task(
            rx, missions, running_overlay, dashboard, memory, memory_ui, wiki, lattice, settings, agents,
            teams, documents, voice, skills, skills_ui, mcp, tools, gallery, schedules_ui,
            notifications, audit_page, sessions_page, connected, session, transcript, streaming,
            gate, mission_ui,
        )
    });
```

Also add `use_context_provider(|| memory_ui);` right after the existing `use_context_provider(|| memory);` line.

2. `ws_task`'s own signature (search for `async fn ws_task(`):

```rust
async fn ws_task(
    mut rx: UnboundedReceiver<FrontendMessage>,
    missions: Signal<Vec<TeamMissionView>>,
    running_overlay: Signal<HashMap<String, HashSet<usize>>>,
    dashboard: Signal<Dashboard>,
    memory: Signal<MemoryState>,
    wiki: Signal<WikiState>,
```

Replace with (add `memory_ui` right after `memory`):

```rust
async fn ws_task(
    mut rx: UnboundedReceiver<FrontendMessage>,
    missions: Signal<Vec<TeamMissionView>>,
    running_overlay: Signal<HashMap<String, HashSet<usize>>>,
    dashboard: Signal<Dashboard>,
    memory: Signal<MemoryState>,
    memory_ui: Signal<MemoryUi>,
    wiki: Signal<WikiState>,
```

And its own `spawn(read_task(...))` call (search for `spawn(read_task(`):

```rust
        spawn(read_task(
            read, missions, running_overlay, dashboard, memory, wiki, lattice, settings, agents,
            teams, documents, voice, skills, skills_ui, mcp, tools, gallery, schedules_ui,
            notifications, audit_page, sessions_page, connected, session, transcript, streaming,
            gate, mission_ui,
        ));
```

Replace with:

```rust
        spawn(read_task(
            read, missions, running_overlay, dashboard, memory, memory_ui, wiki, lattice, settings, agents,
            teams, documents, voice, skills, skills_ui, mcp, tools, gallery, schedules_ui,
            notifications, audit_page, sessions_page, connected, session, transcript, streaming,
            gate, mission_ui,
        ));
```

3. `read_task`'s own signature (search for `async fn read_task(`):

```rust
async fn read_task(
    mut read: futures_util::stream::SplitStream<WebSocket>,
    mut missions: Signal<Vec<TeamMissionView>>,
    mut running_overlay: Signal<HashMap<String, HashSet<usize>>>,
    mut dashboard: Signal<Dashboard>,
    mut memory: Signal<MemoryState>,
    mut wiki: Signal<WikiState>,
```

Replace with (`mut`, matching every other signal in this signature — note this is `mut` here even though it was plain in the `App` call site and `ws_task`'s own signature, exactly like `skills_ui`'s own non-uniform mut-qualifier pattern across these same 3 sites):

```rust
async fn read_task(
    mut read: futures_util::stream::SplitStream<WebSocket>,
    mut missions: Signal<Vec<TeamMissionView>>,
    mut running_overlay: Signal<HashMap<String, HashSet<usize>>>,
    mut dashboard: Signal<Dashboard>,
    mut memory: Signal<MemoryState>,
    mut memory_ui: Signal<MemoryUi>,
    mut wiki: Signal<WikiState>,
```

- [ ] **Step 5: Add the badge and the resolve/dismiss panel**

Find the topic-rail loop in `MemoryPanel` (search for `for t in m.topics.iter() {`):

```rust
                for t in m.topics.iter() {
                    {
                        let topic = t.clone();
                        let label = t.clone();
                        let sel = scope() == format!("topic:{t}");
                        rsx! {
                            button {
                                class: if sel { "mem-topic active" } else { "mem-topic" },
                                onclick: move |_| {
                                    scope.set(format!("topic:{topic}"));
                                    ws.send(mem_topic_query(topic.clone()));
                                },
                                "{label}"
                            }
                        }
                    }
                }
```

Replace with (badges a topic button when any conflict names it on either side):

```rust
                for t in m.topics.iter() {
                    {
                        let topic = t.clone();
                        let label = t.clone();
                        let sel = scope() == format!("topic:{t}");
                        let conflicted = m.conflicts.iter().any(|c| c.a.topic == topic || c.b.topic == topic);
                        rsx! {
                            button {
                                class: if sel { "mem-topic active" } else { "mem-topic" },
                                onclick: move |_| {
                                    scope.set(format!("topic:{topic}"));
                                    ws.send(mem_topic_query(topic.clone()));
                                },
                                "{label}"
                                if conflicted {
                                    span { class: "chip amber", title: "contradictory entries", " ⚠" }
                                }
                            }
                        }
                    }
                }
```

Add a new `ConflictsPanel` component (place it near `MemoryPanel`, e.g. directly after it):

```rust
/// POLISH_WAVES.md sub-project 5, item E — the currently-selected topic's
/// open conflicts (if any), with resolve/dismiss actions matching the
/// CLI's own `aivyx memory conflicts` semantics exactly: "keep this one"
/// deletes the OTHER side (`ResolveMemoryConflict` names the loser's own
/// `topic`/`seq` as `archive_seq`); "not a conflict" dismisses the pair as
/// a false positive without deleting anything.
#[component]
fn ConflictsPanel(topic: String, conflicts: Vec<aivyx_ipc::conflict::MemoryConflict>) -> Element {
    let ws = use_context::<Sender>();
    let relevant: Vec<_> = conflicts
        .into_iter()
        .filter(|c| c.a.topic == topic || c.b.topic == topic)
        .collect();
    if relevant.is_empty() {
        return rsx! { Fragment {} };
    }
    rsx! {
        div { class: "conflicts",
            for c in relevant.iter() {
                {
                    let conflict_id = c.id.clone();
                    let a = c.a.clone();
                    let b = c.b.clone();
                    let (keep_a_topic, keep_a_seq) = (b.topic.clone(), b.seq);
                    let (keep_b_topic, keep_b_seq) = (a.topic.clone(), a.seq);
                    let dismiss_id = conflict_id.clone();
                    rsx! {
                        div { class: "glass-card conflict", key: "{conflict_id}",
                            p { class: "notice err", "{c.reason}" }
                            div { class: "conflict-side", span { class: "label-tech", "{a.topic} #{a.seq}" } p { "{a.body}" } }
                            div { class: "conflict-side", span { class: "label-tech", "{b.topic} #{b.seq}" } p { "{b.body}" } }
                            div { class: "conflict-actions",
                                button {
                                    class: "btn btn-sage btn-xs",
                                    onclick: move |_| ws.send(resolve_memory_conflict_query(keep_a_topic.clone(), keep_a_seq)),
                                    "Keep \"{a.topic} #{a.seq}\""
                                }
                                button {
                                    class: "btn btn-sage btn-xs",
                                    onclick: move |_| ws.send(resolve_memory_conflict_query(keep_b_topic.clone(), keep_b_seq)),
                                    "Keep \"{b.topic} #{b.seq}\""
                                }
                                button {
                                    class: "btn btn-ghost btn-xs",
                                    onclick: move |_| ws.send(dismiss_memory_conflict_query(dismiss_id.clone())),
                                    "Not a conflict"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn resolve_memory_conflict_query(topic: String, archive_seq: u64) -> FrontendMessage {
    FrontendMessage::ResolveMemoryConflict {
        id: "mc-mem-conflict-resolve".to_string(),
        topic,
        archive_seq,
    }
}

fn dismiss_memory_conflict_query(conflict_id: String) -> FrontendMessage {
    FrontendMessage::DismissMemoryConflict {
        id: "mc-mem-conflict-dismiss".to_string(),
        conflict_id,
    }
}
```

("Keep `{a.topic} #{a.seq}`" deletes the *other* side — `keep_a_topic`/`keep_a_seq` are bound to `b`'s own `topic`/`seq` deliberately, since choosing to keep A means archiving B; re-read this mapping carefully when implementing, it is easy to invert.)

Wire `ConflictsPanel` into `MemoryPanel`'s main content area, rendered only when a specific topic is selected. Find the end of the list/graph toggle block (search for `div { class: "mem-entries",` — it's the last arm of an `if graph_view() { ... } else if !m.loaded { ... } else if m.entries.is_empty() { ... } else { ... }` chain):

```rust
                } else {
                    div { class: "mem-entries",
                        for e in m.entries.iter() {
                            MemoryEntry { entry: e.clone() }
                        }
                    }
                }
            }
        }
    }
}
```

Replace with (adds the conflicts panel + notice line right after that `if`/`else if` chain closes, still inside the same `div { class: "mem-main", ... }`):

```rust
                } else {
                    div { class: "mem-entries",
                        for e in m.entries.iter() {
                            MemoryEntry { entry: e.clone() }
                        }
                    }
                }
                if let Some(current_topic) = scope().strip_prefix("topic:").map(|s| s.to_string()) {
                    ConflictsPanel { topic: current_topic, conflicts: m.conflicts.clone() }
                }
                if let Some((ok, msg)) = memory_ui().notice.clone() {
                    div { class: if ok { "notice ok" } else { "notice err" }, "{msg}" }
                }
            }
        }
    }
}
```

- [ ] **Step 6: Compile-check and clippy (native, no wasm32 target needed)**

Run: `cargo check -p aivyx-web && cargo clippy -p aivyx-web --all-targets -- -D warnings`
Expected: clean. Fix any signature-threading mismatches found in Step 4/5 (e.g. a missed `memory_ui` parameter on `ws_task`/`read_task`) before proceeding — this task's correctness depends on that threading compiling clean, not on a test asserting it (Dioxus component wiring like this is exercised the same way sub-project 3's `SkillsUi` threading was: compile-checked, not unit-tested).

- [ ] **Step 7: Rebuild the wasm bundle**

Same recipe as Task 1 Step 3 (rustup toolchain on `PATH`, `dx bundle --release --platform web`, `rm -rf` + `cp -r` replace into `dist/`, delete `*.br`, verify zero `.br` tracked, verify exactly one new `.wasm`).

- [ ] **Step 8: Commit**

```bash
git add crates/aivyx-web/src/main.rs crates/aivyx-web/dist/
git commit -m "feat(web): badge contradictory memory topics, wire resolve/dismiss

POLISH_WAVES.md sub-project 5, item E. Concord already detects memory
contradictions and the full resolve/dismiss backend (GetMemoryConflicts/
ResolveMemoryConflict/DismissMemoryConflict) already exists and is
already tested — the CLI's 'aivyx memory conflicts' already drives it.
Studio had zero wiring to any of it. Adds a badge on conflicted topics
in the rail and a per-conflict resolve ('keep this one')/dismiss ('not
a conflict') panel matching the CLI's own semantics exactly.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Final Verification

After all 5 tasks:

```bash
cargo clippy --workspace --exclude aivyx-desktop --all-targets -- -D warnings
cargo test --workspace --exclude aivyx-desktop
```

Expected: zero warnings, zero failures.
