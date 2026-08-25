//! `send_message` / `read_message` — the agent interface to the team
//! [`MessageBus`] (J.3.2). Both require the **`team.message`** scope, which
//! every member holds (lead + specialists), so peers can talk.
//!
//! Sending honours two [`DialogueConfig`](crate::config::DialogueConfig)
//! caps: `enable_peer_dialogue` (when off, only the lead may send) and
//! `max_messages_per_turn` (a per-turn send budget; the runtime calls
//! [`SendMessageTool::reset_turn`] at each turn boundary in J.4+).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::config::DialogueConfig;
use crate::message_bus::{MessageBus, Recipient, Subscription, TeamMessage};

const MESSAGE_SCOPE: &str = "team.message";

fn scope() -> Scope {
    Scope::parse(MESSAGE_SCOPE).expect("team.message must be a known base")
}

fn fail(id: ToolId, detail: impl Into<String>) -> ToolOutcome {
    ToolOutcome::Failed(AivyxError::Tool {
        tool: id,
        detail: detail.into(),
    })
}

/// `send_message` — post to a teammate or the whole team.
pub struct SendMessageTool {
    id: ToolId,
    agent: String,
    bus: Arc<MessageBus>,
    enable_peer_dialogue: bool,
    is_lead: bool,
    max_per_turn: u32,
    sent_this_turn: AtomicU32,
    schema: Value,
}

impl SendMessageTool {
    pub fn new(
        agent: impl Into<String>,
        bus: Arc<MessageBus>,
        dialogue: &DialogueConfig,
        is_lead: bool,
    ) -> Self {
        SendMessageTool {
            id: ToolId::new(),
            agent: agent.into(),
            bus,
            enable_peer_dialogue: dialogue.enable_peer_dialogue,
            is_lead,
            max_per_turn: dialogue.max_messages_per_turn,
            sent_this_turn: AtomicU32::new(0),
            schema: json!({
                "type": "object",
                "properties": {
                    "to": { "type": "string", "description": "Recipient member; omit to broadcast to the team." },
                    "content": { "type": "string" }
                },
                "required": ["content"],
                "additionalProperties": false
            }),
        }
    }

    /// Reset the per-turn send budget. The team runtime calls this at each
    /// turn boundary (J.4+).
    pub fn reset_turn(&self) {
        self.sent_this_turn.store(0, Ordering::Relaxed);
    }
}

#[async_trait]
impl Tool for SendMessageTool {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "send_message"
    }
    fn description(&self) -> &str {
        "Send a message to a teammate, or broadcast to the whole team. Input: \
         { \"to\"?: string (omit to broadcast), \"content\": string }."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _: &Value) -> Scope {
        scope()
    }
    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        if !self.enable_peer_dialogue && !self.is_lead {
            return fail(self.id, "peer dialogue is disabled; only the lead may send messages");
        }
        let Some(content) = input.get("content").and_then(Value::as_str) else {
            return fail(self.id, "`content` (string) is required");
        };
        // Per-turn budget (count before this send).
        if self.sent_this_turn.fetch_add(1, Ordering::Relaxed) >= self.max_per_turn {
            return fail(
                self.id,
                format!("message budget ({}) reached this turn", self.max_per_turn),
            );
        }
        let msg = match input.get("to").and_then(Value::as_str) {
            Some(to) if !to.trim().is_empty() => TeamMessage::direct(&self.agent, to, content),
            _ => TeamMessage::broadcast(&self.agent, content),
        };
        let reached = self.bus.publish(msg);
        ToolOutcome::Completed {
            output: json!({ "sent": true, "reached": reached }),
            verified: Verification::Unverified,
        }
    }
}

/// `read_message` — drain the messages addressed to this member.
pub struct ReadMessagesTool {
    id: ToolId,
    sub: Mutex<Subscription>,
    schema: Value,
}

impl ReadMessagesTool {
    /// Subscribe `agent` to `bus` — only messages sent *after* this call
    /// are delivered, so construct it at team assembly time.
    pub fn new(bus: &MessageBus, agent: &str) -> Self {
        ReadMessagesTool {
            id: ToolId::new(),
            sub: Mutex::new(bus.subscribe(agent)),
            schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
        }
    }
}

