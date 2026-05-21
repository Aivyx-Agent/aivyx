# Phase 96 — ANN Index for Semantic Memory Search (Phase 75's ANN-index deferral, closed)

Phase 75 introduced semantic memory search: embeddings
are written alongside each memory entry, and
`Memory::semantic_search_scored` ranks the entire vector
index against the query via brute-force cosine. That works
for any operator with up to a few thousand entries but
scales linearly. Phase 75 listed "ANN index" as the first
deferral and has carried it for 20 phases.

Phase 96 closes the deferral with a hand-rolled
**IVF-style clustering** approach that preserves the
project's zero-new-deps streak: the embeddings are
partitioned into `K ≈ √N` clusters at build time; at query
time, the query vector is cosine-ranked against the K
centroids (cheap — K is small), the top-N clusters are
selected, and brute-force cosine runs only within those
clusters' members. The result narrows to a candidate set
of size `≈ N * (top_clusters / K)`, which the existing
brute-force re-rank then orders exactly. End-to-end: ANN
scales `O(N) → O(K + N · top_clusters / K) ≈ O(√N)` for
the right K; the brute-force re-rank on the candidate set
guarantees the final ordering is exact within the
candidates returned.

The composition is the Q3a hybrid: **ANN narrows →
brute-force re-ranks**. The operator's existing brute-
force path remains the default; the ANN path arms via the
new `[embedding].ann_index = true` knob.

## Why this, why now

- Phase 75's first listed deferral; 20 phases old.
- The recall+persona learning loop (Phases 86–95) is now
  end-to-end. Operators with long-running daemons
  accumulate memory entries quickly; brute-force cosine
  pays per-recall O(N) regardless of which entries are
  semantically nearby. ANN narrows the comparison set.
- Phase 75's vector-index substrate is already in place:
  `RedbMemory.vector_index: Mutex<Vec<(String, u64,
  Vec<f32>)>>` is the source of truth; Phase 96 builds a
  derived `AnnIndex` on top without modifying the
  underlying storage shape.
- The change is **purely additive**. With
  `ann_index = false` (the default), every recall path is
  byte-identical to pre-Phase-96 brute-force. With
  `true`, the brute-force `rank_by_cosine` still runs —
  on a narrowed candidate set — so the final ordering is
  guaranteed correct within the candidates ANN returned.
- Reuse is total. The existing `rank_by_cosine` helper
  becomes the re-rank step; no new dependency, no new
  `KeyDomain`, no chain-schema migration.

## Streak predictions

- **DESIGN.md** — **Will hold.** ANN is an indexing
  strategy over an existing vector store; the
  `Memory` trait's semantic-search contract is unchanged
  at its observable behaviour (top-K by cosine
  similarity). No locked technical-contract decision is
  touched. Hash at entry:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Prediction: streak **extends to forty-three** (currently
  42).

- **PRODUCT.md** — **Will hold.** P9 (recall) is
  delivered; this is a performance refinement on its
  semantic-search backend. No commitment changed; the
  operator-facing contract is *strengthened* (large
  memory stores stay responsive when the operator opts
  in). Hash at entry:
  `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.
  Prediction: streak **extends to thirty-six** (currently
  35).

- **Production-core `aivyx-core/src/lib.rs`** — **Will
  hold, by design.** The `AnnIndex` data structure +
  build + query live in `aivyx-memory`; the dispatch
  decision lives in `aivyx-channel::memory_recall`; the
  config knobs live in `aivyx-config`. No `aivyx-core`
  touch; no new `AuditTag`; no new `KeyDomain`. Hash at
  entry:
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.
  Prediction: streak **extends to forty-four**
  consecutive phases (new project record, beats Phase
  95's 43).

- **New workspace deps** — Zero. IVF clustering is
  hand-rolled in `aivyx-memory`. The k-means assignment
  uses the existing `cosine_similarity` helper that
  Phase 75 already ships.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_96.md` + `docs/README.md` status row.

### Task 2 — `[embedding].ann_index` + `ann_rebuild_threshold` knobs

`aivyx-config`:

- `EmbeddingConfig` gains two optional fields:
  - `ann_index: bool` (default `false`). Master switch.
    With `false`, brute-force is the only path
    (byte-identical to pre-Phase-96).
  - `ann_rebuild_threshold: u32` (default `100`). Number
    of new entries after the last index build before the
    index is marked stale and the next recall triggers a
    rebuild. Bounded `>= 1` when `ann_index = true`
    (zero would force a rebuild every recall).
