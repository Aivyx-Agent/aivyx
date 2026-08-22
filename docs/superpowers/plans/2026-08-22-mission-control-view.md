# Mission Control — Piece 3: The Nav View Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** a dedicated nav destination in `aivyx-web` showing one active
mission's live LEAD/specialist graph — who's doing what right now — with
drill-in detail and the operator's first-ever UI affordances for abort,
plus the new pause/resume from Piece 2.

**Architecture:** a pure, `Signal`/Dioxus-free transform function
(`TeamMissionView` + `TeamConfig` → a node/edge graph data structure) feeds
a Dioxus component that renders it and re-renders live as the existing
`missions`/`running_overlay` signals (Piece 1) update — no new live-data
path. Reuses `TeamsPanel`'s exact NT-02 capability logic for drill-in, and
the existing `resolve_team_query`/`GateControls` pattern for gate
approve/reject; abort and pause/resume are genuinely new query senders
(neither has ever had a UI affordance before this plan).

**Tech Stack:** Rust, Dioxus 0.6 (rsx!), the existing Stitch CSS token
classes (`glass-card`, `chip`, `btn btn-sage`, `btn btn-ghost-danger`,
`label-tech`, etc. — no new CSS framework, no new dependency).

## Global Constraints

- **A design-doc correction, found during this plan's own verification**:
  the approved design doc's Piece 3 section claimed graph edges could
  reflect "the mission plan's DAG dependencies (`MissionPlan.steps`,
  already available via `TeamMissionView.steps`)" — this was **wrong**.
  `TeamStepView` (`crates/aivyx-ipc/src/team_mission.rs`) only carries
  `label: String, state: TeamStepState` today; a client cannot recover
  step dependencies, the specialist's name, or the step kind (delegate vs.
  gate) without parsing the formatted `label` string. Task 1 of this plan
  fixes the actual gap (adds real structured fields) rather than the
  narrower rendering layer working around it with string parsing.
- v1 shows **one active mission at a time**, with a selector if more than
  one is active — no simultaneous multi-mission graph (per the design
  doc's own explicit scope boundary).
- "Active" for this view's own selector means a mission genuinely worth
  watching or acting on: `Executing`, `AwaitingApproval`, or `Paused`.
  This is a **narrower** filter than the existing Command Center's own
  "Active" stat (which also counts `Halted`, since it's a general
  dashboard metric, not scoped to this feature) — a deliberate choice,
  not a reuse of that filter, since a `Halted` mission has nothing left to
  watch or resume.
- No *functional* changes to `aivyx-desktop` or `aivyx-tui` — this plan's
  real work is entirely within `aivyx-web` (plus the one small, additive
  `aivyx-ipc` wire-type enrichment in Task 1). Task 1's own execution
  found this needs one caveat: `TeamStepView` gaining 4 required fields
  breaks 8 pre-existing `TeamStepView { .. }` test-fixture literals in
  `aivyx-tui/src/model.rs`'s and this crate's own `#[cfg(test)]` modules
  (Rust requires every field named in a struct literal) — those got
  mechanical placeholder values, zero behavior change, purely to keep
  `cargo test --workspace` compiling. `aivyx-desktop` has zero references
  to `TeamStepView` (confirmed), so it needs no such fix.
- No true mid-step interruption — abort/pause both stop at the next wave
  boundary (already true of the underlying mechanism; this plan only adds
  a UI surface for it, not new behavior).
- Every pure, testable piece of logic gets a real test on the native
  target (confirmed genuinely viable — `cargo test -p aivyx-web` compiles
  and runs natively on this host despite the crate's own `wasm32`-only
  description, verified empirically during Piece 1). Rendering-only code
  (rsx! components) is not unit-tested — verified by a real
  `wasm32-unknown-unknown` build instead, matching Pieces 1/2's own
  precedent.

---

## Task 1: Enrich `TeamStepView` with real structured fields

**Files:**
- Modify: `crates/aivyx-ipc/src/team_mission.rs`

**Interfaces:**
- Produces: `TeamStepView` gains `step_id: String`, `member: String`, `kind: String` (`"delegate"` or `"gate"` — `String`, not `&'static str`: the latter fails to compile through this struct's `Deserialize` derive, a real constraint found during Task 1's own execution, not a style choice), `deps: Vec<String>` — all new fields, `label`/`state` unchanged (backward compatible with `aivyx-tui`'s existing renderer, which only reads `label`/`state` today).

**Verified**: `to_view_with_running`'s existing step-building closure
already computes `(kind, member)` locally, right before formatting
`label` — read below, copied verbatim from the current file. `Step` (in
`aivyx-team-types`) already has a real `deps: Vec<String>` field (the
actual struct field name — `.after([...])` is just a builder method that
sets it).

- [ ] **Step 1: Write the failing tests**

Find `to_view_with_running_marks_the_given_step_running` and
`to_view_derives_step_states_and_progress`-style tests (or whichever
exists) in this file's `mod tests`, and add:

```rust
    #[test]
    fn to_view_exposes_structured_step_fields_not_just_the_label() {
        let rec = sample("v1");
        let view = rec.to_view();
        // sample()'s fixture: count (delegate, inventory) -> approve
        // (human gate, manager) -> order (delegate, purchasing).
        assert_eq!(view.steps[0].step_id, "count");
        assert_eq!(view.steps[0].member, "inventory");
        assert_eq!(view.steps[0].kind, "delegate");
        assert_eq!(view.steps[0].deps, Vec::<String>::new(), "count has no deps");

        assert_eq!(view.steps[1].step_id, "approve");
        assert_eq!(view.steps[1].member, "manager");
        assert_eq!(view.steps[1].kind, "gate");
        assert_eq!(view.steps[1].deps, vec!["count".to_string()]);

        assert_eq!(view.steps[2].step_id, "order");
        assert_eq!(view.steps[2].member, "purchasing");
        assert_eq!(view.steps[2].kind, "delegate");
        assert_eq!(view.steps[2].deps, vec!["approve".to_string()]);

        // label is UNCHANGED -- existing aivyx-tui rendering still works.
        assert!(view.steps[1].label.contains("manager (gate)"));
    }
```

(Confirm `sample()`'s exact fixture — it should already build exactly this
`count → [human gate approve] → order` chain with `.after([...])` calls
setting the dependencies shown above; read it first rather than assuming,
since this plan was written from a snapshot.)

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p aivyx-ipc to_view_exposes_structured_step_fields -- --test-threads=1`
Expected: FAIL to compile — `TeamStepView` has no `step_id`/`member`/`kind`/`deps` fields yet.

- [ ] **Step 3: Add the fields and populate them**

Find `TeamStepView`:

```rust
/// One step in a [`TeamMissionView`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamStepView {
    /// e.g. `"approve — reviewer (gate)"`.
    pub label: String,
    pub state: TeamStepState,
}
```

Change to:

```rust
/// One step in a [`TeamMissionView`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeamStepView {
    /// e.g. `"approve — reviewer (gate)"`.
    pub label: String,
    pub state: TeamStepState,
    /// Chapter Mission Control — the step's own id (`MissionPlan`'s
    /// `Step::id`). A client that only had `label` before this had to
    /// parse a formatted display string to recover this — now it's a
    /// real field.
    pub step_id: String,
    /// Chapter Mission Control — the specialist (for a `delegate` step) or
    /// reviewer (for a `gate` step) this step runs on.
    pub member: String,
    /// Chapter Mission Control — `"delegate"` or `"gate"`, matching the
    /// two `StepKind` variants. `String`, not `&'static str`: this struct
    /// derives `Deserialize` and reaches the caller through
    /// `TeamMissionView`'s own generic `Deserialize<'de>` impl (via
    /// `Vec<TeamStepView>`), which isn't parameterized by a fixed
    /// lifetime — a `&'static str` field there requires `'de: 'static`,
    /// which can't be discharged, and fails to compile
    /// ("lifetime may not live long enough ... requires that `'de` must
    /// outlive `'static`"). No new public enum either, since these are
    /// the only two values `StepKind` has.
    pub kind: String,
    /// Chapter Mission Control — the ids of steps that must complete
    /// before this one is ready (`Step::deps`, unchanged from the engine
    /// type) — lets a client draw real dependency edges without touching
    /// `MissionPlan`/`StepKind` directly.
    #[serde(default)]
    pub deps: Vec<String>,
}
```

Find `to_view_with_running`'s step-building closure:

```rust
        let steps: Vec<TeamStepView> = self
            .plan
            .steps
            .iter()
            .map(|step| {
                let (kind, member) = match &step.kind {
                    StepKind::Delegate { specialist, .. } => ("delegate", specialist.as_str()),
                    StepKind::Gate { reviewer, .. } => ("gate", reviewer.as_str()),
                };
                TeamStepView {
                    label: format!("{} — {member} ({kind})", step.id),
                    state: self.step_state(&step.id, running_steps),
                }
            })
            .collect();
