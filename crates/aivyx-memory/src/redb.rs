//! `RedbMemory` — the redb-backed, AEAD-encrypted `Memory` impl.
//!
//! Wraps a single `aivyx_storage::DomainHandle` for `KeyDomain::Memory`
//! and ships the exact same semantics as [`crate::InMemoryMemory`]: the
//! two impls are interchangeable for task 3's tool wrappers, which is
//! why the fake exists at all. If this impl ever diverges from the fake,
//! that divergence is a bug in whichever is wrong.
//!
//! ## Key layout
//!
//! The `KeyDomain::Memory` table multiplexes two logical spaces using
//! a one-byte discriminator prefix:
//!
//! ```text
//! entries  := b"e\x00" || topic_bytes || b"\x00" || seq_be     (u64 BE)
//! metadata := b"m\x00" || b"next_seq"
//! ```
//!
//! - The `e\0` / `m\0` discriminators are disjoint (`0x65` vs `0x6D`),
//!   so there is no way a valid entry key can collide with a metadata
//!   key regardless of what topic bytes the agent chooses.
//! - The inner `0x00` separator between topic and seq prevents two
//!   sibling topics (`"notes"` vs `"notesfoo"`) from overlapping in a
//!   prefix scan — exactly the same isolation story `scan_prefix`
//!   unit-tests lock in over on the storage side.
//! - Sequence numbers are stored big-endian so lexicographic ordering
//!   of keys inside one topic is identical to numeric ordering of
//!   sequence values.  `scan_prefix` returns them ascending and
//!   [`RedbMemory::get_recent`] reverses to get newest-first.
//!
//! ## Sequence counter recovery
//!
//! The monotonic sequence counter is the load-bearing invariant task
//! 1's [`super::InMemoryMemory`] documents with
//! `put_after_forget_continues_the_monotonic_sequence`: two entries
//! written at different points in time must never share a seq,
//! because the key encoding relies on globally-unique seq-per-topic.
//!
//! For the in-process fake that's free (a `u64` field on the struct
//! that only resets on drop). For redb this has to survive a process
//! restart, so `RedbMemory::open` reads the metadata key at
//! construction time; if it's absent (cold store), the counter seeds
//! to 0; if present, it decodes the stored u64 and resumes from there.
//! Every successful `put` reserves a seq, performs the entry write,
//! and then persists the incremented counter back to the metadata
//! key in the same task so a crash mid-put means the crash-recovery
//! behavior is "next put gets a fresh seq strictly greater than the
//! last successful one" — no seq reuse on recovery.
//!
//! ## Concurrency
//!
//! The counter lives behind a `tokio::sync::Mutex` so that a
//! concurrent `put`+`put` serializes cleanly. The hot path is short:
//! lock counter → reserve seq → write entry → write counter → unlock.
//! The two writes are separate `DomainHandle::put` calls (redb
//! commits are per-call), so there is a tiny crash-window where the
//! entry is on disk but the counter update isn't. On reopen we
//! recover by scanning the max seq from the on-disk entries, not by
//! trusting the counter — that is the whole point of
//! [`RedbMemory::seed_counter_from_storage`] below.

use std::sync::Arc;

use async_trait::async_trait;

use aivyx_storage::{DomainHandle, KeyDomain, Storage};

use crate::{
    rank_by_cosine, InMemoryMemory, Memory, MemoryEntry, MemoryError,
    RetentionMatcher,
};

/// Discriminator prefix for entry keys. Every entry in the memory
/// domain starts with these two bytes; nothing else does.
const ENTRY_PREFIX: &[u8] = b"e\x00";

/// Full metadata key for the `next_seq` counter. The leading `m\x00`
/// is the metadata-discriminator prefix — disjoint from `ENTRY_PREFIX`
/// (`e\x00`), which is how entries and metadata coexist in one
/// `KeyDomain::Memory` table without collision. Future metadata keys
/// (schema version, stats) would share the `m\x00` prefix.
const META_NEXT_SEQ_KEY: &[u8] = b"m\x00next_seq";

/// Phase 75 — discriminator prefix for vector keys in the
/// separate `KeyDomain::MemoryVectors` table. The layout mirrors
/// the entry key (`v\x00 || topic || \x00 || seq_be`) so the
/// same big-endian-seq prefix-scan isolation story holds. It
/// lives in its *own* domain (distinct HKDF subkey) so a vector
/// blob and an entry body are never decryptable with the same
/// key — the Phase 75 Task 2 `KeyDomain::MemoryVectors`
/// isolation guarantee.
const VECTOR_PREFIX: &[u8] = b"v\x00";

/// `RedbMemory` — the persistent [`Memory`] implementation Phase 6
/// ships to the binary. Construct via [`RedbMemory::open`], share as
/// `Arc<dyn Memory>`.
pub struct RedbMemory {
    handle: DomainHandle,
    /// Monotonic sequence counter, persisted to `META_NEXT_SEQ_KEY`
    /// after every successful `put` and re-seeded from the on-disk
    /// entries at `open` time.
    next_seq: tokio::sync::Mutex<u64>,
    /// Phase 75 — handle to the separate `KeyDomain::MemoryVectors`
    /// table where embedding vectors are persisted.
    vectors_handle: DomainHandle,
    /// Phase 75 — in-memory `(topic, seq, vector)` index rebuilt
    /// from `vectors_handle` at `open`. `semantic_search` ranks
    /// over this rather than scanning + decrypting every vector
    /// row per query; `put_vector` keeps it and the table in
    /// lock-step.
    vector_index: tokio::sync::Mutex<Vec<(String, u64, Vec<f32>)>>,
}

impl std::fmt::Debug for RedbMemory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Redact the counter to avoid leaking write-rate info in
        // debug logs; whoever is debugging can peek at the mutex
        // directly if they need to.
        f.debug_struct("RedbMemory")
            .field("domain", &self.handle.domain())
            .finish()
    }
}

impl RedbMemory {
    /// Open a `RedbMemory` backed by the given storage handle. Seeds
    /// the monotonic sequence counter from whatever is already on
    /// disk: first it reads the persisted counter under
    /// `META_NEXT_SEQ_KEY`, then it scans every entry and takes
    /// `max(persisted, max_entry_seq + 1)`. The scan step is the
    /// crash-recovery path — if the last `put` committed the entry
    /// write but not the counter write, the scan will notice and
    /// correct.
    pub async fn open(storage: Arc<dyn Storage>) -> Result<Arc<Self>, MemoryError> {
        let handle = storage.domain(KeyDomain::Memory);
        let vectors_handle = storage.domain(KeyDomain::MemoryVectors);
        let seed = Self::seed_counter_from_storage(&handle).await?;
        let index = Self::load_vector_index(&vectors_handle).await?;
        Ok(Arc::new(RedbMemory {
            handle,
            next_seq: tokio::sync::Mutex::new(seed),
            vectors_handle,
            vector_index: tokio::sync::Mutex::new(index),
        }))
    }

