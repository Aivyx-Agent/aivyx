//! Phase 112 Task 3 — LLM-judge surface for skill auto-
//! proposal candidates.
//!
//! **Phase 114 generalization:** the judge now picks the
//! best `PersonaDeltaCategory` for the turn (not just
//! `LearnedSkill`), and the proposed draft shape varies by
//! category — `LearnedSkill` is full `{name, trigger,
//! procedure}`, list categories are a single string to
//! append, scalar categories are a single string to set.
//! The existing-persona snapshot the judge sees covers
//! every category, not just skills, so dedup can fire
//! across the whole Persona surface.
//!
//! Q1b's second stage. Given a turn the [`heuristic`] gate
//! flagged as a candidate, this module asks an LLM:
//!
//! 1. Is the turn pattern worth proposing as a Persona
//!    refinement (any category)?
//! 2. With what confidence?
//! 3. If yes, which category — and draft the proposal in
//!    the shape that category expects.
//! 4. Does it semantically duplicate any existing Persona
//!    entry?
//!
//! All four questions in **one** LLM round-trip per Q4b:
//! the dedup check piggybacks on the same call so the auto-
//! proposer pays a single LLM cost per candidate, not two.
//!
//! ## Why structured JSON output (not tool-call)
//!
//! Phase 87 (phrasing) and Phase 91 (judgment) both settled
//! on "ask the LLM to return structured JSON" as the leverage
//! shape for one-shot judgment-style calls. The tool-call
//! protocol is the right shape for *interactive* loops where
//! the LLM might call back; here we want a single-shot
//! verdict. JSON-only response keeps the call cheap (one
//! step, no tool-loop overhead).
//!
//! ## Parser tolerance
//!
//! Real LLMs sometimes wrap JSON in markdown fences or add a
//! short preamble even when told not to. The parser walks the
//! response looking for the first balanced `{...}` block and
//! parses that. If parsing fails, [`judge`] returns
//! [`JudgeError::ParseFailure`] carrying the raw response; the
//! Task 4 background-task wiring logs this as `judge-error`
//! outcome and moves on without firing a proposal.
//!
//! [`heuristic`]: super::heuristic

use std::sync::Arc;

use aivyx_llm::{ContentBlock, LlmError, LlmMessage, LlmProvider, LlmRequest, LlmStepEnd};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use super::heuristic::FailureKind;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A snapshot of one existing approved skill, in the form the
/// judge prompt needs for the dedup check. Built by the
/// caller from the Persona chain at judge-call time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExistingSkillSnapshot {
    pub name: String,
    pub trigger: String,
    /// First ~200 chars of the procedure body. Full procedure
    /// not sent — keeps prompt tokens bounded when the skill
    /// set grows. The trigger + summary is enough signal for
    /// the LLM to decide "this is the same skill."
    pub procedure_summary: String,
}

/// Phase 114 — Full Persona snapshot the judge sees for
/// cross-category dedup and pattern-awareness. Built by the
/// caller from the daemon's `SharedEffectivePersona` at
/// judge-call time. Every field is owned for prompt-budget
/// predictability (the snapshot can be truncated by the
/// caller before being passed to the judge if a category
/// has grown too large).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExistingPersonaSnapshot {
    pub assistant_name: Option<String>,
    pub operator_profile: Option<String>,
    pub communication_style: Option<String>,
    pub primary_use_cases: Vec<String>,
    pub behavioral_preferences: Vec<String>,
    pub behavioral_constraints: Vec<String>,
    pub learned_context: Vec<String>,
    pub communication_adaptations: Vec<String>,
    pub character_traits: Vec<String>,
    pub relationship_milestones: Vec<String>,
    pub learned_skills: Vec<ExistingSkillSnapshot>,
}

impl ExistingPersonaSnapshot {
    /// `true` when no field has any populated content. Useful
    /// for the prompt builder's "(no Persona state yet)"
    /// short-circuit.
    pub fn is_empty(&self) -> bool {
        self.assistant_name.is_none()
            && self.operator_profile.is_none()
            && self.communication_style.is_none()
            && self.primary_use_cases.is_empty()
            && self.behavioral_preferences.is_empty()
            && self.behavioral_constraints.is_empty()
            && self.learned_context.is_empty()
            && self.communication_adaptations.is_empty()
            && self.character_traits.is_empty()
            && self.relationship_milestones.is_empty()
            && self.learned_skills.is_empty()
    }
}

/// Phase 115 — the source of a judge call. Distinguishes
/// Phase 114's positive-pattern path (turn completed
/// successfully; what pattern is worth saving?) from the
/// Phase 115 negative-feedback path (turn failed; what
/// refinement would prevent recurrence?). The system prompt
/// stays unified; the user prompt fans out per source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposalSource {
    /// Phase 114 positive-pattern path: `TurnOutcome::
    /// Completed`. The judge looks for reusable multi-step
    /// patterns worth saving as Persona refinements.
    CompletedTurn,
    /// Phase 115 negative-feedback path: a non-Completed
    /// `TurnOutcome`. The judge looks for refinements that
    /// would prevent this kind of failure from recurring.
    FailedTurn {
        /// Which failure-outcome variant fired the pipeline.
        kind: FailureKind,
        /// Short narrative of what went wrong — error
        /// message, cancellation context, timeout reason,
        /// etc. Caller-formed; budget ~200 chars.
        summary: String,
    },
}

impl ProposalSource {
    /// Short stable label for the source — used in audit
    /// events to distinguish completion-source from
    /// failure-source proposals.
    pub fn label(&self) -> &'static str {
        match self {
            ProposalSource::CompletedTurn => "completed_turn",
            ProposalSource::FailedTurn { .. } => "failed_turn",
        }
    }
}

