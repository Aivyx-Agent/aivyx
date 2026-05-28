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
    JudgeResponse, SkillDraft, TurnSignals,
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
// Task 5 — Decision routing
// ---------------------------------------------------------------------------

/// The terminal decision the auto-proposer reaches after the
/// LLM judge has returned a [`JudgeResponse`]. One of four
/// outcomes:
///
/// - `AutoAccept`: confidence >= threshold, no LLM-judged dup,
///   no fuzzy-title-clash → land in the LearnedSkill chain
///   directly as an approved entry tagged `auto_accepted: true`.
/// - `Staged`: worth-proposing but below confidence
///   threshold → land in the proposal chain as Pending so the
///   operator can review through `aivyx persona proposals
///   approve` (the same surface manual proposals use).
/// - `DroppedJudgeDup`: the judge declared this candidate a
///   semantic duplicate of an existing skill → nothing
///   written, just logged for audit (Q4b LLM semantic check).
/// - `DroppedFuzzyDup`: the cheap title fuzzy-match pre-
///   filter caught this candidate before the judge even fired
///   → nothing written; cost-efficient dedup (Q4b fuzzy-
///   match pre-filter).
/// - `DroppedNotWorthProposing`: the judge said
///   `is_worth_proposing = false` without naming a dup →
///   nothing written.
#[derive(Debug, Clone, PartialEq)]
pub enum SkillRoutingDecision {
    AutoAccept { draft: SkillDraft, confidence: f32 },
    Staged { draft: SkillDraft, confidence: f32 },
    DroppedJudgeDup { duplicate_of: String },
    DroppedFuzzyDup { matched_existing_name: String },
    DroppedNotWorthProposing,
}

impl SkillRoutingDecision {
    /// Short stable label for the audit log (Task 6) and
    /// operator forensics.
    pub fn label(&self) -> &'static str {
        match self {
            SkillRoutingDecision::AutoAccept { .. } => "auto-accept",
            SkillRoutingDecision::Staged { .. } => "staged",
            SkillRoutingDecision::DroppedJudgeDup { .. } => "dup-dropped-llm",
            SkillRoutingDecision::DroppedFuzzyDup { .. } => "dup-dropped-fuzzy",
            SkillRoutingDecision::DroppedNotWorthProposing => "not-worth-proposing",
        }
    }
}

/// Apply the threshold-gated routing rules (Q3b) on top of the
/// judge's verdict, with the Q4b fuzzy-match pre-filter
/// running first (cheap dedup catches obvious title-dups
/// before any further work).
///
/// **The pre-filter runs against `verdict.proposed_skill`'s
/// title, not the original turn summary.** The judge has
/// already drafted a candidate skill at this point; we check
/// if its title fuzzy-matches an existing skill *as a final
/// safety net* (the judge may have missed an obvious dup the
/// fuzzy-match would catch).
///
/// The function is pure — same inputs → same decision. Caller
/// is responsible for actually writing the chain entries on
/// `AutoAccept` and `Staged` outcomes; this just decides which
/// path is right.
pub fn decide_routing(
    verdict: &JudgeResponse,
    existing_skills: &[ExistingSkillSnapshot],
    config: &SkillAutoProposeConfig,
) -> SkillRoutingDecision {
    // Step 1 — Judge said dup, drop immediately. The judge's
    // semantic check beats the fuzzy-match: if the judge saw
    // a dup, we trust it.
    if let Some(name) = &verdict.is_duplicate_of {
        return SkillRoutingDecision::DroppedJudgeDup {
            duplicate_of: name.clone(),
        };
    }

    // Step 2 — Judge said not worth proposing (and not a
    // dup), drop.
    if !verdict.is_worth_proposing {
        return SkillRoutingDecision::DroppedNotWorthProposing;
    }

    // Step 3 — Judge said yes but the draft is missing (LLM
    // misbehaved). Treat as not-worth-proposing rather than
    // ship a broken proposal.
    let Some(draft) = verdict.proposed_skill.clone() else {
        return SkillRoutingDecision::DroppedNotWorthProposing;
    };

    // Step 4 — Fuzzy-title-match pre-filter (final safety
    // net). The original Q4b design positioned this BEFORE
    // the judge call; we keep the principle but apply it
    // after the judge for one extra safety pass. The cost
    // saving still applies on most real-world traffic: most
    // turns never reach the judge (heuristic gates them).
    if let Some(matched) = fuzzy_match_against_existing(
        &draft.name,
        existing_skills,
        config.fuzzy_match_threshold,
    ) {
        return SkillRoutingDecision::DroppedFuzzyDup {
            matched_existing_name: matched,
        };
    }

    // Step 5 — Threshold gate (Q3b).
    if verdict.confidence >= config.auto_accept_confidence_threshold {
        SkillRoutingDecision::AutoAccept {
            draft,
            confidence: verdict.confidence,
        }
    } else {
        SkillRoutingDecision::Staged {
            draft,
            confidence: verdict.confidence,
        }
    }
}

