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
//! **Scope (Chapter Verdict → #17b):** the judge checks the story's acceptance
//! criteria against the agent's *summary* AND — when a memory handle is wired
//! (`with_memory`) — a snapshot of the **recent memory the agent actually
//! wrote**. Grounding on the real artifact fixes the observed dogfood failure
//! where a genuinely-complete research story was rejected three times because
//! its summary was terse, even though the note was sitting in memory. The
//! summary alone could still be fabricated for artifact types the judge can't
//! see (files, test runs) — stack `gate_command` for those. The judge is a
//! quality layer, not a security gate, so it **fails open**: a judge LLM outage
//! logs and allows the completion rather than wedging the loop.

use std::sync::Arc;

use aivyx_core::CancellationToken;
use aivyx_llm::{LlmMessage, LlmProvider, LlmRequest, LlmStepEnd};
use aivyx_memory::{is_internal_topic, Memory, MemoryEntry};

const JUDGE_SYSTEM: &str =
    "You are a strict, fair acceptance reviewer for an autonomous agent's work. \
     You are given a task (its title + acceptance criteria), the agent's own \
     summary of what it did, and — when available — a snapshot of the recent \
     memory the agent actually wrote. Decide whether the acceptance criteria are \
     met. Treat the recent-memory snapshot as GROUND TRUTH: if it shows the work \
     was done (e.g. the required note exists with the required content), PASS \
     even when the agent's summary is terse or vague. Be skeptical only when \
     NEITHER the summary NOR the evidence shows the criteria are satisfied — then \
     FAIL. Reply with EXACTLY one line, starting with the single word PASS or \
     FAIL, then ' — ' and a brief reason. Example: 'FAIL — neither the summary \
     nor memory shows the required note was written.'";

const JUDGE_MAX_TOKENS: u32 = 256;

/// How many recent memory entries to show the judge as evidence, and the
/// per-entry body cap — enough to ground a story's artifact, bounded so the
/// judge prompt stays small.
const EVIDENCE_MAX_ENTRIES: usize = 12;
const EVIDENCE_BODY_CHARS: usize = 400;

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
    /// #17b — optional memory handle. When set, `verify` snapshots the recent
    /// memory the agent wrote and shows it to the judge as ground-truth
    /// evidence, so a terse summary over real work no longer false-fails.
    memory: Option<Arc<dyn Memory>>,
}

impl CompletionJudge {
    pub fn new(provider: Arc<dyn LlmProvider>, model: impl Into<String>) -> Self {
        CompletionJudge { provider, model: model.into(), memory: None }
    }

    /// Ground completion verdicts on the actual memory artifact (#17b): the
    /// judge is shown a snapshot of recent memory alongside the summary.
    pub fn with_memory(mut self, memory: Arc<dyn Memory>) -> Self {
        self.memory = Some(memory);
        self
    }

