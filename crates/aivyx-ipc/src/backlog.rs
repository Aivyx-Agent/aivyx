//! Autonomous-loop backlog story view (Phase 173/177) — moved to `aivyx-ipc`
//! in M.2b.
//!
//! [`Story`] is the status-derived view of a backlog item, returned by the
//! `loop list` IPC (`QueryResponsePayload::LoopBacklog`). The signed chain it
//! is derived from, and the `PersistentLoopBacklog` that reads/writes it, stay
//! in `aivyx-channel`; only the wire-facing view lives here.

use serde::{Deserialize, Serialize};

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
    /// as the deterministic tiebreaker in the backlog's `next_pending`.
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
