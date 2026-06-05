//! Phase 178 — LLM-judged correction classification.
//!
//! Closes the Phase 172 honest debt: the structural correction
//! signal ("the operator came back within 60 s of a completed
//! turn") counts a genuine rework, a "thanks, perfect," and an
//! unrelated new request all the same. This judges each
//! detected correction's **follow-up message** into a 3-way
//! verdict so only genuine reworks feed the correction ledger.
//!
//! Mirrors Phase 91's `recall_judgment`: a trait + an
//! `aivyx_llm::LlmProvider`-backed adapter, one batched call per
//! reflection cycle, a parser-tolerant deterministic output
//! shape, and best-effort failure (parse/LLM error → `None`, so
//! the event keeps the structural signal rather than dropping).

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use aivyx_core::CancellationToken;
use aivyx_llm::{LlmMessage, LlmProvider, LlmRequest, LlmStepEnd};

/// The 3-way verdict on an operator's immediate follow-up after
/// a completed turn. Only [`CorrectionJudgment::Rework`] folds
/// into the correction ledger.
///
/// Stable snake_case labels for the wire + the parser:
/// `"rework"`, `"praise"`, `"unrelated"`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum CorrectionJudgment {
    /// A genuine correction: the operator is reworking / fixing
    /// / redirecting the previous answer.
    Rework,
    /// Praise / acknowledgment ("thanks", "perfect") — the
    /// re-engagement was NOT a correction.
    Praise,
    /// An unrelated new request — the operator moved on; the
    /// rapid follow-up is coincidental, not a correction.
    Unrelated,
}

/// One correction the judge classifies: the follow-up message
/// plus the corrected turn's topics for context.
#[derive(Debug, Clone, PartialEq)]
pub struct CorrectionJudgeInput {
    /// The corrected turn's distinct topics (context the model
    /// uses to decide if the follow-up reworks *this* subject).
    pub topics: Vec<String>,
    /// The operator's immediate follow-up message (captured on
    /// the follow-up turn's `RecallEvent.query_text`).
    pub follow_up_query: String,
}

/// The per-correction LLM judging seam. The reflection-cron pass
/// calls this once per cycle with the batch of judgeable
/// corrections; the adapter returns one verdict per input in
/// order (`None` = couldn't judge → keep structural).
#[async_trait]
pub trait CorrectionJudge: Send + Sync {
    async fn judge(
        &self,
        inputs: &[CorrectionJudgeInput],
    ) -> Vec<Option<CorrectionJudgment>>;
}

/// Hard cap on the LLM completion. One small label list.
const JUDGE_MAX_TOKENS: u32 = 1024;

/// Fixed system prompt. Conservative: the model classifies,
/// never generates instructions. Emphasises the 3-way contract
/// and the deterministic one-label-per-line output the parser
/// expects.
const JUDGE_SYSTEM_PROMPT: &str =
    "You are classifying an operator's immediate follow-up \
message to an AI assistant. For each numbered case, decide \
whether the follow-up is correcting/reworking the assistant's \
previous answer, or not. Output EXACTLY ONE label on its own \
line, in order: `rework` (the operator is fixing, correcting, \
redirecting, or expressing dissatisfaction with the previous \
answer), `praise` (thanks / acknowledgment / approval — not a \
correction), or `unrelated` (a new request on a different \
topic — not a correction). Output ONLY the labels, one per \
line, no commentary, no numbering.";

/// Production `CorrectionJudge` delegating to the existing
/// `aivyx_llm::LlmProvider` (same provider + model the agent
/// uses). Mirrors `LlmRecallJudge`.
pub struct LlmCorrectionJudge {
    provider: Arc<dyn LlmProvider>,
    model: String,
}

impl LlmCorrectionJudge {
    pub fn new(provider: Arc<dyn LlmProvider>, model: String) -> Self {
        Self { provider, model }
    }

    /// Compose the batch user message — numbered (topics,
    /// follow-up) cases the model classifies in order. Public
    /// for unit tests of the prompt shape.
    pub fn compose_prompt(inputs: &[CorrectionJudgeInput]) -> String {
        let mut s = String::new();
        for (i, input) in inputs.iter().enumerate() {
            s.push_str(&format!(
                "Case {}:\n  previous-answer topics: {}\n  \
                 operator follow-up: {}\n\n",
                i + 1,
                input.topics.join(", "),
                input.follow_up_query,
            ));
        }
        s
    }

    /// Parse the LLM's response into one `Option<CorrectionJudgment>`
    /// per input line. Tolerates whitespace, casing, trailing
    /// punctuation; `None` for unparseable lines; pads/truncates
    /// to `expected`. Public for parser unit tests.
    pub fn parse_response(
        text: &str,
        expected: usize,
    ) -> Vec<Option<CorrectionJudgment>> {
        let mut out: Vec<Option<CorrectionJudgment>> =
            Vec::with_capacity(expected);
        for line in text.lines() {
            let token = line.trim().to_ascii_lowercase();
            let token =
                token.trim_matches(|c: char| !c.is_alphanumeric());
            let parsed = match token {
                "rework" => Some(CorrectionJudgment::Rework),
                "praise" => Some(CorrectionJudgment::Praise),
                "unrelated" => Some(CorrectionJudgment::Unrelated),
                "" => continue,
                _ => None,
            };
            out.push(parsed);
            if out.len() == expected {
                break;
            }
        }
        while out.len() < expected {
            out.push(None);
        }
        out
    }
}