```

Change the `TeamStepView` construction to populate the new fields:

```rust
        let steps: Vec<TeamStepView> = self
            .plan
            .steps
            .iter()
            .map(|step| {
                let (kind, member) = match &step.kind {
                    StepKind::Delegate { specialist, .. } => ("delegate", specialist.as_str()),
                    StepKind::Gate { reviewer, .. } => ("gate", reviewer.as_str()),
                };
                TeamStepView {
                    label: format!("{} — {member} ({kind})", step.id),
                    state: self.step_state(&step.id, running_steps),
                    step_id: step.id.clone(),
                    member: member.to_string(),
                    kind: kind.to_string(),
                    deps: step.deps.clone(),
                }
            })
            .collect();
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p aivyx-ipc -- --test-threads=1`
Expected: all pass, including the new test and every pre-existing one in
this file (the new fields are purely additive to a struct literal built
in one place, so no other construction site breaks).

- [ ] **Step 5: Confirm no other crate needed changes**

Run: `cargo build --workspace --exclude aivyx-desktop --exclude aivyx-web`
Expected: clean. `TeamStepView` is only ever *constructed* in this one
function; every other crate (`aivyx-tui`, `aivyx-web`, `aivyx-cli`) only
*reads* `label`/`state` off an already-built one, so adding fields breaks
nothing (Rust struct field addition doesn't affect field-access call
sites, only construction sites — and this is the only construction site
in the workspace, confirmed via `grep -rn "TeamStepView {" crates/` before
writing this task).

Also run: `cargo build -p aivyx-web --target wasm32-unknown-unknown` (the
isolated toolchain at `/tmp/claude-1000/{rustup-home,cargo-home}` from
earlier pieces should still be present — check there first before
re-bootstrapping).

- [ ] **Step 6: Commit**

```bash
git add crates/aivyx-ipc/src/team_mission.rs
git commit -m "feat: add structured step_id/member/kind/deps to TeamStepView

