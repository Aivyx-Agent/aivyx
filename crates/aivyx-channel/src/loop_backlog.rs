//! Phase 173 — the autonomous-loop backlog substrate.
//!
//! The Aivyx-native analog of Ralph's `prd.json`: an ordered
//! list of **stories** the loop driver works through one per
//! fresh-context iteration. Structurally a sibling of
//! [`crate::persona_proposal`] — an **HMAC-chained, append-only
//! log** over the HKDF-isolated
//! [`aivyx_storage::KeyDomain::LoopBacklog`] — so the backlog is
//! tamper-evident, capability-gated, and survives independent of
//! the working repo.
//!
//! ## Shape
//!
//! - A **story** is the unit of work: a stable `id`, a numeric
//!   `priority` (lower runs first), a one-line `title`, an
//!   optional multi-line `body`, and a status of
//!   `Pending | Done | Skipped`.
//! - Each row is a [`SignedStoryEntry`], HMAC-chained against
//!   the previous entry. The first entry for any id is a
//!   `Created`; later `Done` / `Skipped` entries for the same
//!   id append rather than mutate (the chain only signs
//!   immutable bytes, exactly the Phase 70 rule).
//! - The "current status of story X" is derived from the last
//!   entry referencing X.
//!
//! ## Why append-only-with-status-entries
//!
//! Same two reasons as the persona-proposal chain: the HMAC
//! chain only signs immutable bytes (mutating a status in place
//! would break it), and the story *history* is the audit story
//! — an operator reviewing a finished loop run sees exactly when
//! each story was created and completed.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sha2::Sha256;

use aivyx_storage::DomainHandle;

// ---------------------------------------------------------------------------
// Story — derived view returned by list/get/next APIs.
// ---------------------------------------------------------------------------

/// A single backlog story, status-derived from the chain. `id`
/// is stable across status transitions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Story {
    /// Stable story id. Generated when the story is first
    /// appended in `Created` status.
    pub id: String,
    /// Lower runs first. Ties broken by insertion order (the
    /// `Created` entry's seq).
    pub priority: u32,
    /// One-line summary the loop prompt surfaces to the agent.
    pub title: String,
    /// Optional multi-line detail / acceptance criteria.
    pub body: String,
    /// Wall-clock when the story was first appended.
    pub created_at_unix_ms: u64,
    /// Insertion order — the `Created` entry's chain seq. Used
    /// as the deterministic tiebreaker in [`BacklogChainLog::next_pending`].
    pub created_seq: u64,
    /// Current status, derived from the latest chain entry
    /// referencing this story's `id`.
    pub status: StoryStatus,
}

/// Status of a story. Transitions are encoded as new signed
/// chain entries; this enum is the derived view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum StoryStatus {
    /// Not yet worked — the loop driver will pick it up.
    Pending,
    /// The agent marked it complete (gates passed + committed,
    /// per the canonical loop prompt's discipline).
    Done { resolved_at_unix_ms: u64 },
    /// Skipped — the operator (or agent) set it aside without
    /// completing it. The optional `reason` is preserved.
    Skipped {
        reason: Option<String>,
        resolved_at_unix_ms: u64,
    },
}

// ---------------------------------------------------------------------------
// Signed chain entry — what gets written to KeyDomain::LoopBacklog.
// ---------------------------------------------------------------------------

/// One signed entry in the backlog chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedStoryEntry {
    /// Monotonic, zero-indexed across the whole chain (not per
    /// story id).
    pub seq: u64,
    /// The story id this entry refers to. Multiple entries share
    /// the same id when status transitions append new rows.
    pub story_id: String,
    /// Wall-clock when this entry was appended.
    pub at_unix_ms: u64,
    /// What this entry asserts about the story. The first entry
    /// for any given `story_id` is always `Created`.
    pub body: StoryEntryBody,
    /// MAC of the entry preceding this one.
    pub prev_mac: [u8; 32],
    /// MAC over `prev_mac || serde_jcs(body envelope)`.
    pub mac: [u8; 32],
}

