//! Phase 112 Task 7 — End-to-end scripted test for the skill
//! auto-proposer pipeline.
//!
//! Exercises [`run_auto_propose_pipeline`] against a real
//! Persona log + Persona proposal log + audit log, with a
//! scripted `LlmProvider` returning a deterministic verdict.
//! Confirms the chain-write integration that the unit tests
//! couldn't reach: the auto-accept path appends a Pending
//! entry, a PersonaDelta, AND an Approved entry, then
//! recomputes the shared persona so the next turn sees the
//! new skill rendered in its system prompt.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use aivyx_audit::{AuditEvent, AuditLog, PersistentAuditLog};
use aivyx_channel::persona::{
    self, EffectivePersona, PersistentPersonaLog, SharedEffectivePersona,
};
use aivyx_channel::persona_proposal::PersistentPersonaProposalLog;
use aivyx_channel::skill_auto_proposer::{
    run_auto_propose_pipeline, SkillAutoProposeConfig, SkillAutoProposerContext,
    TurnSignals,
};
use aivyx_core::{CancellationToken, SessionId};
use aivyx_crypto::MasterKey;
use aivyx_llm::{
    ContentBlock, LlmError, LlmMessage, LlmProvider, LlmRequest, LlmStepEnd,
    LlmStream, LlmStreamEvent, LlmUsage,
};
use aivyx_storage::{KeyDomain, RedbStorage, Storage, StorageConfig};
use async_trait::async_trait;

// ---------------------------------------------------------------------------
// Scripted LLM provider (mirrors the in-module test fixture so the e2e
// stays self-contained).
// ---------------------------------------------------------------------------

struct ScriptedProvider {
    responses: Mutex<VecDeque<String>>,
}

impl ScriptedProvider {
    fn new(responses: Vec<&str>) -> Arc<Self> {
        Arc::new(ScriptedProvider {
            responses: Mutex::new(
                responses.into_iter().map(|s| s.to_string()).collect(),
            ),
        })
    }
}

#[async_trait]
impl LlmProvider for ScriptedProvider {
    async fn chat_stream(
        &self,
        _request: LlmRequest<'_>,
        _cancellation: &CancellationToken,
    ) -> Result<Box<dyn LlmStream>, LlmError> {
        let text = self.responses.lock().unwrap().pop_front().ok_or_else(
            || LlmError::Config("ScriptedProvider exhausted".to_string()),
        )?;
        Ok(Box::new(ScriptedStream { text: Some(text) }))
    }
}

struct ScriptedStream {
    text: Option<String>,
}

#[async_trait]
impl LlmStream for ScriptedStream {
    async fn next_event(&mut self) -> Result<Option<LlmStreamEvent>, LlmError> {
        Ok(None)
    }
    async fn finish(self: Box<Self>) -> Result<LlmStepEnd, LlmError> {
        Ok(LlmStepEnd::FinalMessage {
            text: self.text.unwrap_or_default(),
            usage: LlmUsage::default(),
        })
    }
}

// Silence the imports-not-used warning on ContentBlock + LlmMessage that
// we don't directly use here — the scripted provider never has to
// construct them; they're available for callers that do.
#[allow(dead_code)]
fn _smoke() {
    let _: LlmMessage = LlmMessage::User {
        content: vec![ContentBlock::Text { text: "x".into() }],
    };
}

// ---------------------------------------------------------------------------
// Storage helper — uses std::env::temp_dir + a uuid so the workspace
// stays off the tempfile crate (see cli_e2e.rs / fs_tool_e2e.rs /
// storage_persistence_e2e.rs for the same pattern).
// ---------------------------------------------------------------------------

struct ScratchStorage {
    _parent: std::path::PathBuf,
    handle: Arc<dyn Storage>,
}

