//! Chapter Lattice (LT.1) — the persistent typed knowledge-graph store.
//!
//! One encrypted row per **directed triple** `(subject, predicate,
//! object)` in [`aivyx_storage::KeyDomain::KnowledgeGraph`], keyed by the
//! NUL-joined canonical triple so re-extracting the same fact upserts
//! rather than duplicates. This module owns only the *storage* of triples
//! (CRUD + adjacency primitives + the per-topic incremental fingerprint);
//! the extractor that *fills* triples from memory + the LLM lands in LT.2,
//! the sweep cadence in LT.3, and the `graph.query` tool in LT.4.
//!
//! Triples are **derived** — the source of truth is always memory. The
//! store is a cache: a missing or stale triple costs only a re-extraction,
//! never correctness, and a corrupt row degrades only the graph (the
//! Studio view + `graph.query` + the opt-in recall source), never memory
//! or recall.
//!
//! ## Keys
//!
//! - **Triple rows** key on `subject\x00predicate\x00object` (all
//!   canonicalized via [`aivyx_ipc::graph::canonical_label`]). Subjects
//!   are non-empty, so a triple key never starts with `\x00`.
//! - **Fingerprint markers** (the incremental-extraction bookkeeping) key
//!   on `\x00fp\x00<canonical-topic>` — the leading `\x00` is what keeps
//!   them out of the triple scan.

use aivyx_storage::DomainHandle;

pub use aivyx_ipc::graph::{canonical_label, GraphEntity, GraphPath, GraphTriple};

/// Prefix byte that marks a non-triple (metadata) row. A canonical triple
/// key starts with the subject's first byte, which is never `\x00`.
const META_PREFIX: u8 = 0;

/// Errors from the graph store. Same shape as the wiki/ledger stores: a
/// storage-layer failure or a (de)serialization failure, carried as
/// detail strings so the caller can log and skip.
#[derive(Debug, thiserror::Error)]
pub enum GraphStoreError {
    #[error("knowledge-graph storage error: {0}")]
    Storage(String),
    #[error("knowledge-graph encode/decode error: {0}")]
    Encode(String),
}

/// Persistent store for directed typed [`GraphTriple`]s.
pub struct PersistentGraphStore {
    storage: DomainHandle,
}

impl PersistentGraphStore {
    pub fn new(storage: DomainHandle) -> Self {
        Self { storage }
    }

    /// Upsert a triple. Subject / predicate / object are canonicalized so
    /// the stored row keys consistently regardless of the caller's casing
    /// or spacing. An empty canonical part is rejected (a triple must
    /// connect two named entities by a named relation).
    pub async fn put_triple(&self, triple: &GraphTriple) -> Result<(), GraphStoreError> {
        let subject = canonical_label(&triple.subject);
        let predicate = canonical_label(&triple.predicate);
        let object = canonical_label(&triple.object);
        if subject.is_empty() || predicate.is_empty() || object.is_empty() {
            return Err(GraphStoreError::Encode(
                "triple subject/predicate/object must be non-empty after canonicalization".into(),
            ));
        }
        let stored = GraphTriple {
            subject: subject.clone(),
            predicate: predicate.clone(),
            object: object.clone(),
            ..triple.clone()
        };
        let bytes = serde_json::to_vec(&stored)
            .map_err(|e| GraphStoreError::Encode(e.to_string()))?;
        self.storage
            .put(&GraphTriple::key(&subject, &predicate, &object), &bytes)
            .await
            .map_err(|e| GraphStoreError::Storage(e.to_string()))
    }

    /// Fetch a specific triple, if present.
    pub async fn get_triple(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
    ) -> Result<Option<GraphTriple>, GraphStoreError> {
        let key = GraphTriple::key(
            &canonical_label(subject),
            &canonical_label(predicate),
            &canonical_label(object),
        );
        match self.storage.get(&key).await.map_err(|e| GraphStoreError::Storage(e.to_string()))? {
            Some(bytes) => Ok(Some(
                serde_json::from_slice(&bytes).map_err(|e| GraphStoreError::Encode(e.to_string()))?,
            )),
            None => Ok(None),
        }
    }

