//! The concrete reference `Agent` implementation — the Phase 1 turn loop.
//!
//! `ConcreteAgent` composes:
//! - a `CapabilitySet` (the agent's granted scopes)
//! - a `ToolRegistry` (tools it can call)
//! - a `TurnPlanner` (what drives the tool-calling loop — fake in Phase 1)
//! - an `AuditHook` (where audit events go — any `aivyx_audit::AuditWriter`)
//!
//! The loop follows D1's paragraph, in order:
//!
//! 1. Generate `TurnId`. Resolve the channel's trust tier.
//! 2. Compute `effective = caps.intersect(tier.default_ceiling())`.
//! 3. Emit `TurnStarted` audit event.
//! 4. Tool-calling loop. Each pass: check cancellation (→ `Cancelled` if
//!    fired), ask the planner for the next step, and dispatch.
//!    On `ToolCall`: compute required scope via R1, check `effective.grants`,
//!    then either execute and emit a `ToolCall` audit or emit `ScopeDenied`
//!    and pass a `Denied` observation back to the planner.
//!    On `FinalMessage` / `Stop`: terminate with `Completed`.
//! 5. Emit `TurnEnded`, finalize the channel, return.
//!
//! Deferred from Phase 1:
//! - Timeout enforcement (variant exists, loop doesn't check a budget yet —
//!   real deadlines land when a real use case shows up)
//! - `RequiresEscalation` propagation from tools (variant exists, no tool
//!   emits it in Phase 1)
//! - LLM-backed planning (covered by Phase 2's `LlmProvider` + its own
//!   `TurnPlanner` impl)

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use aivyx_capability::{CapabilitySet, Scope};

use crate::{
    Agent, AgentId, AivyxError, AuditHook, AuditTag, CancellationToken, ChannelContext, Message,
    ToolContext, ToolId, ToolOutcome, ToolOutcomeSummary, TurnId, TurnOutcome, TurnOutcomeSummary,
};
use crate::planner::{NextStep, StepObservation, ToolRegistry, TurnPlanner};

/// Hard upper bound on steps per turn. The LLM-backed planner added in
/// Phase 2 is the first planner that *can* loop indefinitely
/// (`VecPlanner` is bounded by its script length), so the loop now
/// guards against a runaway agent with a fixed budget. 32 is high
/// enough for realistic tool chains and low enough that a misbehaving
/// planner fails loudly rather than burning the host.
pub const MAX_STEPS_PER_TURN: usize = 32;

/// Wall-clock deadline for a single turn. Phase 3 task 4 adds the first
/// code path that emits `TurnOutcome::TimedOut`. A background task
/// spawned by the turn loop cancels the channel's cancellation token
/// when the deadline fires; the planner's mid-stream cancel check then
/// propagates the cancel and the loop translates it into `TimedOut`
/// rather than `Cancelled`.
///
/// 120 seconds is deliberately generous — a multi-step tool chain with
/// several large LLM completions can legitimately take most of a
/// minute, and the point of the budget is to catch *stuck* turns, not
/// to police slow ones. Follows the same "const, not config knob"
/// philosophy as [`MAX_STEPS_PER_TURN`]: a caller who needs a custom
/// budget is almost certainly papering over a real bug.
pub const TURN_TIMEOUT: Duration = Duration::from_secs(120);

/// The reference `Agent` implementation.
///
/// Holds all the collaborators a turn loop needs by `Arc` / interior
/// mutability so that the `Agent::turn(&self, ...)` contract holds:
/// concurrent turns share the agent via `Arc<dyn Agent>`, and each turn
/// produces its own transient state over shared immutable collaborators.
pub struct ConcreteAgent {
    id: AgentId,
    capabilities: CapabilitySet,
    tools: Arc<ToolRegistry>,
    audit: Arc<dyn AuditHook>,
    /// Planner factory: called once per turn. We store a boxed `Fn` so
    /// callers can hand us a fresh planner per turn without us needing to
    /// own a `Mutex<Planner>` (which would serialize concurrent turns).
    planner_factory: Box<dyn Fn() -> Box<dyn TurnPlanner> + Send + Sync>,
}

impl ConcreteAgent {
    pub fn new(
        id: AgentId,
        capabilities: CapabilitySet,
        tools: Arc<ToolRegistry>,
        audit: Arc<dyn AuditHook>,
        planner_factory: impl Fn() -> Box<dyn TurnPlanner> + Send + Sync + 'static,
    ) -> Self {
        ConcreteAgent {
            id,
            capabilities,
            tools,
            audit,
            planner_factory: Box::new(planner_factory),
        }
    }
}

#[async_trait]
impl Agent for ConcreteAgent {
    fn id(&self) -> AgentId {
        self.id
    }

    fn capabilities(&self) -> &CapabilitySet {
        &self.capabilities
    }

