# Recursive-Scheduling Guard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent a scheduled/triggered run from recursively calling `schedule.create`/`schedule.update`/`schedule.delete`, closing the gap the 2026-08-25 Hermes Agent comparison flagged as unchecked.

**Architecture:** Reuse the existing, already-correct `Message.origin: MessageOrigin` distinction (`Operator` vs `System`) — already set by the one shared `TriggerDispatch::fire()` function 5 of the 6 `TriggerSource` variants funnel through — by threading it one hop further into `ToolContext`, then gating the three schedule write-tools on it. For the team-mission path (which doesn't go through `Message`/`fire()` at all), classify each mission's own `triggered_by` provenance at mission-assembly time and use that classification to tag the `Message` that `SpecialistPool::run` already constructs for every specialist *and* the lead (confirmed: `"lead"` is just another named pool member, dispatched through the identical code path) — which then flows through the same `ToolContext` plumbing from Task 1 for free.

**Tech Stack:** Rust, `aivyx-core`, `aivyx-channel`, `aivyx-team`.

## Global Constraints

- Every new guard-check test must be a genuine mutation-proof test, shown to actually fail when the guard code is reverted/removed — verified by actually performing the mutation, not just asserted.
- A companion "still succeeds under Operator/interactive origin" test is required alongside every "refuses under triggered origin" test, so the guard is proven not to over-block legitimate interactive use.
- Channel-triggered missions (`start_from_goal_for_channel_trigger`) must NOT be blocked by this guard — a real, explicit test proving a channel-triggered mission's `schedule.create` call still succeeds is required, not just the absence of a blocking test.
- Error messages the guard produces on refusal must match `schedule_tool.rs`'s existing style (e.g. "the autonomy level does not permit self-scheduling" / "the agent-created schedule cap ... is reached") — clear, told-you-why, consistent tone.
- `cargo build -p aivyx-core -p aivyx-channel -p aivyx-team` and `cargo test -p aivyx-core -p aivyx-channel -p aivyx-team -- --test-threads=1` must stay clean throughout. Do NOT run `cargo build --workspace`/`cargo test --workspace` — the full workspace has an unrelated, pre-existing, out-of-scope build failure (`javascriptcoregtk-4.1` missing system library in a GUI crate). If `cargo test` shows a large batch of unexpected failures unrelated to this plan's own changes, suspect environment `/tmp` exhaustion (a real, recurring issue in this sandboxed environment this session) before assuming a real regression — retry with `TMPDIR` pointed at a directory under `/home` before concluding otherwise.
- `compute_backcompat_floor` (`aivyx-cli/src/bin/aivyx.rs`) is NOT touched by this plan — confirmed during planning research that `schedule.create`/`update`/`delete` correctly stay in the default floor; this plan closes the gap at the point of USE (the tool's own `execute()`), not by narrowing who can reach the tool.
- **Real finding from planning research, worth recording**: `TriggerSource::Mission` (one of the design's "6 variants") is used today only as a notify/audit-classification tag for mission-outcome auto-notifications (`emit_auto_notify_audit` in `team_mission_driver.rs`) — it does not correspond to any actual "a mission recursively fires another mission" code path. There is nothing for this plan to guard there beyond what Task 1 (5 real `TriggerDispatch::fire()`-based sources) and Task 3 (the team-mission path itself, covering scheduled and — via provenance — any future mission-chain scenario) already cover. No separate task is needed for it.

---

### Task 1: Thread `Message.origin` into `ToolContext`

**Files:**
- Modify: `crates/aivyx-core/src/lib.rs` (`ToolContext` struct, ~line 978)
- Modify: `crates/aivyx-core/src/agent.rs` (`TurnCallEnv` struct at ~line 1000; its two construction sites at ~lines 524 and 602; the `ToolContext { .. }` construction site at ~line 1253; new tests in the `mod tests` block, which starts at line 1434)

**Interfaces:**
- Produces: `ToolContext.message_origin: MessageOrigin` — every tool's `execute()` can now read `ctx.message_origin` to see whether the turn's own message was `MessageOrigin::Operator` (an operator/interactive turn) or `MessageOrigin::System` (fired by `TriggerDispatch::fire()` — Cron, Webhook, FileWatch, Reflection, or Loop). Task 2 consumes this field directly. Task 3's `SpecialistPool::run` produces a `Message` that flows through this exact same plumbing, so Task 3 does not need to touch `ToolContext` at all.

- [ ] **Step 1: Add the field to `ToolContext`**

In `crates/aivyx-core/src/lib.rs`, change:

```rust
pub struct ToolContext<'a> {
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub channel: &'a dyn ChannelContext,
    pub audit: &'a dyn AuditHook,
    pub cancellation: &'a CancellationToken,
}
```

to:

```rust
pub struct ToolContext<'a> {
    pub agent_id: AgentId,
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub channel: &'a dyn ChannelContext,
    pub audit: &'a dyn AuditHook,
    pub cancellation: &'a CancellationToken,
    /// The origin of the message that started this turn — `Operator` for
    /// an interactive turn, `System` for one fired by `TriggerDispatch::
    /// fire()` (cron, webhook, file-watch, reflection, or loop) or by a
    /// trigger-originated team-mission specialist/lead turn. Tools that
    /// must never act unattended (e.g. the schedule.* write tools) check
    /// this before proceeding.
    pub message_origin: MessageOrigin,
}
```

- [ ] **Step 2: Add the field to `TurnCallEnv` and populate both construction sites**

In `crates/aivyx-core/src/agent.rs`, change:

```rust
struct TurnCallEnv<'a> {
    turn_id: TurnId,
    channel: &'a dyn ChannelContext,
    cancellation: &'a CancellationToken,
    effective: &'a CapabilitySet,
}
```

to:

```rust
struct TurnCallEnv<'a> {
    turn_id: TurnId,
    channel: &'a dyn ChannelContext,
    cancellation: &'a CancellationToken,
    effective: &'a CapabilitySet,
    message_origin: MessageOrigin,
}
```

Then update both construction sites (the `message: Message` parameter is in
scope at both — they're both inside `turn()`, ~130-260 lines below its own
`message` parameter):

```rust
                    let env = TurnCallEnv {
                        turn_id,
                        channel,
                        cancellation: &cancellation,
                        effective: &effective,
                        message_origin: message.origin,
                    };
