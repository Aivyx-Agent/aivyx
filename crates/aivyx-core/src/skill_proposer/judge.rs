//! Phase 112 Task 3 — LLM-judge surface for skill auto-
//! proposal candidates.
//!
//! Q1b's second stage. Given a turn the [`heuristic`] gate
//! flagged as a candidate, this module asks an LLM:
//!
//! 1. Is the turn pattern worth proposing as a learned skill?
//! 2. With what confidence?
//! 3. If yes, draft the skill (name + trigger + procedure).
//! 4. Does it semantically duplicate any existing skill?
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

/// Input to [`judge`]. Caller builds this from the
/// just-finalized turn's signals + the current approved skill
/// set.
#[derive(Debug, Clone)]
pub struct JudgeRequest<'a> {
    /// Short narrative of what happened in the turn — user
    /// input excerpt + tool calls made (names + brief input
    /// summary) + final reply excerpt. The caller is free to
    /// truncate; the judge prompt assumes the summary is
    /// already operator-budget-shaped (target ~500-800
    /// tokens).
    pub turn_summary: &'a str,

    /// Snapshots of every currently-approved skill. The judge
    /// scans this list for the dedup check; if none exist,
    /// pass an empty slice.
    pub existing_skills: &'a [ExistingSkillSnapshot],

    /// Provider-specific model identifier. Operator-configured
    /// in the TOML `[skills.auto_propose] judge_model` field
    /// (Task 5).
    pub model: &'a str,

    /// Maximum tokens the judge may emit. Defaults to a
    /// generous-but-bounded 800 if not overridden; long
    /// enough for a full SkillDraft + reasoning, short
    /// enough to keep cost predictable.
    pub max_tokens: u32,
}

/// What the judge actually proposes when it decides a turn is
/// skill-worthy. Mirrors the `LearnedSkill` shape from
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

/// The structured response shape the judge LLM must produce.
/// Returned as JSON; parsed by [`parse_judge_response`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JudgeResponse {
    /// Top-line verdict. If `false`, the turn pattern isn't
    /// general enough or recurring enough to warrant a skill.
    pub is_worth_proposing: bool,

    /// LLM's self-reported confidence, 0.0 – 1.0. The
    /// threshold-gate in Task 5 compares this against the
    /// operator-configured `auto_accept_confidence_threshold`.
    pub confidence: f32,

    /// The drafted skill, populated iff
    /// `is_worth_proposing == true`. (The parser doesn't
    /// enforce the cross-field constraint — a downstream
    /// consumer can decide whether to require non-None here.)
    pub proposed_skill: Option<SkillDraft>,

    /// Name of an existing skill this candidate semantically
    /// duplicates, if any. The Q4b dedup signal — populated
    /// when the LLM concludes the candidate is paraphrase of
    /// an existing skill even though title fuzzy-match
    /// didn't catch it.
    pub is_duplicate_of: Option<String>,

    /// Optional short rationale. Useful for the audit log
    /// (Task 6) and for operator inspection of the auto-
    /// proposer's behavior.
    #[serde(default)]
    pub reasoning: Option<String>,
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
        "You are a learned-skills judge for an AI personal assistant. \
Your job is to look at one completed turn and decide whether the \
pattern is worth saving as a reusable skill. A 'skill' is a short \
named procedure the assistant can invoke on future similar turns. \
\n\nReturn ONLY a single JSON object matching this schema:\n\
{\n\
  \"is_worth_proposing\": bool,\n\
  \"confidence\": float in [0.0, 1.0],\n\
  \"proposed_skill\": { \"name\": kebab-case string, \"trigger\": \
string, \"procedure\": markdown string } | null,\n\
  \"is_duplicate_of\": existing skill name string | null,\n\
  \"reasoning\": short string explaining the verdict\n\
}\n\n\
Criteria:\n\
- Worth proposing: the turn shows a reusable multi-step pattern that's \
likely to recur, not a one-off chat or trivial single-step action.\n\
- Confidence: how strongly the pattern reads as a 'real skill.' \
Reserve >= 0.85 for clear, well-defined, recurring patterns.\n\
- Duplicates: if the candidate is semantically the same as an existing \
skill (even with different wording), set is_duplicate_of to that \
skill's name and is_worth_proposing to false.\n\
- No prose outside the JSON. No markdown fences. JSON only.",
    )
}