/// The signed payload of a chain entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum StoryEntryBody {
    /// First entry per story id; carries the story content.
    Created {
        priority: u32,
        title: String,
        body: String,
    },
    /// The story was completed.
    Done,
    /// The story was set aside.
    Skipped { reason: Option<String> },
}

// ---------------------------------------------------------------------------
// Errors.
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum BacklogError {
    #[error("backlog story validation failed: {reason}")]
    InvalidStory { reason: String },
    #[error("backlog chain broken at seq {seq}: {reason}")]
    ChainBroken { seq: u64, reason: String },
    #[error("backlog chain serialize failed: {0}")]
    Serialize(String),
    #[error("backlog chain storage failed: {0}")]
    Storage(String),
    #[error("backlog story `{0}` not found")]
    UnknownStory(String),
    #[error(
        "backlog story `{story_id}` cannot transition from {current} to {requested}"
    )]
    InvalidTransition {
        story_id: String,
        current: &'static str,
        requested: &'static str,
    },
}

// ---------------------------------------------------------------------------
// In-memory chain.
// ---------------------------------------------------------------------------

/// Distinct from every other chain's genesis seed so a
/// chain-confusion attack (a backlog entry slotted into the
/// persona chain, or vice versa) is structurally rejected. Must
/// fit in the 32-byte genesis buffer.
const BACKLOG_GENESIS_SEED: &[u8] = b"aivyx-loop-backlog-genesis-v1";

/// Max title length — a one-liner, not an essay.
const MAX_TITLE_LEN: usize = 200;

/// In-memory backlog chain. Owns its HMAC key + the ordered
/// entry vector. Persistence is layered on top via
/// [`PersistentLoopBacklog`].
pub struct BacklogChainLog {
    key: Vec<u8>,
    entries: std::sync::Mutex<Vec<SignedStoryEntry>>,
}