    /// Judge whether `summary` satisfies the story's `title` + `criteria`.
    /// Fails **open**: an LLM error → `passed: true` (a judge outage must not
    /// wedge the loop), with the error noted in `reason`.
    pub async fn verify(&self, title: &str, criteria: &str, summary: &str) -> Verdict {
        let evidence = self.gather_evidence().await;
        let evidence_block = match &evidence {
            Some(e) => format!(
                "\n\n## Recent memory the agent wrote (ground-truth evidence)\n{e}"
            ),
            None => String::new(),
        };
        let user = format!(
            "## Task\nTitle: {title}\nAcceptance criteria:\n{}\n\n## Agent's summary of what it did\n{}{evidence_block}\n\n\
             Are the acceptance criteria met (by the summary OR the evidence)? Reply PASS or FAIL with a brief reason.",
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

    /// Snapshot the most-recent memory entries across all (non-internal)
    /// topics as ground-truth evidence for the judge. Best-effort: no memory
    /// handle, a store error, or an empty store ⇒ `None` (the judge falls
    /// back to summary-only). Newest-first, deduped nothing, bounded.
    async fn gather_evidence(&self) -> Option<String> {
        let memory = self.memory.as_ref()?;
        let topics = memory.list_topics().await.ok()?;
        let mut entries: Vec<MemoryEntry> = Vec::new();
        for topic in topics {
            if is_internal_topic(&topic) {
                continue;
            }
            // A few newest per topic; the global sort+truncate below keeps the
            // overall most-recent set.
            if let Ok(mut es) = memory.get_recent(&topic, 3).await {
                entries.append(&mut es);
            }
        }
        if entries.is_empty() {
            return None;
        }
        // Most-recent first (global insertion order = seq; created_at breaks ties).
        entries.sort_by(|a, b| {
            b.created_at_secs
                .cmp(&a.created_at_secs)
                .then_with(|| b.seq.cmp(&a.seq))
        });
        entries.truncate(EVIDENCE_MAX_ENTRIES);
        let mut out = String::new();
        for e in &entries {
            let body: String = if e.body.chars().count() > EVIDENCE_BODY_CHARS {
                e.body.chars().take(EVIDENCE_BODY_CHARS).collect::<String>() + "…"
            } else {
                e.body.clone()
            };
            out.push_str(&format!("- [{}] {}\n", e.topic, body.replace('\n', " ")));
        }
        Some(out)
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
    use aivyx_llm::{LlmError, LlmStream, LlmStreamEvent, LlmUsage};
    use aivyx_memory::InMemoryMemory;
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// A provider that records the user prompt it was handed and replies with a
    /// fixed line — lets a test assert what the judge actually SAW.
    struct CapturingProvider {
        reply: String,
        seen: Arc<Mutex<String>>,
    }
    struct OneShot {
        text: Option<String>,
    }
    #[async_trait]
    impl LlmStream for OneShot {
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
    #[async_trait]
    impl LlmProvider for CapturingProvider {
        async fn chat_stream(
            &self,
            request: LlmRequest<'_>,
            _cancel: &CancellationToken,
        ) -> Result<Box<dyn LlmStream>, LlmError> {
            // Capture the (single) user message text.
            if let Some(LlmMessage::User { content }) = request.messages.first() {
                let text: String = content
                    .iter()
                    .filter_map(|b| match b {
                        aivyx_llm::ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect();
                *self.seen.lock().unwrap() = text;
            }
            Ok(Box::new(OneShot { text: Some(self.reply.clone()) }))
        }
    }

    #[tokio::test]
    async fn verify_grounds_on_recent_memory_evidence() {
        // The exact #17b failure: real work in memory, terse summary.
        let mem: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
        mem.put(
            "natural-therapies",
            "Melatonin (1–3 mg) shortens sleep onset; CBT-I reduces insomnia severity 30–50%.",
        )
        .await
        .unwrap();
        let seen = Arc::new(Mutex::new(String::new()));
        let judge = CompletionJudge::new(
            Arc::new(CapturingProvider {
                reply: "PASS — the memory snapshot shows the required note.".into(),
                seen: Arc::clone(&seen),
            }),
            "test-model",
        )
        .with_memory(Arc::clone(&mem));

        let v = judge
            .verify(
                "Find 2 natural approaches to better sleep",
                "memory topic 'natural-therapies' names 2 approaches with a rationale each",
                "added two approaches", // terse — would fail summary-only
            )
            .await;
        assert!(v.passed, "verdict: {v:?}");
        // The judge actually saw the memory artifact as evidence.
        let prompt = seen.lock().unwrap().clone();
        assert!(prompt.contains("ground-truth evidence"), "evidence block present");
        assert!(prompt.contains("natural-therapies"), "topic in evidence");
        assert!(prompt.contains("Melatonin"), "artifact body in evidence");
    }

    #[tokio::test]
    async fn verify_omits_evidence_block_without_memory() {
        let seen = Arc::new(Mutex::new(String::new()));
        let judge = CompletionJudge::new(
            Arc::new(CapturingProvider {
                reply: "PASS — fine.".into(),
                seen: Arc::clone(&seen),
            }),
            "test-model",
        ); // no with_memory
        let _ = judge.verify("t", "c", "s").await;
        let prompt = seen.lock().unwrap().clone();
        assert!(!prompt.contains("ground-truth evidence"), "no evidence block");
    }

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