- `RawEmbedding` + the build path. Validation per the
  established staged-config pattern (knob set but
  `ann_index = false` doesn't validate the threshold).
- Tests: defaults; explicit values win; staged-config
  posture; zero-threshold rejection when armed.

### Task 3 — `AnnIndex` data structure + build + query (the crux)

`aivyx-memory`:

- New `ann_index` module exposing:
  - `pub struct AnnIndex { centroids: Vec<Vec<f32>>,
    clusters: Vec<Vec<(String, u64, Vec<f32>)>> }` —
    K parallel arrays. Each cluster holds the
    `(topic, seq, embedding)` triples assigned to its
    centroid.
  - `pub fn build_ann_index(entries: &[(String, u64,
    Vec<f32>)]) -> AnnIndex` — partitions entries into
    K ≈ √N clusters using nearest-centroid assignment.
    Centroids are seeded by spaced sampling from the
    input (deterministic, no randomness — testable +
    reproducible). One assignment pass; no iterative
    refinement (v1 simplicity).
  - `pub fn query_ann(index: &AnnIndex, query: &[f32],
    top_clusters: usize, candidate_limit: usize) ->
    Vec<(String, u64, f32)>` — cosine-rank centroids,
    take top-N clusters, brute-force cosine within the
    union, return top-`candidate_limit` candidates.
- Quality knobs: `top_clusters` defaults to
  `(K / 4).max(2)` (a quarter of clusters, minimum two);
  `candidate_limit` defaults to `4 * caller_limit` (the
  candidate pool the caller's re-rank will trim).
- Edge cases: `entries.is_empty()` → empty index;
  `entries.len() <= 16` → degenerate single cluster
  (brute-force everything); `query.is_empty()` → empty
  result; embedding-dim mismatch → empty result (defended).
- Unit tests on the pure data structure:
  - Empty input → empty index.
  - Single entry → single cluster.
  - Small input (≤16 entries) → single-cluster
    degenerate path.
  - K-cluster build: assert K ≈ √N (within a small
    tolerance); every entry appears in exactly one
    cluster.
  - Deterministic centroid seeding: shuffled input
    produces identical centroids (modulo order).
  - Query returns top-K by cosine (validated against
    brute-force on a small fixture).
  - Query with `top_clusters` covering all clusters
    is equivalent to brute-force (recall == 1).
  - Query with `top_clusters = 1` still returns
    sensible results (degraded but non-empty).
  - Edge case: query dim mismatch → empty (defensive).

### Task 4 — `Memory` trait extension + RedbMemory integration

`aivyx-memory`:

- New `Memory::semantic_search_scored_ann(query_vec,
  limit, top_clusters, candidate_limit) -> Result<Vec<(
  MemoryEntry, f32)>, MemoryError>` method. Default impl
  on the trait falls back to brute-force
  `semantic_search_scored` (so any future Memory impl
  works unchanged).
- `RedbMemory` adds a second
  `ann_index: tokio::sync::Mutex<Option<AnnIndex>>` +
  `writes_since_build: AtomicU32` (lockless counter
  used by the stale-check). The flat `vector_index`
  remains the source of truth; the `AnnIndex` is a
  derived cache rebuilt on demand.
- Stale detection: `writes_since_build` increments on
  every `put_vector`; when a `semantic_search_scored_ann`
  call sees `writes_since_build >= threshold` (caller-
  supplied), it rebuilds before querying. Rebuilds
  reset the counter atomically. The threshold-versus-
  zero edge case is the operator's `ann_rebuild_threshold`
  (Task 2 config knob).
- `InMemoryMemory` uses the same default-trait
  brute-force path; the ANN path is RedbMemory-only for
  v1 (in-memory tests don't need scaling).
- Integration tests against `RedbMemory`:
  - Write 100 entries with distinguishable embeddings;
    `semantic_search_scored_ann` returns the same top-K
    as `semantic_search_scored` (brute-force re-rank
    guarantees ordering within candidate set).
  - Write 100 more entries (crosses threshold); index
    rebuilds; results stay correct.
  - Write below threshold; index stays warm;
    `writes_since_build` accumulates but doesn't rebuild.
  - Concurrent put + query: no data races, no
    panics (lock discipline preserved).

### Task 5 — `aivyx-channel` dispatch + surface + docs + exit

`aivyx-channel/src/memory_recall.rs`:

- `SemanticMemoryContext::recall` reads
  `[embedding].ann_index` from config and dispatches:
  `true` → `semantic_search_scored_ann(...)`; `false` →
  existing `semantic_search_scored(...)`. The two paths
  return the same shape; the caller is signal-blind.
- `[embedding].ann_rebuild_threshold` plumbed through
  to the per-recall ANN call as the stale-check
  threshold.
- `aivyx learning` render block: extend the recall
  section with a `recall backend: ann` line when the
  knob is on; falls back to `brute-force` when off.
  Visibility-only — no actuator behaviour change.
- `docs/INSTALL.md` — new "ANN index for semantic memory
  search (Phase 96)" subsection under the existing
  Phase 75 semantic-search section: the IVF approach,
  the hybrid composition (ANN narrows → brute re-ranks),
  the two opt-in knobs, the default-off posture, the
  scaling story.
- `examples/aivyx.toml` — document
  `[embedding].ann_index = true` +
  `ann_rebuild_threshold = 100` with the canonical
  default-off comment.
- Exit: ROADMAP + PRODUCT_ROADMAP frozen entries,
  docs/README status flip, prediction-vs-reality, hash
  backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Algorithm:** (a) IVF-style clustering,
  hand-rolled. Preserves the project's zero-new-deps
  streak; ~150 lines of code in `aivyx-memory`;
  scales `O(N) → O(√N)`. Quality is adequate for
  any operator with ≤100K entries; HNSW-level recall
  defers as a Phase 96-follow-up if operators with
  larger stores need it.
- **Q2 — Lifecycle:** (a) Rebuild on daemon boot + on
  demand. The index lives in-memory; rebuilt at daemon
  startup from the persisted embeddings, and re-rebuilt
  on demand when `writes_since_build` crosses the
  operator-configured threshold. No new schema, no
  serialization, no incremental-update complexity.
- **Q3 — Composition:** (a) ANN narrows → brute-force
  re-ranks. ANN returns top-`candidate_limit`
  candidates (e.g., `4 * limit`); brute-force cosine
  re-ranks within and returns the top `limit`. The
  exact-cosine guarantee on the final K is preserved
  within the candidate set.
- **Q4 — Knob:** (a) `[embedding].ann_index: bool`
  (default `false`) + `ann_rebuild_threshold: u32`
  (default `100`). Per-`[embedding]` opt-in; matches the
  Phase 87/88/91/92/93/95 actuator opt-in pattern. With
  the knob off the recall path is byte-identical to
  pre-Phase-96.

## Deferrals

**Rolling deferrals carried into Phase 96** (Phase 75's
ANN-index deferral is **THIS PHASE**):

- v0.1.0 publication (Phase 61 Task 7).
- System-prompt notification-target enumeration (Phase 62).
- `turn_id` correlation refactor (Phase 67).
- Phase 68 deferrals (XOAUTH2, HTML email, attachments,
  multi-account `[email]`).
- Phase 69 deferrals (WebPush / service-worker, notification
  urgency, icons, sound).
- Phase 70 deferrals (multi-window reflection).
- Phase 71 deferrals (per-fire role override, operator-
  customizable reflection prompt — reflection cadence
  learning closed by Phase 95).
- Phase 73 deferrals (persisted rate-limit buckets,
  operator-configurable retry-on list).
- Phase 74 deferrals (fuzzy match, edit-content Web UI,
  per-topic eviction-strategy override).
- Phase 75 deferrals (**ANN index — THIS PHASE**,
  `aivyx memory reembed`, hybrid keyword+semantic
  fusion, query-embedding cache).
- Phase 76 deferrals (token-budget context sizing).
- Phase 77 deferrals — closed by Phase 93.
- Phase 78 deferrals (per-memory-entry drill-down, Web UI
  live refresh, actionable insights).
- Phase 79 deferrals (`[persona]` tuning block, behavioural
  Persona).
- Phase 80 deferrals (standalone `[[proactive_schedule]]`,
  LLM-composed proactive prose, conversational/interactive
  proactive, additional proactive signal classes).
- Phase 81 deferrals (contradiction-based supersession,
  standalone `[[persona_lifecycle_schedule]]`, facet-scoped
  one-click revert).
- Phase 82 deferrals (operator-tunable half-life/retention).
- Phase 83 deferrals (sequential/temporal patterns, n-ary
  clusters, operator-tunable top-K/half-life).
- Phase 84 deferrals (affinity re-ranking, operator-
  tunable affinity policy).
- Phase 85 deferrals (reflection-facet decay via fuzzy
  embedding).
- Phase 86 deferrals (token-budget context sizing,
  embed-each-and-pool windows, persisted windows).
- Phase 87 deferrals (n-ary cluster proposals, operator-
  tunable LLM prompt).
- Phase 88 deferrals (n-ary cluster decay, pair-affinity
  hysteresis).
- Phase 89 deferrals (operator-tunable `[[topic_alias]]`
  mappings, topic-by-topic exception list, one-time
  migration of existing fragmented data, non-ASCII /
  Unicode stemming).
- Phase 90 deferrals (pattern-based stoplist, LLM-judged
  gate, adaptive thresholds).
- Phase 91 deferrals (per-recall LLM critique, adaptive
  batch size, multi-model ensembling, response-text
  recovery).
- Phase 92 deferrals (atomic chain-level supersession
  primitive, n-ary cluster supersession, semantic-
  similarity supersession; Web UI grouping closed by
  Phase 94).
- Phase 93 deferrals (per-domain/per-topic verdict-mapping
  weights, replace mode, asymmetric Hurt penalty, sum
  mode, on-disk buffer).
- Phase 94 deferrals (atomic transaction IPC for "Approve
  both", backend-side grouping enrichment, drag UI
  affordances, n-ary group rendering).
- Phase 95 deferrals (backoff-multiplier mode, adaptive
  interval, time-of-day pattern learning, persisted
  cadence stat, LLM-based signal-density classifier,
  per-pass skip granularity).

**Likely Phase 96 deferrals:**

- **HNSW-quality recall.** v1's IVF approach trades
  some recall for simplicity + zero-new-deps. Operators
  with very large memory stores (>100K entries) may want
  HNSW-level quality — defers as a follow-up that
  introduces the first new workspace dep (likely
  `instant-distance`) or hand-rolls HNSW.
- **Iterative k-means refinement.** v1's one-pass
  nearest-centroid assignment uses spaced-sampling for
  seeds with no Lloyd's-algorithm refinement passes.
  Centroids stabilize to a local optimum but aren't
  globally optimal. A future phase could add 2-3
  refinement passes for higher cluster quality.
- **Persisted ANN index.** v1 rebuilds at daemon boot
  from the persisted embeddings. For very large stores
  the rebuild cost on boot becomes noticeable; future
  work could serialize the index to a new `KeyDomain`.
- **Incremental updates.** v1 marks the index stale and
  rebuilds on demand. Incremental insert (with eventual
  rebalance) is the obvious next move if writes outpace
  the rebuild threshold.
- **Topic-aware cluster seeding.** v1's centroid seeding
  is geometry-only. A future phase could bias initial
  centroids by topic to improve recall on topic-coherent
  queries.
- **ANN for `aivyx memory search` (the CLI/Web UI
  surface).** v1 wires ANN only into the auto-recall
  path; the operator-driven search may still want
  brute-force for exactness. Future phase decides.

## Prediction vs. reality

To be filled in at phase exit.

## Exit criteria

- [ ] `[embedding].ann_index: bool` (default `false`) +
  `ann_rebuild_threshold: u32` (default `100`, bounded
  `>= 1` when `ann_index = true`) — Task 2.
- [ ] `AnnIndex` data structure + `build_ann_index` +
  `query_ann` pure functions in `aivyx-memory` — Task 3.
- [ ] Unit tests on the pure ANN functions: empty input;
  single entry; degenerate small-N; K-cluster build
  invariants; deterministic centroid seeding; query-
  equivalence to brute-force when `top_clusters` covers
  all; query with restricted `top_clusters`; dim-
  mismatch defended — Task 3.
- [ ] `Memory::semantic_search_scored_ann` trait method
  with default brute-force fallback impl — Task 4.
- [ ] `RedbMemory` ANN integration: stale-counter,
  on-demand rebuild, concurrent-safe lock discipline —
  Task 4.
- [ ] Integration test: ANN path returns the same top-K
  as brute-force on a 100-entry RedbMemory fixture —
  Task 4.
- [ ] `SemanticMemoryContext::recall` dispatches on the
  `ann_index` config knob — Task 5.
- [ ] `aivyx learning` surface shows the backend choice —
  Task 5.
- [ ] `docs/INSTALL.md` + `examples/aivyx.toml` updated —
  Task 5.
- [ ] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 5.
- [ ] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [ ] DESIGN.md streak extends to forty-three.
- [ ] PRODUCT.md streak extends to thirty-six.
- [ ] Production-core streak extends to forty-four (new
  record) — `lib.rs` byte-identical.
- [ ] Test count delta: positive (~+15-25; the pure ANN
  module is the largest new surface this phase, with
  many boundary cases (Task 3); brute-vs-ANN equivalence
  integration test on a real RedbMemory fixture is
  worth +1-2; config knobs +3-4; surface render +1-2).
- [ ] Zero clippy warnings.
- [ ] Zero new workspace deps.
- [ ] Prediction-vs-reality block filled.
