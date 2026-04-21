//! Channel-layer [`PruneSink`] implementation backed by [`Memory`].
//!
//! When the LLM planner prunes messages to stay within the context
//! window budget, it invokes this sink to persist a summary of the
//! pruned content under `context:pruned:<session_id>` in the memory
//! subsystem. This allows the agent (or a reflection tool) to later
//! recall what was discussed before pruning.

use std::sync::Arc;

use async_trait::async_trait;
use aivyx_core::llm_planner::PruneSink;
use aivyx_core::SessionId;
use aivyx_memory::Memory;

/// Persists pruned-context summaries to memory under a per-session
/// topic. The topic format is `context:pruned:<session_id>`.
pub struct MemoryPruneSink {
    memory: Arc<dyn Memory>,
}

impl MemoryPruneSink {
    pub fn new(memory: Arc<dyn Memory>) -> Self {
        MemoryPruneSink { memory }
    }
}

#[async_trait]
impl PruneSink for MemoryPruneSink {
    async fn on_prune(&self, session_id: SessionId, pruned_count: usize, summary: &str) {
        let topic = format!("context:pruned:{session_id}");
        let body = format!(
            "Pruned {pruned_count} messages from conversation history.\n\n{summary}"
        );
        if let Err(e) = self.memory.put(&topic, &body).await {
            eprintln!("prune-sink: failed to persist pruned context: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_memory::InMemoryMemory;

    #[tokio::test]
    async fn prune_sink_persists_to_memory() {
        let mem: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
        let sink = MemoryPruneSink::new(Arc::clone(&mem));
        let sid = SessionId::new();

        sink.on_prune(sid, 5, "[user] hello\n[assistant] hi").await;

        let topic = format!("context:pruned:{sid}");
        let entries = mem.get_recent(&topic, 10).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].body.contains("Pruned 5 messages"));
        assert!(entries[0].body.contains("[user] hello"));
    }

    #[tokio::test]
    async fn prune_sink_accumulates_entries() {
        let mem: Arc<dyn Memory> = Arc::new(InMemoryMemory::new());
        let sink = MemoryPruneSink::new(Arc::clone(&mem));
        let sid = SessionId::new();

        sink.on_prune(sid, 3, "batch-1").await;
        sink.on_prune(sid, 7, "batch-2").await;

        let topic = format!("context:pruned:{sid}");
        let entries = mem.get_recent(&topic, 10).await.unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].body.contains("batch-2")); // newest first
        assert!(entries[1].body.contains("batch-1"));
    }
}