    /// Phase 75 — rebuild the in-memory vector index from the
    /// `MemoryVectors` domain. One decrypt + decode per row,
    /// paid once at open. A row whose value isn't a clean f32
    /// blob is skipped rather than failing the whole open — a
    /// corrupt vector should degrade semantic search, not brick
    /// the daemon.
    async fn load_vector_index(
        handle: &DomainHandle,
    ) -> Result<Vec<(String, u64, Vec<f32>)>, MemoryError> {
        let rows = handle
            .scan_prefix(VECTOR_PREFIX)
            .await
            .map_err(|e| MemoryError::Backend(e.to_string()))?;
        let mut index = Vec::with_capacity(rows.len());
        for (key, value) in &rows {
            let Some((topic, seq)) = parse_vector_key(key) else {
                continue;
            };
            let Some(vector) = decode_vector(value) else {
                continue;
            };
            index.push((topic, seq, vector));
        }
        Ok(index)
    }

    /// Build the vector key for `(topic, seq)`. Same layout as
    /// the entry key, distinct discriminator (`v\x00`).
    fn vector_key(topic: &str, seq: u64) -> Vec<u8> {
        let topic_bytes = topic.as_bytes();
        let mut key = Vec::with_capacity(
            VECTOR_PREFIX.len() + topic_bytes.len() + 1 + 8,
        );
        key.extend_from_slice(VECTOR_PREFIX);
        key.extend_from_slice(topic_bytes);
        key.push(0x00);
        key.extend_from_slice(&seq.to_be_bytes());
        key
    }

    /// Single-topic vector scan prefix: every vector under
    /// `topic` and nothing else.
    fn topic_vector_prefix(topic: &str) -> Vec<u8> {
        let topic_bytes = topic.as_bytes();
        let mut prefix =
            Vec::with_capacity(VECTOR_PREFIX.len() + topic_bytes.len() + 1);
        prefix.extend_from_slice(VECTOR_PREFIX);
        prefix.extend_from_slice(topic_bytes);
        prefix.push(0x00);
        prefix
    }

    /// Scan the domain once to recover the correct `next_seq` value.
    /// Returns the first `seq` that a new `put` may safely assign —
    /// strictly greater than every `seq` currently on disk.
    ///
    /// Algorithm: read the persisted counter from `META_NEXT_SEQ_KEY`
    /// (may be absent), scan every entry under `ENTRY_PREFIX` and
    /// compute `max_entry_seq + 1`, return `max(persisted_seq,
    /// max_entry_seq + 1, 0)`. The scan-based arm dominates any
    /// crash between "entry committed" and "counter persisted."
    async fn seed_counter_from_storage(
        handle: &DomainHandle,
    ) -> Result<u64, MemoryError> {
        let persisted = match handle.get(META_NEXT_SEQ_KEY).await {
            Ok(Some(bytes)) => decode_u64_be(&bytes).unwrap_or(0),
            Ok(None) => 0,
            Err(e) => return Err(MemoryError::Backend(e.to_string())),
        };

        // Scan every entry and find the max seq tail. Because seq is
        // stored big-endian, the lexicographically-largest entry key
        // also has the largest seq — we could technically pick only
        // the last row. The pragmatic choice is to decode every row
        // and take the max regardless of ordering, so a future key
        // layout change doesn't silently break recovery.
        let rows = handle
            .scan_prefix(ENTRY_PREFIX)
            .await
            .map_err(|e| MemoryError::Backend(e.to_string()))?;

        let mut max_entry_seq: Option<u64> = None;
        for (key, _value) in &rows {
            if let Some(seq) = seq_from_entry_key(key) {
                max_entry_seq = Some(max_entry_seq.map(|m| m.max(seq)).unwrap_or(seq));
            }
        }

        let from_scan = max_entry_seq.map(|m| m.saturating_add(1)).unwrap_or(0);
        Ok(persisted.max(from_scan))
    }

    /// Build the entry key for `(topic, seq)`. Layout documented in
    /// the module header.
    fn entry_key(topic: &str, seq: u64) -> Vec<u8> {
        let topic_bytes = topic.as_bytes();
        let mut key = Vec::with_capacity(ENTRY_PREFIX.len() + topic_bytes.len() + 1 + 8);
        key.extend_from_slice(ENTRY_PREFIX);
        key.extend_from_slice(topic_bytes);
        key.push(0x00);
        key.extend_from_slice(&seq.to_be_bytes());
        key
    }

    /// Build the inclusive-lower prefix that `scan_prefix` uses for
    /// a single-topic range scan. Everything below this prefix is
    /// entries under `topic`; everything above starts with a
    /// different topic or a different discriminator.
    fn topic_scan_prefix(topic: &str) -> Vec<u8> {
        let topic_bytes = topic.as_bytes();
        let mut prefix = Vec::with_capacity(ENTRY_PREFIX.len() + topic_bytes.len() + 1);
        prefix.extend_from_slice(ENTRY_PREFIX);
        prefix.extend_from_slice(topic_bytes);
        prefix.push(0x00);
        prefix
    }
}

#[async_trait]
impl Memory for RedbMemory {
    async fn put(&self, topic: &str, body: &str) -> Result<u64, MemoryError> {
        if topic.is_empty() {
            return Err(MemoryError::EmptyTopic);
        }

        // Reserve a sequence number under the async mutex. We hold
        // the lock across both writes (entry + counter) so a
        // concurrent put doesn't race us to the same seq. The lock
        // is not held across `open` (another task at a different
        // point in time can happily open its own RedbMemory).
        let mut guard = self.next_seq.lock().await;
        let seq = *guard;

        // Build and encode the entry BEFORE we touch storage — any
        // pre-write failure path aborts cleanly without touching
        // the counter.
        let entry = MemoryEntry {
            topic: topic.to_string(),
            body: body.to_string(),
            seq,
            created_at_secs: now_secs(),
            last_read_at_secs: 0,
        };
        let entry_bytes = InMemoryMemory::encode_entry(&entry)?;

        let key = Self::entry_key(topic, seq);
        self.handle
            .put(&key, &entry_bytes)
            .await
            .map_err(|e| MemoryError::Backend(e.to_string()))?;

        // Entry persisted. Advance the in-memory counter and
        // mirror it to disk. A crash between these two writes is
        // recovered by `seed_counter_from_storage` on the next open
        // (which scans the entries), so we don't have to bracket the
        // pair in a transaction.
        let next = seq.saturating_add(1);
        let next_bytes = next.to_be_bytes();
        self.handle
            .put(META_NEXT_SEQ_KEY, &next_bytes)
            .await
            .map_err(|e| MemoryError::Backend(e.to_string()))?;
        *guard = next;

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

        let prefix = Self::topic_scan_prefix(topic);
        let rows = self
            .handle
            .scan_prefix(&prefix)
            .await
            .map_err(|e| MemoryError::Backend(e.to_string()))?;

        // `scan_prefix` returns rows sorted by key ascending, which
        // (because seq is big-endian) is seq ascending. We want
        // newest-first, so reverse and take `limit`. Decode each
        // payload through the same path InMemoryMemory uses.
        //
        // Phase 74 — LRU stamp: every entry returned to a caller
        // gets its `last_read_at_secs` rewritten to now on disk.
        // We persist the bump because eviction queries the field
        // at GC time; an in-memory-only stamp would be lost on
        // restart and break LRU semantics across daemon lifetimes.
        let now = now_secs();
        let mut out: Vec<MemoryEntry> = Vec::with_capacity(limit.min(rows.len()));
        for (key, value) in rows.iter().rev().take(limit) {
            let mut entry = InMemoryMemory::decode_entry(value)?;
            entry.last_read_at_secs = now;
            let entry_bytes = InMemoryMemory::encode_entry(&entry)?;
            self.handle
                .put(key, &entry_bytes)
                .await
                .map_err(|e| MemoryError::Backend(e.to_string()))?;
            out.push(entry);
        }
        Ok(out)
    }