label stays exactly as before (aivyx-tui's existing renderer keeps
working unchanged) -- these are new, additive fields so a client building
a real graph (Piece 3) doesn't need to parse the formatted label string
to recover a step's specialist, kind, or dependencies. Corrects a real
inaccuracy in the approved design doc's own Piece 3 text, which claimed
this data was 'already available via TeamMissionView.steps' before this
task actually added it."
```

---

## Task 2: `View::MissionControl` nav scaffold + active-mission selector

**Files:**
- Modify: `crates/aivyx-web/src/main.rs`

**Interfaces:**
- Consumes: `TeamMissionView` (Task 1's enriched shape, and Piece 1's already-flowing `missions: Signal<Vec<TeamMissionView>>` context — no new fetch needed for the mission list itself).
- Produces: `View::MissionControl` (new enum variant). `fn watchable_missions(missions: &[TeamMissionView]) -> Vec<&TeamMissionView>` (pure helper — the "Active" filter this plan's own Global Constraint defines). A new `selected_mission: Signal<Option<String>>` context (the currently-picked mission id for this view, `None` ⇒ show the selector).

**Verified**: `View`'s enum/`ALL`/`slug`/`label` shape, the `Sidebar`'s
data-driven `NavEntry`/`NavGroup` table, and the main routing `match
view() { ... }` block are all read in full above (this plan's own
research) — mirror their exact existing conventions.

- [ ] **Step 1: Write the failing test for the pure filter**

Add near `upsert_mission_view`'s own test module (`mod mission_control_tests`,
already established by Piece 1):

```rust
    #[test]
    fn watchable_missions_excludes_terminal_and_halted() {
        let mut done = view("m1", 100);
        done.phase = TeamMissionPhase::Done;
        let mut rejected = view("m2", 0);
        rejected.phase = TeamMissionPhase::Rejected;
        let mut halted = view("m3", 50);
        halted.phase = TeamMissionPhase::Halted;
        let mut executing = view("m4", 20);
        executing.phase = TeamMissionPhase::Executing;
        let mut paused = view("m5", 60);
        paused.phase = TeamMissionPhase::Paused;
        let mut awaiting = view("m6", 40);
        awaiting.phase = TeamMissionPhase::AwaitingApproval;

        let all = vec![done, rejected, halted, executing.clone(), paused.clone(), awaiting.clone()];
        let watchable = watchable_missions(&all);
        let ids: Vec<&str> = watchable.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["m4", "m5", "m6"], "only Executing/Paused/AwaitingApproval, in original order");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p aivyx-web watchable_missions_excludes -- --test-threads=1`
Expected: FAIL to compile — `watchable_missions` doesn't exist yet.

- [ ] **Step 3: Add `watchable_missions`**

Add near `upsert_mission_view`:

```rust
/// Chapter Mission Control — which missions this view's selector offers:
/// genuinely worth watching or acting on right now. Narrower than the
/// Command Center's own "Active" stat (which also counts `Halted`, since
/// that's a general dashboard metric) — a `Halted` mission has nothing
/// left to watch or resume, so it's excluded here.
fn watchable_missions(missions: &[TeamMissionView]) -> Vec<&TeamMissionView> {
    missions
        .iter()
        .filter(|m| {
            matches!(
                m.phase,
                TeamMissionPhase::Executing
                    | TeamMissionPhase::Paused
                    | TeamMissionPhase::AwaitingApproval
            )
        })
        .collect()
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p aivyx-web watchable_missions_excludes -- --test-threads=1`
Expected: PASS.

- [ ] **Step 5: Add the `View::MissionControl` variant**

Find `View`'s enum declaration (add right after `View::Missions`, since
Mission Control is a deeper view of the same domain):

```rust
enum View {
    Command,
    Missions,
    /// Chapter Mission Control — a live view of ONE active mission's
    /// LEAD/specialist graph, with drill-in and controls (approve/reject,
    /// abort, pause/resume). Distinct from `Missions` (a flat list/history
    /// + "start a new mission" bar) — this is the deep-dive, one-mission-
    /// at-a-time surface.
    MissionControl,
    Schedules,
    // ... (rest unchanged)
```

Update `View::ALL`'s array (bump the size from `19` to `20`, add
`View::MissionControl` right after `View::Missions` in the array literal
too, matching the enum's own declared order):

```rust
    const ALL: [View; 20] = [
        View::Command,
        View::Chat,
        View::Missions,
        View::MissionControl,
        View::Schedules,
        // ... (rest unchanged)
    ];
```

Add to `slug`:
```rust
            View::MissionControl => "mission-control",
```

Add to `label`:
```rust
            View::MissionControl => "Mission Control",
```

- [ ] **Step 6: Add the sidebar nav item**

Find the `Sidebar` component's `groups` table — the `"Workspace"` group
already has Chat/Missions/Schedules. Add Mission Control right after
Missions, reusing the existing `ICON_MISSIONS` asset (no new icon file —
checked `crates/aivyx-web/assets/icons/`, nothing more specific exists,
and inventing new SVG content in a text plan risks a broken/ugly asset;
reusing the closest existing icon is the established, lower-risk choice):

```rust
        (
            "Workspace",
            vec![
                (ICON_CHAT, "Chat", View::Chat),
                (ICON_MISSIONS, "Missions", View::Missions),
                (ICON_MISSIONS, "Mission Control", View::MissionControl),
                (ICON_SCHEDULES, "Schedules", View::Schedules),
            ],
        ),
```

- [ ] **Step 7: Add the `selected_mission` context and a minimal routed panel**

Verified real pattern (this file's app-shell component declares a plain
`use_signal` binding, then registers it into context with a *separate*
`use_context_provider(|| the_binding)` call later in the same function —
`use_context_provider`'s own return value is never bound; the local
`use_signal` binding is what's used both directly and via context).
`selected_mission` is pure UI-navigation state — unlike `missions`/
`running_overlay`, it's never written by `ws_task`'s coroutine, so it must
**not** be threaded into `ws_task`'s parameter list; only `missions`/
`running_overlay` need that.

Find where the simple, non-`ws_task`-threaded signals are declared
(`session`/`transcript`/`gate` are the nearest precedent — search for
`let gate = use_signal(|| None::<GateInfo>);`) and add right after:

```rust
    let selected_mission = use_signal(|| None::<String>);
```

Find where those same signals get their own `use_context_provider` call
later in the function (search for `use_context_provider(|| gate);` or
wherever the block of provider calls ends) and add:

```rust
    use_context_provider(|| selected_mission);
```

Find the main routing `match view() { ... }` block and add, right after
the `View::Missions` arm:

```rust
                        View::MissionControl => rsx! {
                            MissionControlPanel { missions: missions(), selected_mission }
                        },
```

Add a minimal `MissionControlPanel` component — just the selector for
this task; the real graph is Task 4:

```rust
#[component]
fn MissionControlPanel(missions: Vec<TeamMissionView>, selected_mission: Signal<Option<String>>) -> Element {
    let watchable = watchable_missions(&missions);
    let Some(current_id) = selected_mission() else {
        return rsx! {
            div { class: "mission-control",
                div { class: "panel-head", h3 { "Mission Control" } }
                if watchable.is_empty() {
                    div { class: "empty card",
                        p { "No mission is currently executing, paused, or awaiting approval." }
                        p { class: "label-tech", "Start one from the Missions screen." }
                    }
                } else {
                    div { class: "mission-picker",
                        for m in watchable.iter() {
                            button {
                                class: "glass-card mission-pick",
                                key: "{m.id}",
                                onclick: {
                                    let id = m.id.clone();
                                    move |_| selected_mission.set(Some(id.clone()))
                                },
                                span { class: "chip {phase_class(m.phase)}", "{phase_label(m.phase)}" }
                                span { class: "goal", "{m.goal}" }
                                span { class: "lead label-tech", "{m.lead}" }
                            }
                        }
                    }
                }
            }
        };
    };
    // If the selected mission is no longer watchable (finished/halted
    // while this view was open), fall back to the selector rather than
    // showing a stale/missing graph.
    let Some(current) = watchable.iter().find(|m| m.id == current_id) else {
        selected_mission.set(None);
        return rsx! { div { class: "mission-control", "…" } };
    };
    rsx! {
        div { class: "mission-control",
            div { class: "panel-head",
                h3 { "Mission Control" }
                button { class: "btn btn-ghost", onclick: move |_| selected_mission.set(None), "← All missions" }
            }
            div { class: "glass-card", "{current.goal}" }
        }
    }
}
```

(The last `div` is a deliberate placeholder for the real graph, replaced
entirely by Task 4 — this task's own scope is routing + selection only.
This is NOT a "TBD" placeholder in the forbidden sense: it's working,
correct code for this task's own deliverable, just visually minimal.)

- [ ] **Step 8: Run the tests and build**

Run: `cargo test -p aivyx-web mission_control_tests -- --test-threads=1`
Expected: all pass, including the new `watchable_missions` test.

Run: `cargo build -p aivyx-web --target wasm32-unknown-unknown`
Expected: clean.

- [ ] **Step 9: Commit**

```bash
git add crates/aivyx-web/src/main.rs
git commit -m "feat: add the Mission Control nav view (routing + mission selector)

View::MissionControl, a sidebar entry (reusing the Missions icon -- no
new asset), and MissionControlPanel with a watchable_missions filter
(Executing/Paused/AwaitingApproval -- narrower than the Command Center's
own Active stat, which also counts Halted). The real graph replaces this
task's placeholder body in Task 4."
```

---

## Task 3: The pure graph-transform function + its tests

**Files:**
- Modify: `crates/aivyx-web/src/main.rs`

**Interfaces:**
- Consumes: `TeamMissionView` (Task 1's enriched `TeamStepView`), `TeamConfig`/`TeamMember` (already fetched via `GetTeamRoster`, `TeamsState.roster`).
- Produces: `MissionGraphNode`, `MissionGraphEdge`, `MissionGraph` structs. `fn build_mission_graph(mission: &TeamMissionView, roster: &TeamConfig) -> MissionGraph` — the pure transform Task 4's rendering consumes.

**Verified**: `TeamConfig`/`TeamMember` are already used by `TeamsPanel`
(`team.lead: String`, `team.members: Vec<TeamMember>`, each with
`.name`/`.capability_scopes` — read `TeamsPanel`'s own body for the exact
field access pattern before writing this task's code, since this plan's
own research read it but didn't transcribe every field).

- [ ] **Step 1: Write the failing tests**

```rust
    /// Verified real `TeamMember` shape (`crates/aivyx-team-types/src/config.rs`)
    /// has 8 fields, not just `name`/`capability_scopes` — this literal
    /// matches the exact construction pattern already used elsewhere in
    /// this same file (`TeamsPanel`'s own "add specialist" button, which
    /// pushes a `TeamMember { .. }` literal with all 8 fields).
    fn sample_member(name: &str) -> TeamMember {
        TeamMember {
            name: name.to_string(),
            role: "Specialist".to_string(),
            soul: String::new(),
            tool_allowlist: vec!["team.message".to_string()],
            capability_scopes: vec![],
            trust_ceiling: TrustTier::SemiTrusted,
            model: None,
            base_url: None,
        }
    }

    fn sample_roster() -> TeamConfig {
        TeamConfig {
            lead: "coordinator".to_string(),
            members: vec![
                sample_member("coordinator"),
                sample_member("inventory"),
                sample_member("purchasing"),
            ],
        }
    }

    #[test]
    fn build_mission_graph_has_one_node_per_roster_member_including_idle_ones() {
        let mut m = view("v1", 33);
        m.lead = "coordinator".to_string();
        m.steps = vec![
            TeamStepView { label: "count — inventory (delegate)".into(), state: TeamStepState::Running, step_id: "count".into(), member: "inventory".into(), kind: "delegate".into(), deps: vec![] },
        ];
        let roster = sample_roster();
        let graph = build_mission_graph(&m, &roster);
        // All 3 roster members get a node, even "purchasing" (idle -- no
        // step of theirs has run or is running yet).
        let names: Vec<&str> = graph.nodes.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"coordinator"));
        assert!(names.contains(&"inventory"));
        assert!(names.contains(&"purchasing"));
        let lead_node = graph.nodes.iter().find(|n| n.name == "coordinator").unwrap();
        assert!(lead_node.is_lead);
        let inventory_node = graph.nodes.iter().find(|n| n.name == "inventory").unwrap();
        assert!(!inventory_node.is_lead);
        assert_eq!(inventory_node.state, TeamStepState::Running, "inventory is running the 'count' step");
        let purchasing_node = graph.nodes.iter().find(|n| n.name == "purchasing").unwrap();
        assert_eq!(purchasing_node.state, TeamStepState::Pending, "idle -- no step touches purchasing yet");
    }

    #[test]
    fn build_mission_graph_edges_reflect_step_deps() {
        let mut m = view("v1", 0);
        m.steps = vec![
            TeamStepView { label: "a".into(), state: TeamStepState::Done, step_id: "a".into(), member: "inventory".into(), kind: "delegate".into(), deps: vec![] },
            TeamStepView { label: "b".into(), state: TeamStepState::Pending, step_id: "b".into(), member: "purchasing".into(), kind: "delegate".into(), deps: vec!["a".to_string()] },
        ];
        let roster = sample_roster();
        let graph = build_mission_graph(&m, &roster);
        assert_eq!(graph.edges.len(), 1);
        assert_eq!(graph.edges[0].from_step, "a");
        assert_eq!(graph.edges[0].to_step, "b");
    }

    #[test]
    fn build_mission_graph_a_specialist_with_multiple_steps_shows_the_most_attention_worthy_state() {
        // A specialist who ran one step to Done and has another Pending
        // should show Pending (still work to do), not Done (which would
        // read as "finished" when they aren't).
        let mut m = view("v1", 0);
        m.steps = vec![
            TeamStepView { label: "a".into(), state: TeamStepState::Done, step_id: "a".into(), member: "inventory".into(), kind: "delegate".into(), deps: vec![] },
            TeamStepView { label: "b".into(), state: TeamStepState::Pending, step_id: "b".into(), member: "inventory".into(), kind: "delegate".into(), deps: vec!["a".to_string()] },
        ];
        let roster = sample_roster();
        let graph = build_mission_graph(&m, &roster);
        let inventory_node = graph.nodes.iter().find(|n| n.name == "inventory").unwrap();
        assert_eq!(inventory_node.state, TeamStepState::Pending);
    }
