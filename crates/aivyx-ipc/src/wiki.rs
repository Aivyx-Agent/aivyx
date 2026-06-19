//! Knowledge-wiki page types (Chapter Codex) — the wasm-clean DTOs
//! shared by the daemon, the IPC protocol, and the Studio Wiki view.
//!
//! A [`WikiPage`] is the synthesized, consolidated view of one memory
//! **topic**: an LLM-written summary of that topic's entries, the topic's
//! co-occurrence **backlinks**, and the bookkeeping needed to regenerate
//! it incrementally. Pages are **derived** — the source of truth is
//! always memory + the co-occurrence ledger; a page is a cache of
//! "what is known about this topic," never a second place facts live.
//!
//! The persistent store + the synthesizer that fills these in live in
//! `aivyx-channel` (they need encrypted storage + the LLM provider);
//! these types stay here so they cross the IPC boundary and compile to
//! wasm for the Studio without dragging native deps.

use serde::{Deserialize, Serialize};

/// One co-occurrence backlink from a page to a related topic.
///
/// `affinity` is the decayed graph-walk score to the related topic (the
/// same number Loom's `neighbors_within` returns); `hops` is the path
/// length (1 = a direct sibling). The Studio renders these as clickable
/// links that navigate to the linked topic's page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WikiBacklink {
    pub topic: String,
    pub affinity: f32,
    pub hops: u32,
}

/// A synthesized knowledge-wiki page for one canonical memory topic.
///
/// `source_seqs` records which memory entries (by `seq`) the summary was
/// consolidated from; `source_fingerprint` is a cheap hash of those plus
/// the entry count, so regeneration can be skipped when a topic hasn't
/// changed since its page was built (incremental synthesis). `updated_at`
/// is the wall-clock second the page was last (re)generated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WikiPage {
    /// Canonical topic (the page's key).
    pub topic: String,
    /// LLM-consolidated summary of the topic's memory entries.
    pub summary: String,
    /// `seq`s of the memory entries this summary was built from.
    pub source_seqs: Vec<u64>,
    /// Number of entries the page consolidates (== `source_seqs.len()`,
    /// carried explicitly for cheap list rendering).
    pub entry_count: u32,
    /// Co-occurrence backlinks to related topics, affinity-descending.
    pub backlinks: Vec<WikiBacklink>,
    /// Wall-clock second the page was last (re)generated.
    pub updated_at: u64,
    /// Incremental-regeneration guard — a stable hash of the contributing
    /// entry `seq`s + count. Equal fingerprints ⇒ the topic is unchanged
    /// ⇒ regeneration can be skipped.
    pub source_fingerprint: u64,
}

/// A compact page row for the Studio Wiki list view — enough to render
/// the index without shipping every full summary + backlink set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WikiPageSummary {
    pub topic: String,
    /// A short snippet of the summary's opening, for the list row.
    pub snippet: String,
    pub entry_count: u32,
    pub updated_at: u64,
}

impl WikiPage {
    /// Compute the incremental-regeneration fingerprint for a set of
    /// entry `seq`s. Order-independent (the caller may pass seqs in any
    /// order) and deterministic — a small FNV-1a-style fold over the
    /// sorted seqs plus the count, so the same entry set always yields
    /// the same fingerprint and any add/remove changes it.
    pub fn fingerprint(seqs: &[u64]) -> u64 {
        let mut sorted: Vec<u64> = seqs.to_vec();
        sorted.sort_unstable();
        // FNV-1a 64-bit over the count then each seq's bytes.
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        let mut fold = |v: u64| {
            for b in v.to_le_bytes() {
                hash ^= b as u64;
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        fold(sorted.len() as u64);
        for s in sorted {
            fold(s);
        }
        hash
    }

    /// A short snippet of the summary for list rendering — the first
    /// `max_chars` Unicode chars, with an ellipsis when truncated.
    pub fn snippet(&self, max_chars: usize) -> String {
        if self.summary.chars().count() <= max_chars {
            self.summary.clone()
        } else {
            let head: String = self.summary.chars().take(max_chars).collect();
            format!("{head}…")
        }
    }

    /// Project to the compact list row.
    pub fn to_summary(&self, snippet_chars: usize) -> WikiPageSummary {
        WikiPageSummary {
            topic: self.topic.clone(),
            snippet: self.snippet(snippet_chars),
            entry_count: self.entry_count,
            updated_at: self.updated_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_order_independent_and_change_sensitive() {
        let a = WikiPage::fingerprint(&[3, 1, 2]);
        let b = WikiPage::fingerprint(&[1, 2, 3]);
        assert_eq!(a, b, "order must not matter");
        assert_ne!(a, WikiPage::fingerprint(&[1, 2]), "removing a seq changes it");
        assert_ne!(a, WikiPage::fingerprint(&[1, 2, 3, 4]), "adding a seq changes it");
        assert_ne!(
            WikiPage::fingerprint(&[]),
            WikiPage::fingerprint(&[0]),
            "empty vs one seq differ"
        );
    }

    fn page(summary: &str) -> WikiPage {
        WikiPage {
            topic: "deploy".into(),
            summary: summary.into(),
            source_seqs: vec![1, 2],
            entry_count: 2,
            backlinks: vec![WikiBacklink { topic: "ci".into(), affinity: 0.8, hops: 1 }],
            updated_at: 100,
            source_fingerprint: WikiPage::fingerprint(&[1, 2]),
        }
    }

    #[test]
    fn snippet_truncates_with_ellipsis() {
        let p = page("the quick brown fox jumps");
        assert_eq!(p.snippet(100), "the quick brown fox jumps");
        let s = p.snippet(9);
        assert_eq!(s, "the quick…");
    }

    #[test]
    fn to_summary_projects_fields() {
        let p = page("hello world summary");
        let s = p.to_summary(5);
        assert_eq!(s.topic, "deploy");
        assert_eq!(s.entry_count, 2);
        assert_eq!(s.updated_at, 100);
        assert_eq!(s.snippet, "hello…");
    }

    #[test]
    fn page_round_trips_through_json() {
        let p = page("round trip");
        let bytes = serde_json::to_vec(&p).unwrap();
        let back: WikiPage = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(p, back);
    }
}
