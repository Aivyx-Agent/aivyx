//! Chapter Verdict — an LLM **acceptance judge** for autonomous-loop story
//! completion (backlog Opp E).
//!
//! Without `[loop] gate_command`, a story is marked `Done` purely on the agent's
//! self-reported `loop.complete` — the same "trust a self-report" hole the digest
//! fix (Chapter Ledger) closed for reporting. This adds an opt-in
//! `[loop] verify_completion`: at completion time, an independent LLM judge reads
//! the story's **acceptance criteria** (its `body`) and the agent's **summary**
//! of what it did, and renders PASS/FAIL. A FAIL blocks the completion — the
//! story stays `Pending` (no backlog reopen needed) and the agent is told why.
//!
//! **Honest scope:** the judge checks the agent's *summary* against the criteria.
//! It catches vague, empty, or off-target completions and forces the agent to
//! articulate its work — but a convincingly-fabricated summary can still pass.
//! For artifact-grounded truth (did the tests pass, did the file change), stack
//! `gate_command` on top. The judge is a quality layer, not a security gate, so
//! it **fails open**: a judge LLM outage logs and allows the completion rather
//! than wedging the loop.

use std::sync::Arc;

use aivyx_core::CancellationToken;
use aivyx_llm::{LlmMessage, LlmProvider, LlmRequest, LlmStepEnd};

const JUDGE_SYSTEM: &str =
    "You are a strict, fair acceptance reviewer for an autonomous agent's work. \
     You are given a task (its title + acceptance criteria) and the agent's own \
     summary of what it did this run. Decide whether the summary concretely \
     satisfies the acceptance criteria. Be skeptical: FAIL a summary that is \
     vague, empty, generic, or does not actually address the criteria. Reply \
     with EXACTLY one line, starting with the single word PASS or FAIL, then \
     ' — ' and a brief reason. Example: 'FAIL — the summary restates the task \
     but gives no evidence the work was done.'";

const JUDGE_MAX_TOKENS: u32 = 256;

/// The judge's decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verdict {
    pub passed: bool,
    pub reason: String,
}

/// An LLM acceptance judge. Owns its own provider + model (the binary builds it
/// from the daemon's provider when `[loop] verify_completion` is on).
pub struct CompletionJudge {
    provider: Arc<dyn LlmProvider>,
    model: String,
}

impl CompletionJudge {
    pub fn new(provider: Arc<dyn LlmProvider>, model: impl Into<String>) -> Self {
        CompletionJudge { provider, model: model.into() }
    }

    /// Judge whether `summary` satisfies the story's `title` + `criteria`.
    /// Fails **open**: an LLM error → `passed: true` (a judge outage must not
    /// wedge the loop), with the error noted in `reason`.
    pub async fn verify(&self, title: &str, criteria: &str, summary: &str) -> Verdict {
        let user = format!(
            "## Task\nTitle: {title}\nAcceptance criteria:\n{}\n\n## Agent's summary of what it did\n{}\n\n\
             Does the summary satisfy the acceptance criteria? Reply PASS or FAIL with a brief reason.",
            if criteria.trim().is_empty() { "(none given beyond the title)" } else { criteria },
            if summary.trim().is_empty() { "(the agent provided no summary)" } else { summary },
        );
        let messages = vec![LlmMessage::user_text(user)];
        let request = LlmRequest {
            model: &self.model,
            system: Some(JUDGE_SYSTEM),
            messages: &messages,
            tools: &[],
            max_tokens: JUDGE_MAX_TOKENS,
            temperature: Some(0.0),
        };
        let cancel = CancellationToken::new();
        let text = match self.provider.chat_stream(request, &cancel).await {
            Ok(mut stream) => {
                while let Ok(Some(_)) = stream.next_event().await {}
                match stream.finish().await {
                    Ok(LlmStepEnd::FinalMessage { text, .. }) => text,
                    Ok(_) => return fail_open("judge returned no final message"),
                    Err(e) => return fail_open(&format!("judge LLM error: {e}")),
                }
            }
            Err(e) => return fail_open(&format!("judge LLM error: {e}")),
        };
        parse_verdict(&text)
    }
}

fn fail_open(reason: &str) -> Verdict {
    eprintln!("aivyx loop: completion judge unavailable — allowing ({reason})");
    Verdict { passed: true, reason: format!("not verified ({reason})") }
}

/// Parse the judge's reply. The first non-empty line decides: a leading `FAIL`
/// → reject; a leading `PASS` → accept; anything else **fails open** (accept)
/// so a malformed verdict never blocks the loop. Pure + tested.
pub fn parse_verdict(text: &str) -> Verdict {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let upper = line.to_uppercase();
    if upper.starts_with("FAIL") {
        let reason = line.trim_start_matches(|c: char| c.is_alphabetic())
            .trim_start_matches([' ', '—', '-', ':'])
            .trim();
        Verdict {
            passed: false,
            reason: if reason.is_empty() { line.to_string() } else { reason.to_string() },
        }
    } else if upper.starts_with("PASS") {
        Verdict { passed: true, reason: line.to_string() }
    } else {
        // Ambiguous → fail open (don't block on a confused judge).
        Verdict { passed: true, reason: format!("unparseable verdict: {line}") }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_fail_with_reason() {
        let v = parse_verdict("FAIL — the summary gives no evidence the file changed.");
        assert!(!v.passed);
        assert_eq!(v.reason, "the summary gives no evidence the file changed.");
    }

    #[test]
    fn parses_pass() {
        assert!(parse_verdict("PASS — looks complete, criteria met.").passed);
        assert!(parse_verdict("pass").passed);
    }

    #[test]
    fn fails_only_on_a_leading_fail() {
        // "fail" in the reason of a PASS must not flip it.
        assert!(parse_verdict("PASS — no failures found").passed);
        // leading FAIL (any case) rejects.
        assert!(!parse_verdict("fail: nope").passed);
    }

    #[test]
    fn ambiguous_fails_open() {
        // A judge that didn't follow the format must not block the loop.
        assert!(parse_verdict("I think it is probably fine").passed);
        assert!(parse_verdict("").passed);
    }
}
