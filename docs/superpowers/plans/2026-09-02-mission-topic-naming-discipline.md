# Mission Topic-Naming Discipline (Take 2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** POLISH_WAVES.md sub-project 5's one remaining item — give the Nonagon mission LEAD a way to assign a canonical `memory_topic` to a delegate step, enforced by rewriting (not prefixing) the specialist's own topic choice at the turn-loop dispatch layer.

**Architecture:** A new optional `memory_topic: Option<String>` field on `StepKind::Delegate`, surfaced in both places the LEAD's decomposition JSON shape is described (`DecomposeTaskTool`'s schema and `decompose_goal`'s hand-authored prompt), threaded through 3 call sites (`TeamRuntime`'s step loop → `SpecialistPool::run` → `SpecialistFactory::build`) to a new, separate `ConcreteAgent` field (`memory_topic_override`), enforced by a new dispatch-layer block in the turn loop that rewrites `input["topic"]` in place for `memory.write` calls specifically — the same "rewrite before `required_scope` runs" pattern the existing `role_prefix`/`session` injections already use, but a rewrite instead of a side-channel key, since this must change the *logical* topic every downstream consumer (audit, Concord, the Memory screen) sees.

**Tech Stack:** Rust, the existing Nonagon mission/`Tool`/`Agent` trait machinery — no new crates, no new IPC surface (this is entirely internal to mission execution).

## Global Constraints

- `StepKind::Delegate`'s new `memory_topic` field is `#[serde(default)]` — every pre-existing persisted mission plan must still decode.
- Exactly two places construct `StepKind::Delegate { ... }` as a literal: `Step::delegate()` (`crates/aivyx-team-types/src/mission.rs`) and `parse_step` (`crates/aivyx-team/src/orchestration.rs`). Everywhere else pattern-matches with `..` and needs no change.
- The new `ConcreteAgent` field (`memory_topic_override`) is separate from the existing `memory_topic_prefix` — do not repurpose or merge them. `memory_topic_prefix` prepends and stays invisible to the audit chain (the interactive/operator-role path, untouched by this plan); `memory_topic_override` **replaces** the topic and is deliberately audit-visible (the mission path, this plan's whole subject).
- The dispatch-layer rewrite is gated on `tool.name() == "memory.write"` specifically — `topic` is also a field name on `memory.read`/`memory.forget`, so gating on "the input has a `topic` key" would be a correctness bug.
- No changes to `aivyx-memory` at all. The rewrite happens before `tool.required_scope(&input)` runs in `aivyx-core/src/agent.rs`'s dispatch code, on the same `input: &mut Value` the existing `role_prefix`/`session` injections already mutate — every downstream consumer of `topic_from_input` in `aivyx-memory` already reads whatever's in `input["topic"]`, so nothing there needs to change.
- No Studio/wasm changes, no IPC wire-type changes — this plan touches only `aivyx-team-types`, `aivyx-team`, and `aivyx-core`. No `dist/` rebuild needed in the final task.
- Full sweep before merge: `cargo clippy --workspace --exclude aivyx-desktop --all-targets -- -D warnings` and `cargo test --workspace --exclude aivyx-desktop` (or this environment's established `default-members`-only fallback).

---

## Task 1: Plan shape — `memory_topic` on `StepKind::Delegate`

**Files:**
- Modify: `crates/aivyx-team-types/src/mission.rs`
- Modify: `crates/aivyx-team/src/orchestration.rs`
- Modify: `crates/aivyx-team/src/planner.rs`

**Interfaces:**
- Consumes: nothing from other tasks — this task is self-contained.
- Produces: `StepKind::Delegate { specialist: String, prompt: String, memory_topic: Option<String> }`, `StepKind::memory_topic(&self) -> Option<&str>`, `Step::with_memory_topic(self, topic: impl Into<String>) -> Self` — Task 3 reads `step.kind.memory_topic()`.

- [ ] **Step 1: Add the field to `StepKind::Delegate`**

In `crates/aivyx-team-types/src/mission.rs`, find `pub enum StepKind { ... }` (search for it) and change the `Delegate` variant from:

```rust
    /// Run the named specialist on `prompt` (the doc's Execute/Delegate).
    Delegate { specialist: String, prompt: String },
```

to:

```rust
    /// Run the named specialist on `prompt` (the doc's Execute/Delegate).
    Delegate {
        specialist: String,
        prompt: String,
        /// POLISH_WAVES.md sub-project 5 — the LEAD's canonical memory-
        /// topic assignment for this step's `memory.write` calls, if
        /// any. `None` (the default, and every pre-existing plan's
        /// implicit value via `#[serde(default)]`) means the specialist
        /// chooses its own topic — today's behavior. When `Some`, every
        /// `memory.write` call this step's specialist makes has its
        /// topic REPLACED with this exact value — not merely namespaced
        /// — so steps the LEAD assigns the same topic produce entries
        /// under one real, consistent name (see
        /// `ConcreteAgent::with_memory_topic_override` in
        /// `aivyx-core/src/agent.rs` for the enforcement).
        #[serde(default)]
        memory_topic: Option<String>,
    },
```

- [ ] **Step 2: Add the `StepKind::memory_topic()` accessor**

In the same file, find `impl StepKind { pub fn member(&self) -> &str { ... } }` (search for `pub fn member`) and add a new method right after it:

```rust
    /// The LEAD-assigned canonical memory topic for a `Delegate` step, if
    /// any. `Gate` steps never write memory as part of judging, so this
    /// is always `None` for them.
    pub fn memory_topic(&self) -> Option<&str> {
        match self {
            StepKind::Delegate { memory_topic, .. } => memory_topic.as_deref(),
            StepKind::Gate { .. } => None,
        }
    }
```

- [ ] **Step 3: Update `Step::delegate()` and add `Step::with_memory_topic()`**

In the same file, find `impl Step { pub fn delegate(...) -> Self { ... } }` (search for `pub fn delegate`) and change it from:

```rust
    /// A `Delegate` step with no dependencies.
    pub fn delegate(id: impl Into<String>, specialist: impl Into<String>, prompt: impl Into<String>) -> Self {
        Step {
            id: id.into(),
            kind: StepKind::Delegate {
                specialist: specialist.into(),
                prompt: prompt.into(),
            },
            deps: Vec::new(),
        }
    }
```

to:

```rust
    /// A `Delegate` step with no dependencies.
    pub fn delegate(id: impl Into<String>, specialist: impl Into<String>, prompt: impl Into<String>) -> Self {
        Step {
            id: id.into(),
            kind: StepKind::Delegate {
                specialist: specialist.into(),
                prompt: prompt.into(),
                memory_topic: None,
            },
            deps: Vec::new(),
        }
    }
```

Then find `pub fn after(mut self, deps: ...) -> Self { ... }` (the existing builder, search for `pub fn after`) and add a new builder right after it:

```rust
    /// Sub-project 5 — assign this `Delegate` step's canonical memory
    /// topic (see `StepKind::Delegate`'s own doc comment). A no-op on a
    /// `Gate` step (there is no `memory_topic` field to set), matching
    /// `after`'s own unconditional-builder shape — callers only use this
    /// on delegate steps in practice, and a `Gate` step silently ignoring
    /// it is harmless (it never reads the field).
    pub fn with_memory_topic(mut self, topic: impl Into<String>) -> Self {
        if let StepKind::Delegate { memory_topic, .. } = &mut self.kind {
            *memory_topic = Some(topic.into());
        }
        self
    }
```

- [ ] **Step 4: Update `parse_step` to read `memory_topic` from JSON**

In `crates/aivyx-team/src/orchestration.rs`, find `fn parse_step(v: &Value) -> Result<Step, String> { ... }` (search for it) and change the `Delegate` construction from:

```rust
    } else if let Some(specialist) = v.get("specialist").and_then(Value::as_str) {
        let prompt = v
            .get("prompt")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("delegate step {id:?} needs a `prompt` (string)"))?;
        StepKind::Delegate {
            specialist: specialist.to_string(),
            prompt: prompt.to_string(),
        }
    } else {
```

to:

```rust
    } else if let Some(specialist) = v.get("specialist").and_then(Value::as_str) {
        let prompt = v
            .get("prompt")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("delegate step {id:?} needs a `prompt` (string)"))?;
        let memory_topic = v.get("memory_topic").and_then(Value::as_str).map(str::to_string);
        StepKind::Delegate {
            specialist: specialist.to_string(),
            prompt: prompt.to_string(),
            memory_topic,
        }
    } else {
```

- [ ] **Step 5: Update `DecomposeTaskTool`'s schema and description**

In the same file, find `impl DecomposeTaskTool { pub fn new(...) -> Self { ... schema: json!({ ... }) ... } }` (search for `DecomposeTaskTool::new`) and change the step-item schema from:

```rust
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "specialist": { "type": "string" },
                                "prompt": { "type": "string" },
                                "reviewer": { "type": "string" },
                                "criteria": { "type": "string" },
                                "deps": { "type": "array", "items": { "type": "string" } }
                            },
                            "required": ["id"]
                        }
