//! Memory-conflict types (Chapter Concord) — the wasm-clean DTOs shared
//! by the daemon, the IPC protocol, and the CLI/Studio for surfacing
//! contradictory stored facts to the operator for resolution.
//!
//! A [`MemoryConflict`] is a *detected* pair of entries under one memory
//! topic that assert incompatible facts (e.g. "home airport is YPPH
//! (Perth)" vs "...Sydney, YSSY"). Conflicts are **derived** on demand by
//! an LLM pass over memory — never a second place facts live — so this
//! type is a transient report, not durable state. The operator resolves a
//! conflict by choosing which side is true; the other entry is deleted
//! (archived out of active recall) via `Memory::delete_entry`.
//!
//! The detector + judge live in `aivyx-channel` (they need the LLM
//! provider); these types stay here so they cross the IPC boundary and
//! compile to wasm without dragging native deps.

use serde::{Deserialize, Serialize};

/// One side of a conflict: a single memory entry, identified by its
/// `(topic, seq)` so the operator's resolution can name exactly which
/// entry to delete. Each side carries its own `topic` so a conflict can
/// span two DIFFERENT topics (the `home-airport` topic vs a stray
/// `operator-notes` fact), not just two entries under one topic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictSide {
    /// The topic this entry lives under — part of the delete key.
    pub topic: String,
    /// The entry's per-topic sequence number — the rest of the delete key.
    pub seq: u64,
    /// The entry body (what the fact says), for display.
    pub body: String,
    /// Wall-clock seconds when the entry was written, so the UI can mark
    /// which side is newer (0 when the substrate didn't record it).
    pub created_at_secs: u64,
}

/// A detected contradiction between two memory entries — same topic or
/// across two topics.
///
/// `a` is always the older side and `b` the newer (by `created_at_secs`,
/// then `topic`, then `seq`), so a "newest-wins" reading is `--keep b`.
/// `id` is a deterministic short hash of both `(topic, seq)` pairs —
/// stable across re-detection so a listed conflict can be referenced (or
/// durably dismissed) without the daemon holding state between calls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryConflict {
    pub id: String,
    pub a: ConflictSide,
    pub b: ConflictSide,
    /// The judge's one-line reason the two are incompatible.
    pub reason: String,
}

impl MemoryConflict {
    /// Deterministic short id for a `(topic_a, seq_a, topic_b, seq_b)`
    /// conflict. FNV-1a over the tuple, rendered as 12 lowercase hex chars
    /// — no crypto needed (it is a display/reference handle, not a
    /// secret), and stable so `list` then `resolve`/`dismiss` agree
    /// without server state.
    pub fn make_id(topic_a: &str, seq_a: u64, topic_b: &str, seq_b: u64) -> String {
        let mut h: u64 = 0xcbf29ce484222325;
        let mut mix = |bytes: &[u8]| {
            for &byte in bytes {
                h ^= byte as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
        };
        mix(topic_a.as_bytes());
        mix(&seq_a.to_be_bytes());
        mix(topic_b.as_bytes());
        mix(&seq_b.to_be_bytes());
        format!("{:012x}", h & 0xffff_ffff_ffff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_id_is_deterministic_and_order_sensitive() {
        let id1 = MemoryConflict::make_id("operator-note", 3, "operator-note", 7);
        let id2 = MemoryConflict::make_id("operator-note", 3, "operator-note", 7);
        assert_eq!(id1, id2, "same tuple → same id");
        assert_eq!(id1.len(), 12);
        assert!(id1.chars().all(|c| c.is_ascii_hexdigit()));
        // Different inputs → (almost surely) different ids.
        assert_ne!(id1, MemoryConflict::make_id("operator-note", 7, "operator-note", 3));
        assert_ne!(id1, MemoryConflict::make_id("other-topic", 3, "other-topic", 7));
        // Cross-topic vs same-topic with matching seqs differ.
        assert_ne!(id1, MemoryConflict::make_id("home-airport", 3, "operator-note", 7));
    }
}
