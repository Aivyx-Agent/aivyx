//! `SpecialistChannel` + `SpecialistPool` — run an attenuated specialist as
//! a **sub-turn** inside the lead's session (J.2.2).
//!
//! The `SpecialistChannel` is the mechanism that makes multi-agent work on
//! the single-agent loop: it's a thin [`ChannelContext`] that gives a
//! specialist the **lead's** session / cancellation / platform, but reports
//! a **floored trust tier** (`effective_trust(member.ceiling, lead_tier)`).
//! Because the core's turn loop already computes `caps ∩ tier.ceiling()`
//! every turn, the specialist ends up **double-bounded** — attenuated caps
//! *and* the floored tier ceiling — with zero new enforcement code.
//!
//! Output flows back via the returned [`TurnOutcome::Completed`], so the
//! channel's `stream_event` / `finalize` are no-ops (operator-facing
//! streaming of specialist progress is the J.7 Fleet panel).

use aivyx_capability::{CapabilitySet, TrustTier};
use aivyx_core::{
    Agent, CancellationToken, ChannelContext, ChannelError, ChannelPlatform, Message, SessionId,
    StreamEvent, TurnOutcome,
};
use async_trait::async_trait;

use crate::attenuation::effective_trust;
use crate::config::{TeamConfig, TeamError, TeamMember};
use crate::factory::SpecialistFactory;

/// A derived channel for one specialist sub-turn (see module docs).
pub struct SpecialistChannel {
    session_id: SessionId,
    trust_tier: TrustTier,
    platform: ChannelPlatform,
    cancellation: CancellationToken,
    /// Chapter Spyglass — the specialist's name, so its tool calls are
    /// legible in the journal (a mission's inner work used to be a black box).
    label: String,
}

impl SpecialistChannel {
    pub fn new(
        session_id: SessionId,
        trust_tier: TrustTier,
        platform: ChannelPlatform,
        cancellation: CancellationToken,
        label: impl Into<String>,
    ) -> Self {
        SpecialistChannel {
            session_id,
            trust_tier,
            platform,
            cancellation,
            label: label.into(),
        }
    }
}

#[async_trait]
impl ChannelContext for SpecialistChannel {
    fn channel_name(&self) -> &str {
        "specialist"
    }
    fn platform(&self) -> ChannelPlatform {
        self.platform
    }
    fn trust_tier(&self) -> TrustTier {
        self.trust_tier
    }
    fn session_id(&self) -> SessionId {
        self.session_id
    }
    async fn stream_event(&self, event: StreamEvent<'_>) -> Result<(), ChannelError> {
        // Chapter Spyglass — surface a specialist's TOOL activity in the journal
        // so a mission's inner work is observable (previously a black box that
        // made "reports done but produced nothing" hard to diagnose). Only the
        // tool start/finish pair is logged; token text stays quiet. The live
        // operator-facing Fleet panel (J.7) is still deferred.
        match event {
            StreamEvent::ToolCallStarted { tool_name, .. } => {
                eprintln!("aivyx team: [{}] → {tool_name}", self.label);
            }
            StreamEvent::ToolCallFinished { tool_name, outcome_summary, .. } => {
                let summary: String = outcome_summary.chars().take(120).collect();
                eprintln!("aivyx team: [{}] ← {tool_name} — {summary}", self.label);
            }
            _ => {}
        }
        Ok(())
    }
    async fn finalize(&self, _outcome: &TurnOutcome) -> Result<(), ChannelError> {
        Ok(()) // the lead reads the result from run()'s return value
    }
    fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }
}

/// Constructs + runs attenuated specialists for one team.
pub struct SpecialistPool {
    factory: SpecialistFactory,
    config: TeamConfig,
    ceiling: CapabilitySet,
}

impl SpecialistPool {
    pub fn new(factory: SpecialistFactory, config: TeamConfig, ceiling: CapabilitySet) -> Self {
        SpecialistPool {
            factory,
            config,
            ceiling,
        }
    }