    async fn forget(&self, topic: &str) -> Result<usize, MemoryError> {
        if topic.is_empty() {
            return Err(MemoryError::EmptyTopic);
        }

        let prefix = Self::topic_scan_prefix(topic);
        let rows = self
            .handle
            .scan_prefix(&prefix)
            .await
            .map_err(|e| MemoryError::Backend(e.to_string()))?;

        // Delete every matching key. We intentionally do NOT touch
        // the next_seq counter here: forgetting a topic must not
        // reset the counter, or two entries written at different
        // points in time could share a seq. The same invariant
        // task 1's `put_after_forget_continues_the_monotonic_sequence`
        // test locks in for the fake.
        let mut deleted: usize = 0;
        for (key, _value) in &rows {
            self.handle
                .delete(key)
                .await
                .map_err(|e| MemoryError::Backend(e.to_string()))?;
            deleted += 1;
        }

        // Phase 75 — drop the topic's vectors from the table and
        // the in-memory index so a forgotten topic leaves no
        // orphan vector behind.
        let vprefix = Self::topic_vector_prefix(topic);
        let vrows = self
            .vectors_handle
            .scan_prefix(&vprefix)
            .await
            .map_err(|e| MemoryError::Backend(e.to_string()))?;
        for (key, _value) in &vrows {
            self.vectors_handle
                .delete(key)
                .await
                .map_err(|e| MemoryError::Backend(e.to_string()))?;
        }
        self.vector_index
            .lock()
            .await
            .retain(|(t, _, _)| t != topic);

        Ok(deleted)
    }

    async fn gc_topic(
        &self,
        topic: &str,
        max_entries: usize,
    ) -> Result<usize, MemoryError> {
        if topic.is_empty() {
            return Err(MemoryError::EmptyTopic);
        }

        let prefix = Self::topic_scan_prefix(topic);
        let rows = self
            .handle
            .scan_prefix(&prefix)
            .await
            .map_err(|e| MemoryError::Backend(e.to_string()))?;

        if rows.len() <= max_entries {
            return Ok(0);
        }

        // Rows are sorted by key ascending (seq ascending because
        // seq is big-endian). Delete the oldest entries (front of
        // the list) to keep only `max_entries` newest.
        let to_remove = rows.len() - max_entries;
        let mut deleted = 0usize;
        for (key, _value) in rows.iter().take(to_remove) {
            self.handle
                .delete(key)
                .await
                .map_err(|e| MemoryError::Backend(e.to_string()))?;
            deleted += 1;
        }
        Ok(deleted)
    }

    async fn gc_expired(
        &self,
        cutoff_secs: u64,
    ) -> Result<usize, MemoryError> {
        // Scan every entry in the memory domain.
        let rows = self
            .handle
            .scan_prefix(ENTRY_PREFIX)
            .await
            .map_err(|e| MemoryError::Backend(e.to_string()))?;

        let mut deleted = 0usize;
        for (key, value) in &rows {
            let entry = InMemoryMemory::decode_entry(value)?;
            if entry.created_at_secs < cutoff_secs {
                self.handle
                    .delete(key)
                    .await
                    .map_err(|e| MemoryError::Backend(e.to_string()))?;
                deleted += 1;
            }
        }
        Ok(deleted)
    }

    async fn scan_prefix(
        &self,
        topic_prefix: &str,
        per_topic_limit: usize,
    ) -> Result<Vec<(String, Vec<MemoryEntry>)>, MemoryError> {
        if per_topic_limit == 0 {
            return Err(MemoryError::ZeroLimit);
        }

        // Build the full redb key prefix: ENTRY_PREFIX || topic_prefix.
        // This selects every entry whose physical topic begins with
        // `topic_prefix`. Metadata keys (`m\x00...`) are excluded
        // because they start with a different discriminator byte.
        let mut key_prefix =
            Vec::with_capacity(ENTRY_PREFIX.len() + topic_prefix.len());
        key_prefix.extend_from_slice(ENTRY_PREFIX);
        key_prefix.extend_from_slice(topic_prefix.as_bytes());

        let rows = self
            .handle
            .scan_prefix(&key_prefix)
            .await
            .map_err(|e| MemoryError::Backend(e.to_string()))?;

        // Group rows by physical topic. `scan_prefix` returns rows
        // byte-lexicographically ascending, which means all entries
        // for one topic are contiguous — we can group in one pass
        // without a hash map, but using a BTreeMap gives us the
        // same contract with less fiddly accumulation logic. The
        // BTreeMap also guarantees the returned vector is in
        // byte-lexicographic topic order, matching the trait
        // contract's documented order.
        use std::collections::BTreeMap;
        let mut grouped: BTreeMap<String, Vec<MemoryEntry>> = BTreeMap::new();
        for (key, value) in &rows {
            // Extract the physical topic from the key layout:
            // `e\x00 || topic_bytes || \x00 || seq_be(8)`.
            // Skip the leading `e\x00`, then find the `\x00`
            // separator immediately before the 8-byte seq tail.
            // The seq tail is always the last 8 bytes, so the
            // separator index is `key.len() - 9`.
            if key.len() < ENTRY_PREFIX.len() + 1 + 8 {
                continue;
            }
            let sep_idx = key.len() - 9;
            if key[sep_idx] != 0x00 {
                continue;
            }
            let topic_bytes = &key[ENTRY_PREFIX.len()..sep_idx];
            let Ok(topic_str) = std::str::from_utf8(topic_bytes) else {
                continue;
            };

            let entry = InMemoryMemory::decode_entry(value)?;
            grouped
                .entry(topic_str.to_string())
                .or_default()
                .push(entry);
        }

        // Within each topic, rows came back seq-ascending (because
        // seq is big-endian in the key, and lexicographic order
        // over big-endian u64 is numeric order). We want newest-
        // first and at most `per_topic_limit` entries.
        let mut out: Vec<(String, Vec<MemoryEntry>)> = Vec::with_capacity(grouped.len());
        for (topic, mut entries) in grouped {
            entries.reverse();
            entries.truncate(per_topic_limit);
            out.push((topic, entries));
        }
        Ok(out)
    }