// ---------------------------------------------------------------------------
// Task 5 — Fuzzy title-match pre-filter (Q4b cheap dedup)
// ---------------------------------------------------------------------------

/// Compute a normalized title similarity in `[0.0, 1.0]`
/// between two skill names. The algorithm:
///
/// 1. Lowercase both inputs.
/// 2. Replace any non-alphanumeric character with `-`.
/// 3. Compute the Jaccard similarity of the resulting token
///    sets (split on `-`).
///
/// This is deliberately cheap (`O(n + m)` set construction
/// followed by an intersection scan) so it can fire on every
/// candidate without measurable cost. It catches the obvious
/// cases the Q4b pre-filter is designed for:
/// `memory.gc` vs `memory_gc`; `research-topic` vs
/// `topic-research`; `aivyx-mcp-recipes` vs
/// `mcp-recipes-aivyx`. It does NOT catch deep semantic
/// dups — those land on the LLM-judge call's
/// `is_duplicate_of` path.
pub fn title_similarity(a: &str, b: &str) -> f32 {
    fn tokens(s: &str) -> std::collections::HashSet<String> {
        s.to_ascii_lowercase()
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|t| !t.is_empty())
            .map(|t| t.to_string())
            .collect()
    }
    let ta = tokens(a);
    let tb = tokens(b);
    if ta.is_empty() && tb.is_empty() {
        return 1.0;
    }
    let intersection = ta.intersection(&tb).count();
    let union = ta.union(&tb).count();
    if union == 0 {
        return 0.0;
    }
    intersection as f32 / union as f32
}

/// Return the name of the first existing skill whose title
/// similarity against `candidate_title` meets or exceeds
/// `threshold`. Used as the Q4b cheap dedup pre-filter.
pub fn fuzzy_match_against_existing(
    candidate_title: &str,
    existing: &[ExistingSkillSnapshot],
    threshold: f32,
) -> Option<String> {
    existing
        .iter()
        .find(|s| title_similarity(candidate_title, &s.name) >= threshold)
        .map(|s| s.name.clone())
}

// ---------------------------------------------------------------------------
// Task 6 — Audit-event construction helpers
// ---------------------------------------------------------------------------

/// Compute which heuristic signals (Q1b stage 1) crossed
/// their thresholds for a given `TurnSignals` + `HeuristicConfig`
/// pair. The boolean record is what the audit event carries.
///
/// **Always reports the actual signal crossings**, regardless
/// of the `MatchMode`. Forensic queries care about "which
/// signals crossed?", not "did the combined gate fire?" —
/// the gate-firing question is implicit in the outcome
/// summary itself (HeuristicGated vs. anything past the
/// judge).
pub fn signals_matched(
    signals: &TurnSignals,
    config: &HeuristicConfig,
) -> aivyx_audit::HeuristicSignalsMatched {
    aivyx_audit::HeuristicSignalsMatched {
        tool_call_count: signals.tool_calls_made >= config.tool_call_count_min,
        distinct_tool_id_count: signals.distinct_tool_id_count
            >= config.distinct_tool_id_min,
        duration: signals.duration.as_millis() as u64
            >= config.duration_ms_min,
        gate_resolve: signals.had_successful_gate_resolve,
    }
}