async fn scratch_storage() -> ScratchStorage {
    let parent = std::env::temp_dir().join(format!(
        "aivyx-phase112-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&parent).expect("create scratch dir");
    let storage: Arc<dyn Storage> = RedbStorage::open(
        StorageConfig::new(parent.join("store.redb")),
        MasterKey::from_raw([42u8; 32]),
    )
    .await
    .expect("scratch storage opens");
    ScratchStorage {
        _parent: parent,
        handle: storage,
    }
}

fn test_chain_key() -> Vec<u8> {
    vec![0xAB; 32]
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn fire_threshold_signals() -> TurnSignals {
    TurnSignals {
        tool_calls_made: 5,
        distinct_tool_id_count: 3,
        duration: std::time::Duration::from_millis(8_000),
        had_successful_gate_resolve: false,
    }
}

const AUTO_ACCEPT_VERDICT_JSON: &str = r#"{
    "is_worth_proposing": true,
    "confidence": 0.92,
    "category": "LearnedSkill",
    "proposed_draft": {
        "kind": "LearnedSkill",
        "name": "research-multi-source",
        "trigger": "user asks to research a topic across multiple sources",
        "procedure": "1. fs.read project notes\n2. web.fetch each candidate URL\n3. summarize"
    },
    "is_duplicate_of": null,
    "reasoning": "multi-step recurring research pattern"
}"#;

const STAGED_VERDICT_JSON: &str = r#"{
    "is_worth_proposing": true,
    "confidence": 0.72,
    "category": "LearnedSkill",
    "proposed_draft": {
        "kind": "LearnedSkill",
        "name": "borderline-skill",
        "trigger": "uncertain",
        "procedure": "..."
    },
    "is_duplicate_of": null,
    "reasoning": "useful but not high-confidence"
}"#;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn high_confidence_verdict_auto_accepts_into_persona_chain() {
    let storage = scratch_storage().await;
    let persona_log = Arc::new(
        PersistentPersonaLog::open(
            storage.handle.domain(KeyDomain::Persona),
            test_chain_key(),
        )
        .await
        .expect("persona log opens"),
    );
    let proposal_log = Arc::new(
        PersistentPersonaProposalLog::open(
            storage.handle.domain(KeyDomain::PersonaProposals),
            test_chain_key(),
        )
        .await
        .expect("proposal log opens"),
    );
    let audit_log = Arc::new(
        PersistentAuditLog::open(
            Arc::clone(&storage.handle),
            [0xABu8; 32],
        )
        .await
        .expect("audit log opens"),
    );
    let shared: SharedEffectivePersona =
        persona::shared_effective_persona(EffectivePersona::default());

    let proposer_ctx = Arc::new(SkillAutoProposerContext {
        config: SkillAutoProposeConfig::default(),
        llm_provider: ScriptedProvider::new(vec![AUTO_ACCEPT_VERDICT_JSON]),
    });

    let session_id = SessionId::new();
    let cancel = CancellationToken::new();

    run_auto_propose_pipeline(
        &proposer_ctx,
        Some(&audit_log),
        Some(&persona_log),
        Some(&proposal_log),
        &shared,
        session_id,
        fire_threshold_signals(),
        "User asked X; agent did Y; reply Z.".into(),
        &cancel,
    )
    .await;

    // Persona chain: one PersonaDelta of category LearnedSkill
    assert_eq!(persona_log.len(), 1, "persona chain should have 1 entry");

    // Proposal chain: pending + approved = 2 entries
    assert_eq!(
        proposal_log.len(),
        2,
        "proposal chain should have pending+approved"
    );

    // Audit log: one SkillAutoProposal event with AutoAccepted outcome
    assert_eq!(AuditLog::len(audit_log.as_ref()), 1);
    let entries = audit_log.entries().expect("entries fetch");
    let event = &entries[0].event;
    match event {
        AuditEvent::SkillAutoProposal {
            outcome,
            proposed_skill_name,
            confidence_thousandths,
            ..
        } => {
            assert!(matches!(
                outcome,
                aivyx_audit::SkillAutoProposalOutcomeSummary::AutoAccepted
            ));
            assert_eq!(
                proposed_skill_name.as_deref(),
                Some("research-multi-source")
            );
            assert_eq!(*confidence_thousandths, Some(920));
        }
        other => panic!("expected SkillAutoProposal; got {other:?}"),
    }

    // Shared persona: learned_skills now has one entry.
    let state = shared.read().unwrap();
    assert_eq!(
        state.learned_skills.len(),
        1,
        "shared persona must reflect the new skill"
    );
    let parsed = persona::LearnedSkill::from_json_value(&state.learned_skills[0])
        .expect("learned-skill JSON parses");
    assert_eq!(parsed.name, "research-multi-source");
}