    // ---- Phase 74 — search / list / LRU evict ---------------

    async fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        if limit == 0 {
            return Err(MemoryError::ZeroLimit);
        }
        let rows = self
            .handle
            .scan_prefix(ENTRY_PREFIX)
            .await
            .map_err(|e| MemoryError::Backend(e.to_string()))?;
        let needle = query.to_lowercase();
        let mut hits: Vec<MemoryEntry> = Vec::new();
        for (_key, value) in &rows {
            let entry = InMemoryMemory::decode_entry(value)?;
            let matches = if needle.is_empty() {
                true
            } else {
                entry.topic.to_lowercase().contains(&needle)
                    || entry.body.to_lowercase().contains(&needle)
            };
            if matches {
                hits.push(entry);
            }
        }
        // Newest first.
        hits.sort_by_key(|h| std::cmp::Reverse(h.seq));
        hits.truncate(limit);
        Ok(hits)
    }

    async fn list_topics(&self) -> Result<Vec<String>, MemoryError> {
        use std::collections::BTreeSet;
        let rows = self
            .handle
            .scan_prefix(ENTRY_PREFIX)
            .await
            .map_err(|e| MemoryError::Backend(e.to_string()))?;
        let mut topics: BTreeSet<String> = BTreeSet::new();
        for (key, _value) in &rows {
            // Key layout: `e\x00 || topic_bytes || \x00 || seq_be(8)`.
            if key.len() < ENTRY_PREFIX.len() + 1 + 8 {
                continue;
            }
            let sep_idx = key.len() - 9;
            if key[sep_idx] != 0x00 {
                continue;
            }
            let topic_bytes = &key[ENTRY_PREFIX.len()..sep_idx];
            if let Ok(topic_str) = std::str::from_utf8(topic_bytes) {
                topics.insert(topic_str.to_string());
            }
        }
        Ok(topics.into_iter().collect())
    }

    async fn gc_expired_with_rules(
        &self,
        rules: &[RetentionMatcher<'_>],
        default_cutoff_secs: Option<u64>,
    ) -> Result<usize, MemoryError> {
        let rows = self
            .handle
            .scan_prefix(ENTRY_PREFIX)
            .await
            .map_err(|e| MemoryError::Backend(e.to_string()))?;
        let mut deleted = 0usize;
        for (key, value) in &rows {
            let entry = InMemoryMemory::decode_entry(value)?;
            let cutoff = rules
                .iter()
                .find(|r| (r.matches)(entry.topic.as_str()))
                .map(|r| r.cutoff_secs)
                .unwrap_or(default_cutoff_secs);
            let Some(cutoff) = cutoff else {
                continue;
            };
            if entry.created_at_secs < cutoff {
                self.handle
                    .delete(key)
                    .await
                    .map_err(|e| MemoryError::Backend(e.to_string()))?;
                deleted += 1;
            }
        }
        Ok(deleted)
    }

    async fn evict_oldest_unread(
        &self,
        topic: &str,
        keep: usize,
    ) -> Result<usize, MemoryError> {
        if topic.is_empty() {
            return Err(MemoryError::EmptyTopic);
        }
        let prefix = Self::topic_scan_prefix(topic);
        let rows = self
            .handle
            .scan_prefix(&prefix)
            .await
            .map_err(|e| MemoryError::Backend(e.to_string()))?;
        if rows.len() <= keep {
            return Ok(0);
        }
        // Decode each row to read `last_read_at_secs`, then rank.
        // Ties on read time break by older seq loses first.
        let mut ranked: Vec<(Vec<u8>, MemoryEntry)> =
            Vec::with_capacity(rows.len());
        for (key, value) in rows {
            let entry = InMemoryMemory::decode_entry(&value)?;
            ranked.push((key, entry));
        }
        ranked.sort_by(|a, b| {
            a.1.last_read_at_secs
                .cmp(&b.1.last_read_at_secs)
                .then(a.1.seq.cmp(&b.1.seq))
        });
        let to_remove = ranked.len() - keep;
        let mut deleted = 0;
        let mut removed_seqs: Vec<u64> = Vec::with_capacity(to_remove);
        for (key, entry) in ranked.into_iter().take(to_remove) {
            self.handle
                .delete(&key)
                .await
                .map_err(|e| MemoryError::Backend(e.to_string()))?;
            // Phase 75 — evict the matching vector too.
            self.vectors_handle
                .delete(&Self::vector_key(topic, entry.seq))
                .await
                .map_err(|e| MemoryError::Backend(e.to_string()))?;
            removed_seqs.push(entry.seq);
            deleted += 1;
        }
        self.vector_index
            .lock()
            .await
            .retain(|(t, s, _)| !(t == topic && removed_seqs.contains(s)));
        Ok(deleted)
    }

    async fn put_vector(
        &self,
        topic: &str,
        seq: u64,
        vector: Vec<f32>,
    ) -> Result<(), MemoryError> {
        if topic.is_empty() {
            return Err(MemoryError::EmptyTopic);
        }
        let key = Self::vector_key(topic, seq);
        self.vectors_handle
            .put(&key, &encode_vector(&vector))
            .await
            .map_err(|e| MemoryError::Backend(e.to_string()))?;
        // Keep the in-memory index in lock-step. Idempotent on
        // (topic, seq): a re-embed replaces, never duplicates.
        let mut index = self.vector_index.lock().await;
        if let Some(row) = index
            .iter_mut()
            .find(|(t, s, _)| t == topic && *s == seq)
        {
            row.2 = vector;
        } else {
            index.push((topic.to_string(), seq, vector));
        }
        Ok(())
    }

    async fn load_all_vectors(
        &self,
    ) -> Result<Vec<(String, u64, Vec<f32>)>, MemoryError> {
        Ok(self.vector_index.lock().await.clone())
    }

    async fn semantic_search(
        &self,
        query_vec: &[f32],
        limit: usize,
    ) -> Result<Vec<MemoryEntry>, MemoryError> {
        Ok(self
            .semantic_search_scored(query_vec, limit)
            .await?
            .into_iter()
            .map(|(entry, _score)| entry)
            .collect())
    }

    async fn semantic_search_scored(
        &self,
        query_vec: &[f32],
        limit: usize,
    ) -> Result<Vec<(MemoryEntry, f32)>, MemoryError> {
        if limit == 0 {
            return Err(MemoryError::ZeroLimit);
        }
        // Rank under the index lock, then release it before the
        // per-winner entry fetches (decrypt I/O) so a concurrent
        // put_vector isn't blocked on storage latency.
        let ranked = {
            let index = self.vector_index.lock().await;
            rank_by_cosine(&index, query_vec, limit)
        };
        let mut out = Vec::with_capacity(ranked.len());
        for (topic, seq, score) in ranked {
            let key = Self::entry_key(&topic, seq);
            // A winner whose entry body is gone (entry GC ran but
            // the vector wasn't cleaned) is skipped — semantic
            // search is the consistency backstop.
            if let Some(bytes) = self
                .handle
                .get(&key)
                .await
                .map_err(|e| MemoryError::Backend(e.to_string()))?
            {
                out.push((InMemoryMemory::decode_entry(&bytes)?, score));
            }
        }
        Ok(out)
    }

    async fn promote_recall_helpful(
        &self,
        topic: &str,
        seq: u64,
    ) -> Result<bool, MemoryError> {
        if topic.is_empty() {
            return Err(MemoryError::EmptyTopic);
        }
        let key = Self::entry_key(topic, seq);
        let Some(bytes) = self
            .handle
            .get(&key)
            .await
            .map_err(|e| MemoryError::Backend(e.to_string()))?
        else {
            return Ok(false);
        };
        let mut entry = InMemoryMemory::decode_entry(&bytes)?;
        entry.last_read_at_secs = now_secs();
        let encoded = InMemoryMemory::encode_entry(&entry)?;
        self.handle
            .put(&key, &encoded)
            .await
            .map_err(|e| MemoryError::Backend(e.to_string()))?;
        Ok(true)
    }
}