    /// Resolve a specialist reference — must be a member, and not the lead.
    ///
    /// Matching is **case-insensitive** and accepts EITHER the roster `name`
    /// id (`ops`) OR the human-readable `role` label (`Operations`). The LLM
    /// planner refers to specialists by the role labels it's shown, not the
    /// internal ids, so a name-only match errored ("no specialist
    /// \"Operations\"") → the mission failed → the loop's auto-delegation
    /// retried and skipped a doable story. #15 first made this
    /// case-insensitive (fixing "Researcher" vs "researcher", where name and
    /// role differ only in case); this extends it to name↔role mismatches
    /// (`ops`/`Operations`, `coordinator`/`Lead`) which case-folding alone
    /// never covered. Both fields are short ASCII, so case-insensitive
    /// matching is safe.
    fn resolve(&self, specialist: &str) -> Result<&TeamMember, TeamError> {
        let matched = self.config.members.iter().find(|m| {
            m.name.eq_ignore_ascii_case(specialist)
                || m.role.eq_ignore_ascii_case(specialist)
        });
        match matched {
            // The lead is identified by its roster id in `config.lead`; a
            // reference resolving to that member (by name or role) is the
            // lead, not a delegable specialist.
            Some(m) if m.name.eq_ignore_ascii_case(&self.config.lead) => {
                Err(TeamError::Config(format!(
                    "{specialist:?} is the lead, not a delegable specialist"
                )))
            }
            Some(m) => Ok(m),
            // Graceful fallback: the LLM planner sometimes names a specialist
            // that isn't in the roster (a generic role word like "Operations"
            // or "QA Engineer"). Hard-failing the whole delegation there just
            // skips a doable story (seen live in the loop dogfood, sharpened by
            // the ops→verifier rename). Instead, map the requested word to a
            // capability and route to the best-fit specialist — the acceptance
            // gate (Keystone) still catches a bad result, so a best-effort
            // attempt strictly beats an abandoned story.
            None => match self.fallback_specialist(specialist) {
                Some(m) => {
                    eprintln!(
                        "aivyx team: planner named unknown specialist \
                         {specialist:?}; routing to best-fit {:?}",
                        m.name
                    );
                    Ok(m)
                }
                None => Err(TeamError::Config(format!(
                    "no specialist {specialist:?} in team {:?}",
                    self.config.name
                ))),
            },
        }
    }

    /// Best-effort recovery when an exact name/role match fails: map a generic
    /// role word the planner reached for to a capability, then pick the roster
    /// member that best provides it. Capability-based rather than name-based so
    /// it works for ANY roster (incl. a vertical pack's), and prefers the
    /// least-privilege fit (e.g. a run-only "operations" step goes to a
    /// shell-but-not-write specialist over the coder). `None` when nothing
    /// sensible fits — the caller then errors as before.
    fn fallback_specialist(&self, specialist: &str) -> Option<&TeamMember> {
        let q = specialist.to_ascii_lowercase();
        let hit = |kws: &[&str]| kws.iter().any(|kw| q.contains(kw));
        // A member's declared scope bases (bare or qualified, `base` or
        // `base:qualifier`).
        let has = |m: &&TeamMember, base: &str| {
            m.capability_scopes
                .iter()
                .any(|s| s.split(':').next() == Some(base))
        };
        let specialists = || {
            self.config
                .members
                .iter()
                .filter(|m| !m.name.eq_ignore_ascii_case(&self.config.lead))
        };

        // Order matters: more specific capability wants are checked first so
        // e.g. "developer" routes to the coder, not merely any writer/runner.
        if hit(&["cod", "develop", "engineer", "program", "implement", "build"]) {
            if let Some(m) = specialists().find(|m| has(m, "fs.write") && has(m, "shell.exec")) {
                return Some(m);
            }
        }
        if hit(&["writ", "author", "scribe", "editor", "document", "content", "note"]) {
            // Prefer a pure writer (can write, no shell) over the coder, both
            // for semantic fit and least privilege; else any writer.
            if let Some(m) = specialists()
                .find(|m| (has(m, "fs.write") || has(m, "workspace")) && !has(m, "shell.exec"))
            {
                return Some(m);
            }
            if let Some(m) = specialists().find(|m| has(m, "fs.write") || has(m, "workspace")) {
                return Some(m);
            }
        }
        if hit(&[
            "operation", "ops", "devops", "sysadmin", "sre", "infra", "execut", "deploy", "run",
            "qa", "test", "verif", "validat",
        ]) {
            // Prefer a least-privilege runner (shell without write), else any.
            if let Some(m) = specialists().find(|m| has(m, "shell.exec") && !has(m, "fs.write")) {
                return Some(m);
            }
            if let Some(m) = specialists().find(|m| has(m, "shell.exec")) {
                return Some(m);
            }
        }
        if hit(&["research", "investigat", "gather", "search", "analy", "data", "fetch"]) {
            if let Some(m) = specialists().find(|m| has(m, "net.fetch") || has(m, "web.search")) {
                return Some(m);
            }
        }
        if hit(&["review", "critic", "audit", "inspect", "read"]) {
            if let Some(m) = specialists().find(|m| has(m, "fs.read")) {
                return Some(m);
            }
        }
        None
    }