    async fn turn(
        &self,
        message: Message,
        channel: &dyn ChannelContext,
    ) -> TurnOutcome {
        let turn_id = TurnId::new();
        let session_id = channel.session_id();
        let tier = channel.trust_tier();
        let effective = self.capabilities.intersect(tier.default_ceiling());
        let cancellation = channel.cancellation_token();
        let start = Instant::now();

        // D1: "trust tier resolution happens first" — fire TurnStarted
        // *before* the loop begins, with the authoritative effective set.
        self.audit.on_event(AuditTag::TurnStarted {
            turn_id,
            session_id,
            channel: channel.platform(),
            trust_tier: tier,
            effective_capabilities: effective.clone(),
        });

        let mut planner = (self.planner_factory)();
        planner.begin_turn(&message).await;

        // Wall-clock deadline task. Spawns in the background, sleeps
        // for TURN_TIMEOUT, and then (a) sets the deadline_fired flag
        // so the loop's outcome translation can distinguish TimedOut
        // from Cancelled, and (b) cancels the channel's token so the
        // planner's in-flight stream (if any) gets interrupted. We
        // hold a handle so the task is aborted cleanly when the turn
        // ends normally — otherwise a fleet of long-running agents
        // would leak timeout tasks until they eventually fired.
        let deadline_fired = Arc::new(AtomicBool::new(false));
        let deadline_task = {
            let deadline_fired = Arc::clone(&deadline_fired);
            let token = cancellation.clone();
            tokio::spawn(async move {
                tokio::time::sleep(TURN_TIMEOUT).await;
                deadline_fired.store(true, Ordering::SeqCst);
                token.cancel();
            })
        };

        let mut observed: Vec<StepObservation> = Vec::new();
        let mut tool_calls_made: usize = 0;
        let mut final_message: String = String::new();
        let mut steps: usize = 0;
        let loop_outcome: LoopOutcome;

        loop {
            if cancellation.is_cancelled() {
                loop_outcome = if deadline_fired.load(Ordering::SeqCst) {
                    LoopOutcome::TimedOut
                } else {
                    LoopOutcome::Cancelled
                };
                break;
            }
            if steps >= MAX_STEPS_PER_TURN {
                loop_outcome = LoopOutcome::MaxStepsExceeded;
                break;
            }
            steps += 1;

            let step = planner.next_step(&observed, channel).await;

            // Post-next_step cancellation re-check. The planner may
            // have returned because it detected cancellation inside
            // its own stream consumer (mid-LLM-completion) — in that
            // case we must NOT process its return value as a
            // FinalMessage / Stop, because doing so would emit a
            // Completed outcome when the turn was actually
            // interrupted. We let the next loop iteration's top-of-
            // loop check handle the termination uniformly.
            if cancellation.is_cancelled() {
                loop_outcome = if deadline_fired.load(Ordering::SeqCst) {
                    LoopOutcome::TimedOut
                } else {
                    LoopOutcome::Cancelled
                };
                break;
            }

            match step {
                NextStep::FinalMessage(msg) => {
                    final_message = msg;
                    loop_outcome = LoopOutcome::Completed;
                    break;
                }
                NextStep::Stop => {
                    loop_outcome = LoopOutcome::Completed;
                    break;
                }
                NextStep::ToolCall { tool_id, input } => {
                    tool_calls_made += 1;
                    let (observation, outcome) = self
                        .run_tool_call(
                            turn_id,
                            tool_id,
                            input,
                            channel,
                            &cancellation,
                            &effective,
                        )
                        .await;
                    observed.push(observation);
                    planner.observe_tool_outcome(tool_id, &outcome).await;
                }
            }
        }

        // Abort the deadline task — it's either (a) already fired and
        // cancelled the token, in which case the abort is a no-op, or
        // (b) still sleeping, in which case we want it gone so it
        // doesn't leak. Either way, explicit abort is cheap and
        // intentional.
        deadline_task.abort();

        let duration = start.elapsed();
        let outcome = match loop_outcome {
            LoopOutcome::Completed => TurnOutcome::Completed {
                final_message,
                tool_calls_made,
                duration,
            },
            LoopOutcome::Cancelled => TurnOutcome::Cancelled { tool_calls_made },
            LoopOutcome::TimedOut => TurnOutcome::TimedOut {
                tool_calls_made,
                elapsed: duration,
            },
            LoopOutcome::MaxStepsExceeded => TurnOutcome::Failed(AivyxError::Internal(
                format!("planner exceeded {MAX_STEPS_PER_TURN} steps per turn"),
            )),
        };

        self.audit.on_event(AuditTag::TurnEnded {
            turn_id,
            outcome: TurnOutcomeSummary::from(&outcome),
            tool_calls_made,
            duration,
        });

        // D1: "returning a TurnOutcome to the channel, and yielding control."
        // finalize is synchronous from the loop's perspective — if the
        // channel send fails we downgrade to Failed but still return an
        // outcome, per D3's "every turn completes in some way."
        if let Err(e) = channel.finalize(&outcome).await {
            return TurnOutcome::Failed(crate::AivyxError::Channel(e.to_string()));
        }

        outcome
    }
}