impl Default for ProposalSource {
    /// Backward-compatibility default: callers built before
    /// Phase 115 that don't set the field get the Phase 114
    /// behavior (CompletedTurn).
    fn default() -> Self {
        ProposalSource::CompletedTurn
    }
}

/// Input to [`judge`]. Caller builds this from the
/// just-finalized turn's signals + the current Persona
/// snapshot.
#[derive(Debug, Clone)]
pub struct JudgeRequest<'a> {
    /// Short narrative of what happened in the turn — user
    /// input excerpt + tool calls made (names + brief input
    /// summary) + final reply excerpt. The caller is free to
    /// truncate; the judge prompt assumes the summary is
    /// already operator-budget-shaped (target ~500-800
    /// tokens).
    pub turn_summary: &'a str,

    /// Phase 114 — full Persona snapshot. The judge scans
    /// this for the cross-category dedup check and for
    /// pattern-awareness (e.g. "the operator already has a
    /// CommunicationStyle field set; don't propose a
    /// conflicting BehavioralPreferences refinement").
    pub existing_persona: &'a ExistingPersonaSnapshot,

    /// Provider-specific model identifier. Operator-configured
    /// in the TOML `[persona.auto_propose] judge_model` field
    /// (Phase 114 Task 3, alias of Phase 113's
    /// `[skills.auto_propose] judge_model`).
    pub model: &'a str,

    /// Maximum tokens the judge may emit. Defaults to a
    /// generous-but-bounded 800 if not overridden; long
    /// enough for a full draft + reasoning, short enough to
    /// keep cost predictable.
    pub max_tokens: u32,

    /// Phase 115 — what triggered this judge call. Phase 114
    /// callers passed `ProposalSource::CompletedTurn` by
    /// default; the negative-feedback path passes
    /// `ProposalSource::FailedTurn { .. }` carrying the
    /// failure context.
    pub source: ProposalSource,
}

/// What the judge actually proposes when it decides a turn is
/// proposal-worthy. Mirrors the `LearnedSkill` shape from
/// `aivyx-channel::persona`, but kept independent here so
/// `aivyx-core` doesn't take on a dep edge upward.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillDraft {
    /// kebab-case slug, suitable for `skills.invoke` lookup.
    pub name: String,
    /// Short one-sentence "use this skill when …" trigger.
    pub trigger: String,
    /// Multi-line markdown procedure body. The agent reads
    /// this when invoking the skill.
    pub procedure: String,
}

/// Phase 114 — Polymorphic proposed-draft shape, varying by
/// `PersonaDeltaCategory`. The `kind` discriminator is set
/// by the LLM judge to one of `"LearnedSkill"`,
/// `"ListAppend"`, or `"ScalarSet"`; serde-tagged so the
/// JSON wire form is self-describing.
///
/// Category → variant mapping (the caller validates the
/// pair):
/// - `LearnedSkill` → [`ProposedDraft::LearnedSkill`]
/// - `PrimaryUseCases`, `BehavioralPreferences`,
///   `BehavioralConstraints`, `LearnedContext`,
///   `CommunicationAdaptations`, `CharacterTraits`,
///   `RelationshipMilestones` → [`ProposedDraft::ListAppend`]
/// - `AssistantName`, `OperatorProfile`,
///   `CommunicationStyle` → [`ProposedDraft::ScalarSet`]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ProposedDraft {
    /// Full `LearnedSkill` shape (name + trigger + procedure).
    /// Used iff the judge picks `category = "LearnedSkill"`.
    LearnedSkill {
        name: String,
        trigger: String,
        procedure: String,
    },
    /// Append a string to a list category. Used for the seven
    /// list-shaped categories.
    ListAppend { value: String },
    /// Set a scalar category's value. Used for the three
    /// scalar categories (`AssistantName`, `OperatorProfile`,
    /// `CommunicationStyle`).
    ScalarSet { value: String },
}

impl ProposedDraft {
    /// If this draft is a `LearnedSkill`, return the
    /// `SkillDraft` view. Backwards-compatibility helper for
    /// callers that already consumed the Phase 113 shape.
    pub fn as_skill_draft(&self) -> Option<SkillDraft> {
        match self {
            ProposedDraft::LearnedSkill {
                name,
                trigger,
                procedure,
            } => Some(SkillDraft {
                name: name.clone(),
                trigger: trigger.clone(),
                procedure: procedure.clone(),
            }),
            _ => None,
        }
    }

    /// Short stable label for the variant — used in audit
    /// events and operator-facing messages.
    pub fn kind_label(&self) -> &'static str {
        match self {
            ProposedDraft::LearnedSkill { .. } => "LearnedSkill",
            ProposedDraft::ListAppend { .. } => "ListAppend",
            ProposedDraft::ScalarSet { .. } => "ScalarSet",
        }
    }

    /// Operator-readable label for the proposed draft —
    /// `LearnedSkill` returns the kebab-case name, list/scalar
    /// variants return a truncated value. Used as the
    /// `proposed_skill_name` audit-event field (Phase 112's
    /// name kept for chain backward compatibility, generalized
    /// in semantics at Phase 114).
    pub fn display_name(&self) -> String {
        match self {
            ProposedDraft::LearnedSkill { name, .. } => name.clone(),
            ProposedDraft::ListAppend { value }
            | ProposedDraft::ScalarSet { value } => {
                let mut truncated: String =
                    value.chars().take(80).collect();
                if value.chars().count() > 80 {
                    truncated.push('…');
                }
                truncated
            }
        }
    }
}