/// Build the user prompt the judge sees for one candidate
/// turn. Embeds the turn summary and the existing-skills
/// catalog (for the dedup check).
pub fn build_user_prompt(request: &JudgeRequest<'_>) -> String {
    let mut s = String::new();
    s.push_str("## Completed turn\n\n");
    s.push_str(request.turn_summary);
    s.push_str("\n\n## Currently approved skills");
    if request.existing_skills.is_empty() {
        s.push_str("\n\n(none — the skill set is empty)\n\n");
    } else {
        s.push_str(" (for dedup check)\n\n");
        for skill in request.existing_skills {
            s.push_str(&format!(
                "- **{}** — trigger: {}\n  summary: {}\n",
                skill.name, skill.trigger, skill.procedure_summary
            ));
        }
        s.push('\n');
    }
    s.push_str("Respond with the JudgeResponse JSON now.");
    s
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

    // ----- Prompt-shape stability (golden) -----

    #[test]
    fn system_prompt_includes_the_schema_and_criteria() {
        let p = build_system_prompt();
        assert!(p.contains("is_worth_proposing"));
        assert!(p.contains("confidence"));
        assert!(p.contains("proposed_skill"));
        assert!(p.contains("is_duplicate_of"));
        assert!(p.contains("reasoning"));
        assert!(p.contains("JSON only"));
    }

    #[test]
    fn user_prompt_embeds_turn_summary_and_empty_skill_set() {
        let req = JudgeRequest {
            turn_summary: "User asked X; agent ran fs.read, web.fetch; replied Y.",
            existing_skills: &[],
            model: "claude-haiku-4-5",
            max_tokens: 800,
        };
        let p = build_user_prompt(&req);
        assert!(p.contains("User asked X"));
        assert!(p.contains("the skill set is empty"));
    }

    #[test]
    fn user_prompt_lists_existing_skills_for_dedup() {
        let existing = vec![
            ExistingSkillSnapshot {
                name: "research-topic".into(),
                trigger: "user asks 'research X'".into(),
                procedure_summary: "fs.read project notes, web.fetch ...".into(),
            },
            ExistingSkillSnapshot {
                name: "summarize-pdf".into(),
                trigger: "user shares a PDF".into(),
                procedure_summary: "fs.read PDF, extract sections ...".into(),
            },
        ];
        let req = JudgeRequest {
            turn_summary: "ignored",
            existing_skills: &existing,
            model: "m",
            max_tokens: 800,
        };
        let p = build_user_prompt(&req);
        assert!(p.contains("research-topic"));
        assert!(p.contains("summarize-pdf"));
        assert!(p.contains("dedup check"));
    }

    // ----- Parser tolerance -----

    #[test]
    fn parses_clean_json_response() {
        let raw = r#"{"is_worth_proposing":true,"confidence":0.91,
          "proposed_skill":{"name":"research-topic","trigger":"research X",
          "procedure":"step 1 ..."},"is_duplicate_of":null,
          "reasoning":"recurring multi-step pattern"}"#;
        let r = parse_judge_response(raw).expect("parse");
        assert!(r.is_worth_proposing);
        assert!((r.confidence - 0.91).abs() < 1e-6);
        assert_eq!(r.proposed_skill.as_ref().unwrap().name, "research-topic");
        assert!(r.is_duplicate_of.is_none());
    }

    #[test]
    fn parses_response_wrapped_in_markdown_fences() {
        let raw = r#"Sure, here you go:
```json
{"is_worth_proposing":false,"confidence":0.4,"proposed_skill":null,
 "is_duplicate_of":null,"reasoning":"one-off chat"}
```
that's my call."#;
        let r = parse_judge_response(raw).expect("parse");
        assert!(!r.is_worth_proposing);
        assert!(r.proposed_skill.is_none());
    }

    #[test]
    fn parses_response_with_short_preamble() {
        let raw = r#"Here's the verdict: {"is_worth_proposing":true,
          "confidence":0.88,"proposed_skill":{"name":"a","trigger":"b",
          "procedure":"c"},"is_duplicate_of":null}"#;
        let r = parse_judge_response(raw).expect("parse");
        assert!(r.is_worth_proposing);
    }

    #[test]
    fn parses_response_with_dup_set() {
        let raw = r#"{"is_worth_proposing":false,"confidence":0.95,
          "proposed_skill":null,"is_duplicate_of":"research-topic",
          "reasoning":"paraphrase of an existing skill"}"#;
        let r = parse_judge_response(raw).expect("parse");
        assert!(!r.is_worth_proposing);
        assert_eq!(r.is_duplicate_of.as_deref(), Some("research-topic"));
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
          "proposed_skill":null,"is_duplicate_of":null}"#;
        let err = parse_judge_response(raw).unwrap_err();
        matches!(err, JudgeError::ConfidenceOutOfRange(_));
    }

    #[test]
    fn rejects_confidence_below_zero() {
        let raw = r#"{"is_worth_proposing":true,"confidence":-0.1,
          "proposed_skill":null,"is_duplicate_of":null}"#;
        let err = parse_judge_response(raw).unwrap_err();
        matches!(err, JudgeError::ConfidenceOutOfRange(_));
    }

    #[test]
    fn extracts_balanced_object_in_presence_of_inner_braces() {
        // The procedure string contains `{` — extractor must
        // respect quoting, not just brace count.
        let raw = r#"{"is_worth_proposing":true,"confidence":0.9,
          "proposed_skill":{"name":"x","trigger":"y",
          "procedure":"call shell.exec with {arg: value}"},
          "is_duplicate_of":null}"#;
        let r = parse_judge_response(raw).expect("parse");
        assert!(r.is_worth_proposing);
        let proc_text = r.proposed_skill.unwrap().procedure;
        assert!(proc_text.contains("{arg: value}"));
    }

    // ----- Integration via scripted provider -----

    #[tokio::test]
    async fn judge_returns_worth_proposing_branch() {
        let provider = ScriptedProvider::new(vec![
            r#"{"is_worth_proposing":true,"confidence":0.92,
               "proposed_skill":{"name":"a","trigger":"t","procedure":"p"},
               "is_duplicate_of":null,"reasoning":"r"}"#,
        ]);
        let req = JudgeRequest {
            turn_summary: "summary",
            existing_skills: &[],
            model: "m",
            max_tokens: 800,
        };
        let cancel = CancellationToken::new();
        let resp = judge(provider, req, &cancel).await.expect("ok");
        assert!(resp.is_worth_proposing);
        assert!((resp.confidence - 0.92).abs() < 1e-6);
        assert_eq!(resp.proposed_skill.unwrap().name, "a");
    }

    #[tokio::test]
    async fn judge_returns_dup_branch() {
        let provider = ScriptedProvider::new(vec![
            r#"{"is_worth_proposing":false,"confidence":0.96,
               "proposed_skill":null,"is_duplicate_of":"existing-skill",
               "reasoning":"dup"}"#,
        ]);
        let req = JudgeRequest {
            turn_summary: "summary",
            existing_skills: &[],
            model: "m",
            max_tokens: 800,
        };
        let cancel = CancellationToken::new();
        let resp = judge(provider, req, &cancel).await.expect("ok");
        assert!(!resp.is_worth_proposing);
        assert_eq!(resp.is_duplicate_of.as_deref(), Some("existing-skill"));
    }

    #[tokio::test]
    async fn judge_returns_not_worth_branch() {
        let provider = ScriptedProvider::new(vec![
            r#"{"is_worth_proposing":false,"confidence":0.3,
               "proposed_skill":null,"is_duplicate_of":null,
               "reasoning":"one-off chat"}"#,
        ]);
        let req = JudgeRequest {
            turn_summary: "summary",
            existing_skills: &[],
            model: "m",
            max_tokens: 800,
        };
        let cancel = CancellationToken::new();
        let resp = judge(provider, req, &cancel).await.expect("ok");
        assert!(!resp.is_worth_proposing);
    }

    #[tokio::test]
    async fn judge_propagates_parse_failure_for_garbage_response() {
        let provider = ScriptedProvider::new(vec!["this is not json at all"]);
        let req = JudgeRequest {
            turn_summary: "summary",
            existing_skills: &[],
            model: "m",
            max_tokens: 800,
        };
        let cancel = CancellationToken::new();
        let err = judge(provider, req, &cancel).await.unwrap_err();
        matches!(err, JudgeError::ParseFailure { .. });
    }

    // ----- Serde round-trip -----

    #[test]
    fn judge_response_round_trips_through_serde_json() {
        let original = JudgeResponse {
            is_worth_proposing: true,
            confidence: 0.87,
            proposed_skill: Some(SkillDraft {
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
}
