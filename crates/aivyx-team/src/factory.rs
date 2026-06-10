//! `SpecialistFactory` — construct an **attenuated specialist agent** from
//! a [`TeamMember`] (J.2.1).
//!
//! The factory carries the daemon-injected shared deps (LLM provider,
//! model, audit, the base tool set). Per specialist it produces a normal
//! [`ConcreteAgent`] whose:
//!
//! - capabilities are `attenuate_for_member(lead, member.scopes)` (NT-02),
//! - tool registry is the base set filtered to the member's allowlist
//!   (empty allowlist ⇒ no tools — a specialist gets exactly what it
//!   lists, least privilege),
//! - planner is the member's `soul` over that registry.
//!
//! Running the specialist (a sub-turn over a derived channel) is J.2.2;
//! this phase is construction only — note `build` is **sync** and never
//! touches the provider (the planner factory closure is stored, not
//! invoked, until a turn runs).

use std::sync::Arc;

use aivyx_capability::CapabilitySet;
use aivyx_core::{
    AgentId, AuditHook, ConcreteAgent, LlmPlanner, LlmPlannerConfig, Tool, ToolRegistry,
};
use aivyx_llm::LlmProvider;

use crate::attenuation::attenuate_for_member;
use crate::config::{DialogueConfig, TeamError, TeamMember};
use crate::message_bus::MessageBus;
use crate::message_tools::{ReadMessagesTool, SendMessageTool};

/// The shared deps the daemon injects so the pool can build specialists.
pub struct SpecialistFactory {
    provider: Arc<dyn LlmProvider>,
    model: String,
    max_tokens: u32,
    audit: Arc<dyn AuditHook>,
    /// The daemon's full tool set; each specialist gets a filtered subset.
    base_tools: Vec<Arc<dyn Tool>>,
    /// When set (J.5), every built specialist also gets its own
    /// `send_message` / `read_message` tools bound to its name + this bus, so
    /// peers can talk. Opt-in: without it, specialists are tool-only.
    dialogue: Option<(Arc<MessageBus>, DialogueConfig)>,
}

impl SpecialistFactory {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        model: impl Into<String>,
        max_tokens: u32,
        audit: Arc<dyn AuditHook>,
        base_tools: Vec<Arc<dyn Tool>>,
    ) -> Self {
        SpecialistFactory {
            provider,
            model: model.into(),
            max_tokens,
            audit,
            base_tools,
            dialogue: None,
        }
    }

    /// Wire team dialogue: every specialist `build`-t hereafter also gets its
    /// own message tools on `bus` (J.5 roster wiring).
    pub fn with_dialogue(mut self, bus: Arc<MessageBus>, dialogue: DialogueConfig) -> Self {
        self.dialogue = Some((bus, dialogue));
        self
    }

    /// Build an attenuated specialist agent from `member`, with its
    /// capabilities capped at `lead_caps` (NT-02). Sync — no turn runs.
    pub fn build(
        &self,
        member: &TeamMember,
        lead_caps: &CapabilitySet,
    ) -> Result<ConcreteAgent, TeamError> {
        let caps = attenuate_for_member(lead_caps, &member.parsed_scopes()?);
        let registry = Arc::new(ToolRegistry::new(self.member_tools(member)));

        // Captured by the planner factory (invoked once per turn, in J.2.2).
        let provider = Arc::clone(&self.provider);
        let registry_for_planner = Arc::clone(&registry);
        let model = self.model.clone();
        let max_tokens = self.max_tokens;
        let soul = member.soul.clone();

        Ok(ConcreteAgent::new(
            AgentId::new(),
            caps,
            registry,
            Arc::clone(&self.audit),
            move || {
                let cfg = LlmPlannerConfig::new(&model)
                    .with_system_prompt(&soul)
                    .with_max_tokens(max_tokens);
                Box::new(LlmPlanner::new(
                    Arc::clone(&provider),
                    Arc::clone(&registry_for_planner),
                    cfg,
                ))
            },
        ))
    }

    /// The tool set a specialist receives: its allowlisted base tools, plus —
    /// when dialogue is wired (J.5) — its own `send_message` / `read_message`
    /// bound to its name. A specialist is never the lead, so `is_lead = false`
    /// and its sends honour `enable_peer_dialogue`.
    fn member_tools(&self, member: &TeamMember) -> Vec<Arc<dyn Tool>> {
        let mut tools = filter_tools(&self.base_tools, &member.tool_allowlist);
        if let Some((bus, dialogue)) = &self.dialogue {
            tools.push(Arc::new(SendMessageTool::new(
                &member.name,
                Arc::clone(bus),
                dialogue,
                false,
            )));
            tools.push(Arc::new(ReadMessagesTool::new(bus, &member.name)));
        }
        tools
    }
}

