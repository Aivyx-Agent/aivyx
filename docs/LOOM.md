# Graph-Augmented Recall — fusing the graph into RAG (Chapter Loom)

> **Status:** 🧵 **LM.3 — multi-hop graph walk shipped (inert).** Added
> `PersistentCooccurrenceLedger::neighbors_within` (BFS over the Phase 83 ledger:
> decayed bottleneck-path affinity, `hops=1` == `siblings_of`, branch-pruned by
> `min_affinity`) + the `GraphNeighbor` type. With LM.1 (**weighted RRF**) and
> LM.2 (**BM25** lexical scorer), all three of Loom's candidate sources now
> exist as pure, tested, **inert** building blocks. LM.4 is the single seam that
> fuses them into live recall. The locked reference for the
> chapter that turns Aivyx's co-occurrence graph from a passive *view*
> into an active *retrieval signal*, and fuses a lexical path into recall
> alongside the existing semantic one. It **refines** the recall layer
> already in place (Phase 76 semantic recall, Phase 84 1-hop co-recall,
> Phase 96/97 ANN-hybrid + token budget) — it adds **no new operator
> tool, no new capability base, and no substrate-count change.** All work
> is recall-layer machinery in `aivyx-channel` + a scorer in
> `aivyx-memory`. The operator picked the **graph-augmented RAG** track
> over the deferred typed-knowledge-graph and knowledge-wiki tracks.

## 1. Why — what recall does today, and the three gaps

Recall is already good and already *partly* graph-aware. Per turn,
`SemanticMemoryContext` (Phase 76) embeds the user message, ranks
memories by cosine (brute-force, or the Phase 96 ANN-narrow→exact
hybrid), drops everything under `rag_min_similarity`, and injects a
labeled top-`rag_top_k` block. Phase 84's opt-in `[recall_cluster]`
then pulls **one hop** of co-occurrence siblings (`siblings_of` over the
Phase 83 EWMA ledger) and **budget-shares** them in by displacing the
weakest primary hits. Phase 97 enforces a token budget last.

That is a real foundation — but it leaves three specific gaps:

1. **The graph walk is one hop only.** `siblings_of(topic, …)` returns
   direct neighbors. A memory two hops away (`deploy → ci → flaky-test`)
   — exactly the associative recall a human makes — is unreachable.

2. **The lexical path is unscored substring matching.** Phase 98's
   `recall_hybrid` *does* already fuse a keyword ranker with the semantic
   one via RRF — but the keyword ranker is `Memory::search`, a naive
   case-folded `contains` scan (no TF-IDF/BM25 term weighting). A two-word
   query ranks a memory that merely *contains* one common word the same as
   one that contains the rare discriminating term. Real BM25 scoring is
   the missing quality (rare terms, codes, proper nouns).

3. **The graph is not a fusion input.** Phase 84 bolts co-occurrence
   siblings on by *displacement* — knocking out the weakest cosine hits —
   instead of feeding the graph in as a third **ranker** the shared RRF
   fuses fairly against the others. (And it walks only one hop, per gap 1.)
   So the three signals never compete on one honest scale.

Closing these three turns recall from "nearest vectors (+ a sibling
nudge)" into "the genuinely most relevant memories, found by meaning,
by word, and by association, ranked on one scale."

## 2. Architecture & decisions (locked)

### One fusion, three candidate sources
The recall set becomes the **rank-fusion** of up to three ranked lists:

- **Semantic** — the existing cosine ranking (brute-force or ANN-hybrid).
  Unchanged.
- **Lexical** — a new **BM25** scorer over memory entries (LM.2).
- **Graph-walk** — multi-hop co-occurrence neighbors of the *seed*
  topics, weight-decayed per hop (LM.3), each contributing its most
  relevant entry.

They merge through **Reciprocal Rank Fusion (RRF)**: `score(d) =
Σ_sources weight·1/(k + rank_source(d))`. RRF is the right primitive
because it fuses rankings whose **scores are not comparable** (cosine
similarity vs. BM25 magnitude vs. decayed edge weight) using only each
item's *rank* within its source. The unweighted core already exists and
is live (Phase 98, `recall_fusion.rs`, fusing two rankers); LM.1 added the
per-source `weight` so an operator can bias toward exact-term recall. It
is **deterministic and needs no new dependency** — matching the
IVF/canonicalizer zero-dep, zero-RNG precedent.

