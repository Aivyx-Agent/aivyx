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

    /// Phase 119 Task 6 — operator-inspection iteration. Returns
    /// every `(keyword_key, RelevanceEntry)` pair in the ledger,
    /// optionally filtered to a single keyword key for the
    /// `aivyx tool-relevance dump --keyword-key <key>` flow.
    ///
    /// The storage layer's `scan_prefix` with an empty prefix walks
    /// every row in the domain (same path
    /// `scan_prefix_empty_prefix_returns_every_key_in_domain` test
    /// pins in aivyx-storage). Keys are stored as UTF-8 keyword-
    /// key bytes; we decode each one for the rendered surface.
    ///
    /// Returns rows sorted ascending by `keyword_key` (matches the
    /// underlying redb byte-ordering — operator-stable for the
    /// dump table). Malformed rows (corrupt JSON, invalid UTF-8
    /// keys) are skipped with a stderr warn; the ledger contract
    /// is best-effort inspection, not strict-decode.
    pub async fn list_all_entries(
        &self,
        keyword_key_filter: Option<&str>,
    ) -> Result<Vec<(String, RelevanceEntry)>, ToolRelevanceLedgerError> {
        let prefix: Vec<u8> = match keyword_key_filter {
            Some(k) => k.as_bytes().to_vec(),
            None => Vec::new(),
        };
        let rows = self
            .handle
            .scan_prefix(&prefix)
            .await
            .map_err(|e| ToolRelevanceLedgerError::Storage(e.to_string()))?;
        let mut out: Vec<(String, RelevanceEntry)> = Vec::with_capacity(rows.len());
        for (key_bytes, value_bytes) in rows {
            let Ok(key) = std::str::from_utf8(&key_bytes) else {
                eprintln!(
                    "aivyx tool-relevance: skipping non-UTF8 keyword key in ledger"
                );
                continue;
            };
            // With a non-empty filter, scan_prefix returns rows where
            // the key STARTS WITH the filter — for our exact-match
            // semantics we require the keys to be equal.
            if let Some(filter) = keyword_key_filter {
                if key != filter {
                    continue;
                }
            }
            let entry: RelevanceEntry = match serde_json::from_slice(&value_bytes) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!(
                        "aivyx tool-relevance: skipping malformed entry for \
                         keyword key `{key}`: {e}"
                    );
                    continue;
                }
            };
            out.push((key.to_string(), entry));
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Phase 116 Task 4 — Outcome-recording helper
// ---------------------------------------------------------------------------

/// Phase 116 — record per-turn tool outcomes into the
/// relevance ledger from a slice of post-turn audit entries.
/// Pure-ish helper extracted from the daemon turn loop so it
/// can be unit-tested without the full daemon.
///
/// Walks the entries, picks out `ToolCall` variants, and
/// records one outcome per call using the call's
/// `scope_used.base()` as the identifier (operator-readable;
/// matches the `ToolDescriptor.scope_base` convention).
///
/// Skills tracking (Q3a's "tools AND skills together") is a
/// Phase-116-Task-4-internal deferral: the audit chain hashes
/// `skills.invoke`'s input so the skill name isn't recoverable
/// at recording time without a separate capture path. The
/// skills surface stays on the ledger schema (`Skill` variant
/// of `RelevanceSurfaceKind`) so a follow-on can ship it
/// without a schema migration.
///
/// Failure-isolated: any single record_outcome failure is
/// swallowed (logged at WARN by the caller); a corrupt
/// ledger row degrades only the relevance hint, never the
/// turn itself.
pub async fn record_turn_outcomes(
    ledger: &PersistentToolRelevanceLedger,
    keyword_key: &str,
    audit_entries: &[aivyx_audit::SignedEntry],
    now_unix_ms: u64,
) {
    if keyword_key.is_empty() {
        // No usable keyword signal from the turn — nothing to
        // index against. Skip the recording pass.
        return;
    }
    for entry in audit_entries {
        match &entry.event {
            aivyx_audit::AuditEvent::ToolCall {
                scope_used,
                outcome,
                ..
            } => {
                // Tool identifier = the scope base (e.g.
                // "fs.read", "net.fetch"). Operator-readable,
                // deterministic.
                let identifier = scope_used.base().to_string();
                let was_success = matches!(
                    outcome,
                    aivyx_core::ToolOutcomeSummary::Completed { .. }
                );
                if let Err(e) = ledger
                    .record_outcome(
                        keyword_key,
                        RelevanceSurfaceKind::Tool,
                        &identifier,
                        was_success,
                        now_unix_ms,
                    )
                    .await
                {
                    eprintln!(
                        "aivyx tool-relevance: record_outcome failed for \
                         tool ({keyword_key}, {identifier}): {e}"
                    );
                }
            }
            aivyx_audit::AuditEvent::SkillInvocation {
                skill_name, ..
            } => {
                // Phase 117 — `skills.invoke` emits this entry
                // alongside its regular ToolCall entry; the
                // skill_name is the operator-readable identifier
                // for the Skill subsection of the relevance
                // section. Always recorded as success because
                // `skills.invoke` only emits SkillInvocation
                // on the Completed branch.
                if let Err(e) = ledger
                    .record_outcome(
                        keyword_key,
                        RelevanceSurfaceKind::Skill,
                        skill_name,
                        true,
                        now_unix_ms,
                    )
                    .await
                {
                    eprintln!(
                        "aivyx tool-relevance: record_outcome failed for \
                         skill ({keyword_key}, {skill_name}): {e}"
                    );
                }
            }
            _ => continue,
        }
    }
}

