//! Chapter Accord — contradiction detection over the Persona ("Soul").
//!
//! The self-consistency half of the identity stack, the sibling of Chapter
//! Concord (which does this for *memory*). The Soul accretes reflection-learned
//! facets under operator approval; the lifecycle layer merges near-*duplicates*
//! (Phase 87) and decays the *unreinforced* (Phase 85), but nothing ever
//! notices two approved facets that flatly *contradict* each other — a seeded
//! "communicate concisely" living alongside a later "give thorough, detailed
//! explanations", both injected into every prompt. This type is the wasm-clean
//! result of a detection pass over the Soul (and, cross-layer, against the
//! operator's declared Profile constraints).
//!
//! Detection is best-effort and on-demand (the operator runs `aivyx persona
//! conflicts` / the Studio asks) — zero background cost, no new config. It only
//! *finds*; resolution removes the losing facet via a normal `RemoveList`
//! persona delta on the signed chain (operator-authored, revertible).

use serde::{Deserialize, Serialize};

/// One side of a Soul contradiction: a single facet, named by its
/// `(category, value)` so a resolution can remove exactly that facet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoulFacet {
    /// The persona category the facet lives under (e.g.
    /// `character_traits`, `communication_adaptations`), or the sentinel
    /// `profile_constraint` for the operator-declared Profile side of a
    /// cross-layer conflict (which is immutable — never removable here).
    pub category: String,
    /// The facet text itself — the thing injected into the system prompt.
    pub value: String,
}

impl SoulFacet {
    /// The sentinel category for the operator's declared Profile
    /// behavioral-constraint side of a cross-layer conflict. Not a real
    /// persona-delta category — it marks an immutable side.
    pub const PROFILE_CONSTRAINT: &'static str = "profile_constraint";

    /// The sentinel category for a learned SKILL side of a conflict. The
    /// facet `value` is the skill's *name*; resolution removes that skill
    /// (by name), not a soft-list `RemoveList`.
    pub const LEARNED_SKILL: &'static str = "learned_skill";

    /// Whether this side is the immutable operator Profile constraint
    /// (so a resolution may only remove the *other* side).
    pub fn is_profile_constraint(&self) -> bool {
        self.category == Self::PROFILE_CONSTRAINT
    }

    /// Whether this side is a learned skill (removed by name on resolve).
    pub fn is_learned_skill(&self) -> bool {
        self.category == Self::LEARNED_SKILL
    }
}

/// A detected contradiction between two Soul facets — two learned facets, or
/// a learned facet against an operator Profile constraint (`cross_layer`).
///
/// `id` is a deterministic short hash of both `(category, value)` pairs —
/// stable across re-detection so a listed conflict can be referenced (or
/// resolved) without the daemon holding state between calls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoulConflict {
    pub id: String,
    pub a: SoulFacet,
    pub b: SoulFacet,
    /// The judge's one-line reason the two are incompatible.
    pub reason: String,
    /// True when side `b` is the operator's declared Profile constraint —
    /// the Soul has drifted against the operator's stated identity. Only
    /// side `a` (the learned facet) is removable in that case.
    pub cross_layer: bool,
}

impl SoulConflict {
    /// Deterministic short id for a `(category_a, value_a, category_b,
    /// value_b)` conflict. FNV-1a over the tuple, 12 lowercase hex chars —
    /// a display/reference handle (not a secret), stable so `list` then
    /// `resolve` agree without server state. The two sides are hashed in
    /// the order given, so callers should order them canonically first.
    pub fn make_id(category_a: &str, value_a: &str, category_b: &str, value_b: &str) -> String {
        let mut h: u64 = 0xcbf29ce484222325;
        let mut mix = |bytes: &[u8]| {
            for &byte in bytes {
                h ^= byte as u64;
                h = h.wrapping_mul(0x100000001b3);
            }
        };
        mix(category_a.as_bytes());
        mix(&[0]);
        mix(value_a.as_bytes());
        mix(&[0]);
        mix(category_b.as_bytes());
        mix(&[0]);
        mix(value_b.as_bytes());
        format!("{:012x}", h & 0xffff_ffff_ffff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_id_is_deterministic_and_order_sensitive() {
        let x = SoulConflict::make_id("character_traits", "concise", "character_traits", "verbose");
        let y = SoulConflict::make_id("character_traits", "concise", "character_traits", "verbose");
        assert_eq!(x, y, "same inputs → same id");
        assert_eq!(x.len(), 12);
        let z = SoulConflict::make_id("character_traits", "verbose", "character_traits", "concise");
        assert_ne!(x, z, "swapped sides → different id (order canonically first)");
    }

    #[test]
    fn profile_constraint_side_is_flagged() {
        let f = SoulFacet {
            category: SoulFacet::PROFILE_CONSTRAINT.to_string(),
            value: "never flatter me".to_string(),
        };
        assert!(f.is_profile_constraint());
        let g = SoulFacet {
            category: "character_traits".to_string(),
            value: "warm".to_string(),
        };
        assert!(!g.is_profile_constraint());
    }
}