### Reuse the Phase 83 ledger as the graph — no new storage, no typed edges
The "graph" is the existing `PersistentCooccurrenceLedger` (undirected,
EWMA-weighted topic pairs). LM.3 adds a `neighbors_within(seed, hops,
per_hop_decay, min_affinity, cap)` walk **over that ledger**, reusing its
HKDF-isolated storage and read-time decay. **Typed/directed
entity–relation edges and entity extraction stay out** — that is the
separately-tracked "real DAG" chapter. Loom makes the graph we already
have *earn its keep in retrieval*; it does not change the graph's model.

### BM25 over the existing entries — a scorer, not a new index
LM.2's lexical retriever scores the **entries already in the substrate**
(tokenize body+topic, IDF over the corpus, BM25 term weighting). It is a
pure function exposed as a `Memory` method (e.g. `lexical_search_scored`)
or a recall-side scorer over a candidate scan — decided in-phase, leaning
toward the substrate so `RedbMemory` and `InMemoryMemory` share it. No
persistent inverted index in v1 (the corpus is ≤ ~100K entries, the same
scale IVF targets); a persistent index is a documented deferral if
profiling demands it.

### Opt-in, back-compatible, and recall **never errors a turn**
Every addition is config-gated and defaults to **today's behavior
byte-identical**. The fusion path activates only when the operator
enables it; with it off, the Phase 76/84/96 path runs unchanged. The
best-effort invariant is absolute: any failure in the lexical scorer, the
graph walk, or the fusion (empty corpus, ledger miss, embed failure)
**falls back to the current recall, never errors the turn** — the same
contract Phase 76/84 already hold.

### Determinism + observability
RRF, BM25, and the decayed walk are all deterministic (no RNG, stable
tie-breaks on `seq`). The per-turn recall breadcrumb + the Phase 78
cluster stat extend to label each injected hit by its winning source
(semantic / lexical / graph-hop-N) so the operator can *see* why a memory
was recalled — and so the LM.5 eval harness has ground truth to score.

