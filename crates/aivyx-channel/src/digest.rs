//! Chapter Ledger — the **deterministic** weekly digest.
//!
//! Backlog #6 (reopened): the LLM-generated `weekly-digest` cron confabulates
//! profile-themed history (a "Flutter revenue dashboard" the chef-operator
//! never built), even after Chapter Plumb hardened the prompt to read-first /
//! never-invent — because a small local model asked to *summarize* sparse real
//! memory fills the gap with plausible fiction. Prompt-hardening can't stop it.
//!
//! The fix is to take the LLM out of the content path entirely: this module
//! **assembles the digest from the real memory substrate** (entries written
//! since the last digest, plus the live pending-proposal count) and renders it
//! deterministically. Confabulation is then *structurally impossible* — every
//! line traces to a real `MemoryEntry`. If there is nothing new, it says so
//! plainly. This matches the "verifiable / never fabricate" charter the digest
//! exists to serve.

use std::sync::Arc;

use aivyx_memory::Memory;

use crate::persona_proposal::{PersistentPersonaProposalLog, ProposalStatusFilter};

/// Topics excluded from the digest: the pruned-context archives (machine
/// bookkeeping, not "activity") and the digest topic itself (so a digest never
/// summarizes its own prior entries — the exact loop that let the old LLM
/// digest regenerate its own fabrications).
const EXCLUDE_PREFIX: &str = "context:pruned:";
/// The memory topic the digest is written to.
pub const DIGEST_TOPIC: &str = "weekly-digest";
/// Per-entry snippet cap so one long memory can't blow up the digest.
const SNIPPET_CHARS: usize = 140;
/// Max entries shown per topic.
const MAX_PER_TOPIC: usize = 5;

/// Builds the deterministic weekly digest from the memory substrate.
pub struct WeeklyDigestBuilder {
    memory: Arc<dyn Memory>,
    proposals: Option<Arc<PersistentPersonaProposalLog>>,
}

impl WeeklyDigestBuilder {
    /// A builder over `memory`. Attach the proposal log with
    /// [`with_proposals`](Self::with_proposals) to include the pending count.
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        WeeklyDigestBuilder { memory, proposals: None }
    }

    pub fn with_proposals(
        mut self,
        proposals: Arc<PersistentPersonaProposalLog>,
    ) -> Self {
        self.proposals = Some(proposals);
        self
    }

    /// Render the digest text from memory written since `since_secs` (wall-clock
    /// seconds) + the live pending-proposal count. Pure: gathers + renders, does
    /// not persist. `now_secs` stamps the heading with a REAL date (never a
    /// model-hallucinated one).
    pub async fn render(&self, since_secs: u64, now_secs: u64) -> String {
        let topics = self.memory.list_topics().await.unwrap_or_default();
        let mut sections: Vec<(String, Vec<String>)> = Vec::new();
        for topic in topics {
            if topic.starts_with(EXCLUDE_PREFIX) || topic == DIGEST_TOPIC {
                continue;
            }
            let entries =
                self.memory.get_recent(&topic, 50).await.unwrap_or_default();
            let recent: Vec<String> = entries
                .iter()
                .filter(|e| e.created_at_secs >= since_secs)
                .take(MAX_PER_TOPIC)
                .map(|e| snippet(&e.body))
                .collect();
            if !recent.is_empty() {
                sections.push((topic, recent));
            }
        }
        sections.sort_by(|a, b| a.0.cmp(&b.0));

        let pending = match &self.proposals {
            Some(p) => p.list(ProposalStatusFilter::Pending).len(),
            None => 0,
        };

        render_markdown(&sections, pending, since_secs, now_secs)
    }

    /// Render the digest and persist it to [`DIGEST_TOPIC`], returning the text
    /// (for notification). The whole content path is deterministic.
    pub async fn run(&self, since_secs: u64, now_secs: u64) -> String {
        let text = self.render(since_secs, now_secs).await;
        if let Err(e) = self.memory.put(DIGEST_TOPIC, &text).await {
            eprintln!("aivyx digest: failed to persist weekly-digest: {e}");
        }
        text
    }
}