```

(Adapt `view(id, progress)`'s fixture helper call and `TeamMember`'s
literal fields to whatever their real current shapes are — confirmed
`view()` already exists from Piece 1, and `TeamMember`'s fields need a
direct read of its real struct before writing this literal, since this
plan's own research read `TeamsPanel`'s *usage* of `TeamMember` fields
but not its full struct declaration.)

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p aivyx-web build_mission_graph -- --test-threads=1`
Expected: FAIL to compile — none of `MissionGraph`/`build_mission_graph` exist yet.

- [ ] **Step 3: Add the graph types and the transform**

```rust
/// Chapter Mission Control — one node in a mission's live graph: the LEAD
/// or a specialist, with the "worst" (most attention-worthy) state across
/// every step of theirs in this mission.
#[derive(Debug, Clone, PartialEq)]
struct MissionGraphNode {
    name: String,
    is_lead: bool,
    state: TeamStepState,
    /// The step id this node is currently `Running`, if any -- for the
    /// drill-in panel (Task 5) to show "doing: <step>".
    current_step: Option<String>,
}

/// Chapter Mission Control — one dependency edge between two steps
/// (`TeamStepView::deps`, Task 1).
#[derive(Debug, Clone, PartialEq)]
struct MissionGraphEdge {
    from_step: String,
    to_step: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
struct MissionGraph {
    nodes: Vec<MissionGraphNode>,
    edges: Vec<MissionGraphEdge>,
}

/// Chapter Mission Control — the pure transform Task 4's rendering
/// consumes: project one mission's live steps onto the team roster's real
/// member list, so every roster member gets a node (including one with no
/// step run yet -- genuinely idle, not merely absent from the DAG), and
/// edges come from each step's real `deps` (Task 1), not from parsing
/// `label`.
fn build_mission_graph(mission: &TeamMissionView, roster: &TeamConfig) -> MissionGraph {
    let nodes = roster
        .members
        .iter()
        .map(|member| {
            let member_steps: Vec<&TeamStepView> = mission
                .steps
                .iter()
                .filter(|s| s.member == member.name)
                .collect();
            let state = step_state_priority(&member_steps);
            let current_step = member_steps
                .iter()
                .find(|s| s.state == TeamStepState::Running)
                .map(|s| s.step_id.clone());
            MissionGraphNode {
                name: member.name.clone(),
                is_lead: member.name == roster.lead,
                state,
                current_step,
            }
        })
        .collect();
    let edges = mission
        .steps
        .iter()
        .flat_map(|step| {
            step.deps
                .iter()
                .map(move |dep| MissionGraphEdge { from_step: dep.clone(), to_step: step.step_id.clone() })
        })
        .collect();
    MissionGraph { nodes, edges }
}

/// Chapter Mission Control — the single most attention-worthy state across
/// a specialist's own steps in this mission: `Running` (something's
/// happening right now) > `Awaiting` (blocked on a decision) > `Pending`
/// (still work to do, even if some of their steps are `Done`) > `Rejected`
/// > `Done` (only if every one of their steps is `Done`) > `Pending`
/// (genuinely idle -- no steps at all). Priority order chosen so a
/// specialist with mixed Done/Pending steps never reads as "finished."
fn step_state_priority(steps: &[&TeamStepView]) -> TeamStepState {
    if steps.is_empty() {
        return TeamStepState::Pending;
    }
    if steps.iter().any(|s| s.state == TeamStepState::Running) {
        return TeamStepState::Running;
    }
    if steps.iter().any(|s| s.state == TeamStepState::Awaiting) {
        return TeamStepState::Awaiting;
    }
    if steps.iter().any(|s| s.state == TeamStepState::Pending) {
        return TeamStepState::Pending;
    }
    if steps.iter().any(|s| s.state == TeamStepState::Rejected) {
        return TeamStepState::Rejected;
    }
    TeamStepState::Done
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p aivyx-web build_mission_graph -- --test-threads=1`
Expected: all 3 new tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/aivyx-web/src/main.rs
git commit -m "feat: pure build_mission_graph transform (TeamMissionView + TeamConfig -> graph)