### No tool / capability / amendment surface
This is recall-layer infrastructure (the agent's own machinery to *be*
itself, P10's "infrastructure" tier), not an operator-facing substrate
tool. **No new `KNOWN_BASES` base, no P10 substrate-count amendment, no
DESIGN Deliverable 4 change.** Lighter governance than Chapter Forge by
design — the contract here is config + recall behavior, verified by tests
and the eval harness, not a charter change.

## 3. Scope

**In:** the RRF fusion core, the BM25 lexical scorer, the multi-hop
weight-decayed graph walk, their composition in `SemanticMemoryContext`,
the `[recall]` config knobs (all opt-in), the source-labeled breadcrumb,
and a retrieval-quality eval harness (recall@k over fixtures).

**Out:** typed/directed entity–relation edges + entity extraction (the
"real DAG" chapter); the synthesized knowledge-wiki / per-topic summary
layer (its own chapter); the HNSW ANN upgrade (a separate RAG-quality
item, recorded since Phase 96); any new operator tool or capability base;
any change to the at-rest memory encoding or the ledger's data model.

## 4. Phase plan (docs-first, small phases per convention)

| Phase | Deliverable | Notes |
|---|---|---|
| **LM.0** ✅ | **This design contract** | locked reference; banner flips per phase. DONE. |
| **LM.1** ✅ | **Weighted RRF** | DONE. The unweighted RRF core already existed (Phase 98, `recall_fusion.rs`, live in `recall_hybrid`). LM.1 added `reciprocal_rank_fusion_weighted` (per-source weight multiplier; NaN/∞→1.0, negative→0.0/silenced) for the 3-source `lexical_weight` biasing LM.4 needs; the unweighted fn now delegates (proven behavior unchanged). Pure, dep-free, **inert** (not wired). 6 new tests (weight scaling, biasing, zero-drop, defended weights, all-ones≡unweighted). |
| **LM.2** ✅ | **BM25 lexical scorer** | DONE. Pure `aivyx-memory/src/bm25.rs` (tokenize + non-negative BM25+ IDF + k1/b saturation, zero-dep, deterministic) + `Memory::lexical_search_scored` **default** trait method that pulls the corpus via `search("", MAX)` and BM25-ranks it — shared by `InMemoryMemory` + `RedbMemory` with no per-impl code; `search` stays the unscored discovery scan. 11 tests (7 scorer: rare-term-dominates, tf-saturation, more-terms-win, determinism; 4 integration: rare-term-first, topic-term match, empty/zero-limit, deterministic limit). **Inert** — becomes the 3rd fusion source in LM.4. |
| **LM.3** ✅ | **Multi-hop graph walk** | DONE. `PersistentCooccurrenceLedger::neighbors_within(seed, now, hops, per_hop_decay, min_affinity, cap)` — bounded BFS over the Phase 83 ledger returning `GraphNeighbor { topic, affinity, hops }`. Path affinity = decayed **bottleneck** (`min` edge · `decay^(hops-1)`), so `hops=1` reproduces `siblings_of` exactly and 2-hop is strictly weaker; best-path-wins on multi-path arrival; monotonic decrease lets `min_affinity` prune branches early; positive edges only; seed excluded. 6 tests (hop1≡siblings, 2-hop decay, bottleneck, min-affinity prune, direct-wins, degenerate). **Inert** — wired as the 3rd RRF source in LM.4 (`hops` labels `graph-hop-N`). |
| **LM.4** | **Fuse + config + wiring** | compose semantic ∪ lexical ∪ graph-walk through RRF in `SemanticMemoryContext`; add `[recall]` knobs (`rrf_k`, `lexical_weight`, `graph_hops`, `graph_decay`) — all default-off / back-compat; preserve recall-never-errors + the token budget; extend the breadcrumb/stat with the winning source. Config + recall integration tests. |
| **LM.5** | **Eval harness + finalize** | a small **recall@k** eval harness over seeded fixtures (queries → expected memories), proving fusion ≥ semantic-only on the fixtures and that default-off is byte-identical. Full suite + clippy + `cargo deny` green; status flip; record. |

**Discipline:** LM.1–LM.3 each ship **inert** (pure helpers + tests, no
recall behavior change); LM.4 is the single seam that activates fusion,
behind config, with the never-errors fallback. So at every commit before
LM.4 the live recall path is byte-identical, and LM.4's diff is small and
reviewable. The eval harness (LM.5) is what makes the whole chapter
*measurable* rather than vibes — the recurring lesson that retrieval
changes need a yardstick.

## 5. Open questions (resolve in-phase)

- **RRF `k` default** — the canonical RRF constant is 60; validate against
  the fixture set in LM.5 and expose `rrf_k` for operators who tune
  (LM.1/LM.4).
- **Lexical scorer home** — `Memory` trait method (shared by both impls,
  cleanest) vs. a recall-side scorer over a candidate scan (keeps the
  substrate minimal). Lean trait-method unless the corpus-IDF pass proves
  too heavy for `InMemoryMemory`'s test-speed contract (LM.2).
- **Graph seed set** — walk from the semantic top-K topics only, or also
  from lexical-hit topics? Default to the union of both source's top
  topics; cap total walk cost (LM.3/LM.4).
- **Per-source weighting** — pure RRF weights all sources equally; expose
  a `lexical_weight` (and implicit graph weight via `graph_decay`) so an
  operator can bias toward exact-term recall without recompiling (LM.4).
- **Eval fixtures** — hand-seeded query→expected pairs vs. a generated
  set; start hand-seeded (small, legible, deterministic) and grow (LM.5).

---

*Chapter Loom weaves the three threads recall already has — meaning
(vectors), words (lexical), and association (the co-occurrence graph) —
onto a single loom (rank fusion), so a memory is recalled when **any** of
the three says it matters, ranked on one honest scale. It is a refinement
chapter: no new tool, no new base, no charter change — just the recall the
agent already does, made deeper, fairer, and for the first time
measurable.*
