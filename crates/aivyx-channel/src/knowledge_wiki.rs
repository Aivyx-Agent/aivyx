//! Chapter Codex (CX.1) — the persistent knowledge-wiki page store.
//!
//! One encrypted row per **canonical topic** in
//! [`aivyx_storage::KeyDomain::KnowledgeWiki`], holding the synthesized
//! [`WikiPage`] for that topic. This module owns only the *storage* of
//! pages (CRUD + an incremental-regeneration check); the synthesizer that
//! *fills* a page from memory + the LLM lands in CX.2, and the generation
//! cadence in CX.3.
//!
//! Pages are **derived** — the source of truth is always memory + the
//! co-occurrence ledger. The store is a cache: a missing or stale page
//! costs only a (re)synthesis, never correctness, and a corrupt row
//! degrades only the codex (the Studio Wiki view + the opt-in recall
//! unit), never memory or recall.
//!
//! Topics are canonicalized on the way in (the Phase 89 / Loom
//! canonicalizer) so a page keys identically to the memory entries it
//! summarizes — `deploy` / `deploying` / `Deploy` share one page.

use aivyx_memory::canonicalize_topic;
use aivyx_storage::DomainHandle;

pub use aivyx_ipc::wiki::{WikiBacklink, WikiPage, WikiPageSummary};

/// Snippet length (Unicode chars) for the Studio list rows.
pub const WIKI_SNIPPET_CHARS: usize = 160;

/// Errors from the wiki store. Mirrors the co-occurrence ledger's shape:
/// a storage-layer failure or an (de)serialization failure, both carried
/// as detail strings so the caller can log and skip.
#[derive(Debug, thiserror::Error)]
pub enum WikiStoreError {
    #[error("knowledge-wiki storage error: {0}")]
    Storage(String),
    #[error("knowledge-wiki encode/decode error: {0}")]
    Encode(String),
}

/// Persistent store for synthesized [`WikiPage`]s, keyed by canonical
/// topic in [`aivyx_storage::KeyDomain::KnowledgeWiki`].
pub struct PersistentWikiStore {
    storage: DomainHandle,
}

impl PersistentWikiStore {
    pub fn new(storage: DomainHandle) -> Self {
        Self { storage }
    }

    /// Storage key for a topic: its canonical form, as bytes.
    fn key(topic: &str) -> Vec<u8> {
        canonicalize_topic(topic).into_bytes()
    }

    /// Upsert a page. The `topic` field is canonicalized so the stored
    /// page keys consistently regardless of the caller's casing/inflection.
    pub async fn put_page(&self, page: &WikiPage) -> Result<(), WikiStoreError> {
        let canonical = canonicalize_topic(&page.topic);
        let stored = WikiPage { topic: canonical.clone(), ..page.clone() };
        let bytes = serde_json::to_vec(&stored)
            .map_err(|e| WikiStoreError::Encode(e.to_string()))?;
        self.storage
            .put(canonical.as_bytes(), &bytes)
            .await
            .map_err(|e| WikiStoreError::Storage(e.to_string()))
    }

    /// Fetch a topic's page, if one has been synthesized.
    pub async fn get_page(&self, topic: &str) -> Result<Option<WikiPage>, WikiStoreError> {
        let raw = self
            .storage
            .get(&Self::key(topic))
            .await
            .map_err(|e| WikiStoreError::Storage(e.to_string()))?;
        match raw {
            Some(bytes) => {
                let page = serde_json::from_slice(&bytes)
                    .map_err(|e| WikiStoreError::Encode(e.to_string()))?;
                Ok(Some(page))
            }
            None => Ok(None),
        }
    }

    /// Delete a topic's page (e.g. after the topic is forgotten). Idempotent.
    pub async fn delete_page(&self, topic: &str) -> Result<(), WikiStoreError> {
        self.storage
            .delete(&Self::key(topic))
            .await
            .map_err(|e| WikiStoreError::Storage(e.to_string()))
    }