```

to:

```rust
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "specialist": { "type": "string" },
                                "prompt": { "type": "string" },
                                "memory_topic": { "type": "string" },
                                "reviewer": { "type": "string" },
                                "criteria": { "type": "string" },
                                "deps": { "type": "array", "items": { "type": "string" } }
                            },
                            "required": ["id"]
                        }
```

Then find `fn description(&self) -> &str { "Decompose a mission..." }` (same `impl Tool for DecomposeTaskTool` block, search for `"Decompose a mission"`) and change it from:

```rust
    fn description(&self) -> &str {
        "Decompose a mission into a DAG of steps and run it. Steps run concurrently where their \
         dependencies allow. Input: { \"goal\": string, \"steps\": [{ \"id\": string, \
         \"specialist\"+\"prompt\" (delegate) | \"reviewer\"+\"criteria\" (gate), \"deps\"?: [id] }] }. \
         Returns each step's output."
    }
```

to:

```rust
    fn description(&self) -> &str {
        "Decompose a mission into a DAG of steps and run it. Steps run concurrently where their \
         dependencies allow. Input: { \"goal\": string, \"steps\": [{ \"id\": string, \
         \"specialist\"+\"prompt\"+\"memory_topic\"? (delegate) | \"reviewer\"+\"criteria\" (gate), \
         \"deps\"?: [id] }] }. Optional `memory_topic` (string): when two or more steps write \
         memory about the same logical subject (e.g. both refine a shared summary), give them \
         the SAME memory_topic so their writes land under one consistent name instead of each \
         specialist inventing its own. Leave unset when a step's memory writes don't need to \
         share a name with any other step. Returns each step's output."
    }
