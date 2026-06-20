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

// ---------------------------------------------------------------------------
// GraphExtractor — Chapter Lattice (LT.2)
// ---------------------------------------------------------------------------

use std::sync::Arc;

use aivyx_core::CancellationToken;
use aivyx_llm::{ContentBlock, LlmMessage, LlmProvider, LlmRequest, LlmStepEnd};
use aivyx_memory::{Memory, MemoryEntry};
use serde::Deserialize;

/// Tuning for triple extraction. Defaults aim for a cheap, bounded pass.
#[derive(Debug, Clone)]
pub struct GraphExtractConfig {
    /// Newest entries (per topic) to extract from.
    pub max_entries: usize,
    /// Per-entry body cap (chars) in the prompt.
    pub max_entry_chars: usize,
    /// LLM token budget for the triple list.
    pub max_tokens: u32,
    /// Hard cap on triples kept per topic per pass (defends a runaway
    /// model dump).
    pub max_triples: usize,
}

impl Default for GraphExtractConfig {
    fn default() -> Self {
        Self { max_entries: 50, max_entry_chars: 500, max_tokens: 700, max_triples: 64 }
    }
}

/// What an extraction pass did for one topic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphRegenOutcome {
    /// Up to date (fingerprint matched) — nothing re-extracted.
    Skipped,
    /// The topic has no entries to extract from.
    NoEntries,
    /// `n` triples were (re)extracted and stored.
    Wrote(usize),
}

/// One LLM-emitted triple, before canonicalization/validation.
#[derive(Debug, Deserialize)]
struct RawTriple {
    #[serde(default)]
    subject: String,
    #[serde(default)]
    predicate: String,
    #[serde(default)]
    object: String,
}

/// Extracts directed typed triples from memory into the graph store.
/// Best-effort throughout (no provider, an LLM error, an unparseable
/// response, or a storage hiccup leaves the existing graph untouched) and
/// incremental (a topic whose entries are unchanged is skipped).
pub struct GraphExtractor {
    memory: Arc<dyn Memory>,
    provider: Arc<dyn LlmProvider>,
    store: Arc<PersistentGraphStore>,
    model: String,
    config: GraphExtractConfig,
}