/// The structured response shape the judge LLM must produce.
/// Returned as JSON; parsed by [`parse_judge_response`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JudgeResponse {
    /// Top-line verdict. If `false`, the turn pattern isn't
    /// general enough or recurring enough to warrant a
    /// Persona refinement.
    pub is_worth_proposing: bool,

    /// LLM's self-reported confidence, 0.0 – 1.0. The
    /// threshold-gate compares this against the operator-
    /// configured `auto_accept_confidence_threshold` for the
    /// picked category.
    pub confidence: f32,

    /// Phase 114 — the `PersonaDeltaCategory` label the judge
    /// picked. `None` when `is_worth_proposing == false` or
    /// the judge declined to commit to a category.
    /// Expected values (the runtime validates):
    /// `"AssistantName"`, `"OperatorProfile"`,
    /// `"CommunicationStyle"`, `"PrimaryUseCases"`,
    /// `"BehavioralPreferences"`, `"BehavioralConstraints"`,
    /// `"LearnedContext"`, `"CommunicationAdaptations"`,
    /// `"CharacterTraits"`, `"RelationshipMilestones"`,
    /// `"LearnedSkill"`.
    #[serde(default)]
    pub category: Option<String>,

    /// Phase 114 — the drafted proposal in the shape the
    /// picked category expects. Populated iff
    /// `is_worth_proposing == true` and `category` is set.
    /// (The parser doesn't enforce the cross-field
    /// constraint — a downstream consumer can decide
    /// whether to require non-None here.)
    #[serde(default)]
    pub proposed_draft: Option<ProposedDraft>,

    /// Name of an existing Persona entry this candidate
    /// semantically duplicates, if any. The Q4b dedup signal
    /// — populated when the LLM concludes the candidate
    /// paraphrases an existing entry even though title
    /// fuzzy-match didn't catch it. For `LearnedSkill`, this
    /// is the skill name; for list categories, it's the
    /// existing list-item value; for scalar categories, it's
    /// a description of the conflicting scalar.
    pub is_duplicate_of: Option<String>,

    /// Optional short rationale. Useful for the audit log
    /// and for operator inspection of the auto-proposer's
    /// behavior.
    #[serde(default)]
    pub reasoning: Option<String>,
}

impl JudgeResponse {
    /// Backwards-compatibility helper: if the picked category
    /// is `LearnedSkill`, return the draft as a `SkillDraft`.
    /// Phase 113 callers expecting `proposed_skill` flow can
    /// call this without changing their match shape.
    pub fn proposed_skill(&self) -> Option<SkillDraft> {
        self.proposed_draft
            .as_ref()
            .and_then(|d| d.as_skill_draft())
    }
}

