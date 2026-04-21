//! Memory GC tool — Phase 42 Task 6.
//!
//! Agent-invocable garbage collection trigger for the memory substrate.
//! Takes a topic and a maximum entry count, evicts oldest entries that
//! exceed the cap via `Memory::gc_topic()`. Scoped under `memory.gc`.
//!
//! This tool lives in the channel layer (not `aivyx-memory::tools`)
//! to preserve the lib.rs byte-identity streak. It follows the same
//! `Tool` trait pattern as `OllamaListTool` and `ReflectionTool`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};
use aivyx_memory::Memory;

// ---------------------------------------------------------------------------
// MemoryGcTool
// ---------------------------------------------------------------------------

/// Agent-facing tool that triggers per-topic garbage collection.
///
/// Input schema:
/// ```json
/// {
///   "topic": "string — the memory topic to GC",
///   "max_entries": "integer — keep at most this many entries (oldest evicted first)"
/// }
/// ```
///
/// Returns `{ "evicted": N }` on success.
pub struct MemoryGcTool {
    id: ToolId,
    schema: Value,
    memory: Arc<dyn Memory>,
}

impl std::fmt::Debug for MemoryGcTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryGcTool")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl MemoryGcTool {
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        MemoryGcTool {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "topic": {
                        "type": "string",
                        "description": "The memory topic to garbage-collect."
                    },
                    "max_entries": {
                        "type": "integer",
                        "description": "Maximum number of entries to keep. Oldest entries beyond this cap are evicted.",
                        "minimum": 0
                    }
                },
                "required": ["topic", "max_entries"],
                "additionalProperties": false
            }),
            memory,
        }
    }
}

