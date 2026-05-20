//! Phase 89 — `CanonicalizingMemory` — a thin wrapper that
//! canonicalizes the topic-string argument at every
//! topic-keyed entry point in the [`Memory`] trait before
//! delegating to an inner impl.
//!
//! ## Why a wrapper, not an inline flag
//!
//! Both [`crate::InMemoryMemory`] and [`crate::RedbMemory`]
//! have ~7 topic-keyed entry points each. Threading a
//! `canonicalize: bool` field through both impls would duplicate
//! the same `if self.canonicalize { canonicalize_topic(topic) }
//! else { topic }` shim in 14 places. A single wrapper type
//! collapses that to one canonicalization site per method,
//! applied uniformly to whichever inner impl the binary picked.
//!
//! ## What canonicalizes vs. what doesn't
//!
//! **Canonicalizes** (Q3a — the topic-string boundary of the
//! Memory trait): `put`, `get_recent`, `forget`, `gc_topic`,
//! `evict_oldest_unread`, `put_vector`, `promote_recall_helpful`.
//!
//! **Does not canonicalize** (these arguments are not topics
//! in the same sense, and canonicalizing them would change
//! semantics): `scan_prefix` (prefix matching — canonicalizing
//! a prefix `"proj"` to `"proj"` is a no-op but a prefix
//! `"deploys"` to `"deploy"` would over-fold the match);
//! `search` (text query, not a topic); `gc_expired` /
//! `gc_expired_with_rules` (no topic arg / glob rules); the
//! semantic-search vector path (no topic).

use std::sync::Arc;

use async_trait::async_trait;

use crate::{
    canonicalize_topic, Memory, MemoryEntry, MemoryError,
    RetentionMatcher,
};

/// A [`Memory`] wrapper that canonicalizes the topic-string
/// argument of every topic-keyed entry point before
/// delegating. Constructed by the binary at startup when the
/// operator sets `[memory].canonicalize_topics = true`;
/// otherwise the wrapper is not created and the inner impl is
/// used directly (byte-identical to pre-Phase-89).
pub struct CanonicalizingMemory {
    inner: Arc<dyn Memory>,
}

impl CanonicalizingMemory {
    pub fn new(inner: Arc<dyn Memory>) -> Self {
        Self { inner }
    }

    /// Inspect the inner impl. Useful in tests that need to
    /// reach past the wrapper to assert on the underlying
    /// store.
    pub fn inner(&self) -> &Arc<dyn Memory> {
        &self.inner
    }
}

#[async_trait]
impl Memory for CanonicalizingMemory {
    async fn put(
        &self,
        topic: &str,
        body: &str,
    ) -> Result<u64, MemoryError> {
        let canonical = canonicalize_topic(topic);
        self.inner.put(&canonical, body).await
    }

    async fn get_recent(
        &self,
        topic: &str,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        let canonical = canonicalize_topic(topic);
        self.inner.get_recent(&canonical, limit).await
    }

    async fn forget(
        &self,
        topic: &str,
    ) -> Result<usize, MemoryError> {
        let canonical = canonicalize_topic(topic);
        self.inner.forget(&canonical).await
    }

    async fn scan_prefix(
        &self,
        topic_prefix: &str,
        per_topic_limit: usize,
    ) -> Result<Vec<(String, Vec<MemoryEntry>)>, MemoryError> {
        // Prefix matching — passes through unchanged. Folding
        // a prefix would change the matching semantics (a
        // prefix `"deploys"` would canonicalize to `"deploy"`
        // and silently widen the match). Documented in the
        // module header.
        self.inner.scan_prefix(topic_prefix, per_topic_limit).await
    }

    async fn gc_topic(
        &self,
        topic: &str,
        max_entries: usize,
    ) -> Result<usize, MemoryError> {
        let canonical = canonicalize_topic(topic);
        self.inner.gc_topic(&canonical, max_entries).await
    }

    async fn gc_expired(
        &self,
        cutoff_secs: u64,
    ) -> Result<usize, MemoryError> {
        self.inner.gc_expired(cutoff_secs).await
    }