#[derive(Debug, Error)]
pub enum JudgeError {
    #[error("LLM provider error: {0}")]
    Provider(#[from] LlmError),

    #[error("judge response was not parseable JSON: {raw}")]
    ParseFailure { raw: String },

    #[error("judge confidence out of range [0.0, 1.0]: {0}")]
    ConfidenceOutOfRange(f32),
}

// ---------------------------------------------------------------------------
// Prompt construction
// ---------------------------------------------------------------------------

/// Build the system prompt the judge sees. Stable, low-
/// variance content — keeps the LLM's behavior predictable
/// turn-to-turn. The pin-the-shape unit test below treats this
/// as a golden value.
pub fn build_system_prompt() -> String {
    String::from(
        "You are a Persona-refinement judge for an AI personal \
assistant. Your job is to look at one turn (completed OR \
failed) and decide whether to propose a Persona refinement. \
\n\nThere are two source modes: \n\
- Completed turn (Phase 114 positive-pattern path): look for \
  reusable multi-step patterns worth saving as Persona \
  refinements.\n\
- Failed turn (Phase 115 negative-feedback path): the agent \
  failed, timed out, was cancelled, or escalated. Look for \
  refinements that would prevent this kind of failure from \
  recurring next time — typically BehavioralConstraints \
  (\"never X\"), LearnedContext (\"remember Y\"), or \
  CommunicationAdaptations (\"phrase Z this way\").\n\
\n\
The agent's Persona has eleven categories; you pick the right \
one and draft the refinement in the shape that category expects. \
\n\nThe eleven categories (and their expected draft shape):\n\
- LearnedSkill — a named procedure the assistant can invoke. \
  Draft: { kind: \"LearnedSkill\", name: kebab-case string, \
  trigger: string, procedure: markdown string }.\n\
- BehavioralPreferences — a single string the operator prefers \
  the assistant to follow (\"prefer terse replies\"). Draft: \
  { kind: \"ListAppend\", value: string }.\n\
- BehavioralConstraints — a single string the assistant must \
  avoid (\"never run shell commands without operator approval\"). \
  Draft: { kind: \"ListAppend\", value: string }.\n\
- LearnedContext — a fact about the operator's world the \
  assistant should remember (\"the operator's primary repo is \
  aivyx\"). Draft: { kind: \"ListAppend\", value: string }.\n\
- CommunicationAdaptations — a tone or style adaptation \
  (\"the operator likes brief responses with code blocks\"). \
  Draft: { kind: \"ListAppend\", value: string }.\n\
- CharacterTraits — a personality trait that emerged across \
  many turns (\"curious about underlying mechanisms\"). \
  Draft: { kind: \"ListAppend\", value: string }.\n\
- RelationshipMilestones — a noteworthy moment in the \
  operator-assistant relationship. \
  Draft: { kind: \"ListAppend\", value: string }.\n\
- PrimaryUseCases — a use-case the operator actually uses \
  the assistant for. Draft: { kind: \"ListAppend\", \
  value: string }.\n\
- AssistantName — a name for the assistant (scalar; rare). \
  Draft: { kind: \"ScalarSet\", value: string }.\n\
- OperatorProfile — a short identity-summary of the operator \
  (scalar; rare). Draft: { kind: \"ScalarSet\", value: \
  string }.\n\
- CommunicationStyle — the assistant's overall tone (scalar; \
  rare). Draft: { kind: \"ScalarSet\", value: string }.\n\
\n\
Return ONLY a single JSON object matching this schema:\n\
{\n\
  \"is_worth_proposing\": bool,\n\
  \"confidence\": float in [0.0, 1.0],\n\
  \"category\": one of the eleven category names | null,\n\
  \"proposed_draft\": draft object (shape per the category) \
  | null,\n\
  \"is_duplicate_of\": existing entry name/value/description \
  | null,\n\
  \"reasoning\": short string explaining the verdict\n\
}\n\n\
Criteria:\n\
- Worth proposing: the turn shows a recurring or generalizable \
pattern worth saving, not a one-off chat. Pick the category \
that fits best.\n\
- Confidence: how strongly the pattern fits the picked \
category. Reserve >= 0.85 for clear, well-defined, recurring \
patterns. Scalar categories (AssistantName, OperatorProfile, \
CommunicationStyle) need very high confidence (>= 0.95) \
because each new value replaces the previous one.\n\
- Duplicates: if the candidate is semantically the same as an \
existing Persona entry in ANY category, set is_duplicate_of \
to a description of that entry and is_worth_proposing to \
false.\n\
- No prose outside the JSON. No markdown fences. JSON only.",
    )
}

/// Build the user prompt the judge sees for one candidate
/// turn. Embeds the turn summary and the full Persona
/// snapshot (for cross-category dedup and pattern-
/// awareness). Phase 115 — also embeds the failure context
/// when the source is `FailedTurn`.
pub fn build_user_prompt(request: &JudgeRequest<'_>) -> String {
    let mut s = String::new();
    match &request.source {
        ProposalSource::CompletedTurn => {
            s.push_str("## Completed turn\n\n");
            s.push_str(request.turn_summary);
        }
        ProposalSource::FailedTurn { kind, summary } => {
            s.push_str("## Failed turn (correction context)\n\n");
            s.push_str(&format!(
                "**Failure kind:** `{}`\n\n**Failure summary:** {}\n\n",
                kind.label(),
                summary,
            ));
            s.push_str("**Turn narrative:**\n\n");
            s.push_str(request.turn_summary);
            s.push_str(
                "\n\nYour job: pick a Persona refinement that would prevent \
this kind of failure from recurring. Prefer BehavioralConstraints (\"never X\"), \
LearnedContext (\"remember Y\"), or CommunicationAdaptations (\"phrase Z this \
way\"). If the failure isn't actionable (e.g. transient network error, \
operator changed their mind for unrelated reasons), set is_worth_proposing \
to false.",
            );
        }
    }
    s.push_str("\n\n## Current Persona state");
    let p = request.existing_persona;
    if p.is_empty() {
        s.push_str("\n\n(no Persona state yet — every category is empty)\n\n");
    } else {
        s.push_str(" (for cross-category dedup and pattern-awareness)\n\n");
        if let Some(v) = &p.assistant_name {
            s.push_str(&format!("- **AssistantName** (scalar): {v}\n"));
        }
        if let Some(v) = &p.operator_profile {
            s.push_str(&format!("- **OperatorProfile** (scalar): {v}\n"));
        }
        if let Some(v) = &p.communication_style {
            s.push_str(&format!("- **CommunicationStyle** (scalar): {v}\n"));
        }
        render_list(&mut s, "PrimaryUseCases", &p.primary_use_cases);
        render_list(
            &mut s,
            "BehavioralPreferences",
            &p.behavioral_preferences,
        );
        render_list(
            &mut s,
            "BehavioralConstraints",
            &p.behavioral_constraints,
        );
        render_list(&mut s, "LearnedContext", &p.learned_context);
        render_list(
            &mut s,
            "CommunicationAdaptations",
            &p.communication_adaptations,
        );
        render_list(&mut s, "CharacterTraits", &p.character_traits);
        render_list(
            &mut s,
            "RelationshipMilestones",
            &p.relationship_milestones,
        );
        if !p.learned_skills.is_empty() {
            s.push_str("- **LearnedSkill** (list of named procedures):\n");
            for skill in &p.learned_skills {
                s.push_str(&format!(
                    "  - **{}** — trigger: {}\n    summary: {}\n",
                    skill.name, skill.trigger, skill.procedure_summary
                ));
            }
        }
        s.push('\n');
    }
    s.push_str("Respond with the JudgeResponse JSON now.");
    s
}

fn render_list(out: &mut String, label: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    out.push_str(&format!("- **{label}** (list):\n"));
    for item in items {
        out.push_str(&format!("  - {item}\n"));
    }
}

// ---------------------------------------------------------------------------
// JSON extraction (parser tolerance per the module doc)
// ---------------------------------------------------------------------------

/// Pull the first balanced `{...}` JSON object out of an LLM
/// response. Tolerant of markdown fences and short preamble.
/// Returns the substring with the braces; the caller parses
/// that with `serde_json`.
fn extract_first_json_object(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let mut depth: i32 = 0;
    let mut start: Option<usize> = None;
    let mut in_string = false;
    let mut escape = false;
    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_string = false;
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    if let Some(s_idx) = start {
                        return std::str::from_utf8(&bytes[s_idx..=i]).ok();
                    }
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse a raw LLM response into a [`JudgeResponse`]. Tolerant
/// of markdown fences and short preamble per the module doc.
pub fn parse_judge_response(raw: &str) -> Result<JudgeResponse, JudgeError> {
    let json = extract_first_json_object(raw)
        .ok_or_else(|| JudgeError::ParseFailure { raw: raw.to_string() })?;
    let parsed: JudgeResponse = serde_json::from_str(json)
        .map_err(|_| JudgeError::ParseFailure { raw: raw.to_string() })?;
    if !(0.0..=1.0).contains(&parsed.confidence) {
        return Err(JudgeError::ConfidenceOutOfRange(parsed.confidence));
    }
    Ok(parsed)
}

// ---------------------------------------------------------------------------
// The async entry point
// ---------------------------------------------------------------------------

/// Fire the judge call against the configured provider. Drains
/// the stream (the judge is single-shot text; tool calls are
/// not expected) and parses the terminal `FinalMessage` text
/// into a [`JudgeResponse`].
///
/// If the provider returns `LlmStepEnd::ToolCalls` (the judge
/// shouldn't ever ask for a tool, but a misbehaving model
/// might), the function fails with [`JudgeError::ParseFailure`]
/// carrying a placeholder note — the Task 4 wiring treats
/// either failure mode the same way (`judge-error` outcome).
pub async fn judge(
    provider: Arc<dyn LlmProvider>,
    request: JudgeRequest<'_>,
    cancellation: &CancellationToken,
) -> Result<JudgeResponse, JudgeError> {
    let system = build_system_prompt();
    let user = build_user_prompt(&request);

    let messages = vec![LlmMessage::User {
        content: vec![ContentBlock::Text { text: user }],
    }];

    let llm_request = LlmRequest {
        model: request.model,
        system: Some(&system),
        messages: &messages,
        tools: &[],
        max_tokens: request.max_tokens,
        temperature: Some(0.2), // low temp for stable judgment
    };

    let mut stream = provider.chat_stream(llm_request, cancellation).await?;
    // Drain mid-stream events; the judge is text-only.
    while (stream.next_event().await?).is_some() {}

    let terminal = stream.finish().await?;
    let text = match terminal {
        LlmStepEnd::FinalMessage { text, .. } => text,
        LlmStepEnd::ToolCalls { .. } => {
            return Err(JudgeError::ParseFailure {
                raw: "judge emitted ToolCalls instead of FinalMessage".to_string(),
            });
        }
    };

    parse_judge_response(&text)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    // ----- A minimal scripted FakeLlmProvider that returns
    // predetermined FinalMessage text -----

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
        ) -> Result<Box<dyn aivyx_llm::LlmStream>, LlmError> {
            let text = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| {
                    LlmError::Config("ScriptedProvider exhausted".to_string())
                })?;
            Ok(Box::new(ScriptedStream { text: Some(text) }))
        }
    }

    struct ScriptedStream {
        text: Option<String>,
    }

    #[async_trait]
    impl aivyx_llm::LlmStream for ScriptedStream {
        async fn next_event(
            &mut self,
        ) -> Result<Option<aivyx_llm::LlmStreamEvent>, LlmError> {
            Ok(None) // skip mid-stream events; jump to terminal
        }
        async fn finish(self: Box<Self>) -> Result<LlmStepEnd, LlmError> {
            Ok(LlmStepEnd::FinalMessage {
                text: self.text.unwrap_or_default(),
                usage: aivyx_llm::LlmUsage::default(),
            })
        }
    }

    fn empty_persona() -> ExistingPersonaSnapshot {
        ExistingPersonaSnapshot::default()
    }

    // ----- Prompt-shape stability (golden) -----

    #[test]
    fn system_prompt_includes_the_schema_and_criteria() {
        let p = build_system_prompt();
        assert!(p.contains("is_worth_proposing"));
        assert!(p.contains("confidence"));
        assert!(p.contains("category"));
        assert!(p.contains("proposed_draft"));
        assert!(p.contains("is_duplicate_of"));
        assert!(p.contains("reasoning"));
        assert!(p.contains("JSON only"));
    }

    #[test]
    fn system_prompt_lists_all_eleven_categories() {
        let p = build_system_prompt();
        for cat in [
            "LearnedSkill",
            "BehavioralPreferences",
            "BehavioralConstraints",
            "LearnedContext",
            "CommunicationAdaptations",
            "CharacterTraits",
            "RelationshipMilestones",
            "PrimaryUseCases",
            "AssistantName",
            "OperatorProfile",
            "CommunicationStyle",
        ] {
            assert!(p.contains(cat), "system prompt missing category {cat}");
        }
    }

    #[test]
    fn system_prompt_lists_all_three_draft_kinds() {
        let p = build_system_prompt();
        for kind in ["LearnedSkill", "ListAppend", "ScalarSet"] {
            assert!(p.contains(kind), "system prompt missing draft kind {kind}");
        }
    }

    #[test]
    fn user_prompt_embeds_turn_summary_and_empty_persona() {
        let persona = empty_persona();
        let req = JudgeRequest {
            turn_summary: "User asked X; agent ran fs.read, web.fetch; replied Y.",
            existing_persona: &persona,
            model: "claude-haiku-4-5",
            max_tokens: 800,
            source: ProposalSource::CompletedTurn,
        };
        let p = build_user_prompt(&req);
        assert!(p.contains("User asked X"));
        assert!(p.contains("no Persona state yet"));
    }

    #[test]
    fn user_prompt_lists_existing_persona_state_across_categories() {
        let persona = ExistingPersonaSnapshot {
            assistant_name: Some("Aivyx".into()),
            behavioral_preferences: vec![
                "prefer terse replies".into(),
                "use code blocks for shell commands".into(),
            ],
            learned_skills: vec![ExistingSkillSnapshot {
                name: "research-topic".into(),
                trigger: "user asks 'research X'".into(),
                procedure_summary: "fs.read project notes, web.fetch ...".into(),
            }],
            ..ExistingPersonaSnapshot::default()
        };
        let req = JudgeRequest {
            turn_summary: "ignored",
            existing_persona: &persona,
            model: "m",
            max_tokens: 800,
            source: ProposalSource::CompletedTurn,
        };
        let p = build_user_prompt(&req);
        assert!(p.contains("AssistantName"));
        assert!(p.contains("Aivyx"));
        assert!(p.contains("BehavioralPreferences"));
        assert!(p.contains("prefer terse replies"));
        assert!(p.contains("LearnedSkill"));
        assert!(p.contains("research-topic"));
        assert!(p.contains("cross-category dedup"));
    }

    // ----- Parser tolerance -----

    #[test]
    fn parses_clean_learned_skill_response() {
        let raw = r#"{"is_worth_proposing":true,"confidence":0.91,
          "category":"LearnedSkill",
          "proposed_draft":{"kind":"LearnedSkill","name":"research-topic",
          "trigger":"research X","procedure":"step 1 ..."},
          "is_duplicate_of":null,"reasoning":"recurring multi-step pattern"}"#;
        let r = parse_judge_response(raw).expect("parse");
        assert!(r.is_worth_proposing);
        assert!((r.confidence - 0.91).abs() < 1e-6);
        assert_eq!(r.category.as_deref(), Some("LearnedSkill"));
        match r.proposed_draft.as_ref().unwrap() {
            ProposedDraft::LearnedSkill { name, .. } => {
                assert_eq!(name, "research-topic");
            }
            other => panic!("expected LearnedSkill draft, got {other:?}"),
        }
        // Backward-compat helper still works.
        assert_eq!(r.proposed_skill().unwrap().name, "research-topic");
    }

    #[test]
    fn parses_list_append_response() {
        let raw = r#"{"is_worth_proposing":true,"confidence":0.88,
          "category":"BehavioralPreferences",
          "proposed_draft":{"kind":"ListAppend",
          "value":"prefer terse replies for command-style requests"},
          "is_duplicate_of":null}"#;
        let r = parse_judge_response(raw).expect("parse");
        assert!(r.is_worth_proposing);
        assert_eq!(r.category.as_deref(), Some("BehavioralPreferences"));
        match r.proposed_draft.as_ref().unwrap() {
            ProposedDraft::ListAppend { value } => {
                assert!(value.contains("terse"));
            }
            other => panic!("expected ListAppend, got {other:?}"),
        }
        // Backward-compat helper returns None for non-skill drafts.
        assert!(r.proposed_skill().is_none());
    }

    #[test]
    fn parses_scalar_set_response() {
        let raw = r#"{"is_worth_proposing":true,"confidence":0.96,
          "category":"AssistantName",
          "proposed_draft":{"kind":"ScalarSet","value":"Aivyx"},
          "is_duplicate_of":null}"#;
        let r = parse_judge_response(raw).expect("parse");
        assert!(r.is_worth_proposing);
        assert_eq!(r.category.as_deref(), Some("AssistantName"));
        match r.proposed_draft.as_ref().unwrap() {
            ProposedDraft::ScalarSet { value } => {
                assert_eq!(value, "Aivyx");
            }
            other => panic!("expected ScalarSet, got {other:?}"),
        }
    }

    #[test]
    fn parses_response_wrapped_in_markdown_fences() {
        let raw = r#"Sure, here you go:
```json
{"is_worth_proposing":false,"confidence":0.4,"category":null,
 "proposed_draft":null,"is_duplicate_of":null,"reasoning":"one-off chat"}
```
that's my call."#;
        let r = parse_judge_response(raw).expect("parse");
        assert!(!r.is_worth_proposing);
        assert!(r.proposed_draft.is_none());
        assert!(r.category.is_none());
    }

    #[test]
    fn parses_response_with_short_preamble() {
        let raw = r#"Here's the verdict: {"is_worth_proposing":true,
          "confidence":0.88,"category":"LearnedSkill",
          "proposed_draft":{"kind":"LearnedSkill","name":"a","trigger":"b",
          "procedure":"c"},"is_duplicate_of":null}"#;
        let r = parse_judge_response(raw).expect("parse");
        assert!(r.is_worth_proposing);
    }

    #[test]
    fn parses_response_with_dup_set() {
        let raw = r#"{"is_worth_proposing":false,"confidence":0.95,
          "category":null,"proposed_draft":null,
          "is_duplicate_of":"research-topic",
          "reasoning":"paraphrase of an existing skill"}"#;
        let r = parse_judge_response(raw).expect("parse");
        assert!(!r.is_worth_proposing);
        assert_eq!(r.is_duplicate_of.as_deref(), Some("research-topic"));
    }

    #[test]
    fn parses_response_with_omitted_optional_fields() {
        // category, proposed_draft, reasoning are all #[serde(default)]
        // optional — the judge may omit them when not relevant.
        let raw = r#"{"is_worth_proposing":false,"confidence":0.2,
          "is_duplicate_of":null}"#;
        let r = parse_judge_response(raw).expect("parse");
        assert!(!r.is_worth_proposing);
        assert!(r.category.is_none());
        assert!(r.proposed_draft.is_none());
    }

    #[test]
    fn rejects_response_with_no_json_object() {
        let raw = "I'm not sure what to make of this turn.";
        let err = parse_judge_response(raw).unwrap_err();
        matches!(err, JudgeError::ParseFailure { .. });
    }

    #[test]
    fn rejects_response_with_invalid_json() {
        let raw = r#"{ not really json }"#;
        assert!(parse_judge_response(raw).is_err());
    }

    #[test]
    fn rejects_confidence_above_one() {
        let raw = r#"{"is_worth_proposing":true,"confidence":1.5,
          "category":null,"proposed_draft":null,"is_duplicate_of":null}"#;
        let err = parse_judge_response(raw).unwrap_err();
        matches!(err, JudgeError::ConfidenceOutOfRange(_));
    }

    #[test]
    fn rejects_confidence_below_zero() {
        let raw = r#"{"is_worth_proposing":true,"confidence":-0.1,
          "category":null,"proposed_draft":null,"is_duplicate_of":null}"#;
        let err = parse_judge_response(raw).unwrap_err();
        matches!(err, JudgeError::ConfidenceOutOfRange(_));
    }

    #[test]
    fn extracts_balanced_object_in_presence_of_inner_braces() {
        // The procedure string contains `{` — extractor must
        // respect quoting, not just brace count.
        let raw = r#"{"is_worth_proposing":true,"confidence":0.9,
          "category":"LearnedSkill",
          "proposed_draft":{"kind":"LearnedSkill","name":"x","trigger":"y",
          "procedure":"call shell.exec with {arg: value}"},
          "is_duplicate_of":null}"#;
        let r = parse_judge_response(raw).expect("parse");
        assert!(r.is_worth_proposing);
        let proc_text = r.proposed_skill().unwrap().procedure;
        assert!(proc_text.contains("{arg: value}"));
    }

    // ----- Integration via scripted provider -----

    #[tokio::test]
    async fn judge_returns_worth_proposing_branch_for_learned_skill() {
        let provider = ScriptedProvider::new(vec![
            r#"{"is_worth_proposing":true,"confidence":0.92,
               "category":"LearnedSkill",
               "proposed_draft":{"kind":"LearnedSkill","name":"a",
               "trigger":"t","procedure":"p"},
               "is_duplicate_of":null,"reasoning":"r"}"#,
        ]);
        let persona = empty_persona();
        let req = JudgeRequest {
            turn_summary: "summary",
            existing_persona: &persona,
            model: "m",
            max_tokens: 800,
            source: ProposalSource::CompletedTurn,
        };
        let cancel = CancellationToken::new();
        let resp = judge(provider, req, &cancel).await.expect("ok");
        assert!(resp.is_worth_proposing);
        assert!((resp.confidence - 0.92).abs() < 1e-6);
        assert_eq!(resp.category.as_deref(), Some("LearnedSkill"));
        assert_eq!(resp.proposed_skill().unwrap().name, "a");
    }

    #[tokio::test]
    async fn judge_returns_list_append_for_behavioral_preference() {
        let provider = ScriptedProvider::new(vec![
            r#"{"is_worth_proposing":true,"confidence":0.88,
               "category":"BehavioralPreferences",
               "proposed_draft":{"kind":"ListAppend",
               "value":"prefer terse replies"},
               "is_duplicate_of":null}"#,
        ]);
        let persona = empty_persona();
        let req = JudgeRequest {
            turn_summary: "summary",
            existing_persona: &persona,
            model: "m",
            max_tokens: 800,
            source: ProposalSource::CompletedTurn,
        };
        let cancel = CancellationToken::new();
        let resp = judge(provider, req, &cancel).await.expect("ok");
        assert_eq!(resp.category.as_deref(), Some("BehavioralPreferences"));
        match resp.proposed_draft.unwrap() {
            ProposedDraft::ListAppend { value } => {
                assert!(value.contains("terse"));
            }
            other => panic!("expected ListAppend, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn judge_returns_dup_branch() {
        let provider = ScriptedProvider::new(vec![
            r#"{"is_worth_proposing":false,"confidence":0.96,
               "category":null,"proposed_draft":null,
               "is_duplicate_of":"existing-entry","reasoning":"dup"}"#,
        ]);
        let persona = empty_persona();
        let req = JudgeRequest {
            turn_summary: "summary",
            existing_persona: &persona,
            model: "m",
            max_tokens: 800,
            source: ProposalSource::CompletedTurn,
        };
        let cancel = CancellationToken::new();
        let resp = judge(provider, req, &cancel).await.expect("ok");
        assert!(!resp.is_worth_proposing);
        assert_eq!(resp.is_duplicate_of.as_deref(), Some("existing-entry"));
    }

    #[tokio::test]
    async fn judge_returns_not_worth_branch() {
        let provider = ScriptedProvider::new(vec![
            r#"{"is_worth_proposing":false,"confidence":0.3,
               "category":null,"proposed_draft":null,
               "is_duplicate_of":null,"reasoning":"one-off chat"}"#,
        ]);
        let persona = empty_persona();
        let req = JudgeRequest {
            turn_summary: "summary",
            existing_persona: &persona,
            model: "m",
            max_tokens: 800,
            source: ProposalSource::CompletedTurn,
        };
        let cancel = CancellationToken::new();
        let resp = judge(provider, req, &cancel).await.expect("ok");
        assert!(!resp.is_worth_proposing);
    }

    #[tokio::test]
    async fn judge_propagates_parse_failure_for_garbage_response() {
        let provider = ScriptedProvider::new(vec!["this is not json at all"]);
        let persona = empty_persona();
        let req = JudgeRequest {
            turn_summary: "summary",
            existing_persona: &persona,
            model: "m",
            max_tokens: 800,
            source: ProposalSource::CompletedTurn,
        };
        let cancel = CancellationToken::new();
        let err = judge(provider, req, &cancel).await.unwrap_err();
        matches!(err, JudgeError::ParseFailure { .. });
    }

    // ----- Serde round-trip -----

    #[test]
    fn judge_response_round_trips_for_learned_skill() {
        let original = JudgeResponse {
            is_worth_proposing: true,
            confidence: 0.87,
            category: Some("LearnedSkill".into()),
            proposed_draft: Some(ProposedDraft::LearnedSkill {
                name: "research-topic".into(),
                trigger: "user asks 'research X'".into(),
                procedure: "1. fs.read\n2. web.fetch\n3. summarize".into(),
            }),
            is_duplicate_of: None,
            reasoning: Some("multi-step recurring pattern".into()),
        };
        let s = serde_json::to_string(&original).unwrap();
        let back: JudgeResponse = serde_json::from_str(&s).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn judge_response_round_trips_for_list_append() {
        let original = JudgeResponse {
            is_worth_proposing: true,
            confidence: 0.78,
            category: Some("LearnedContext".into()),
            proposed_draft: Some(ProposedDraft::ListAppend {
                value: "the operator's primary repo is aivyx".into(),
            }),
            is_duplicate_of: None,
            reasoning: None,
        };
        let s = serde_json::to_string(&original).unwrap();
        let back: JudgeResponse = serde_json::from_str(&s).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn judge_response_round_trips_for_scalar_set() {
        let original = JudgeResponse {
            is_worth_proposing: true,
            confidence: 0.97,
            category: Some("AssistantName".into()),
            proposed_draft: Some(ProposedDraft::ScalarSet {
                value: "Aivyx".into(),
            }),
            is_duplicate_of: None,
            reasoning: None,
        };
        let s = serde_json::to_string(&original).unwrap();
        let back: JudgeResponse = serde_json::from_str(&s).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn existing_skill_snapshot_round_trips() {
        let original = ExistingSkillSnapshot {
            name: "research-topic".into(),
            trigger: "user asks 'research X'".into(),
            procedure_summary: "fs.read + web.fetch ...".into(),
        };
        let s = serde_json::to_string(&original).unwrap();
        let back: ExistingSkillSnapshot = serde_json::from_str(&s).unwrap();
        assert_eq!(back, original);
    }

    #[test]
    fn existing_persona_snapshot_round_trips() {
        let original = ExistingPersonaSnapshot {
            assistant_name: Some("Aivyx".into()),
            behavioral_preferences: vec!["terse".into()],
            learned_skills: vec![ExistingSkillSnapshot {
                name: "x".into(),
                trigger: "y".into(),
                procedure_summary: "z".into(),
            }],
            ..ExistingPersonaSnapshot::default()
        };
        let s = serde_json::to_string(&original).unwrap();
        let back: ExistingPersonaSnapshot =
            serde_json::from_str(&s).unwrap();
        assert_eq!(back, original);
    }

    // ----- ProposedDraft helpers -----

    #[test]
    fn proposed_draft_kind_label_is_stable() {
        let learned = ProposedDraft::LearnedSkill {
            name: "x".into(),
            trigger: "y".into(),
            procedure: "z".into(),
        };
        assert_eq!(learned.kind_label(), "LearnedSkill");

        let list = ProposedDraft::ListAppend { value: "v".into() };
        assert_eq!(list.kind_label(), "ListAppend");

        let scalar = ProposedDraft::ScalarSet { value: "v".into() };
        assert_eq!(scalar.kind_label(), "ScalarSet");
    }

    // ----- Phase 115 — Failure-source prompt + ProposalSource -----

    #[test]
    fn proposal_source_default_is_completed_turn() {
        assert_eq!(ProposalSource::default(), ProposalSource::CompletedTurn);
    }

    #[test]
    fn proposal_source_label_is_stable() {
        assert_eq!(ProposalSource::CompletedTurn.label(), "completed_turn");
        let failed = ProposalSource::FailedTurn {
            kind: FailureKind::Failed,
            summary: "boom".into(),
        };
        assert_eq!(failed.label(), "failed_turn");
    }

    #[test]
    fn system_prompt_mentions_both_source_modes() {
        let p = build_system_prompt();
        assert!(p.contains("Completed turn"));
        assert!(p.contains("Failed turn"));
        assert!(p.contains("BehavioralConstraints"));
        assert!(p.contains("recurring"));
    }

    #[test]
    fn user_prompt_for_completed_source_does_not_mention_failure_context() {
        let persona = empty_persona();
        let req = JudgeRequest {
            turn_summary: "user asked X; agent ran 3 tools",
            existing_persona: &persona,
            model: "m",
            max_tokens: 800,
            source: ProposalSource::CompletedTurn,
        };
        let p = build_user_prompt(&req);
        assert!(p.contains("Completed turn"));
        assert!(!p.contains("Failed turn"));
    }

    #[test]
    fn user_prompt_for_failed_source_embeds_failure_kind_and_summary() {
        let persona = empty_persona();
        let req = JudgeRequest {
            turn_summary: "user asked for X; agent tried Y; planner errored",
            existing_persona: &persona,
            model: "m",
            max_tokens: 800,
            source: ProposalSource::FailedTurn {
                kind: FailureKind::Failed,
                summary: "planner returned MaxStepsExceeded".into(),
            },
        };
        let p = build_user_prompt(&req);
        assert!(p.contains("Failed turn"));
        assert!(p.contains("`failed`"));
        assert!(p.contains("MaxStepsExceeded"));
        assert!(p.contains("BehavioralConstraints"));
    }

    #[test]
    fn user_prompt_for_timed_out_failure_carries_kind_label() {
        let persona = empty_persona();
        let req = JudgeRequest {
            turn_summary: "agent was working on X when budget exhausted",
            existing_persona: &persona,
            model: "m",
            max_tokens: 800,
            source: ProposalSource::FailedTurn {
                kind: FailureKind::TimedOut,
                summary: "exceeded 30 second budget at step 12".into(),
            },
        };
        let p = build_user_prompt(&req);
        assert!(p.contains("`timed_out`"));
        assert!(p.contains("30 second budget"));
    }

    #[test]
    fn proposed_draft_as_skill_draft_returns_some_only_for_learned_skill() {
        let learned = ProposedDraft::LearnedSkill {
            name: "x".into(),
            trigger: "y".into(),
            procedure: "z".into(),
        };
        let sd = learned.as_skill_draft().unwrap();
        assert_eq!(sd.name, "x");
        assert_eq!(sd.trigger, "y");
        assert_eq!(sd.procedure, "z");

        assert!(ProposedDraft::ListAppend { value: "v".into() }
            .as_skill_draft()
            .is_none());
        assert!(ProposedDraft::ScalarSet { value: "v".into() }
            .as_skill_draft()
            .is_none());
    }
}