#[tokio::test]
async fn borderline_confidence_stages_into_proposal_chain_only() {
    let storage = scratch_storage().await;
    let persona_log = Arc::new(
        PersistentPersonaLog::open(
            storage.handle.domain(KeyDomain::Persona),
            test_chain_key(),
        )
        .await
        .expect("persona log opens"),
    );
    let proposal_log = Arc::new(
        PersistentPersonaProposalLog::open(
            storage.handle.domain(KeyDomain::PersonaProposals),
            test_chain_key(),
        )
        .await
        .expect("proposal log opens"),
    );
    let audit_log = Arc::new(
        PersistentAuditLog::open(
            Arc::clone(&storage.handle),
            [0xABu8; 32],
        )
        .await
        .expect("audit log opens"),
    );
    let shared: SharedEffectivePersona =
        persona::shared_effective_persona(EffectivePersona::default());

    let proposer_ctx = Arc::new(SkillAutoProposerContext {
        config: SkillAutoProposeConfig::default(),
        llm_provider: ScriptedProvider::new(vec![STAGED_VERDICT_JSON]),
    });

    let session_id = SessionId::new();
    let cancel = CancellationToken::new();

    run_auto_propose_pipeline(
        &proposer_ctx,
        Some(&audit_log),
        Some(&persona_log),
        Some(&proposal_log),
        &shared,
        session_id,
        fire_threshold_signals(),
        "summary".into(),
        &cancel,
    )
    .await;

    // Persona chain: STAYS EMPTY — staged proposals don't write the delta.
    assert_eq!(
        persona_log.len(),
        0,
        "persona chain should NOT have entries for a staged proposal"
    );

    // Proposal chain: one Pending entry awaiting operator approval.
    assert_eq!(
        proposal_log.len(),
        1,
        "proposal chain should have only the Pending entry"
    );

    // Audit log: Staged outcome.
    let entries = audit_log.entries().expect("entries fetch");
    let event = &entries[0].event;
    match event {
        AuditEvent::SkillAutoProposal {
            outcome,
            proposed_skill_name,
            ..
        } => {
            assert!(matches!(
                outcome,
                aivyx_audit::SkillAutoProposalOutcomeSummary::Staged
            ));
            assert_eq!(
                proposed_skill_name.as_deref(),
                Some("borderline-skill")
            );
        }
        other => panic!("expected SkillAutoProposal; got {other:?}"),
    }

    // Shared persona stays empty.
    let state = shared.read().unwrap();
    assert!(state.learned_skills.is_empty());
}

#[tokio::test]
async fn heuristic_gate_short_circuits_skip_writes_audit_only() {
    let storage = scratch_storage().await;
    let persona_log = Arc::new(
        PersistentPersonaLog::open(
            storage.handle.domain(KeyDomain::Persona),
            test_chain_key(),
        )
        .await
        .expect("persona log opens"),
    );
    let proposal_log = Arc::new(
        PersistentPersonaProposalLog::open(
            storage.handle.domain(KeyDomain::PersonaProposals),
            test_chain_key(),
        )
        .await
        .expect("proposal log opens"),
    );
    let audit_log = Arc::new(
        PersistentAuditLog::open(
            Arc::clone(&storage.handle),
            [0xABu8; 32],
        )
        .await
        .expect("audit log opens"),
    );
    let shared: SharedEffectivePersona =
        persona::shared_effective_persona(EffectivePersona::default());

    // Empty scripted provider — heuristic gate must short-circuit
    // before any LLM call would fire ("exhausted" would otherwise
    // surface as a JudgeError audit event).
    let proposer_ctx = Arc::new(SkillAutoProposerContext {
        config: SkillAutoProposeConfig::default(),
        llm_provider: ScriptedProvider::new(vec![]),
    });

    let below = TurnSignals {
        tool_calls_made: 1,
        distinct_tool_id_count: 1,
        duration: std::time::Duration::from_millis(800),
        had_successful_gate_resolve: false,
    };

    let session_id = SessionId::new();
    let cancel = CancellationToken::new();

    run_auto_propose_pipeline(
        &proposer_ctx,
        Some(&audit_log),
        Some(&persona_log),
        Some(&proposal_log),
        &shared,
        session_id,
        below,
        "trivial chat".into(),
        &cancel,
    )
    .await;

    assert_eq!(persona_log.len(), 0);
    assert_eq!(proposal_log.len(), 0);
    let entries = audit_log.entries().expect("entries fetch");
    assert_eq!(entries.len(), 1);
    match &entries[0].event {
        AuditEvent::SkillAutoProposal {
            outcome,
            judge_latency_ms,
            ..
        } => {
            assert!(matches!(
                outcome,
                aivyx_audit::SkillAutoProposalOutcomeSummary::HeuristicGated
            ));
            assert!(
                judge_latency_ms.is_none(),
                "no judge call → no latency"
            );
        }
        other => panic!("expected SkillAutoProposal; got {other:?}"),
    }
}
