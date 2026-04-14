//! # aivyx-memory
//!
//! Memory substrate for Aivyx agents. Implements the data model and
//! async trait that back the `memory.read`, `memory.write`, and
//! `memory.forget` tools (Phase 6 tasks 3–4). This crate is deliberately
//! substrate-only: the `Tool` impls themselves live in a `tools`
//! sub-module added in task 3, and the redb-backed implementation
//! (`RedbMemory`) lands in task 2. Task 1 ships the trait and a
//! deterministic in-process fake so tool tests can run at microsecond
//! speed without paying the crypto/redb cost.
//!
//! ## D1 contract recap
//!
//! Per DESIGN.md Deliverable 1, memory is **a tool, not an ambient
//! system**. There is no hidden "memory injection" at turn start — every
//! memory access is an explicit, scope-checked, audited tool call. This
//! crate holds the substrate the tools will wrap; the
//! capability-typed surface (scopes like `memory.read:session:<id>`)
//! lives in `aivyx-capability`, and the per-call scope gate lives in
//! `aivyx-core`'s turn loop.
//!
//! ## Model
//!
//! A memory entry is a small structured record tagged with a **topic**
//! (a non-empty UTF-8 string the agent picks when writing) and a
//! **body** (UTF-8 text). Every entry also carries a monotonic
//! per-substrate **sequence number** assigned at write time and a
//! **wall-clock timestamp** (`created_at_secs`, seconds since UNIX
//! epoch) captured by the substrate, not by the caller.
//!
//! Recall is **topic-scoped** and **sequence-ordered**: given a topic
//! and a limit N, `get_recent` returns the N entries with that topic
//! sorted by sequence descending (newest first). Phase 6 Q2 resolved
//! this as "by insertion sequence, not by wall clock" — agent recall is
//! naturally about "what did I say most recently in this conversation,"
//! which is an insertion ordering. Timestamps are kept for display and
//! future GC decisions, not for ordering.
//!
//! ## Q1 — entry encoding
//!
//! Entries round-trip as **JSON** via `serde_json`. Alternatives
//! (bincode, postcard, packed) were considered at phase entry and
//! rejected because (a) the workspace already pays for `serde_json` in
//! every crate that builds a tool descriptor or an audit envelope,
//! (b) the substrate below this layer is already encrypted so
//! "compact" doesn't buy read-path performance, and (c) human-readable
//! at-rest bytes make it cheap to drop down to `redb::open` during
//! debugging. The trade is ~30% storage overhead vs. postcard, which
//! we pay gladly.
//!
//! ## Deliberately out of scope for this crate
//!
//! - **No `Tool` impls.** Those land in task 3 in a new `tools`
//!   sub-module and depend on this trait.
//! - **No scope checks.** Scope enforcement is the tool wrapper's
//!   job, not the substrate's. The substrate is a trusted inner layer;
//!   callers that hold an `Arc<dyn Memory>` have already been
//!   authorized by the turn loop's scope gate.
//! - **No `list_topics`.** Agents query by topic they already know
//!   (either one they picked when writing, or a reserved sentinel —
//!   Phase 6 Q3 will resolve whether `"*"` means "all topics" in the
//!   tool-level API; the substrate itself is topic-scoped so this
//!   decision lives one layer up).
//!
//! ## Implementations
//!
//! - [`InMemoryMemory`] — deterministic in-process fake, no
//!   persistence. Used by this crate's unit tests and by task 3's
//!   tool-wrapper tests so they can stay at microsecond speed.
//! - [`RedbMemory`] — redb-backed, AEAD-encrypted, persistent.
//!   Task 2's deliverable. Wraps a
//!   `aivyx_storage::DomainHandle` for `KeyDomain::Memory` and
//!   seeds its monotonic sequence counter from any existing
//!   entries at construction time, so a restart preserves the
//!   invariant that no two entries ever share a `seq`.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

mod redb;
pub use crate::redb::RedbMemory;

mod tools;
pub use crate::tools::{
    MemoryForgetTool, MemoryReadTool, MemoryWriteTool, DEFAULT_MAX_PER_TOPIC,
};