```

(first site, ~line 524, inside the `NextStep::ToolCall` arm) and identically
for the second site (~line 602, inside the `NextStep::ToolCalls` arm) —
same four original fields plus `message_origin: message.origin,`.

- [ ] **Step 3: Destructure the new field in `run_tool_call` and pass it into `ToolContext`**

Change the destructure near the top of `run_tool_call` (~line 1025):

```rust
        let TurnCallEnv {
            turn_id,
            channel,
            cancellation,
            effective,
        } = *env;
```

to:

```rust
        let TurnCallEnv {
            turn_id,
            channel,
            cancellation,
            effective,
            message_origin,
        } = *env;
```

Then add the field to the `ToolContext { .. }` literal (~line 1253):

```rust
        let ctx = ToolContext {
            agent_id: self.id,
            session_id: channel.session_id(),
            turn_id,
            channel,
            audit: self.audit.as_ref(),
            cancellation,
            message_origin,
        };
```

- [ ] **Step 4: Run the build to confirm every other `ToolContext { .. }` and `TurnCallEnv { .. }` construction site in the workspace still compiles**

Run: `cargo build -p aivyx-core -p aivyx-channel -p aivyx-team 2>&1 | tail -80`

Expected: compile errors at every other `ToolContext { .. }`/test-fixture
construction site missing the new field (this workspace's own tests build
`ToolContext` directly in several places — `schedule_tool.rs`'s
`make_ctx`, and likely others in `aivyx-core/src/agent.rs`'s own test
module). List every failing site the compiler reports; each needs
`message_origin: MessageOrigin::Operator` added (the safe default,
matching production code's own `#[default] Operator` — see
`MessageOrigin`'s definition, `crates/aivyx-core/src/lib.rs:140`) unless
Task 2/3's own steps say otherwise for a specific site.

- [ ] **Step 5: Write a capturing test tool and a test proving the plumbing works both ways**

In `crates/aivyx-core/src/agent.rs`'s `mod tests` block, add (near
`FakeTool`, ~line 1650):

```rust
    /// Captures the `message_origin` a turn's tool call actually observed,
    /// via a shared `Mutex` — the write happens inside `execute()`, so the
    /// test can assert on it after `.turn()` returns.
    struct OriginCapturingTool {
        id: ToolId,
        schema: Value,
        observed: Arc<Mutex<Option<MessageOrigin>>>,
    }

    impl OriginCapturingTool {
        fn new(observed: Arc<Mutex<Option<MessageOrigin>>>) -> Self {
            OriginCapturingTool {
                id: ToolId::new(),
                schema: json!({}),
                observed,
            }
        }
    }

    #[async_trait]
    impl Tool for OriginCapturingTool {
        fn id(&self) -> ToolId {
            self.id
        }
        fn name(&self) -> &str {
            "origin.capture"
        }
        fn description(&self) -> &str {
            "test-only: records ctx.message_origin"
        }
        fn input_schema(&self) -> &Value {
            &self.schema
        }
        fn required_scope(&self, _input: &Value) -> Scope {
            Scope::parse("memory.read").unwrap()
        }
        async fn execute(&self, _input: Value, ctx: &ToolContext<'_>) -> ToolOutcome {
            *self.observed.lock().unwrap() = Some(ctx.message_origin);
            ToolOutcome::Completed {
                output: json!({"ok": true}),
                verified: Verification::NotApplicable,
            }
        }
    }

    #[tokio::test]
    async fn tool_context_reflects_system_originated_message() {
        let observed = Arc::new(Mutex::new(None));
        let tool = Arc::new(OriginCapturingTool::new(Arc::clone(&observed)));
        let tool_id = tool.id();
        let audit = RecordingAudit::new();
        let agent = make_agent(
            CapabilitySet::from_scopes([Scope::parse("memory.read").unwrap()]),
            vec![tool],
            audit,
            vec![NextStep::ToolCall {
                tool_id,
                input: json!({}),
                auto_corrected_from: None,
                extracted_from_text: None,
            }],
        );
        let channel = RecordingChannel::new();
        let msg = Message::text(channel.session_id(), "fire").system_originated();
        agent.turn(msg, &channel).await;
        assert_eq!(*observed.lock().unwrap(), Some(MessageOrigin::System));
    }

    #[tokio::test]
    async fn tool_context_reflects_operator_originated_message() {
        // Companion to the test above — proves the plumbing carries BOTH
        // values correctly, not just that it's non-empty.
        let observed = Arc::new(Mutex::new(None));
        let tool = Arc::new(OriginCapturingTool::new(Arc::clone(&observed)));
        let tool_id = tool.id();
        let audit = RecordingAudit::new();
        let agent = make_agent(
            CapabilitySet::from_scopes([Scope::parse("memory.read").unwrap()]),
            vec![tool],
            audit,
            vec![NextStep::ToolCall {
                tool_id,
                input: json!({}),
                auto_corrected_from: None,
                extracted_from_text: None,
            }],
        );
        let channel = RecordingChannel::new();
        let msg = Message::text(channel.session_id(), "hi"); // no .system_originated()
        agent.turn(msg, &channel).await;
        assert_eq!(*observed.lock().unwrap(), Some(MessageOrigin::Operator));
    }
```