Every roster member gets a node (including genuinely idle ones with no
step run yet); edges come from Task 1's real step.deps, not label
parsing. step_state_priority resolves a specialist's own worst-in-
attention-terms state across all their steps, so mixed Done/Pending never
reads as finished. No rendering yet -- Task 4."
```

---

## Task 4: The graph rendering component (live-updating)

**Files:**
- Modify: `crates/aivyx-web/src/main.rs`

**Interfaces:**
- Consumes: `build_mission_graph` (Task 3), `MissionControlPanel` (Task 2 — this task replaces its placeholder body), `TeamsState.roster` (fetched the same way `TeamsPanel`/`OnboardingTeamStep` already do).

**Verified**: `TeamsPanel`/`OnboardingTeamStep` both fetch the roster via
`ws.send(get_team_roster_query())` inside a `use_future`/`use_effect` on
mount, reading it back from the shared `teams: Signal<TeamsState>`
context — mirror this exact pattern rather than inventing a new fetch
path. `missions`/`running_overlay` are already live (Piece 1) — no new
subscription needed; `MissionControlPanel` already receives `missions:
Vec<TeamMissionView>` as a prop (Task 2), which Dioxus re-renders from
automatically whenever the parent's `missions()` context signal updates.

- [ ] **Step 1: Fetch the roster on mount**

In `MissionControlPanel` (from Task 2), add roster fetching at the top:

```rust
#[component]
fn MissionControlPanel(missions: Vec<TeamMissionView>, selected_mission: Signal<Option<String>>) -> Element {
    let ws = use_context::<Sender>();
    let teams = use_context::<Signal<TeamsState>>();
    use_effect(move || {
        ws.send(get_team_roster_query());
    });
    let watchable = watchable_missions(&missions);
```

(This mirrors `TeamsPanel`/`OnboardingTeamStep`'s own exact
`use_effect`-on-mount pattern — safe to call even if another view already
triggered the same fetch; `TeamsState.roster` is idempotently overwritten
with the same data.)

- [ ] **Step 2: Replace the placeholder body with the real graph**

Replace the `let Some(current) = ...` branch's trailing placeholder
`div { class: "glass-card", "{current.goal}" }` with the real graph,
guarded on the roster actually being loaded:

```rust
    let Some(current) = watchable.iter().find(|m| m.id == current_id) else {
        selected_mission.set(None);
        return rsx! { div { class: "mission-control", "…" } };
    };
    let Some(roster) = teams().roster else {
        return rsx! {
            div { class: "mission-control",
                div { class: "panel-head", h3 { "Mission Control" } }
                SkeletonList { rows: 3 }
            }
        };
    };
    let graph = build_mission_graph(current, &roster);
    let mut selected_node = use_signal(|| None::<String>);
    rsx! {
        div { class: "mission-control",
            div { class: "panel-head",
                h3 { "Mission Control" }
                button { class: "btn btn-ghost", onclick: move |_| selected_mission.set(None), "← All missions" }
            }
            div { class: "row1",
                span { class: "chip {phase_class(current.phase)}", "{phase_label(current.phase)}" }
                span { class: "goal", "{current.goal}" }
            }
            div { class: "mission-graph",
                div { class: "mission-graph-lead" }
                for node in graph.nodes.iter() {
                    MissionGraphNodeCard {
                        key: "{node.name}",
                        node: node.clone(),
                        selected: selected_node() == Some(node.name.clone()),
                        onclick: {
                            let name = node.name.clone();
                            move |_| selected_node.set(Some(name.clone()))
                        },
                    }
                }
            }
            MissionControls { mission: current.clone() }
            if let Some(name) = selected_node() {
                if let Some(node) = graph.nodes.iter().find(|n| n.name == name) {
                    SpecialistDrillIn { node: node.clone(), roster: roster.clone(), mission: current.clone() }
                }
            }
        }
    }
}

