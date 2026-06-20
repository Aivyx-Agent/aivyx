# The Typed Knowledge Graph — entities + directed relations (Chapter Lattice)

> **Status:** 🕸️ **LT.4 — `graph.read` base + `graph.query` tool shipped.** The
> agent can now traverse its own typed graph: a new `graph.read` infrastructure
> capability base (`KNOWN_BASES` 86→87 + `CEILING_TRUSTED` + taxonomy addendum +
> DESIGN D4 + TOOLS.md — **no P10 amendment**) gates `graph.query`, a multi-hop
> directed/typed traversal (direction + predicate filter + hop cap, cycle-safe,
> deterministic). The base landed **with** the tool. Registered for all
> channels + granted in the default floor; Trusted-tier. Studio view (LT.5) +
> recall fusion (LT.6) remain. The locked reference for the
> chapter that gives Aivyx a **real, directed, typed knowledge graph**:
> nodes are **entities** (people, systems, concepts) and edges are
> **typed, directed relations** (`deploy` —*depends-on*→ `ci`), extracted
> from memory by the LLM. It is the last of the two tracks [[LOOM]]
> deferred — the "real DAG" beyond the undirected topic co-occurrence
> graph. The operator chose the **full + agent-query-tool** reach: the
> graph is browsable (Studio), queryable by the agent (a `graph.query`
> tool), and fusable into recall. It builds directly on the extraction +
> storage + sweep + IPC + Studio + recall patterns [[CODEX]] and [[LOOM]]
> established.

## 1. Why — the graph today has no *meaning* on its edges