    /// Delete a specific triple. Idempotent.
    pub async fn delete_triple(
        &self,
        subject: &str,
        predicate: &str,
        object: &str,
    ) -> Result<(), GraphStoreError> {
        let key = GraphTriple::key(
            &canonical_label(subject),
            &canonical_label(predicate),
            &canonical_label(object),
        );
        self.storage.delete(&key).await.map_err(|e| GraphStoreError::Storage(e.to_string()))
    }

    /// Every triple in the graph. Metadata rows (fingerprint markers) and
    /// corrupt rows are skipped — best-effort, so one bad row never blanks
    /// the graph.
    pub async fn all_triples(&self) -> Result<Vec<GraphTriple>, GraphStoreError> {
        let rows = self
            .storage
            .scan_prefix(&[])
            .await
            .map_err(|e| GraphStoreError::Storage(e.to_string()))?;
        Ok(rows
            .iter()
            .filter(|(k, _)| k.first() != Some(&META_PREFIX))
            .filter_map(|(_, v)| serde_json::from_slice(v).ok())
            .collect())
    }

    /// All triples whose **subject** is `entity` (outgoing edges).
    pub async fn out_edges(&self, entity: &str) -> Result<Vec<GraphTriple>, GraphStoreError> {
        let e = canonical_label(entity);
        Ok(self.all_triples().await?.into_iter().filter(|t| t.subject == e).collect())
    }

    /// All triples whose **object** is `entity` (incoming edges).
    pub async fn in_edges(&self, entity: &str) -> Result<Vec<GraphTriple>, GraphStoreError> {
        let e = canonical_label(entity);
        Ok(self.all_triples().await?.into_iter().filter(|t| t.object == e).collect())
    }

    /// The distinct entities (graph nodes) with their degree — how many
    /// triples touch them as subject or object — sorted degree-descending
    /// then name ascending. `kind` is empty in v1 (entity-kind storage is
    /// a future enhancement).
    pub async fn entities(&self) -> Result<Vec<GraphEntity>, GraphStoreError> {
        use std::collections::HashMap;
        let mut deg: HashMap<String, u32> = HashMap::new();
        for t in self.all_triples().await? {
            *deg.entry(t.subject).or_insert(0) += 1;
            *deg.entry(t.object).or_insert(0) += 1;
        }
        let mut out: Vec<GraphEntity> = deg
            .into_iter()
            .map(|(name, degree)| GraphEntity { name, degree, kind: String::new() })
            .collect();
        out.sort_by(|a, b| b.degree.cmp(&a.degree).then_with(|| a.name.cmp(&b.name)));
        Ok(out)
    }

    // ---- incremental-extraction bookkeeping (per topic) ----------------

    fn fingerprint_key(topic: &str) -> Vec<u8> {
        let mut k = vec![META_PREFIX];
        k.extend_from_slice(b"fp");
        k.push(META_PREFIX);
        k.extend_from_slice(canonical_label(topic).as_bytes());
        k
    }

    /// Record that `topic` was extracted at the given source fingerprint.
    pub async fn set_topic_fingerprint(
        &self,
        topic: &str,
        fingerprint: u64,
    ) -> Result<(), GraphStoreError> {
        self.storage
            .put(&Self::fingerprint_key(topic), &fingerprint.to_le_bytes())
            .await
            .map_err(|e| GraphStoreError::Storage(e.to_string()))
    }

    /// The fingerprint a topic was last extracted at, if any.
    pub async fn topic_fingerprint(&self, topic: &str) -> Result<Option<u64>, GraphStoreError> {
        let raw = self
            .storage
            .get(&Self::fingerprint_key(topic))
            .await
            .map_err(|e| GraphStoreError::Storage(e.to_string()))?;
        Ok(raw.and_then(|b| b.try_into().ok().map(u64::from_le_bytes)))
    }