/// A single memory record.
///
/// This is what `Memory::put` stores and what `Memory::get_recent`
/// returns. Fields are pub to keep round-trip asserts in tests concise;
/// construction by callers outside the crate goes through the substrate
/// (`put` takes a `topic` and `body` and fills in `seq` / `created_at`
/// internally), so there's no API surface to protect.
///
/// The on-disk form is the JSON serialization of this struct. If the
/// struct ever grows a non-optional field, task 2's `RedbMemory` will
/// need a migration — at which point the right play is to bump the
/// HKDF salt in `aivyx-crypto` (`"aivyx-v1-storage"` →
/// `"aivyx-v2-storage"`) and let the old values become cleanly
/// unreadable, exactly as noted in Phase 5's roadmap handoff.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryEntry {
    /// Non-empty UTF-8 topic chosen by the agent when writing. Used as
    /// the recall key; entries with the same topic come back together
    /// from `get_recent`.
    pub topic: String,
    /// UTF-8 body — whatever the agent asked to remember.
    pub body: String,
    /// Monotonic per-substrate insertion counter. First write gets
    /// `seq = 0`, second gets `seq = 1`, and so on. Sort key for
    /// `get_recent` (descending).
    pub seq: u64,
    /// Wall-clock seconds since UNIX epoch at the moment the substrate
    /// accepted the write. Captured by the substrate via
    /// `SystemTime::now()`; callers do not pass this in. Not used for
    /// ordering (see module doc, Q2 resolution) — kept for display and
    /// future TTL/GC decisions.
    pub created_at_secs: u64,
}

/// Errors the memory substrate can return.
///
/// Deliberately thin at this layer: the substrate distinguishes only
/// between "caller handed me bad input" (empty topic, zero limit) and
/// "the backing store broke" (only reachable from the real
/// `RedbMemory` in task 2). The `InMemoryMemory` fake never returns
/// `Backend`.
#[derive(Debug, Error)]
pub enum MemoryError {
    /// A caller passed an empty topic to `put`, `get_recent`, or
    /// `forget`. Topics are part of the recall contract; an empty
    /// topic would silently unify with a sentinel later.
    #[error("memory topic must be non-empty")]
    EmptyTopic,
    /// A caller passed `limit == 0` to `get_recent`. `limit` is a cap,
    /// not a filter; zero means "I don't want any results" which
    /// almost always indicates a caller bug rather than a real query.
    /// Returning an error here catches the mistake loudly instead of
    /// pretending everything is fine.
    #[error("memory get_recent limit must be > 0")]
    ZeroLimit,
    /// Entry JSON serialization or deserialization failed. Reaching
    /// this from `InMemoryMemory` would mean a type mismatch in
    /// `MemoryEntry` itself. Kept in the enum so task 2's `RedbMemory`
    /// can reuse the same variant when it decodes a payload from
    /// `DomainHandle::get`.
    #[error("memory entry encoding error: {0}")]
    Encoding(String),
    /// The backing store returned an error. Only reachable from the
    /// real `RedbMemory` impl landing in task 2; task 1's fake never
    /// produces this.
    #[error("memory backend error: {0}")]
    Backend(String),
}

/// The minimum memory substrate surface.
///
/// Three async methods: `put` (write one entry, returning its assigned
/// `seq`), `get_recent` (read the N newest entries for a topic), and
/// `forget` (delete every entry for a topic). No `list_topics`, no
/// cross-topic scans, no time-range queries — that's all deferred to
/// later phases if Phase 6's integration test shows something's
/// missing.
///
/// `Send + Sync` because this gets cloned into each turn's
/// `ToolContext`, same pattern as `Arc<dyn Storage>` and
/// `Arc<dyn LlmProvider>`. Implementors should expect to be called
/// from multiple tasks concurrently.
#[async_trait]
pub trait Memory: Send + Sync {
    /// Store one entry under `topic` with the given `body`. Returns
    /// the `seq` the substrate assigned (useful for tests; tool
    /// wrappers in task 3 may or may not expose it).
    ///
    /// Fails fast on empty topic.
    async fn put(&self, topic: &str, body: &str) -> Result<u64, MemoryError>;

