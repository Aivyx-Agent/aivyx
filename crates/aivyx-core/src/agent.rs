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

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use aivyx_capability::{CapabilitySet, Scope};

use crate::{
    Agent, AgentId, AuditHook, AuditTag, CancellationToken, ChannelContext, Message, ToolContext,
    ToolId, ToolOutcomeSummary, TurnId, TurnOutcome, TurnOutcomeSummary,
};
use crate::planner::{NextStep, StepObservation, ToolRegistry, TurnPlanner};

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

        // Silence the unused-message warning: we don't inspect the message
        // contents in Phase 1, but the type is part of the D3 signature and
        // Phase 2's LLM planner will read it. Drop it explicitly so the
        // name is visible in the loop body's scope for future use.
        let _ = message;

        let mut planner = (self.planner_factory)();
        let mut observed: Vec<StepObservation> = Vec::new();
        let mut tool_calls_made: usize = 0;
        let mut final_message: String = String::new();
        let loop_outcome: LoopOutcome;

        loop {
            if cancellation.is_cancelled() {
                loop_outcome = LoopOutcome::Cancelled;
                break;
            }

            let step = planner.next_step(&observed).await;
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
                    let observation = self
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
                }
            }
        }

        let duration = start.elapsed();
        let outcome = match loop_outcome {
            LoopOutcome::Completed => TurnOutcome::Completed {
                final_message,
                tool_calls_made,
                duration,
            },
            LoopOutcome::Cancelled => TurnOutcome::Cancelled { tool_calls_made },
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
/// `TurnOutcome`. Phase 1 only produces `Completed` and `Cancelled`;
/// `TimedOut`, `Escalated`, and `Failed` will join this enum as the loop
/// grows in later tasks.
enum LoopOutcome {
    Completed,
    Cancelled,
}

impl ConcreteAgent {
    /// Execute one tool call: resolve the tool, compute its required
    /// scope via R1, scope-check, execute-or-deny, emit the matching
    /// audit event, return what the planner will observe.
    async fn run_tool_call(
        &self,
        turn_id: TurnId,
        tool_id: ToolId,
        input: serde_json::Value,
        channel: &dyn ChannelContext,
        cancellation: &CancellationToken,
        effective: &CapabilitySet,
    ) -> StepObservation {
        let Some(tool) = self.tools.get(tool_id) else {
            // Unknown tool — no scope check possible. This shouldn't happen
            // with a well-behaved planner; treat it as a failed step and
            // let the planner observe a Failed summary.
            return StepObservation {
                tool_id,
                summary: ToolOutcomeSummary::Failed,
            };
        };

        let needed: Scope = tool.required_scope(&input);

        if !effective.grants(&needed) {
            // D4: scope denial emits a `ScopeDenied` audit event carrying
            // the held snapshot. The planner observes a `Denied` summary.
            self.audit.on_event(AuditTag::ScopeDenied {
                turn_id,
                tool_attempted: tool_id,
                scope_requested: needed,
                held_capabilities: effective.clone(),
            });
            return StepObservation {
                tool_id,
                summary: ToolOutcomeSummary::Denied,
            };
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

        // Consume the full outcome to avoid unused-variable warnings on
        // the `output` field; Phase 2's LLM planner will read it.
        let _ = outcome;

        StepObservation { tool_id, summary }
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
}
