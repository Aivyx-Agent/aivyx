//! Phase 112 Task 4 — Background-task wiring for the skill
//! auto-proposer.
//!
//! Stitches together the two stages shipped in Tasks 2-3:
//!
//! 1. **Heuristic gate** (Phase 112 Task 2,
//!    [`aivyx_core::skill_proposer::heuristic::is_candidate`])
//!    — cheap deterministic filter.
//! 2. **LLM-judge call** (Phase 112 Task 3,
//!    [`aivyx_core::skill_proposer::judge::judge`]) — confirms
//!    the candidate is worth proposing and runs the dedup check
//!    in the same round-trip.
//!
//! Returns a single [`SkillProposerOutcome`] enum so the caller
//! can fan out on the verdict without dealing with a
//! `Result<...>` — the outcome enum encodes both happy paths
//! and every failure mode the auto-proposer needs to be
//! resilient against. **The auto-proposer never propagates an
//! error to the turn loop;** the turn outcome is already
//! committed by the time this fires.
//!
//! ## Q2b implication: background-spawn from post-`finalize`
//!
//! The Phase 112 Q-block (Q2b sign-off) put the auto-proposer
//! on the inline-at-turn-boundary firing path. The
//! implementation guarantee is that the user **never waits
//! for the auto-proposer** — the daemon's turn driver calls
//! [`spawn_auto_proposer_task`] **after** the
//! `TurnOutcome` finalize has already been forwarded to the
//! channel. The spawn is detached; whether the proposer
//! finishes is independent of when the next turn begins.
//!
//! ## Task 4 vs. Task 5 vs. Task 6 split
//!
//! Task 4 (this file) ships the **orchestration and failure-
//! isolation**. The verdict-routing decision logic (threshold-
//! gate auto-accept vs. staged proposal) lands in Task 5; the
//! audit-event emission lands in Task 6. The
//! [`SkillProposerOutcome::Verdict`] variant carries the raw
//! [`JudgeResponse`] so Tasks 5/6 can act on it without
//! re-running the call.

use std::sync::Arc;

use aivyx_core::skill_proposer::{
    self, ExistingSkillSnapshot, HeuristicConfig, JudgeError, JudgeRequest,
    JudgeResponse, TurnSignals,
};
use aivyx_core::CancellationToken;
use aivyx_llm::LlmProvider;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Runtime configuration for the skill auto-proposer. Task 5
/// promotes this struct to `aivyx-config` and wires the TOML
/// `[skills.auto_propose]` section; Task 4 lands the shape in
/// `aivyx-channel` so the orchestration is testable without
/// the TOML round-trip.
///
/// The operator picked the more autonomous shape at sign-off
/// (Q2b inline + Q3b auto-accept + Q4b LLM dedup), so the
/// defaults below reflect "this should actually fire" rather
/// than "opt-in-only."
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillAutoProposeConfig {
    /// Master switch. Default `true` per Q3b — the operator
    /// opted into the auto-accept trust window deliberately,
    /// so the phase doesn't bury the feature behind a
    /// disabled-by-default flag.
    pub enabled: bool,

    /// Heuristic thresholds for the cheap-gate stage.
    pub heuristic: HeuristicConfig,

    /// LLM-judge model identifier. Same format as the
    /// operator's main `model` field; defaults to the
    /// fast-and-cheap end of the provider's lineup since the
    /// judge is a one-shot structured-output call.
    pub judge_model: String,

    /// Max tokens the judge may emit on one call. Defaults to
    /// 800 — generous for a full SkillDraft + reasoning, short
    /// enough to keep cost bounded.
    pub judge_max_tokens: u32,

    /// Confidence threshold for the Task 5 auto-accept path
    /// (Q3b). `JudgeResponse.confidence >= threshold AND no
    /// duplicate AND no fuzzy-title-clash` lands in the
    /// LearnedSkill chain as an `auto_accepted: true` entry;
    /// everything else stages for manual approval.
    ///
    /// Default `0.85`. Read as: "the operator wants the LLM
    /// to be quite sure before auto-accept, but not certain
    /// to the point that the path never fires."
    ///
    /// Task 4 stores this field but doesn't act on it; Task 5
    /// fills in the auto-accept routing.
    pub auto_accept_confidence_threshold: f32,

    /// Fuzzy-title-match cutoff for the cheap dedup pre-filter
    /// (Q4b). A candidate with title fuzzy-match similarity
    /// against any existing skill at or above this threshold
    /// is dropped before the LLM-judge call (cost saver).
    /// Default `0.80`.
    ///
    /// Task 4 stores this field but doesn't act on it; Task 5
    /// fills in the dedup pre-filter.
    pub fuzzy_match_threshold: f32,
}