```

- [ ] **Step 6: Update `decompose_goal`'s prompt (`planner.rs`)**

In `crates/aivyx-team/src/planner.rs`, find `fn planner_system_prompt(config: &TeamConfig, allow_gates: bool) -> String { ... }` (search for it) and change the `format!` call's shape-description lines from:

```rust
JSON shape: {{\"goal\": string, \"steps\": [step, ...]}}\n\
Each step is one of:\n\
  - delegate: {{\"id\": string, \"specialist\": <name>, \"prompt\": string, \"deps\": [id, ...]}}\n\
  - gate:     {{\"id\": string, \"reviewer\": <name>, \"criteria\": string, \"mode\": \"auto\" | \"human\", \"deps\": [id, ...]}}\n\n\
Rules:\n\
  - ids are unique and match [a-zA-Z0-9_-].\n\
  - `deps` lists step ids that must finish first; omit or use [] for none.\n\
  - Steps with disjoint deps run concurrently — exploit that.\n\
{gate_rule}\
```

to:

```rust
JSON shape: {{\"goal\": string, \"steps\": [step, ...]}}\n\
Each step is one of:\n\
  - delegate: {{\"id\": string, \"specialist\": <name>, \"prompt\": string, \"memory_topic\": \
string (optional), \"deps\": [id, ...]}}\n\
  - gate:     {{\"id\": string, \"reviewer\": <name>, \"criteria\": string, \"mode\": \"auto\" | \"human\", \"deps\": [id, ...]}}\n\n\
Rules:\n\
  - ids are unique and match [a-zA-Z0-9_-].\n\
  - `deps` lists step ids that must finish first; omit or use [] for none.\n\
  - Steps with disjoint deps run concurrently — exploit that.\n\
  - MEMORY TOPIC AGREEMENT: when two or more delegate steps will write memory about the same \
logical subject (e.g. both steps refine one shared summary), give them the SAME `memory_topic` \
so their writes land under one consistent name — do not let each specialist invent its own name \
for the same thing. Leave `memory_topic` unset when a step's memory writes are their own \
distinct subject.\n\
{gate_rule}\
```

- [ ] **Step 7: Write the tests**

In `crates/aivyx-team-types/src/mission.rs`'s `#[cfg(test)] mod tests` block (if none exists yet in this file, check via `grep -n "mod tests" crates/aivyx-team-types/src/mission.rs` and add one following this crate's own established convention — a plain `#[cfg(test)] mod tests { use super::*; ... }` at the end of the file), add:

```rust
    #[test]
    fn memory_topic_defaults_to_none_and_round_trips_when_set() {
        let s = Step::delegate("a", "worker", "do work");
        assert_eq!(s.kind.memory_topic(), None);
        let s2 = Step::delegate("b", "worker", "do work").with_memory_topic("overall_conditions");
        assert_eq!(s2.kind.memory_topic(), Some("overall_conditions"));
    }

    #[test]
    fn gate_step_memory_topic_is_always_none() {
        let s = Step::gate("g", "reviewer", "good?");
        assert_eq!(s.kind.memory_topic(), None);
    }

    #[test]
    fn with_memory_topic_is_a_no_op_on_a_gate_step() {
        let s = Step::gate("g", "reviewer", "good?").with_memory_topic("x");
        assert_eq!(s.kind.memory_topic(), None, "a gate step has no memory_topic field to set");
    }

    #[test]
    fn old_plan_json_without_memory_topic_still_decodes() {
        // A pre-sub-project-5 persisted plan has no `memory_topic` key at
        // all — #[serde(default)] must still decode it as None.
        let json = r#"{"id":"a","kind":{"Delegate":{"specialist":"worker","prompt":"do it"}},"deps":[]}"#;
        let step: Step = serde_json::from_str(json).unwrap();
        assert_eq!(step.kind.memory_topic(), None);
    }
```

In `crates/aivyx-team/src/orchestration.rs`'s `#[cfg(test)] mod tests` block (search for `#[cfg(test)]` in this file — it already has one, given `decompose_surface_and_scope`/`decompose_rejects_a_cyclic_plan` etc. tests referenced elsewhere in this codebase), add:

```rust
    #[test]
    fn parse_step_reads_memory_topic_when_present() {
        let v = serde_json::json!({
            "id": "a",
            "specialist": "worker",
            "prompt": "do it",
            "memory_topic": "overall_conditions"
        });
        let step = parse_step(&v).unwrap();
        assert_eq!(step.kind.memory_topic(), Some("overall_conditions"));
    }

    #[test]
    fn parse_step_leaves_memory_topic_none_when_absent() {
        let v = serde_json::json!({
            "id": "a",
            "specialist": "worker",
            "prompt": "do it"
        });
        let step = parse_step(&v).unwrap();
        assert_eq!(step.kind.memory_topic(), None);
    }
```

- [ ] **Step 8: Run the tests**

Run: `cargo test -p aivyx-team-types` and `cargo test -p aivyx-team parse_step -- --nocapture`.
Expected: all new tests pass, nothing else broke.

- [ ] **Step 9: Run clippy on both crates**

Run: `cargo clippy -p aivyx-team-types -p aivyx-team --all-targets -- -D warnings`.
Expected: no warnings.

- [ ] **Step 10: Commit**

```bash
git add crates/aivyx-team-types/src/mission.rs crates/aivyx-team/src/orchestration.rs crates/aivyx-team/src/planner.rs
git commit -m "feat(team): add memory_topic to StepKind::Delegate

The LEAD's decomposition can now assign a canonical memory-topic name
to a delegate step (Option<String>, #[serde(default)] for plan
compat). Both places the LEAD's {goal, steps} JSON shape is described
(DecomposeTaskTool's schema/description and decompose_goal's prompt)
gain the field plus guidance: assign the SAME memory_topic to steps
writing about the same logical subject. No enforcement yet -- that's
Tasks 2-3.

POLISH_WAVES.md sub-project 5, Task 1.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 2: `ConcreteAgent`'s memory-topic override + dispatch-layer enforcement

**Files:**
- Modify: `crates/aivyx-core/src/agent.rs`

**Interfaces:**
- Consumes: nothing from Task 1 — this task is independently testable (a raw `ConcreteAgent` built directly, not via the mission machinery).
- Produces: `ConcreteAgent::with_memory_topic_override(self, topic: Option<String>) -> Self` — Task 3's `SpecialistFactory::build` calls this.

- [ ] **Step 1: Add the new field**

Find `pub struct ConcreteAgent { ... memory_topic_prefix: Option<String>, ... }` (search for `memory_topic_prefix: Option<String>,` inside the struct definition, NOT the builder method) and add a new field right after it:

```rust
    memory_topic_prefix: Option<String>,
    /// POLISH_WAVES.md sub-project 5 — the LEAD's canonical memory-topic
    /// assignment for the mission step this agent instance is running,
    /// if any. Separate from `memory_topic_prefix` above: that field
    /// PREPENDS a namespace and stays invisible to the audit chain (the
    /// interactive/operator-role path); this field REPLACES the topic
    /// entirely and is deliberately audit-visible — the whole point is
    /// making the audit chain, Concord's conflict-detector, and the
    /// Memory screen's topic rail all see ONE name across every step
    /// the LEAD assigned it to, not each specialist's own guess. `None`
    /// (the default) preserves pre-sub-project-5 behavior byte-for-byte.
    memory_topic_override: Option<String>,
```

- [ ] **Step 2: Initialize it in `ConcreteAgent::new`**

Find `pub fn new(...) -> Self { ConcreteAgent { ... memory_topic_prefix: None, ... } }` (search for `memory_topic_prefix: None,` inside the constructor body) and add the new field right after it:

```rust
            memory_topic_prefix: None,
            memory_topic_override: None,
```

- [ ] **Step 3: Add the builder method**

Find `pub fn with_memory_topic_prefix(mut self, prefix: Option<String>) -> Self { ... }` (search for it) and add a new builder right after it:

```rust
    /// Sub-project 5 — attach the LEAD's canonical memory-topic
    /// assignment for this agent's mission step. Builder-style, mirroring
    /// `with_memory_topic_prefix`. `None` preserves pre-sub-project-5
    /// behavior byte-for-byte: the specialist's own chosen topic reaches
    /// `memory.write` unmodified, same as today.
    pub fn with_memory_topic_override(mut self, topic: Option<String>) -> Self {
        self.memory_topic_override = topic;
        self
    }
```

- [ ] **Step 4: Add the dispatch-layer enforcement**

Find the existing `role_prefix` injection block (search for `// Phase 11 Task 2 — role memory-topic-prefix injection.`) — it ends with:

```rust
        if let Some(prefix) = self.memory_topic_prefix.as_ref()
            && let Some(obj) = input.as_object_mut()
        {
            obj.insert(
                "role_prefix".to_string(),
                serde_json::Value::String(prefix.clone()),
            );
        }

        let needed: Scope = tool.required_scope(&input);
```

Insert a new block between the closing `}` of that `if let` and `let needed: Scope = ...`:

```rust
        if let Some(prefix) = self.memory_topic_prefix.as_ref()
            && let Some(obj) = input.as_object_mut()
        {
            obj.insert(
                "role_prefix".to_string(),
                serde_json::Value::String(prefix.clone()),
            );
        }

        // Sub-project 5 — LEAD-assigned canonical memory topic. Unlike
        // the role_prefix injection just above (invisible to the model,
        // preserved in the audit chain as the logical topic the agent
        // actually typed), this REWRITES the topic itself: the whole
        // point is that Concord's conflict-detector and the Memory
        // screen's topic rail see ONE name across every step the LEAD
        // assigned it to, not each specialist's own guess. Gated on the
        // tool's own name, not merely "has a topic field" —
        // memory.read/memory.forget also use `topic`, and rewriting
        // theirs would be a correctness bug (a read/forget under a
        // topic the operator or a different tool call didn't ask for).
        if tool.name() == "memory.write"
            && let Some(topic) = self.memory_topic_override.as_ref()
            && let Some(obj) = input.as_object_mut()
        {
            obj.insert("topic".to_string(), serde_json::Value::String(topic.clone()));
        }

        let needed: Scope = tool.required_scope(&input);
```

- [ ] **Step 5: Add an input-capturing `FakeTool` variant for testing**

Find `struct FakeTool { ... }` and `impl FakeTool { ... }` (search for `struct FakeTool`) and add a new constructor + a capture mechanism. Change the struct from:

```rust
    struct FakeTool {
        id: ToolId,
        name: &'static str,
        schema: Value,
        scope_fn: Box<dyn Fn(&Value) -> Scope + Send + Sync>,
    }
```

to:

```rust
    struct FakeTool {
        id: ToolId,
        name: &'static str,
        schema: Value,
        scope_fn: Box<dyn Fn(&Value) -> Scope + Send + Sync>,
        /// Sub-project 5 — when set, `execute` records the exact input
        /// it received here, so a test can assert what actually reached
        /// the tool after any dispatch-layer rewrite (role_prefix
        /// injection, the new memory_topic_override rewrite, etc.) —
        /// not just what the planner originally emitted.
        captured: Option<std::sync::Arc<std::sync::Mutex<Vec<Value>>>>,
    }
```

Then add a new constructor right after the existing `fn new_with_schema(...)` (search for it, inside `impl FakeTool`):

```rust
        /// Sub-project 5 helper — like `new_with_schema`, but also
        /// records every `execute` input into `captured` for later
        /// assertion.
        fn new_capturing(
            name: &'static str,
            schema: Value,
            captured: std::sync::Arc<std::sync::Mutex<Vec<Value>>>,
        ) -> Self {
            FakeTool {
                id: ToolId::new(),
                name,
                schema,
                scope_fn: Box::new(|_| Scope::parse("memory.write").unwrap()),
                captured: Some(captured),
            }
        }
```

Update the 3 existing constructors (`new_bare`, `new_r1`, `new_with_schema`) to initialize the new field as `captured: None` — find each `FakeTool { id: ToolId::new(), name, schema: ..., scope_fn: ... }` literal (3 of them) and add `captured: None,` as the last field in each.

Update `impl Tool for FakeTool`'s `execute` — find:

```rust
        async fn execute(&self, _input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
            ToolOutcome::Completed {
                output: json!({"ok": true}),
                verified: Verification::NotApplicable,
            }
        }
```

and change to:

```rust
        async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
            if let Some(captured) = &self.captured {
                captured.lock().unwrap().push(input);
            }
            ToolOutcome::Completed {
                output: json!({"ok": true}),
                verified: Verification::NotApplicable,
            }
        }
```

- [ ] **Step 6: Write the dispatch-layer test**

Add a new test near `checkpoint_fires_only_for_mutates_fs_root_tools` (search for it — mirror its `VecPlanner`/`NextStep::ToolCall` driving shape exactly):

```rust
    #[tokio::test]
    async fn memory_topic_override_rewrites_the_topic_before_execute() {
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let write_tool: Arc<dyn Tool> = Arc::new(FakeTool::new_capturing(
            "memory.write",
            json!({
                "type": "object",
                "properties": { "topic": { "type": "string" }, "body": { "type": "string" } },
                "required": ["topic", "body"]
            }),
            captured.clone(),
        ));
        let write_id = write_tool.id();
        let registry = Arc::new(ToolRegistry::new(vec![write_tool]));
        let caps = CapabilitySet::from_scopes([Scope::parse("memory.write").unwrap()]);
        let audit = RecordingAudit::new();

        let plan = vec![
            NextStep::ToolCall {
                tool_id: write_id,
                input: json!({ "topic": "specialist-chosen-name", "body": "hello" }),
                auto_corrected_from: None,
                extracted_from_text: None,
            },
            NextStep::FinalMessage("done".to_string()),
        ];
        let plan_arc = Arc::new(plan);

        let agent = ConcreteAgent::new(
            AgentId::new(),
            caps,
            registry,
            audit.clone(),
            move || Box::new(crate::planner::VecPlanner::new((*plan_arc).clone())),
        )
        .with_memory_topic_override(Some("overall_conditions".to_string()));

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let message = Message::text(channel.session, "write a note");
        let _ = agent.turn(message, &channel).await;

        let calls = captured.lock().unwrap();
        assert_eq!(calls.len(), 1, "the memory.write call executed");
        assert_eq!(
            calls[0]["topic"], "overall_conditions",
            "the override REPLACED the specialist's own topic choice, not merely prefixed it"
        );
    }

    #[tokio::test]
    async fn memory_topic_override_does_not_touch_other_tools() {
        // memory.read also has a `topic` field -- the rewrite must be
        // gated on the tool's NAME, not on "has a topic key".
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let read_tool: Arc<dyn Tool> = Arc::new(FakeTool::new_capturing(
            "memory.read",
            json!({
                "type": "object",
                "properties": { "topic": { "type": "string" } },
                "required": ["topic"]
            }),
            captured.clone(),
        ));
        let read_id = read_tool.id();
        let registry = Arc::new(ToolRegistry::new(vec![read_tool]));
        let caps = CapabilitySet::from_scopes([Scope::parse("memory.write").unwrap()]);
        let audit = RecordingAudit::new();

        let plan = vec![
            NextStep::ToolCall {
                tool_id: read_id,
                input: json!({ "topic": "specialist-chosen-name" }),
                auto_corrected_from: None,
                extracted_from_text: None,
            },
            NextStep::FinalMessage("done".to_string()),
        ];
        let plan_arc = Arc::new(plan);

        let agent = ConcreteAgent::new(
            AgentId::new(),
            caps,
            registry,
            audit.clone(),
            move || Box::new(crate::planner::VecPlanner::new((*plan_arc).clone())),
        )
        .with_memory_topic_override(Some("overall_conditions".to_string()));

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let message = Message::text(channel.session, "read a note");
        let _ = agent.turn(message, &channel).await;

        let calls = captured.lock().unwrap();
        assert_eq!(calls.len(), 1, "the memory.read call executed");
        assert_eq!(
            calls[0]["topic"], "specialist-chosen-name",
            "memory.read must NOT be rewritten -- only memory.write is gated"
        );
    }

    #[tokio::test]
    async fn no_memory_topic_override_preserves_pre_sub_project_5_behavior() {
        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let write_tool: Arc<dyn Tool> = Arc::new(FakeTool::new_capturing(
            "memory.write",
            json!({
                "type": "object",
                "properties": { "topic": { "type": "string" }, "body": { "type": "string" } },
                "required": ["topic", "body"]
            }),
            captured.clone(),
        ));
        let write_id = write_tool.id();
        let registry = Arc::new(ToolRegistry::new(vec![write_tool]));
        let caps = CapabilitySet::from_scopes([Scope::parse("memory.write").unwrap()]);
        let audit = RecordingAudit::new();

        let plan = vec![
            NextStep::ToolCall {
                tool_id: write_id,
                input: json!({ "topic": "specialist-chosen-name", "body": "hello" }),
                auto_corrected_from: None,
                extracted_from_text: None,
            },
            NextStep::FinalMessage("done".to_string()),
        ];
        let plan_arc = Arc::new(plan);

        // No .with_memory_topic_override(...) call at all.
        let agent = ConcreteAgent::new(
            AgentId::new(),
            caps,
            registry,
            audit.clone(),
            move || Box::new(crate::planner::VecPlanner::new((*plan_arc).clone())),
        );

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let message = Message::text(channel.session, "write a note");
        let _ = agent.turn(message, &channel).await;

        let calls = captured.lock().unwrap();
        assert_eq!(calls[0]["topic"], "specialist-chosen-name", "unmodified when no override is set");
    }
```

Check the exact real signatures of `RecordingAudit::new()`, `FakeChannel::new(...)`, `Message::text(...)`, and `crate::planner::VecPlanner::new(...)` against `checkpoint_fires_only_for_mutates_fs_root_tools`'s own real, already-compiling usage in this same file (search for that test) before finalizing — the snippets above follow its exact calling shape, but confirm field/variant names (`NextStep::ToolCall`'s exact fields, `AgentId::new()`) match this crate's real, current types, not a stale recollection.