#[component]
fn MissionGraphNodeCard(node: MissionGraphNode, selected: bool, onclick: EventHandler<MouseEvent>) -> Element {
    let state_class = match node.state {
        TeamStepState::Running => "amber",
        TeamStepState::Awaiting => "amber",
        TeamStepState::Rejected => "error",
        TeamStepState::Done => "sage",
        TeamStepState::Pending => "",
    };
    rsx! {
        button {
            class: if selected { "glass-card mission-node selected" } else { "glass-card mission-node" },
            onclick: move |e| onclick.call(e),
            span { class: "chip {state_class}", if node.is_lead { "LEAD" } else { "specialist" } }
            span { class: "goal", "{node.name}" }
            if let Some(step) = &node.current_step {
                span { class: "step label-tech", "running: {step}" }
            }
        }
    }
}
```

(`MissionControls` and `SpecialistDrillIn` are stubbed as empty
placeholders for THIS task — `MissionControls` in Task 6, `SpecialistDrillIn`
in Task 5. Add minimal, real, compiling stub components so this task's
own code compiles and the graph renders standalone:)

```rust
#[component]
fn MissionControls(mission: TeamMissionView) -> Element {
    let _ = mission;
    rsx! { div {} }
}

#[component]
fn SpecialistDrillIn(node: MissionGraphNode, roster: TeamConfig, mission: TeamMissionView) -> Element {
    let _ = (node, roster, mission);
    rsx! { div {} }
}
```

- [ ] **Step 3: Build and manually verify**

Run: `cargo test -p aivyx-web mission_control_tests -- --test-threads=1`
Expected: all pass (this task adds no new pure-function tests of its own
— it's rendering code, verified by build + read, matching this plan's own
Global Constraint on what's testable).

Run: `cargo build -p aivyx-web --target wasm32-unknown-unknown`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/aivyx-web/src/main.rs
git commit -m "feat: render the live mission graph in Mission Control

Roster fetched the same way TeamsPanel/OnboardingTeamStep already do.
Graph re-renders live from the existing missions/running_overlay signals
-- no new subscription. MissionControls and SpecialistDrillIn are stub
components (real bodies in Tasks 5/6) so this task's own graph rendering
compiles and is independently reviewable."
```

---

## Task 5: Specialist drill-in (reusing TeamsPanel's exact NT-02 logic)

**Files:**
- Modify: `crates/aivyx-web/src/main.rs`

**Interfaces:**
- Consumes: `MissionGraphNode` (Task 3), `TeamConfig` (already threaded through from Task 4).

**Verified**: `TeamsPanel`'s NT-02 computation — read in full during this
plan's own research, copied verbatim below.

- [ ] **Step 1: Extract the NT-02 computation into a reusable pure helper**

Find `TeamsPanel`'s existing inline computation:

```rust
    // The lead's declared scopes — for the NT-02 "inert" hint on specialists.
    let lead_scopes: std::collections::HashSet<String> = team
        .members
        .iter()
        .find(|m| m.name == team.lead)
        .map(|m| m.capability_scopes.iter().cloned().collect())
        .unwrap_or_default();
```

Extract this into a standalone function (so `SpecialistDrillIn` can call
the exact same logic, not a reimplementation):

```rust
/// Chapter Mission Control — the lead's declared capability scopes, for
/// the NT-02 "inert" hint: a specialist's own declared scope the lead
/// doesn't also hold is attenuated to nothing at spawn (never granted).
/// Extracted from `TeamsPanel`'s own inline computation so both surfaces
/// share one implementation, not two that could silently drift apart.
fn lead_scopes(team: &TeamConfig) -> std::collections::HashSet<String> {
    team.members
        .iter()
        .find(|m| m.name == team.lead)
        .map(|m| m.capability_scopes.iter().cloned().collect())
        .unwrap_or_default()
}
```

Update `TeamsPanel` to call it instead of the inline version:

```rust
    let lead_scopes = lead_scopes(&team);
```

Find where `TeamsPanel` renders the actual "inert" hint text (search for
`"Lead lacks {widened_text} — inert until the lead holds them"`) and
confirm its exact surrounding computation of `widened_text` (the
specialist's own scopes minus the lead's) — read it fully before writing
Step 3 below, so `SpecialistDrillIn` reuses the identical phrasing/logic,
not an approximation.