    /// All pages, full. Corrupt rows are skipped (best-effort) rather than
    /// failing the whole read — one bad page must not blank the codex.
    pub async fn all_pages(&self) -> Result<Vec<WikiPage>, WikiStoreError> {
        let rows = self
            .storage
            .scan_prefix(&[])
            .await
            .map_err(|e| WikiStoreError::Storage(e.to_string()))?;
        let mut out: Vec<WikiPage> = rows
            .iter()
            .filter_map(|(_k, v)| serde_json::from_slice(v).ok())
            .collect();
        // Stable, operator-friendly order: most-recently-updated first,
        // then topic ascending for ties.
        out.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.topic.cmp(&b.topic))
        });
        Ok(out)
    }

    /// Compact list rows for the Studio Wiki index, most-recent first.
    pub async fn list_summaries(&self) -> Result<Vec<WikiPageSummary>, WikiStoreError> {
        Ok(self
            .all_pages()
            .await?
            .iter()
            .map(|p| p.to_summary(WIKI_SNIPPET_CHARS))
            .collect())
    }

    /// Incremental-regeneration check: does the topic need a (re)synthesis
    /// given the current set of contributing entry `seq`s? `true` when no
    /// page exists yet or the stored `source_fingerprint` differs from the
    /// fingerprint of `current_seqs`. A storage/decode error is treated as
    /// "needs regen" (fail toward freshness, never panic).
    pub async fn needs_regen(&self, topic: &str, current_seqs: &[u64]) -> bool {
        let want = WikiPage::fingerprint(current_seqs);
        match self.get_page(topic).await {
            Ok(Some(page)) => page.source_fingerprint != want,
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    async fn store() -> PersistentWikiStore {
        use aivyx_crypto::MasterKey;
        use aivyx_storage::{KeyDomain, RedbStorage, Storage, StorageConfig};
        let base = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let dir = std::path::PathBuf::from(base)
            .join(format!("aivyx-wiki-store-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let s: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("store.redb")),
            MasterKey::from_raw([67u8; 32]),
        )
        .await
        .unwrap();
        PersistentWikiStore::new(s.domain(KeyDomain::KnowledgeWiki))
    }

    fn page(topic: &str, summary: &str, seqs: Vec<u64>, updated: u64) -> WikiPage {
        WikiPage {
            topic: topic.into(),
            summary: summary.into(),
            source_seqs: seqs.clone(),
            entry_count: seqs.len() as u32,
            backlinks: vec![],
            updated_at: updated,
            source_fingerprint: WikiPage::fingerprint(&seqs),
        }
    }

    #[tokio::test]
    async fn put_get_round_trips() {
        let s = store().await;
        assert!(s.get_page("deploy").await.unwrap().is_none());
        let p = page("deploy", "the deploy summary", vec![1, 2, 3], 100);
        s.put_page(&p).await.unwrap();
        let got = s.get_page("deploy").await.unwrap().expect("page present");
        assert_eq!(got.summary, "the deploy summary");
        assert_eq!(got.entry_count, 3);
    }

    #[tokio::test]
    async fn topic_is_canonicalized_on_key() {
        let s = store().await;
        s.put_page(&page("Deploying", "x", vec![1], 1)).await.unwrap();
        // A different inflection of the same canonical topic finds it.
        assert!(s.get_page("deploy").await.unwrap().is_some());
        let got = s.get_page("Deploys").await.unwrap().expect("canonical match");
        assert_eq!(got.topic, canonicalize_topic("Deploying"));
    }

    #[tokio::test]
    async fn delete_removes_the_page() {
        let s = store().await;
        s.put_page(&page("deploy", "x", vec![1], 1)).await.unwrap();
        s.delete_page("Deploy").await.unwrap(); // canonicalizes to same key
        assert!(s.get_page("deploy").await.unwrap().is_none());
        // Idempotent — deleting a missing page is fine.
        s.delete_page("deploy").await.unwrap();
    }

    #[tokio::test]
    async fn list_orders_most_recent_first() {
        let s = store().await;
        s.put_page(&page("old", "o", vec![1], 10)).await.unwrap();
        s.put_page(&page("new", "n", vec![2], 30)).await.unwrap();
        s.put_page(&page("mid", "m", vec![3], 20)).await.unwrap();
        let rows = s.list_summaries().await.unwrap();
        let topics: Vec<&str> = rows.iter().map(|r| r.topic.as_str()).collect();
        assert_eq!(topics, vec!["new", "mid", "old"]);
    }

    #[tokio::test]
    async fn needs_regen_tracks_the_source_fingerprint() {
        let s = store().await;
        // No page yet → needs regen.
        assert!(s.needs_regen("deploy", &[1, 2]).await);
        s.put_page(&page("deploy", "x", vec![1, 2], 5)).await.unwrap();
        // Same entry set → up to date.
        assert!(!s.needs_regen("deploy", &[2, 1]).await); // order-independent
        // A new entry → stale.
        assert!(s.needs_regen("deploy", &[1, 2, 3]).await);
    }
}