impl BacklogChainLog {
    pub fn new(key: impl Into<Vec<u8>>) -> Self {
        BacklogChainLog {
            key: key.into(),
            entries: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn from_verified_entries(
        key: impl Into<Vec<u8>>,
        entries: Vec<SignedStoryEntry>,
    ) -> Self {
        BacklogChainLog {
            key: key.into(),
            entries: std::sync::Mutex::new(entries),
        }
    }

    /// Append a `Created` entry for a fresh story.
    pub fn append_created(
        &self,
        story_id: String,
        at_unix_ms: u64,
        priority: u32,
        title: String,
        body: String,
    ) -> Result<u64, BacklogError> {
        let trimmed = title.trim();
        if trimmed.is_empty() {
            return Err(BacklogError::InvalidStory {
                reason: "story title must be non-empty".into(),
            });
        }
        if trimmed.chars().count() > MAX_TITLE_LEN {
            return Err(BacklogError::InvalidStory {
                reason: format!(
                    "story title exceeds {MAX_TITLE_LEN} characters"
                ),
            });
        }
        let entry_body = StoryEntryBody::Created {
            priority,
            title: trimmed.to_string(),
            body,
        };
        self.append_entry(story_id, at_unix_ms, entry_body)
    }

    /// Append a `Done` entry for an existing pending story.
    pub fn append_done(
        &self,
        story_id: String,
        at_unix_ms: u64,
    ) -> Result<u64, BacklogError> {
        self.assert_pending(&story_id, "Done")?;
        self.append_entry(story_id, at_unix_ms, StoryEntryBody::Done)
    }

    /// Append a `Skipped` entry for an existing pending story.
    pub fn append_skipped(
        &self,
        story_id: String,
        at_unix_ms: u64,
        reason: Option<String>,
    ) -> Result<u64, BacklogError> {
        self.assert_pending(&story_id, "Skipped")?;
        self.append_entry(
            story_id,
            at_unix_ms,
            StoryEntryBody::Skipped { reason },
        )
    }

    fn assert_pending(
        &self,
        story_id: &str,
        requested: &'static str,
    ) -> Result<(), BacklogError> {
        let view = self
            .get(story_id)
            .ok_or_else(|| BacklogError::UnknownStory(story_id.to_string()))?;
        match &view.status {
            StoryStatus::Pending => Ok(()),
            StoryStatus::Done { .. } => Err(BacklogError::InvalidTransition {
                story_id: story_id.to_string(),
                current: "Done",
                requested,
            }),
            StoryStatus::Skipped { .. } => Err(BacklogError::InvalidTransition {
                story_id: story_id.to_string(),
                current: "Skipped",
                requested,
            }),
        }
    }

    fn append_entry(
        &self,
        story_id: String,
        at_unix_ms: u64,
        body: StoryEntryBody,
    ) -> Result<u64, BacklogError> {
        let envelope = MacEnvelope {
            story_id: &story_id,
            at_unix_ms,
            body: &body,
        };
        let body_bytes = serde_jcs::to_vec(&envelope)
            .map_err(|e| BacklogError::Serialize(e.to_string()))?;
        let mut inner = self.entries.lock().unwrap();
        let seq = inner.len() as u64;
        let prev_mac = match inner.last() {
            Some(prev) => prev.mac,
            None => genesis_prev_mac(),
        };
        let mac = compute_mac(&self.key, &prev_mac, &body_bytes);
        inner.push(SignedStoryEntry {
            seq,
            story_id,
            at_unix_ms,
            body,
            prev_mac,
            mac,
        });
        Ok(seq)
    }

    /// All entries (cloned, chain order).
    pub fn entries(&self) -> Vec<SignedStoryEntry> {
        self.entries.lock().unwrap().clone()
    }

    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.lock().unwrap().is_empty()
    }

    /// Walk every entry; check `prev_mac` linkage + recompute
    /// every `mac`. Returns the first broken link or `Ok(())`.
    pub fn verify(&self) -> Result<(), BacklogError> {
        let inner = self.entries.lock().unwrap();
        let mut expected_prev = genesis_prev_mac();
        for (idx, entry) in inner.iter().enumerate() {
            if entry.seq != idx as u64 {
                return Err(BacklogError::ChainBroken {
                    seq: entry.seq,
                    reason: format!("expected seq {idx}, got {}", entry.seq),
                });
            }
            if entry.prev_mac != expected_prev {
                return Err(BacklogError::ChainBroken {
                    seq: entry.seq,
                    reason: "prev_mac does not match previous entry's mac".into(),
                });
            }
            let envelope = MacEnvelope {
                story_id: &entry.story_id,
                at_unix_ms: entry.at_unix_ms,
                body: &entry.body,
            };
            let body_bytes = serde_jcs::to_vec(&envelope)
                .map_err(|e| BacklogError::Serialize(e.to_string()))?;
            let recomputed = compute_mac(&self.key, &entry.prev_mac, &body_bytes);
            if recomputed != entry.mac {
                return Err(BacklogError::ChainBroken {
                    seq: entry.seq,
                    reason: "mac does not match recomputed value (tampered entry?)".into(),
                });
            }
            expected_prev = entry.mac;
        }
        Ok(())
    }

    /// Derive the current view of a single story, or `None` if
    /// no entries reference that id.
    pub fn get(&self, story_id: &str) -> Option<Story> {
        let entries = self.entries.lock().unwrap();
        derive_one(&entries, story_id)
    }

    /// Derive every distinct story, optionally filtered by
    /// status discriminant. Insertion order (first-seen).
    pub fn list(&self, filter: StoryStatusFilter) -> Vec<Story> {
        let entries = self.entries.lock().unwrap();
        derive_all(&entries, filter)
    }

    /// The next story the loop driver should work: the
    /// lowest-`priority` `Pending` story, ties broken by
    /// `created_seq` (insertion order). `None` → the backlog is
    /// complete (no pending stories remain).
    pub fn next_pending(&self) -> Option<Story> {
        let entries = self.entries.lock().unwrap();
        let mut pending: Vec<Story> =
            derive_all(&entries, StoryStatusFilter::Pending);
        pending.sort_by(|a, b| {
            a.priority
                .cmp(&b.priority)
                .then(a.created_seq.cmp(&b.created_seq))
        });
        pending.into_iter().next()
    }

    /// How many stories are still `Pending`.
    pub fn remaining_count(&self) -> usize {
        let entries = self.entries.lock().unwrap();
        derive_all(&entries, StoryStatusFilter::Pending).len()
    }
}

/// Status filter for [`BacklogChainLog::list`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoryStatusFilter {
    All,
    Pending,
    Done,
    Skipped,
}