#[async_trait]
impl Tool for ReadMessagesTool {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "read_message"
    }
    fn description(&self) -> &str {
        "Read the messages addressed to you (or broadcast to the team) since you last read. \
         No input."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _: &Value) -> Scope {
        scope()
    }
    async fn execute(&self, _input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let drained = self.sub.lock().unwrap().drain();
        let messages: Vec<Value> = drained
            .messages
            .iter()
            .map(|m| {
                let to = match &m.to {
                    Recipient::Broadcast => "team".to_string(),
                    Recipient::Agent(n) => n.clone(),
                };
                json!({ "from": m.from, "to": to, "content": m.content })
            })
            .collect();
        ToolOutcome::Completed {
            output: json!({ "messages": messages, "lagged": drained.lagged }),
            verified: Verification::Unverified,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::FakeLeadChannel;
    use aivyx_capability::TrustTier;
    use aivyx_core::{AgentId, CancellationToken, ChannelContext, NullAuditHook, TurnId};

    // A ToolContext over a fake lead channel (execute ignores most of it).
    macro_rules! ctx {
        ($ch:expr, $audit:expr, $tok:expr) => {
            ToolContext {
                agent_id: AgentId::new(),
                session_id: $ch.session_id(),
                turn_id: TurnId::new(),
                channel: &$ch,
                audit: &$audit,
                cancellation: &$tok,
                message_origin: aivyx_core::MessageOrigin::Operator,
            }
        };
    }

    fn dialogue(peer: bool, max: u32) -> DialogueConfig {
        DialogueConfig {
            enable_peer_dialogue: peer,
            max_messages_per_turn: max,
            ..DialogueConfig::default()
        }
    }

    #[test]
    fn tool_surface() {
        let bus = MessageBus::new(8);
        let send = SendMessageTool::new("a", Arc::clone(&bus), &DialogueConfig::default(), false);
        assert_eq!(send.name(), "send_message");
        assert_eq!(send.required_scope(&Value::Null).base(), "team.message");
        let read = ReadMessagesTool::new(&bus, "a");
        assert_eq!(read.name(), "read_message");
        assert_eq!(read.required_scope(&Value::Null).base(), "team.message");
    }

    #[tokio::test]
    async fn send_then_read_round_trips_a_broadcast() {
        let bus = MessageBus::new(16);
        let send = SendMessageTool::new("lead", Arc::clone(&bus), &dialogue(true, 10), true);
        let read = ReadMessagesTool::new(&bus, "inventory");

        let ch = FakeLeadChannel::at(TrustTier::Trusted);
        let audit = NullAuditHook;
        let tok = CancellationToken::new();

        let s = send.execute(json!({ "content": "stand-up in 5" }), &ctx!(ch, audit, tok)).await;
        assert!(matches!(s, ToolOutcome::Completed { .. }));

        match read.execute(json!({}), &ctx!(ch, audit, tok)).await {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["messages"][0]["from"], json!("lead"));
                assert_eq!(output["messages"][0]["content"], json!("stand-up in 5"));
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn peer_dialogue_off_blocks_a_specialist_send() {
        let bus = MessageBus::new(8);
        let spec = SendMessageTool::new("inventory", bus, &dialogue(false, 10), false);
        let ch = FakeLeadChannel::at(TrustTier::Trusted);
        let audit = NullAuditHook;
        let tok = CancellationToken::new();
        let out = spec.execute(json!({ "content": "hey" }), &ctx!(ch, audit, tok)).await;
        assert!(matches!(out, ToolOutcome::Failed(_)), "specialist blocked when peer dialogue off");
    }

    #[tokio::test]
    async fn per_turn_budget_is_enforced_and_resettable() {
        let bus = MessageBus::new(64);
        let send = SendMessageTool::new("lead", bus, &dialogue(true, 2), true);
        let ch = FakeLeadChannel::at(TrustTier::Trusted);
        let audit = NullAuditHook;
        let tok = CancellationToken::new();

        assert!(matches!(send.execute(json!({"content":"1"}), &ctx!(ch, audit, tok)).await, ToolOutcome::Completed { .. }));
        assert!(matches!(send.execute(json!({"content":"2"}), &ctx!(ch, audit, tok)).await, ToolOutcome::Completed { .. }));
        // Third send this turn exceeds the budget of 2.
        assert!(matches!(send.execute(json!({"content":"3"}), &ctx!(ch, audit, tok)).await, ToolOutcome::Failed(_)));
        // New turn → budget refreshed.
        send.reset_turn();
        assert!(matches!(send.execute(json!({"content":"4"}), &ctx!(ch, audit, tok)).await, ToolOutcome::Completed { .. }));
    }
}