    /// Return up to `limit` entries for `topic`, newest first (sorted
    /// by `seq` descending). Topics that have never been written
    /// return an empty `Vec`, not an error.
    ///
    /// Fails on empty topic or `limit == 0`.
    async fn get_recent(
        &self,
        topic: &str,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>, MemoryError>;

    /// Delete every entry under `topic`. Returns the number of entries
    /// deleted (0 is not an error — forgetting a topic that was never
    /// written is a no-op).
    ///
    /// Fails fast on empty topic.
    async fn forget(&self, topic: &str) -> Result<usize, MemoryError>;

    /// Walk every topic whose literal-byte name starts with
    /// `topic_prefix`, returning one `(topic, entries)` pair per
    /// matching topic. `entries` is sorted newest-first per topic
    /// (matching `get_recent`'s ordering), capped at `per_topic_limit`
    /// entries per topic.
    ///
    /// The substrate is **session-oblivious**: it walks raw topic
    /// strings and has no concept of what any particular prefix
    /// "means." Callers that want session-scoped cross-topic reads
    /// (Phase 10 Task 1's wildcard `memory.read`) pass the
    /// `\x01s\x01<session>\x01` namespace prefix from the tool
    /// layer, which guarantees every returned topic belongs to
    /// exactly that session.
    ///
    /// An empty `topic_prefix` is legal and means "every topic in
    /// the substrate" — but the capability layer (`aivyx-capability`)
    /// does not grant any wildcard scope whose prefix resolves to
    /// empty, so this is a substrate-level permissive behavior, not
    /// a tool-level one. Callers that care must enforce minimum
    /// prefix discipline themselves.
    ///
    /// `per_topic_limit == 0` is an error (same as
    /// `get_recent(_, 0)`), because a zero limit is almost always a
    /// caller bug.
    ///
    /// The iteration order of topics within the returned vector is
    /// **byte-lexicographic ascending**, which matches redb's
    /// underlying `scan_prefix` order. Callers that want a specific
    /// sort order must resort in the tool layer.
    async fn scan_prefix(
        &self,
        topic_prefix: &str,
        per_topic_limit: usize,
    ) -> Result<Vec<(String, Vec<MemoryEntry>)>, MemoryError>;
}

/// Deterministic in-process `Memory` implementation.
///
/// Backed by a `Mutex<State>` holding a topic → list mapping and a
/// monotonic counter. Used by this crate's own unit tests and — once
/// task 3 lands — by the `memory_tool_e2e.rs` integration test's
/// faster unit-test siblings that exercise the tool wrappers without
/// paying the redb/crypto cost.
///
/// Semantics are **exactly** what `RedbMemory` will ship in task 2:
/// same ordering (sequence descending), same empty-topic handling,
/// same `limit == 0` rejection. That equivalence is what makes it
/// safe for task 3's tool tests to use `InMemoryMemory` instead of
/// spinning up a real encrypted store — if the two diverge, task 2
/// is buggy, not this fake.
#[derive(Debug, Default)]
pub struct InMemoryMemory {
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    /// Topic → entries, oldest first within each topic.
    topics: BTreeMap<String, Vec<MemoryEntry>>,
    /// Monotonic insertion counter, shared across topics. Every `put`
    /// increments this and assigns the post-increment value to the new
    /// entry's `seq`. Global (not per-topic) so that two entries
    /// written under different topics still have totally-ordered
    /// sequence numbers — important for any future cross-topic
    /// tie-break logic.
    next_seq: u64,
}

impl InMemoryMemory {
    /// Construct an empty in-memory substrate.
    pub fn new() -> Self {
        InMemoryMemory::default()
    }

    /// Serialize an entry to JSON the same way task 2's `RedbMemory`
    /// will. Exposed at module scope so tests (and, eventually, task
    /// 2's round-trip tests) can share one encoding path.
    pub fn encode_entry(entry: &MemoryEntry) -> Result<Vec<u8>, MemoryError> {
        serde_json::to_vec(entry).map_err(|e| MemoryError::Encoding(e.to_string()))
    }