#[async_trait]
impl CorrectionJudge for LlmCorrectionJudge {
    async fn judge(
        &self,
        inputs: &[CorrectionJudgeInput],
    ) -> Vec<Option<CorrectionJudgment>> {
        if inputs.is_empty() {
            return Vec::new();
        }
        let user = Self::compose_prompt(inputs);
        let messages = vec![LlmMessage::user_text(user)];
        let request = LlmRequest {
            model: &self.model,
            system: Some(JUDGE_SYSTEM_PROMPT),
            messages: &messages,
            tools: &[],
            max_tokens: JUDGE_MAX_TOKENS,
            temperature: Some(0.0),
        };
        let cancel = CancellationToken::new();
        let mut stream =
            match self.provider.chat_stream(request, &cancel).await {
                Ok(s) => s,
                Err(_) => return vec![None; inputs.len()],
            };
        while let Ok(Some(_)) = stream.next_event().await {}
        let step = match stream.finish().await {
            Ok(s) => s,
            Err(_) => return vec![None; inputs.len()],
        };
        match step {
            LlmStepEnd::FinalMessage { text, .. } => {
                Self::parse_response(&text, inputs.len())
            }
            LlmStepEnd::ToolCalls { .. } => vec![None; inputs.len()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn judgment_serde_round_trips_each_variant() {
        for (variant, expected) in [
            (CorrectionJudgment::Rework, "\"rework\""),
            (CorrectionJudgment::Praise, "\"praise\""),
            (CorrectionJudgment::Unrelated, "\"unrelated\""),
        ] {
            let j = serde_json::to_string(&variant).unwrap();
            assert_eq!(j, expected);
            let back: CorrectionJudgment =
                serde_json::from_str(&j).unwrap();
            assert_eq!(back, variant);
        }
    }

    #[test]
    fn parse_response_maps_clean_labels_in_order() {
        let out = LlmCorrectionJudge::parse_response(
            "rework\npraise\nunrelated\n",
            3,
        );
        assert_eq!(
            out,
            vec![
                Some(CorrectionJudgment::Rework),
                Some(CorrectionJudgment::Praise),
                Some(CorrectionJudgment::Unrelated),
            ]
        );
    }

    #[test]
    fn parse_response_tolerates_casing_and_punctuation() {
        let out = LlmCorrectionJudge::parse_response(
            "  REWORK.\n- Praise!\n\n  unrelated  \n",
            3,
        );
        assert_eq!(
            out,
            vec![
                Some(CorrectionJudgment::Rework),
                Some(CorrectionJudgment::Praise),
                Some(CorrectionJudgment::Unrelated),
            ]
        );
    }

    #[test]
    fn parse_response_pads_and_marks_unparseable() {
        // Two lines, one garbage; expected 3 → pad to 3.
        let out = LlmCorrectionJudge::parse_response(
            "rework\nbananas\n",
            3,
        );
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], Some(CorrectionJudgment::Rework));
        assert_eq!(out[1], None); // unparseable
        assert_eq!(out[2], None); // padded
    }

    #[test]
    fn parse_response_truncates_excess() {
        let out = LlmCorrectionJudge::parse_response(
            "rework\npraise\nunrelated\nrework\n",
            2,
        );
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn compose_prompt_numbers_cases_with_topics_and_followup() {
        let inputs = vec![
            CorrectionJudgeInput {
                topics: vec!["auth".into(), "jwt".into()],
                follow_up_query: "no, I meant the refresh path".into(),
            },
            CorrectionJudgeInput {
                topics: vec!["css".into()],
                follow_up_query: "thanks!".into(),
            },
        ];
        let p = LlmCorrectionJudge::compose_prompt(&inputs);
        assert!(p.contains("Case 1:"));
        assert!(p.contains("auth, jwt"));
        assert!(p.contains("no, I meant the refresh path"));
        assert!(p.contains("Case 2:"));
        assert!(p.contains("thanks!"));
    }

    /// A deterministic fake judge for the Phase 178 Task 4 fold
    /// tests + here: classifies by a keyword in the follow-up.
    struct KeywordJudge;
    #[async_trait]
    impl CorrectionJudge for KeywordJudge {
        async fn judge(
            &self,
            inputs: &[CorrectionJudgeInput],
        ) -> Vec<Option<CorrectionJudgment>> {
            inputs
                .iter()
                .map(|i| {
                    let q = i.follow_up_query.to_ascii_lowercase();
                    if q.contains("thank") {
                        Some(CorrectionJudgment::Praise)
                    } else if q.contains("no") || q.contains("wrong") {
                        Some(CorrectionJudgment::Rework)
                    } else {
                        Some(CorrectionJudgment::Unrelated)
                    }
                })
                .collect()
        }
    }

    #[tokio::test]
    async fn fake_judge_classifies_batch_in_order() {
        let inputs = vec![
            CorrectionJudgeInput {
                topics: vec!["a".into()],
                follow_up_query: "no that's wrong".into(),
            },
            CorrectionJudgeInput {
                topics: vec!["b".into()],
                follow_up_query: "thanks, perfect".into(),
            },
            CorrectionJudgeInput {
                topics: vec!["c".into()],
                follow_up_query: "deploy the changes".into(),
            },
        ];
        let out = KeywordJudge.judge(&inputs).await;
        assert_eq!(
            out,
            vec![
                Some(CorrectionJudgment::Rework),
                Some(CorrectionJudgment::Praise),
                Some(CorrectionJudgment::Unrelated),
            ]
        );
    }

    #[tokio::test]
    async fn empty_batch_returns_empty() {
        assert!(KeywordJudge.judge(&[]).await.is_empty());
    }
}
