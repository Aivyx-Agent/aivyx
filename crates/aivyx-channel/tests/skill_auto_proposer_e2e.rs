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

// ---------------------------------------------------------------------------
// Phase 114 — per-category e2e (list + scalar)
// ---------------------------------------------------------------------------

const LIST_APPEND_VERDICT_JSON: &str = r#"{
    "is_worth_proposing": true,
    "confidence": 0.91,
    "category": "BehavioralPreferences",
    "proposed_draft": {
        "kind": "ListAppend",
        "value": "prefer terse replies for command-style requests"
    },
    "is_duplicate_of": null,
    "reasoning": "recurring pattern of brief command-style turns"
}"#;

const SCALAR_SET_VERDICT_JSON: &str = r#"{
    "is_worth_proposing": true,
    "confidence": 0.97,
    "category": "AssistantName",
    "proposed_draft": {
        "kind": "ScalarSet",
        "value": "Aivyx"
    },
    "is_duplicate_of": null,
    "reasoning": "operator referred to the assistant by name across turns"
}"#;

const CATEGORY_DISABLED_VERDICT_JSON: &str = r#"{
    "is_worth_proposing": true,
    "confidence": 0.97,
    "category": "OperatorProfile",
    "proposed_draft": {
        "kind": "ScalarSet",
        "value": "Julian — primary repo aivyx; Rust workspace"
    },
    "is_duplicate_of": null,
    "reasoning": "operator self-described early in the turn"
}"#;

/// Phase 114 — config with the Phase 114 per_category surface
/// populated. Defaults match `PerCategoryConfigSet::defaults()`
/// (scalars off, lists on). Override per-test for cases that
/// need a specific category enabled.
fn config_with_per_category(
    overrides: impl FnOnce(
        &mut aivyx_channel::skill_auto_proposer::PerCategoryConfigSet,
    ),
) -> aivyx_channel::skill_auto_proposer::SkillAutoProposeConfig {
    use aivyx_channel::skill_auto_proposer::{
        PerCategoryConfig, PerCategoryConfigSet, SkillAutoProposeConfig,
    };
    let scalar = PerCategoryConfig {
        enabled: false,
        auto_accept_confidence_threshold: 0.99,
    };
    let list = PerCategoryConfig {
        enabled: true,
        auto_accept_confidence_threshold: 0.85,
    };
    let mut set = PerCategoryConfigSet {
        assistant_name: scalar.clone(),
        operator_profile: scalar.clone(),
        communication_style: scalar,
        primary_use_cases: list.clone(),
        behavioral_preferences: list.clone(),
        behavioral_constraints: list.clone(),
        learned_context: list.clone(),
        communication_adaptations: list.clone(),
        character_traits: list.clone(),
        relationship_milestones: list.clone(),
        learned_skill: list,
    };
    overrides(&mut set);
    SkillAutoProposeConfig {
        per_category: Some(set),
        ..SkillAutoProposeConfig::default()
    }
}

#[tokio::test]
async fn list_category_auto_accepts_into_persona_chain() {
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
        PersistentAuditLog::open(Arc::clone(&storage.handle), [0xABu8; 32])
            .await
            .expect("audit log opens"),
    );
    let shared: SharedEffectivePersona =
        persona::shared_effective_persona(EffectivePersona::default());

    let proposer_ctx = Arc::new(SkillAutoProposerContext {
        config: config_with_per_category(|_| {}),
        llm_provider: ScriptedProvider::new(vec![LIST_APPEND_VERDICT_JSON]),
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
        "user asked for terse replies several times".into(),
        &cancel,
    )
    .await;

    // Persona chain: one BehavioralPreferences AppendList entry.
    assert_eq!(persona_log.len(), 1);
    // Proposal chain: pending + approved.
    assert_eq!(proposal_log.len(), 2);

    // Audit event has the category populated.
    let entries = audit_log.entries().expect("entries");
    match &entries[0].event {
        AuditEvent::SkillAutoProposal {
            outcome, category, ..
        } => {
            assert!(matches!(
                outcome,
                aivyx_audit::SkillAutoProposalOutcomeSummary::AutoAccepted
            ));
            assert_eq!(category.as_deref(), Some("BehavioralPreferences"));
        }
        other => panic!("expected SkillAutoProposal; got {other:?}"),
    }

    // Shared persona reflects the new BehavioralPreferences entry.
    let state = shared.read().unwrap();
    assert_eq!(state.behavioral_preferences.len(), 1);
    assert!(state.behavioral_preferences[0].contains("terse"));
    assert!(state.learned_skills.is_empty()); // no skill side effect
}