impl StoryStatusFilter {
    fn accepts(self, status: &StoryStatus) -> bool {
        matches!(
            (self, status),
            (StoryStatusFilter::All, _)
                | (StoryStatusFilter::Pending, StoryStatus::Pending)
                | (StoryStatusFilter::Done, StoryStatus::Done { .. })
                | (StoryStatusFilter::Skipped, StoryStatus::Skipped { .. })
        )
    }
}

fn derive_one(entries: &[SignedStoryEntry], story_id: &str) -> Option<Story> {
    let mut id_entries: Vec<&SignedStoryEntry> = entries
        .iter()
        .filter(|e| e.story_id == story_id)
        .collect();
    id_entries.sort_by_key(|e| e.seq);
    derive_from_id_entries(&id_entries)
}

fn derive_all(
    entries: &[SignedStoryEntry],
    filter: StoryStatusFilter,
) -> Vec<Story> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<&str, Vec<&SignedStoryEntry>> = HashMap::new();
    for entry in entries {
        if !groups.contains_key(entry.story_id.as_str()) {
            order.push(entry.story_id.clone());
        }
        groups
            .entry(entry.story_id.as_str())
            .or_default()
            .push(entry);
    }
    let mut out = Vec::with_capacity(order.len());
    for id in &order {
        if let Some(ents) = groups.get(id.as_str()) {
            if let Some(view) = derive_from_id_entries(ents) {
                if filter.accepts(&view.status) {
                    out.push(view);
                }
            }
        }
    }
    out
}

fn derive_from_id_entries(entries: &[&SignedStoryEntry]) -> Option<Story> {
    let first = entries.first()?;
    let (priority, title, body) = match &first.body {
        StoryEntryBody::Created {
            priority,
            title,
            body,
        } => (*priority, title.clone(), body.clone()),
        _ => return None, // chain invariant violation; defensive None
    };
    let last = entries.last()?;
    let status = match &last.body {
        StoryEntryBody::Created { .. } => StoryStatus::Pending,
        StoryEntryBody::Done => StoryStatus::Done {
            resolved_at_unix_ms: last.at_unix_ms,
        },
        StoryEntryBody::Skipped { reason } => StoryStatus::Skipped {
            reason: reason.clone(),
            resolved_at_unix_ms: last.at_unix_ms,
        },
    };
    Some(Story {
        id: first.story_id.clone(),
        priority,
        title,
        body,
        created_at_unix_ms: first.at_unix_ms,
        created_seq: first.seq,
        status,
    })
}

// Envelope serialized for MAC computation. Stable JCS shape.
#[derive(Serialize)]
struct MacEnvelope<'a> {
    story_id: &'a str,
    at_unix_ms: u64,
    body: &'a StoryEntryBody,
}

fn genesis_prev_mac() -> [u8; 32] {
    let mut out = [0u8; 32];
    let src = BACKLOG_GENESIS_SEED;
    let start = out.len() - src.len();
    out[start..].copy_from_slice(src);
    out
}

fn compute_mac(key: &[u8], prev_mac: &[u8; 32], body: &[u8]) -> [u8; 32] {
    use hmac::{Hmac, KeyInit, Mac};
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = <HmacSha256 as KeyInit>::new_from_slice(key)
        .expect("HMAC accepts any key length");
    mac.update(prev_mac);
    mac.update(body);
    let out = mac.finalize().into_bytes();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    arr
}