/// Internal loop termination reason before it's translated into a public
/// `TurnOutcome`. Phase 2 added `MaxStepsExceeded`; Phase 3 task 4
/// adds `TimedOut` (the first code path that emits the long-
/// advertised `TurnOutcome::TimedOut`). `Escalated` will join this
/// enum if and when a real tool emits it.
enum LoopOutcome {
    Completed,
    Cancelled,
    TimedOut,
    MaxStepsExceeded,
}

impl ConcreteAgent {
    /// Execute one tool call: resolve the tool, compute its required
    /// scope via R1, scope-check, execute-or-deny, emit the matching
    /// audit event, and return both the observation (for the
    /// `StepObservation` trail) and the full `ToolOutcome` (for the
    /// planner's `observe_tool_outcome` callback). The observation is
    /// what the audit sees; the full outcome is what a smart planner
    /// (e.g. the LLM planner) needs to reason about next.
    async fn run_tool_call(
        &self,
        turn_id: TurnId,
        tool_id: ToolId,
        input: serde_json::Value,
        channel: &dyn ChannelContext,
        cancellation: &CancellationToken,
        effective: &CapabilitySet,
    ) -> (StepObservation, ToolOutcome) {
        let Some(tool) = self.tools.get(tool_id) else {
            // Unknown tool — no scope check possible. This shouldn't happen
            // with a well-behaved planner; treat it as a failed step and
            // synthesize a Failed outcome so the planner sees it too.
            let outcome = ToolOutcome::Failed(AivyxError::NotFound {
                kind: "tool",
                id: tool_id.to_string(),
            });
            return (
                StepObservation {
                    tool_id,
                    summary: ToolOutcomeSummary::Failed,
                },
                outcome,
            );
        };

        // Phase 10 Task 2 — hand-rolled JSON-schema validation.
        //
        // Runs *before* session injection, not after: memory tool
        // schemas set `additionalProperties: false` and do not
        // declare a `session` property, so validating the
        // post-injection input would reject every memory call the
        // moment a Telegram-style channel provides a partition. The
        // session field is a turn-loop internal, not an agent-
        // visible surface, so it sits outside the schema contract.
        //
        // On mismatch the loop short-circuits to
        // `ToolOutcome::Failed` with a human-readable detail, which
        // the planner observes via `observe_tool_outcome` exactly
        // like any other tool failure. We deliberately do NOT route
        // through the deny-scope path: structural malformation is a
        // *planner* bug (or prompt-injection attempt), not a
        // capability question, and routing it through `Denied`
        // would pollute the scope-denial telemetry stream.
        if let Err(err) = crate::schema::validate(tool.input_schema(), &input) {
            let outcome = ToolOutcome::Failed(AivyxError::Tool {
                tool: tool_id,
                detail: format!("input validation failed: {err}"),
            });
            return (
                StepObservation {
                    tool_id,
                    summary: ToolOutcomeSummary::Failed,
                },
                outcome,
            );
        }

        // Phase 8 Task 2 — session partition injection. Channels that
        // want per-instance memory isolation (Telegram: one chat = one
        // partition) override `ChannelContext::session_partition`. The
        // turn loop threads that partition into the tool's JSON input
        // under a reserved `"session"` key *before* `required_scope`
        // runs, so session-scoped tools (memory.read/write/forget)
        // derive a `session:<partition>` qualifier that the capability
        // check enforces. The LLM never sees this field — it is not
        // in any advertised `input_schema` and is added after the
        // planner emits the call. Non-object inputs (unlikely — all
        // current tools take object inputs) are left untouched.
        let mut input = input;
        if let Some(partition) = channel.session_partition()
            && let Some(obj) = input.as_object_mut()
        {
            obj.insert("session".to_string(), serde_json::Value::String(partition));
        }

        let needed: Scope = tool.required_scope(&input);

        if !effective.grants(&needed) {
            // D4: scope denial emits a `ScopeDenied` audit event carrying
            // the held snapshot. The planner observes a `Denied` summary
            // via the observation trail and a full `Denied { scope, held }`
            // outcome via `observe_tool_outcome`.
            self.audit.on_event(AuditTag::ScopeDenied {
                turn_id,
                tool_attempted: tool_id,
                scope_requested: needed.clone(),
                held_capabilities: effective.clone(),
            });
            let outcome = ToolOutcome::Denied {
                scope: needed,
                held: effective.clone(),
            };
            return (
                StepObservation {
                    tool_id,
                    summary: ToolOutcomeSummary::Denied,
                },
                outcome,
            );
        }

        let ctx = ToolContext {
            agent_id: self.id,
            session_id: channel.session_id(),
            turn_id,
            channel,
            audit: self.audit.as_ref(),
            cancellation,
        };

        let input_bytes = serde_json::to_vec(&input).unwrap_or_default();
        let input_hash = sha256_array(&input_bytes);

        let step_start = Instant::now();
        let outcome = tool.execute(input, &ctx).await;
        let step_duration = step_start.elapsed();

        let summary = ToolOutcomeSummary::from(&outcome);

        self.audit.on_event(AuditTag::ToolCall {
            turn_id,
            tool_id,
            scope_used: needed,
            input_hash,
            outcome: summary.clone(),
            duration: step_duration,
        });

        (StepObservation { tool_id, summary }, outcome)
    }
}