impl Default for SkillAutoProposeConfig {
    fn default() -> Self {
        SkillAutoProposeConfig {
            enabled: true,
            heuristic: HeuristicConfig::default(),
            judge_model: "claude-haiku-4-5".into(),
            judge_max_tokens: 800,
            auto_accept_confidence_threshold: 0.85,
            fuzzy_match_threshold: 0.80,
        }
    }
}

// ---------------------------------------------------------------------------
// Outcome
// ---------------------------------------------------------------------------

/// What happened when the auto-proposer ran. Pure outcome
/// enum — no `Result<>` wrapping because the caller treats
/// every variant as "the proposer is done; move on."
#[derive(Debug, Clone)]
pub enum SkillProposerOutcome {
    /// Master switch is off (`config.enabled == false`).
    Disabled,

    /// Heuristic gate rejected the turn. Most common variant;
    /// chit-chat turns and single-tool quick turns end here.
    HeuristicGated,

    /// Judge call ran successfully. The verdict is in the
    /// `JudgeResponse`. Task 5 reads this variant and routes
    /// to either auto-accept or staged.
    Verdict(JudgeResponse),

    /// Judge call failed (LLM error, parse failure, or
    /// confidence out-of-range). Carries the error message
    /// for the audit log (Task 6) and operator inspection.
    /// **Always logged at WARN level by the spawn wrapper.**
    JudgeError(String),
}

