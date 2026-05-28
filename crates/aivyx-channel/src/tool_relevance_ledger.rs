//! Phase 116 Task 3 — Persistent tool/skill relevance ledger.
//!
//! Persistent map of `keyword_key → per-surface outcome rows`.
//! Phase 116 Task 4 calls `record_outcome` after each turn's
//! tool/skill calls finalize; Phase 116 Task 5 calls
//! `lookup` from the system-prompt assembly to render the
//! `## Tools recently used for similar tasks` section.
//!
//! ## Storage shape
//!
//! One key per keyword_key (Q1a string from
//! `aivyx_core::relevance::keyword_key`). Value is a
//! `RelevanceEntry` whose `outcomes` vec carries per-`(kind,
//! identifier)` running counts. Updates rewrite the whole
//! entry (cheap — entries are bounded by the per-keyword tool
//! universe, typically <20 rows).
//!
//! ## Why no decay (yet)
//!
//! Phase 82 / Phase 83 ledgers carry EWMA decay because they
//! track long-running preference signals. The Phase 116 ledger
//! is structurally bounded by the operator's actual tool/skill
//! set + the keyword diversity of their use cases. A decay
//! pass could ship as a follow-on if growth becomes an issue
//! in practice; Phase 116 keeps the surface small.
//!
//! ## Operator-readability
//!
//! Entries are JSON-serialized for diagnostic dumpability via
//! a future `aivyx tool-relevance dump` CLI (deferred — the
//! Phase 116 surface is the system-prompt section, which
//! covers operator-readability through the LLM's prompt
//! view). The ledger is encrypted at rest via
//! `KeyDomain::ToolRelevanceLedger`.

use serde::{Deserialize, Serialize};

use aivyx_storage::DomainHandle;

/// Phase 116 — which surface an outcome row covers. The
/// system-prompt assembly renders tools and skills in
/// separate sub-sections so the LLM can read each at a
/// glance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelevanceSurfaceKind {
    /// A substrate-tool call (e.g. `fs.read`, `web.fetch`).
    Tool,
    /// A LearnedSkill invocation via `skills.invoke`.
    Skill,
}

impl RelevanceSurfaceKind {
    pub fn label(&self) -> &'static str {
        match self {
            RelevanceSurfaceKind::Tool => "tool",
            RelevanceSurfaceKind::Skill => "skill",
        }
    }
}

/// Phase 116 — one outcome row per `(surface_kind,
/// identifier)` under a given keyword key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutcomeRow {
    pub surface_kind: RelevanceSurfaceKind,
    pub identifier: String,
    pub success_count: u32,
    pub failure_count: u32,
    pub last_seen_unix_ms: u64,
}

impl OutcomeRow {
    pub fn total(&self) -> u32 {
        self.success_count + self.failure_count
    }
}

/// Phase 116 — the value stored under a keyword_key.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RelevanceEntry {
    pub outcomes: Vec<OutcomeRow>,
}

#[derive(Debug, thiserror::Error)]
pub enum ToolRelevanceLedgerError {
    #[error("tool-relevance ledger storage error: {0}")]
    Storage(String),
    #[error("tool-relevance ledger encode error: {0}")]
    Encode(String),
    #[error("tool-relevance ledger decode error: {0}")]
    Decode(String),
}

/// Phase 116 — persistent tool/skill relevance ledger.
///
/// Thin wrapper over a `DomainHandle` from
/// `KeyDomain::ToolRelevanceLedger`. Methods are async to
/// match the storage layer.
pub struct PersistentToolRelevanceLedger {
    handle: DomainHandle,
}

impl PersistentToolRelevanceLedger {
    pub fn new(handle: DomainHandle) -> Self {
        PersistentToolRelevanceLedger { handle }
    }

    /// Record a tool/skill outcome under the given keyword
    /// key. Idempotent at the `(surface_kind, identifier)`
    /// level: a second call with the same triple accumulates
    /// counters; a new identifier appends a new row.
    pub async fn record_outcome(
        &self,
        keyword_key: &str,
        surface_kind: RelevanceSurfaceKind,
        identifier: &str,
        was_success: bool,
        now_unix_ms: u64,
    ) -> Result<(), ToolRelevanceLedgerError> {
        let mut entry = self.lookup(keyword_key).await?.unwrap_or_default();
        let existing = entry.outcomes.iter_mut().find(|r| {
            r.surface_kind == surface_kind && r.identifier == identifier
        });
        match existing {
            Some(row) => {
                if was_success {
                    row.success_count = row.success_count.saturating_add(1);
                } else {
                    row.failure_count = row.failure_count.saturating_add(1);
                }
                row.last_seen_unix_ms = now_unix_ms;
            }
            None => {
                entry.outcomes.push(OutcomeRow {
                    surface_kind,
                    identifier: identifier.to_string(),
                    success_count: if was_success { 1 } else { 0 },
                    failure_count: if was_success { 0 } else { 1 },
                    last_seen_unix_ms: now_unix_ms,
                });
            }
        }
        self.put(keyword_key, &entry).await
    }

