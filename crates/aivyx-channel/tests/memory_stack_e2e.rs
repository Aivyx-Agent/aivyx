//! Chapter Synapse (SY.2) — end-to-end composition proof for the memory
//! stack.
//!
//! The Loom → Codex → Lattice → Lexicon chapters each tested their piece
//! against a *scripted fake* LLM in *isolation*. This is the test the arc
//! lacked: the **real** components wired together, proving the whole
//! pipeline composes —
//!
//! ```text
//! write memories
//!   → the wiki sweep synthesizes a page          (Codex)
//!   → the graph sweep extracts typed triples      (Lattice + Lexicon)
//!   → graph.query traverses them                  (Lattice)
//!   → recall fuses the wiki summary AND the typed-graph neighbor
//!                                                  (Codex + Lattice in Loom)
//! ```
//!
//! Fully autonomous (no live model): one routing provider answers the
//! wiki call with a summary and the graph call with a JSON triple array,
//! switching on the request's system prompt.

use std::sync::Arc;

use async_trait::async_trait;

use aivyx_channel::cooccurrence_ledger::PersistentCooccurrenceLedger;
use aivyx_channel::graph_query_tool::GraphQueryTool;
use aivyx_channel::knowledge_graph::{GraphDirection, GraphExtractor, PersistentGraphStore};
use aivyx_channel::knowledge_wiki::{PersistentWikiStore, WikiSynthesizer};
use aivyx_channel::memory_recall::SemanticMemoryContext;
use aivyx_core::llm_planner::ContextProvider;
use aivyx_core::{CancellationToken, SessionId, Tool, ToolOutcome};
use aivyx_llm::embedding::{EmbeddingError, EmbeddingProvider};
use aivyx_llm::{LlmError, LlmRequest, LlmStepEnd, LlmStream, LlmStreamEvent, LlmUsage};
use aivyx_memory::{InMemoryMemory, Memory};
use aivyx_storage::{KeyDomain, RedbStorage, Storage, StorageConfig};

// ---- a provider that routes wiki-summary vs. graph-triple calls --------

struct RoutingProvider;
struct CannedStream {
    text: String,
}

#[async_trait]
impl aivyx_llm::LlmProvider for RoutingProvider {
    async fn chat_stream(
        &self,
        request: LlmRequest<'_>,
        _c: &CancellationToken,
    ) -> Result<Box<dyn LlmStream>, LlmError> {
        let system = request.system.unwrap_or("");
        // The graph extractor's system prompt is the discriminator.
        let text = if system.contains("JSON array of directed relation triples") {
            // Use a synonym ("requires") to also exercise the Lexicon fold.
            r#"[{"subject":"deploy","predicate":"requires","object":"ci"}]"#.to_string()
        } else {
            // The wiki consolidation. "consolidated" is a marker we assert on.
            "Deploy is consolidated: it ships via CI and runs the tests.".to_string()
        };
        Ok(Box::new(CannedStream { text }))
    }
}

#[async_trait]
impl LlmStream for CannedStream {
    async fn next_event(&mut self) -> Result<Option<LlmStreamEvent>, LlmError> {
        Ok(None)
    }
    async fn finish(self: Box<Self>) -> Result<LlmStepEnd, LlmError> {
        Ok(LlmStepEnd::FinalMessage { text: self.text, usage: LlmUsage::default() })
    }
}

/// A constant embedding: every input → `[1, 0]`. A doc's semantic cosine
/// is then governed purely by the vector we assign it via `put_vector`.
struct ConstEmbed;
#[async_trait]
impl EmbeddingProvider for ConstEmbed {
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
        Ok(texts.iter().map(|_| vec![1.0, 0.0]).collect())
    }
    fn model(&self) -> &str {
        "const"
    }
    fn dimensions(&self) -> usize {
        2
    }
}