Check the top of the `mod tests` block for existing `use` statements —
`Arc`, `Mutex`, `json`, `async_trait` are almost certainly already
imported (used by `FakeTool` and `RecordingAudit` nearby); add
`MessageOrigin` to whatever `use super::*;`/explicit import list is
already there if it isn't pulled in transitively.

- [ ] **Step 6: Run the two new tests**

Run: `cargo test -p aivyx-core tool_context_reflects -- --test-threads=1`
Expected: both `tool_context_reflects_system_originated_message` and
`tool_context_reflects_operator_originated_message` pass.

- [ ] **Step 7: Mutation-proof — comment out `message_origin: message.origin,` at one `TurnCallEnv` site, confirm `tool_context_reflects_system_originated_message` fails, then restore it**

Temporarily change the first `TurnCallEnv` construction site (~line 524)
to `message_origin: MessageOrigin::Operator,` (hard-coded, ignoring the
real message) and re-run the System-origin test:

Run: `cargo test -p aivyx-core tool_context_reflects_system_originated_message -- --test-threads=1`
Expected: FAIL (asserts `Some(MessageOrigin::System)`, observes
`Some(MessageOrigin::Operator)`).

Restore the real `message_origin: message.origin,` line, re-run:

Run: `cargo test -p aivyx-core tool_context_reflects -- --test-threads=1`
Expected: both tests PASS again.

- [ ] **Step 8: Run the full `aivyx-core` suite and commit**

Run: `cargo test -p aivyx-core -- --test-threads=1`
Expected: all tests pass (this task only added fields and two new
tests — no existing behavior changed).

```bash
git add crates/aivyx-core/src/lib.rs crates/aivyx-core/src/agent.rs
git commit -m "Thread Message.origin into ToolContext

Adds ToolContext.message_origin (via a new TurnCallEnv field), sourced
from the turn's own Message.origin at both TurnCallEnv construction
sites. Pure plumbing -- no tool behavior changes yet. Every tool can
now distinguish an operator-originated turn from a System-originated
one (already set correctly today by TriggerDispatch::fire() for cron/
webhook/file-watch/reflection/loop triggers). Part of the recursive-
scheduling-guard design -- see docs/superpowers/specs/2026-08-25-
recursive-scheduling-guard-design.md.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 2: Guard `schedule.create`/`update`/`delete` on `message_origin`

**Files:**
- Modify: `crates/aivyx-channel/src/schedule_tool.rs` (`ScheduleCreateTool::execute`, `ScheduleUpdateTool::execute`, `ScheduleDeleteTool::execute`, and the `make_ctx` test helper in `mod tests`)

**Interfaces:**
- Consumes: `ToolContext.message_origin: MessageOrigin` (Task 1).
- Produces: nothing new for later tasks — this is the guard itself.

- [ ] **Step 1: Update the `make_ctx` test helper**

The existing test helper (in `schedule_tool.rs`'s own `mod tests`) builds
`ToolContext` directly:

```rust
    fn make_ctx<'a>(
        ch: &'a NoopChannel,
        audit: &'a dyn aivyx_core::AuditHook,
    ) -> ToolContext<'a> {
        ToolContext {
            agent_id: AgentId::new(),
            session_id: ch.session,
            turn_id: TurnId::new(),
            channel: ch,
            audit,
            cancellation: &ch.token,
        }
    }
```

Change its signature to take the origin explicitly, so tests can request
either value:

```rust
    fn make_ctx<'a>(
        ch: &'a NoopChannel,
        audit: &'a dyn aivyx_core::AuditHook,
        message_origin: aivyx_core::MessageOrigin,
    ) -> ToolContext<'a> {
        ToolContext {
            agent_id: AgentId::new(),
            session_id: ch.session,
            turn_id: TurnId::new(),
            channel: ch,
            audit,
            cancellation: &ch.token,
            message_origin,
        }
    }