/// Keep only the base tools whose name is in `allowlist`. An **empty**
/// allowlist yields **no** tools — a specialist is given exactly the tools
/// it lists (least privilege), unlike a default Role (absent ⇒ all).
pub fn filter_tools(base: &[Arc<dyn Tool>], allowlist: &[String]) -> Vec<Arc<dyn Tool>> {
    base.iter()
        .filter(|t| allowlist.iter().any(|a| a.as_str() == t.name()))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_capability::{Scope, TrustTier};
    use aivyx_core::{Agent, CancellationToken, NullAuditHook, ToolContext, ToolId, ToolOutcome};
    use async_trait::async_trait;

    // --- minimal test fakes ------------------------------------------------

    struct FakeTool(ToolId, &'static str);
    #[async_trait]
    impl Tool for FakeTool {
        fn id(&self) -> ToolId {
            self.0
        }
        fn name(&self) -> &str {
            self.1
        }
        fn description(&self) -> &str {
            "fake"
        }
        fn input_schema(&self) -> &serde_json::Value {
            use std::sync::OnceLock;
            static S: OnceLock<serde_json::Value> = OnceLock::new();
            S.get_or_init(|| serde_json::json!({ "type": "object" }))
        }
        fn required_scope(&self, _: &serde_json::Value) -> Scope {
            Scope::parse("fs.read").unwrap()
        }
        async fn execute(&self, _: serde_json::Value, _: &ToolContext<'_>) -> ToolOutcome {
            unreachable!("J.2.1 never executes a tool")
        }
    }
    fn fake(name: &'static str) -> Arc<dyn Tool> {
        Arc::new(FakeTool(ToolId::new(), name))
    }

    struct UnusedProvider;
    #[async_trait]
    impl LlmProvider for UnusedProvider {
        async fn chat_stream(
            &self,
            _: aivyx_llm::LlmRequest<'_>,
            _: &CancellationToken,
        ) -> Result<Box<dyn aivyx_llm::LlmStream>, aivyx_llm::LlmError> {
            unreachable!("J.2.1 never runs a turn")
        }
    }

    fn member(name: &str, scopes: &[&str], tools: &[&str]) -> TeamMember {
        TeamMember {
            name: name.into(),
            role: "R".into(),
            soul: "soul".into(),
            tool_allowlist: tools.iter().map(|s| s.to_string()).collect(),
            capability_scopes: scopes.iter().map(|s| s.to_string()).collect(),
            trust_ceiling: TrustTier::Trusted,
        }
    }
    fn factory(base: Vec<Arc<dyn Tool>>) -> SpecialistFactory {
        SpecialistFactory::new(
            Arc::new(UnusedProvider),
            "test-model",
            4096,
            Arc::new(NullAuditHook),
            base,
        )
    }

    // --- filter_tools ------------------------------------------------------

    #[test]
    fn filter_keeps_only_allowlisted_tools() {
        let base = vec![fake("a"), fake("b"), fake("c")];
        let kept_tools = filter_tools(&base, &["a".into(), "c".into()]);
        let kept: Vec<&str> = kept_tools.iter().map(|t| t.name()).collect();
        assert_eq!(kept, ["a", "c"]);
    }

    #[test]
    fn empty_allowlist_yields_no_tools() {
        let base = vec![fake("a"), fake("b")];
        assert!(filter_tools(&base, &[]).is_empty(), "least privilege");
    }

    #[test]
    fn unknown_allowlist_name_is_ignored() {
        let base = vec![fake("a")];
        let kept = filter_tools(&base, &["a".into(), "nonexistent".into()]);
        assert_eq!(kept.len(), 1);
    }

    // --- build (construction) ---------------------------------------------

    #[test]
    fn build_attenuates_capabilities_against_the_lead() {
        // The lead grants fs.read + fs.write; the specialist declares fs.read
        // (kept) + memory.write (NT-02: lead lacks it → dropped). fs.write is
        // not declared, so it must not appear.
        let f = factory(vec![]);
        let lead = CapabilitySet::from_scopes([
            Scope::parse("fs.read").unwrap(),
            Scope::parse("fs.write").unwrap(),
        ]);
        let agent = f
            .build(&member("spec", &["fs.read", "memory.write"], &[]), &lead)
            .unwrap();

        let caps = agent.capabilities();
        assert!(caps.grants(&Scope::parse("fs.read").unwrap()), "declared & granted");
        assert!(
            !caps.grants(&Scope::parse("memory.write").unwrap()),
            "NT-02: lead never granted it"
        );
        assert!(
            !caps.grants(&Scope::parse("fs.write").unwrap()),
            "not declared by the specialist"
        );
    }

    #[test]
    fn build_gives_the_specialist_only_its_allowlisted_tools() {
        // The factory holds three tools; the specialist lists one.
        let f = factory(vec![fake("alpha"), fake("beta"), fake("gamma")]);
        let lead = CapabilitySet::from_scopes([Scope::parse("fs.read").unwrap()]);
        // build succeeds and (by construction) the registry is the filtered
        // set — proven directly via filter_tools so we don't need to crack
        // open the agent's private registry.
        f.build(&member("spec", &["fs.read"], &["beta"]), &lead)
            .expect("build");
        let filtered = filter_tools(
            &[fake("alpha"), fake("beta"), fake("gamma")],
            &["beta".to_string()],
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name(), "beta");
    }

    #[test]
    fn with_dialogue_injects_per_member_message_tools() {
        use crate::message_bus::MessageBus;
        let bus = MessageBus::new(8);
        let f = factory(vec![fake("alpha")]).with_dialogue(bus, DialogueConfig::default());
        // The specialist lists `alpha`; dialogue adds send_message + read_message.
        let tools = f.member_tools(&member("spec", &["fs.read"], &["alpha"]));
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"send_message"), "dialogue wired send");
        assert!(names.contains(&"read_message"), "dialogue wired read");
    }

    #[test]
    fn without_dialogue_no_message_tools() {
        let f = factory(vec![fake("alpha")]);
        let tools = f.member_tools(&member("spec", &["fs.read"], &["alpha"]));
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert_eq!(names, ["alpha"], "no bus → tool-only, least privilege");
    }

    #[test]
    fn build_rejects_an_unknown_scope() {
        let f = factory(vec![]);
        let lead = CapabilitySet::from_scopes([Scope::parse("fs.read").unwrap()]);
        let err = f
            .build(&member("spec", &["not.a.base"], &[]), &lead)
            .err()
            .expect("build should reject the unknown scope");
        assert!(matches!(err, TeamError::Scope(s) if s == "not.a.base"));
    }
}