/// First non-empty line of `body`, whitespace-collapsed, capped.
fn snippet(body: &str) -> String {
    let line = body.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let collapsed = line.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > SNIPPET_CHARS {
        let cut: String = collapsed.chars().take(SNIPPET_CHARS).collect();
        format!("{cut}…")
    } else {
        collapsed
    }
}

/// YYYY-MM-DD for a wall-clock second count (UTC). Real date, deterministic.
fn ymd(secs: u64) -> String {
    chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "—".to_string())
}

fn render_markdown(
    sections: &[(String, Vec<String>)],
    pending: usize,
    since_secs: u64,
    now_secs: u64,
) -> String {
    let mut out = format!(
        "Weekly digest ({} → {})\n",
        ymd(since_secs),
        ymd(now_secs)
    );

    if sections.is_empty() && pending == 0 {
        out.push_str(
            "\nNo new activity to summarize since the last digest. \
             (Assembled from memory — nothing was recorded, so there is \
             nothing to report.)\n",
        );
        return out;
    }

    if sections.is_empty() {
        out.push_str("\nNo new memories were recorded this period.\n");
    } else {
        out.push_str("\nNew in memory this period:\n");
        for (topic, items) in sections {
            out.push_str(&format!("\n**{topic}**\n"));
            for item in items {
                out.push_str(&format!("- {item}\n"));
            }
        }
    }

    out.push_str(&format!(
        "\n_Pending proposals awaiting your review: {pending}._\n"
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_memory::InMemoryMemory;

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }

    #[tokio::test]
    async fn empty_memory_says_nothing_to_report_never_invents() {
        let mem: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
        let b = WeeklyDigestBuilder::new(mem);
        let text = b.render(0, now()).await;
        assert!(
            text.contains("No new activity to summarize"),
            "empty digest must say nothing, got: {text}"
        );
        // The hallmark confabulations must never appear out of thin air.
        assert!(!text.to_lowercase().contains("flutter"));
        assert!(!text.to_lowercase().contains("ashwagandha"));
    }

    #[tokio::test]
    async fn digest_reports_only_real_memories_written_since() {
        let mem: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
        mem.put("coffee", "Tried an Ethiopian pour-over, liked it").await.unwrap();
        mem.put("ppl/medical", "Booked the class-2 medical for next week").await.unwrap();
        // A pruned-context archive + the digest's own topic must be excluded.
        mem.put("context:pruned:abc", "old conversation").await.unwrap();
        mem.put(DIGEST_TOPIC, "a prior digest").await.unwrap();

        let b = WeeklyDigestBuilder::new(mem);
        let text = b.render(0, now()).await;

        assert!(text.contains("**coffee**"), "real topic present: {text}");
        assert!(text.contains("Ethiopian pour-over"));
        assert!(text.contains("**ppl/medical**"));
        assert!(text.contains("class-2 medical"));
        // Excluded topics never appear.
        assert!(!text.contains("context:pruned"));
        assert!(!text.contains("a prior digest"));
    }

    #[tokio::test]
    async fn entries_older_than_since_are_excluded() {
        let mem: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
        mem.put("coffee", "an old note").await.unwrap();
        // since = far future → nothing qualifies.
        let b = WeeklyDigestBuilder::new(mem);
        let text = b.render(u64::MAX, now()).await;
        assert!(text.contains("No new activity to summarize"), "got: {text}");
    }

    #[test]
    fn snippet_collapses_and_caps() {
        assert_eq!(snippet("  hello   world \n more"), "hello world");
        let long = "x".repeat(300);
        let s = snippet(&long);
        assert!(s.ends_with('…') && s.chars().count() == SNIPPET_CHARS + 1);
    }
}
