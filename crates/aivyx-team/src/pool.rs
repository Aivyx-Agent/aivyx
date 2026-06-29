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
}

impl SpecialistChannel {
    pub fn new(
        session_id: SessionId,
        trust_tier: TrustTier,
        platform: ChannelPlatform,
        cancellation: CancellationToken,
    ) -> Self {
        SpecialistChannel {
            session_id,
            trust_tier,
            platform,
            cancellation,
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
    async fn stream_event(&self, _event: StreamEvent<'_>) -> Result<(), ChannelError> {
        Ok(()) // specialist progress isn't relayed to the operator (J.7)
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
    lead_caps: CapabilitySet,
}

impl SpecialistPool {
    pub fn new(factory: SpecialistFactory, config: TeamConfig, lead_caps: CapabilitySet) -> Self {
        SpecialistPool {
            factory,
            config,
            lead_caps,
        }
    }

    /// Resolve a specialist by name — must be a member, and not the lead.
    fn resolve(&self, specialist: &str) -> Result<&TeamMember, TeamError> {
        if specialist == self.config.lead {
            return Err(TeamError::Config(format!(
                "{specialist:?} is the lead, not a delegable specialist"
            )));
        }
        self.config
            .members
            .iter()
            .find(|m| m.name == specialist)
            .ok_or_else(|| {
                TeamError::Config(format!(
                    "no specialist {specialist:?} in team {:?}",
                    self.config.name
                ))
            })
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
        let agent = self.factory.build(member, &self.lead_caps)?;
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
}
