# Constrained Relation Vocabulary — a controlled lexicon (Chapter Lexicon)

> **Status:** ✅ **COMPLETE (LX.0–LX.3).** The typed graph has a controlled
> relation vocabulary: a curated 14-type lexicon + synonym table folds
> free-text predicates into canonical types at extraction and at query, and a
> best-effort sweep re-normalizes existing triples (merging synonym collisions).
> The graph **converges** — `depends on` / `requires` / `needs` become one
> `depends-on` edge. A pure refinement: **no new tool, base, amendment, storage
> domain, or dependency**; full suite + clippy + `cargo deny` green. The locked
> reference for the
> chapter that gives [[LATTICE]]'s typed knowledge graph a **controlled
> relation vocabulary**: a curated set of canonical relation types
> (`depends-on`, `causes`, `part-of`, …) that synonymous free-text
> predicates fold into, so `depends on` / `requires` / `needs` become one
> edge type instead of three. The operator chose the **curated built-in +
> open-world fallback** approach — a fixed built-in lexicon, no config,
> with unknown predicates kept as-is. It is a **refinement** of the
> Lattice graph: **no new tool, no capability base, no P10 amendment** —
> it improves the *quality* of an existing layer.

## 1. Why — open predicates fragment the graph

Lattice extracts triples with **open-vocabulary** predicates: the LLM
writes whatever relation phrase fits. That is flexible but fragmenting —
the same relation arrives under many surface forms:

- `deploy —depends on→ ci`, `deploy —requires→ ci`, `deploy —needs→ ci`
  are **three separate edges** for one fact.
- `graph.query` filtered by `predicate = "requires"` misses the
  `depends-on` and `needs` edges of the same relation.
- The Studio graph shows a noisy spray of near-synonym labels.
- Recall's typed-graph walk (LT.6) splits a relation's pull across
  variants instead of concentrating it.

A small **controlled vocabulary** fixes all four: fold the synonyms into
one canonical type, and the graph consolidates, queries cleanly, renders
legibly, and steers recall coherently — without losing the
genuinely-novel relations (those fall through to an open-world fallback).

## 2. Architecture & decisions (locked)

### A curated built-in lexicon + a synonym table (no config)
A fixed, hand-curated set of ~14 canonical relation types ships in
`aivyx-ipc::graph` as a pure const table — each canonical key paired with
its synonym phrases:

```text
depends-on ← depends on, requires, needs, relies on
causes     ← causes, caused, leads to, led to, results in, triggered, …
part-of    ← part of, belongs to, contained in, component of, member of
contains   ← contains, includes, comprises
related-to ← related to, associated with, linked to, connected to
located-in ← located in, resides in, hosted in, runs in
created-by ← created by, authored by, made by, built by
produces   ← produces, generates, outputs, emits
instance-of← instance of, is a, type of, kind of
replaces   ← replaces, supersedes, deprecates, succeeds
owns       ← owns, owner of, maintains, responsible for
uses       ← uses, utilizes, leverages
precedes / follows  ← (temporal ordering)
```

(The exact set is finalized in LX.1.) **Zero config** — the operator
picked the built-in-only option; an operator-configurable
`[graph.vocabulary]` is explicitly *not* in scope.

### `canonical_predicate` — clean → map → fallback (pure)
One pure, wasm-clean function: `canonical_predicate(s) -> String`:
1. **Clean** with the existing `canonical_label` (lowercase / trim /
   collapse whitespace — no stemming).
2. **Map** the cleaned phrase through the synonym table to its canonical
   key.
3. **Fallback** (open-world): an unmatched predicate keeps its cleaned
   label, so a genuinely-new relation is never discarded — the lexicon is
   a curated *core*, not a closed set.

Deterministic, allocation-simple, no new dependency.