// ---------------------------------------------------------------------------
// Persistent wrapper — talks to KeyDomain::LoopBacklog.
// ---------------------------------------------------------------------------

/// Persistent backlog chain backed by
/// [`aivyx_storage::KeyDomain::LoopBacklog`]. One redb row per
/// signed entry, keyed by big-endian u64 seq so scan reads
/// return chain-ordered.
pub struct PersistentLoopBacklog {
    chain: BacklogChainLog,
    storage: DomainHandle,
}

impl PersistentLoopBacklog {
    /// Open (or initialize) the persistent backlog. Reads every
    /// row, reconstructs the chain, verifies linkage, and bails
    /// on tamper.
    pub async fn open(
        storage: DomainHandle,
        key: Vec<u8>,
    ) -> Result<Self, BacklogError> {
        let rows = storage
            .scan_prefix(&[])
            .await
            .map_err(|e| BacklogError::Storage(e.to_string()))?;
        let mut entries: Vec<SignedStoryEntry> = Vec::with_capacity(rows.len());
        for (_k, v) in rows {
            let entry: SignedStoryEntry = serde_json::from_slice(&v)
                .map_err(|e| BacklogError::Serialize(e.to_string()))?;
            entries.push(entry);
        }
        entries.sort_by_key(|e| e.seq);
        let verify_chain =
            BacklogChainLog::from_verified_entries(key.clone(), entries.clone());
        verify_chain.verify()?;
        Ok(PersistentLoopBacklog {
            chain: BacklogChainLog::from_verified_entries(key, entries),
            storage,
        })
    }

    /// Append a `Created` entry + persist it. Returns the seq.
    pub async fn add_story(
        &self,
        story_id: String,
        at_unix_ms: u64,
        priority: u32,
        title: String,
        body: String,
    ) -> Result<u64, BacklogError> {
        let seq = self
            .chain
            .append_created(story_id, at_unix_ms, priority, title, body)?;
        self.persist_last(seq).await
    }

    /// Mark a pending story `Done` + persist it.
    pub async fn mark_done(
        &self,
        story_id: String,
        at_unix_ms: u64,
    ) -> Result<u64, BacklogError> {
        let seq = self.chain.append_done(story_id, at_unix_ms)?;
        self.persist_last(seq).await
    }

    /// Mark a pending story `Skipped` + persist it.
    pub async fn mark_skipped(
        &self,
        story_id: String,
        at_unix_ms: u64,
        reason: Option<String>,
    ) -> Result<u64, BacklogError> {
        let seq = self.chain.append_skipped(story_id, at_unix_ms, reason)?;
        self.persist_last(seq).await
    }

    async fn persist_last(&self, seq: u64) -> Result<u64, BacklogError> {
        let entries = self.chain.entries();
        let last = entries.last().expect("just appended");
        let key = seq.to_be_bytes();
        let value = serde_json::to_vec(last)
            .map_err(|e| BacklogError::Serialize(e.to_string()))?;
        self.storage
            .put(&key, &value)
            .await
            .map_err(|e| BacklogError::Storage(e.to_string()))?;
        Ok(seq)
    }

    pub fn get(&self, story_id: &str) -> Option<Story> {
        self.chain.get(story_id)
    }

    pub fn list(&self, filter: StoryStatusFilter) -> Vec<Story> {
        self.chain.list(filter)
    }

    pub fn next_pending(&self) -> Option<Story> {
        self.chain.next_pending()
    }

    pub fn remaining_count(&self) -> usize {
        self.chain.remaining_count()
    }

    pub fn verify(&self) -> Result<(), BacklogError> {
        self.chain.verify()
    }