impl GraphExtractor {
    pub fn new(
        memory: Arc<dyn Memory>,
        provider: Arc<dyn LlmProvider>,
        store: Arc<PersistentGraphStore>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            memory,
            provider,
            store,
            model: model.into(),
            config: GraphExtractConfig::default(),
        }
    }

    pub fn with_config(mut self, config: GraphExtractConfig) -> Self {
        self.config = config;
        self
    }

    fn system_prompt() -> &'static str {
        "You extract a knowledge graph from an AI assistant's memory notes \
         about one topic. Output ONLY a JSON array of directed relation \
         triples, each `{\"subject\":\"...\",\"predicate\":\"...\",\"object\":\"...\"}`. \
         The predicate is a short directed relation label (e.g. \
         \"depends-on\", \"caused\", \"owns\", \"part-of\"); direction \
         matters (subject → object). Extract ONLY relations the notes \
         actually state — do not invent, infer beyond the text, or add \
         commentary. Use concise noun-phrase entities. If the notes state \
         no clear relations, output `[]`. No prose, no markdown fences."
    }

    /// Render entries into the user prompt. Pure + testable.
    fn user_prompt(&self, topic: &str, entries: &[MemoryEntry]) -> String {
        let mut s = format!("Topic: {topic}\n\nNotes:\n");
        for e in entries {
            let body: String = if e.body.chars().count() > self.config.max_entry_chars {
                e.body.chars().take(self.config.max_entry_chars).collect::<String>() + "…"
            } else {
                e.body.clone()
            };
            s.push_str(&format!("- {}\n", body.replace('\n', " ")));
        }
        s.push_str("\nOutput the JSON triple array now.");
        s
    }

    /// Tolerant parse of the model's response into raw triples: locate the
    /// outermost `[ … ]` (ignoring any prose/fences around it) and decode.
    /// Returns an empty vec on any failure — best-effort.
    fn parse_triples(raw: &str) -> Vec<RawTriple> {
        let (Some(start), Some(end)) = (raw.find('['), raw.rfind(']')) else {
            return Vec::new();
        };
        if end <= start {
            return Vec::new();
        }
        serde_json::from_str::<Vec<RawTriple>>(&raw[start..=end]).unwrap_or_default()
    }

    /// One-shot LLM extraction → validated, deduped `(s, p, o, count)`
    /// triples (count = how many times the model emitted the same triple
    /// in this batch → the `mentions` weight). Best-effort.
    async fn extract(
        &self,
        topic: &str,
        entries: &[MemoryEntry],
    ) -> Vec<(String, String, String, u32)> {
        let user = self.user_prompt(topic, entries);
        let messages = vec![LlmMessage::User { content: vec![ContentBlock::Text { text: user }] }];
        let request = LlmRequest {
            model: &self.model,
            system: Some(Self::system_prompt()),
            messages: &messages,
            tools: &[],
            max_tokens: self.config.max_tokens,
            temperature: Some(0.1),
        };
        let token = CancellationToken::new();
        let Ok(mut stream) = self.provider.chat_stream(request, &token).await else {
            return Vec::new();
        };
        while stream.next_event().await.map(|e| e.is_some()).unwrap_or(false) {}
        let text = match stream.finish().await {
            Ok(LlmStepEnd::FinalMessage { text, .. }) => text,
            _ => return Vec::new(),
        };

        // Validate + canonicalize + dedup (counting repeats as mentions).
        use std::collections::HashMap;
        let mut counts: HashMap<(String, String, String), u32> = HashMap::new();
        for rt in Self::parse_triples(&text) {
            let s = canonical_label(&rt.subject);
            let p = canonical_label(&rt.predicate);
            let o = canonical_label(&rt.object);
            // Reject empties and self-loops (an entity related to itself
            // by the same name is noise).
            if s.is_empty() || p.is_empty() || o.is_empty() || s == o {
                continue;
            }
            *counts.entry((s, p, o)).or_insert(0) += 1;
        }
        let mut out: Vec<(String, String, String, u32)> =
            counts.into_iter().map(|((s, p, o), n)| (s, p, o, n)).collect();
        // Deterministic order, then cap.
        out.sort();
        out.truncate(self.config.max_triples);
        out
    }

    /// Make a topic's triples current: pulls its entries, skips when the
    /// fingerprint matches (incremental), otherwise extracts + stores the
    /// triples and records the new fingerprint. Best-effort: any soft
    /// failure returns [`GraphRegenOutcome::Skipped`].
    pub async fn regenerate(&self, topic: &str, now_secs: u64) -> GraphRegenOutcome {
        let entries = match self.memory.get_recent(topic, self.config.max_entries).await {
            Ok(e) => e,
            Err(_) => return GraphRegenOutcome::Skipped,
        };
        if entries.is_empty() {
            return GraphRegenOutcome::NoEntries;
        }
        let seqs: Vec<u64> = entries.iter().map(|e| e.seq).collect();
        let fingerprint = aivyx_ipc::wiki::WikiPage::fingerprint(&seqs);
        if !self.store.needs_regen(topic, fingerprint).await {
            return GraphRegenOutcome::Skipped;
        }
        let triples = self.extract(topic, &entries).await;
        let mut wrote = 0usize;
        for (subject, predicate, object, mentions) in triples {
            let t = GraphTriple {
                subject,
                predicate,
                object,
                source_seqs: seqs.clone(),
                mentions,
                updated_at: now_secs,
            };
            if self.store.put_triple(&t).await.is_ok() {
                wrote += 1;
            }
        }
        // Record the fingerprint even when zero triples were found, so a
        // topic with no relations isn't re-extracted every sweep.
        let _ = self.store.set_topic_fingerprint(topic, fingerprint).await;
        GraphRegenOutcome::Wrote(wrote)
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

    // ---- LT.2 — GraphExtractor --------------------------------------

    use aivyx_llm::{
        LlmError, LlmRequest, LlmStepEnd, LlmStream, LlmStreamEvent, LlmUsage,
    };
    use aivyx_memory::InMemoryMemory;
    use async_trait::async_trait;

    struct ScriptedProvider {
        text: String,
        fail: bool,
    }
    struct ScriptedStream {
        text: String,
    }
    #[async_trait]
    impl LlmProvider for ScriptedProvider {
        async fn chat_stream(
            &self,
            _request: LlmRequest<'_>,
            _cancellation: &CancellationToken,
        ) -> Result<Box<dyn LlmStream>, LlmError> {
            if self.fail {
                return Err(LlmError::Transport("scripted failure".into()));
            }
            Ok(Box::new(ScriptedStream { text: self.text.clone() }))
        }
    }
    #[async_trait]
    impl LlmStream for ScriptedStream {
        async fn next_event(&mut self) -> Result<Option<LlmStreamEvent>, LlmError> {
            Ok(None)
        }
        async fn finish(self: Box<Self>) -> Result<LlmStepEnd, LlmError> {
            Ok(LlmStepEnd::FinalMessage { text: self.text, usage: LlmUsage::default() })
        }
    }

    fn extractor(
        memory: Arc<dyn Memory>,
        store: Arc<PersistentGraphStore>,
        text: &str,
        fail: bool,
    ) -> GraphExtractor {
        GraphExtractor::new(
            memory,
            Arc::new(ScriptedProvider { text: text.into(), fail }),
            store,
            "fake-model",
        )
    }

    #[test]
    fn parse_triples_tolerates_prose_and_fences() {
        let raw = "Sure! Here are the triples:\n```json\n\
            [{\"subject\":\"deploy\",\"predicate\":\"depends-on\",\"object\":\"ci\"}]\n```";
        let parsed = GraphExtractor::parse_triples(raw);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].subject, "deploy");
        // Garbage → empty, never panics.
        assert!(GraphExtractor::parse_triples("no json here").is_empty());
        assert!(GraphExtractor::parse_triples("[").is_empty());
    }

    #[tokio::test]
    async fn regenerate_extracts_and_stores_directed_triples() {
        let mem: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
        mem.put("deploy", "we ship via the ci pipeline").await.unwrap();
        mem.put("deploy", "rollback reverts the deploy").await.unwrap();
        let g = Arc::new(store().await);
        let resp = r#"[
            {"subject":"deploy","predicate":"depends-on","object":"ci"},
            {"subject":"rollback","predicate":"reverts","object":"deploy"},
            {"subject":"deploy","predicate":"is","object":"deploy"}
        ]"#; // the self-loop must be dropped
        let x = extractor(Arc::clone(&mem), Arc::clone(&g), resp, false);

        match x.regenerate("deploy", 500).await {
            GraphRegenOutcome::Wrote(n) => assert_eq!(n, 2, "self-loop dropped"),
            other => panic!("expected Wrote, got {other:?}"),
        }
        let t = g.get_triple("deploy", "depends-on", "ci").await.unwrap().expect("stored");
        assert_eq!(t.source_seqs.len(), 2);
        assert_eq!(t.updated_at, 500);
        assert!(g.get_triple("rollback", "reverts", "deploy").await.unwrap().is_some());
        // Direction-sensitive: the reverse wasn't asserted.
        assert!(g.get_triple("ci", "depends-on", "deploy").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn regenerate_is_incremental() {
        let mem: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
        mem.put("deploy", "ship via ci").await.unwrap();
        let g = Arc::new(store().await);
        let x = extractor(
            Arc::clone(&mem),
            Arc::clone(&g),
            r#"[{"subject":"deploy","predicate":"uses","object":"ci"}]"#,
            false,
        );
        assert!(matches!(x.regenerate("deploy", 1).await, GraphRegenOutcome::Wrote(1)));
        // Unchanged entries → skipped (no second LLM pass).
        assert_eq!(x.regenerate("deploy", 2).await, GraphRegenOutcome::Skipped);
        // New entry → re-extracts.
        mem.put("deploy", "deploy uses docker too").await.unwrap();
        assert!(matches!(x.regenerate("deploy", 3).await, GraphRegenOutcome::Wrote(_)));
    }

    #[tokio::test]
    async fn regenerate_best_effort_on_failure_and_no_entries() {
        let mem: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
        let g = Arc::new(store().await);
        // No entries.
        let x = extractor(Arc::clone(&mem), Arc::clone(&g), "[]", false);
        assert_eq!(x.regenerate("empty", 1).await, GraphRegenOutcome::NoEntries);
        // LLM failure → Wrote(0), no triples, never errors.
        mem.put("deploy", "note").await.unwrap();
        let xf = extractor(Arc::clone(&mem), Arc::clone(&g), "", true);
        assert_eq!(xf.regenerate("deploy", 1).await, GraphRegenOutcome::Wrote(0));
        assert!(g.all_triples().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn regenerate_counts_repeats_as_mentions() {
        let mem: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
        mem.put("deploy", "ci ci ci").await.unwrap();
        let g = Arc::new(store().await);
        let resp = r#"[
            {"subject":"deploy","predicate":"uses","object":"ci"},
            {"subject":"Deploy","predicate":"USES","object":"CI"}
        ]"#; // same triple twice (different casing) → mentions 2
        let x = extractor(Arc::clone(&mem), Arc::clone(&g), resp, false);
        assert!(matches!(x.regenerate("deploy", 1).await, GraphRegenOutcome::Wrote(1)));
        let t = g.get_triple("deploy", "uses", "ci").await.unwrap().unwrap();
        assert_eq!(t.mentions, 2);
    }
}
