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
    TurnSafety,
};
use aivyx_llm::LlmProvider;

use crate::attenuation::attenuate_for_member;
use crate::config::{DialogueConfig, TeamError, TeamMember};
use crate::message_bus::MessageBus;
use crate::message_tools::{ReadMessagesTool, SendMessageTool};

/// Chapter Ensemble — a per-role LLM backend override (its own provider +
/// model). The daemon builds these for any member that declared a `model` /
/// `base_url` override; members without one fall back to the team's shared
/// provider + model. Distinct endpoints give true parallel execution.
#[derive(Clone)]
pub struct SpecialistBackend {
    pub provider: Arc<dyn LlmProvider>,
    pub model: String,
}

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
    /// Chapter Ensemble — per-member backend overrides, keyed by member name.
    /// Empty ⇒ every specialist uses the shared `provider`/`model` (byte-
    /// identical to pre-Ensemble).
    member_backends: std::collections::HashMap<String, SpecialistBackend>,
    /// `aivyx-checkpoint` — attached to every built specialist so an
    /// fs_root-mutating tool call it makes gets checkpointed, same as the
    /// lead agent. `None` (the default) preserves pre-checkpoint behavior.
    checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>,
    /// The shared kvcache pool/store + served build hash, when `[agent]
    /// provider = "llama_cpp"` -- attached to every built specialist so
    /// its own per-turn `LlmPlanner` shares the exact same `KvSlotPool`
    /// the daemon's main agent uses, not one each. `None` (the default)
    /// disables kvcache for every specialist this factory builds.
    kv_cache_handles: Option<(
        Arc<aivyx_llm::KvSlotPool>,
        Arc<aivyx_kvcache::LlamaServerSlotStore>,
        String,
    )>,
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
            member_backends: std::collections::HashMap::new(),
            checkpointer: None,
            kv_cache_handles: None,
        }
    }

    /// Chapter Ensemble — attach per-member backend overrides (role → its own
    /// provider + model). Members not in the map use the shared default.
    pub fn with_member_backends(
        mut self,
        backends: std::collections::HashMap<String, SpecialistBackend>,
    ) -> Self {
        self.member_backends = backends;
        self
    }

    /// Wire team dialogue: every specialist `build`-t hereafter also gets its
    /// own message tools on `bus` (J.5 roster wiring).
    pub fn with_dialogue(mut self, bus: Arc<MessageBus>, dialogue: DialogueConfig) -> Self {
        self.dialogue = Some((bus, dialogue));
        self
    }

    /// Attach an `aivyx-checkpoint` `GitCheckpointer` to every specialist
    /// this factory builds. `None` means "no checkpointer" (checkpointing
    /// disabled, or `fs_root` isn't a git repository), preserving
    /// pre-checkpoint behavior — same shape as `ConcreteAgent::with_checkpointer`.
    pub fn with_checkpointer(
        mut self,
        checkpointer: Option<Arc<aivyx_core::GitCheckpointer>>,
    ) -> Self {
        self.checkpointer = checkpointer;
        self
    }

    /// Attach the shared kvcache pool/store to every specialist this
    /// factory builds. `None` means "no kvcache" (provider isn't
    /// llama-server, or the `/props` probe failed), preserving
    /// pre-kvcache behavior -- same shape as `with_checkpointer`.
    pub fn with_kv_cache(
        mut self,
        kv_cache_handles: Option<(
            Arc<aivyx_llm::KvSlotPool>,
            Arc<aivyx_kvcache::LlamaServerSlotStore>,
            String,
        )>,
    ) -> Self {
        self.kv_cache_handles = kv_cache_handles;
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
        // Chapter Ensemble — use this member's backend override if it declared
        // one, else the team's shared provider + model.
        let backend = self.member_backends.get(&member.name);
        let provider = backend
            .map(|b| Arc::clone(&b.provider))
            .unwrap_or_else(|| Arc::clone(&self.provider));
        let model = backend
            .map(|b| b.model.clone())
            .unwrap_or_else(|| self.model.clone());
        let registry_for_planner = Arc::clone(&registry);
        let max_tokens = self.max_tokens;
        let soul = member.soul.clone();
        let kv_cache_handles = self.kv_cache_handles.clone();

        let agent = ConcreteAgent::new(
            AgentId::new(),
            caps,
            registry,
            Arc::clone(&self.audit),
            move || {
                let cfg = LlmPlannerConfig::new(&model)
                    .with_system_prompt(&soul)
                    .with_max_tokens(max_tokens);
                let planner = LlmPlanner::new(
                    Arc::clone(&provider),
                    Arc::clone(&registry_for_planner),
                    cfg,
                );
                let planner = match &kv_cache_handles {
                    Some((pool, store, build_hash)) => planner.with_kv_cache(
                        Arc::clone(pool),
                        Arc::clone(store),
                        "llama-server".to_string(),
                        model.clone(),
                        build_hash.clone(),
                    ),
                    None => planner,
                };
                Box::new(planner)
            },
        )
        .with_checkpointer(self.checkpointer.clone());
        // Team specialists run autonomously inside a mission — no human watches
        // each turn to `/cancel` a runaway — so they take the autonomous safety
        // posture: the small-cycle breaker as a built-in floor (always on, like
        // `MAX_STEPS_PER_TURN`), independent of the interactive `[agent]
        // cycle_detection` knob.
        Ok(TurnSafety::autonomous().apply(agent))
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
    // "mcp.call" is a MARKER entry, not a tool name: MCP-bridged tools carry
    // their server-native names (get_metar, web_search, …) which a static
    // roster cannot enumerate, so an exact-name allowlist could never admit
    // them (live rig 2026-07-05: the whole team was blind to the operator's
    // configured MCP servers). The marker admits every tool whose required
    // scope base is `mcp.call`.
    let admit_mcp = allowlist.iter().any(|a| a == "mcp.call");
    base.iter()
        .filter(|t| {
            allowlist.iter().any(|a| a.as_str() == t.name())
                || (admit_mcp
                    && t.required_scope(&serde_json::Value::Null).base()
                        == "mcp.call")
        })
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
            model: None,
            base_url: None,
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

    /// A fake MCP-bridged tool: server-native name, `mcp.call` scope.
    struct FakeMcpTool(ToolId, &'static str);
    #[async_trait]
    impl Tool for FakeMcpTool {
        fn id(&self) -> ToolId {
            self.0
        }
        fn name(&self) -> &str {
            self.1
        }
        fn description(&self) -> &str {
            "fake mcp"
        }
        fn input_schema(&self) -> &serde_json::Value {
            use std::sync::OnceLock;
            static S: OnceLock<serde_json::Value> = OnceLock::new();
            S.get_or_init(|| serde_json::json!({ "type": "object" }))
        }
        fn required_scope(&self, _: &serde_json::Value) -> Scope {
            Scope::parse("mcp.call:aviation-weather:get_metar").unwrap()
        }
        async fn execute(&self, _: serde_json::Value, _: &ToolContext<'_>) -> ToolOutcome {
            unreachable!("filter tests never execute a tool")
        }
    }

    #[test]
    fn mcp_marker_admits_bridged_tools_without_naming_them() {
        // MCP tool names are server-native and dynamic — the roster can't
        // enumerate them; the "mcp.call" marker admits them by scope base.
        let base: Vec<Arc<dyn Tool>> = vec![
            fake("a"),
            Arc::new(FakeMcpTool(ToolId::new(), "get_metar")),
        ];
        let with_marker = filter_tools(&base, &["mcp.call".into()]);
        let kept: Vec<&str> = with_marker.iter().map(|t| t.name()).collect();
        assert_eq!(kept, ["get_metar"]);
        // Without the marker the bridged tool stays hidden (least privilege),
        // and the marker never admits non-MCP tools.
        assert!(filter_tools(&base, &["b".into()]).is_empty());
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

    /// End-to-end proof that `SpecialistFactory::build` actually wires the
    /// checkpointer into `ConcreteAgent::new(...).with_checkpointer(...)` —
    /// not just that `build()` still returns `Ok` with `None` (that would
    /// pass identically whether the wiring exists or not, since `new`
    /// already defaults `checkpointer` to `None`).
    ///
    /// Unlike the three channel crates (Discord/Slack/Telegram), whose
    /// channels are hardcoded `TrustTier::SemiTrusted` and so can never
    /// legitimately hold `fs.write` (`CEILING_TRUSTED`-only), a team
    /// specialist genuinely can: real mission channels
    /// (`MissionLeadChannel` / `MissionChannel`) are `TrustTier::Trusted`,
    /// same as this file's own `member(...)` helper defaults to. So this
    /// test drives a real `fs.write` call — no `checkpoint.probe`-style
    /// stand-in needed — through a real `SpecialistFactory::build`-
    /// constructed `ConcreteAgent`, mirroring aivyx-core's own
    /// `checkpoint_fires_only_for_mutates_fs_root_tools` precedent
    /// (agent.rs) one level up the stack.
    #[tokio::test]
    async fn build_attaches_the_checkpointer_when_configured() {
        use crate::testutil::{FakeLeadChannel, FakeProvider};
        use aivyx_core::{ChannelContext, Message};

        // A real git-backed fs_root the specialist is allowed to write under.
        let dir = tempfile::tempdir().unwrap();
        aivyx_checkpoint::test_support::init_repo(dir.path()).await;
        let fs_root = dir.path().to_path_buf();

        let write_tool: Arc<dyn Tool> = Arc::new(
            aivyx_core::tools::fs::FsWriteToolConfig::new(fs_root.clone())
                .build()
                .expect("fs_root must be canonicalizable"),
        );

        let checkpointer = Arc::new(
            aivyx_checkpoint::GitCheckpointer::detect(&fs_root, vec![])
                .await
                .expect("fs_root is a real git repo"),
        );

        // The lead grants fs.write under fs_root; the specialist declares
        // the same scope (mission channels/specialists are Trusted, unlike
        // the SemiTrusted-ceilinged channel crates, so this is legitimate).
        let write_scope = format!("fs.write:{}/**", fs_root.display());
        let lead = CapabilitySet::from_scopes([Scope::parse(&write_scope).unwrap()]);
        let m = member("spec", &[write_scope.as_str()], &["fs.write"]);

        let provider = FakeProvider::tool_call_then_done(
            "fs.write",
            serde_json::json!({ "path": "new.txt", "content": "hello" }),
        );
        let f = SpecialistFactory::new(provider, "test-model", 4096, Arc::new(NullAuditHook), vec![write_tool])
            .with_checkpointer(Some(checkpointer));

        let specialist = f.build(&m, &lead).expect("build");

        let channel = FakeLeadChannel::at(TrustTier::Trusted);
        let message = Message::text(channel.session_id(), "write a file");
        let _ = specialist.turn(message, &channel).await;

        let refs = aivyx_checkpoint::test_support::git(
            dir.path(),
            &["for-each-ref", "refs/aivyx/checkpoints/"],
        )
        .await;
        let ref_count = refs.lines().filter(|l| !l.is_empty()).count();
        assert_eq!(
            ref_count, 1,
            "the specialist's fs.write must produce exactly one checkpoint \
             — proves SpecialistFactory::build actually wired the \
             checkpointer through, not just that build() tolerates None: {refs}"
        );
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

    // --- the autonomous cycle-breaker floor (end-to-end) -------------------

    /// An *executing* fake tool — the `FakeTool` above panics in `execute`
    /// because the construction tests never run a turn. This one completes so a
    /// real turn can dispatch it.
    struct ExecTool(ToolId, &'static str);
    #[async_trait]
    impl Tool for ExecTool {
        fn id(&self) -> ToolId {
            self.0
        }
        fn name(&self) -> &str {
            self.1
        }
        fn description(&self) -> &str {
            "exec"
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
            ToolOutcome::Completed {
                output: serde_json::json!({ "ok": true }),
                verified: aivyx_core::Verification::NotApplicable,
            }
        }
    }

    /// End-to-end proof of the team safety floor: a specialist built through the
    /// real `SpecialistFactory` — with NO `[agent] cycle_detection` configured
    /// anywhere — stops an alternating `a,b,a,b,…` tool loop with
    /// `TurnOutcome::Looping`. That only happens if `TurnSafety::autonomous`
    /// armed the small-cycle breaker as a built-in floor (the consecutive
    /// breaker resets on the alternation, and the 32-step cap is never reached).
    #[tokio::test]
    async fn specialist_trips_the_autonomous_cycle_floor() {
        use crate::testutil::{FakeLeadChannel, FakeProvider};
        use aivyx_core::{Agent, ChannelContext, Message, TurnOutcome};

        let base: Vec<Arc<dyn Tool>> = vec![
            Arc::new(ExecTool(ToolId::new(), "a")),
            Arc::new(ExecTool(ToolId::new(), "b")),
        ];
        // Period-2 cycle × 3 repeats trips at the 6th call (the default floor);
        // a couple of extra scripted steps are harmless (never reached).
        let provider = FakeProvider::tool_loop(&["a", "b", "a", "b", "a", "b", "a", "b"]);
        let factory =
            SpecialistFactory::new(provider, "test-model", 4096, Arc::new(NullAuditHook), base);
        let lead_caps = CapabilitySet::from_scopes([Scope::parse("fs.read").unwrap()]);
        let specialist = factory
            .build(&member("spec", &["fs.read"], &["a", "b"]), &lead_caps)
            .expect("specialist builds");

        let channel = FakeLeadChannel::at(TrustTier::Trusted);
        let outcome = specialist
            .turn(Message::text(channel.session_id(), "go"), &channel)
            .await;

        match outcome {
            // Period-2 × 3-repeats trips on the 6th call, before it dispatches —
            // so exactly 5 ran. This pins it to the small-cycle floor: the
            // consecutive breaker can't fire on an alternation, and the 32-step
            // cap is nowhere near.
            TurnOutcome::Looping {
                tool_calls_made, ..
            } => {
                assert_eq!(tool_calls_made, 5, "tripped at the 6th (cycle) call");
            }
            other => panic!(
                "the autonomous cycle floor must stop an alternating loop even \
                 with no [agent] config; got {other:?}"
            ),
        }
    }
}
