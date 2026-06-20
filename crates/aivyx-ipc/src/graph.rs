//! Typed knowledge-graph types (Chapter Lattice) — the wasm-clean DTOs
//! shared by the daemon, the IPC protocol, the `graph.query` tool, and
//! the Studio graph view.
//!
//! The graph is a set of **directed, typed triples**: `(subject)
//! —[predicate]→ (object)`. Subject and object are **entity** strings
//! (canonicalized for dedup); the predicate is a **free-text relation
//! label** (`depends-on`, `caused`, `owns`) — open-vocabulary, not a
//! fixed enum. Each triple carries provenance (the memory entry `seq`s it
//! was extracted from), a `mentions`-derived weight, and an `updated_at`.
//!
//! Like the wiki DTOs, these live here so they cross the IPC boundary and
//! compile to wasm for the Studio without dragging native deps. The
//! persistent store + the extractor that fills them in live in
//! `aivyx-channel` (they need encrypted storage + the LLM provider).

use serde::{Deserialize, Serialize};

/// Canonicalize an entity or predicate string for dedup: lowercase
/// (ASCII fold), trim, and collapse internal whitespace runs to one
/// space. Deliberately **does not stem** (unlike the topic canonicalizer)
/// — an entity name like `settings` must not fold to `setting`, and a
/// predicate like `depends on` should stay distinct from `depend`. Empty
/// input (or all-whitespace) yields an empty string, which callers reject.
pub fn canonical_label(s: &str) -> String {
    let lowered = s.trim().to_lowercase();
    lowered.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One directed, typed relation: `subject —[predicate]→ object`.
///
/// `subject` / `object` are canonical entity strings; `predicate` is a
/// free-text relation label. `source_seqs` are the memory entries the
/// triple was extracted from (provenance); `mentions` is how many entries
/// assert it (the legible weight — not a hallucinated confidence);
/// `updated_at` is the wall-clock second it was last (re)extracted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphTriple {
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub source_seqs: Vec<u64>,
    pub mentions: u32,
    pub updated_at: u64,
}

impl GraphTriple {
    /// The deterministic storage key for a triple: its three canonical
    /// parts joined by NUL (which cannot appear in a canonical entity or
    /// predicate), so re-extracting the same fact upserts the same row
    /// rather than duplicating it. Direction matters — `(a, r, b)` and
    /// `(b, r, a)` are distinct keys.
    pub fn key(subject: &str, predicate: &str, object: &str) -> Vec<u8> {
        let mut k = Vec::with_capacity(subject.len() + predicate.len() + object.len() + 2);
        k.extend_from_slice(subject.as_bytes());
        k.push(0);
        k.extend_from_slice(predicate.as_bytes());
        k.push(0);
        k.extend_from_slice(object.as_bytes());
        k
    }

    /// This triple's own storage key.
    pub fn storage_key(&self) -> Vec<u8> {
        Self::key(&self.subject, &self.predicate, &self.object)
    }
}

/// A node in the graph view: a distinct entity plus how many triples
/// touch it (degree) and an optional LLM-inferred kind (`person` /
/// `system` / `concept` / …). Derived from the triple set.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphEntity {
    pub name: String,
    /// Number of triples this entity participates in (as subject or
    /// object) — its degree, for sizing in the Studio render.
    pub degree: u32,
    /// Optional inferred kind; empty when unknown.
    #[serde(default)]
    pub kind: String,
}

/// One step of a `graph.query` traversal result: the entity reached, the
/// number of hops from the start, and the typed path taken to get there
/// (each element a `predicate` label, in order). The final `predicate`
/// points *into* `entity`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphPath {
    pub entity: String,
    pub hops: u32,
    /// The predicate labels along the path, start → entity.
    pub path: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_direction_sensitive_and_stable() {
        let ab = GraphTriple::key("deploy", "depends-on", "ci");
        let ba = GraphTriple::key("ci", "depends-on", "deploy");
        assert_ne!(ab, ba, "direction matters");
        assert_eq!(ab, GraphTriple::key("deploy", "depends-on", "ci"), "stable");
        // A different predicate is a different edge.
        assert_ne!(ab, GraphTriple::key("deploy", "triggers", "ci"));
    }

    #[test]
    fn key_uses_nul_separators_no_collision() {
        // "a","b","c" vs "a\0b","","c" must not collide — NUL is the
        // separator, and canonical entities never contain it.
        let k1 = GraphTriple::key("a", "b", "c");
        let k2 = GraphTriple::key("ab", "", "c");
        assert_ne!(k1, k2);
    }

    #[test]
    fn triple_round_trips_through_json() {
        let t = GraphTriple {
            subject: "deploy".into(),
            predicate: "depends-on".into(),
            object: "ci".into(),
            source_seqs: vec![1, 4, 9],
            mentions: 3,
            updated_at: 1000,
        };
        let bytes = serde_json::to_vec(&t).unwrap();
        let back: GraphTriple = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(t, back);
        assert_eq!(back.storage_key(), GraphTriple::key("deploy", "depends-on", "ci"));
    }

    #[test]
    fn canonical_label_lowercases_trims_collapses_no_stem() {
        assert_eq!(canonical_label("  Deploy   Pipeline "), "deploy pipeline");
        assert_eq!(canonical_label("CI"), "ci");
        // No stemming: plurals/inflections are preserved.
        assert_eq!(canonical_label("Settings"), "settings");
        assert_eq!(canonical_label("depends on"), "depends on");
        assert_eq!(canonical_label("   "), "");
    }

    #[test]
    fn entity_kind_defaults_empty_when_absent() {
        // Older/partial rows without `kind` decode with an empty string.
        let e: GraphEntity =
            serde_json::from_str(r#"{"name":"ci","degree":2}"#).unwrap();
        assert_eq!(e.kind, "");
        assert_eq!(e.degree, 2);
    }
}
