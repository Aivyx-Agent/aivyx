//! Chapter Lattice (LT.4) — the `graph.query` agent tool.
//!
//! A read-only **multi-hop, directed, typed traversal** of the agent's
//! own typed knowledge graph (the `(subject)-[predicate]->(object)`
//! relations Chapter Lattice extracts from memory). From a start entity,
//! follow directed edges — optionally filtered to one predicate, in a
//! chosen direction — up to a hop cap, returning the reachable entities
//! and the typed path to each. This is what makes the graph *reasoned
//! with* ("what depends on the deploy pipeline?"), not just stored.
//!
//! Gated by the `graph.read` capability base (Chapter Lattice) — an
//! **infrastructure** read, like `skills.list`: the agent querying its
//! own derived self-knowledge. `OnceLock`-store pattern (registered with
//! an empty slot, filled by the binary's startup path), the same shape as
//! `mission_tool` / `loop_tool`.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use serde_json::{json, Value};

use aivyx_capability::Scope;
use aivyx_core::{AivyxError, Tool, ToolContext, ToolId, ToolOutcome, Verification};

use crate::knowledge_graph::{GraphDirection, PersistentGraphStore};

/// Default + hard caps on a traversal, so a query can't walk an
/// unbounded graph or return an unbounded result.
const DEFAULT_MAX_HOPS: u32 = 3;
const MAX_HOPS_CAP: u32 = 6;
const MAX_RESULTS: usize = 50;

/// `graph.query` — multi-hop typed traversal of the knowledge graph.
pub struct GraphQueryTool {
    id: ToolId,
    schema: Value,
    store: OnceLock<Arc<PersistentGraphStore>>,
}

impl std::fmt::Debug for GraphQueryTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphQueryTool")
            .field("id", &self.id)
            .field("has_store", &self.store.get().is_some())
            .finish()
    }
}

impl Default for GraphQueryTool {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphQueryTool {
    pub fn new() -> Self {
        GraphQueryTool {
            id: ToolId::new(),
            schema: json!({
                "type": "object",
                "properties": {
                    "start": {
                        "type": "string",
                        "description": "The entity to traverse from \
                                        (e.g. \"deploy pipeline\")."
                    },
                    "direction": {
                        "type": "string",
                        "enum": ["out", "in", "both"],
                        "description": "Edge direction to follow. \"out\" \
                                        (default) follows subject→object \
                                        (\"what does X relate to?\"); \"in\" \
                                        follows object→subject (\"what \
                                        relates to X?\"); \"both\" follows \
                                        either."
                    },
                    "predicate": {
                        "type": "string",
                        "description": "Optional: only follow edges with \
                                        this relation label (e.g. \
                                        \"depends-on\")."
                    },
                    "max_hops": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": MAX_HOPS_CAP as i64,
                        "description": "How many hops to traverse. Default \
                                        3, max 6."
                    }
                },
                "required": ["start"]
            }),
            store: OnceLock::new(),
        }
    }

    /// Install the graph store. Called once by the binary's startup path.
    pub fn set_store(
        &self,
        store: Arc<PersistentGraphStore>,
    ) -> Result<(), Arc<PersistentGraphStore>> {
        self.store.set(store)
    }
}

#[async_trait]
impl Tool for GraphQueryTool {
    fn id(&self) -> ToolId {
        self.id
    }

    fn name(&self) -> &str {
        "graph.query"
    }

    fn description(&self) -> &str {
        "Query the agent's typed knowledge graph: a read-only multi-hop \
         traversal over the directed (subject)-[predicate]->(object) \
         relations extracted from memory. Input: { start, direction? \
         (out|in|both, default out), predicate? (only follow this \
         relation), max_hops? (default 3, max 6) }. Returns the reachable \
         entities with the typed path to each. Reach for it whenever the \
         user asks how things relate, depend, connect, or what something \
         caused/owns/contains — \"what depends on X?\", \"what did Y \
         cause?\", \"how is A connected to B?\" — questions a plain memory \
         recall can't answer by walking relations."
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn required_scope(&self, _input: &Value) -> Scope {
        Scope::parse("graph.read").expect("graph.read is in KNOWN_BASES")
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext<'_>) -> ToolOutcome {
        let Some(store) = self.store.get() else {
            return ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: "graph.query: no knowledge-graph store configured".to_string(),
            });
        };
        let start = match input.get("start").and_then(Value::as_str) {
            Some(s) if !s.trim().is_empty() => s,
            _ => {
                return ToolOutcome::Failed(AivyxError::Tool {
                    tool: self.id,
                    detail: "graph.query requires a non-empty `start` entity".to_string(),
                });
            }
        };
        let direction = GraphDirection::from_arg(input.get("direction").and_then(Value::as_str));
        let predicate = input
            .get("predicate")
            .and_then(Value::as_str)
            .filter(|p| !p.trim().is_empty());
        let max_hops = input
            .get("max_hops")
            .and_then(Value::as_u64)
            .map(|h| (h as u32).clamp(1, MAX_HOPS_CAP))
            .unwrap_or(DEFAULT_MAX_HOPS);