    /// Inverse of [`encode_entry`].
    pub fn decode_entry(bytes: &[u8]) -> Result<MemoryEntry, MemoryError> {
        serde_json::from_slice(bytes).map_err(|e| MemoryError::Encoding(e.to_string()))
    }
}

#[async_trait]
impl Memory for InMemoryMemory {
    async fn put(&self, topic: &str, body: &str) -> Result<u64, MemoryError> {
        if topic.is_empty() {
            return Err(MemoryError::EmptyTopic);
        }
        let mut state = self.state.lock().unwrap();
        let seq = state.next_seq;
        state.next_seq = state.next_seq.saturating_add(1);
        let entry = MemoryEntry {
            topic: topic.to_string(),
            body: body.to_string(),
            seq,
            created_at_secs: now_secs(),
        };
        // Round-trip through the real JSON encoder so the fake exercises
        // the same path the real impl will in task 2. Catches any future
        // mistake like adding a non-Serialize field to MemoryEntry
        // inside the fake's own test runs, not just in integration.
        let _ = InMemoryMemory::encode_entry(&entry)?;
        state
            .topics
            .entry(topic.to_string())
            .or_default()
            .push(entry);
        Ok(seq)
    }

    async fn get_recent(
        &self,
        topic: &str,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        if topic.is_empty() {
            return Err(MemoryError::EmptyTopic);
        }
        if limit == 0 {
            return Err(MemoryError::ZeroLimit);
        }
        let state = self.state.lock().unwrap();
        let Some(entries) = state.topics.get(topic) else {
            return Ok(Vec::new());
        };
        // Newest first. `entries` is stored oldest-first, so iterate in
        // reverse and take `limit`. Cloning is fine — memory bodies are
        // small by construction and this is a fake.
        Ok(entries.iter().rev().take(limit).cloned().collect())
    }

    async fn forget(&self, topic: &str) -> Result<usize, MemoryError> {
        if topic.is_empty() {
            return Err(MemoryError::EmptyTopic);
        }
        let mut state = self.state.lock().unwrap();
        Ok(state.topics.remove(topic).map(|v| v.len()).unwrap_or(0))
    }

    async fn scan_prefix(
        &self,
        topic_prefix: &str,
        per_topic_limit: usize,
    ) -> Result<Vec<(String, Vec<MemoryEntry>)>, MemoryError> {
        if per_topic_limit == 0 {
            return Err(MemoryError::ZeroLimit);
        }
        let state = self.state.lock().unwrap();
        let mut out: Vec<(String, Vec<MemoryEntry>)> = Vec::new();
        for (topic, entries) in state.topics.iter() {
            if !topic.starts_with(topic_prefix) {
                continue;
            }
            let newest_first: Vec<MemoryEntry> =
                entries.iter().rev().take(per_topic_limit).cloned().collect();
            out.push((topic.clone(), newest_first));
        }
        Ok(out)
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn put_then_get_recent_round_trips_single_entry() {
        let mem = InMemoryMemory::new();
        let seq = mem.put("notes", "favorite color is purple").await.unwrap();
        assert_eq!(seq, 0, "first put must get seq 0");

        let entries = mem.get_recent("notes", 10).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].topic, "notes");
        assert_eq!(entries[0].body, "favorite color is purple");
        assert_eq!(entries[0].seq, 0);
        // created_at_secs is captured by the substrate, not the caller.
        // We can't assert a literal value but we can assert it's not
        // zero unless UNIX epoch is somehow "now".
        assert!(entries[0].created_at_secs > 0);
    }

    #[tokio::test]
    async fn get_recent_returns_newest_first() {
        let mem = InMemoryMemory::new();
        mem.put("notes", "first").await.unwrap();
        mem.put("notes", "second").await.unwrap();
        mem.put("notes", "third").await.unwrap();

        let entries = mem.get_recent("notes", 10).await.unwrap();
        let bodies: Vec<&str> = entries.iter().map(|e| e.body.as_str()).collect();
        assert_eq!(
            bodies,
            vec!["third", "second", "first"],
            "get_recent must return newest first"
        );
        // And the seq numbers match: newest has the highest seq.
        assert_eq!(entries[0].seq, 2);
        assert_eq!(entries[1].seq, 1);
        assert_eq!(entries[2].seq, 0);
    }

    #[tokio::test]
    async fn get_recent_caps_at_limit() {
        let mem = InMemoryMemory::new();
        for i in 0..5 {
            mem.put("notes", &format!("entry {i}")).await.unwrap();
        }
        let entries = mem.get_recent("notes", 2).await.unwrap();
        assert_eq!(entries.len(), 2, "limit must cap the returned slice");
        // Newest two are seq 4 and 3.
        assert_eq!(entries[0].seq, 4);
        assert_eq!(entries[1].seq, 3);
    }