```

Update every existing call site of `make_ctx(&ch, &audit)` in this file's
`mod tests` (there are several — one per existing test that runs a tool)
to `make_ctx(&ch, &audit, aivyx_core::MessageOrigin::Operator)`, preserving
today's behavior for all of them (they're all testing operator-originated
scenarios, which is what the guard added below must continue to allow).

- [ ] **Step 2: Write the failing tests — `schedule.create` refuses under `System` origin, still succeeds under `Operator`**

Add to `schedule_tool.rs`'s `mod tests`:

```rust
    #[tokio::test]
    async fn schedule_create_refuses_when_message_origin_is_system() {
        let tool = ScheduleCreateTool::new();
        let store = schedule_domain().await;
        tool.set_schedule_store(store).unwrap();
        tool.set_growth(GrowthAdoption::BroadAuto).unwrap();
        let (ch, audit) = ctx_parts();
        let ctx = make_ctx(&ch, &audit, aivyx_core::MessageOrigin::System);
        let outcome = tool
            .execute(
                json!({"cron": "0 0 9 * * * *", "prompt": "check something"}),
                &ctx,
            )
            .await;
        let ToolOutcome::Failed(AivyxError::Tool { detail, .. }) = outcome else {
            panic!("expected a refusal, got {outcome:?}");
        };
        assert!(
            detail.contains("triggered") || detail.contains("scheduled"),
            "error should explain the refusal reason: {detail}"
        );
    }

    #[tokio::test]
    async fn schedule_create_still_succeeds_under_operator_origin() {
        // Companion to the refusal test above -- proves the guard doesn't
        // over-block ordinary interactive self-scheduling.
        let tool = ScheduleCreateTool::new();
        let store = schedule_domain().await;
        tool.set_schedule_store(store).unwrap();
        tool.set_growth(GrowthAdoption::BroadAuto).unwrap();
        let (ch, audit) = ctx_parts();
        let ctx = make_ctx(&ch, &audit, aivyx_core::MessageOrigin::Operator);
        let outcome = tool
            .execute(
                json!({"cron": "0 0 9 * * * *", "prompt": "check something"}),
                &ctx,
            )
            .await;
        assert!(matches!(outcome, ToolOutcome::Completed { .. }));
    }
```

Write matching pairs for `schedule.update` and `schedule.delete` too —
each needs a schedule to already exist (create one first via
`ScheduleCreateTool` under `Operator` origin, matching the existing
`schedule_update_rejects_a_prompt_edit_on_a_team_mission_schedule` test's
own setup pattern in this same file), then attempt the update/delete
under `System` origin and assert refusal, plus an `Operator`-origin
companion proving the existing behavior (own-schedules-only authority,
etc.) is unaffected:

```rust
    #[tokio::test]
    async fn schedule_update_refuses_when_message_origin_is_system() {
        let create_tool = ScheduleCreateTool::new();
        let store = schedule_domain().await;
        create_tool.set_schedule_store(store.clone()).unwrap();
        create_tool.set_growth(GrowthAdoption::BroadAuto).unwrap();
        let (ch, audit) = ctx_parts();
        let create_ctx = make_ctx(&ch, &audit, aivyx_core::MessageOrigin::Operator);
        let created = create_tool
            .execute(
                json!({"cron": "0 0 2 * * * *", "prompt": "do a thing"}),
                &create_ctx,
            )
            .await;
        let ToolOutcome::Completed { output, .. } = created else {
            panic!("setup failed")
        };
        let schedule_id = output["schedule_id"].as_str().unwrap().to_string();

        let update_tool = ScheduleUpdateTool::new();
        update_tool.set_schedule_store(store).unwrap();
        let sys_ctx = make_ctx(&ch, &audit, aivyx_core::MessageOrigin::System);
        let outcome = update_tool
            .execute(json!({"schedule_id": schedule_id, "enabled": true}), &sys_ctx)
            .await;
        assert!(matches!(outcome, ToolOutcome::Failed(_)));
    }

    #[tokio::test]
    async fn schedule_delete_refuses_when_message_origin_is_system() {
        let create_tool = ScheduleCreateTool::new();
        let store = schedule_domain().await;
        create_tool.set_schedule_store(store.clone()).unwrap();
        create_tool.set_growth(GrowthAdoption::BroadAuto).unwrap();
        let (ch, audit) = ctx_parts();
        let create_ctx = make_ctx(&ch, &audit, aivyx_core::MessageOrigin::Operator);
        let created = create_tool
            .execute(
                json!({"cron": "0 0 2 * * * *", "prompt": "do a thing"}),
                &create_ctx,
            )
            .await;
        let ToolOutcome::Completed { output, .. } = created else {
            panic!("setup failed")
        };
        let schedule_id = output["schedule_id"].as_str().unwrap().to_string();

        let delete_tool = ScheduleDeleteTool::new();
        delete_tool.set_schedule_store(store).unwrap();
        let sys_ctx = make_ctx(&ch, &audit, aivyx_core::MessageOrigin::System);
        let outcome = delete_tool
            .execute(json!({"schedule_id": schedule_id}), &sys_ctx)
            .await;
        assert!(matches!(outcome, ToolOutcome::Failed(_)));
    }
```

(Operator-origin companions for update/delete already exist in this
file's current test suite — `schedule_update_still_allows_enabled_and_
cron_edits_on_a_team_mission_schedule` and the delete tests already
present — once Step 1's `make_ctx` signature change is applied to them
with `MessageOrigin::Operator`, they continue to serve as the
"still succeeds interactively" companions; no new Operator-origin
delete/update test is required beyond what already exists.)

- [ ] **Step 3: Run the new tests to verify they fail**

Run: `cargo test -p aivyx-channel schedule_create_refuses_when_message_origin_is_system schedule_update_refuses_when_message_origin_is_system schedule_delete_refuses_when_message_origin_is_system -- --test-threads=1`
Expected: all three FAIL (the guard doesn't exist yet — `schedule.create`
currently succeeds regardless of origin).

- [ ] **Step 4: Implement the guard in all three tools**

In `ScheduleCreateTool::execute` (`schedule_tool.rs`), add this check
immediately after the existing cron-empty-string check (before the
prompt/goal validation):

```rust
        if ctx.message_origin == aivyx_core::MessageOrigin::System {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "schedule.create: cannot be called from within a triggered or \
                         scheduled run — creating new schedules is an operator/interactive-\
                         only action, to prevent unattended runs from recursively \
                         propagating more automation"
                    .to_string(),
            });
        }
