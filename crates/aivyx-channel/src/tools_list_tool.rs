//! `ToolsListTool` — Chapter Atlas (AT.2): runtime tool introspection.
//!
//! A read-only infrastructure tool that returns the agent's **own** tool
//! surface — the live name + description (+ optional input schema) of every
//! tool the agent has. The agent calls it to answer "what tools do I have?"
//! with **ground truth** instead of guessing — local models in particular
//! hallucinate tool names when asked to enumerate (see the
//! `local-model-tool-introspection` finding), and this gives them the real
//! list.
//!
//! ## Why a snapshot, not a registry handle
//!
//! `tools.list` lives *inside* the `ToolRegistry` it would describe, so it
//! can't hold an `Arc<ToolRegistry>` (reference cycle). Instead it is handed a
//! **precomputed `Vec<ToolInfo>`** captured at agent-build time, after the tool
//! list is assembled. The tool set is fixed once the agent is built, so a
//! snapshot is faithful and avoids the cycle entirely.
//!
//! ## Scope
//!
//! Gated by `audit.read` — the same "inspect your own state" base
//! `turn.history` and `daemon.state` use. No new capability base.
//!
//! ## Input
//!
//! ```json
//! { "filter": "memory",   // optional: case-insensitive substring on name/description
//!   "detail": true }       // optional: include each tool's input JSON schema
//! ```

use aivyx_capability::Scope;
use aivyx_core::{Tool, ToolContext, ToolId, ToolOutcome, Verification};
use serde_json::{json, Value};

/// One tool's introspectable metadata, captured at agent-build time.
#[derive(Debug, Clone)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub schema: Value,
}

impl ToolInfo {
    /// Capture from a live tool.
    pub fn from_tool(tool: &dyn Tool) -> Self {
        ToolInfo {
            name: tool.name().to_string(),
            description: tool.description().to_string(),
            schema: tool.input_schema().clone(),
        }
    }
}

/// `tools.list` — enumerate the agent's own tools.
pub struct ToolsListTool {
    id: ToolId,
    tools: Vec<ToolInfo>,
    schema: Value,
}

impl std::fmt::Debug for ToolsListTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolsListTool")
            .field("tools", &self.tools.len())
            .finish()
    }
}

impl ToolsListTool {
    /// Build from a precomputed snapshot of the agent's tools (including, if
    /// the caller wishes, a self-entry — see [`ToolsListTool::self_info`]).
    pub fn new(tools: Vec<ToolInfo>) -> Self {
        ToolsListTool {
            id: ToolId::new(),
            tools,
            schema: json!({
                "type": "object",
                "properties": {
                    "filter": {
                        "type": "string",
                        "description": "Optional case-insensitive substring; only tools whose name or description contains it are returned."
                    },
                    "detail": {
                        "type": "boolean",
                        "description": "When true, include each tool's input JSON schema. Defaults to false (name + description only)."
                    }
                },
                "additionalProperties": false
            }),
        }
    }

    /// The self-descriptor, so `tools.list` lists itself too. Callers append
    /// this to the snapshot before constructing the tool.
    pub fn self_info() -> ToolInfo {
        ToolInfo {
            name: "tools.list".to_string(),
            description:
                "List the agent's own tools (name + description; set detail=true for input schemas). Use this to know exactly which tools exist instead of guessing."
                    .to_string(),
            // Mirrors `schema` below; kept in sync by the self_info_matches_schema test.
            schema: json!({
                "type": "object",
                "properties": {
                    "filter": { "type": "string" },
                    "detail": { "type": "boolean" }
                },
                "additionalProperties": false
            }),
        }
    }
}