    pub fn entries(&self) -> Vec<SignedStoryEntry> {
        self.chain.entries()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_crypto::MasterKey;
    use aivyx_storage::{KeyDomain, RedbStorage, Storage, StorageConfig};
    use std::path::PathBuf;
    use std::sync::Arc;

    fn chain() -> BacklogChainLog {
        BacklogChainLog::new(b"backlog-test-key".to_vec())
    }

    #[test]
    fn add_then_list_returns_one_pending_story() {
        let c = chain();
        let seq = c
            .append_created("s-1".into(), 1_000, 5, "Fix login".into(), "details".into())
            .expect("add");
        assert_eq!(seq, 0);
        let all = c.list(StoryStatusFilter::All);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "s-1");
        assert_eq!(all[0].priority, 5);
        assert_eq!(all[0].title, "Fix login");
        assert_eq!(all[0].body, "details");
        assert!(matches!(all[0].status, StoryStatus::Pending));
    }

    #[test]
    fn empty_title_is_rejected() {
        let c = chain();
        assert!(matches!(
            c.append_created("s".into(), 1, 0, "   ".into(), "".into()),
            Err(BacklogError::InvalidStory { .. })
        ));
        assert!(c.is_empty());
    }

    #[test]
    fn title_is_trimmed() {
        let c = chain();
        c.append_created("s".into(), 1, 0, "  hello  ".into(), "".into())
            .unwrap();
        assert_eq!(c.get("s").unwrap().title, "hello");
    }

    #[test]
    fn done_status_overrides_created_for_same_id() {
        let c = chain();
        c.append_created("s".into(), 1, 0, "t".into(), "".into())
            .unwrap();
        c.append_done("s".into(), 2).unwrap();
        let v = c.get("s").unwrap();
        assert!(matches!(
            v.status,
            StoryStatus::Done { resolved_at_unix_ms: 2 }
        ));
    }

    #[test]
    fn cannot_complete_an_already_done_story() {
        let c = chain();
        c.append_created("s".into(), 1, 0, "t".into(), "".into())
            .unwrap();
        c.append_done("s".into(), 2).unwrap();
        assert!(matches!(
            c.append_done("s".into(), 3),
            Err(BacklogError::InvalidTransition { current: "Done", .. })
        ));
    }

    #[test]
    fn cannot_skip_an_already_resolved_story() {
        let c = chain();
        c.append_created("s".into(), 1, 0, "t".into(), "".into())
            .unwrap();
        c.append_done("s".into(), 2).unwrap();
        // A done story can't be skipped (the Phase 177 `loop skip`
        // path relies on this guard).
        assert!(matches!(
            c.append_skipped("s".into(), 3, None),
            Err(BacklogError::InvalidTransition { current: "Done", .. })
        ));
    }

    #[test]
    fn completing_unknown_story_errors() {
        let c = chain();
        assert!(matches!(
            c.append_done("nope".into(), 1),
            Err(BacklogError::UnknownStory(_))
        ));
    }

    #[test]
    fn next_pending_is_priority_then_insertion_order() {
        let c = chain();
        // Insert out of priority order; same-priority ties break
        // by insertion (created_seq).
        c.append_created("a".into(), 10, 5, "a".into(), "".into())
            .unwrap();
        c.append_created("b".into(), 11, 1, "b".into(), "".into())
            .unwrap();
        c.append_created("c".into(), 12, 1, "c".into(), "".into())
            .unwrap();
        // Lowest priority (1) first; between b and c, b was
        // created first.
        assert_eq!(c.next_pending().unwrap().id, "b");
        // Completing b advances to c (same priority, later seq).
        c.append_done("b".into(), 20).unwrap();
        assert_eq!(c.next_pending().unwrap().id, "c");
        // Then the priority-5 story.
        c.append_done("c".into(), 21).unwrap();
        assert_eq!(c.next_pending().unwrap().id, "a");
        // Empty backlog → None.
        c.append_done("a".into(), 22).unwrap();
        assert!(c.next_pending().is_none());
    }

