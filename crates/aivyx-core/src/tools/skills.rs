//! Phase 110 — `skills.list` and `skills.invoke` substrate
//! tools for the Skills Auto-Creation (Reflection Staging)
//! work. Read-only enumeration and on-demand procedure
//! rendering of the operator-approved skill set stored in the
//! Persona chain as `PersonaDeltaCategory::LearnedSkill`
//! entries.
//!
//! ## Why two tools instead of one
//!
//! Q3(c) at Phase 110 sign-off chose **both** the system-
//! prompt render path AND a callable tool surface. The split
//! into two tools mirrors the substrate-tool convention:
//!
//! - `skills.list` enumerates the approved skill set as
//!   `{name, trigger}` records — short, suitable for the
//!   agent to scan before deciding which skill applies.
//! - `skills.invoke` takes a skill name and returns the
//!   full `procedure` body — the agent reads, then follows
//!   the procedure.
//!
//! Folding both behaviors into one tool would conflate
//! enumeration and dereference, the same way `fs.metadata` is
//! deliberately separate from `fs.read` (Phase 100 / A11).
//!
//! ## How the tools see the skill set
//!
//! The Persona chain is held by the daemon as a
//! `SharedEffectivePersona = Arc<RwLock<EffectivePersona>>`
//! handle. The tools take a callback closure at construction
//! time so the daemon can pass its own read-side accessor
//! without `aivyx-core` taking on a dep edge to
//! `aivyx-channel`. The closure reads under the read lock,
//! materializes the `Vec<String>` of JSON-serialized skills,
//! and returns it. Tools then parse with
//! `LearnedSkill::from_json_value` at render time.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};
use aivyx_capability::Scope;

/// Callback the daemon supplies so the tools can read the
/// current approved skill set without depending on
/// `aivyx-channel`. Returns the list of JSON-serialized
/// `LearnedSkill` strings (one per approved skill); the
/// caller parses them back through
/// `aivyx_channel::persona::LearnedSkill::from_json_value`.
///
/// The `Send + Sync + 'static` bounds let the closure be
/// stored inside the tool struct (which is itself
/// `Send + Sync + 'static` so it can live inside
/// `Arc<dyn Tool>`).
pub type SkillReader = Arc<dyn Fn() -> Vec<String> + Send + Sync + 'static>;

// ---------------------------------------------------------------------------
// skills.list
// ---------------------------------------------------------------------------

/// `skills.list` — enumerate every operator-approved skill in
/// the current Persona state. Returns a JSON object with a
/// `skills` array of `{name, trigger}` records. Procedure
/// bodies are elided; the agent uses `skills.invoke` to read
/// a specific procedure on demand.
pub struct SkillsListTool {
    id: ToolId,
    schema: Value,
    reader: SkillReader,
}

impl std::fmt::Debug for SkillsListTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkillsListTool")
            .field("id", &self.id)
            .finish()
    }
}

impl SkillsListTool {
    pub fn new(reader: SkillReader) -> Self {
        SkillsListTool {
            id: ToolId::new(),
            schema: list_input_schema(),
            reader,
        }
    }
}

#[async_trait]
impl Tool for SkillsListTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "skills.list"
    }

    fn description(&self) -> &str {
        "List every operator-approved learned skill. Input is \
         a JSON object (no fields required). Returns a JSON \
         object with a `skills` array — each entry has `name` \
         (stable skill identifier) and `trigger` (short \
         description of when the skill applies). The procedure \
         body is elided here; use `skills.invoke` with a skill \
         name to read the full procedure."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("skills.list")
            .expect("skills.list must parse — it is in KNOWN_BASES from Phase 110")
    }

    async fn execute(&self, _input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let raw_entries = (self.reader)();
        let parsed: Vec<Value> = raw_entries
            .iter()
            .filter_map(|s| {
                // Parse just enough to surface name + trigger.
                // Skip malformed entries silently — the
                // renderer pattern (skip-malformed-rather-than-
                // fail-render) matches Phase 60 Persona revert
                // semantics for chain corruption defense.
                let v: serde_json::Value = serde_json::from_str(s).ok()?;
                let name = v.get("name")?.as_str()?.to_string();
                let trigger = v.get("trigger")?.as_str()?.to_string();
                Some(json!({ "name": name, "trigger": trigger }))
            })
            .collect();
        ToolOutcome::Completed {
            output: json!({ "skills": parsed }),
            verified: Verification::Verified,
        }
    }
}

// ---------------------------------------------------------------------------
// skills.invoke
// ---------------------------------------------------------------------------

/// `skills.invoke` — render the full procedure body of one
/// approved skill into the current turn's working context.
/// The agent passes a `name`; the tool returns a JSON object
/// with `name`, `trigger`, and `procedure` (the full text).
pub struct SkillsInvokeTool {
    id: ToolId,
    schema: Value,
    reader: SkillReader,
}

impl std::fmt::Debug for SkillsInvokeTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SkillsInvokeTool")
            .field("id", &self.id)
            .finish()
    }
}

