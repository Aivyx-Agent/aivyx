//! Phase 2 Task 1 — bridge integration test.
//!
//! Drives a real `ConcreteAgent` turn loop through the `AuditBridge` into
//! a real `HmacChainLog`. If this passes, the forward-declared `AuditHook`
//! in `aivyx-core` is successfully satisfied by the real chain writer in
//! `aivyx-audit` — closing the loop that Phase 1 deliberately left open.
//!
//! This is an integration test (not a unit test) because it exercises the
//! bridge strictly through the public surface of both crates. If it ever
//! needs a `#[cfg(test)]`-only item from either side, the bridge's public
//! API is broken and that's the signal.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_audit::{AuditBridge, AuditEvent, AuditLog, HmacChainLog};
use aivyx_capability::{CapabilitySet, Scope, TrustTier};
use aivyx_core::{
    agent::ConcreteAgent,
    planner::{NextStep, ToolRegistry, VecPlanner},
    Agent, AgentId, CancellationToken, ChannelContext, ChannelError, ChannelPlatform, Message,
    SessionId, StreamEvent, Tool, ToolContext, ToolId, ToolOutcome, TurnOutcome,
    TurnOutcomeSummary, Verification,
};

// ---------------------------------------------------------------------------
// Minimal fakes. Deliberately thinner than the ones inside
// aivyx-core/src/agent.rs::tests — this test only needs the shapes the
// bridge touches, not the full D1-scenario coverage.
// ---------------------------------------------------------------------------

struct FakeChannel {
    session: SessionId,
    platform: ChannelPlatform,
    tier: TrustTier,
    token: CancellationToken,
}

impl FakeChannel {
    fn new(platform: ChannelPlatform, tier: TrustTier) -> Self {
        FakeChannel {
            session: SessionId::new(),
            platform,
            tier,
            token: CancellationToken::new(),
        }
    }
}

#[async_trait]
impl ChannelContext for FakeChannel {
    fn channel_name(&self) -> &str {
        "bridge-integration"
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
        Ok(())
    }
    async fn finalize(&self, _outcome: &TurnOutcome) -> Result<(), ChannelError> {
        Ok(())
    }
    fn cancellation_token(&self) -> CancellationToken {
        self.token.clone()
    }
}

struct FakeMemoryTool {
    id: ToolId,
    schema: Value,
}

impl FakeMemoryTool {
    fn new() -> Self {
        FakeMemoryTool {
            id: ToolId::new(),
            schema: json!({}),
        }
    }
}

#[async_trait]
impl Tool for FakeMemoryTool {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "memory.read"
    }
    fn description(&self) -> &str {
        "fake memory tool"
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("memory.read").unwrap()
    }
    async fn execute(&self, _input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        ToolOutcome::Completed {
            output: json!({"ok": true}),
            verified: Verification::NotApplicable,
        }
    }
}

// ---------------------------------------------------------------------------
// The test: a turn loop → bridge → HmacChainLog, verified end-to-end.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn concrete_agent_writes_real_hmac_chain_through_bridge() {
    // Real chain, real key. Wrapped in the default bridge (panic-on-error).
    let log = HmacChainLog::new(b"phase2-task1-integration-key".to_vec());
    let bridge: Arc<AuditBridge<HmacChainLog>> = Arc::new(AuditBridge::new(log));

    // Golden-path agent: one memory.read tool call, then a final message.
    let tool = Arc::new(FakeMemoryTool::new());
    let tool_id = tool.id();
    let registry = Arc::new(ToolRegistry::new(vec![tool as Arc<dyn Tool>]));
    let caps = CapabilitySet::from_scopes([Scope::parse("memory.read").unwrap()]);

    let plan = vec![
        NextStep::ToolCall {
            tool_id,
            input: json!({"query": "yesterday"}),
            auto_corrected_from: None,
            extracted_from_text: None,
        },
        NextStep::FinalMessage("here is what I found".to_string()),
    ];
    let plan_arc = Arc::new(plan);

    let bridge_hook = Arc::clone(&bridge);
    let agent = ConcreteAgent::new(
        AgentId::new(),
        caps,
        registry,
        bridge_hook, // Arc<AuditBridge<HmacChainLog>> coerces to Arc<dyn AuditHook>
        move || Box::new(VecPlanner::new((*plan_arc).clone())),
    );

    let channel = FakeChannel::new(ChannelPlatform::Local, TrustTier::Trusted);
    let outcome = agent
        .turn(Message::text(channel.session, "recall"), &channel)
        .await;

    // Turn returned Completed.
    assert!(
        matches!(outcome, TurnOutcome::Completed { .. }),
        "expected Completed, got {outcome:?}"
    );

    // The chain inside the bridge has exactly 3 entries: TurnStarted,
    // ToolCall, TurnEnded. The whole point of this test is that those
    // entries came from the real turn loop via the real bridge — no
    // test-local AuditHook in the path.
    let log = bridge.writer();
    assert_eq!(log.len(), 3, "expected 3 chain entries, got {}", log.len());

    // And the chain verifies — the HMAC links are intact across three
    // appends, proving the bridge did not corrupt the canonical bytes.
    log.verify().expect("HMAC chain must verify after a real turn");

    // Spot-check each entry's variant, so a future regression that
    // miswires the bridge (e.g. sends every event as ToolCall) fails loud.
    match log.get(0).unwrap().event {
        AuditEvent::TurnStarted {
            channel: ChannelPlatform::Local,
            ..
        } => {}
        ref other => panic!("seq 0: expected TurnStarted on Local, got {other:?}"),
    }
    match log.get(1).unwrap().event {
        AuditEvent::ToolCall { .. } => {}
        ref other => panic!("seq 1: expected ToolCall, got {other:?}"),
    }
    match log.get(2).unwrap().event {
        AuditEvent::TurnEnded {
            outcome: TurnOutcomeSummary::Completed,
            tool_calls_made: 1,
            ..
        } => {}
        ref other => panic!("seq 2: expected TurnEnded(Completed, 1), got {other:?}"),
    }
}