    #[test]
    fn skipped_stories_are_not_pending() {
        let c = chain();
        c.append_created("a".into(), 1, 0, "a".into(), "".into())
            .unwrap();
        c.append_skipped("a".into(), 2, Some("not needed".into()))
            .unwrap();
        assert_eq!(c.remaining_count(), 0);
        assert!(c.next_pending().is_none());
        assert_eq!(c.list(StoryStatusFilter::Skipped).len(), 1);
    }

    #[test]
    fn remaining_count_tracks_pending_only() {
        let c = chain();
        c.append_created("a".into(), 1, 0, "a".into(), "".into())
            .unwrap();
        c.append_created("b".into(), 1, 0, "b".into(), "".into())
            .unwrap();
        assert_eq!(c.remaining_count(), 2);
        c.append_done("a".into(), 2).unwrap();
        assert_eq!(c.remaining_count(), 1);
    }

    #[test]
    fn verify_detects_tamper() {
        let c = chain();
        c.append_created("a".into(), 1, 0, "a".into(), "".into())
            .unwrap();
        c.append_done("a".into(), 2).unwrap();
        assert!(c.verify().is_ok());
        // Tamper: flip a stored title via a hand-built chain.
        let mut entries = c.entries();
        if let StoryEntryBody::Created { title, .. } = &mut entries[0].body {
            *title = "TAMPERED".into();
        }
        let tampered = BacklogChainLog::from_verified_entries(
            b"backlog-test-key".to_vec(),
            entries,
        );
        assert!(matches!(
            tampered.verify(),
            Err(BacklogError::ChainBroken { .. })
        ));
    }

    // ---- Persistent round-trip ---------------------------------

    struct Scratch {
        dir: PathBuf,
    }
    impl Scratch {
        fn new() -> Self {
            let base =
                std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into());
            let dir = PathBuf::from(base).join(format!(
                "aivyx-loop-backlog-test-{}",
                uuid::Uuid::new_v4()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Scratch { dir }
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    async fn open_store(s: &Scratch, mb: u8) -> Arc<dyn Storage> {
        RedbStorage::open(
            StorageConfig::new(s.dir.join("store.redb")),
            MasterKey::from_raw([mb; 32]),
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn persistent_round_trip_survives_reopen() {
        let s = Scratch::new();
        {
            let store = open_store(&s, 7).await;
            let bl = PersistentLoopBacklog::open(
                store.domain(KeyDomain::LoopBacklog),
                b"k".to_vec(),
            )
            .await
            .unwrap();
            bl.add_story("a".into(), 1, 1, "first".into(), "".into())
                .await
                .unwrap();
            bl.add_story("b".into(), 2, 2, "second".into(), "".into())
                .await
                .unwrap();
            bl.mark_done("a".into(), 3).await.unwrap();
        }
        // Reopen: chain verifies, derived state survives.
        let store = open_store(&s, 7).await;
        let bl = PersistentLoopBacklog::open(
            store.domain(KeyDomain::LoopBacklog),
            b"k".to_vec(),
        )
        .await
        .unwrap();
        assert!(bl.verify().is_ok());
        assert_eq!(bl.remaining_count(), 1);
        assert_eq!(bl.next_pending().unwrap().id, "b");
        assert!(matches!(
            bl.get("a").unwrap().status,
            StoryStatus::Done { .. }
        ));
    }

    #[tokio::test]
    async fn reopen_with_wrong_key_fails_verification() {
        let s = Scratch::new();
        {
            let store = open_store(&s, 8).await;
            let bl = PersistentLoopBacklog::open(
                store.domain(KeyDomain::LoopBacklog),
                b"right-key".to_vec(),
            )
            .await
            .unwrap();
            bl.add_story("a".into(), 1, 1, "first".into(), "".into())
                .await
                .unwrap();
        }
        let store = open_store(&s, 8).await;
        let reopened = PersistentLoopBacklog::open(
            store.domain(KeyDomain::LoopBacklog),
            b"wrong-key".to_vec(),
        )
        .await;
        assert!(matches!(reopened, Err(BacklogError::ChainBroken { .. })));
    }
}
