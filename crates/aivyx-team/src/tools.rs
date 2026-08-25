//! Delegation tools — the lead's interface to the [`SpecialistPool`]
//! (J.2.3). These are the tools that turn an ordinary agent into a team
//! lead: calling one runs an attenuated specialist sub-turn and returns
//! its result.
//!
//! Both require the **`team.delegate`** scope — the lead's orchestration
//! authority. Specialists never declare it, so the lead→specialist
//! attenuation drops it: a specialist cannot convene its own team.

use std::sync::Arc;

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};
use async_trait::async_trait;
use serde_json::{json, Value};

use crate::pool::SpecialistPool;

const DELEGATE_SCOPE: &str = "team.delegate";

fn scope() -> Scope {
    Scope::parse(DELEGATE_SCOPE).expect("team.delegate must be a known base")
}

/// Pull `specialist` + the prompt field out of the input and run the
/// specialist, mapping the result to a `ToolOutcome`.
async fn run(
    pool: &SpecialistPool,
    tool_id: ToolId,
    input: &Value,
    ctx: &ToolContext<'_>,
    prompt_key: &str,
) -> ToolOutcome {
    let specialist = input.get("specialist").and_then(Value::as_str);
    let prompt = input.get(prompt_key).and_then(Value::as_str);
    let (Some(specialist), Some(prompt)) = (specialist, prompt) else {
        return ToolOutcome::Failed(AivyxError::Tool {
            tool: tool_id,
            detail: format!("`specialist` and `{prompt_key}` (strings) are required"),
        });
    };
    match pool.run(specialist, prompt, ctx.channel).await {
        Ok(result) => ToolOutcome::Completed {
            output: json!({ "specialist": specialist, "result": result }),
            verified: Verification::Unverified,
        },
        Err(e) => ToolOutcome::Failed(AivyxError::Tool {
            tool: tool_id,
            detail: e.to_string(),
        }),
    }
}

fn two_field_schema(prompt: &str) -> Value {
    json!({
        "type": "object",
        "properties": {
            "specialist": { "type": "string" },
            prompt: { "type": "string" }
        },
        "required": ["specialist", prompt],
        "additionalProperties": false
    })
}

/// `delegate_task` — hand a task to a named specialist and get its result.
pub struct DelegateTaskTool {
    id: ToolId,
    pool: Arc<SpecialistPool>,
    schema: Value,
}

impl DelegateTaskTool {
    pub fn new(pool: Arc<SpecialistPool>) -> Self {
        DelegateTaskTool {
            id: ToolId::new(),
            pool,
            schema: two_field_schema("task"),
        }
    }
}

#[async_trait]
impl Tool for DelegateTaskTool {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "delegate_task"
    }
    fn description(&self) -> &str {
        "Delegate a task to a named specialist on your team. Input: \
         { \"specialist\": string, \"task\": string }. Returns the specialist's result."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _: &Value) -> Scope {
        scope()
    }
    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> ToolOutcome {
        run(&self.pool, self.id, &input, ctx, "task").await
    }
}

/// `query_agent` — ask a specialist a quick question (same mechanism as
/// delegate, different framing for short follow-ups).
pub struct QueryAgentTool {
    id: ToolId,
    pool: Arc<SpecialistPool>,
    schema: Value,
}

impl QueryAgentTool {
    pub fn new(pool: Arc<SpecialistPool>) -> Self {
        QueryAgentTool {
            id: ToolId::new(),
            pool,
            schema: two_field_schema("question"),
        }
    }
}

#[async_trait]
impl Tool for QueryAgentTool {
    fn id(&self) -> ToolId {
        self.id
    }
    fn name(&self) -> &str {
        "query_agent"
    }
    fn description(&self) -> &str {
        "Ask a named specialist a quick question. Input: \
         { \"specialist\": string, \"question\": string }. Returns the specialist's answer."
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn required_scope(&self, _: &Value) -> Scope {
        scope()
    }
    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> ToolOutcome {
        run(&self.pool, self.id, &input, ctx, "question").await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{member, team_pool, FakeLeadChannel, FakeProvider};
    use aivyx_capability::TrustTier;
    use aivyx_core::{AgentId, CancellationToken, ChannelContext, NullAuditHook, TurnId};

    fn delegate_pool(answer: &str) -> Arc<SpecialistPool> {
        Arc::new(team_pool(
            FakeProvider::says(answer),
            vec![
                member("lead", &[], TrustTier::Trusted),
                member("inventory", &["fs.read"], TrustTier::Trusted),
            ],
            "lead",
            &["fs.read"],
        ))
    }

    #[test]
    fn delegate_task_surface() {
        let tool = DelegateTaskTool::new(delegate_pool("x"));
        assert_eq!(tool.name(), "delegate_task");
        assert_eq!(tool.required_scope(&Value::Null).base(), "team.delegate");
        assert_eq!(tool.input_schema()["required"][1], json!("task"));
    }

    #[test]
    fn query_agent_uses_a_question_field() {
        let tool = QueryAgentTool::new(delegate_pool("x"));
        assert_eq!(tool.name(), "query_agent");
        assert_eq!(tool.required_scope(&Value::Null).base(), "team.delegate");
        assert_eq!(tool.input_schema()["required"][1], json!("question"));
    }

    #[tokio::test]
    async fn delegate_task_runs_the_specialist_and_returns_its_result() {
        let tool = DelegateTaskTool::new(delegate_pool("inventory looks healthy"));
        let lead_ch = FakeLeadChannel::at(TrustTier::Trusted);
        let audit = NullAuditHook;
        let token = CancellationToken::new();
        let ctx = ToolContext {
            agent_id: AgentId::new(),
            session_id: lead_ch.session_id(),
            turn_id: TurnId::new(),
            channel: &lead_ch,
            audit: &audit,
            cancellation: &token,
            message_origin: aivyx_core::MessageOrigin::Operator,
        };
        let outcome = tool
            .execute(json!({ "specialist": "inventory", "task": "check stock" }), &ctx)
            .await;
        match outcome {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["specialist"], json!("inventory"));
                assert_eq!(output["result"], json!("inventory looks healthy"));
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn delegate_task_requires_both_fields() {
        let tool = DelegateTaskTool::new(delegate_pool("x"));
        let lead_ch = FakeLeadChannel::at(TrustTier::Trusted);
        let audit = NullAuditHook;
        let token = CancellationToken::new();
        let ctx = ToolContext {
            agent_id: AgentId::new(),
            session_id: lead_ch.session_id(),
            turn_id: TurnId::new(),
            channel: &lead_ch,
            audit: &audit,
            cancellation: &token,
            message_origin: aivyx_core::MessageOrigin::Operator,
        };
        // Missing `task`.
        let outcome = tool.execute(json!({ "specialist": "inventory" }), &ctx).await;
        assert!(matches!(outcome, ToolOutcome::Failed(_)));
    }
}