```

In `ScheduleUpdateTool::execute`, add the identical shape immediately
after the schedule-id-empty check and before the `get_schedule` call:

```rust
        if ctx.message_origin == aivyx_core::MessageOrigin::System {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "schedule.update: cannot be called from within a triggered or \
                         scheduled run — updating schedules is an operator/interactive-\
                         only action, to prevent unattended runs from recursively \
                         propagating more automation"
                    .to_string(),
            });
        }
```

In `ScheduleDeleteTool::execute`, same shape, same placement (after the
schedule-id-empty check):

```rust
        if ctx.message_origin == aivyx_core::MessageOrigin::System {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "schedule.delete: cannot be called from within a triggered or \
                         scheduled run — deleting schedules is an operator/interactive-\
                         only action, to prevent unattended runs from recursively \
                         propagating more automation"
                    .to_string(),
            });
        }
```

- [ ] **Step 5: Run the tests again to verify they pass**

Run: `cargo test -p aivyx-channel schedule_create schedule_update schedule_delete -- --test-threads=1`
Expected: every test in this file passes, including all the new ones and
every pre-existing one (Step 1's `make_ctx` update kept them all on
`Operator` origin, which the new guard allows through unchanged).

- [ ] **Step 6: Mutation-proof — comment out the new guard block in `ScheduleCreateTool::execute`, confirm the refusal test fails, then restore it**

Temporarily delete (or comment out) the `if ctx.message_origin ==
aivyx_core::MessageOrigin::System { ... }` block just added to
`ScheduleCreateTool::execute`, re-run:

Run: `cargo test -p aivyx-channel schedule_create_refuses_when_message_origin_is_system -- --test-threads=1`
Expected: FAIL (the call now succeeds instead of refusing).

Restore the block, re-run:

Run: `cargo test -p aivyx-channel schedule_create_refuses_when_message_origin_is_system -- --test-threads=1`
Expected: PASS.

Repeat this same remove/confirm-fail/restore/confirm-pass cycle for the
`schedule.update` and `schedule.delete` guard blocks against their own
refusal tests.

- [ ] **Step 7: Run the full `aivyx-channel` schedule-tool test module and commit**

Run: `cargo test -p aivyx-channel --lib schedule -- --test-threads=1`
Expected: all tests in `schedule_tool.rs` (and any other file matching
`schedule` in its test names) pass.

```bash
git add crates/aivyx-channel/src/schedule_tool.rs
git commit -m "Guard schedule.create/update/delete against triggered origin

