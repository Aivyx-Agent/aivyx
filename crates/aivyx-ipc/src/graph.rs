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

/// Chapter Lexicon — the curated controlled vocabulary of relation types.
/// Each entry is `(canonical_key, &[synonym_phrases])`; the synonyms are
/// already in [`canonical_label`] form (lowercase, single-spaced) so they
/// compare directly against a cleaned predicate. Same-direction phrasings
/// only — inverse phrasings (`owned by`, `caused by`) are deliberately
/// absent (folding them would flip subject↔object); they fall through to
/// the open-world fallback. Kept deliberately small: a big lexicon is just
/// open-world with extra steps.
pub const RELATION_LEXICON: &[(&str, &[&str])] = &[
    ("depends-on", &["depends on", "depend on", "requires", "require", "needs", "need", "relies on", "rely on", "dependent on"]),
    ("uses", &["use", "utilizes", "utilize", "leverages", "leverage"]),
    ("causes", &["cause", "caused", "leads to", "lead to", "led to", "results in", "result in", "resulted in", "triggers", "trigger", "triggered"]),
    ("part-of", &["part of", "belongs to", "belong to", "contained in", "component of", "member of", "subset of"]),
    ("contains", &["contain", "includes", "include", "comprises", "comprise", "has part"]),
    ("related-to", &["related to", "relates to", "relate to", "associated with", "associates with", "linked to", "links to", "connected to", "connects to"]),
    ("located-in", &["located in", "resides in", "reside in", "hosted in", "runs in", "run in", "lives in", "live in"]),
    ("created-by", &["created by", "authored by", "made by", "built by", "written by", "developed by"]),
    ("produces", &["produce", "produced", "generates", "generate", "outputs", "output", "emits", "emit"]),
    ("instance-of", &["instance of", "is a", "is an", "type of", "a type of", "kind of", "a kind of", "an example of", "example of"]),
    ("replaces", &["replace", "replaced", "supersedes", "supersede", "deprecates", "deprecate", "succeeds", "succeed"]),
    ("owns", &["own", "owner of", "maintains", "maintain", "responsible for", "manages", "manage"]),
    ("precedes", &["precede", "comes before", "before", "preceded"]),
    ("follows", &["follow", "comes after", "after", "followed"]),
];

/// Chapter Lexicon — fold a free-text predicate into the controlled
/// vocabulary: clean it ([`canonical_label`]), map it through
/// [`RELATION_LEXICON`] to its canonical relation type, and — when no
/// synonym matches — keep the cleaned label (open-world fallback, so a
/// genuinely-new relation is never discarded). The single source of truth
/// the extractor, `graph.query`'s filter, and the re-normalization sweep
/// all call. Pure + deterministic.
pub fn canonical_predicate(s: &str) -> String {
    let cleaned = canonical_label(s);
    if cleaned.is_empty() {
        return cleaned;
    }
    for (canonical, synonyms) in RELATION_LEXICON {
        if cleaned == *canonical || synonyms.contains(&cleaned.as_str()) {
            return (*canonical).to_string();
        }
    }
    cleaned
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
    fn canonical_predicate_folds_synonyms_and_keeps_unknowns() {
        // Synonyms fold to the canonical type, regardless of casing/spacing.
        assert_eq!(canonical_predicate("depends on"), "depends-on");
        assert_eq!(canonical_predicate("Requires"), "depends-on");
        assert_eq!(canonical_predicate("  NEEDS "), "depends-on");
        assert_eq!(canonical_predicate("led to"), "causes");
        assert_eq!(canonical_predicate("part of"), "part-of");
        // The canonical key maps to itself.
        assert_eq!(canonical_predicate("depends-on"), "depends-on");
        // Unknown relation → open-world fallback (cleaned, not dropped).
        assert_eq!(canonical_predicate("Rivals"), "rivals");
        assert_eq!(canonical_predicate("  smells  like "), "smells like");
        assert_eq!(canonical_predicate("   "), "");
    }

    #[test]
    fn lexicon_canonical_keys_are_stable_under_their_own_mapping() {
        // Every canonical key is idempotent (maps to itself) — a guard that
        // no key is itself a synonym of another type.
        for (canonical, _) in RELATION_LEXICON {
            assert_eq!(canonical_predicate(canonical), *canonical, "key {canonical}");
        }
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