/// Extract the trailing `seq_be` from an entry key, or `None` if the
/// bytes don't parse as a valid entry key (unexpected prefix, too
/// short). Used by the open-time counter recovery; not part of the
/// hot path.
fn seq_from_entry_key(key: &[u8]) -> Option<u64> {
    if !key.starts_with(ENTRY_PREFIX) || key.len() < ENTRY_PREFIX.len() + 1 + 8 {
        return None;
    }
    // The last 8 bytes are the seq. Anything before that is
    // `ENTRY_PREFIX || topic || 0x00`, which we don't need to parse
    // for recovery.
    let tail = &key[key.len() - 8..];
    let mut buf = [0u8; 8];
    buf.copy_from_slice(tail);
    Some(u64::from_be_bytes(buf))
}

/// Decode an 8-byte big-endian u64, or `None` if the slice is not 8
/// bytes. Used for the persisted counter value at
/// `META_NEXT_SEQ_KEY`.
fn decode_u64_be(bytes: &[u8]) -> Option<u64> {
    if bytes.len() != 8 {
        return None;
    }
    let mut buf = [0u8; 8];
    buf.copy_from_slice(bytes);
    Some(u64::from_be_bytes(buf))
}

/// Phase 75 — parse `(topic, seq)` out of a vector key
/// (`v\x00 || topic || \x00 || seq_be(8)`). `None` if the bytes
/// don't fit the layout or the topic isn't UTF-8.
fn parse_vector_key(key: &[u8]) -> Option<(String, u64)> {
    if !key.starts_with(VECTOR_PREFIX)
        || key.len() < VECTOR_PREFIX.len() + 1 + 8
    {
        return None;
    }
    let sep_idx = key.len() - 9;
    if key[sep_idx] != 0x00 {
        return None;
    }
    let topic = std::str::from_utf8(&key[VECTOR_PREFIX.len()..sep_idx])
        .ok()?
        .to_string();
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&key[key.len() - 8..]);
    Some((topic, u64::from_be_bytes(buf)))
}