async fn open_store() -> Arc<dyn Storage> {
    use aivyx_crypto::MasterKey;
    let base = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
    let dir = std::path::PathBuf::from(base)
        .join(format!("aivyx-memstack-e2e-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    RedbStorage::open(
        StorageConfig::new(dir.join("store.redb")),
        MasterKey::from_raw([42u8; 32]),
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn memory_stack_composes_end_to_end() {
    let store = open_store().await;
    let provider: Arc<dyn aivyx_llm::LlmProvider> = Arc::new(RoutingProvider);

    // --- seed memory ---------------------------------------------------
    let mem: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
    let ds = mem.put("deploy", "the deploy pipeline ships via ci").await.unwrap();
    // "deploy" is semantically reachable from the query (vector [1,0]).
    mem.put_vector("deploy", ds, vec![1.0, 0.0]).await.unwrap();
    // "ci" has a real entry but NO vector and shares no query words → it
    // is reachable ONLY through the typed graph (deploy -> ci).
    mem.put("ci", "ci runs the whole test suite nightly").await.unwrap();

    // --- the stores + the co-occurrence ledger -------------------------
    let wiki_store = Arc::new(PersistentWikiStore::new(store.domain(KeyDomain::KnowledgeWiki)));
    let graph_store =
        Arc::new(PersistentGraphStore::new(store.domain(KeyDomain::KnowledgeGraph)));
    let cooc = Arc::new(PersistentCooccurrenceLedger::new(
        store.domain(KeyDomain::CooccurrenceLedger),
    ));

    // --- (Codex) the wiki sweep synthesizes pages ----------------------
    let synth = WikiSynthesizer::new(
        Arc::clone(&mem),
        Arc::clone(&provider),
        Arc::clone(&wiki_store),
        "fake-model",
    );
    let wiki_report = synth.sweep(1000, 50).await;
    assert!(wiki_report.wrote >= 1, "wiki sweep wrote pages: {wiki_report:?}");
    let deploy_page = wiki_store.get_page("deploy").await.unwrap().expect("deploy page");
    assert!(deploy_page.summary.contains("consolidated"), "the page is the LLM summary");

    // --- (Lattice + Lexicon) the graph sweep extracts triples ----------
    let extractor = GraphExtractor::new(
        Arc::clone(&mem),
        Arc::clone(&provider),
        Arc::clone(&graph_store),
        "fake-model",
    );
    let graph_report = extractor.sweep(1000, 50).await;
    assert!(graph_report.triples >= 1, "graph sweep extracted triples: {graph_report:?}");
    // The synonym "requires" was folded to the canonical "depends-on" (Lexicon).
    assert!(
        graph_store.get_triple("deploy", "depends-on", "ci").await.unwrap().is_some(),
        "extracted triple is canonical, not the raw synonym",
    );

    // --- (Lattice) graph.query traverses the extracted graph -----------
    let q = GraphQueryTool::new();
    let _ = q.set_store(Arc::clone(&graph_store));
    let direct = graph_store
        .query("deploy", GraphDirection::Out, None, 2, 50)
        .await
        .unwrap();
    assert!(direct.iter().any(|p| p.entity == "ci"), "graph.query reaches ci from deploy");
    // And the tool itself answers (capability surface works end to end).
    let tool_out = q
        .execute(serde_json::json!({ "start": "deploy" }), &test_ctx().0.ctx())
        .await;
    assert!(matches!(tool_out, ToolOutcome::Completed { .. }), "graph.query tool runs");

    // --- (Loom) recall fuses the wiki summary AND the typed-graph hop --
    // The "smart" bundle: hybrid + the wiki + typed-graph sources armed.
    let recall = SemanticMemoryContext::new(
        Arc::clone(&mem),
        Arc::new(ConstEmbed),
        5,
        0.0,
    )
    .with_recall_hybrid(true)
    .with_cluster(Arc::clone(&cooc), aivyx_config::RecallClusterConfig {
        enabled: true,
        max_siblings: 4,
        min_affinity: 1.0,
    })
    .with_recall_wiki(Arc::clone(&wiki_store), 1.0)
    .with_recall_typed_graph(Arc::clone(&graph_store), 1.0);

    let block = recall
        .recall("how does deploy work", SessionId::new(), aivyx_core::TurnId::new())
        .await
        .expect("recall produces a block");

    // The whole pipeline shows up in one turn's context:
    assert!(block.contains("ships via ci"), "semantic recall of deploy's entry");
    assert!(block.contains("consolidated"), "the wiki page summary is fused in");
    assert!(
        block.contains("test suite"),
        "ci — reachable ONLY via the typed graph — is fused in: {block}",
    );
}

// A tiny ToolContext harness for the graph.query tool call above.
mod ctx_support {
    use aivyx_core::{
        AgentId, AuditHook, ChannelContext, ChannelError, ChannelPlatform, NullAuditHook,
        SessionId, StreamEvent, ToolContext, TurnId, TurnOutcome,
    };
    use aivyx_core::CancellationToken;
    use async_trait::async_trait;

    pub struct Harness {
        session: SessionId,
        token: CancellationToken,
        audit: NullAuditHook,
    }
    #[async_trait]
    impl ChannelContext for Harness {
        fn channel_name(&self) -> &str {
            "test"
        }
        fn platform(&self) -> ChannelPlatform {
            ChannelPlatform::Local
        }
        fn trust_tier(&self) -> aivyx_capability::TrustTier {
            aivyx_capability::TrustTier::Trusted
        }
        fn session_id(&self) -> SessionId {
            self.session
        }
        async fn stream_event(&self, _e: StreamEvent<'_>) -> Result<(), ChannelError> {
            Ok(())
        }
        async fn finalize(&self, _o: &TurnOutcome) -> Result<(), ChannelError> {
            Ok(())
        }
        fn cancellation_token(&self) -> CancellationToken {
            self.token.clone()
        }
    }
    impl Harness {
        pub fn ctx(&self) -> ToolContext<'_> {
            ToolContext {
                agent_id: AgentId::new(),
                session_id: self.session,
                turn_id: TurnId::new(),
                channel: self,
                audit: &self.audit as &dyn AuditHook,
                cancellation: &self.token,
            }
        }
    }
    pub fn harness() -> Harness {
        Harness {
            session: SessionId::new(),
            token: CancellationToken::new(),
            audit: NullAuditHook,
        }
    }
}

fn test_ctx() -> (ctx_support::Harness,) {
    (ctx_support::harness(),)
}