impl SkillsInvokeTool {
    pub fn new(reader: SkillReader) -> Self {
        SkillsInvokeTool {
            id: ToolId::new(),
            schema: invoke_input_schema(),
            reader,
        }
    }
}

#[async_trait]
impl Tool for SkillsInvokeTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "skills.invoke"
    }

    fn description(&self) -> &str {
        "Render the full procedure body of one operator-approved \
         learned skill. Input is a JSON object with a `name` field \
         (the skill's stable identifier from `skills.list`). \
         Returns a JSON object with `name`, `trigger`, and \
         `procedure` (the full text). Fails cleanly if no skill \
         with that name is in the approved set."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("skills.invoke")
            .expect("skills.invoke must parse — it is in KNOWN_BASES from Phase 110")
    }

    async fn execute(&self, input: Value, ctx: &ToolContext<'_>) -> ToolOutcome {
        let name = match input.get("name").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: "skills.invoke: `name` field missing or empty".to_string(),
                });
            }
        };

        let raw_entries = (self.reader)();
        for raw in &raw_entries {
            let v: serde_json::Value = match serde_json::from_str(raw) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let entry_name = match v.get("name").and_then(|n| n.as_str()) {
                Some(n) => n,
                None => continue,
            };
            if entry_name == name {
                let trigger = v.get("trigger").and_then(|t| t.as_str()).unwrap_or("");
                let procedure = v.get("procedure").and_then(|p| p.as_str()).unwrap_or("");
                // Phase 117 — emit a dedicated SkillInvocation
                // audit entry alongside the regular ToolCall the
                // planner will write. The ToolCall's input_hash
                // hides the skill name (D4 secrets-safety); the
                // SkillInvocation carries it in cleartext so
                // Phase 116's record_turn_outcomes can populate
                // per-skill ledger rows (RelevanceSurfaceKind::
                // Skill).
                ctx.audit.on_event(crate::AuditTag::SkillInvocation {
                    turn_id: ctx.turn_id,
                    session_id: ctx.session_id,
                    skill_name: name.clone(),
                });
                return ToolOutcome::Completed {
                    output: json!({
                        "name": name,
                        "trigger": trigger,
                        "procedure": procedure,
                    }),
                    verified: Verification::Verified,
                };
            }
        }
        ToolOutcome::Failed(AivyxError::Tool {
            tool: self.id,
            detail: format!("skills.invoke: no approved skill named {name:?}"),
        })
    }
}

// ---------------------------------------------------------------------------
// Schemas
// ---------------------------------------------------------------------------

fn list_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "required": []
    })
}