- [ ] **Step 7: Run the tests**

Run: `cargo test -p aivyx-core memory_topic_override -- --nocapture`
Expected: all 3 new tests pass.

- [ ] **Step 8: Run the full crate suite and clippy**

Run: `cargo test -p aivyx-core` (confirm nothing else broke — the `FakeTool` struct-literal changes touch 3 existing constructors) and `cargo clippy -p aivyx-core --all-targets -- -D warnings`.
Expected: all pass, zero warnings.

- [ ] **Step 9: Commit**

```bash
git add crates/aivyx-core/src/agent.rs
git commit -m "feat(core): ConcreteAgent memory-topic override, enforced at dispatch

New with_memory_topic_override builder + a dispatch-layer rewrite
(gated on tool.name() == \"memory.write\") that replaces input[\"topic\"]
in place before required_scope runs -- unlike the existing
memory_topic_prefix (prepends, stays audit-invisible), this REPLACES
the logical topic and is deliberately audit-visible, so downstream
consumers (audit chain, Concord's conflict-detector, the Memory
screen) all see one consistent LEAD-assigned name.

POLISH_WAVES.md sub-project 5, Task 2.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 3: Thread the override through the mission-execution path

**Files:**
- Modify: `crates/aivyx-team/src/factory.rs`
- Modify: `crates/aivyx-team/src/pool.rs`
- Modify: `crates/aivyx-team/src/runtime.rs`

**Interfaces:**
- Consumes: Task 1's `StepKind::memory_topic(&self) -> Option<&str>`; Task 2's `ConcreteAgent::with_memory_topic_override`.
- Produces: `SpecialistFactory::build(&self, member: &TeamMember, ceiling: &CapabilitySet, memory_topic: Option<&str>) -> Result<ConcreteAgent, TeamError>`, `SpecialistPool::run(&self, specialist: &str, task: &str, memory_topic: Option<&str>, lead_channel: &dyn ChannelContext) -> Result<String, TeamError>` — this is the last task in the chain; nothing downstream depends on these signatures beyond this task's own tests.

- [ ] **Step 1: Extend `SpecialistFactory::build`**

In `crates/aivyx-team/src/factory.rs`, find `pub fn build(&self, member: &TeamMember, ceiling: &CapabilitySet) -> Result<ConcreteAgent, TeamError> { ... }` (search for `pub fn build`) and change its signature from:

```rust
    pub fn build(
        &self,
        member: &TeamMember,
        ceiling: &CapabilitySet,
    ) -> Result<ConcreteAgent, TeamError> {
```

to:

```rust
    pub fn build(
        &self,
        member: &TeamMember,
        ceiling: &CapabilitySet,
        memory_topic: Option<&str>,
    ) -> Result<ConcreteAgent, TeamError> {
```

Then find `.with_checkpointer(self.checkpointer.clone());` (the chain call right after `ConcreteAgent::new(...)`, search for it) and change:

```rust
        .with_checkpointer(self.checkpointer.clone());
```

to:

```rust
        .with_checkpointer(self.checkpointer.clone())
        .with_memory_topic_override(memory_topic.map(String::from));
```

(Note the removed trailing `;` on the first line — it moves to the end of the new chained call.)

- [ ] **Step 2: Update `build`'s existing callers in this same file's tests**

Search this file for `SpecialistFactory::build` call sites — `grep -n '\.build(' crates/aivyx-team/src/factory.rs` finds 12 `.build(` matches, but ONE (around line 577, `FsWriteToolConfig::new(...).build()`) is an unrelated type's builder, not this method — 11 real call sites need the new argument. Add `, None` as the third argument to each, e.g. `f.build(&m, &lead).expect("build")` becomes `f.build(&m, &lead, None).expect("build")`. There is no production call site in this file; the `pool.rs` change in Step 3 below is the only production caller.

- [ ] **Step 3: Extend `SpecialistPool::run`**

In `crates/aivyx-team/src/pool.rs`, find `pub async fn run(&self, specialist: &str, task: &str, lead_channel: &dyn ChannelContext) -> Result<String, TeamError> { ... }` (search for `pub async fn run`) and change:

```rust
    pub async fn run(
        &self,
        specialist: &str,
        task: &str,
        lead_channel: &dyn ChannelContext,
    ) -> Result<String, TeamError> {
        let member = self.resolve(specialist)?;
        let agent = self.factory.build(member, &self.ceiling)?;
```

to:

```rust
    pub async fn run(
        &self,
        specialist: &str,
        task: &str,
        memory_topic: Option<&str>,
        lead_channel: &dyn ChannelContext,
    ) -> Result<String, TeamError> {
        let member = self.resolve(specialist)?;
        let agent = self.factory.build(member, &self.ceiling, memory_topic)?;
```

- [ ] **Step 4: Update `pool.rs`'s own existing test call sites**

Search this file's `#[cfg(test)] mod tests` block for `.run(` calls on a `SpecialistPool` — there are exactly 4 (`grep -n '\.run(' crates/aivyx-team/src/pool.rs` to find them) — and add `, None` as the new third positional argument (before `lead_channel`) to each, e.g. `p.run("inventory", "check stock", &lead_ch)` becomes `p.run("inventory", "check stock", None, &lead_ch)`.

- [ ] **Step 5: Update `TeamRuntime`'s step loop**

In `crates/aivyx-team/src/runtime.rs`, find the `futures = runnable.iter().map(|step| { ... self.pool.run(&member, &input, lead_channel).await ... })` block (search for `self.pool.run(&member, &input, lead_channel)`) and change:

```rust
            let futures = runnable.iter().map(|step| {
                let id = step.id.clone();
                let member = step.kind.member().to_string();
                let input = self.build_input(step, &outputs);
                async move {
                    observer.on_step_started(&id, &member);
                    let res = self.pool.run(&member, &input, lead_channel).await;
                    (id, res)
                }
            });
```

to:

```rust
            let futures = runnable.iter().map(|step| {
                let id = step.id.clone();
                let member = step.kind.member().to_string();
                let memory_topic = step.kind.memory_topic().map(str::to_string);
                let input = self.build_input(step, &outputs);
                async move {
                    observer.on_step_started(&id, &member);
                    let res = self.pool.run(&member, &input, memory_topic.as_deref(), lead_channel).await;
                    (id, res)
                }
            });
```

- [ ] **Step 6: Write the end-to-end mission-level test**

In `crates/aivyx-team/src/runtime.rs`'s `#[cfg(test)] mod tests` block, add a new test right after `run_observed_reports_progress_in_order` (search for it — reuse this file's own `FakeProvider`/`member`/`TeamConfig`/`SpecialistFactory`/`SpecialistPool` imports already in scope in that module). This test builds its own pool (not the shared `runtime()`/`team_pool` helpers, which hardcode an empty tool set) so the specialist roster has a real `memory.write`-named tool to call:

```rust
    #[tokio::test]
    async fn two_steps_sharing_a_memory_topic_both_get_the_override() {
        use crate::factory::SpecialistFactory;
        use crate::pool::SpecialistPool;
        use aivyx_capability::CapabilitySet;
        use aivyx_core::{NullAuditHook, Scope, Tool, ToolId, ToolOutcome, Verification};
        use async_trait::async_trait;
        use serde_json::{json, Value};

        struct CapturingWriteTool {
            id: ToolId,
            schema: Value,
            captured: std::sync::Arc<std::sync::Mutex<Vec<Value>>>,
        }
        #[async_trait]
        impl Tool for CapturingWriteTool {
            fn id(&self) -> ToolId {
                self.id
            }
            fn name(&self) -> &str {
                "memory.write"
            }
            fn description(&self) -> &str {
                "fake memory.write"
            }
            fn input_schema(&self) -> &Value {
                &self.schema
            }
            fn required_scope(&self, _input: &Value) -> Scope {
                Scope::parse("memory.write").unwrap()
            }
            async fn execute(&self, input: Value, _ctx: &aivyx_core::ToolContext<'_>) -> ToolOutcome {
                self.captured.lock().unwrap().push(input);
                ToolOutcome::Completed {
                    output: json!({"ok": true}),
                    verified: Verification::NotApplicable,
                }
            }
        }

        let captured = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let write_tool: Arc<dyn Tool> = Arc::new(CapturingWriteTool {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": { "topic": { "type": "string" }, "body": { "type": "string" } },
                "required": ["topic", "body"]
            }),
            captured: captured.clone(),
        });

        // Each specialist call writes memory under its OWN topic guess
        // (mirroring the original finding: different steps disagree).
        let provider = FakeProvider::tool_call_then_done(
            "memory.write",
            json!({ "topic": "specialist-own-guess", "body": "note" }),
        );

        let members = vec![
            member("lead", &[], TrustTier::Trusted),
            member("a", &["memory.write"], TrustTier::Trusted),
            member("b", &["memory.write"], TrustTier::Trusted),
        ];
        let config = crate::config::TeamConfig {
            name: "t".into(),
            description: String::new(),
            lead: "lead".into(),
            members,
            dialogue: Default::default(),
        };
        let factory = SpecialistFactory::new(
            provider,
            "test-model",
            4096,
            Arc::new(NullAuditHook),
            vec![write_tool],
        );
        let lead_caps = CapabilitySet::from_scopes([]);
        let pool = SpecialistPool::new(factory, config, lead_caps, aivyx_core::MessageOrigin::Operator);
        let rt = TeamRuntime::new(Arc::new(pool));

        let plan = MissionPlan::new(
            "shared summary",
            vec![
                Step::delegate("a", "a", "write the summary").with_memory_topic("overall_conditions"),
                Step::delegate("b", "b", "refine the summary").with_memory_topic("overall_conditions"),
            ],
        );
        let lead = FakeLeadChannel::at(TrustTier::Trusted);
        let report = rt.run(&plan, &lead).await.unwrap();
        assert!(report.succeeded());

        let calls = captured.lock().unwrap();
        assert_eq!(calls.len(), 2, "both steps' memory.write calls executed");
        assert!(
            calls.iter().all(|c| c["topic"] == "overall_conditions"),
            "both steps' writes must land under the LEAD-assigned canonical topic, \
             not each specialist's own guess: {calls:?}"
        );
    }
```

Check `TeamRuntime::run`'s (vs. `run_observed`'s) exact real signature and `MissionPlan::new`'s exact real signature against this file's own already-compiling neighboring tests before finalizing — the snippet above follows the existing `run_observed_reports_progress_in_order` test's shape, but confirm `rt.run(&plan, &lead)` (no observer) is a real, current method on `TeamRuntime` in this crate, not a stale recollection; if only `run_observed` exists, use that with a throwaway observer (e.g. `RecordingObserver::default()`, already defined in this same test module) instead.

- [ ] **Step 7: Run the tests**

Run: `cargo test -p aivyx-team` (full suite — this task's changes touch 3 files' call sites across the crate).
Expected: all tests pass, including the 3 new ones (Task 1's `parse_step` tests should already be green from Task 1; this step re-confirms nothing regressed).

- [ ] **Step 8: Run clippy**

Run: `cargo clippy -p aivyx-team --all-targets -- -D warnings`.
Expected: no warnings.

- [ ] **Step 9: Check for other crates' call sites**

Run: `cargo build --workspace --exclude aivyx-desktop 2>&1 | grep -E "error|warning: unused"` (or the `default-members`-only fallback if `--exclude` doesn't apply cleanly in this environment) to confirm no OTHER crate (`aivyx-cli`, `aivyx-channel`) calls `SpecialistFactory::build`/`SpecialistPool::run` directly with the old 2-argument/3-argument shape — the earlier research for this plan found none, but this step is the real verification, not an assumption. If any turn up, fix them the same way Step 2/4 did (pass `None` for `memory_topic` — no production caller needs to supply a topic yet; `TeamRuntime`'s own step loop, updated in Step 5 above, is the only caller that ever will).

Expected: clean build, no call-site errors.

- [ ] **Step 10: Commit**

```bash
git add crates/aivyx-team/src/factory.rs crates/aivyx-team/src/pool.rs crates/aivyx-team/src/runtime.rs
git commit -m "feat(team): thread memory_topic from a mission step to its specialist

SpecialistFactory::build and SpecialistPool::run both gain a
memory_topic: Option<&str> parameter; TeamRuntime's step loop extracts
it from step.kind.memory_topic() and threads it through. A fresh
specialist agent is built per step execution (confirmed, not assumed),
so this is a genuinely per-step assignment, not a mission-wide one.
End-to-end test: two delegate steps sharing one LEAD-assigned
memory_topic both produce memory.write calls under that one name,
closing the original finding (different specialists disagreeing on
what to call the same logical subject within one mission).

POLISH_WAVES.md sub-project 5, Task 3.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## Task 4: Full workspace sweep

**Files:**
- None created; verification only. No `dist/` rebuild — this plan touches no Studio/wasm code.

**Interfaces:**
- Consumes: everything from Tasks 1-3.
- Produces: nothing — this is the plan's final task, gating the whole-branch review.

- [ ] **Step 1: Full clippy sweep**

Run: `cargo clippy --workspace --exclude aivyx-desktop --all-targets -- -D warnings`
Expected: zero warnings. If `aivyx-desktop` cannot be excluded this way in this checkout, fall back to `cargo clippy --all-targets -- -D warnings` (no `--workspace`, touches only `default-members`, which already excludes `aivyx-desktop`).

- [ ] **Step 2: Full test sweep**

Run: `cargo test --workspace --exclude aivyx-desktop`
Expected: zero failures. Same fallback as Step 1 if needed: `cargo test` (default-members only).

- [ ] **Step 3: Commit**

Only if Steps 1-2 found and fixed anything; otherwise there is nothing to commit for this task (all 3 prior tasks already committed clean, sweep-verified work). If a fix was needed:

```bash
git add -A
git commit -m "chore: workspace sweep for sub-project 5's remaining item

cargo clippy/test clean across the workspace (aivyx-desktop excluded --
missing system webkit2gtk libs in this environment, pre-existing and
unrelated to this branch).

POLISH_WAVES.md sub-project 5, Task 4.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

## After all tasks: whole-branch review

Dispatch the final whole-branch code review on the most capable available model. Point it at:

- The design spec: `docs/superpowers/specs/2026-09-02-mission-topic-naming-discipline-design.md`.
- This plan file.
- A `scripts/review-package MERGE_BASE HEAD` diff package (`MERGE_BASE = git merge-base main HEAD`).

Specifically ask the reviewer to verify:

1. **The rewrite really replaces, not merely adds alongside.** Confirm `input["topic"]` is overwritten (`obj.insert("topic", ...)` on an already-present key replaces its value in `serde_json::Map` — this is standard `HashMap`/`Map` `insert` semantics, but verify the actual behavior wasn't accidentally implemented as "insert only if absent" or a different key name that would leave the original `topic` untouched alongside a new one).
2. **The `tool.name() == "memory.write"` gate is checked before the rewrite, not after** — a wrongly-ordered check would either rewrite unconditionally (breaking `memory.read`/`memory.forget`) or never rewrite at all silently.
3. **`SpecialistFactory::build`'s new parameter reaches `ConcreteAgent` correctly through every call site** — re-verify Task 3's own Step 9 (the workspace-wide grep for other callers) actually found everything; a stale caller elsewhere in the workspace that the plan's own research missed would silently never get a topic override, which is a correctness gap this review should hunt for specifically, not just trust the plan's own claim that only `TeamRuntime`'s step loop calls it in production.
4. **The `#[serde(default)]` compatibility claim holds** — deserialize an actual pre-existing persisted mission-plan JSON fixture (if one exists in the repo, e.g. under test fixtures or `docs/`) missing the `memory_topic` key entirely, confirming it still decodes without error.
5. Standard sweep: run the tests and clippy yourself (`cargo test` default-members, `cargo clippy --all-targets -- -D warnings`) — don't just trust the per-task reports.

After the review (and any fix wave + re-review it triggers), invoke `superpowers:finishing-a-development-branch` for the feature branch, then update `docs/POLISH_WAVES.md` (closing out sub-project 5's last open item — the whole sub-project can then be marked fully done) and `aivyx-ecosystem/ROADMAP.md`.