fn sha256_array(bytes: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(bytes);
    let out = h.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    arr
}

// ---------------------------------------------------------------------------
// Tests — the first end-to-end turn-loop run.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use serde_json::{json, Value};

    use aivyx_capability::TrustTier;

    use crate::{
        AuditTag, ChannelError, ChannelPlatform, NullAuditHook, SessionId, StreamEvent, Tool,
        ToolOutcome, Verification,
    };

    // ---- Test fakes ----

    /// Records every audit event the loop emits so tests can assert on
    /// the exact sequence. Wraps `AuditTag` rather than re-serializing
    /// through `aivyx_audit` because the HMAC-chain property is already
    /// tested in that crate — here we're testing the *loop's* behavior.
    #[derive(Default)]
    struct RecordingAudit {
        events: Mutex<Vec<AuditTag>>,
    }

    impl RecordingAudit {
        fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }
        fn snapshot(&self) -> Vec<AuditTag> {
            self.events.lock().unwrap().clone()
        }
    }

    impl AuditHook for RecordingAudit {
        fn on_event(&self, tag: AuditTag) {
            self.events.lock().unwrap().push(tag);
        }
    }

    /// Fake channel. Records stream events and finalize calls so tests
    /// can assert the loop talked to the channel in the right order.
    struct FakeChannel {
        session: SessionId,
        platform: ChannelPlatform,
        tier: TrustTier,
        token: CancellationToken,
        finalized: Mutex<Option<TurnOutcomeSummary>>,
        stream_calls: Mutex<usize>,
    }

    impl FakeChannel {
        fn new(platform: ChannelPlatform, tier: TrustTier) -> Self {
            FakeChannel {
                session: SessionId::new(),
                platform,
                tier,
                token: CancellationToken::new(),
                finalized: Mutex::new(None),
                stream_calls: Mutex::new(0),
            }
        }
    }

    #[async_trait]
    impl ChannelContext for FakeChannel {
        fn channel_name(&self) -> &str {
            "fake"
        }
        fn platform(&self) -> ChannelPlatform {
            self.platform
        }
        fn trust_tier(&self) -> TrustTier {
            self.tier
        }
        fn session_id(&self) -> SessionId {
            self.session
        }
        async fn stream_event(&self, _event: StreamEvent<'_>) -> Result<(), ChannelError> {
            *self.stream_calls.lock().unwrap() += 1;
            Ok(())
        }
        async fn finalize(&self, outcome: &TurnOutcome) -> Result<(), ChannelError> {
            *self.finalized.lock().unwrap() = Some(TurnOutcomeSummary::from(outcome));
            Ok(())
        }
        fn cancellation_token(&self) -> CancellationToken {
            self.token.clone()
        }
    }

    /// A fake tool that always completes successfully. `required_scope`
    /// is parameterized so tests can construct tools that need either a
    /// bare or qualified scope.
    struct FakeTool {
        id: ToolId,
        name: &'static str,
        schema: Value,
        scope_fn: Box<dyn Fn(&Value) -> Scope + Send + Sync>,
    }

    impl FakeTool {
        fn new_bare(name: &'static str, scope: &str) -> Self {
            let s = Scope::parse(scope).unwrap();
            FakeTool {
                id: ToolId::new(),
                name,
                schema: json!({}),
                scope_fn: Box::new(move |_| s.clone()),
            }
        }
        fn new_r1(
            name: &'static str,
            f: impl Fn(&Value) -> Scope + Send + Sync + 'static,
        ) -> Self {
            FakeTool {
                id: ToolId::new(),
                name,
                schema: json!({}),
                scope_fn: Box::new(f),
            }
        }

        /// Task-2 helper: build a FakeTool whose input_schema is a
        /// real JSON-Schema fragment. `scope_fn` is still invoked
        /// for well-formed inputs; callers that expect validation
        /// to short-circuit before `scope_fn` can plant a panicking
        /// closure there to prove the validator fired first.
        fn new_with_schema(
            name: &'static str,
            schema: Value,
            f: impl Fn(&Value) -> Scope + Send + Sync + 'static,
        ) -> Self {
            FakeTool {
                id: ToolId::new(),
                name,
                schema,
                scope_fn: Box::new(f),
            }
        }
    }

    #[async_trait]
    impl Tool for FakeTool {
        fn id(&self) -> ToolId {
            self.id
        }
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "fake"
        }
        fn input_schema(&self) -> &Value {
            &self.schema
        }
        fn required_scope(&self, input: &Value) -> Scope {
            (self.scope_fn)(input)
        }
        async fn execute(&self, _input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
            ToolOutcome::Completed {
                output: json!({"ok": true}),
                verified: Verification::NotApplicable,
            }
        }
    }

    fn make_agent(
        caps: CapabilitySet,
        tools: Vec<Arc<dyn Tool>>,
        audit: Arc<dyn AuditHook>,
        plan: Vec<NextStep>,
    ) -> ConcreteAgent {
        let registry = Arc::new(ToolRegistry::new(tools));
        let plan_arc = Arc::new(plan);
        ConcreteAgent::new(AgentId::new(), caps, registry, audit, move || {
            Box::new(crate::planner::VecPlanner::new((*plan_arc).clone()))
        })
    }

    // ---- Golden path: one tool call, final message, clean completion ----

    #[tokio::test]
    async fn golden_path_completes_with_correct_audit_trail() {
        let audit = RecordingAudit::new();

        let tool = Arc::new(FakeTool::new_bare("memory.read", "memory.read"));
        let tool_id = tool.id();

        let agent_caps =
            CapabilitySet::from_scopes([Scope::parse("memory.read").unwrap()]);

        let plan = vec![
            NextStep::ToolCall {
                tool_id,
                input: json!({"query": "yesterday"}),
            },
            NextStep::FinalMessage("here's what I found".to_string()),
        ];

        let agent = make_agent(agent_caps, vec![tool], audit.clone(), plan);

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let message = Message::text(channel.session, "what did I work on yesterday?");
        let outcome = agent.turn(message, &channel).await;

        // Return-value shape
        match outcome {
            TurnOutcome::Completed {
                final_message,
                tool_calls_made,
                ..
            } => {
                assert_eq!(final_message, "here's what I found");
                assert_eq!(tool_calls_made, 1);
            }
            other => panic!("expected Completed, got {other:?}"),
        }

        // Audit trail: exactly TurnStarted → ToolCall → TurnEnded
        let events = audit.snapshot();
        assert_eq!(events.len(), 3);
        assert!(matches!(events[0], AuditTag::TurnStarted { .. }));
        assert!(matches!(events[1], AuditTag::ToolCall { .. }));
        assert!(matches!(events[2], AuditTag::TurnEnded { .. }));

        // Channel was finalized exactly once with a Completed summary
        assert_eq!(
            *channel.finalized.lock().unwrap(),
            Some(TurnOutcomeSummary::Completed)
        );
    }

    // ---- Scope denied: Tier 2 + shell.exec → ScopeDenied in trail ----

    #[tokio::test]
    async fn scope_denied_emits_scope_denied_audit_not_tool_call() {
        let audit = RecordingAudit::new();

        let tool = Arc::new(FakeTool::new_r1("shell.exec", |input| {
            let cmd = input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            Scope::parse(&format!("shell.exec:{cmd}")).unwrap()
        }));
        let tool_id = tool.id();

        // Agent nominally holds shell.exec, but the Tier 2 ceiling strips
        // it — that's the whole point of D5 Scenario 3.
        let agent_caps =
            CapabilitySet::from_scopes([Scope::parse("shell.exec").unwrap()]);

        let plan = vec![NextStep::ToolCall {
            tool_id,
            input: json!({"command": "rm"}),
        }];

        let agent = make_agent(agent_caps, vec![tool], audit.clone(), plan);

        let channel = FakeChannel::new(ChannelPlatform::Telegram, TrustTier::SemiTrusted);
        let message = Message::text(channel.session, "run rm -rf from Telegram");
        let outcome = agent.turn(message, &channel).await;

        // Denial is not a termination — the turn still Completes, just with
        // tool_calls_made reflecting the attempted call.
        match outcome {
            TurnOutcome::Completed {
                tool_calls_made, ..
            } => assert_eq!(tool_calls_made, 1),
            other => panic!("expected Completed (denial is not termination), got {other:?}"),
        }

        // Audit trail: TurnStarted → ScopeDenied → TurnEnded
        let events = audit.snapshot();
        assert_eq!(events.len(), 3);
        match &events[1] {
            AuditTag::ScopeDenied {
                scope_requested,
                held_capabilities,
                ..
            } => {
                assert_eq!(scope_requested.base(), "shell.exec");
                assert_eq!(scope_requested.qualifier(), Some("rm"));
                // Tier 2 ceiling has no shell.exec at all, so intersected
                // held set does not grant shell.exec in any form.
                assert!(!held_capabilities.grants(&Scope::parse("shell.exec").unwrap()));
                assert!(!held_capabilities.grants(&Scope::parse("shell.exec:rm").unwrap()));
            }
            other => panic!("expected ScopeDenied at index 1, got {other:?}"),
        }

        // No ToolCall event — the tool never ran.
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, AuditTag::ToolCall { .. })),
            "no ToolCall event should appear for a denied call"
        );
    }

    // ---- Cancellation before the loop body runs ----

    #[tokio::test]
    async fn cancellation_before_first_step_yields_cancelled() {
        let audit = RecordingAudit::new();
        let tool = Arc::new(FakeTool::new_bare("memory.read", "memory.read"));
        let tool_id = tool.id();

        let plan = vec![NextStep::ToolCall {
            tool_id,
            input: json!({}),
        }];
        let agent = make_agent(
            CapabilitySet::from_scopes([Scope::parse("memory.read").unwrap()]),
            vec![tool],
            audit.clone(),
            plan,
        );

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        channel.token.cancel(); // cancel BEFORE turn() is called

        let message = Message::text(channel.session, "hi");
        let outcome = agent.turn(message, &channel).await;

        match outcome {
            TurnOutcome::Cancelled { tool_calls_made } => assert_eq!(tool_calls_made, 0),
            other => panic!("expected Cancelled, got {other:?}"),
        }

        let events = audit.snapshot();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], AuditTag::TurnStarted { .. }));
        assert!(matches!(
            events[1],
            AuditTag::TurnEnded {
                outcome: TurnOutcomeSummary::Cancelled,
                tool_calls_made: 0,
                ..
            }
        ));
    }

    // ---- R1 interaction: derived scope satisfied by bare capability ----

    #[tokio::test]
    async fn r1_derived_scope_recorded_in_audit_not_bare_scope() {
        let audit = RecordingAudit::new();

        // R1 tool: derives memory.read:session:abc from input
        let tool = Arc::new(FakeTool::new_r1("memory.read", |input| {
            let sid = input
                .get("session")
                .and_then(|v| v.as_str())
                .unwrap_or("any");
            Scope::parse(&format!("memory.read:session:{sid}")).unwrap()
        }));
        let tool_id = tool.id();

        // Agent holds *bare* memory.read — rule 2 (unqualified grants
        // qualified) lets the derived scope pass the check.
        let agent_caps =
            CapabilitySet::from_scopes([Scope::parse("memory.read").unwrap()]);

        let plan = vec![
            NextStep::ToolCall {
                tool_id,
                input: json!({"session": "abc"}),
            },
            NextStep::FinalMessage("done".to_string()),
        ];

        let agent = make_agent(agent_caps, vec![tool], audit.clone(), plan);
        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let message = Message::text(channel.session, "recall session abc");
        let outcome = agent.turn(message, &channel).await;

        assert!(matches!(outcome, TurnOutcome::Completed { .. }));

        let events = audit.snapshot();
        let tool_call = events
            .iter()
            .find_map(|e| match e {
                AuditTag::ToolCall { scope_used, .. } => Some(scope_used.clone()),
                _ => None,
            })
            .expect("expected a ToolCall audit event");

        assert_eq!(tool_call.base(), "memory.read");
        assert_eq!(
            tool_call.qualifier(),
            Some("session:abc"),
            "audit must record the *derived* scope, not the agent's bare capability"
        );
    }

    // ---- Parent-level: D1 Scenario 3 end-to-end ----

    #[tokio::test]
    async fn d1_scenario3_rm_rf_from_telegram_e2e() {
        // A dress rehearsal of the whole stack: channel → loop → scope
        // check → audit emission → finalize. If this test passes, Phase 1
        // has delivered the "structural proof that Phase 0 contract can
        // carry real execution" that the phase is actually about.
        let audit = RecordingAudit::new();

        let shell = Arc::new(FakeTool::new_r1("shell.exec", |input| {
            let cmd = input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            Scope::parse(&format!("shell.exec:{cmd}")).unwrap()
        }));
        let shell_id = shell.id();

        let agent = make_agent(
            CapabilitySet::from_scopes([Scope::parse("shell.exec").unwrap()]),
            vec![shell],
            audit.clone(),
            vec![
                NextStep::ToolCall {
                    tool_id: shell_id,
                    input: json!({"command": "rm"}),
                },
                NextStep::FinalMessage(
                    "I can't run shell commands from Telegram.".to_string(),
                ),
            ],
        );

        let channel = FakeChannel::new(ChannelPlatform::Telegram, TrustTier::SemiTrusted);
        let message = Message::text(channel.session, "run rm -rf /");
        let outcome = agent.turn(message, &channel).await;

        // The turn still completes — denial isn't termination.
        match outcome {
            TurnOutcome::Completed {
                tool_calls_made,
                final_message,
                ..
            } => {
                assert_eq!(tool_calls_made, 1);
                assert!(final_message.contains("can't run shell"));
            }
            other => panic!("expected Completed, got {other:?}"),
        }

        // Audit trail has the Denied event with the right details.
        let events = audit.snapshot();
        assert_eq!(events.len(), 3);
        assert!(matches!(events[0], AuditTag::TurnStarted { channel: ChannelPlatform::Telegram, trust_tier: TrustTier::SemiTrusted, .. }));
        assert!(matches!(events[1], AuditTag::ScopeDenied { .. }));
        assert!(matches!(events[2], AuditTag::TurnEnded { outcome: TurnOutcomeSummary::Completed, tool_calls_made: 1, .. }));

        // Channel was finalized.
        assert_eq!(
            *channel.finalized.lock().unwrap(),
            Some(TurnOutcomeSummary::Completed)
        );
    }

    // ---- NullAuditHook end-to-end (sanity check that the hook abstraction
    //      does not silently swallow anything the RecordingAudit tests were
    //      catching) ----

    #[tokio::test]
    async fn null_audit_hook_end_to_end_still_produces_correct_outcome() {
        let tool = Arc::new(FakeTool::new_bare("memory.read", "memory.read"));
        let tool_id = tool.id();
        let agent = make_agent(
            CapabilitySet::from_scopes([Scope::parse("memory.read").unwrap()]),
            vec![tool],
            Arc::new(NullAuditHook),
            vec![
                NextStep::ToolCall {
                    tool_id,
                    input: json!({}),
                },
                NextStep::Stop,
            ],
        );
        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let outcome = agent
            .turn(Message::text(channel.session, "test"), &channel)
            .await;
        match outcome {
            TurnOutcome::Completed {
                tool_calls_made, ..
            } => assert_eq!(tool_calls_made, 1),
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    // ---- Wall-clock timeout: deadline task fires → TimedOut outcome ----
    //
    // Phase 3 task 4 introduced `TURN_TIMEOUT` and the deadline task.
    // This test proves the loop actually emits `TurnOutcome::TimedOut`
    // (the first code path in the project to do so) when the planner
    // hangs past the deadline. Uses tokio's virtual-time test-util so
    // the test doesn't actually wait 120s wall-clock.

    /// Planner whose `next_step` awaits forever. The only way a turn
    /// using it can terminate is the loop's own cancellation re-check
    /// after the deadline task fires.
    struct HangingPlanner;

    #[async_trait]
    impl TurnPlanner for HangingPlanner {
        async fn begin_turn(&mut self, _message: &Message) {}

        async fn next_step(
            &mut self,
            _observed: &[StepObservation],
            channel: &dyn ChannelContext,
        ) -> NextStep {
            // Mirror the real LLM planner's one_step: race the
            // channel's cancellation against a future that never
            // completes. When the deadline task cancels the token,
            // this branch wins and we surface `Stop` — which the
            // loop then translates to `TimedOut` via its post-
            // next_step cancellation re-check + `deadline_fired` flag.
            let cancel = channel.cancellation_token();
            tokio::select! {
                biased;
                _ = cancel.cancelled() => NextStep::Stop,
                _ = std::future::pending::<()>() => unreachable!(),
            }
        }
    }

    #[tokio::test(start_paused = true)]
    async fn wall_clock_timeout_emits_timed_out_outcome() {
        let audit = RecordingAudit::new();
        let registry = Arc::new(ToolRegistry::new(Vec::new()));
        let agent = ConcreteAgent::new(
            AgentId::new(),
            CapabilitySet::empty(),
            registry,
            audit.clone(),
            || Box::new(HangingPlanner),
        );

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let message = Message::text(channel.session, "hang forever");

        // Race the turn against a virtual-time advance that walks past
        // the deadline. `start_paused = true` pauses the tokio clock
        // at t=0; the advance hops directly to t > TURN_TIMEOUT so the
        // deadline task wakes up immediately in wall-clock terms.
        let turn_fut = agent.turn(message, &channel);
        let advance_fut = async {
            // Yield so the turn task actually starts and spawns the
            // deadline task before we advance time past its sleep.
            tokio::task::yield_now().await;
            tokio::time::advance(TURN_TIMEOUT + Duration::from_secs(1)).await;
        };
        let (outcome, _) = tokio::join!(turn_fut, advance_fut);

        match outcome {
            TurnOutcome::TimedOut {
                tool_calls_made,
                elapsed: _,
            } => {
                // We don't assert on `elapsed` because the loop
                // measures it with `std::time::Instant`, which is not
                // virtualized by `tokio::time::pause()`. In production
                // the field is meaningful; in this test it will be
                // ~microseconds. Asserting outcome variant + no tools
                // called is sufficient to prove the deadline path.
                assert_eq!(tool_calls_made, 0);
            }
            other => panic!("expected TimedOut, got {other:?}"),
        }

        // Audit: TurnStarted → TurnEnded(TimedOut), nothing in between.
        let events = audit.snapshot();
        assert_eq!(events.len(), 2);
        assert!(matches!(events[0], AuditTag::TurnStarted { .. }));
        assert!(matches!(
            events[1],
            AuditTag::TurnEnded {
                outcome: TurnOutcomeSummary::TimedOut,
                ..
            }
        ));
    }

    // ---- Max-steps guard: a runaway planner is terminated with Failed ----
    //
    // Phase 2 introduced MAX_STEPS_PER_TURN = 32 so an LLM-backed planner
    // that never emits FinalMessage cannot loop forever. This test proves
    // the guard fires by feeding the loop a script that's longer than the
    // budget — 64 ToolCalls, no FinalMessage — and asserting the loop
    // terminates with Failed(Internal) rather than running to completion.

    #[tokio::test]
    async fn runaway_planner_terminates_with_max_steps_exceeded() {
        let audit = RecordingAudit::new();
        let tool = Arc::new(FakeTool::new_bare("memory.read", "memory.read"));
        let tool_id = tool.id();

        // Build a script of 64 ToolCalls with no FinalMessage. The
        // budget is MAX_STEPS_PER_TURN; anything past the budget should
        // never run.
        let plan: Vec<NextStep> = (0..64)
            .map(|_| NextStep::ToolCall {
                tool_id,
                input: json!({}),
            })
            .collect();

        let agent = make_agent(
            CapabilitySet::from_scopes([Scope::parse("memory.read").unwrap()]),
            vec![tool],
            audit.clone(),
            plan,
        );

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let outcome = agent
            .turn(Message::text(channel.session, "spam"), &channel)
            .await;

        match outcome {
            TurnOutcome::Failed(AivyxError::Internal(msg)) => {
                assert!(
                    msg.contains("exceeded"),
                    "expected 'exceeded' in error, got {msg:?}"
                );
            }
            other => panic!("expected Failed(Internal), got {other:?}"),
        }

        // The loop should have called exactly MAX_STEPS_PER_TURN tools
        // before bailing — one tool per step, no shortcut.
        let tool_calls: Vec<_> = audit
            .snapshot()
            .into_iter()
            .filter(|e| matches!(e, AuditTag::ToolCall { .. }))
            .collect();
        assert_eq!(tool_calls.len(), MAX_STEPS_PER_TURN);

        // TurnEnded should record the Failed summary.
        let events = audit.snapshot();
        let ended = events
            .iter()
            .find(|e| matches!(e, AuditTag::TurnEnded { .. }))
            .expect("TurnEnded should still be emitted");
        match ended {
            AuditTag::TurnEnded { outcome, .. } => {
                assert_eq!(*outcome, TurnOutcomeSummary::Failed);
            }
            _ => unreachable!(),
        }
    }

    // ---- Phase 10 task 2: JSON-schema validation at the turn loop ----
    //
    // The validator has its own unit tests in `schema.rs`; these two
    // tests prove the *wiring*: the turn loop runs validation before
    // `required_scope`, a failure short-circuits to `Failed` without
    // invoking `scope_fn` or `execute`, and a well-formed input
    // passes through unchanged.

    #[tokio::test]
    async fn malformed_tool_input_is_rejected_before_required_scope() {
        // The FakeTool has a schema requiring a string `path` field.
        // The planner emits an input missing `path`. If validation
        // works, `scope_fn` is never called; it's set to panic so a
        // regression (validator removed or moved after scope check)
        // would panic loudly rather than silently accept.
        let audit = RecordingAudit::new();

        let schema = json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": ["path"]
        });
        let tool = Arc::new(FakeTool::new_with_schema("fs.read", schema, |_| {
            panic!("required_scope must not run when validation fails");
        }));
        let tool_id = tool.id();

        // Cap grant exists, so the test isolates the effect of
        // validation from the scope-denial path. If the validator
        // were missing, this call would reach `scope_fn` and panic
        // — which is exactly the regression this test locks in.
        let agent_caps = CapabilitySet::from_scopes([Scope::parse("fs.read").unwrap()]);

        let plan = vec![NextStep::ToolCall {
            tool_id,
            input: json!({}), // missing required `path`
        }];

        let agent = make_agent(agent_caps, vec![tool], audit.clone(), plan);

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let message = Message::text(channel.session, "bad call");
        let outcome = agent.turn(message, &channel).await;

        // The turn still Completes — validation failure is a Failed
        // step, not a turn-ending error. Tool_calls_made == 1 because
        // the loop did attempt the call, just not reach execute.
        match outcome {
            TurnOutcome::Completed {
                tool_calls_made, ..
            } => assert_eq!(tool_calls_made, 1),
            other => panic!("expected Completed (validation fail is not termination), got {other:?}"),
        }

        // The audit trail must contain NO ToolCall event and NO
        // ScopeDenied event — validation fails before either runs.
        // It also must not contain a panic trace; if the panic
        // fired we'd never have reached this assertion.
        let events = audit.snapshot();
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, AuditTag::ToolCall { .. })),
            "validation failure must not emit ToolCall"
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, AuditTag::ScopeDenied { .. })),
            "validation failure must route through Failed, not Denied"
        );
    }

    #[tokio::test]
    async fn well_formed_tool_input_passes_validation_and_runs() {
        // Positive case: schema-valid input reaches execute as
        // before, proving validation isn't over-rejecting. This is
        // the counterpart to the negative test above — without it,
        // a buggy validator that rejected *everything* would still
        // pass the negative assertion.
        let audit = RecordingAudit::new();

        let schema = json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": ["path"]
        });
        let tool = Arc::new(FakeTool::new_with_schema("fs.read", schema, |_| {
            Scope::parse("fs.read").unwrap()
        }));
        let tool_id = tool.id();

        let agent_caps = CapabilitySet::from_scopes([Scope::parse("fs.read").unwrap()]);

        let plan = vec![NextStep::ToolCall {
            tool_id,
            input: json!({"path": "notes/today.md"}),
        }];

        let agent = make_agent(agent_caps, vec![tool], audit.clone(), plan);

        let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
        let message = Message::text(channel.session, "good call");
        let outcome = agent.turn(message, &channel).await;

        match outcome {
            TurnOutcome::Completed {
                tool_calls_made, ..
            } => assert_eq!(tool_calls_made, 1),
            other => panic!("expected Completed, got {other:?}"),
        }

        // A valid input reaches execute → ToolCall is recorded.
        let events = audit.snapshot();
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AuditTag::ToolCall { .. })),
            "valid input must reach ToolCall"
        );
    }
}