    async fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        // Text query, not a topic key — passes through.
        self.inner.search(query, limit).await
    }

    async fn list_topics(&self) -> Result<Vec<String>, MemoryError> {
        self.inner.list_topics().await
    }

    async fn evict_oldest_unread(
        &self,
        topic: &str,
        keep: usize,
    ) -> Result<usize, MemoryError> {
        let canonical = canonicalize_topic(topic);
        self.inner.evict_oldest_unread(&canonical, keep).await
    }

    async fn gc_expired_with_rules(
        &self,
        rules: &[RetentionMatcher<'_>],
        default_cutoff_secs: Option<u64>,
    ) -> Result<usize, MemoryError> {
        // Glob rules — passes through unchanged. The glob
        // matcher already operates on the canonical-form
        // topics that landed at write time.
        self.inner
            .gc_expired_with_rules(rules, default_cutoff_secs)
            .await
    }

    async fn put_vector(
        &self,
        topic: &str,
        seq: u64,
        vector: Vec<f32>,
    ) -> Result<(), MemoryError> {
        let canonical = canonicalize_topic(topic);
        self.inner.put_vector(&canonical, seq, vector).await
    }

    async fn load_all_vectors(
        &self,
    ) -> Result<Vec<(String, u64, Vec<f32>)>, MemoryError> {
        self.inner.load_all_vectors().await
    }

    async fn semantic_search(
        &self,
        query_vec: &[f32],
        limit: usize,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        self.inner.semantic_search(query_vec, limit).await
    }

    async fn semantic_search_scored(
        &self,
        query_vec: &[f32],
        limit: usize,
    ) -> Result<Vec<(MemoryEntry, f32)>, MemoryError> {
        self.inner.semantic_search_scored(query_vec, limit).await
    }

    async fn promote_recall_helpful(
        &self,
        topic: &str,
        seq: u64,
    ) -> Result<bool, MemoryError> {
        let canonical = canonicalize_topic(topic);
        self.inner.promote_recall_helpful(&canonical, seq).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InMemoryMemory;

    /// The seam contract: a write with morphological variant
    /// (`Deploys`) is found by a read with another variant
    /// (`deploy`) because both fold to the same canonical
    /// topic at the wrapper boundary.
    #[tokio::test]
    async fn put_with_variant_and_get_with_variant_match_via_canonical_fold(
    ) {
        let inner: Arc<dyn Memory> =
            Arc::new(InMemoryMemory::new());
        let mem = CanonicalizingMemory::new(Arc::clone(&inner));

        mem.put("Deploys", "the runbook lives in wiki")
            .await
            .unwrap();
        mem.put("DEPLOYING", "today's 09:00 window")
            .await
            .unwrap();

        // All three variants fold to `deploy`; both writes
        // land under that canonical topic; a `deploy` read
        // finds both.
        let hits = mem.get_recent("deploy", 10).await.unwrap();
        assert_eq!(hits.len(), 2);
        // Inner storage uses the canonical form.
        let inner_hits =
            inner.get_recent("deploy", 10).await.unwrap();
        assert_eq!(inner_hits.len(), 2);
        // The pre-canonical form yields no inner hits — it
        // was never written under the operator's typed
        // string.
        let inner_raw =
            inner.get_recent("Deploys", 10).await.unwrap();
        assert!(inner_raw.is_empty());
    }

    /// Without the wrapper, the variant fragmentation
    /// persists — the baseline that Phase 89 is meant to
    /// fix. Documented as a control: future regressions on
    /// the wrapper's seam behavior will surface as the two
    /// reads returning the same count.
    #[tokio::test]
    async fn without_wrapper_variants_stay_distinct_baseline(
    ) {
        let mem = InMemoryMemory::new();
        mem.put("Deploys", "one").await.unwrap();
        mem.put("deploy", "two").await.unwrap();
        // Distinct topics — each holds one entry.
        let upper =
            mem.get_recent("Deploys", 10).await.unwrap();
        let lower =
            mem.get_recent("deploy", 10).await.unwrap();
        assert_eq!(upper.len(), 1);
        assert_eq!(lower.len(), 1);
        // Different bodies — the wrapper would have merged.
        assert_ne!(upper[0].body, lower[0].body);
    }

    /// `forget` also canonicalizes — forgetting any variant
    /// removes the underlying canonical topic + all its
    /// entries.
    #[tokio::test]
    async fn forget_canonicalizes_the_topic_argument() {
        let inner: Arc<dyn Memory> =
            Arc::new(InMemoryMemory::new());
        let mem = CanonicalizingMemory::new(Arc::clone(&inner));
        mem.put("policies", "x").await.unwrap();
        mem.put("policy", "y").await.unwrap();
        // Both fold to `policy`; one shared bucket of 2.
        let forgotten =
            mem.forget("POLICIES").await.unwrap();
        assert_eq!(forgotten, 2);
        let after =
            mem.get_recent("policy", 10).await.unwrap();
        assert!(after.is_empty());
    }

    /// `put_vector` canonicalizes so the embedding index
    /// remains aligned with the canonical-form entries that
    /// `put` wrote.
    #[tokio::test]
    async fn put_vector_canonicalizes_so_index_aligns() {
        let inner: Arc<dyn Memory> =
            Arc::new(InMemoryMemory::new());
        let mem = CanonicalizingMemory::new(Arc::clone(&inner));
        let seq =
            mem.put("Deploys", "body").await.unwrap();
        mem.put_vector("Deploys", seq, vec![1.0, 0.0])
            .await
            .unwrap();
        // The inner index has one row under the canonical
        // topic `deploy`.
        let vectors =
            inner.load_all_vectors().await.unwrap();
        assert_eq!(vectors.len(), 1);
        assert_eq!(vectors[0].0, "deploy");
    }
}