        match store.query(start, direction, predicate, max_hops, MAX_RESULTS).await {
            Ok(paths) => ToolOutcome::Completed {
                output: json!({
                    "start": start,
                    "count": paths.len(),
                    "results": paths.iter().map(|p| json!({
                        "entity": p.entity,
                        "hops": p.hops,
                        "path": p.path,
                    })).collect::<Vec<_>>(),
                }),
                verified: Verification::Verified,
            },
            Err(e) => ToolOutcome::Failed(AivyxError::Tool {
                tool: self.id,
                detail: format!("graph.query failed: {e}"),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::knowledge_graph::GraphTriple;

    async fn store_with_chain() -> Arc<PersistentGraphStore> {
        use aivyx_crypto::MasterKey;
        use aivyx_storage::{KeyDomain, RedbStorage, Storage, StorageConfig};
        let base = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let dir = std::path::PathBuf::from(base)
            .join(format!("aivyx-graphq-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let s: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("s.redb")),
            MasterKey::from_raw([55u8; 32]),
        )
        .await
        .unwrap();
        let g = Arc::new(PersistentGraphStore::new(s.domain(KeyDomain::KnowledgeGraph)));
        for (su, p, o) in [
            ("deploy", "depends-on", "ci"),
            ("ci", "triggers", "rollback"),
            ("deploy", "uses", "docker"),
        ] {
            g.put_triple(&GraphTriple {
                subject: su.into(),
                predicate: p.into(),
                object: o.into(),
                source_seqs: vec![1],
                mentions: 1,
                updated_at: 1,
            })
            .await
            .unwrap();
        }
        g
    }

    // Minimal ChannelContext fake so the tool tests can build a
    // ToolContext (graph.query ignores it, but `execute` requires one).
    use aivyx_core::{
        AgentId, ChannelContext, ChannelError, ChannelPlatform, NullAuditHook, SessionId,
        StreamEvent, ToolContext, TurnId, TurnOutcome,
    };
    use aivyx_core::CancellationToken;

    struct NoopChannel {
        session: SessionId,
        token: CancellationToken,
    }
    #[async_trait]
    impl ChannelContext for NoopChannel {
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
        async fn stream_event(&self, _event: StreamEvent<'_>) -> Result<(), ChannelError> {
            Ok(())
        }
        async fn finalize(&self, _outcome: &TurnOutcome) -> Result<(), ChannelError> {
            Ok(())
        }
        fn cancellation_token(&self) -> CancellationToken {
            self.token.clone()
        }
    }
    fn fresh_channel() -> NoopChannel {
        NoopChannel { session: SessionId::new(), token: CancellationToken::new() }
    }
    fn make_ctx<'a>(channel: &'a NoopChannel, audit: &'a dyn aivyx_core::AuditHook) -> ToolContext<'a> {
        ToolContext {
            agent_id: AgentId::new(),
            session_id: channel.session,
            turn_id: TurnId::new(),
            channel,
            audit,
            cancellation: &channel.token,
            message_origin: aivyx_core::MessageOrigin::Operator,
        }
    }

    #[test]
    fn required_scope_is_graph_read() {
        let t = GraphQueryTool::new();
        assert_eq!(t.required_scope(&json!({"start": "deploy"})).base(), "graph.read");
        assert_eq!(t.name(), "graph.query");
    }

    #[tokio::test]
    async fn execute_returns_reachable_entities_with_paths() {
        let t = GraphQueryTool::new();
        assert!(t.set_store(store_with_chain().await).is_ok());
        let channel = fresh_channel();
        let audit = NullAuditHook;
        let ctx = make_ctx(&channel, &audit);
        let outcome = t.execute(json!({ "start": "Deploy", "max_hops": 3 }), &ctx).await;
        match outcome {
            ToolOutcome::Completed { output, .. } => {
                assert_eq!(output["start"], "Deploy");
                let results = output["results"].as_array().unwrap();
                assert!(results.iter().any(|r| r["entity"] == "rollback" && r["hops"] == 2));
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn execute_without_store_or_start_fails_cleanly() {
        let channel = fresh_channel();
        let audit = NullAuditHook;
        let ctx = make_ctx(&channel, &audit);
        // No store set.
        let t = GraphQueryTool::new();
        assert!(matches!(
            t.execute(json!({ "start": "x" }), &ctx).await,
            ToolOutcome::Failed(_)
        ));
        // Store but empty `start`.
        assert!(t.set_store(store_with_chain().await).is_ok());
        assert!(matches!(
            t.execute(json!({ "start": "  " }), &ctx).await,
            ToolOutcome::Failed(_)
        ));
    }
}