// ---------------------------------------------------------------------------
// Phase 116 Task 5 — System-prompt section rendering
// ---------------------------------------------------------------------------

/// Phase 116 — render the `## Tools recently used for
/// similar tasks` system-prompt section for a given
/// keyword_key. Returns the rendered section (with trailing
/// newline) or the empty string when:
///
/// - the ledger has no entry for the key, OR
/// - no row in the entry has total outcomes >=
///   `min_outcomes_to_show`.
///
/// The section format mirrors the open doc's spec — keyword
/// banner + Tools subsection + Skills subsection (only when
/// non-empty). Within each subsection, rows are sorted by
/// success count descending (ties broken by failure count
/// ascending, then identifier ascending). Top-K rows per
/// subsection.
///
/// `keyword_key_display` is the same key but rendered for
/// human reading — typically `"code, rust"` (comma-joined)
/// rather than `"code|rust"` (the storage key).
pub async fn render_relevance_section(
    ledger: &PersistentToolRelevanceLedger,
    keyword_key: &str,
    keyword_key_display: &str,
    min_outcomes_to_show: u32,
    top_k_per_subsection: usize,
) -> String {
    if keyword_key.is_empty() {
        return String::new();
    }
    let Ok(Some(entry)) = ledger.lookup(keyword_key).await else {
        return String::new();
    };
    let mut rows: Vec<&OutcomeRow> = entry
        .outcomes
        .iter()
        .filter(|r| r.total() >= min_outcomes_to_show)
        .collect();
    if rows.is_empty() {
        return String::new();
    }
    // Sort: successes desc, then failures asc, then identifier asc.
    rows.sort_by(|a, b| {
        b.success_count
            .cmp(&a.success_count)
            .then_with(|| a.failure_count.cmp(&b.failure_count))
            .then_with(|| a.identifier.cmp(&b.identifier))
    });

    let tools: Vec<&OutcomeRow> = rows
        .iter()
        .filter(|r| r.surface_kind == RelevanceSurfaceKind::Tool)
        .take(top_k_per_subsection)
        .copied()
        .collect();
    let skills: Vec<&OutcomeRow> = rows
        .iter()
        .filter(|r| r.surface_kind == RelevanceSurfaceKind::Skill)
        .take(top_k_per_subsection)
        .copied()
        .collect();

    if tools.is_empty() && skills.is_empty() {
        return String::new();
    }

    let mut out = String::from("## Tools recently used for similar tasks\n\n");
    if !keyword_key_display.is_empty() {
        out.push_str(&format!("Based on keywords: {keyword_key_display}\n\n"));
    }
    if !tools.is_empty() {
        out.push_str("Tools:\n");
        for r in tools {
            out.push_str(&format!(
                "- {}: {} successes, {} failures\n",
                r.identifier, r.success_count, r.failure_count,
            ));
        }
        if !skills.is_empty() {
            out.push('\n');
        }
    }
    if !skills.is_empty() {
        out.push_str("Skills:\n");
        for r in skills {
            out.push_str(&format!(
                "- {}: {} invocations ({} successes, {} failures)\n",
                r.identifier,
                r.total(),
                r.success_count,
                r.failure_count,
            ));
        }
    }
    out
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

    // ----- record_turn_outcomes (Task 4 wiring) -----

    fn tool_call_entry(
        seq: u64,
        scope_str: &str,
        was_success: bool,
    ) -> aivyx_audit::SignedEntry {
        use aivyx_audit::{AuditEvent, SignedEntry};
        use aivyx_core::{ToolOutcomeSummary, VerificationSummary};
        use aivyx_capability::Scope;
        use std::time::{Duration, SystemTime};
        let outcome = if was_success {
            ToolOutcomeSummary::Completed {
                verified: VerificationSummary::NotApplicable,
            }
        } else {
            ToolOutcomeSummary::Failed
        };
        SignedEntry {
            seq,
            appended_at: SystemTime::now(),
            event: AuditEvent::ToolCall {
                turn_id: aivyx_core::TurnId::new(),
                tool_id: aivyx_core::ToolId::new(),
                scope_used: Scope::parse(scope_str).expect("valid scope"),
                input_hash: [0u8; 32],
                outcome,
                duration: Duration::from_millis(10),
            },
            mac: [0u8; 32],
            prev_mac: [0u8; 32],
        }
    }

    fn turn_started_entry(seq: u64) -> aivyx_audit::SignedEntry {
        use aivyx_audit::{
            AuditEvent, SignedEntry, TrustTierSummary,
        };
        use aivyx_capability::CapabilitySet;
        use aivyx_core::ChannelPlatform;
        use std::time::SystemTime;
        SignedEntry {
            seq,
            appended_at: SystemTime::now(),
            event: AuditEvent::TurnStarted {
                turn_id: aivyx_core::TurnId::new(),
                session_id: aivyx_core::SessionId::new(),
                channel: ChannelPlatform::Local,
                trust_tier: TrustTierSummary::Trusted,
                effective_capabilities: CapabilitySet::empty(),
            },
            mac: [0u8; 32],
            prev_mac: [0u8; 32],
        }
    }

    #[tokio::test]
    async fn record_turn_outcomes_picks_tool_call_entries_only() {
        let (_dir, ledger) = scratch_ledger().await;
        let entries = vec![
            turn_started_entry(0),
            tool_call_entry(1, "memory.read", true),
            tool_call_entry(2, "llm.call", true),
            tool_call_entry(3, "memory.read", false),
        ];
        record_turn_outcomes(&ledger, "code|rust", &entries, 1000).await;

        let entry = ledger.lookup("code|rust").await.unwrap().unwrap();
        assert_eq!(entry.outcomes.len(), 2); // memory.read, llm.call

        let mem_read = entry
            .outcomes
            .iter()
            .find(|r| r.identifier == "memory.read")
            .expect("memory.read row");
        assert_eq!(mem_read.success_count, 1);
        assert_eq!(mem_read.failure_count, 1);

        let llm_call = entry
            .outcomes
            .iter()
            .find(|r| r.identifier == "llm.call")
            .expect("llm.call row");
        assert_eq!(llm_call.success_count, 1);
        assert_eq!(llm_call.failure_count, 0);
    }

    #[tokio::test]
    async fn record_turn_outcomes_skips_recording_for_empty_keyword_key() {
        let (_dir, ledger) = scratch_ledger().await;
        let entries = vec![tool_call_entry(0, "memory.read", true)];
        record_turn_outcomes(&ledger, "", &entries, 1000).await;
        // Empty key → no ledger write → no entry under "".
        assert!(ledger.lookup("").await.unwrap().is_none());
    }

    fn skill_invocation_entry(
        seq: u64,
        skill_name: &str,
    ) -> aivyx_audit::SignedEntry {
        use aivyx_audit::{AuditEvent, SignedEntry};
        use std::time::SystemTime;
        SignedEntry {
            seq,
            appended_at: SystemTime::now(),
            event: AuditEvent::SkillInvocation {
                turn_id: aivyx_core::TurnId::new(),
                session_id: aivyx_core::SessionId::new(),
                skill_name: skill_name.to_string(),
            },
            mac: [0u8; 32],
            prev_mac: [0u8; 32],
        }
    }

    #[tokio::test]
    async fn record_turn_outcomes_records_skill_invocations() {
        let (_dir, ledger) = scratch_ledger().await;
        let entries = vec![
            turn_started_entry(0),
            tool_call_entry(1, "memory.read", true),
            skill_invocation_entry(2, "research-topic"),
            tool_call_entry(3, "llm.call", true),
            skill_invocation_entry(4, "research-topic"),
            skill_invocation_entry(5, "summarize-pdf"),
        ];
        record_turn_outcomes(&ledger, "code|research", &entries, 1000).await;

        let entry = ledger.lookup("code|research").await.unwrap().unwrap();
        // 2 tools + 2 skills = 4 rows.
        assert_eq!(entry.outcomes.len(), 4);

        let research = entry
            .outcomes
            .iter()
            .find(|r| {
                r.surface_kind == RelevanceSurfaceKind::Skill
                    && r.identifier == "research-topic"
            })
            .expect("research-topic skill row");
        assert_eq!(research.success_count, 2);

        let summarize = entry
            .outcomes
            .iter()
            .find(|r| {
                r.surface_kind == RelevanceSurfaceKind::Skill
                    && r.identifier == "summarize-pdf"
            })
            .expect("summarize-pdf skill row");
        assert_eq!(summarize.success_count, 1);
    }

    #[tokio::test]
    async fn record_turn_outcomes_no_tool_calls_writes_nothing() {
        let (_dir, ledger) = scratch_ledger().await;
        let entries = vec![turn_started_entry(0)];
        record_turn_outcomes(&ledger, "key", &entries, 1000).await;
        assert!(ledger.lookup("key").await.unwrap().is_none());
    }

    // ----- render_relevance_section (Task 5) -----

    #[tokio::test]
    async fn render_section_returns_empty_when_key_unknown() {
        let (_dir, ledger) = scratch_ledger().await;
        let s = render_relevance_section(
            &ledger,
            "never-seen-key",
            "never seen",
            1,
            5,
        )
        .await;
        assert!(s.is_empty());
    }

    #[tokio::test]
    async fn render_section_returns_empty_when_empty_keyword_key() {
        let (_dir, ledger) = scratch_ledger().await;
        let s = render_relevance_section(&ledger, "", "", 1, 5).await;
        assert!(s.is_empty());
    }

    #[tokio::test]
    async fn render_section_filters_below_min_outcomes_threshold() {
        let (_dir, ledger) = scratch_ledger().await;
        // One successful call; min_outcomes_to_show = 2 → filtered out.
        ledger
            .record_outcome(
                "k",
                RelevanceSurfaceKind::Tool,
                "memory.read",
                true,
                100,
            )
            .await
            .unwrap();
        let s =
            render_relevance_section(&ledger, "k", "test", 2, 5).await;
        assert!(s.is_empty(), "below-threshold should render empty: {s}");
    }

    #[tokio::test]
    async fn render_section_includes_tools_and_skills_subsections() {
        let (_dir, ledger) = scratch_ledger().await;
        // Tool: memory.read 3x, all success.
        for _ in 0..3 {
            ledger
                .record_outcome(
                    "k",
                    RelevanceSurfaceKind::Tool,
                    "memory.read",
                    true,
                    100,
                )
                .await
                .unwrap();
        }
        // Skill: research-topic 2 successes + 1 failure.
        for ok in [true, true, false] {
            ledger
                .record_outcome(
                    "k",
                    RelevanceSurfaceKind::Skill,
                    "research-topic",
                    ok,
                    200,
                )
                .await
                .unwrap();
        }
        let s =
            render_relevance_section(&ledger, "k", "code, rust", 2, 5).await;
        assert!(s.contains("## Tools recently used for similar tasks"));
        assert!(s.contains("Based on keywords: code, rust"));
        assert!(s.contains("Tools:"));
        assert!(s.contains("memory.read: 3 successes, 0 failures"));
        assert!(s.contains("Skills:"));
        assert!(s.contains("research-topic: 3 invocations (2 successes, 1 failures)"));
    }

    #[tokio::test]
    async fn render_section_sorts_by_success_count_desc() {
        let (_dir, ledger) = scratch_ledger().await;
        // Three tools with different success counts.
        for _ in 0..5 {
            ledger
                .record_outcome("k", RelevanceSurfaceKind::Tool, "a.tool", true, 0)
                .await
                .unwrap();
        }
        for _ in 0..2 {
            ledger
                .record_outcome("k", RelevanceSurfaceKind::Tool, "b.tool", true, 0)
                .await
                .unwrap();
        }
        for _ in 0..3 {
            ledger
                .record_outcome("k", RelevanceSurfaceKind::Tool, "c.tool", true, 0)
                .await
                .unwrap();
        }
        let s = render_relevance_section(&ledger, "k", "x", 1, 5).await;
        let a_pos = s.find("a.tool").unwrap();
        let b_pos = s.find("b.tool").unwrap();
        let c_pos = s.find("c.tool").unwrap();
        assert!(a_pos < c_pos, "a (5 successes) before c (3 successes)");
        assert!(c_pos < b_pos, "c (3 successes) before b (2 successes)");
    }

    #[tokio::test]
    async fn render_section_top_k_truncates() {
        let (_dir, ledger) = scratch_ledger().await;
        for i in 0..10 {
            ledger
                .record_outcome(
                    "k",
                    RelevanceSurfaceKind::Tool,
                    &format!("tool.{i}"),
                    true,
                    0,
                )
                .await
                .unwrap();
        }
        // top_k_per_subsection = 3 → only 3 rows in the Tools list.
        let s = render_relevance_section(&ledger, "k", "x", 1, 3).await;
        let line_count = s.lines().filter(|l| l.starts_with("- ")).count();
        assert_eq!(line_count, 3, "expected top-3 tools, got: {s}");
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