Refuses all three write-half schedule tools when ctx.message_origin is
System (set by TriggerDispatch::fire() for cron/webhook/file-watch/
reflection/loop-triggered turns) -- closes the recursive-scheduling
gap for the prompt-turn path. Operator-origin calls are unaffected.
Part of the recursive-scheduling-guard design -- see docs/superpowers/
specs/2026-08-25-recursive-scheduling-guard-design.md.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```

---

### Task 3: Close the team-mission path

**Files:**
- Modify: `crates/aivyx-team/src/pool.rs` (`SpecialistPool` struct + `::new` + `run`)
- Modify: `crates/aivyx-team/src/assembly.rs` (`TeamAssembly::build` signature + its `SpecialistPool::new` call + its 3 test call sites)
- Modify: `crates/aivyx-channel/src/team_mission_driver.rs` (`assemble_runtime` signature + its `TeamAssembly::build` call; `assemble_runtime`'s one caller, the function starting ~line 570, to compute and pass the classification)

**Interfaces:**
- Consumes: `MessageOrigin` (`aivyx-core`, already a dependency of `aivyx-team` and `aivyx-channel`); the guard from Task 2 (consumed automatically once `SpecialistPool::run`'s `Message` carries the right origin — no direct call into Task 2's code).
- Produces: nothing further downstream — this is the last task.

- [ ] **Step 1: Add the new field to `SpecialistPool` and use it in `run`**

In `crates/aivyx-team/src/pool.rs`, change:

```rust
pub struct SpecialistPool {
    factory: SpecialistFactory,
    config: TeamConfig,
    ceiling: CapabilitySet,
}
```

to:

```rust
pub struct SpecialistPool {
    factory: SpecialistFactory,
    config: TeamConfig,
    ceiling: CapabilitySet,
    /// `System` when this mission's own provenance is a trigger this
    /// codebase treats as unattended (a schedule, or any of the other
    /// `TriggerSource` shapes) -- every specialist AND the lead (just
    /// another named pool member) gets a `System`-origin `Message` for
    /// every turn, so the schedule.* write-tool guard (aivyx-channel's
    /// schedule_tool.rs) refuses them. `Operator` for an interactively-
    /// started mission or a channel-triggered one (a real person sent
    /// the command, authenticated via the channel's own sender
    /// allowlist) -- deliberately excluded from `System` classification.
    message_origin: aivyx_core::MessageOrigin,
}
```

Update `::new`:

```rust
    pub fn new(factory: SpecialistFactory, config: TeamConfig, ceiling: CapabilitySet) -> Self {
        SpecialistPool {
            factory,
            config,
            ceiling,
        }
```

to:

```rust
    pub fn new(
        factory: SpecialistFactory,
        config: TeamConfig,
        ceiling: CapabilitySet,
        message_origin: aivyx_core::MessageOrigin,
    ) -> Self {
        SpecialistPool {
            factory,
            config,
            ceiling,
            message_origin,
        }
```

(Read the rest of `::new`'s body first — there may be more fields set
after the ones shown above; keep every existing line, only add the new
parameter and field.)

Update `run` (~line 251) — change:

```rust
        let msg = Message::text(channel.session_id(), task);

        match agent.turn(msg, &channel).await {
```

to:

```rust
        let mut msg = Message::text(channel.session_id(), task);
        if self.message_origin == aivyx_core::MessageOrigin::System {
            msg = msg.system_originated();
        }

        match agent.turn(msg, &channel).await {
```

- [ ] **Step 2: Thread the new parameter through `TeamAssembly::build`**

In `crates/aivyx-team/src/assembly.rs`, add a `message_origin:
aivyx_core::MessageOrigin` parameter to `build`'s signature (after
`ceiling: CapabilitySet` in the existing parameter list — read the full
current signature first, since the earlier design/plan-research pass
only confirmed the parameters up to `kv_cache_handles`; insert the new
parameter in a sensible position without reordering the existing ones,
and update every place inside `build`'s body that constructs
`SpecialistPool::new(factory, config.clone(), ceiling.clone())` — found
at one call site (~line 88) as of this research — to pass
`message_origin` through: `SpecialistPool::new(factory, config.clone(),
ceiling.clone(), message_origin)`.

Update all 3 test call sites of `TeamAssembly::build(...)` inside this
same file (~lines 194, 213, 278, found via `grep -n "TeamAssembly::
build(" crates/aivyx-team/src/assembly.rs` — re-confirm the exact count
and line numbers fresh, since this plan's own research is a snapshot)
to pass `aivyx_core::MessageOrigin::Operator` for the new parameter,
preserving today's tested behavior for all of them.

- [ ] **Step 3: Thread the classification through `assemble_runtime`**

In `crates/aivyx-channel/src/team_mission_driver.rs`, add a
`message_origin: aivyx_core::MessageOrigin` parameter to
`assemble_runtime`'s signature (after `config: TeamConfig`), and pass it
through to the `TeamAssembly::build(...)` call inside it (~line 1502) as
the new argument Step 2 added.

`assemble_runtime` has exactly one real caller (confirmed via `grep -n
"assemble_runtime(" crates/aivyx-channel/src/team_mission_driver.rs` —
re-confirm this is still true), inside the function whose body starts
around line 570. That function already has `record_snapshot:
Option<MissionRecord>` in scope (from `shared.snapshot(id)`, a few
lines above the `assemble_runtime` call) — classify it right before
the call:

```rust
        let message_origin = classify_trigger_origin(
            record_snapshot.as_ref().and_then(|r| r.triggered_by.as_deref()),
        );
        let (runtime, meter) = assemble_runtime(deps, config, seed_tokens, seed_usd, message_origin)?;
```

Add the classification function itself near the top of
`team_mission_driver.rs` (module-level, alongside its other free
functions):

```rust
/// Classify a mission's `triggered_by` provenance for the recursive-
/// scheduling guard. `None` (interactively started, e.g. `team.run`) and
/// a channel tag (`"channel:<platform>"` — a real person sent the
/// command, authenticated via that channel's own sender allowlist) both
/// map to `Operator`. Anything else (today: a schedule id, either
/// `cfg-...` for a config-defined schedule or `agt-...` for an agent-
/// created one) maps to `System` -- an unattended trigger fired this
/// mission, so its specialists and lead must not be able to recursively
/// create more schedules.
fn classify_trigger_origin(triggered_by: Option<&str>) -> aivyx_core::MessageOrigin {
    match triggered_by {
        None => aivyx_core::MessageOrigin::Operator,
        Some(tag) if tag.starts_with("channel:") => aivyx_core::MessageOrigin::Operator,
        Some(_) => aivyx_core::MessageOrigin::System,
    }
}
```

Re-confirm `MissionRecord.triggered_by`'s exact type (`Option<String>`,
per this plan's own research — re-verify it wasn't changed) and that
schedule-tagged missions really do use `cfg-`/`agt-`-prefixed ids while
channel-tagged missions really do use the `"channel:<platform>"` shape
(both confirmed during this plan's own research — re-confirm fresh, not
from this plan's memory, since the field is on a struct this task
doesn't otherwise touch and could have drifted).

- [ ] **Step 4: Run the build to find and fix every other call site**

Run: `cargo build -p aivyx-team -p aivyx-channel 2>&1 | tail -100`

Expected: compile errors at any remaining `SpecialistPool::new(...)` or
`TeamAssembly::build(...)` call site not yet updated (this plan's own
research found exactly 4 real `TeamAssembly::build` call sites — 1
production, 3 test — and did not exhaustively search for direct
`SpecialistPool::new` call sites outside `assembly.rs` itself; the
compiler will find any this plan's research missed). Fix each with
`aivyx_core::MessageOrigin::Operator` unless it's testing
trigger-originated behavior specifically (only Step 5's new tests
should pass `System`).

- [ ] **Step 5: Write the `SpecialistPool::run` origin-propagation tests**

Test the actual mechanism Task 3 adds — does `SpecialistPool::run` tag
its `Message` correctly based on `self.message_origin` — directly
against `SpecialistPool`, in `crates/aivyx-team/src/pool.rs`'s own
`mod tests`, reusing its existing `FakeProvider`/`pool()`/`member()`
fixtures (confirmed present via this plan's own research: `pool()` at
~line 434 builds a `SpecialistPool` directly from a fake LLM provider,
no `TeamMissionService`/mission-driving machinery needed — this is the
right, minimal, already-established layer to test this at, not a full
end-to-end mission drive). `classify_trigger_origin`'s own contract
(which `triggered_by` shapes map to which `MessageOrigin`) is proven
independently by its own focused unit tests below; together the two
test groups cover the whole path without one large, fragile
integration test spanning three crates.

First, extend `FakeProvider` with a two-step script — request a tool
call, then (after the tool result is fed back) finish with a plain
message — since testing tool-call dispatch needs more than the
existing `says()` constructor's single text-only step:

```rust
    impl FakeProvider {
        /// Scripts a tool call on the first step, then a plain final
        /// message on the second (after the tool result is fed back).
        fn calls_tool(tool_name: &str, input: serde_json::Value) -> Arc<Self> {
            let call_step = FakeStep {
                events: vec![],
                terminal: LlmStepEnd::ToolCalls {
                    calls: vec![aivyx_llm::ToolCallEnd {
                        call_id: "call-1".to_string(),
                        tool_name: tool_name.to_string(),
                        input,
                        name_resolution: aivyx_llm::NameResolution::default(),
                    }],
                    text_so_far: String::new(),
                    usage: aivyx_llm::LlmUsage::default(),
                },
            };
            let final_step = FakeStep {
                events: vec![LlmStreamEvent::TextChunk("done".to_string())],
                terminal: LlmStepEnd::FinalMessage {
                    text: "done".to_string(),
                    usage: aivyx_llm::LlmUsage::default(),
                },
            };
            Arc::new(FakeProvider {
                script: Mutex::new(VecDeque::from(vec![call_step, final_step])),
            })
        }
    }
```

Add a capturing tool (mirrors Task 1's `OriginCapturingTool`, but local
to this file so this test doesn't need a cross-crate dependency on
`aivyx-channel`'s real `schedule.create` — Task 2's own tests already
prove the real tool's behavior against `ctx.message_origin`; this test
proves `SpecialistPool::run` sets that field correctly in the first
place):

```rust
    struct OriginCapturingTool {
        id: aivyx_core::ToolId,
        schema: serde_json::Value,
        observed: Arc<Mutex<Option<aivyx_core::MessageOrigin>>>,
    }
    impl OriginCapturingTool {
        fn new(observed: Arc<Mutex<Option<aivyx_core::MessageOrigin>>>) -> Self {
            OriginCapturingTool {
                id: aivyx_core::ToolId::new(),
                schema: serde_json::json!({}),
                observed,
            }
        }
    }
    #[async_trait]
    impl aivyx_core::Tool for OriginCapturingTool {
        fn id(&self) -> aivyx_core::ToolId {
            self.id
        }
        fn name(&self) -> &str {
            "origin.capture"
        }
        fn description(&self) -> &str {
            "test-only: records ctx.message_origin"
        }
        fn input_schema(&self) -> &serde_json::Value {
            &self.schema
        }
        fn required_scope(&self, _input: &serde_json::Value) -> Scope {
            Scope::parse("fs.read").unwrap()
        }
        async fn execute(
            &self,
            _input: serde_json::Value,
            ctx: &aivyx_core::ToolContext<'_>,
        ) -> aivyx_core::ToolOutcome {
            *self.observed.lock().unwrap() = Some(ctx.message_origin);
            aivyx_core::ToolOutcome::Completed {
                output: serde_json::json!({"ok": true}),
                verified: aivyx_core::Verification::NotApplicable,
            }
        }
    }
```

Extend the `pool()` fixture (or add a sibling) to accept a
`message_origin` and a `base_tools` list — `pool()` currently hard-codes
`vec![]` for `SpecialistFactory::new`'s tools parameter and doesn't pass
a `message_origin` to `SpecialistPool::new` (Step 1 of this task added
that parameter); update `pool()` itself to take both:

```rust
    fn pool(
        provider: Arc<dyn LlmProvider>,
        members: Vec<TeamMember>,
        lead: &str,
        base_tools: Vec<Arc<dyn aivyx_core::Tool>>,
        message_origin: aivyx_core::MessageOrigin,
    ) -> SpecialistPool {
        let config = TeamConfig {
            name: "t".into(),
            description: String::new(),
            lead: lead.into(),
            members,
            dialogue: DialogueConfig::default(),
        };
        let factory = SpecialistFactory::new(provider, "test-model", 4096, Arc::new(NullAuditHook), base_tools);
        let lead_caps = CapabilitySet::from_scopes([Scope::parse("fs.read").unwrap()]);
        SpecialistPool::new(factory, config, lead_caps, message_origin)
    }
```

Update every existing call site of `pool(...)` in this file (the 5
tests already read above — `specialist_channel_floors_trust_to_the_
lead`, `channel_floor_keeps_a_lower_member_ceiling`,
`resolve_matches_specialist_names_case_insensitively`,
`resolve_matches_by_role_label_not_just_name`,
`cannot_delegate_to_the_lead_or_an_unknown_member`, plus
`run_executes_a_specialist_sub_turn_and_returns_its_output` — 6 total,
re-confirm the exact count fresh) to pass `vec![]` for `base_tools` and
`aivyx_core::MessageOrigin::Operator` for `message_origin`, preserving
their current tested behavior exactly.

Now the two new tests:

```rust
    #[tokio::test]
    async fn specialist_message_is_system_originated_when_pool_is_triggered() {
        let observed = Arc::new(Mutex::new(None));
        let tool: Arc<dyn aivyx_core::Tool> =
            Arc::new(OriginCapturingTool::new(Arc::clone(&observed)));
        let p = pool(
            FakeProvider::calls_tool("origin.capture", serde_json::json!({})),
            vec![
                member("lead", &[], TrustTier::Trusted),
                member("worker", &["fs.read"], TrustTier::Trusted),
            ],
            "lead",
            vec![tool],
            aivyx_core::MessageOrigin::System,
        );
        let lead_ch = FakeLeadChannel::at(TrustTier::Trusted);
        p.run("worker", "capture your origin", &lead_ch)
            .await
            .expect("specialist turn completes");
        assert_eq!(*observed.lock().unwrap(), Some(aivyx_core::MessageOrigin::System));
    }

    #[tokio::test]
    async fn specialist_message_is_operator_originated_when_pool_is_interactive() {
        // Companion to the test above -- proves the pool doesn't
        // over-tag every specialist turn as System.
        let observed = Arc::new(Mutex::new(None));
        let tool: Arc<dyn aivyx_core::Tool> =
            Arc::new(OriginCapturingTool::new(Arc::clone(&observed)));
        let p = pool(
            FakeProvider::calls_tool("origin.capture", serde_json::json!({})),
            vec![
                member("lead", &[], TrustTier::Trusted),
                member("worker", &["fs.read"], TrustTier::Trusted),
            ],
            "lead",
            vec![tool],
            aivyx_core::MessageOrigin::Operator,
        );
        let lead_ch = FakeLeadChannel::at(TrustTier::Trusted);
        p.run("worker", "capture your origin", &lead_ch)
            .await
            .expect("specialist turn completes");
        assert_eq!(*observed.lock().unwrap(), Some(aivyx_core::MessageOrigin::Operator));
    }
```

Also add two focused unit tests directly against
`classify_trigger_origin` (in `crates/aivyx-channel/src/team_mission_
driver.rs`'s `mod tests` — this is a free function in that file, not
`pool.rs`; cheap, no pool/mission machinery needed, and pins the
function's own contract independently):

```rust
    #[test]
    fn classify_trigger_origin_maps_schedule_ids_to_system() {
        assert_eq!(
            classify_trigger_origin(Some("cfg-nightly-close")),
            aivyx_core::MessageOrigin::System
        );
        assert_eq!(
            classify_trigger_origin(Some("agt-abc123")),
            aivyx_core::MessageOrigin::System
        );
    }

    #[test]
    fn classify_trigger_origin_maps_channel_tags_and_none_to_operator() {
        assert_eq!(
            classify_trigger_origin(Some("channel:telegram")),
            aivyx_core::MessageOrigin::Operator
        );
        assert_eq!(
            classify_trigger_origin(None),
            aivyx_core::MessageOrigin::Operator
        );
    }
```

- [ ] **Step 6: Run the new tests**

Run: `cargo test -p aivyx-team specialist_message_is -- --test-threads=1`
Expected: both `specialist_message_is_system_originated_when_pool_is_
triggered` and `specialist_message_is_operator_originated_when_pool_is_
interactive` pass, alongside all pre-existing `pool.rs` tests (updated
in Step 5 to pass the two new `pool()` parameters).

Run: `cargo test -p aivyx-channel classify_trigger_origin -- --test-threads=1`
Expected: both unit tests pass.

- [ ] **Step 7: Mutation-proof — verify both new layers are real**

In `pool.rs`, temporarily comment out the `if self.message_origin ==
aivyx_core::MessageOrigin::System { msg = msg.system_originated(); }`
block added in Step 1 (leaving `msg` always `Operator`-origin). Re-run:

Run: `cargo test -p aivyx-team specialist_message_is_system_originated_when_pool_is_triggered -- --test-threads=1`
Expected: FAIL (the specialist's message is observed as `Operator`
instead of `System`).

Restore the block, re-run:

Run: `cargo test -p aivyx-team specialist_message_is -- --test-threads=1`
Expected: both tests PASS again.

Separately, in `team_mission_driver.rs`, temporarily change
`classify_trigger_origin`'s `Some(_) => aivyx_core::MessageOrigin::
System,` arm to `Some(_) => aivyx_core::MessageOrigin::Operator,`. Re-run:

Run: `cargo test -p aivyx-channel classify_trigger_origin_maps_schedule_ids_to_system -- --test-threads=1`
Expected: FAIL.

Restore the real match arm, re-run:

Run: `cargo test -p aivyx-channel classify_trigger_origin -- --test-threads=1`
Expected: both unit tests PASS again.

- [ ] **Step 8: Run the full affected-crate suites and commit**

Run: `cargo test -p aivyx-core -p aivyx-channel -p aivyx-team -- --test-threads=1`
Expected: all tests pass across all three crates — nothing this plan
touched should have broken any pre-existing test.

```bash
git add crates/aivyx-team/src/pool.rs crates/aivyx-team/src/assembly.rs crates/aivyx-channel/src/team_mission_driver.rs
git commit -m "Close the team-mission-path recursive-scheduling gap

Classifies each mission's triggered_by provenance (schedule id ->
System, channel tag or None -> Operator) once at mission-assembly
time, threaded through TeamAssembly::build into SpecialistPool, which
now tags every specialist's AND the lead's own Message (lead is just
another named pool member, dispatched through the identical run()
path) with .system_originated() when appropriate -- reusing Task 1's
ToolContext plumbing and Task 2's guard automatically, no new guard
code needed here. Channel-triggered missions are explicitly excluded
per the design's own scope decision. Closes the recursive-scheduling-
guard design's last open path -- see docs/superpowers/specs/
2026-08-25-recursive-scheduling-guard-design.md.

Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>"
```