    /// Whether a topic needs (re)extraction given its current entry
    /// fingerprint: `true` when it was never extracted or the fingerprint
    /// changed. A storage error fails toward freshness.
    pub async fn needs_regen(&self, topic: &str, fingerprint: u64) -> bool {
        match self.topic_fingerprint(topic).await {
            Ok(Some(fp)) => fp != fingerprint,
            _ => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    async fn store() -> PersistentGraphStore {
        use aivyx_crypto::MasterKey;
        use aivyx_storage::{KeyDomain, RedbStorage, Storage, StorageConfig};
        let base = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let dir = std::path::PathBuf::from(base)
            .join(format!("aivyx-graph-store-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let s: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("store.redb")),
            MasterKey::from_raw([76u8; 32]),
        )
        .await
        .unwrap();
        PersistentGraphStore::new(s.domain(KeyDomain::KnowledgeGraph))
    }

    fn triple(s: &str, p: &str, o: &str) -> GraphTriple {
        GraphTriple {
            subject: s.into(),
            predicate: p.into(),
            object: o.into(),
            source_seqs: vec![1],
            mentions: 1,
            updated_at: 1,
        }
    }

    #[tokio::test]
    async fn put_get_round_trips_and_canonicalizes() {
        let g = store().await;
        g.put_triple(&triple("Deploy", "Depends-On", "CI")).await.unwrap();
        // Differently-cased lookup finds the canonical row.
        let got = g.get_triple("deploy", "depends-on", "ci").await.unwrap().expect("present");
        assert_eq!(got.subject, "deploy");
        assert_eq!(got.predicate, "depends-on");
        assert_eq!(got.object, "ci");
    }

    #[tokio::test]
    async fn empty_part_rejected() {
        let g = store().await;
        assert!(g.put_triple(&triple("deploy", "  ", "ci")).await.is_err());
    }

    #[tokio::test]
    async fn out_and_in_edges_respect_direction() {
        let g = store().await;
        g.put_triple(&triple("deploy", "depends-on", "ci")).await.unwrap();
        g.put_triple(&triple("ci", "triggers", "rollback")).await.unwrap();
        g.put_triple(&triple("rollback", "reverts", "deploy")).await.unwrap();

        let out = g.out_edges("ci").await.unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].object, "rollback");

        let into = g.in_edges("deploy").await.unwrap();
        assert_eq!(into.len(), 1);
        assert_eq!(into[0].subject, "rollback");
    }

    #[tokio::test]
    async fn entities_count_degree() {
        let g = store().await;
        g.put_triple(&triple("deploy", "depends-on", "ci")).await.unwrap();
        g.put_triple(&triple("deploy", "uses", "docker")).await.unwrap();
        let ents = g.entities().await.unwrap();
        // deploy touches 2 triples → highest degree, listed first.
        assert_eq!(ents[0].name, "deploy");
        assert_eq!(ents[0].degree, 2);
        assert!(ents.iter().any(|e| e.name == "ci" && e.degree == 1));
        assert!(ents.iter().any(|e| e.name == "docker" && e.degree == 1));
    }

    #[tokio::test]
    async fn delete_removes_only_that_triple() {
        let g = store().await;
        g.put_triple(&triple("deploy", "depends-on", "ci")).await.unwrap();
        g.put_triple(&triple("deploy", "uses", "docker")).await.unwrap();
        g.delete_triple("Deploy", "depends-on", "CI").await.unwrap();
        assert!(g.get_triple("deploy", "depends-on", "ci").await.unwrap().is_none());
        assert!(g.get_triple("deploy", "uses", "docker").await.unwrap().is_some());
        assert_eq!(g.all_triples().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn fingerprint_markers_dont_leak_into_triples() {
        let g = store().await;
        g.put_triple(&triple("deploy", "depends-on", "ci")).await.unwrap();
        g.set_topic_fingerprint("deploy", 42).await.unwrap();
        // The marker is invisible to the triple scan...
        assert_eq!(g.all_triples().await.unwrap().len(), 1);
        // ...but readable for the incremental check.
        assert_eq!(g.topic_fingerprint("Deploy").await.unwrap(), Some(42));
        assert!(!g.needs_regen("deploy", 42).await);
        assert!(g.needs_regen("deploy", 43).await);
        assert!(g.needs_regen("never-extracted", 1).await);
    }
}