#[async_trait::async_trait]
impl Tool for ToolsListTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "tools.list"
    }

    fn description(&self) -> &str {
        "List the agent's own tools (name + description; set detail=true for input schemas). \
         Use this to know exactly which tools exist instead of guessing."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("audit.read").expect("audit.read is a known base")
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let filter = input
            .get("filter")
            .and_then(Value::as_str)
            .map(|s| s.to_lowercase());
        let detail = input
            .get("detail")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let matched: Vec<&ToolInfo> = self
            .tools
            .iter()
            .filter(|t| match &filter {
                Some(q) => {
                    t.name.to_lowercase().contains(q)
                        || t.description.to_lowercase().contains(q)
                }
                None => true,
            })
            .collect();

        let tools: Vec<Value> = matched
            .iter()
            .map(|t| {
                if detail {
                    json!({ "name": t.name, "description": t.description, "input_schema": t.schema })
                } else {
                    json!({ "name": t.name, "description": t.description })
                }
            })
            .collect();

        ToolOutcome::Completed {
            output: json!({ "count": tools.len(), "tools": tools }),
            verified: Verification::NotApplicable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn infos() -> Vec<ToolInfo> {
        vec![
            ToolInfo {
                name: "memory.read".into(),
                description: "Recall stored memories".into(),
                schema: json!({"type": "object", "properties": {"topic": {"type": "string"}}}),
            },
            ToolInfo {
                name: "fs.write".into(),
                description: "Write a file".into(),
                schema: json!({"type": "object"}),
            },
        ]
    }

    fn tool() -> ToolsListTool {
        let mut v = infos();
        v.push(ToolsListTool::self_info());
        ToolsListTool::new(v)
    }

    // Minimal test channel/audit — same pattern as memory_gc_tool.rs.
    use aivyx_core::{
        AgentId, CancellationToken, ChannelContext, ChannelError, ChannelPlatform, SessionId,
        StreamEvent, TurnId, TurnOutcome,
    };

    struct NoopChannel {
        session: SessionId,
        token: CancellationToken,
    }

    #[async_trait::async_trait]
    impl ChannelContext for NoopChannel {
        fn session_id(&self) -> SessionId {
            self.session
        }
        fn platform(&self) -> ChannelPlatform {
            ChannelPlatform::Local
        }
        fn channel_name(&self) -> &str {
            "test"
        }
        fn trust_tier(&self) -> aivyx_capability::TrustTier {
            aivyx_capability::TrustTier::Trusted
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

    struct NoopAudit;
    impl aivyx_core::AuditHook for NoopAudit {
        fn on_event(&self, _tag: aivyx_core::AuditTag) {}
    }

    async fn run(input: Value) -> Value {
        let ch = NoopChannel {
            session: SessionId::new(),
            token: CancellationToken::new(),
        };
        let audit = NoopAudit;
        let ctx = ToolContext {
            agent_id: AgentId::new(),
            session_id: ch.session,
            turn_id: TurnId::new(),
            channel: &ch,
            audit: &audit,
            cancellation: &ch.token,
        };
        match tool().execute(input, &ctx).await {
            ToolOutcome::Completed { output, .. } => output,
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[test]
    fn scope_is_audit_read() {
        assert_eq!(tool().required_scope(&Value::Null).base(), "audit.read");
    }

    #[test]
    fn self_info_matches_live_schema() {
        // The self-descriptor's schema shape must match the real tool's keys.
        let t = ToolsListTool::new(vec![]);
        let live = t.input_schema();
        let si = ToolsListTool::self_info();
        let keys = |v: &Value| {
            v.get("properties")
                .and_then(Value::as_object)
                .map(|o| {
                    let mut k: Vec<String> = o.keys().cloned().collect();
                    k.sort();
                    k
                })
                .unwrap_or_default()
        };
        assert_eq!(keys(live), keys(&si.schema));
    }

    #[tokio::test]
    async fn lists_all_by_default_including_self() {
        let out = run(json!({})).await;
        assert_eq!(out["count"], 3);
        let names: Vec<&str> = out["tools"].as_array().unwrap()
            .iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"tools.list"), "lists itself: {names:?}");
        assert!(names.contains(&"memory.read"));
    }

    #[tokio::test]
    async fn filter_is_case_insensitive_substring() {
        let out = run(json!({"filter": "MEM"})).await;
        assert_eq!(out["count"], 1);
        assert_eq!(out["tools"][0]["name"], "memory.read");
    }

    #[tokio::test]
    async fn detail_false_omits_schema_true_includes_it() {
        let plain = run(json!({"filter": "fs.write"})).await;
        assert!(plain["tools"][0].get("input_schema").is_none());
        let detailed = run(json!({"filter": "fs.write", "detail": true})).await;
        assert!(detailed["tools"][0].get("input_schema").is_some());
    }
}