Aivyx already has *a* graph: Chapter MG draws topic nodes and the Phase
83 **co-occurrence** edges (Loom's `neighbors_within` walks them). But a
co-occurrence edge says only "these two topics were recalled together" —
it is **undirected and untyped**. It cannot answer:

- *"What **depends on** the deploy pipeline?"* (a directed, typed query)
- *"What did **incident-417 cause**?"* (a relation kind, a direction)
- *"How is **Jane** connected to the **billing** rewrite?"* (a path of
  typed hops)

These are the questions a knowledge base exists to answer, and memory —
a pile of timestamped notes — can't, nor can a weight-only topic graph. A
typed, directed graph of **(subject) —[predicate]→ (object)** facts,
extracted from what the agent already remembers, is the missing layer:
it turns "the agent remembers notes about deploy" into "the agent knows
that deploy depends-on ci, ci triggers rollback, and rollback reverts
deploy" — and can *traverse* that.

## 2. Architecture & decisions (locked)

### The model: open, directed triples with provenance
A graph fact is a directed triple **`(subject, predicate, object)`**:
- **subject / object** — canonicalized entity strings (lowercased/trimmed,
  reusing the Phase 89 / Loom canonicalizer so `Deploy Pipeline` and
  `deploy pipeline` are one node), each carrying an optional LLM-inferred
  **kind** (`person` / `system` / `concept` / …) for richer rendering.
- **predicate** — a **free-text relation label** (`depends-on`,
  `caused`, `owns`, `part-of`). **Open-vocabulary**, not a fixed enum:
  the LLM picks the natural relation, which is flexible, model-friendly,
  and avoids a brittle ontology. A *constrained* relation vocabulary
  (canonicalizing `depends on` / `requires` / `needs` into one type) is a
  documented refinement for a later chapter.
- **provenance** — the memory entry `seq`s the triple was extracted from
  (for incremental regeneration and so the operator/agent can trace a
  fact back to its source), plus a `confidence` and `updated_at`.

"Typed" = the predicate label carries the type; "directed" = the
subject→object order is meaningful. This is the modern open-IE shape,
deliberately lighter than a schema-first ontology.

### Extraction is best-effort LLM, derived from memory
A `GraphExtractor` (the `WikiSynthesizer`'s sibling) prompts the daemon's
existing `Arc<dyn LlmProvider>` to pull directed triples from a topic's
memory entries, constrained to **only what the notes state** (no
invention, no new instructions). **Best-effort + incremental** (a per-
topic `source_fingerprint`, exactly like Codex): a topic whose entries
haven't changed isn't re-extracted; any LLM/storage failure skips it and
never touches memory. The graph is **always derived** — memory is the
source of truth; you correct a fact by correcting the memory it came
from, never by hand-editing the graph.

### Storage: one new infrastructure domain
Triples live in a new HKDF-isolated **`KeyDomain::KnowledgeGraph`**, one
row per directed triple (keyed by a canonical `subject\x00predicate\x00
object` encoding so re-extraction upserts rather than duplicates). The
entity set + adjacency are derived from a scan (the graph is small —
bounded by the operator's memory). Routine storage growth, the same
shape `KnowledgeWiki` followed.

### The agent can *query* it — `graph.query` + a new `graph.read` base
The headline capability: an agent-facing **`graph.query`** tool for
**multi-hop typed traversal** — from a start entity, follow directed
edges (optionally filtered by predicate and direction) up to `N` hops,
returning the reachable entities and the typed paths to them. This is
what makes the graph *reasoned with*, not just stored.

**Governance — infrastructure, a new base, no P10 amendment.** Querying
the agent's **own derived self-knowledge** is the agent organizing
itself — P10's *infrastructure* tier, exactly like `reflection.*` /
`skills.*` / `loop.*` (all added to `KNOWN_BASES` as infrastructure
**without** a P10 substrate-count amendment). So Lattice:
- adds **one new `graph.read` capability base** (`KNOWN_BASES` +
  `CEILING_TRUSTED`, the capability-taxonomy-growth addendum, the
  count-test bump, and a DESIGN Deliverable 4 row) — the A3/A12-style
  base addition;
- gates `graph.query` on it; **Trusted-tier** by default (reading derived
  memory-knowledge, on par with the other reflection-layer reads);
- takes **no P10 substrate-count amendment** — the graph is derived, not
  a new operator-owned resource primitive, and the tool is infrastructure,
  not substrate.

### Surfacing reuses Codex/Loom machinery
- **Studio** — a typed/directed graph view (entities as nodes, labeled
  directed edges), over a new read-only IPC, parallel to the MG
  co-occurrence view (which stays as-is). Reuses the Studio graph-render
  + screen patterns.
- **Recall** — the typed-graph neighbors of a query's entities become an
  opt-in **Loom fusion source** (a 5th candidate), behind a config knob,
  byte-identical default off — built on `reciprocal_rank_fusion_weighted`.

### Determinism, best-effort, opt-in
Generation, the query tool, the Studio view, and the recall source are
each **opt-in and default-off / byte-identical**; every read path is
best-effort (a corrupt/absent graph degrades only Lattice, never memory
or recall); traversal + extraction ordering are deterministic.

## 3. Scope

**In:** the triple/entity model + `KnowledgeGraph` storage, the LLM
extraction engine + incremental sweep + `[graph]` config, the `graph.read`
base + the `graph.query` traversal tool, the read-only IPC + Studio graph
view, and the opt-in recall fusion source.

**Out:** a *constrained* relation vocabulary / ontology (open predicates
in v1); entity resolution beyond string canonicalization (no coreference
/ alias merging — `JH` and `Jane Henderson` stay distinct unless a note
links them); graph editing by hand (the graph is derived); cross-document
extraction beyond memory entries; any P10 substrate-count amendment.

## 4. Phase plan (docs-first, small phases per convention)

| Phase | Deliverable | Notes |
|---|---|---|
| **LT.0** | **This design contract** | locked reference; banner flips per phase |
| **LT.1** ✅ | **Model + storage** | DONE. `aivyx-ipc::graph` — `GraphTriple` (directed, NUL-joined `key`, provenance + `mentions` weight), `GraphEntity` (name + degree + optional kind), `GraphPath`, and `canonical_label` (lowercase/trim/collapse, **no stemming** — `settings` ≠ `setting`). `KeyDomain::KnowledgeGraph` (ALL/subkeys 22→23, 2 count tests; README 23). `PersistentGraphStore` (`aivyx-channel`) — canonical triple upsert/get/delete (empty part rejected), `all_triples` (skips meta rows), `out_edges`/`in_edges` (direction-aware), `entities` (degree-ranked), + per-topic incremental fingerprint markers keyed `\x00fp\x00<topic>` (invisible to the triple scan). **Inert**. 12 tests. |
| **LT.2** ✅ | **Extraction engine** | DONE. `GraphExtractor::regenerate(topic, now) -> GraphRegenOutcome { Skipped, NoEntries, Wrote(n) }` in `aivyx-channel::knowledge_graph`. Pulls entries (capped), one-shot LLM extraction (constrained system prompt, low temp, **JSON triple array** parsed tolerantly — finds the outermost `[ … ]` through prose/fences), canonicalizes + drops empties/self-loops + dedups (repeats → `mentions`), upserts with the topic's seqs as batch provenance. Incremental (records the fingerprint even on zero triples so a relation-less topic isn't re-extracted); best-effort (any soft failure → no triples, never errors). 5 tests w/ a scripted fake `LlmProvider` (parse tolerance, directed store, incremental, failure/no-entries, mentions). |
| **LT.3** ✅ | **Generation trigger** | DONE. `GraphExtractor::sweep(now, max_topics) -> GraphSweepReport` (walk `list_topics`, re-extract stale, cap **extractions**/LLM-calls per pass) + `run_graph_sweep_loop` (periodic, first-tick-skipped, shutdown-aware) + a `[graph]` config (`enabled` default off, `max_topics_per_sweep` 20, `interval_secs` 3600; validated only when enabled). Wired via `DaemonConfig.graph_sweep: Option<GraphSweepConfig>` spawned in the daemon, constructed in `aivyx.rs` from the LLM provider + memory + `KnowledgeGraph` domain when enabled. Default-off ⇒ byte-identical. 3 sweep/loop tests + 1 config test (7 e2e `DaemonConfig` literals updated). |
| **LT.4** ✅ | **`graph.read` base + `graph.query` tool** | DONE. New `graph.read` base (KNOWN_BASES 86→87 + CEILING_TRUSTED + count-test + taxonomy-growth addendum + DESIGN D4 row + docs/TOOLS.md — **infrastructure, no P10 amendment**, like `skills.*`). Pure `traverse(triples, start, direction, predicate, max_hops, max_results)` BFS (shortest-path, cycle-safe, deterministic) + `PersistentGraphStore::query` + `GraphDirection`{Out,In,Both}. `GraphQueryTool` (`aivyx-channel::graph_query_tool`, OnceLock-store pattern) gated by `graph.read`, registered in `aivyx.rs` + `graph.read` granted in the backcompat floor (the agent may query its own graph by default); graph store now built unconditionally. 8 tests (capability parse/tier, 5 traversal, 3 tool). |
| **LT.5** | **Read-only IPC + Studio graph view** | `GetKnowledgeGraph` (typed nodes + directed labeled edges) wasm-clean types + handler reading the store; a Studio view rendering the directed/typed graph (distinct from the MG co-occurrence view). Bundle rebuilt. |
| **LT.6** | **Recall fusion source** | typed-graph neighbors of the query's entities as an opt-in Loom RRF source (`recall_graph_typed_weight`, default 0.0), byte-identical default; recall-never-errors preserved. Tests + eval extension. |
| **LT.7** | **Finalize** | full suite + clippy + `cargo deny` green; status flip; record. |

**Discipline:** LT.1–LT.2 ship the substrate inert; LT.3 is the first
behavior, gated to the cadence + opt-in; **LT.4's `graph.read` base lands
with (not after) the `graph.query` tool** (the Forge gate-before-tool
rule); LT.6's recall change is opt-in default-off like every Loom/Codex
seam. Each read/write boundary (storage, tool, IPC, recall) is its own
phase so the diff stays reviewable.

## 5. Open questions (resolve in-phase)

- **Triple confidence** — keep the LLM's self-reported confidence, or a
  fixed weight + a `mentions` count (how many memory entries assert it)?
  Default to a `mentions`-derived weight (legible, not a hallucinated
  number); revisit in LT.2.
- **Extraction unit** — per topic (like Codex pages) vs. a rolling window
  of recent entries across topics (catches cross-topic relations a
  single-topic prompt misses). Default per-topic for v1 simplicity; a
  cross-topic pass is a deferral (LT.2/LT.3).
- **`graph.query` output shape** — entities + flat paths, or a subgraph
  (nodes + edges) the agent can reason over? Default a ranked list of
  `(entity, typed path, hops)`; a subgraph projection is a later add
  (LT.4).
- **`graph.read` tier** — Trusted-only (the reflection-layer default) vs.
  SemiTrusted (a remote researcher querying the graph). Default Trusted;
  relax if real use warrants (LT.4).
- **Studio view vs. MG** — a separate "Graph" screen, or a typed/directed
  *mode* toggle inside the existing Memory graph. Default a focused new
  view; reconcile with MG in LT.5.

---

*Chapter Lattice is the graph Aivyx has been circling since the
co-occurrence ledger: not "these topics go together" but "**this** relates
to **that**, in **this** way, in **this** direction." It is extracted from
what the agent already remembers, queryable by the agent, browsable by the
operator, and — opt-in — a signal in recall. Entries are what happened;
the codex is what is known; the lattice is **how it all connects.***