#[async_trait]
impl Tool for MemoryGcTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "memory.gc"
    }

    fn description(&self) -> &str {
        "Garbage-collect a memory topic by evicting the oldest entries \
         that exceed a maximum count. Returns the number of entries evicted."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, input: &Value) -> Scope {
        let topic = input
            .get("topic")
            .and_then(Value::as_str)
            .unwrap_or("\x00denied");
        let qualifier = format!("topic:{topic}");
        Scope::parse(&format!("memory.gc:{qualifier}")).unwrap_or_else(|| {
            Scope::parse("memory.gc:topic:\x00denied").expect("known base")
        })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let topic = match input.get("topic").and_then(Value::as_str) {
            Some(t) if !t.is_empty() => t,
            _ => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: "missing or empty `topic` field".to_string(),
                });
            }
        };
        let max_entries = match input.get("max_entries").and_then(Value::as_u64) {
            Some(n) => n as usize,
            None => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: "missing or invalid `max_entries` field".to_string(),
                });
            }
        };

        match self.memory.gc_topic(topic, max_entries).await {
            Ok(evicted) => ToolOutcome::Completed {
                output: json!({ "evicted": evicted }),
                verified: Verification::NotApplicable,
            },
            Err(e) => ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("memory.gc failed: {e}"),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_core::{
        AgentId, CancellationToken, ChannelContext, ChannelError, ChannelPlatform, SessionId,
        StreamEvent, TurnId, TurnOutcome,
    };
    use aivyx_memory::InMemoryMemory;

    // -- Minimal test channel (same pattern as aivyx-memory/src/tools.rs) --

    struct NoopChannel {
        session: SessionId,
        token: CancellationToken,
    }

    #[async_trait]
    impl ChannelContext for NoopChannel {
        fn session_id(&self) -> SessionId { self.session }
        fn platform(&self) -> ChannelPlatform { ChannelPlatform::Local }
        fn channel_name(&self) -> &str { "test" }
        fn trust_tier(&self) -> aivyx_capability::TrustTier {
            aivyx_capability::TrustTier::Trusted
        }
        async fn stream_event(&self, _event: StreamEvent<'_>) -> Result<(), ChannelError> {
            Ok(())
        }
        async fn finalize(&self, _outcome: &TurnOutcome) -> Result<(), ChannelError> {
            Ok(())
        }
        fn cancellation_token(&self) -> CancellationToken { self.token.clone() }
    }

    struct NoopAudit;

    impl aivyx_core::AuditHook for NoopAudit {
        fn on_event(&self, _tag: aivyx_core::AuditTag) {}
    }

    fn make_ctx<'a>(ch: &'a NoopChannel, audit: &'a dyn aivyx_core::AuditHook) -> ToolContext<'a> {
        ToolContext {
            agent_id: AgentId::new(),
            session_id: ch.session,
            turn_id: TurnId::new(),
            channel: ch,
            audit,
            cancellation: &ch.token,
        }
    }

    fn make_tool() -> MemoryGcTool {
        let mem: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
        MemoryGcTool::new(mem)
    }

    #[test]
    fn name_and_description() {
        let tool = make_tool();
        assert_eq!(tool.name(), "memory.gc");
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn input_schema_has_required_fields() {
        let tool = make_tool();
        let schema = tool.input_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&json!("topic")));
        assert!(required.contains(&json!("max_entries")));
    }

    #[test]
    fn required_scope_contains_topic() {
        let tool = make_tool();
        let input = json!({ "topic": "notes", "max_entries": 5 });
        let scope = tool.required_scope(&input);
        assert!(scope.to_string().contains("memory.gc"));
        assert!(scope.to_string().contains("notes"));
    }

    #[test]
    fn required_scope_deny_on_missing_topic() {
        let tool = make_tool();
        let input = json!({ "max_entries": 5 });
        let scope = tool.required_scope(&input);
        assert!(scope.to_string().contains("denied"));
    }

    #[tokio::test]
    async fn execute_gc_evicts_oldest() {
        let mem = Arc::new(InMemoryMemory::new());
        for i in 0..10 {
            mem.put("logs", &format!("entry {i}")).await.unwrap();
        }
        let tool = MemoryGcTool::new(mem.clone() as Arc<dyn Memory>);
        let ch = NoopChannel { session: SessionId::new(), token: CancellationToken::new() };
        let audit = NoopAudit;
        let ctx = make_ctx(&ch, &audit);
        let input = json!({ "topic": "logs", "max_entries": 3 });
        let outcome = tool.execute(input, &ctx).await;
        match outcome {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["evicted"], 7);
            }
            other => panic!("expected Success, got {other:?}"),
        }
        // Verify only 3 remain.
        let remaining = mem.get_recent("logs", 100).await.unwrap();
        assert_eq!(remaining.len(), 3);
    }

    #[tokio::test]
    async fn execute_gc_empty_topic_fails() {
        let tool = make_tool();
        let ch = NoopChannel { session: SessionId::new(), token: CancellationToken::new() };
        let audit = NoopAudit;
        let ctx = make_ctx(&ch, &audit);
        let input = json!({ "topic": "", "max_entries": 5 });
        let outcome = tool.execute(input, &ctx).await;
        assert!(matches!(outcome, ToolOutcome::Failed { .. }));
    }

    #[tokio::test]
    async fn execute_gc_missing_max_entries_fails() {
        let tool = make_tool();
        let ch = NoopChannel { session: SessionId::new(), token: CancellationToken::new() };
        let audit = NoopAudit;
        let ctx = make_ctx(&ch, &audit);
        let input = json!({ "topic": "notes" });
        let outcome = tool.execute(input, &ctx).await;
        assert!(matches!(outcome, ToolOutcome::Failed { .. }));
    }

    #[tokio::test]
    async fn execute_gc_noop_when_under_cap() {
        let mem = Arc::new(InMemoryMemory::new());
        for i in 0..3 {
            mem.put("logs", &format!("entry {i}")).await.unwrap();
        }
        let tool = MemoryGcTool::new(mem.clone() as Arc<dyn Memory>);
        let ch = NoopChannel { session: SessionId::new(), token: CancellationToken::new() };
        let audit = NoopAudit;
        let ctx = make_ctx(&ch, &audit);
        let input = json!({ "topic": "logs", "max_entries": 10 });
        let outcome = tool.execute(input, &ctx).await;
        match outcome {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["evicted"], 0);
            }
            other => panic!("expected Success, got {other:?}"),
        }
    }
}