    /// Fetch the relevance entry for a keyword key. Returns
    /// `None` when no prior outcome has been recorded under
    /// that key.
    pub async fn lookup(
        &self,
        keyword_key: &str,
    ) -> Result<Option<RelevanceEntry>, ToolRelevanceLedgerError> {
        let bytes = self
            .handle
            .get(keyword_key.as_bytes())
            .await
            .map_err(|e| ToolRelevanceLedgerError::Storage(e.to_string()))?;
        let Some(bytes) = bytes else {
            return Ok(None);
        };
        let entry: RelevanceEntry = serde_json::from_slice(&bytes)
            .map_err(|e| ToolRelevanceLedgerError::Decode(e.to_string()))?;
        Ok(Some(entry))
    }

    async fn put(
        &self,
        keyword_key: &str,
        entry: &RelevanceEntry,
    ) -> Result<(), ToolRelevanceLedgerError> {
        let bytes = serde_json::to_vec(entry)
            .map_err(|e| ToolRelevanceLedgerError::Encode(e.to_string()))?;
        self.handle
            .put(keyword_key.as_bytes(), &bytes)
            .await
            .map_err(|e| ToolRelevanceLedgerError::Storage(e.to_string()))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use aivyx_crypto::MasterKey;
    use aivyx_storage::{KeyDomain, RedbStorage, Storage, StorageConfig};

    async fn scratch_ledger()
    -> (std::path::PathBuf, PersistentToolRelevanceLedger) {
        let parent = std::env::temp_dir().join(format!(
            "aivyx-phase116-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&parent).expect("scratch dir");
        let storage: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(parent.join("store.redb")),
            MasterKey::from_raw([42u8; 32]),
        )
        .await
        .expect("scratch storage");
        let handle = storage.domain(KeyDomain::ToolRelevanceLedger);
        (parent, PersistentToolRelevanceLedger::new(handle))
    }

    #[tokio::test]
    async fn record_then_lookup_round_trips_single_success() {
        let (_dir, ledger) = scratch_ledger().await;
        ledger
            .record_outcome(
                "code|rust",
                RelevanceSurfaceKind::Tool,
                "fs.read",
                true,
                1_000_000,
            )
            .await
            .expect("record");
        let entry =
            ledger.lookup("code|rust").await.expect("ok").expect("present");
        assert_eq!(entry.outcomes.len(), 1);
        assert_eq!(entry.outcomes[0].identifier, "fs.read");
        assert_eq!(entry.outcomes[0].success_count, 1);
        assert_eq!(entry.outcomes[0].failure_count, 0);
        assert_eq!(entry.outcomes[0].last_seen_unix_ms, 1_000_000);
    }

    #[tokio::test]
    async fn multiple_outcomes_for_same_tool_accumulate_counters() {
        let (_dir, ledger) = scratch_ledger().await;
        ledger
            .record_outcome(
                "code|rust",
                RelevanceSurfaceKind::Tool,
                "fs.read",
                true,
                1_000,
            )
            .await
            .unwrap();
        ledger
            .record_outcome(
                "code|rust",
                RelevanceSurfaceKind::Tool,
                "fs.read",
                true,
                2_000,
            )
            .await
            .unwrap();
        ledger
            .record_outcome(
                "code|rust",
                RelevanceSurfaceKind::Tool,
                "fs.read",
                false,
                3_000,
            )
            .await
            .unwrap();
        let entry =
            ledger.lookup("code|rust").await.unwrap().unwrap();
        assert_eq!(entry.outcomes.len(), 1);
        let row = &entry.outcomes[0];
        assert_eq!(row.success_count, 2);
        assert_eq!(row.failure_count, 1);
        assert_eq!(row.total(), 3);
        assert_eq!(row.last_seen_unix_ms, 3_000);
    }

    #[tokio::test]
    async fn different_identifiers_get_separate_rows() {
        let (_dir, ledger) = scratch_ledger().await;
        ledger
            .record_outcome(
                "key",
                RelevanceSurfaceKind::Tool,
                "fs.read",
                true,
                100,
            )
            .await
            .unwrap();
        ledger
            .record_outcome(
                "key",
                RelevanceSurfaceKind::Tool,
                "web.fetch",
                true,
                200,
            )
            .await
            .unwrap();
        ledger
            .record_outcome(
                "key",
                RelevanceSurfaceKind::Skill,
                "research-topic",
                true,
                300,
            )
            .await
            .unwrap();
        let entry = ledger.lookup("key").await.unwrap().unwrap();
        assert_eq!(entry.outcomes.len(), 3);
        let identifiers: Vec<&str> =
            entry.outcomes.iter().map(|r| r.identifier.as_str()).collect();
        assert!(identifiers.contains(&"fs.read"));
        assert!(identifiers.contains(&"web.fetch"));
        assert!(identifiers.contains(&"research-topic"));
    }

    #[tokio::test]
    async fn tool_and_skill_with_same_identifier_are_separate_rows() {
        let (_dir, ledger) = scratch_ledger().await;
        // Hypothetical case: a tool and a skill both named
        // "research". The surface_kind discriminator must keep
        // them separate.
        ledger
            .record_outcome(
                "k",
                RelevanceSurfaceKind::Tool,
                "research",
                true,
                100,
            )
            .await
            .unwrap();
        ledger
            .record_outcome(
                "k",
                RelevanceSurfaceKind::Skill,
                "research",
                true,
                200,
            )
            .await
            .unwrap();
        let entry = ledger.lookup("k").await.unwrap().unwrap();
        assert_eq!(entry.outcomes.len(), 2);
    }

    #[tokio::test]
    async fn different_keyword_keys_isolate_entries() {
        let (_dir, ledger) = scratch_ledger().await;
        ledger
            .record_outcome(
                "alpha",
                RelevanceSurfaceKind::Tool,
                "fs.read",
                true,
                100,
            )
            .await
            .unwrap();
        ledger
            .record_outcome(
                "beta",
                RelevanceSurfaceKind::Tool,
                "fs.read",
                false,
                200,
            )
            .await
            .unwrap();
        let a = ledger.lookup("alpha").await.unwrap().unwrap();
        let b = ledger.lookup("beta").await.unwrap().unwrap();
        assert_eq!(a.outcomes[0].success_count, 1);
        assert_eq!(a.outcomes[0].failure_count, 0);
        assert_eq!(b.outcomes[0].success_count, 0);
        assert_eq!(b.outcomes[0].failure_count, 1);
    }

    #[tokio::test]
    async fn lookup_for_unknown_key_returns_none() {
        let (_dir, ledger) = scratch_ledger().await;
        assert!(ledger.lookup("never-recorded").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn entries_persist_across_ledger_reopens() {
        let parent = std::env::temp_dir().join(format!(
            "aivyx-phase116-persist-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&parent).expect("scratch dir");
        let path = parent.join("store.redb");

        // First open: record an outcome.
        {
            let storage: Arc<dyn Storage> = RedbStorage::open(
                StorageConfig::new(path.clone()),
                MasterKey::from_raw([0x11u8; 32]),
            )
            .await
            .expect("scratch storage");
            let ledger = PersistentToolRelevanceLedger::new(
                storage.domain(KeyDomain::ToolRelevanceLedger),
            );
            ledger
                .record_outcome(
                    "persist-key",
                    RelevanceSurfaceKind::Tool,
                    "fs.read",
                    true,
                    42,
                )
                .await
                .unwrap();
        }

        // Second open: must see the same entry.
        {
            let storage: Arc<dyn Storage> = RedbStorage::open(
                StorageConfig::new(path),
                MasterKey::from_raw([0x11u8; 32]),
            )
            .await
            .expect("reopen storage");
            let ledger = PersistentToolRelevanceLedger::new(
                storage.domain(KeyDomain::ToolRelevanceLedger),
            );
            let entry =
                ledger.lookup("persist-key").await.unwrap().unwrap();
            assert_eq!(entry.outcomes[0].identifier, "fs.read");
            assert_eq!(entry.outcomes[0].success_count, 1);
            assert_eq!(entry.outcomes[0].last_seen_unix_ms, 42);
        }
    }

    #[test]
    fn surface_kind_label_stable() {
        assert_eq!(RelevanceSurfaceKind::Tool.label(), "tool");
        assert_eq!(RelevanceSurfaceKind::Skill.label(), "skill");
    }

    #[test]
    fn surface_kind_serializes_as_snake_case_string() {
        let s = serde_json::to_string(&RelevanceSurfaceKind::Tool).unwrap();
        assert_eq!(s, "\"tool\"");
        let s = serde_json::to_string(&RelevanceSurfaceKind::Skill).unwrap();
        assert_eq!(s, "\"skill\"");
    }

    #[test]
    fn outcome_row_total_sums_success_and_failure() {
        let row = OutcomeRow {
            surface_kind: RelevanceSurfaceKind::Tool,
            identifier: "x".into(),
            success_count: 5,
            failure_count: 3,
            last_seen_unix_ms: 0,
        };
        assert_eq!(row.total(), 8);
    }
}