    /// Build the derived channel for a specialist sub-turn — trust floored
    /// to `effective_trust(member.ceiling, lead_tier)`.
    pub fn specialist_channel(
        &self,
        member: &TeamMember,
        lead_channel: &dyn ChannelContext,
    ) -> SpecialistChannel {
        SpecialistChannel::new(
            lead_channel.session_id(),
            effective_trust(member.trust_ceiling, lead_channel.trust_tier()),
            lead_channel.platform(),
            lead_channel.cancellation_token(),
            member.name.clone(),
        )
    }

    /// Run a specialist on `task`, attenuated + trust-floored against the
    /// lead's live channel. Returns the specialist's final message.
    pub async fn run(
        &self,
        specialist: &str,
        task: &str,
        lead_channel: &dyn ChannelContext,
    ) -> Result<String, TeamError> {
        let member = self.resolve(specialist)?;
        let agent = self.factory.build(member, &self.ceiling)?;
        let channel = self.specialist_channel(member, lead_channel);
        let msg = Message::text(channel.session_id(), task);

        match agent.turn(msg, &channel).await {
            TurnOutcome::Completed { final_message, .. } => Ok(final_message),
            TurnOutcome::Escalated { reason, .. } => Err(TeamError::Config(format!(
                "specialist {specialist:?} escalated for approval: {reason}"
            ))),
            TurnOutcome::TimedOut { .. } => Err(TeamError::Config(format!(
                "specialist {specialist:?} timed out"
            ))),
            TurnOutcome::Cancelled { .. } => Err(TeamError::Config(format!(
                "specialist {specialist:?} was cancelled"
            ))),
            _ => Err(TeamError::Config(format!(
                "specialist {specialist:?} did not complete its turn"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DialogueConfig;

    #[tokio::test]
    async fn spyglass_channel_logs_tool_events_without_erroring() {
        // Chapter Spyglass — stream_event now surfaces specialist tool calls;
        // the channel accepts tool start/finish (and ignores token text) cleanly.
        let ch = SpecialistChannel::new(
            SessionId::new(),
            TrustTier::Trusted,
            ChannelPlatform::Local,
            CancellationToken::new(),
            "writer",
        );
        let id = aivyx_core::ToolId::new();
        let input = serde_json::json!({"path": "foo.md"});
        assert!(ch
            .stream_event(StreamEvent::ToolCallStarted {
                tool: id,
                tool_name: "workspace.write",
                input: &input,
            })
            .await
            .is_ok());
        assert!(ch
            .stream_event(StreamEvent::ToolCallFinished {
                tool: id,
                tool_name: "workspace.write",
                outcome_summary: "wrote foo.md",
            })
            .await
            .is_ok());
        assert!(ch.stream_event(StreamEvent::Text("thinking…")).await.is_ok());
    }
    use aivyx_capability::Scope;
    use aivyx_core::NullAuditHook;
    use aivyx_llm::{LlmError, LlmProvider, LlmRequest, LlmStepEnd, LlmStream, LlmStreamEvent};
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    // --- a working fake provider that drives one tool-less turn ----------

    struct FakeStep {
        events: Vec<LlmStreamEvent>,
        terminal: LlmStepEnd,
    }
    struct FakeProvider {
        script: Mutex<VecDeque<FakeStep>>,
    }
    impl FakeProvider {
        fn says(text: &str) -> Arc<Self> {
            let step = FakeStep {
                events: vec![LlmStreamEvent::TextChunk(text.to_string())],
                terminal: LlmStepEnd::FinalMessage {
                    text: text.to_string(),
                    usage: aivyx_llm::LlmUsage::default(),
                },
            };
            Arc::new(FakeProvider {
                script: Mutex::new(VecDeque::from(vec![step])),
            })
        }
    }
    #[async_trait]
    impl LlmProvider for FakeProvider {
        async fn chat_stream(
            &self,
            _: LlmRequest<'_>,
            _: &CancellationToken,
        ) -> Result<Box<dyn LlmStream>, LlmError> {
            let step = self
                .script
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| LlmError::Config("fake provider exhausted".into()))?;
            Ok(Box::new(FakeStream {
                events: step.events.into_iter(),
                terminal: Some(step.terminal),
            }))
        }
    }
    struct FakeStream {
        events: std::vec::IntoIter<LlmStreamEvent>,
        terminal: Option<LlmStepEnd>,
    }
    #[async_trait]
    impl LlmStream for FakeStream {
        async fn next_event(&mut self) -> Result<Option<LlmStreamEvent>, LlmError> {
            Ok(self.events.next())
        }
        async fn finish(mut self: Box<Self>) -> Result<LlmStepEnd, LlmError> {
            Ok(self.terminal.take().expect("finish once"))
        }
    }

    // --- a fake lead channel --------------------------------------------

    struct FakeLeadChannel {
        session: SessionId,
        tier: TrustTier,
        token: CancellationToken,
    }
    impl FakeLeadChannel {
        fn at(tier: TrustTier) -> Self {
            FakeLeadChannel {
                session: SessionId::new(),
                tier,
                token: CancellationToken::new(),
            }
        }
    }
    #[async_trait]
    impl ChannelContext for FakeLeadChannel {
        fn channel_name(&self) -> &str {
            "fake-lead"
        }
        fn platform(&self) -> ChannelPlatform {
            ChannelPlatform::Local
        }
        fn trust_tier(&self) -> TrustTier {
            self.tier
        }
        fn session_id(&self) -> SessionId {
            self.session
        }
        async fn stream_event(&self, _: StreamEvent<'_>) -> Result<(), ChannelError> {
            Ok(())
        }
        async fn finalize(&self, _: &TurnOutcome) -> Result<(), ChannelError> {
            Ok(())
        }
        fn cancellation_token(&self) -> CancellationToken {
            self.token.clone()
        }
    }

    // --- fixtures --------------------------------------------------------

    fn member(name: &str, scopes: &[&str], tier: TrustTier) -> TeamMember {
        TeamMember {
            name: name.into(),
            role: "R".into(),
            soul: "You are a specialist.".into(),
            tool_allowlist: vec![],
            capability_scopes: scopes.iter().map(|s| s.to_string()).collect(),
            trust_ceiling: tier,
            model: None,
            base_url: None,
        }
    }

    fn pool(provider: Arc<dyn LlmProvider>, members: Vec<TeamMember>, lead: &str) -> SpecialistPool {
        let config = TeamConfig {
            name: "t".into(),
            description: String::new(),
            lead: lead.into(),
            members,
            dialogue: DialogueConfig::default(),
        };
        let factory = SpecialistFactory::new(provider, "test-model", 4096, Arc::new(NullAuditHook), vec![]);
        let lead_caps = CapabilitySet::from_scopes([Scope::parse("fs.read").unwrap()]);
        SpecialistPool::new(factory, config, lead_caps)
    }

    // --- tests -----------------------------------------------------------

    #[test]
    fn specialist_channel_floors_trust_to_the_lead() {
        // Member declares Kernel; lead channel is Trusted → floored to Trusted.
        let p = pool(
            FakeProvider::says("x"),
            vec![
                member("lead", &[], TrustTier::Trusted),
                member("spec", &["fs.read"], TrustTier::Kernel),
            ],
            "lead",
        );
        let lead_ch = FakeLeadChannel::at(TrustTier::Trusted);
        let m = p.resolve("spec").unwrap();
        let ch = p.specialist_channel(m, &lead_ch);
        assert_eq!(ch.trust_tier(), TrustTier::Trusted, "Kernel floored to the lead");
        assert_eq!(ch.session_id(), lead_ch.session_id(), "shares the lead's session");
    }

    #[test]
    fn channel_floor_keeps_a_lower_member_ceiling() {
        let p = pool(
            FakeProvider::says("x"),
            vec![
                member("lead", &[], TrustTier::Trusted),
                member("spec", &["fs.read"], TrustTier::Trusted),
            ],
            "lead",
        );
        // Lead on a less-trusted channel → specialist floored below its ceiling.
        let lead_ch = FakeLeadChannel::at(TrustTier::SemiTrusted);
        let ch = p.specialist_channel(p.resolve("spec").unwrap(), &lead_ch);
        assert_eq!(ch.trust_tier(), TrustTier::SemiTrusted);
    }

    #[test]
    fn resolve_matches_specialist_names_case_insensitively() {
        // #15 — the planner emits "Researcher"/"RESEARCHER"; the roster is
        // "researcher". All must resolve to the member (else the mission errors).
        let p = pool(
            FakeProvider::says("x"),
            vec![
                member("lead", &[], TrustTier::Trusted),
                member("researcher", &["fs.read"], TrustTier::Trusted),
            ],
            "lead",
        );
        assert_eq!(p.resolve("researcher").unwrap().name, "researcher");
        assert_eq!(p.resolve("Researcher").unwrap().name, "researcher");
        assert_eq!(p.resolve("RESEARCHER").unwrap().name, "researcher");
        // The lead guard is case-insensitive too.
        assert!(p.resolve("Lead").is_err());
    }

    #[test]
    fn resolve_matches_by_role_label_not_just_name() {
        // The planner refers to specialists by their ROLE label ("Operations"),
        // but the roster id is "ops" — a real dogfood failure where the mission
        // errored "no specialist \"Operations\"" and the loop skipped a doable
        // story. Both the role AND the id must resolve.
        let mut ops = member("ops", &["shell.exec"], TrustTier::Trusted);
        ops.role = "Operations".into();
        let mut lead = member("coordinator", &[], TrustTier::Trusted);
        lead.role = "Lead".into();
        let p = pool(FakeProvider::says("x"), vec![lead, ops], "coordinator");

        // By role label (what the planner emits):
        assert_eq!(p.resolve("Operations").unwrap().name, "ops");
        assert_eq!(p.resolve("operations").unwrap().name, "ops");
        // By roster id still works:
        assert_eq!(p.resolve("ops").unwrap().name, "ops");
        // The lead is rejected whether referenced by id OR role label:
        assert!(matches!(
            p.resolve("coordinator"),
            Err(TeamError::Config(m)) if m.contains("is the lead")
        ));
        assert!(matches!(
            p.resolve("Lead"),
            Err(TeamError::Config(m)) if m.contains("is the lead")
        ));
    }

    #[test]
    fn cannot_delegate_to_the_lead_or_an_unknown_member() {
        let p = pool(
            FakeProvider::says("x"),
            vec![member("lead", &[], TrustTier::Trusted)],
            "lead",
        );
        assert!(matches!(p.resolve("lead"), Err(TeamError::Config(m)) if m.contains("is the lead")));
        assert!(matches!(p.resolve("ghost"), Err(TeamError::Config(m)) if m.contains("no specialist")));
    }

    #[tokio::test]
    async fn run_executes_a_specialist_sub_turn_and_returns_its_output() {
        let p = pool(
            FakeProvider::says("inventory looks healthy"),
            vec![
                member("lead", &[], TrustTier::Trusted),
                member("inventory", &["fs.read"], TrustTier::Trusted),
            ],
            "lead",
        );
        let lead_ch = FakeLeadChannel::at(TrustTier::Trusted);
        let out = p.run("inventory", "check stock", &lead_ch).await.unwrap();
        assert_eq!(out, "inventory looks healthy");
    }

    #[tokio::test]
    async fn run_rejects_delegating_to_the_lead() {
        let p = pool(
            FakeProvider::says("x"),
            vec![member("lead", &[], TrustTier::Trusted)],
            "lead",
        );
        let lead_ch = FakeLeadChannel::at(TrustTier::Trusted);
        assert!(p.run("lead", "task", &lead_ch).await.is_err());
    }

    #[test]
    fn resolve_falls_back_to_best_fit_for_an_unknown_specialist() {
        // The planner sometimes names a specialist that isn't in the roster (a
        // generic role word). Rather than hard-fail the story, resolve routes to
        // the best-fit member BY CAPABILITY. Uses the real default Nonagon.
        let p = pool(
            FakeProvider::says("x"),
            crate::roster::default_nonagon().members,
            "coordinator",
        );

        // "Operations" is no longer a role (ops→verifier); a run/inspect word
        // maps to a shell-capable, least-privilege (non-writing) runner.
        assert_eq!(p.resolve("Operations").unwrap().name, "verifier");
        assert_eq!(p.resolve("Tester").unwrap().name, "verifier");
        // A code word → the write+shell coder.
        assert_eq!(p.resolve("Developer").unwrap().name, "coder");
        // A writing word → a pure writer (not the coder), least-privilege.
        assert_eq!(p.resolve("Technical Author").unwrap().name, "writer");
        // A research word → a fetch-capable member.
        assert_eq!(p.resolve("Investigator").unwrap().name, "researcher");
        // No sensible capability fit still errors (unchanged behaviour).
        assert!(matches!(
            p.resolve("Astrologer"),
            Err(TeamError::Config(m)) if m.contains("no specialist")
        ));
        // The lead guard is untouched — naming the lead is still an error, not
        // a fallback.
        assert!(p.resolve("coordinator").is_err());
    }
}