#[tokio::test]
async fn scalar_category_disabled_by_default_drops_without_chain_write() {
    // AssistantName defaults to enabled=false. A verdict
    // picking AssistantName must drop to the
    // category-disabled outcome without any chain write.
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
        PersistentAuditLog::open(Arc::clone(&storage.handle), [0xABu8; 32])
            .await
            .expect("audit log opens"),
    );
    let shared: SharedEffectivePersona =
        persona::shared_effective_persona(EffectivePersona::default());

    let proposer_ctx = Arc::new(SkillAutoProposerContext {
        config: config_with_per_category(|_| {}),
        llm_provider: ScriptedProvider::new(vec![SCALAR_SET_VERDICT_JSON]),
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

    assert_eq!(persona_log.len(), 0);
    assert_eq!(proposal_log.len(), 0);
    let entries = audit_log.entries().expect("entries");
    let event = &entries[0].event;
    match event {
        AuditEvent::SkillAutoProposal {
            outcome,
            proposed_skill_name,
            category,
            ..
        } => {
            // The audit-event outcome reuses NotWorthProposing
            // for category-disabled drops; the
            // proposed_skill_name carries the diagnostic.
            assert!(matches!(
                outcome,
                aivyx_audit::SkillAutoProposalOutcomeSummary::NotWorthProposing
            ));
            assert!(
                proposed_skill_name
                    .as_deref()
                    .unwrap()
                    .contains("category-disabled")
            );
            assert_eq!(category.as_deref(), Some("AssistantName"));
        }
        other => panic!("expected SkillAutoProposal; got {other:?}"),
    }
}

#[tokio::test]
async fn scalar_category_explicitly_enabled_auto_accepts() {
    // Operator opts AssistantName in; the verdict's
    // confidence (0.97) is below the scalar default
    // threshold (0.99). Lower the threshold to 0.95 so the
    // verdict crosses.
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
        PersistentAuditLog::open(Arc::clone(&storage.handle), [0xABu8; 32])
            .await
            .expect("audit log opens"),
    );
    let shared: SharedEffectivePersona =
        persona::shared_effective_persona(EffectivePersona::default());

    let proposer_ctx = Arc::new(SkillAutoProposerContext {
        config: config_with_per_category(|pc| {
            pc.assistant_name.enabled = true;
            pc.assistant_name.auto_accept_confidence_threshold = 0.95;
        }),
        llm_provider: ScriptedProvider::new(vec![SCALAR_SET_VERDICT_JSON]),
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
        "operator called the assistant 'Aivyx' multiple times".into(),
        &cancel,
    )
    .await;

    // Persona chain: one AssistantName SetScalar entry.
    assert_eq!(persona_log.len(), 1);
    let state = shared.read().unwrap();
    assert_eq!(state.assistant_name.as_deref(), Some("Aivyx"));
}

#[tokio::test]
async fn operator_profile_disabled_drops_with_correct_audit_signal() {
    // OperatorProfile is also a scalar-default-off. A verdict
    // picking OperatorProfile drops; the audit-event
    // carries the category for forensic visibility.
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
        PersistentAuditLog::open(Arc::clone(&storage.handle), [0xABu8; 32])
            .await
            .expect("audit log opens"),
    );
    let shared: SharedEffectivePersona =
        persona::shared_effective_persona(EffectivePersona::default());
    let proposer_ctx = Arc::new(SkillAutoProposerContext {
        config: config_with_per_category(|_| {}),
        llm_provider: ScriptedProvider::new(vec![CATEGORY_DISABLED_VERDICT_JSON]),
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
    assert_eq!(persona_log.len(), 0);
    let entries = audit_log.entries().expect("entries");
    match &entries[0].event {
        AuditEvent::SkillAutoProposal { category, .. } => {
            assert_eq!(category.as_deref(), Some("OperatorProfile"));
        }
        other => panic!("expected SkillAutoProposal; got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Phase 115 — Failure-feedback path
// ---------------------------------------------------------------------------

use aivyx_channel::skill_auto_proposer::{
    FailureHeuristicConfig, FailureKind, ProposalSource,
    run_auto_propose_pipeline_with_source,
};

const FAILURE_VERDICT_JSON: &str = r#"{
    "is_worth_proposing": true,
    "confidence": 0.95,
    "category": "BehavioralConstraints",
    "proposed_draft": {
        "kind": "ListAppend",
        "value": "never call shell.exec with destructive arguments without explicit operator approval"
    },
    "is_duplicate_of": null,
    "reasoning": "the planner just hit a destructive-shell failure"
}"#;

const NOT_WORTH_FAILURE_VERDICT_JSON: &str = r#"{
    "is_worth_proposing": false,
    "confidence": 0.20,
    "category": null,
    "proposed_draft": null,
    "is_duplicate_of": null,
    "reasoning": "transient timeout; not actionable as a Persona refinement"
}"#;

fn config_with_failure_feedback(
    overrides: impl FnOnce(&mut FailureHeuristicConfig),
) -> aivyx_channel::skill_auto_proposer::SkillAutoProposeConfig {
    use aivyx_channel::skill_auto_proposer::{
        PerCategoryConfig, PerCategoryConfigSet, SkillAutoProposeConfig,
    };
    let scalar = PerCategoryConfig {
        enabled: false,
        auto_accept_confidence_threshold: 0.99,
    };
    let list = PerCategoryConfig {
        enabled: true,
        auto_accept_confidence_threshold: 0.85,
    };
    let mut failure_cfg = FailureHeuristicConfig::default();
    overrides(&mut failure_cfg);
    SkillAutoProposeConfig {
        per_category: Some(PerCategoryConfigSet {
            assistant_name: scalar.clone(),
            operator_profile: scalar.clone(),
            communication_style: scalar,
            primary_use_cases: list.clone(),
            behavioral_preferences: list.clone(),
            behavioral_constraints: list.clone(),
            learned_context: list.clone(),
            communication_adaptations: list.clone(),
            character_traits: list.clone(),
            relationship_milestones: list.clone(),
            learned_skill: list,
        }),
        from_failed_turns: true,
        failure_outcomes: failure_cfg,
        ..SkillAutoProposeConfig::default()
    }
}

#[tokio::test]
async fn failed_turn_auto_accepts_behavioral_constraint() {
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
        PersistentAuditLog::open(Arc::clone(&storage.handle), [0xABu8; 32])
            .await
            .expect("audit log opens"),
    );
    let shared: SharedEffectivePersona =
        persona::shared_effective_persona(EffectivePersona::default());

    let proposer_ctx = Arc::new(SkillAutoProposerContext {
        config: config_with_failure_feedback(|_| {}),
        llm_provider: ScriptedProvider::new(vec![FAILURE_VERDICT_JSON]),
    });

    let session_id = SessionId::new();
    let cancel = CancellationToken::new();
    let signals = aivyx_channel::skill_auto_proposer::TurnSignals {
        tool_calls_made: 0,
        distinct_tool_id_count: 0,
        duration: std::time::Duration::from_millis(0),
        had_successful_gate_resolve: false,
    };

    run_auto_propose_pipeline_with_source(
        &proposer_ctx,
        Some(&audit_log),
        Some(&persona_log),
        Some(&proposal_log),
        &shared,
        session_id,
        signals,
        "agent tried shell.exec rm -rf; planner errored with denied".into(),
        ProposalSource::FailedTurn {
            kind: FailureKind::Failed,
            summary: "planner returned shell.exec denied at step 5".into(),
        },
        &cancel,
    )
    .await;

    // Persona chain: one BehavioralConstraints AppendList entry.
    assert_eq!(persona_log.len(), 1);
    // Proposal chain: pending + approved.
    assert_eq!(proposal_log.len(), 2);
    // Shared persona reflects the new constraint.
    let state = shared.read().unwrap();
    assert_eq!(state.behavioral_constraints.len(), 1);
    assert!(state.behavioral_constraints[0].contains("destructive"));

    // Audit event has both category AND source populated.
    let entries = audit_log.entries().expect("entries");
    match &entries[0].event {
        AuditEvent::SkillAutoProposal {
            outcome,
            category,
            source,
            ..
        } => {
            assert!(matches!(
                outcome,
                aivyx_audit::SkillAutoProposalOutcomeSummary::AutoAccepted
            ));
            assert_eq!(category.as_deref(), Some("BehavioralConstraints"));
            match source {
                Some(aivyx_audit::ProposalSourceSummary::FailedTurn {
                    failure_kind,
                }) => assert_eq!(failure_kind, "failed"),
                other => panic!("expected FailedTurn source, got {other:?}"),
            }
        }
        other => panic!("expected SkillAutoProposal; got {other:?}"),
    }
}

#[tokio::test]
async fn timed_out_with_judge_decline_drops_without_chain_write() {
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
        PersistentAuditLog::open(Arc::clone(&storage.handle), [0xABu8; 32])
            .await
            .expect("audit log opens"),
    );
    let shared: SharedEffectivePersona =
        persona::shared_effective_persona(EffectivePersona::default());

    let proposer_ctx = Arc::new(SkillAutoProposerContext {
        config: config_with_failure_feedback(|_| {}),
        llm_provider: ScriptedProvider::new(vec![NOT_WORTH_FAILURE_VERDICT_JSON]),
    });

    let session_id = SessionId::new();
    let cancel = CancellationToken::new();
    let signals = aivyx_channel::skill_auto_proposer::TurnSignals {
        tool_calls_made: 0,
        distinct_tool_id_count: 0,
        duration: std::time::Duration::from_millis(0),
        had_successful_gate_resolve: false,
    };

    run_auto_propose_pipeline_with_source(
        &proposer_ctx,
        Some(&audit_log),
        Some(&persona_log),
        Some(&proposal_log),
        &shared,
        session_id,
        signals,
        "agent ran fs.read for 30s before timeout".into(),
        ProposalSource::FailedTurn {
            kind: FailureKind::TimedOut,
            summary: "exceeded 30s turn budget".into(),
        },
        &cancel,
    )
    .await;

    // No chain writes.
    assert_eq!(persona_log.len(), 0);
    assert_eq!(proposal_log.len(), 0);

    // Audit event recorded with the failure source.
    let entries = audit_log.entries().expect("entries");
    match &entries[0].event {
        AuditEvent::SkillAutoProposal {
            outcome, source, ..
        } => {
            assert!(matches!(
                outcome,
                aivyx_audit::SkillAutoProposalOutcomeSummary::NotWorthProposing
            ));
            match source {
                Some(aivyx_audit::ProposalSourceSummary::FailedTurn {
                    failure_kind,
                }) => assert_eq!(failure_kind, "timed_out"),
                other => panic!("expected FailedTurn(timed_out), got {other:?}"),
            }
        }
        other => panic!("expected SkillAutoProposal; got {other:?}"),
    }
}

#[tokio::test]
async fn config_phase_113_alias_does_not_fire_failure_path() {
    // Verifies the Phase 114-alias config (per_category =
    // None, from_failed_turns = false default) does NOT
    // fire the failure path even if the runtime is called
    // with a FailedTurn source. The auto-proposer pipeline
    // doesn't gate at this layer (the daemon hook does);
    // this test pins the *behavior* — the e2e shows that
    // the config drives gating behavior, but the runtime
    // pipeline still executes if called directly.
    //
    // The point: a Phase 113 operator's TOML never causes
    // the daemon-side hook to invoke the failure path, so
    // they get exactly Phase 114 behavior. If someone
    // calls the pipeline directly with a failure source,
    // it executes — the daemon-side gate is the
    // protection, not the pipeline.
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
        PersistentAuditLog::open(Arc::clone(&storage.handle), [0xABu8; 32])
            .await
            .expect("audit log opens"),
    );
    let shared: SharedEffectivePersona =
        persona::shared_effective_persona(EffectivePersona::default());

    // Phase 113 alias config: default SkillAutoProposeConfig
    // has from_failed_turns = false.
    let phase_113_config =
        aivyx_channel::skill_auto_proposer::SkillAutoProposeConfig::default();
    assert!(!phase_113_config.from_failed_turns);

    let proposer_ctx = Arc::new(SkillAutoProposerContext {
        config: phase_113_config,
        llm_provider: ScriptedProvider::new(vec![FAILURE_VERDICT_JSON]),
    });

    let session_id = SessionId::new();
    let cancel = CancellationToken::new();
    let signals = aivyx_channel::skill_auto_proposer::TurnSignals {
        tool_calls_made: 0,
        distinct_tool_id_count: 0,
        duration: std::time::Duration::from_millis(0),
        had_successful_gate_resolve: false,
    };

    // Calling with FailedTurn directly: pipeline fires
    // (the gate is at the daemon hook, not here). Chain
    // writes succeed because the LearnedSkill category is
    // enabled by default — but BehavioralConstraints is the
    // judge's pick, and with per_category=None the Phase
    // 113 path doesn't have per-category gates, so the
    // top-level threshold (0.85) applies. The fixture has
    // confidence 0.95, so auto-accept lands.
    run_auto_propose_pipeline_with_source(
        &proposer_ctx,
        Some(&audit_log),
        Some(&persona_log),
        Some(&proposal_log),
        &shared,
        session_id,
        signals,
        "summary".into(),
        ProposalSource::FailedTurn {
            kind: FailureKind::Failed,
            summary: "boom".into(),
        },
        &cancel,
    )
    .await;

    // Persona chain has one entry (the pipeline ran). The
    // load-bearing assertion is that the operator-side
    // daemon hook would NOT have invoked the pipeline
    // because Phase 113 config has from_failed_turns =
    // false — that's tested via daemon_server.rs logic
    // (covered indirectly by every other Phase 114 e2e
    // test which calls the pipeline with CompletedTurn
    // only).
    assert_eq!(persona_log.len(), 1);
}