/// Phase 75 — encode an f32 vector as little-endian bytes (4
/// bytes per lane). Compact and zero-dep — JSON would ~3x the
/// 1536-lane payload and bincode isn't a workspace dep.
fn encode_vector(vector: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(vector.len() * 4);
    for v in vector {
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    bytes
}

/// Inverse of [`encode_vector`]. `None` if the blob isn't a
/// whole number of f32 lanes.
fn decode_vector(bytes: &[u8]) -> Option<Vec<f32>> {
    if bytes.len() % 4 != 0 {
        return None;
    }
    Some(
        bytes
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect(),
    )
}

fn now_secs() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use aivyx_crypto::MasterKey;
    use aivyx_storage::{RedbStorage, StorageConfig};

    use super::*;

    /// Hand-rolled `$TMPDIR`-based scratch dir, matching the
    /// convention `aivyx-storage`'s own tests use. Avoids `tempfile`.
    struct Scratch {
        dir: PathBuf,
    }

    impl Scratch {
        fn new() -> Self {
            let tmp = std::env::var("TMPDIR")
                .or_else(|_| std::env::var("TEMP"))
                .unwrap_or_else(|_| "/tmp".to_string());
            let dir = PathBuf::from(tmp)
                .join(format!("aivyx-memory-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).expect("scratch dir must be creatable");
            Scratch { dir }
        }

        fn store_path(&self) -> PathBuf {
            self.dir.join("store.redb")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    async fn open_mem(scratch: &Scratch, master_byte: u8) -> Arc<RedbMemory> {
        let master = MasterKey::from_raw([master_byte; 32]);
        let storage: Arc<dyn Storage> =
            RedbStorage::open(StorageConfig::new(scratch.store_path()), master)
                .await
                .expect("open storage");
        RedbMemory::open(storage).await.expect("open memory")
    }

    #[tokio::test]
    async fn put_then_get_recent_round_trips_single_entry() {
        let scratch = Scratch::new();
        let mem = open_mem(&scratch, 1).await;

        let seq = mem.put("notes", "favorite color is purple").await.unwrap();
        assert_eq!(seq, 0);

        let entries = mem.get_recent("notes", 10).await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].topic, "notes");
        assert_eq!(entries[0].body, "favorite color is purple");
        assert_eq!(entries[0].seq, 0);
    }

    #[tokio::test]
    async fn get_recent_returns_newest_first() {
        let scratch = Scratch::new();
        let mem = open_mem(&scratch, 2).await;

        mem.put("notes", "first").await.unwrap();
        mem.put("notes", "second").await.unwrap();
        mem.put("notes", "third").await.unwrap();

        let entries = mem.get_recent("notes", 10).await.unwrap();
        let bodies: Vec<&str> = entries.iter().map(|e| e.body.as_str()).collect();
        assert_eq!(bodies, vec!["third", "second", "first"]);
        assert_eq!(entries[0].seq, 2);
        assert_eq!(entries[2].seq, 0);
    }

    #[tokio::test]
    async fn get_recent_caps_at_limit() {
        let scratch = Scratch::new();
        let mem = open_mem(&scratch, 3).await;
        for i in 0..5 {
            mem.put("notes", &format!("e{i}")).await.unwrap();
        }
        let entries = mem.get_recent("notes", 2).await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].seq, 4);
        assert_eq!(entries[1].seq, 3);
    }

    #[tokio::test]
    async fn topics_are_isolated_on_disk() {
        let scratch = Scratch::new();
        let mem = open_mem(&scratch, 4).await;

        mem.put("notes", "a notes entry").await.unwrap();
        mem.put("todos", "a todos entry").await.unwrap();
        mem.put("notes", "another notes entry").await.unwrap();

        let notes = mem.get_recent("notes", 10).await.unwrap();
        let todos = mem.get_recent("todos", 10).await.unwrap();

        assert_eq!(notes.len(), 2);
        assert_eq!(todos.len(), 1);
        assert!(notes.iter().all(|e| e.topic == "notes"));
        assert_eq!(todos[0].seq, 1);
        // Load-bearing: the middle seq (todos, seq=1) lands
        // between the two notes seqs (0 and 2), proving the
        // counter is global not per-topic — same invariant the
        // fake's test locks in. If this fails, the two impls
        // have diverged on ordering semantics.
    }

    #[tokio::test]
    async fn topic_sibling_prefix_does_not_leak() {
        // Regression lock for the "notes vs notesfoo" case the
        // `scan_prefix_isolates_sibling_topics` test in
        // aivyx-storage covers — here we verify the combined
        // `ENTRY_PREFIX || topic || 0x00` prefix construction
        // actually produces the right disjoint range.
        let scratch = Scratch::new();
        let mem = open_mem(&scratch, 5).await;

        mem.put("notes", "real").await.unwrap();
        mem.put("notesfoo", "sibling").await.unwrap();

        let notes = mem.get_recent("notes", 10).await.unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].body, "real");

        let notesfoo = mem.get_recent("notesfoo", 10).await.unwrap();
        assert_eq!(notesfoo.len(), 1);
        assert_eq!(notesfoo[0].body, "sibling");
    }

    #[tokio::test]
    async fn get_recent_for_unknown_topic_is_empty_not_error() {
        let scratch = Scratch::new();
        let mem = open_mem(&scratch, 6).await;
        mem.put("notes", "x").await.unwrap();
        let result = mem.get_recent("other", 10).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn forget_deletes_only_the_named_topic() {
        let scratch = Scratch::new();
        let mem = open_mem(&scratch, 7).await;

        mem.put("notes", "n1").await.unwrap();
        mem.put("notes", "n2").await.unwrap();
        mem.put("todos", "t1").await.unwrap();

        let removed = mem.forget("notes").await.unwrap();
        assert_eq!(removed, 2);

        let notes = mem.get_recent("notes", 10).await.unwrap();
        let todos = mem.get_recent("todos", 10).await.unwrap();
        assert!(notes.is_empty());
        assert_eq!(todos.len(), 1);
    }

    #[tokio::test]
    async fn put_after_forget_continues_the_monotonic_sequence() {
        // The invariant that makes the key layout safe: two
        // entries written at different points in time must never
        // share a seq, even across a forget that deletes every
        // entry. Mirror of task 1's fake test, verified against
        // the real impl.
        let scratch = Scratch::new();
        let mem = open_mem(&scratch, 8).await;

        mem.put("notes", "a").await.unwrap(); // seq 0
        mem.put("notes", "b").await.unwrap(); // seq 1
        let deleted = mem.forget("notes").await.unwrap();
        assert_eq!(deleted, 2);
        let seq_after_forget = mem.put("notes", "c").await.unwrap();
        assert_eq!(seq_after_forget, 2, "forget must not reset the counter");
    }

    #[tokio::test]
    async fn empty_topic_is_rejected_on_all_three_methods() {
        let scratch = Scratch::new();
        let mem = open_mem(&scratch, 9).await;

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
        let scratch = Scratch::new();
        let mem = open_mem(&scratch, 10).await;
        mem.put("notes", "x").await.unwrap();
        assert!(matches!(
            mem.get_recent("notes", 0).await,
            Err(MemoryError::ZeroLimit)
        ));
    }

    // ---- The persistence-and-recovery tests ----

    #[tokio::test]
    async fn entries_survive_reopen_with_same_master() {
        // The whole point of the phase: write through RedbMemory,
        // drop it, reopen, read it back. This is the task 2
        // deliverable in one test.
        let scratch = Scratch::new();

        {
            let mem = open_mem(&scratch, 11).await;
            mem.put("notes", "first").await.unwrap();
            mem.put("notes", "second").await.unwrap();
            mem.put("todos", "buy milk").await.unwrap();
            // Drop happens at end of scope. Because `open_mem`
            // returns `Arc<RedbMemory>` the drop chain has to
            // unwind through the Arc's last handle and then
            // DomainHandle's Arc<Database>. Storage tests prove
            // redb releases its file lock cleanly on drop.
        }

        let mem = open_mem(&scratch, 11).await;
        let notes = mem.get_recent("notes", 10).await.unwrap();
        let todos = mem.get_recent("todos", 10).await.unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(todos.len(), 1);
        assert_eq!(notes[0].body, "second");
        assert_eq!(notes[1].body, "first");
        assert_eq!(todos[0].body, "buy milk");
    }

    #[tokio::test]
    async fn counter_is_recovered_across_reopen() {
        // Load-bearing recovery test: confirm that reopening a
        // store with existing entries re-seeds the sequence
        // counter so the next put gets a fresh seq. If this
        // fails, seq collisions are possible on restart — every
        // reopen would try to write seq=0 and clobber existing
        // entries (because the entry key includes the seq).
        let scratch = Scratch::new();

        {
            let mem = open_mem(&scratch, 12).await;
            for i in 0..3 {
                mem.put("notes", &format!("e{i}")).await.unwrap();
            }
            // Last assigned seq was 2; next_seq is now 3.
        }

        let mem = open_mem(&scratch, 12).await;
        // First put after reopen must get seq 3, not seq 0.
        let next = mem.put("notes", "after-reopen").await.unwrap();
        assert_eq!(next, 3, "counter must recover from on-disk entries");

        let all = mem.get_recent("notes", 10).await.unwrap();
        assert_eq!(all.len(), 4);
        assert_eq!(all[0].body, "after-reopen");
        assert_eq!(all[0].seq, 3);
    }

    #[tokio::test]
    async fn wrong_master_fails_on_first_read() {
        // Not strictly a memory-crate concern (the AEAD layer is
        // in aivyx-storage), but a minimal smoke that a wrong
        // master passed to `RedbStorage::open` reaches
        // `RedbMemory::open` cleanly and then surfaces as a
        // MemoryError::Backend on first scan. The storage crate
        // has the stronger coverage (`scan_prefix_on_wrong_master_
        // fails_on_first_row`); this is just the "my wrapper
        // doesn't swallow the error" check.
        let scratch = Scratch::new();

        {
            let mem = open_mem(&scratch, 20).await;
            mem.put("notes", "secret").await.unwrap();
        }

        // `RedbMemory::open` itself calls `seed_counter_from_storage`
        // which scans the entries — with a wrong master that scan
        // surfaces DecryptFailed, which we wrap into
        // MemoryError::Backend. So in this impl the failure lands
        // at `open`, not on the first later call. That's strictly
        // stronger than the fake's contract (which never errors
        // in open) and matches the Phase 5 task 5 pattern where
        // wrong-key failures surfaced on first `get`, not on
        // `open` itself — here it's first scan at open time.
        let bad_master = MasterKey::from_raw([99u8; 32]);
        let storage = RedbStorage::open(StorageConfig::new(scratch.store_path()), bad_master)
            .await
            .expect("storage opens even with wrong master");
        let result = RedbMemory::open(storage).await;
        assert!(
            matches!(result, Err(MemoryError::Backend(_))),
            "expected Backend error from failed decrypt, got {result:?}"
        );
    }

    // ---- Pure-helper tests --------------------------------------------

    #[test]
    fn entry_key_layout_matches_module_contract() {
        let key = RedbMemory::entry_key("notes", 42);
        // `e\0` prefix (2 bytes) + "notes" (5 bytes) + `\0` (1 byte)
        // + 8 bytes big-endian seq = 16 bytes.
        assert_eq!(key.len(), 2 + 5 + 1 + 8);
        assert_eq!(&key[..2], b"e\x00");
        assert_eq!(&key[2..7], b"notes");
        assert_eq!(key[7], 0x00);
        assert_eq!(&key[8..16], &42u64.to_be_bytes());
    }

    #[test]
    fn topic_scan_prefix_is_entry_prefix_plus_topic_plus_null() {
        let prefix = RedbMemory::topic_scan_prefix("notes");
        assert_eq!(prefix, b"e\x00notes\x00");
    }

    #[test]
    fn seq_from_entry_key_recovers_the_tail_u64() {
        let key = RedbMemory::entry_key("notes", 42);
        assert_eq!(seq_from_entry_key(&key), Some(42));
    }

    #[test]
    fn seq_from_entry_key_rejects_non_entry_keys() {
        // Metadata key doesn't start with ENTRY_PREFIX, so the
        // recovery walk in `seed_counter_from_storage` will
        // correctly skip it when iterating mixed results.
        assert_eq!(seq_from_entry_key(META_NEXT_SEQ_KEY), None);
        assert_eq!(seq_from_entry_key(b""), None);
        assert_eq!(seq_from_entry_key(b"e\x00"), None); // too short
    }

    #[test]
    fn decode_u64_be_rejects_wrong_length() {
        assert_eq!(decode_u64_be(&[0; 8]), Some(0));
        assert_eq!(decode_u64_be(&[0; 7]), None);
        assert_eq!(decode_u64_be(&[0; 9]), None);
    }

    // ---- Phase 10 task 1: scan_prefix on disk ---------------------
    //
    // The fake in `lib.rs` covers the ordering and limit contracts;
    // these tests prove the disk impl gets the *key decoding* right,
    // because that's the only place the two impls can drift. The
    // redb impl has to locate the `\0` separator at `key.len() - 9`
    // and parse UTF-8 topic bytes out of the middle of the key —
    // none of which the BTreeMap fake exercises.

    #[tokio::test]
    async fn scan_prefix_on_disk_groups_and_orders_by_topic() {
        let scratch = Scratch::new();
        let mem = open_mem(&scratch, 10).await;

        mem.put("zeta", "z").await.unwrap();
        mem.put("alpha", "a").await.unwrap();
        mem.put("mu", "m").await.unwrap();

        let out = mem.scan_prefix("", 10).await.unwrap();
        let topics: Vec<&str> = out.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(topics, vec!["alpha", "mu", "zeta"]);
    }

    #[tokio::test]
    async fn scan_prefix_on_disk_filters_by_prefix_and_respects_sibling_boundary() {
        // Regression lock for the "notes vs notesfoo" case at the
        // scan_prefix layer. A prefix of "notes" must match both —
        // that's the wildcard semantic — but a prefix of "notes\0"
        // must match ONLY "notes" because the key layout ends the
        // topic with a 0x00 separator. This is load-bearing for the
        // session-namespace layout where the prefix is
        // `\x01s\x01<session>\x01`, and two sessions whose ids share
        // a textual prefix must not leak into each other.
        let scratch = Scratch::new();
        let mem = open_mem(&scratch, 11).await;

        mem.put("notes", "real").await.unwrap();
        mem.put("notesfoo", "sibling").await.unwrap();

        // `notes` prefix matches both (this is the permissive path).
        let out = mem.scan_prefix("notes", 10).await.unwrap();
        let topics: Vec<&str> = out.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(topics, vec!["notes", "notesfoo"]);

        // A more specific prefix filters out the sibling.
        let out = mem.scan_prefix("notesf", 10).await.unwrap();
        let topics: Vec<&str> = out.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(topics, vec!["notesfoo"]);
    }

    #[tokio::test]
    async fn scan_prefix_on_disk_caps_each_topic_independently() {
        let scratch = Scratch::new();
        let mem = open_mem(&scratch, 12).await;

        for i in 0..5 {
            mem.put("a", &format!("a{i}")).await.unwrap();
        }
        for i in 0..3 {
            mem.put("b", &format!("b{i}")).await.unwrap();
        }

        let out = mem.scan_prefix("", 2).await.unwrap();
        assert_eq!(out.len(), 2);
        let a_entries = &out.iter().find(|(t, _)| t == "a").unwrap().1;
        let b_entries = &out.iter().find(|(t, _)| t == "b").unwrap().1;
        assert_eq!(a_entries.len(), 2, "a must be capped at per_topic_limit");
        assert_eq!(b_entries.len(), 2, "b must be capped at per_topic_limit");
        // And newest-first within each group.
        assert!(a_entries[0].seq > a_entries[1].seq);
        assert!(b_entries[0].seq > b_entries[1].seq);
    }

    #[tokio::test]
    async fn scan_prefix_on_disk_excludes_metadata_keys() {
        // Metadata is stored under `m\x00...`; entries under `e\x00...`.
        // A scan_prefix with an empty topic prefix (→ key_prefix
        // `e\x00`) must only see entry rows. If the key-prefix
        // construction ever drops the ENTRY_PREFIX byte, metadata
        // would start appearing in results — this test catches that.
        let scratch = Scratch::new();
        let mem = open_mem(&scratch, 13).await;

        // Triggering at least one put ensures the metadata key
        // (`m\x00next_seq`) gets written — the counter is persisted
        // on every put.
        mem.put("notes", "x").await.unwrap();

        let out = mem.scan_prefix("", 10).await.unwrap();
        // Exactly one topic — "notes" — and no spurious metadata
        // topic from a miss-decoded `m\x00next_seq` key.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].0, "notes");
    }

    #[tokio::test]
    async fn scan_prefix_on_disk_rejects_zero_limit() {
        let scratch = Scratch::new();
        let mem = open_mem(&scratch, 14).await;
        mem.put("notes", "x").await.unwrap();
        assert!(matches!(
            mem.scan_prefix("", 0).await,
            Err(MemoryError::ZeroLimit)
        ));
    }

    // ---- Phase 42: gc_topic on disk ---------------------------------

    #[tokio::test]
    async fn gc_topic_on_disk_evicts_oldest() {
        let scratch = Scratch::new();
        let mem = open_mem(&scratch, 15).await;
        for i in 0..5 {
            mem.put("notes", &format!("e{i}")).await.unwrap();
        }
        let removed = mem.gc_topic("notes", 2).await.unwrap();
        assert_eq!(removed, 3);
        let remaining = mem.get_recent("notes", 10).await.unwrap();
        assert_eq!(remaining.len(), 2);
        // Newest survive: seq 4, 3
        assert_eq!(remaining[0].seq, 4);
        assert_eq!(remaining[1].seq, 3);
    }

    #[tokio::test]
    async fn gc_topic_on_disk_noop_when_under_cap() {
        let scratch = Scratch::new();
        let mem = open_mem(&scratch, 16).await;
        mem.put("notes", "a").await.unwrap();
        let removed = mem.gc_topic("notes", 10).await.unwrap();
        assert_eq!(removed, 0);
    }

    #[tokio::test]
    async fn gc_topic_on_disk_does_not_affect_other_topics() {
        let scratch = Scratch::new();
        let mem = open_mem(&scratch, 17).await;
        for i in 0..5 {
            mem.put("notes", &format!("n{i}")).await.unwrap();
        }
        mem.put("todos", "t0").await.unwrap();

        let removed = mem.gc_topic("notes", 2).await.unwrap();
        assert_eq!(removed, 3);
        // todos untouched
        let todos = mem.get_recent("todos", 10).await.unwrap();
        assert_eq!(todos.len(), 1);
    }

    // ---- Phase 42: gc_expired on disk --------------------------------

    #[tokio::test]
    async fn gc_expired_on_disk_removes_old_entries() {
        let scratch = Scratch::new();
        let mem = open_mem(&scratch, 18).await;
        mem.put("notes", "old").await.unwrap();
        mem.put("todos", "also-old").await.unwrap();

        // Cutoff in the future removes everything.
        let future = super::now_secs() + 100;
        let removed = mem.gc_expired(future).await.unwrap();
        assert_eq!(removed, 2);
        let notes = mem.get_recent("notes", 10).await.unwrap();
        let todos = mem.get_recent("todos", 10).await.unwrap();
        assert!(notes.is_empty());
        assert!(todos.is_empty());
    }

    #[tokio::test]
    async fn gc_expired_on_disk_keeps_fresh_entries() {
        let scratch = Scratch::new();
        let mem = open_mem(&scratch, 19).await;
        mem.put("notes", "fresh").await.unwrap();

        // Cutoff in the past keeps everything.
        let removed = mem.gc_expired(0).await.unwrap();
        assert_eq!(removed, 0);
        let notes = mem.get_recent("notes", 10).await.unwrap();
        assert_eq!(notes.len(), 1);
    }

    // ---- Phase 75 — vector store + cosine search ------------

    #[tokio::test]
    async fn put_vector_persists_and_index_rebuilds_on_reopen() {
        let scratch = Scratch::new();
        {
            let mem = open_mem(&scratch, 31).await;
            let s = mem.put("notes", "purple").await.unwrap();
            mem.put_vector("notes", s, vec![0.1, 0.2, 0.3])
                .await
                .unwrap();
        }
        // Reopen the same store + key: the in-memory index must
        // be rebuilt from the MemoryVectors domain.
        let mem = open_mem(&scratch, 31).await;
        let all = mem.load_all_vectors().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0, "notes");
        assert_eq!(all[0].2, vec![0.1, 0.2, 0.3]);
        let hits = mem
            .semantic_search(&[0.1, 0.2, 0.3], 5)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].body, "purple");
    }

    #[tokio::test]
    async fn semantic_search_ranks_most_similar_first() {
        let scratch = Scratch::new();
        let mem = open_mem(&scratch, 32).await;
        let s0 = mem.put("t", "near").await.unwrap();
        let s1 = mem.put("t", "far").await.unwrap();
        mem.put_vector("t", s0, vec![1.0, 0.0]).await.unwrap();
        mem.put_vector("t", s1, vec![0.0, 1.0]).await.unwrap();

        let hits = mem.semantic_search(&[1.0, 0.0], 2).await.unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].body, "near");
        assert_eq!(hits[1].body, "far");
    }

    #[tokio::test]
    async fn forget_drops_vectors_persistently() {
        let scratch = Scratch::new();
        {
            let mem = open_mem(&scratch, 33).await;
            let a = mem.put("keep", "a").await.unwrap();
            let b = mem.put("drop", "b").await.unwrap();
            mem.put_vector("keep", a, vec![1.0, 0.0]).await.unwrap();
            mem.put_vector("drop", b, vec![0.0, 1.0]).await.unwrap();
            mem.forget("drop").await.unwrap();
        }
        // Reopen: the dropped topic's vector must not be in the
        // rebuilt index either (it was deleted from the table).
        let mem = open_mem(&scratch, 33).await;
        let all = mem.load_all_vectors().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].0, "keep");
    }

    #[tokio::test]
    async fn evict_oldest_unread_drops_vectors_persistently() {
        let scratch = Scratch::new();
        {
            let mem = open_mem(&scratch, 34).await;
            let s0 = mem.put("t", "old").await.unwrap();
            let s1 = mem.put("t", "new").await.unwrap();
            mem.put_vector("t", s0, vec![1.0, 0.0]).await.unwrap();
            mem.put_vector("t", s1, vec![0.0, 1.0]).await.unwrap();
            // Read s1 so s0 is the LRU loser.
            let _ = mem.get_recent("t", 1).await.unwrap();
            assert_eq!(mem.evict_oldest_unread("t", 1).await.unwrap(), 1);
        }
        let mem = open_mem(&scratch, 34).await;
        let all = mem.load_all_vectors().await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].1, 1, "only the surviving entry's vector");
    }

    #[tokio::test]
    async fn semantic_search_skips_orphan_vector() {
        let scratch = Scratch::new();
        let mem = open_mem(&scratch, 35).await;
        let s = mem.put("t", "real").await.unwrap();
        mem.put_vector("t", s, vec![1.0, 0.0]).await.unwrap();
        // Orphan: vector for a (topic, seq) with no entry body.
        mem.put_vector("t", 999, vec![1.0, 0.0]).await.unwrap();

        let hits = mem.semantic_search(&[1.0, 0.0], 5).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].body, "real");
    }

    #[test]
    fn vector_codec_round_trips() {
        let v = vec![1.5f32, -2.25, 0.0, 3.125];
        let bytes = encode_vector(&v);
        assert_eq!(decode_vector(&bytes), Some(v));
        // A non-multiple-of-4 blob is rejected (not a panic).
        assert_eq!(decode_vector(&[0, 1, 2]), None);
    }

    #[test]
    fn vector_key_round_trips_topic_and_seq() {
        let key = RedbMemory::vector_key("notes/sub", 42);
        assert_eq!(
            parse_vector_key(&key),
            Some(("notes/sub".to_string(), 42))
        );
        // An entry key (different discriminator) is not a vector key.
        assert_eq!(parse_vector_key(&RedbMemory::entry_key("t", 1)), None);
    }
}