### Applied at extraction + as a hint, and at query
- **Extraction (LT.2's `GraphExtractor`)** maps each parsed predicate
  through `canonical_predicate` before storing, so **new** triples are
  canonical. The extraction prompt also *lists the canonical vocabulary*
  as a preference, nudging the model to pick a canonical relation in the
  first place (fewer fallbacks).
- **`graph.query`** canonicalizes its `predicate` filter argument the same
  way, so filtering by `requires` matches the stored `depends-on` edges.

The `PersistentGraphStore` itself stays vocabulary-agnostic (it stores
whatever canonical label it's handed); the mapping lives one layer up, at
the extractor and the query — the single source of the lexicon is
`aivyx-ipc::graph`.

### Existing triples re-normalize on the cadence — and *merge*
A re-normalization pass (LX.2) re-maps the predicates of **already-stored**
triples and, crucially, **merges synonyms**: when `deploy —requires→ ci`
re-maps onto an existing `deploy —depends-on→ ci`, the two rows collapse
into one with **summed `mentions`** and unioned `source_seqs` (the
fragmentation the chapter exists to fix). Runs best-effort on the existing
graph-extraction sweep cadence; idempotent (a canonical triple re-maps to
itself, a no-op).

### Direction is normalized — inverse phrasings flip (pre-v0.4.0 addendum)
Forward synonyms are same-direction. **Inverse phrasings** (`owned by`,
`caused by`, `required by`, `produced by`) now fold to their forward
canonical type **with a subject↔object swap** so the stored direction is
canonical (`X owned-by Y` ⇒ `Y owns X`) — `canonical_relation(s) ->
(canonical, flip)` in `aivyx-ipc::graph` (an `INVERSE_LEXICON` disjoint
from the forward synonyms; a guard test enforces no overlap). Applied at
extraction and in the re-normalization sweep (both have subject/object to
swap); `graph.query`'s filter uses the direction-agnostic
`canonical_predicate` (it has no endpoints to flip). *Originally a
documented deferral; completed in the pre-v0.4.0 cleanup.*

### Governance: a refinement, nothing new
No new tool, no new capability base, no P10 amendment, no new storage
domain. The lexicon improves the *data quality* of the existing
`KnowledgeGraph` triples — `graph.query`, the Studio Graph view, and the
recall source all benefit for free. The lightest chapter in the memory
arc.

## 3. Scope

**In:** the curated canonical-relation lexicon + synonym table +
`canonical_predicate` (pure, `aivyx-ipc::graph`); applying it at
extraction (+ the prompt hint) and at `graph.query`; the
re-normalization-with-merge pass over existing triples on the sweep
cadence.

**Out:** an operator-configurable vocabulary (`[graph.vocabulary]`);
inverse-relation detection / direction-flipping; LLM-judged relation
clustering; any Studio edge-coloring-by-type beyond the (now cleaner)
labels — a deferred polish; any change to entities (this chapter is about
*predicates* only), the storage model, or the graph's tool/IPC surface.

## 4. Phase plan (docs-first, small phases per convention)

| Phase | Deliverable | Notes |
|---|---|---|
| **LX.0** | **This design contract** | locked reference; banner flips per phase |
| **LX.1** ✅ | **Lexicon core + apply** | DONE. `RELATION_LEXICON` const (14 canonical types — depends-on, uses, causes, part-of, contains, related-to, located-in, created-by, produces, instance-of, replaces, owns, precedes, follows — each with same-direction synonyms) + pure `canonical_predicate` (clean via `canonical_label` → synonym lookup → open-world fallback) in `aivyx-ipc::graph`. `GraphExtractor` folds parsed predicates through it before store + the system prompt lists the canonical vocabulary as a preference; `graph.query`'s `traverse` canonicalizes its predicate filter (so `needs` matches `depends-on`). 6 tests (synonym fold + fallback + key-idempotence; extraction stores canonical not the synonym; query filter matches via lexicon). |
| **LX.2** ✅ | **Re-normalize existing (merge)** | DONE. `PersistentGraphStore::normalize_predicates() -> usize`: re-maps each stored triple's predicate via `canonical_predicate` and **merges** synonym collisions onto the canonical key (sum `mentions`, union+sort `source_seqs`, max `updated_at`), deleting the synonym rows; only changed rows are rewritten (canonical no-collision rows untouched ⇒ idempotent); unknown predicates left as-is. Wired best-effort into `GraphExtractor::sweep` (a `GraphSweepReport.remapped` count). 2 tests (three synonym edges merge into one with summed mentions / unioned seqs / max updated_at; idempotent second pass + unknown left alone). |
| **LX.3** ✅ | **Finalize** | DONE. Full workspace suite + `cargo clippy --workspace` (0) + `cargo deny` (licenses + advisories) green; zero new deps; README phases line + CHANGELOG Lattice entry note the lexicon; banner flipped to COMPLETE; recorded. |

**Discipline:** LX.1's `canonical_predicate` is the single source of truth
the extractor, the query, and the LX.2 sweep all call — so the lexicon
can never disagree with itself. The re-map sweep (LX.2) is the only phase
that mutates existing data, and it is idempotent + merge-safe so a
half-run is always recoverable. Everything is best-effort: a mapping is a
pure string transform that can't fail, and the sweep degrades a corrupt
row to "skipped," never errors.

## 5. Open questions (resolve in-phase)

- **Lexicon membership** — the exact ~14 types + which synonyms. Curate
  for the common technical / personal-assistant relations; keep it small
  (a big lexicon is just open-world with extra steps). Finalize in LX.1.
- **`uses` vs `depends-on`** — fold `uses` into `depends-on`, or keep it a
  distinct (weaker) relation? Default: keep `uses` separate; revisit if it
  proves noisy (LX.1).
- **Re-map cadence** — every graph sweep, or a one-shot migration the
  first time the lexicon ships? Default: every sweep (cheap + idempotent;
  also catches any non-canonical write), with the bulk of the work on the
  first run (LX.2).

---

*Chapter Lexicon is the editor's pass over the knowledge graph: not new
facts, but a controlled vocabulary so the facts the agent already
extracted stop being said five different ways. `depends on`, `requires`,
and `needs` become one relation — and the graph, the query, and recall all
get sharper for it.*