- [ ] **Step 2: Write a test for the extracted helper**

Add near wherever this file's existing pure-helper tests for team-related
logic live (or `mod mission_control_tests` if no more specific module
exists):

```rust
    #[test]
    fn lead_scopes_returns_the_leads_own_declared_scopes() {
        let mut roster = sample_roster();
        roster.members[0].capability_scopes = vec!["fs.write".to_string(), "net.fetch".to_string()];
        let scopes = lead_scopes(&roster);
        assert!(scopes.contains("fs.write"));
        assert!(scopes.contains("net.fetch"));
        assert_eq!(scopes.len(), 2);
    }

    #[test]
    fn lead_scopes_is_empty_when_the_lead_is_not_in_members() {
        let mut roster = sample_roster();
        roster.lead = "nobody".to_string();
        assert!(lead_scopes(&roster).is_empty());
    }
```

(Reuse Task 3's own `sample_roster()` fixture if it's still in scope in
the same test module; otherwise duplicate the minimal literal.)

- [ ] **Step 3: Run the tests, then implement the drill-in panel**

Run: `cargo test -p aivyx-web lead_scopes -- --test-threads=1`
Expected: pass.

Replace `SpecialistDrillIn`'s stub body from Task 4:

```rust
#[component]
fn SpecialistDrillIn(node: MissionGraphNode, roster: TeamConfig, mission: TeamMissionView) -> Element {
    let scopes = lead_scopes(&roster);
    let member = roster.members.iter().find(|m| m.name == node.name);
    let declared: Vec<String> = member.map(|m| m.capability_scopes.clone()).unwrap_or_default();
    let inert: Vec<String> = declared.iter().filter(|s| !scopes.contains(*s)).cloned().collect();
    let current_step_detail = node
        .current_step
        .as_ref()
        .and_then(|id| mission.steps.iter().find(|s| &s.step_id == id));
    rsx! {
        div { class: "glass-card drill-in",
            div { class: "row1",
                span { class: "goal", "{node.name}" }
                if node.is_lead { span { class: "chip", "LEAD" } }
            }
            if let Some(step) = current_step_detail {
                p { class: "label-tech", "currently running: {step.label}" }
            } else {
                p { class: "label-tech", "idle — no step currently running" }
            }
            if !declared.is_empty() {
                div { class: "scopes",
                    p { class: "label-tech", "declared capability scopes:" }
                    for s in declared.iter() {
                        span { class: "chip", "{s}" }
                    }
                }
            }
            if !inert.is_empty() {
                p { class: "label-tech inert-hint",
                    "Lead lacks {inert.join(\", \")} — inert until the lead holds them (attenuated at spawn)."
                }
            }
        }
    }
}
```

(Match the EXACT phrasing found in `TeamsPanel`'s own hint text at Step 1
— if it differs from `"Lead lacks {widened_text} — inert until the lead
holds them (attenuated at spawn)."`, use the real text, not this plan's
guess.)

- [ ] **Step 4: Build and verify**

Run: `cargo test -p aivyx-web -- --test-threads=1`
Expected: all pass, including the 2 new `lead_scopes` tests.

Run: `cargo build -p aivyx-web --target wasm32-unknown-unknown`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/aivyx-web/src/main.rs
git commit -m "feat: specialist drill-in panel, reusing TeamsPanel's exact NT-02 logic

lead_scopes() is extracted out of TeamsPanel into a standalone, tested,
shared function -- both surfaces now call one implementation instead of
risking two that drift apart. SpecialistDrillIn shows role, current step,
declared scopes, and the same inert-hint text TeamsPanel already shows."
```

---

## Task 6: Controls — gate approve/reject, abort (first UI), pause/resume

**Files:**
- Modify: `crates/aivyx-web/src/main.rs`

**Interfaces:**
- Consumes: `resolve_team_query` (existing), `QueryPayload::AbortTeamMission`/`PauseTeamMission`/`ResumeTeamMission` (Piece 2, `crates/aivyx-ipc/src/protocol.rs`).
- Produces: `abort_team_mission_query`, `pause_team_mission_query`, `resume_team_mission_query` (new `FrontendMessage` builders, mirroring `resolve_team_query`'s exact shape).

**Verified**: `resolve_team_query`'s exact shape (read in full during this
plan's own research) is the precedent to mirror. No response-handling
code is needed anywhere in the WS read loop — confirmed `TeamGateResolved`
(the existing response to `ResolveTeamGate`) has **zero** explicit
handling in `main.rs` today; the UI relies entirely on the subsequent
`DaemonEnvelope::TeamMissionUpdated` broadcast (Piece 1, already wired
end-to-end, since `SharedMissionState::put` auto-broadcasts on every
write) to reflect the state change. Abort/pause/resume work identically —
fire the query, let the existing live broadcast update the view.

- [ ] **Step 1: Add the three query builders**

Find `resolve_team_query`:

```rust
fn resolve_team_query(mission_id: String, step: String, approve: bool) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-gate".to_string(),
        payload: QueryPayload::ResolveTeamGate { mission_id, step, approve },
    }
}
```

Add three new builders right after it:

```rust
fn abort_team_mission_query(mission_id: String) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-abort".to_string(),
        payload: QueryPayload::AbortTeamMission { mission_id },
    }
}

fn pause_team_mission_query(mission_id: String) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-pause".to_string(),
        payload: QueryPayload::PauseTeamMission { mission_id },
    }
}