impl SkillProposerOutcome {
    /// Short stable label for the outcome — used by the
    /// audit log (Task 6) and by the spawn wrapper's WARN
    /// log line.
    pub fn label(&self) -> &'static str {
        match self {
            SkillProposerOutcome::Disabled => "disabled",
            SkillProposerOutcome::HeuristicGated => "heuristic-gated",
            SkillProposerOutcome::Verdict(_) => "verdict",
            SkillProposerOutcome::JudgeError(_) => "judge-error",
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Run the heuristic gate + LLM judge for one just-finalized
/// turn. Never propagates an error; failure modes are encoded
/// in [`SkillProposerOutcome`].
///
/// The caller assembles `signals` from `TurnOutcome` + the
/// turn's audit-log entries, builds `turn_summary` from the
/// user input + final reply + tool-call narrative, and
/// snapshots `existing_skills` from the Persona chain.
pub async fn auto_propose_for_turn(
    provider: Arc<dyn LlmProvider>,
    config: &SkillAutoProposeConfig,
    signals: TurnSignals,
    turn_summary: String,
    existing_skills: Vec<ExistingSkillSnapshot>,
    cancellation: &CancellationToken,
) -> SkillProposerOutcome {
    if !config.enabled {
        return SkillProposerOutcome::Disabled;
    }

    if !skill_proposer::is_candidate(&signals, &config.heuristic) {
        return SkillProposerOutcome::HeuristicGated;
    }

    let request = JudgeRequest {
        turn_summary: &turn_summary,
        existing_skills: &existing_skills,
        model: &config.judge_model,
        max_tokens: config.judge_max_tokens,
    };

    match skill_proposer::judge(provider, request, cancellation).await {
        Ok(response) => SkillProposerOutcome::Verdict(response),
        Err(JudgeError::Provider(e)) => {
            SkillProposerOutcome::JudgeError(format!("provider: {e}"))
        }
        Err(JudgeError::ParseFailure { raw }) => {
            SkillProposerOutcome::JudgeError(format!(
                "parse: {} chars unparseable",
                raw.len()
            ))
        }
        Err(JudgeError::ConfidenceOutOfRange(c)) => {
            SkillProposerOutcome::JudgeError(format!(
                "confidence out of range: {c}"
            ))
        }
    }
}

/// Detached-spawn wrapper for the post-`finalize` hook in the
/// daemon turn-driver. Calls [`auto_propose_for_turn`] inside
/// a `tokio::spawn`, logs the outcome label at the
/// appropriate level, and drops. The turn driver does NOT
/// await the returned `JoinHandle` — the auto-proposer runs
/// independently of subsequent turns.
///
/// The `tokio::JoinHandle` is returned so callers (test code,
/// shutdown-drain code) can `.await` if they want to know when
/// the proposer is done. The daemon turn-driver discards it.
pub fn spawn_auto_proposer_task(
    provider: Arc<dyn LlmProvider>,
    config: SkillAutoProposeConfig,
    signals: TurnSignals,
    turn_summary: String,
    existing_skills: Vec<ExistingSkillSnapshot>,
    cancellation: CancellationToken,
) -> tokio::task::JoinHandle<SkillProposerOutcome> {
    tokio::spawn(async move {
        let outcome = auto_propose_for_turn(
            provider,
            &config,
            signals,
            turn_summary,
            existing_skills,
            &cancellation,
        )
        .await;
        // Q2b failure-isolation contract: WARN on the
        // failure label so the operator notices a sustained
        // streak of judge errors, but never panic / never
        // propagate. The audit-event emission in Task 6 will
        // give the operator the full forensic surface.
        match &outcome {
            SkillProposerOutcome::JudgeError(msg) => {
                eprintln!("aivyx skill-auto-proposer: judge-error ({msg})");
            }
            SkillProposerOutcome::Disabled
            | SkillProposerOutcome::HeuristicGated
            | SkillProposerOutcome::Verdict(_) => {}
        }
        outcome
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_core::skill_proposer::{MatchMode, SkillDraft};
    use aivyx_llm::{
        ContentBlock, LlmError, LlmMessage, LlmProvider, LlmRequest, LlmStepEnd,
        LlmStream, LlmStreamEvent, LlmUsage,
    };
    use async_trait::async_trait;
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::time::Duration;

    // ----- A minimal scripted provider -----

    enum ScriptedStep {
        FinalText(String),
        Error(LlmError),
    }

    struct ScriptedProvider {
        steps: Mutex<VecDeque<ScriptedStep>>,
    }

    impl ScriptedProvider {
        fn new(steps: Vec<ScriptedStep>) -> Arc<Self> {
            Arc::new(ScriptedProvider {
                steps: Mutex::new(steps.into()),
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
            let step = self.steps.lock().unwrap().pop_front().ok_or_else(|| {
                LlmError::Config("ScriptedProvider exhausted".to_string())
            })?;
            match step {
                ScriptedStep::FinalText(text) => Ok(Box::new(ScriptedStream {
                    text: Some(text),
                })),
                ScriptedStep::Error(e) => Err(e),
            }
        }
    }

    struct ScriptedStream {
        text: Option<String>,
    }

    #[async_trait]
    impl LlmStream for ScriptedStream {
        async fn next_event(
            &mut self,
        ) -> Result<Option<LlmStreamEvent>, LlmError> {
            Ok(None)
        }
        async fn finish(self: Box<Self>) -> Result<LlmStepEnd, LlmError> {
            Ok(LlmStepEnd::FinalMessage {
                text: self.text.unwrap_or_default(),
                usage: LlmUsage::default(),
            })
        }
    }

    // Avoid an "unused" warning on the imports above we'd
    // need only if a future test wants to construct an
    // LlmMessage::User directly. `_unused_smoke` documents the
    // intent for readers.
    #[allow(dead_code)]
    fn _unused_smoke() {
        let _: LlmMessage = LlmMessage::User {
            content: vec![ContentBlock::Text { text: "x".into() }],
        };
    }

    fn fire_threshold_signals() -> TurnSignals {
        TurnSignals {
            tool_calls_made: 5,
            distinct_tool_id_count: 3,
            duration: Duration::from_millis(8_000),
            had_successful_gate_resolve: false,
        }
    }

    fn below_threshold_signals() -> TurnSignals {
        TurnSignals {
            tool_calls_made: 1,
            distinct_tool_id_count: 1,
            duration: Duration::from_millis(800),
            had_successful_gate_resolve: false,
        }
    }

    // ----- Outcome branches -----

    #[tokio::test]
    async fn disabled_config_returns_disabled_outcome_without_calling_llm() {
        // Empty scripted provider → if the LLM is called, we'd
        // get "exhausted" error. Master switch off should
        // skip the call entirely.
        let provider = ScriptedProvider::new(vec![]);
        let config = SkillAutoProposeConfig {
            enabled: false,
            ..SkillAutoProposeConfig::default()
        };
        let cancel = CancellationToken::new();
        let outcome = auto_propose_for_turn(
            provider,
            &config,
            fire_threshold_signals(),
            "summary".into(),
            vec![],
            &cancel,
        )
        .await;
        assert!(matches!(outcome, SkillProposerOutcome::Disabled));
        assert_eq!(outcome.label(), "disabled");
    }

    #[tokio::test]
    async fn heuristic_gated_turns_skip_the_llm_call() {
        // Empty scripted provider — heuristic gate must
        // short-circuit before we'd hit the "exhausted"
        // error.
        let provider = ScriptedProvider::new(vec![]);
        let config = SkillAutoProposeConfig::default();
        let cancel = CancellationToken::new();
        let outcome = auto_propose_for_turn(
            provider,
            &config,
            below_threshold_signals(),
            "summary".into(),
            vec![],
            &cancel,
        )
        .await;
        assert!(matches!(outcome, SkillProposerOutcome::HeuristicGated));
        assert_eq!(outcome.label(), "heuristic-gated");
    }

    #[tokio::test]
    async fn candidate_turn_with_worth_proposing_verdict_returns_verdict() {
        let provider = ScriptedProvider::new(vec![ScriptedStep::FinalText(
            r#"{"is_worth_proposing":true,"confidence":0.91,
            "proposed_skill":{"name":"research-topic","trigger":"research X",
            "procedure":"1. ...\n2. ..."},"is_duplicate_of":null,
            "reasoning":"recurring"}"#
                .into(),
        )]);
        let config = SkillAutoProposeConfig::default();
        let cancel = CancellationToken::new();
        let outcome = auto_propose_for_turn(
            provider,
            &config,
            fire_threshold_signals(),
            "summary".into(),
            vec![],
            &cancel,
        )
        .await;
        match outcome {
            SkillProposerOutcome::Verdict(r) => {
                assert!(r.is_worth_proposing);
                assert!((r.confidence - 0.91).abs() < 1e-6);
                let draft = r.proposed_skill.as_ref().unwrap();
                assert_eq!(draft.name, "research-topic");
                let _ = SkillDraft::clone(draft); // ensure SkillDraft re-export is wired
            }
            _ => panic!("expected Verdict; got {:?}", outcome),
        }
    }

    #[tokio::test]
    async fn judge_provider_error_becomes_judge_error_outcome() {
        let provider = ScriptedProvider::new(vec![ScriptedStep::Error(
            LlmError::Config("simulated outage".into()),
        )]);
        let config = SkillAutoProposeConfig::default();
        let cancel = CancellationToken::new();
        let outcome = auto_propose_for_turn(
            provider,
            &config,
            fire_threshold_signals(),
            "summary".into(),
            vec![],
            &cancel,
        )
        .await;
        match outcome {
            SkillProposerOutcome::JudgeError(msg) => {
                assert!(msg.contains("provider"));
                assert!(msg.contains("simulated outage"));
            }
            _ => panic!("expected JudgeError; got {:?}", outcome),
        }
    }

    #[tokio::test]
    async fn judge_parse_failure_becomes_judge_error_outcome() {
        let provider = ScriptedProvider::new(vec![ScriptedStep::FinalText(
            "the llm misbehaved and didn't return json".into(),
        )]);
        let config = SkillAutoProposeConfig::default();
        let cancel = CancellationToken::new();
        let outcome = auto_propose_for_turn(
            provider,
            &config,
            fire_threshold_signals(),
            "summary".into(),
            vec![],
            &cancel,
        )
        .await;
        match outcome {
            SkillProposerOutcome::JudgeError(msg) => {
                assert!(msg.contains("parse"));
            }
            _ => panic!("expected JudgeError; got {:?}", outcome),
        }
    }

    #[tokio::test]
    async fn judge_confidence_out_of_range_becomes_judge_error() {
        let provider = ScriptedProvider::new(vec![ScriptedStep::FinalText(
            r#"{"is_worth_proposing":true,"confidence":2.0,
            "proposed_skill":null,"is_duplicate_of":null}"#
                .into(),
        )]);
        let config = SkillAutoProposeConfig::default();
        let cancel = CancellationToken::new();
        let outcome = auto_propose_for_turn(
            provider,
            &config,
            fire_threshold_signals(),
            "summary".into(),
            vec![],
            &cancel,
        )
        .await;
        match outcome {
            SkillProposerOutcome::JudgeError(msg) => {
                assert!(msg.contains("confidence"));
            }
            _ => panic!("expected JudgeError; got {:?}", outcome),
        }
    }

    // ----- Failure isolation: spawn wrapper -----

    #[tokio::test]
    async fn spawn_returns_a_join_handle_that_yields_the_outcome() {
        let provider = ScriptedProvider::new(vec![ScriptedStep::FinalText(
            r#"{"is_worth_proposing":false,"confidence":0.2,
            "proposed_skill":null,"is_duplicate_of":null}"#
                .into(),
        )]);
        let config = SkillAutoProposeConfig::default();
        let cancel = CancellationToken::new();
        let handle = spawn_auto_proposer_task(
            provider,
            config,
            fire_threshold_signals(),
            "summary".into(),
            vec![],
            cancel,
        );
        let outcome = handle.await.expect("task must not panic");
        match outcome {
            SkillProposerOutcome::Verdict(r) => {
                assert!(!r.is_worth_proposing);
            }
            _ => panic!("expected Verdict; got {:?}", outcome),
        }
    }

    #[tokio::test]
    async fn spawn_wrapper_does_not_panic_on_provider_error() {
        // The crux of the Q2b failure-isolation contract:
        // even a provider that simulates an outage cannot
        // bring down the daemon. The spawn wrapper logs WARN
        // and yields a JudgeError outcome; the JoinHandle
        // completes cleanly.
        let provider = ScriptedProvider::new(vec![ScriptedStep::Error(
            LlmError::Config("kapow".into()),
        )]);
        let config = SkillAutoProposeConfig::default();
        let cancel = CancellationToken::new();
        let handle = spawn_auto_proposer_task(
            provider,
            config,
            fire_threshold_signals(),
            "summary".into(),
            vec![],
            cancel,
        );
        let outcome = handle.await.expect("spawn must not panic");
        assert_eq!(outcome.label(), "judge-error");
    }

    // ----- Config -----

    #[test]
    fn default_config_is_enabled_with_sane_thresholds() {
        let c = SkillAutoProposeConfig::default();
        assert!(c.enabled);
        assert_eq!(c.heuristic.mode, MatchMode::Any);
        assert_eq!(c.judge_max_tokens, 800);
        assert!((c.auto_accept_confidence_threshold - 0.85).abs() < 1e-6);
        assert!((c.fuzzy_match_threshold - 0.80).abs() < 1e-6);
    }

    #[test]
    fn config_round_trips_through_serde_json() {
        let original = SkillAutoProposeConfig::default();
        let s = serde_json::to_string(&original).unwrap();
        let back: SkillAutoProposeConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back, original);
    }
}