fn invoke_input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": {
                "type": "string",
                "description": "Stable skill identifier (from skills.list)."
            }
        },
        "required": ["name"]
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod skills_tests {
    use super::*;
    use crate::{AgentId, CancellationToken, MessageOrigin, NullAuditHook, SessionId, TurnId};

    fn json_skill(name: &str, trigger: &str, procedure: &str) -> String {
        json!({
            "name": name,
            "trigger": trigger,
            "procedure": procedure,
        })
        .to_string()
    }

    fn fixed_reader(entries: Vec<String>) -> SkillReader {
        Arc::new(move || entries.clone())
    }

    /// Minimal channel that ignores every callback — skills
    /// tools never touch the channel surface, so a no-op fake
    /// is enough. Mirrors the pattern in `fs.rs`'s test module.
    struct NoopChannel {
        session: SessionId,
        token: CancellationToken,
    }

    #[async_trait]
    impl crate::ChannelContext for NoopChannel {
        fn channel_name(&self) -> &str {
            "skills-test"
        }
        fn platform(&self) -> crate::ChannelPlatform {
            crate::ChannelPlatform::Local
        }
        fn trust_tier(&self) -> aivyx_capability::TrustTier {
            aivyx_capability::TrustTier::Trusted
        }
        fn session_id(&self) -> SessionId {
            self.session
        }
        async fn stream_event(
            &self,
            _event: crate::StreamEvent<'_>,
        ) -> Result<(), crate::ChannelError> {
            Ok(())
        }
        async fn finalize(&self, _outcome: &crate::TurnOutcome) -> Result<(), crate::ChannelError> {
            Ok(())
        }
        fn cancellation_token(&self) -> CancellationToken {
            self.token.clone()
        }
    }

    async fn run_execute(tool: &dyn Tool, input: Value) -> ToolOutcome {
        let channel = NoopChannel {
            session: SessionId::new(),
            token: CancellationToken::new(),
        };
        let audit = NullAuditHook;
        let ctx = ToolContext {
            agent_id: AgentId::new(),
            session_id: channel.session,
            turn_id: TurnId::new(),
            channel: &channel,
            audit: &audit,
            cancellation: &channel.token,
            message_origin: MessageOrigin::Operator,
        };
        tool.execute(input, &ctx).await
    }

    // -- skills.list -----------------------------------------------------

    #[tokio::test]
    async fn skills_list_returns_name_and_trigger_for_each_entry() {
        let reader = fixed_reader(vec![
            json_skill(
                "code-review",
                "When asked to review code",
                "Read every line...",
            ),
            json_skill(
                "debug-flow",
                "When debugging a crash",
                "Start with the stack trace...",
            ),
        ]);
        let tool = SkillsListTool::new(reader);
        let outcome = run_execute(&tool, json!({})).await;
        match outcome {
            ToolOutcome::Completed { output, .. } => {
                let skills = output.get("skills").and_then(|v| v.as_array()).unwrap();
                assert_eq!(skills.len(), 2);
                assert_eq!(skills[0]["name"], "code-review");
                assert_eq!(skills[0]["trigger"], "When asked to review code");
                // Procedure is intentionally elided from the
                // list response.
                assert!(skills[0].get("procedure").is_none());
                assert_eq!(skills[1]["name"], "debug-flow");
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn skills_list_skips_malformed_entries() {
        let reader = fixed_reader(vec![
            json_skill("good", "trigger", "procedure"),
            "not-valid-json".to_string(),
            json!({ "name": "no-trigger" }).to_string(),
            json_skill("also-good", "another trigger", "another procedure"),
        ]);
        let tool = SkillsListTool::new(reader);
        let outcome = run_execute(&tool, json!({})).await;
        match outcome {
            ToolOutcome::Completed { output, .. } => {
                let skills = output.get("skills").and_then(|v| v.as_array()).unwrap();
                assert_eq!(skills.len(), 2);
                assert_eq!(skills[0]["name"], "good");
                assert_eq!(skills[1]["name"], "also-good");
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn skills_list_empty_reader_returns_empty_array() {
        let reader = fixed_reader(vec![]);
        let tool = SkillsListTool::new(reader);
        let outcome = run_execute(&tool, json!({})).await;
        match outcome {
            ToolOutcome::Completed { output, .. } => {
                let skills = output.get("skills").and_then(|v| v.as_array()).unwrap();
                assert!(skills.is_empty());
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[test]
    fn skills_list_required_scope_is_skills_list() {
        let tool = SkillsListTool::new(fixed_reader(vec![]));
        let scope = tool.required_scope(&json!({}));
        assert_eq!(scope.as_str(), "skills.list");
    }

    // -- skills.invoke ---------------------------------------------------

    #[tokio::test]
    async fn skills_invoke_returns_full_procedure_for_existing_skill() {
        let reader = fixed_reader(vec![json_skill(
            "code-review",
            "When asked to review code",
            "1. Read every line.\n2. Check tests.\n3. Suggest improvements.",
        )]);
        let tool = SkillsInvokeTool::new(reader);
        let outcome = run_execute(&tool, json!({ "name": "code-review" })).await;
        match outcome {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["name"], "code-review");
                assert_eq!(output["trigger"], "When asked to review code");
                assert!(output["procedure"].as_str().unwrap().contains("1. Read"));
                assert!(output["procedure"].as_str().unwrap().contains("3. Suggest"));
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn skills_invoke_fails_cleanly_on_unknown_name() {
        let reader = fixed_reader(vec![json_skill("exists", "trigger", "procedure")]);
        let tool = SkillsInvokeTool::new(reader);
        let outcome = run_execute(&tool, json!({ "name": "missing" })).await;
        match outcome {
            ToolOutcome::Failed(AivyxError::Tool { detail, .. }) => {
                assert!(detail.contains("missing"));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn skills_invoke_fails_cleanly_on_missing_name_field() {
        let tool = SkillsInvokeTool::new(fixed_reader(vec![]));
        let outcome = run_execute(&tool, json!({})).await;
        match outcome {
            ToolOutcome::Failed(AivyxError::Tool { detail, .. }) => {
                assert!(detail.contains("name"));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn skills_invoke_fails_cleanly_on_empty_name() {
        let tool = SkillsInvokeTool::new(fixed_reader(vec![]));
        let outcome = run_execute(&tool, json!({ "name": "" })).await;
        match outcome {
            ToolOutcome::Failed(AivyxError::Tool { detail, .. }) => {
                assert!(detail.contains("empty") || detail.contains("name"));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn skills_invoke_skips_malformed_entries_in_lookup() {
        let reader = fixed_reader(vec![
            "garbage".to_string(),
            json_skill("target", "trigger", "procedure"),
            "{}".to_string(),
        ]);
        let tool = SkillsInvokeTool::new(reader);
        let outcome = run_execute(&tool, json!({ "name": "target" })).await;
        match outcome {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["name"], "target");
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[test]
    fn skills_invoke_required_scope_is_skills_invoke() {
        let tool = SkillsInvokeTool::new(fixed_reader(vec![]));
        let scope = tool.required_scope(&json!({ "name": "any" }));
        assert_eq!(scope.as_str(), "skills.invoke");
    }

    // -- schemas ---------------------------------------------------------

    #[test]
    fn list_schema_has_no_required_fields() {
        let s = list_input_schema();
        let required = s["required"].as_array().unwrap();
        assert!(required.is_empty());
    }

    #[test]
    fn invoke_schema_requires_name() {
        let s = invoke_input_schema();
        assert_eq!(s["required"][0], "name");
    }
}