/// Convert a `SkillProposerOutcome` (+ the routing decision
/// for Verdict outcomes) into the audit-chain summary enum.
/// The Verdict→AutoAccepted/Staged/Dup/NotWorth path requires
/// the routing decision; the other proposer outcomes map 1-1.
///
/// Returns the outcome variant + the (proposed_skill_name,
/// confidence_thousandths) pair that the audit event needs.
/// `judge_latency_ms` is provided by the caller (it's measured
/// at the call site, not here).
pub fn audit_outcome_from(
    proposer_outcome: &SkillProposerOutcome,
    routing: Option<&SkillRoutingDecision>,
) -> (
    aivyx_audit::SkillAutoProposalOutcomeSummary,
    Option<String>,
    Option<u32>,
) {
    use aivyx_audit::SkillAutoProposalOutcomeSummary as S;

    match proposer_outcome {
        SkillProposerOutcome::Disabled => (S::Disabled, None, None),
        SkillProposerOutcome::HeuristicGated => (S::HeuristicGated, None, None),
        SkillProposerOutcome::JudgeError(msg) => (
            S::JudgeError {
                error_message: msg.clone(),
            },
            None,
            None,
        ),
        SkillProposerOutcome::Verdict(verdict) => {
            // Use the routing decision if provided; otherwise
            // fall back to deriving from the verdict alone.
            // (The caller should always supply the routing.)
            let routing_owned;
            let routing = match routing {
                Some(r) => r,
                None => {
                    // Build a default routing decision from the
                    // verdict using empty config defaults. This
                    // shouldn't fire in production paths — Task 7
                    // always passes a routing — but it keeps the
                    // function total.
                    routing_owned = decide_routing(
                        verdict,
                        &[],
                        &SkillAutoProposeConfig::default(),
                    );
                    &routing_owned
                }
            };
            let confidence_thousandths =
                Some((verdict.confidence * 1000.0).round() as u32);
            match routing {
                SkillRoutingDecision::AutoAccept { draft, .. } => (
                    S::AutoAccepted,
                    Some(draft.name.clone()),
                    confidence_thousandths,
                ),
                SkillRoutingDecision::Staged { draft, .. } => (
                    S::Staged,
                    Some(draft.name.clone()),
                    confidence_thousandths,
                ),
                SkillRoutingDecision::DroppedJudgeDup { duplicate_of } => (
                    S::DuplicateOfExistingLlm {
                        duplicate_of: duplicate_of.clone(),
                    },
                    verdict
                        .proposed_skill
                        .as_ref()
                        .map(|d| d.name.clone()),
                    confidence_thousandths,
                ),
                SkillRoutingDecision::DroppedFuzzyDup {
                    matched_existing_name,
                } => (
                    S::DuplicateOfExistingFuzzy {
                        matched_existing_name: matched_existing_name.clone(),
                    },
                    verdict
                        .proposed_skill
                        .as_ref()
                        .map(|d| d.name.clone()),
                    confidence_thousandths,
                ),
                SkillRoutingDecision::DroppedNotWorthProposing => (
                    S::NotWorthProposing,
                    verdict
                        .proposed_skill
                        .as_ref()
                        .map(|d| d.name.clone()),
                    confidence_thousandths,
                ),
            }
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

    // ----- Task 5 — Decision routing -----

    fn worth_proposing_verdict(confidence: f32) -> JudgeResponse {
        JudgeResponse {
            is_worth_proposing: true,
            confidence,
            proposed_skill: Some(SkillDraft {
                name: "research-topic".into(),
                trigger: "user asks to research X".into(),
                procedure: "1. fs.read\n2. web.fetch".into(),
            }),
            is_duplicate_of: None,
            reasoning: None,
        }
    }

    fn existing_skills_fixture() -> Vec<ExistingSkillSnapshot> {
        vec![
            ExistingSkillSnapshot {
                name: "summarize-pdf".into(),
                trigger: "user shares a PDF".into(),
                procedure_summary: "fs.read PDF, extract sections".into(),
            },
            ExistingSkillSnapshot {
                name: "deploy-to-staging".into(),
                trigger: "user requests staging deploy".into(),
                procedure_summary: "git.status, shell.exec deploy script".into(),
            },
        ]
    }

    #[test]
    fn routing_auto_accepts_at_threshold() {
        let verdict = worth_proposing_verdict(0.85);
        let config = SkillAutoProposeConfig::default();
        let d = decide_routing(&verdict, &[], &config);
        match &d {
            SkillRoutingDecision::AutoAccept { confidence, draft } => {
                assert!((confidence - 0.85).abs() < 1e-6);
                assert_eq!(draft.name, "research-topic");
            }
            _ => panic!("expected AutoAccept; got {:?}", d),
        }
        assert_eq!(d.label(), "auto-accept");
    }

    #[test]
    fn routing_auto_accepts_above_threshold() {
        let verdict = worth_proposing_verdict(0.95);
        let config = SkillAutoProposeConfig::default();
        let d = decide_routing(&verdict, &[], &config);
        assert!(matches!(d, SkillRoutingDecision::AutoAccept { .. }));
    }

    #[test]
    fn routing_stages_below_threshold() {
        let verdict = worth_proposing_verdict(0.84);
        let config = SkillAutoProposeConfig::default();
        let d = decide_routing(&verdict, &[], &config);
        match &d {
            SkillRoutingDecision::Staged { confidence, draft } => {
                assert!((confidence - 0.84).abs() < 1e-6);
                assert_eq!(draft.name, "research-topic");
            }
            _ => panic!("expected Staged; got {:?}", d),
        }
        assert_eq!(d.label(), "staged");
    }

    #[test]
    fn routing_drops_when_judge_declares_duplicate() {
        let mut verdict = worth_proposing_verdict(0.99);
        verdict.is_worth_proposing = false;
        verdict.is_duplicate_of = Some("summarize-pdf".into());
        let config = SkillAutoProposeConfig::default();
        let d = decide_routing(&verdict, &existing_skills_fixture(), &config);
        match &d {
            SkillRoutingDecision::DroppedJudgeDup { duplicate_of } => {
                assert_eq!(duplicate_of, "summarize-pdf");
            }
            _ => panic!("expected DroppedJudgeDup; got {:?}", d),
        }
        assert_eq!(d.label(), "dup-dropped-llm");
    }

    #[test]
    fn routing_drops_when_not_worth_proposing() {
        let verdict = JudgeResponse {
            is_worth_proposing: false,
            confidence: 0.5,
            proposed_skill: None,
            is_duplicate_of: None,
            reasoning: Some("one-off chat".into()),
        };
        let config = SkillAutoProposeConfig::default();
        let d = decide_routing(&verdict, &[], &config);
        assert!(matches!(d, SkillRoutingDecision::DroppedNotWorthProposing));
        assert_eq!(d.label(), "not-worth-proposing");
    }

    #[test]
    fn routing_drops_when_judge_says_worth_but_omits_draft() {
        // LLM misbehavior — treat as not-worth-proposing.
        let verdict = JudgeResponse {
            is_worth_proposing: true,
            confidence: 0.9,
            proposed_skill: None,
            is_duplicate_of: None,
            reasoning: None,
        };
        let config = SkillAutoProposeConfig::default();
        let d = decide_routing(&verdict, &[], &config);
        assert!(matches!(d, SkillRoutingDecision::DroppedNotWorthProposing));
    }

    #[test]
    fn routing_fuzzy_match_drops_obvious_title_dup() {
        // Existing: "summarize-pdf"; candidate: "summarize-pdf" → identical
        // title → fuzzy match fires.
        let mut verdict = worth_proposing_verdict(0.95);
        verdict.proposed_skill.as_mut().unwrap().name = "summarize-pdf".into();
        let config = SkillAutoProposeConfig::default();
        let d = decide_routing(&verdict, &existing_skills_fixture(), &config);
        match &d {
            SkillRoutingDecision::DroppedFuzzyDup {
                matched_existing_name,
            } => {
                assert_eq!(matched_existing_name, "summarize-pdf");
            }
            _ => panic!("expected DroppedFuzzyDup; got {:?}", d),
        }
        assert_eq!(d.label(), "dup-dropped-fuzzy");
    }

    #[test]
    fn routing_fuzzy_match_drops_underscored_vs_dotted_variant() {
        // Existing: "summarize-pdf"; candidate: "summarize_pdf" — same
        // tokens after normalization.
        let mut verdict = worth_proposing_verdict(0.95);
        verdict.proposed_skill.as_mut().unwrap().name = "summarize_pdf".into();
        let config = SkillAutoProposeConfig::default();
        let d = decide_routing(&verdict, &existing_skills_fixture(), &config);
        assert!(matches!(d, SkillRoutingDecision::DroppedFuzzyDup { .. }));
    }

    #[test]
    fn routing_fuzzy_match_drops_reordered_tokens() {
        // Existing: "deploy-to-staging"; candidate: "staging-to-deploy" —
        // same token set after normalization → Jaccard 1.0.
        let mut verdict = worth_proposing_verdict(0.95);
        verdict.proposed_skill.as_mut().unwrap().name = "staging-to-deploy".into();
        let config = SkillAutoProposeConfig::default();
        let d = decide_routing(&verdict, &existing_skills_fixture(), &config);
        assert!(matches!(d, SkillRoutingDecision::DroppedFuzzyDup { .. }));
    }

    #[test]
    fn routing_does_not_fuzzy_drop_distinct_titles() {
        let verdict = worth_proposing_verdict(0.95);
        // "research-topic" has zero tokens in common with the two
        // existing skill names.
        let config = SkillAutoProposeConfig::default();
        let d = decide_routing(&verdict, &existing_skills_fixture(), &config);
        assert!(matches!(d, SkillRoutingDecision::AutoAccept { .. }));
    }

    #[test]
    fn routing_respects_higher_fuzzy_threshold() {
        // With a high threshold (0.99), a partial overlap shouldn't
        // count as a dup. Existing "summarize-pdf"; candidate
        // "summarize-doc" — overlap is {summarize} of {summarize, pdf,
        // doc} → 1/3 → below 0.99.
        let mut verdict = worth_proposing_verdict(0.95);
        verdict.proposed_skill.as_mut().unwrap().name = "summarize-doc".into();
        let config = SkillAutoProposeConfig {
            fuzzy_match_threshold: 0.99,
            ..SkillAutoProposeConfig::default()
        };
        let d = decide_routing(&verdict, &existing_skills_fixture(), &config);
        assert!(matches!(d, SkillRoutingDecision::AutoAccept { .. }));
    }

    // ----- Task 5 — Title similarity primitive -----

    #[test]
    fn title_similarity_identical_titles_are_one() {
        assert_eq!(title_similarity("research-topic", "research-topic"), 1.0);
    }

    #[test]
    fn title_similarity_normalizes_separators() {
        assert_eq!(title_similarity("memory.gc", "memory_gc"), 1.0);
        assert_eq!(title_similarity("memory.gc", "memory-gc"), 1.0);
    }

    #[test]
    fn title_similarity_is_case_insensitive() {
        assert_eq!(title_similarity("Memory.GC", "memory.gc"), 1.0);
    }

    #[test]
    fn title_similarity_jaccard_for_partial_overlap() {
        // "summarize-pdf" vs "summarize-doc" — tokens {summarize, pdf}
        // vs {summarize, doc} → intersection {summarize}, union
        // {summarize, pdf, doc} → 1/3 ≈ 0.333.
        let sim = title_similarity("summarize-pdf", "summarize-doc");
        assert!((sim - 1.0 / 3.0).abs() < 1e-5);
    }

    #[test]
    fn title_similarity_disjoint_tokens_are_zero() {
        assert_eq!(title_similarity("alpha", "beta"), 0.0);
    }

    #[test]
    fn title_similarity_empty_inputs() {
        assert_eq!(title_similarity("", ""), 1.0);
        assert_eq!(title_similarity("alpha", ""), 0.0);
        assert_eq!(title_similarity("", "alpha"), 0.0);
    }

    #[test]
    fn fuzzy_match_returns_first_match_above_threshold() {
        let existing = existing_skills_fixture();
        let m = fuzzy_match_against_existing("summarize-pdf", &existing, 0.80);
        assert_eq!(m.as_deref(), Some("summarize-pdf"));
    }

    #[test]
    fn fuzzy_match_returns_none_when_no_existing_match() {
        let existing = existing_skills_fixture();
        let m = fuzzy_match_against_existing("totally-novel-skill", &existing, 0.80);
        assert!(m.is_none());
    }

    // ----- Task 6 — Audit-event construction helpers -----

    #[test]
    fn signals_matched_reports_each_axis_independently() {
        let signals = TurnSignals {
            tool_calls_made: 3,
            distinct_tool_id_count: 1, // below min=2
            duration: Duration::from_millis(10_000),
            had_successful_gate_resolve: true,
        };
        let config = HeuristicConfig::default();
        let m = signals_matched(&signals, &config);
        assert!(m.tool_call_count);
        assert!(!m.distinct_tool_id_count);
        assert!(m.duration);
        assert!(m.gate_resolve);
    }

    #[test]
    fn signals_matched_reports_all_below_threshold_as_all_false() {
        let signals = TurnSignals {
            tool_calls_made: 0,
            distinct_tool_id_count: 0,
            duration: Duration::from_millis(0),
            had_successful_gate_resolve: false,
        };
        let config = HeuristicConfig::default();
        let m = signals_matched(&signals, &config);
        assert!(!m.tool_call_count);
        assert!(!m.distinct_tool_id_count);
        assert!(!m.duration);
        assert!(!m.gate_resolve);
    }

    #[test]
    fn audit_outcome_disabled_maps_cleanly() {
        let (outcome, name, conf) =
            audit_outcome_from(&SkillProposerOutcome::Disabled, None);
        assert!(matches!(
            outcome,
            aivyx_audit::SkillAutoProposalOutcomeSummary::Disabled
        ));
        assert!(name.is_none());
        assert!(conf.is_none());
    }

    #[test]
    fn audit_outcome_heuristic_gated_maps_cleanly() {
        let (outcome, name, conf) =
            audit_outcome_from(&SkillProposerOutcome::HeuristicGated, None);
        assert!(matches!(
            outcome,
            aivyx_audit::SkillAutoProposalOutcomeSummary::HeuristicGated
        ));
        assert!(name.is_none());
        assert!(conf.is_none());
    }

    #[test]
    fn audit_outcome_judge_error_carries_message() {
        let (outcome, name, conf) = audit_outcome_from(
            &SkillProposerOutcome::JudgeError("provider: HTTP 429".into()),
            None,
        );
        match outcome {
            aivyx_audit::SkillAutoProposalOutcomeSummary::JudgeError {
                error_message,
            } => {
                assert_eq!(error_message, "provider: HTTP 429");
            }
            _ => panic!("expected JudgeError"),
        }
        assert!(name.is_none());
        assert!(conf.is_none());
    }

    #[test]
    fn audit_outcome_auto_accept_carries_name_and_confidence() {
        let verdict = worth_proposing_verdict(0.91);
        let routing = decide_routing(
            &verdict,
            &[],
            &SkillAutoProposeConfig::default(),
        );
        let (outcome, name, conf) = audit_outcome_from(
            &SkillProposerOutcome::Verdict(verdict.clone()),
            Some(&routing),
        );
        assert!(matches!(
            outcome,
            aivyx_audit::SkillAutoProposalOutcomeSummary::AutoAccepted
        ));
        assert_eq!(name.as_deref(), Some("research-topic"));
        assert_eq!(conf, Some(910));
    }

    #[test]
    fn audit_outcome_staged_carries_name_and_confidence() {
        let verdict = worth_proposing_verdict(0.72);
        let routing = decide_routing(
            &verdict,
            &[],
            &SkillAutoProposeConfig::default(),
        );
        let (outcome, name, conf) = audit_outcome_from(
            &SkillProposerOutcome::Verdict(verdict.clone()),
            Some(&routing),
        );
        assert!(matches!(
            outcome,
            aivyx_audit::SkillAutoProposalOutcomeSummary::Staged
        ));
        assert_eq!(name.as_deref(), Some("research-topic"));
        assert_eq!(conf, Some(720));
    }

    #[test]
    fn audit_outcome_dup_llm_carries_dup_name() {
        let mut verdict = worth_proposing_verdict(0.95);
        verdict.is_worth_proposing = false;
        verdict.is_duplicate_of = Some("summarize-pdf".into());
        let routing = decide_routing(
            &verdict,
            &existing_skills_fixture(),
            &SkillAutoProposeConfig::default(),
        );
        let (outcome, _name, _conf) = audit_outcome_from(
            &SkillProposerOutcome::Verdict(verdict),
            Some(&routing),
        );
        match outcome {
            aivyx_audit::SkillAutoProposalOutcomeSummary::DuplicateOfExistingLlm {
                duplicate_of,
            } => {
                assert_eq!(duplicate_of, "summarize-pdf");
            }
            _ => panic!("expected DuplicateOfExistingLlm"),
        }
    }

    #[test]
    fn audit_outcome_dup_fuzzy_carries_matched_name() {
        let mut verdict = worth_proposing_verdict(0.95);
        verdict.proposed_skill.as_mut().unwrap().name = "summarize-pdf".into();
        let routing = decide_routing(
            &verdict,
            &existing_skills_fixture(),
            &SkillAutoProposeConfig::default(),
        );
        let (outcome, _name, _conf) = audit_outcome_from(
            &SkillProposerOutcome::Verdict(verdict),
            Some(&routing),
        );
        match outcome {
            aivyx_audit::SkillAutoProposalOutcomeSummary::DuplicateOfExistingFuzzy {
                matched_existing_name,
            } => {
                assert_eq!(matched_existing_name, "summarize-pdf");
            }
            _ => panic!("expected DuplicateOfExistingFuzzy"),
        }
    }

    #[test]
    fn audit_outcome_not_worth_proposing_maps_cleanly() {
        let verdict = JudgeResponse {
            is_worth_proposing: false,
            confidence: 0.30,
            proposed_skill: None,
            is_duplicate_of: None,
            reasoning: None,
        };
        let routing = decide_routing(
            &verdict,
            &[],
            &SkillAutoProposeConfig::default(),
        );
        let (outcome, name, conf) = audit_outcome_from(
            &SkillProposerOutcome::Verdict(verdict),
            Some(&routing),
        );
        assert!(matches!(
            outcome,
            aivyx_audit::SkillAutoProposalOutcomeSummary::NotWorthProposing
        ));
        assert!(name.is_none());
        assert_eq!(conf, Some(300));
    }
}