fn resume_team_mission_query(mission_id: String) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-resume".to_string(),
        payload: QueryPayload::ResumeTeamMission { mission_id },
    }
}
```

Confirm `QueryPayload::PauseTeamMission`/`ResumeTeamMission`/
`AbortTeamMission` are already in scope via this file's existing
`aivyx_ipc::protocol::{...QueryPayload...}` import (they should be — the
import brings in the whole `QueryPayload` enum, not individual variants —
but verify the import list actually includes `QueryPayload` itself, not
just specific payload types, before assuming this compiles as-is).

- [ ] **Step 2: Implement `MissionControls`**

Replace `MissionControls`'s stub body from Task 4:

```rust
#[component]
fn MissionControls(mission: TeamMissionView) -> Element {
    let ws = use_context::<Sender>();
    let id = mission.id.clone();
    rsx! {
        div { class: "mission-controls",
            if mission.phase == TeamMissionPhase::AwaitingApproval {
                if let Some(gate) = mission.pending_gate.clone() {
                    GateControls { mission_id: mission.id.clone(), step: gate }
                }
            }
            if mission.phase == TeamMissionPhase::Executing {
                button {
                    class: "btn btn-ghost",
                    onclick: {
                        let id = id.clone();
                        move |_| ws.send(pause_team_mission_query(id.clone()))
                    },
                    "Pause"
                }
                button {
                    class: "btn btn-ghost-danger",
                    onclick: {
                        let id = id.clone();
                        move |_| ws.send(abort_team_mission_query(id.clone()))
                    },
                    "Abort"
                }
            }
            if mission.phase == TeamMissionPhase::Paused {
                button {
                    class: "btn btn-sage",
                    onclick: move |_| ws.send(resume_team_mission_query(id.clone())),
                    "Resume"
                }
            }
        }
    }
}
```

(`GateControls` already exists — reused as-is, matching the plan's own
"reuse the existing gate approve/reject pattern" requirement exactly, not
a reimplementation.)

- [ ] **Step 3: Write a pure test for which controls a given phase shows**

Rendering itself isn't unit-testable (per this plan's own Global
Constraint), but the *decision* of which buttons to show is simple enough
to extract and test purely, avoiding "verified only by reading the code"
for this one piece of real logic:

```rust
    #[test]
    fn mission_controls_shown_for_each_phase() {
        assert_eq!(controls_for_phase(TeamMissionPhase::Executing), vec!["pause", "abort"]);
        assert_eq!(controls_for_phase(TeamMissionPhase::Paused), vec!["resume"]);
        assert_eq!(controls_for_phase(TeamMissionPhase::AwaitingApproval), vec!["gate"]);
        assert!(controls_for_phase(TeamMissionPhase::Done).is_empty());
        assert!(controls_for_phase(TeamMissionPhase::Rejected).is_empty());
        assert!(controls_for_phase(TeamMissionPhase::Halted).is_empty());
    }
```

Add the pure decision function `MissionControls` itself calls, so the rsx!
body and the tested logic are the same code, not a shadow copy that could
drift:

```rust
/// Chapter Mission Control — which controls a mission's current phase
/// shows, as opaque tags a test can assert on without a Dioxus runtime.
/// `MissionControls`'s own rsx! branches on the same phase checks this
/// function encodes -- kept in sync by both reading `mission.phase`
/// directly rather than duplicating a separate enum.
fn controls_for_phase(phase: TeamMissionPhase) -> Vec<&'static str> {
    match phase {
        TeamMissionPhase::Executing => vec!["pause", "abort"],
        TeamMissionPhase::Paused => vec!["resume"],
        TeamMissionPhase::AwaitingApproval => vec!["gate"],
        TeamMissionPhase::Done | TeamMissionPhase::Rejected | TeamMissionPhase::Halted => vec![],
    }
}
```

(This function is descriptive/tested but `MissionControls`'s own rsx!
still branches on `mission.phase` directly for the real render — that's
fine and matches how `MissionRow`'s existing `awaiting` boolean already
works; the point of `controls_for_phase` is to have ONE place a reviewer
or future change can check "did I cover every phase" via the compiler's
own exhaustiveness check on the `match`, which the rsx! branches alone
don't get.)

- [ ] **Step 4: Run the tests and build**

Run: `cargo test -p aivyx-web -- --test-threads=1`
Expected: all pass, including the new `mission_controls_shown_for_each_phase` test.

Run: `cargo build -p aivyx-web --target wasm32-unknown-unknown`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/aivyx-web/src/main.rs
git commit -m "feat: Mission Control's controls -- abort/pause/resume (first UI) + gate reuse

abort_team_mission_query/pause_team_mission_query/resume_team_mission_query
mirror resolve_team_query's exact shape. No response handling needed --
the existing TeamMissionUpdated broadcast (Piece 1) already reflects the
state change once it lands. controls_for_phase is a pure, exhaustively-
matched decision function so a future phase addition can't silently be
missed from this view's controls."
```

---

## Final verification (whole plan)

- [ ] `cargo build --workspace --exclude aivyx-desktop --exclude aivyx-web`
      — clean.
- [ ] `cargo test --workspace --exclude aivyx-desktop --exclude aivyx-web -- --test-threads=1`
      — all green; note the new total vs. the pre-Piece-3 baseline (Piece
      2 shipped with the aivyx-ipc/aivyx-channel/aivyx-cli/aivyx-tui side
      at 5628 workspace-wide; this plan's own new tests are almost all in
      `aivyx-web`, which this workspace command excludes — track that
      crate's own total separately, below).
- [ ] `cargo test -p aivyx-web -- --test-threads=1` — all pass. Report the
      exact total (should include every `mission_control_tests`/
      `lead_scopes`/`build_mission_graph`/`controls_for_phase` test added
      across all 6 tasks).
- [ ] `cargo build -p aivyx-web --target wasm32-unknown-unknown` — clean
      (the isolated toolchain from earlier pieces, if still present at
      `/tmp/claude-1000/{rustup-home,cargo-home}`, should be reused rather
      than re-bootstrapped).
- [ ] `cargo clippy -p aivyx-ipc --all-targets` — no new warnings (this
      plan's only non-`aivyx-web` change is Task 1's `TeamStepView`
      enrichment).
- [ ] Manually confirm (by reading, since this can't be driven by a real
      daemon in this environment) the full click-through: Sidebar →
      Mission Control → (no watchable mission: empty state; a watchable
      mission: picker) → pick a mission → graph renders with one node per
      roster member → click a specialist → drill-in shows role/current
      step/scopes/inert-hint → controls show the phase-appropriate set →
      clicking Abort/Pause/Resume fires the right query.
- [ ] Confirm nothing from this plan touched `aivyx-desktop` or
      `aivyx-tui` — `git diff --stat 9294b2ef..HEAD` (9294b2ef is the
      Piece 2 plan-write commit, i.e. Piece 3's own true base) should show
      only `crates/aivyx-ipc/src/team_mission.rs` and
      `crates/aivyx-web/src/main.rs`.