    #[tokio::test]
    async fn topics_are_isolated() {
        let mem = InMemoryMemory::new();
        mem.put("notes", "a notes entry").await.unwrap();
        mem.put("todos", "a todos entry").await.unwrap();
        mem.put("notes", "another notes entry").await.unwrap();

        let notes = mem.get_recent("notes", 10).await.unwrap();
        let todos = mem.get_recent("todos", 10).await.unwrap();

        assert_eq!(notes.len(), 2);
        assert_eq!(todos.len(), 1);
        assert!(notes.iter().all(|e| e.topic == "notes"));
        assert!(todos.iter().all(|e| e.topic == "todos"));
        // Sequence numbers are globally monotonic, not per-topic. The
        // "todos" entry is seq=1, sandwiched between the two "notes"
        // entries (seq=0 and seq=2). This matters because task 2's
        // RedbMemory will key entries as `topic || 0x00 || seq_be`,
        // and any regression to per-topic seq would make the disk
        // layout divergent from this fake.
        assert_eq!(todos[0].seq, 1);
    }

    #[tokio::test]
    async fn get_recent_for_unknown_topic_is_empty_not_error() {
        let mem = InMemoryMemory::new();
        mem.put("notes", "something").await.unwrap();
        let result = mem.get_recent("other", 10).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn forget_deletes_only_the_named_topic() {
        let mem = InMemoryMemory::new();
        mem.put("notes", "n1").await.unwrap();
        mem.put("notes", "n2").await.unwrap();
        mem.put("todos", "t1").await.unwrap();

        let removed = mem.forget("notes").await.unwrap();
        assert_eq!(removed, 2, "forget must return count deleted");

        let notes = mem.get_recent("notes", 10).await.unwrap();
        let todos = mem.get_recent("todos", 10).await.unwrap();
        assert!(notes.is_empty(), "notes must be gone after forget");
        assert_eq!(todos.len(), 1, "todos must survive forget(notes)");
    }

    #[tokio::test]
    async fn forget_unknown_topic_returns_zero_not_error() {
        let mem = InMemoryMemory::new();
        let removed = mem.forget("never-written").await.unwrap();
        assert_eq!(removed, 0);
    }

    #[tokio::test]
    async fn put_after_forget_continues_the_monotonic_sequence() {
        // This is the subtle one. If we reset next_seq on forget, two
        // entries at different points in time could share a seq number,
        // which would break the "monotonic insertion counter" guarantee
        // that task 2's key encoding depends on (entries keyed by
        // `topic || 0x00 || seq_be` must be globally unique inside a
        // topic, and the simplest way to ensure that is to never reuse
        // a seq). Document the invariant with a test.
        let mem = InMemoryMemory::new();
        mem.put("notes", "a").await.unwrap(); // seq 0
        mem.put("notes", "b").await.unwrap(); // seq 1
        mem.forget("notes").await.unwrap();
        let seq_after_forget = mem.put("notes", "c").await.unwrap();
        assert_eq!(
            seq_after_forget, 2,
            "forget must not reset the sequence counter"
        );
    }

    #[tokio::test]
    async fn empty_topic_is_rejected_on_all_three_methods() {
        let mem = InMemoryMemory::new();
        assert!(matches!(
            mem.put("", "body").await,
            Err(MemoryError::EmptyTopic)
        ));
        assert!(matches!(
            mem.get_recent("", 10).await,
            Err(MemoryError::EmptyTopic)
        ));
        assert!(matches!(
            mem.forget("").await,
            Err(MemoryError::EmptyTopic)
        ));
    }

    #[tokio::test]
    async fn zero_limit_is_rejected() {
        let mem = InMemoryMemory::new();
        mem.put("notes", "x").await.unwrap();
        assert!(matches!(
            mem.get_recent("notes", 0).await,
            Err(MemoryError::ZeroLimit)
        ));
    }

    #[test]
    fn memory_entry_round_trips_through_json() {
        // This is the Q1 load-bearing assertion: every MemoryEntry must
        // round-trip cleanly through the same encode/decode path task
        // 2's RedbMemory will use. If a future field change breaks this,
        // the test fails here before anyone reaches redb.
        let entry = MemoryEntry {
            topic: "with spaces and \"quotes\"".to_string(),
            body: "body with\nnewlines and unicode: résumé 🧠".to_string(),
            seq: 42,
            created_at_secs: 1_700_000_000,
        };
        let bytes = InMemoryMemory::encode_entry(&entry).unwrap();
        let decoded = InMemoryMemory::decode_entry(&bytes).unwrap();
        assert_eq!(entry, decoded);
    }

    // ---- Phase 10 task 1: scan_prefix substrate primitive ---------

    #[tokio::test]
    async fn scan_prefix_groups_entries_by_topic_in_byte_order() {
        // Seed three topics in non-alphabetical order to prove the
        // output is byte-lex ascending, not insertion order. The
        // BTreeMap-backed fake gets this for free, but locking it in
        // with an assertion protects the contract if the impl is ever
        // swapped for a hashmap-based one.
        let mem = InMemoryMemory::new();
        mem.put("zeta", "z").await.unwrap();
        mem.put("alpha", "a").await.unwrap();
        mem.put("mu", "m").await.unwrap();

        let out = mem.scan_prefix("", 10).await.unwrap();
        let topics: Vec<&str> = out.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(topics, vec!["alpha", "mu", "zeta"]);
        // Each group carries its own entry.
        for (topic, entries) in &out {
            assert_eq!(entries.len(), 1);
            match topic.as_str() {
                "alpha" => assert_eq!(entries[0].body, "a"),
                "mu" => assert_eq!(entries[0].body, "m"),
                "zeta" => assert_eq!(entries[0].body, "z"),
                _ => unreachable!(),
            }
        }
    }

    #[tokio::test]
    async fn scan_prefix_filters_by_prefix() {
        let mem = InMemoryMemory::new();
        mem.put("logs-2026", "L1").await.unwrap();
        mem.put("logs-2027", "L2").await.unwrap();
        mem.put("notes", "n").await.unwrap();

        let out = mem.scan_prefix("logs-", 10).await.unwrap();
        let topics: Vec<&str> = out.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(topics, vec!["logs-2026", "logs-2027"]);
        // "notes" is excluded.
        assert!(!topics.contains(&"notes"));
    }

    #[tokio::test]
    async fn scan_prefix_caps_per_topic_and_returns_newest_first() {
        let mem = InMemoryMemory::new();
        for i in 0..5 {
            mem.put("notes", &format!("n{i}")).await.unwrap();
        }
        let out = mem.scan_prefix("notes", 2).await.unwrap();
        assert_eq!(out.len(), 1);
        let (_, entries) = &out[0];
        assert_eq!(entries.len(), 2, "per_topic_limit must cap the slice");
        // Newest first: seq 4 then seq 3.
        assert_eq!(entries[0].seq, 4);
        assert_eq!(entries[1].seq, 3);
    }

    #[tokio::test]
    async fn scan_prefix_zero_limit_is_rejected() {
        let mem = InMemoryMemory::new();
        mem.put("notes", "x").await.unwrap();
        assert!(matches!(
            mem.scan_prefix("notes", 0).await,
            Err(MemoryError::ZeroLimit)
        ));
    }

    #[tokio::test]
    async fn scan_prefix_with_empty_prefix_sees_every_topic() {
        // The trait contract is clear that an empty prefix is legal
        // and means "every topic." The tool layer (`memory.read`
        // wildcard with no session) relies on this for single-
        // partition channels.
        let mem = InMemoryMemory::new();
        mem.put("a", "1").await.unwrap();
        mem.put("b", "2").await.unwrap();
        let out = mem.scan_prefix("", 10).await.unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn decode_entry_on_garbage_bytes_returns_encoding_error() {
        // Prove that backend-layer corruption routes to a typed error
        // rather than a panic. Task 2's RedbMemory will surface this
        // same MemoryError::Encoding when a decrypted value fails to
        // parse as JSON (possible under a partial-write crash).
        let err = InMemoryMemory::decode_entry(b"not-valid-json-{")
            .expect_err("must reject garbage");
        assert!(matches!(err, MemoryError::Encoding(_)));
    }
}
