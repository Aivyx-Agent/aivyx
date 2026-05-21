# Phase 98 — Hybrid Keyword+Semantic Recall Fusion (Phase 75's hybrid-fusion deferral, closed)

Phase 75 introduced semantic memory search; Phase 76 made
recall automatic; Phase 86 widened the embed query to the
conversational window; Phase 96 added the ANN index for
scaling; Phase 97 capped injection by token budget. All of
that pipeline runs on **cosine similarity over embeddings**.
Embeddings encode semantic relationships well but struggle
with **rare-term recall**: project codenames, acronyms,
code identifiers, proper nouns. A query mentioning "ATC-417"
likely won't surface a memory about `ATC-417` if the
embedding doesn't strongly link the term to a learnable
concept. The keyword search tool (Phase 74) handles these
exact-match cases — but operates as a **separate manual
path**, not auto-recall.

Phase 75 listed "hybrid keyword+semantic fusion" as a
deferral; 23 phases old. Phase 98 closes it. With
`[embedding].recall_hybrid = true`, auto-recall runs both
the semantic ranker AND the existing substring search at
recall time, then fuses the two rankings via **Reciprocal
Rank Fusion (RRF)** — the standard production approach.
The fused top-K is what the downstream pipeline (cluster
expansion, `rag_min_similarity` floor, token budget, etc.)
operates on. With the knob off, recall is byte-identical
to pre-Phase-98 (semantic only).

## Why this, why now

- Phase 75 listed hybrid fusion as a deferral 23 phases
  ago. The recall pipeline is otherwise mature (Phases
  76, 84, 86, 89, 90, 91, 93, 96, 97); rare-term recall
  is the biggest remaining open quality gap.
- The substrates are already there. `Memory::search`
  (Phase 74) does the substring side; the semantic side
  is what every recall already runs. Phase 98 just
  composes them via RRF — no new ranker, no new
  tokenization, no new corpus stats.
- Reciprocal Rank Fusion is rank-based, not score-based.
  Cosine scores in `[-1, 1]` and substring hit counts in
  `[0, ∞)` don't need normalization to fuse — RRF only
  cares about each item's position in its ranker's
  ordering. Trivial to implement (~30 lines); trivially
  testable.
- The change is **purely additive**. With
  `recall_hybrid = false` (the default), the recall
  path is the existing semantic-only pipeline byte-
  identical to pre-Phase-98. With `true`, the keyword
  side runs alongside; the fused ranking feeds the same
  downstream stages.
- Reuse is total. No new ranker, no new dependency, no
  new schema. The keyword side reuses
  `Memory::search`; the fusion is a pure helper.

## Streak predictions

- **DESIGN.md** — **Will hold.** Hybrid fusion composes
  two existing rankers via a pure rank-aggregation rule;
  touches no locked technical-contract decision. The
  recall pipeline's downstream stages
  (`rag_min_similarity`, cluster expansion, token
  budget) all see the same `Vec<(MemoryEntry, f32)>`
  shape they always have. Hash at entry:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Prediction: streak **extends to forty-five**
  (currently 44).

- **PRODUCT.md** — **Will hold.** P9 (recall) is
  delivered; this is a quality refinement on its
  ranking. No commitment changed; the operator-facing
  contract is *strengthened* (auto-recall finds rare-
  term hits it currently misses when the operator opts
  in). Hash at entry:
  `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.
  Prediction: streak **extends to thirty-eight**
  (currently 37).

- **Production-core `aivyx-core/src/lib.rs`** — **Will
  hold, by design.** The RRF helper + the keyword-
  ranker call + the integration all live in
  `aivyx-channel`; the config knob in `aivyx-config`.
  No `aivyx-core` touch; no new `AuditTag`; no new
  `KeyDomain`. Hash at entry:
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.
  Prediction: streak **extends to forty-six**
  consecutive phases (new project record, beats Phase
  97's 45).

- **New workspace deps** — Zero. RRF is ~30 lines of
  pure arithmetic over the existing rankers' outputs.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_98.md` + `docs/README.md` status row.

### Task 2 — `[embedding].recall_hybrid` knob

`aivyx-config`:

- `EmbeddingConfig` gains
  `recall_hybrid: bool` (default `false`). With
  `false` the recall path runs semantic-only (byte-
  identical to pre-Phase-98). With `true`, hybrid
  fusion runs (Task 4).
- `RawEmbedding` + the build path. No validation
  bounds (boolean).
- Tests: default; explicit true wins; explicit false
  honored.

### Task 3 — RRF fusion + keyword-ranker pure helper (the crux)

`aivyx-channel`:

- New `recall_fusion` module exposing:
  - `pub fn reciprocal_rank_fusion(
       rankings: &[Vec<(String, u64)>],
       k: usize,
       limit: usize,
     ) -> Vec<(String, u64, f32)>` — pure rank-
     aggregation helper. Each `Vec<(topic, seq)>` is one
     ranker's output in ranked order (position 0 is
     best). Returns up to `limit` items sorted by fused
     score descending. `k` is the RRF constant
     (typically `60`; we use that as a project
     default).
  - `pub const RRF_K: usize = 60;` — the standard
    constant.
- Algorithm: for each `(topic, seq)` appearing in any
  ranker's output, sum `1.0 / (k + rank_in_that_ranker
  + 1)` over the rankers that contain it (rank starts
  at 0). Items not present in a ranker contribute 0
  from that ranker. Sort descending by fused score; ties
  break by topic-asc then seq-desc for determinism.
- Edge cases: empty input → empty output; one ranker →
  the same items in the same order with synthetic fused
  scores; `k = 0` → defended to `k = 1` (the formula
  uses `k + rank + 1` so `k = 0` is well-defined but
  would weight the top-ranked item very heavily; the
  helper defends against an operator setting nonsense
  values).
- Unit tests on the pure fusion helper:
  - Empty rankings → empty output.
  - Single ranker → same order, monotonic-descending
    scores.
  - Two identical rankers → same order as either alone.
  - Two disjoint rankings → all items, ranker-order
    preserved within each subset, interleaved by
    position.
  - Item appearing in both rankings ranks higher than
    items only in one (the core RRF claim).
  - `limit` truncates correctly.
  - Deterministic on tied scores (topic-asc, seq-desc).

### Task 4 — Recall integration

`aivyx-channel/src/memory_recall.rs`:

- `SemanticMemoryContext` gains
  `recall_hybrid: bool` + `with_recall_hybrid(bool)`
  builder.
- Inside `recall()`, when `recall_hybrid = true`:
  1. Run the existing semantic ranker (top-K).
  2. ALSO run `memory.search(query_text, top-K)` — the
     substring side. The same `query_text` that the
     semantic side uses (Phase 86 conversational
     window if engaged, otherwise the bare current
     message — the Q3a commitment).
  3. Build two ranking lists of `(topic, seq)`.
  4. Call `reciprocal_rank_fusion` with both.
  5. Map the fused top-K back to
     `Vec<(MemoryEntry, f32)>` using the existing
     entry fetcher; the score is the RRF score.
  6. The downstream pipeline (`rag_min_similarity`
     floor — note: the RRF score is in a different
     range than cosine, so we skip the floor for
     hybrid path; this is an explicit v1 trade-off,
     documented as a deferral); cluster expansion;
     token budget) operates on the fused result.
- With `recall_hybrid = false`, the call site is
  byte-identical to pre-Phase-98.
- Wiring: the binary's
  `SemanticMemoryContext` construction passes
  `cfg.recall_hybrid` via the new builder.
- Integration tests:
  - Hybrid off → pre-Phase-98 path verbatim (regression
    pin).
  - Hybrid on with a query whose semantic embedding
    misses but whose substring matches → the keyword-
    side hit appears in the fused top-K.
  - Hybrid on with both rankers returning the same top
    entry → that entry is the top of the fused result.

### Task 5 — Surface + docs + exit

- `aivyx learning` surface stays as-is for v1; the
  fusion is invisible at the surface level (it's a
  ranking-internal detail). If operators want a "K
  fused last turn" stat, defer to a follow-up
  observability phase (same shape as Phase 96's
  deferred ANN-surface line).
- `docs/INSTALL.md` — new "Hybrid keyword+semantic
  recall (Phase 98)" subsection under the existing
  Phase 75 semantic-search section: the RRF approach,
  the substring side reuses Phase 74's
  `Memory::search`, the opt-in default, the rare-term
  case.
- `examples/aivyx.toml` — document the new
  `recall_hybrid` knob.
- Exit: ROADMAP + PRODUCT_ROADMAP frozen entries,
  docs/README status flip, prediction-vs-reality,
  hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Fusion:** (a) Reciprocal Rank Fusion (RRF).
  Industry-standard production fusion; rank-based so
  cosine and substring-hit-count don't need
  normalization; ~30 lines of pure code. Default `k =
  60` is the well-known industry value.
- **Q2 — Keyword:** (a) Reuse the existing
  `Memory::search` substring search (Phase 74). Same
  match semantics as the operator-facing search; no
  new tokenization; no corpus stats. RRF is rank-
  based, so the simple substring path gives RRF the
  ranks it needs.
- **Q3 — Query:** (a) Same text the semantic side
  embeds — Phase 86's conversational window if
  engaged, otherwise the bare current message. Single
  source of truth; consistent ranking targets.
- **Q4 — Knob:** (a) New `[embedding].recall_hybrid:
  bool` (default `false`). Matches the established
  Phase 87 / 88 / 91 / 92 / 93 / 95 / 96 / 97 actuator
  opt-in pattern. With the knob off the recall path is
  byte-identical to pre-Phase-98.

## Deferrals

**Rolling deferrals carried into Phase 98** (Phase 75's
hybrid-fusion deferral is **THIS PHASE**):

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
- Phase 75 deferrals (`aivyx memory reembed`, query-
  embedding cache; ANN index closed by Phase 96;
  **hybrid keyword+semantic fusion — THIS PHASE**).
- Phase 76 deferrals — closed by Phase 97.
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
- Phase 86 deferrals (token-budget context sizing —
  closed by Phase 97; embed-each-and-pool windows,
  persisted windows).
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
- Phase 96 deferrals (HNSW-quality recall, iterative
  k-means refinement, persisted ANN index across daemon
  restarts, incremental updates, topic-aware centroid
  seeding, `aivyx learning` ANN backend surface line,
  ANN for `aivyx memory search`).
- Phase 97 deferrals (exact-tokenizer integration; per-
  category budgets; auto-derive from model context
  window; conversational-window budget; mid-item
  truncation strategy; surface line for dropped-by-
  budget count).

**Likely Phase 98 deferrals:**

- **`rag_min_similarity` for the hybrid path.** The
  RRF score isn't a cosine score — the existing floor
  is incomparable to fused scores. v1 skips the
  similarity-floor filter when hybrid is on; a future
  phase could add a separate `rag_hybrid_min_rrf`
  knob or do a post-fusion cosine-recheck.
- **BM25-style keyword scoring.** Substring matching
  is simple but doesn't weight rare terms higher.
  BM25 would give better recall on long-tail queries;
  defers as a future phase that justifies the
  ~100-line addition.
- **Tokenization-aware substring matching.** v1's
  case-insensitive substring tolerates noisy queries
  but matches inside-word fragments ("the" matches
  "their"). A future phase could add word-boundary
  tokenization.
- **`recall_hybrid_k` config knob.** RRF's `k`
  constant is hard-coded to the industry-standard
  `60`. A future phase could expose it if operators
  want to weight rare terms differently.
- **Surface line for `K fused last turn`.** v1's
  `aivyx learning` doesn't show fusion stats. Defers
  as a follow-up observability phase (same shape as
  Phase 96's deferred ANN-backend line).
- **Fused stale-detection / cache.** v1 runs both
  rankers fresh on every recall. A future phase could
  share an embedding cache between the rankers when
  both query texts are identical.

## Prediction vs. reality

**Predictions held — all three streaks correct.**

- **DESIGN.md — held.** Hybrid fusion composes two
  existing rankers via a pure rank-aggregation rule;
  touched no locked technical-contract decision. The
  recall pipeline's downstream stages
  (cluster expansion, token budget) see the same
  `Vec<(MemoryEntry, f32)>` shape they always have —
  only the score's meaning shifts from cosine to RRF
  on the hybrid path. Streak: **45 consecutive phases**
  (was 44).
- **PRODUCT.md — held.** P9 (recall) is delivered;
  this is a quality refinement on its ranking. No
  commitment changed; the operator-facing contract is
  *strengthened* (rare-term queries reliably surface
  memories when the operator opts in). Streak: **38
  consecutive phases** (was 37).
- **`aivyx-core/src/lib.rs` — held, by design.** The
  RRF helper + the recall integration all live in
  `aivyx-channel`; the config knob in `aivyx-config`.
  No `aivyx-core` touch; no new `AuditTag`; no new
  `KeyDomain`. Streak: **46 consecutive phases** —
  new project record, beating Phase 97's 45.

**Test count — `+16`** (workspace `1730 → 1746`).
**Slightly over** the predicted `+10-15` band by 1.
Breakdown:

- Config knob `+3` (default, explicit-true, explicit-
  false-honored).
- Pure RRF module `+10` (empty rankings; all-empty;
  zero-limit; single-ranker preserves order; two
  identical rankings double scores; the core RRF
  claim — item in both outranks single-ranker hits;
  limit truncates; defended k=0; disjoint rankings;
  deterministic on tied scores).
- Recall integration `+3` (off-is-semantic-only
  regression pin; rare-term surfaces via keyword side;
  both-rankers-agree top hit wins).

The `+1 over` is consistent with the project's pattern
for "new pure module with multiple boundary cases."
Phase 96 was +15 on its pure ANN module; Phase 97 was
+13 on its pure token-budget module. The RRF helper
similarly earned ~10 individual boundary-case tests,
matching the established calibration shape.

**Scope — every planned surface shipped exactly as scoped.**
The `recall_fusion` module with `RRF_K = 60` constant +
`reciprocal_rank_fusion` helper; the `SemanticMemoryContext`
gains `recall_hybrid: bool` + `with_recall_hybrid(bool)`
builder; the recall path dispatches on `recall_hybrid`
to either the pre-Phase-98 semantic-only path or the
new hybrid path that runs both `semantic_search_scored`
AND `Memory::search`, fuses via RRF, and maps fused
`(topic, seq)` back to `MemoryEntry`; the
`rag_min_similarity` floor is explicitly skipped on the
hybrid path (RRF scores incomparable to cosine) with a
documented `rag_hybrid_min_rrf` deferral; the binary
wires the knob from `[embedding].recall_hybrid`. Zero
clippy warnings after two trivial `cloned_ref_to_slice_refs`
cleanups (a pattern that's now appeared in both Phase 97
and Phase 98 — newer clippy revision). Zero new
workspace deps.

**Implementation note on entry-body lookup.** The fused
result is `Vec<(topic, seq, f32)>` but the downstream
pipeline (cluster expansion, token budget, format_block)
expects `Vec<(MemoryEntry, f32)>`. The hybrid path
builds a `HashMap<(String, u64), MemoryEntry>` lookup
from BOTH rankers' returns (the semantic side returns
`(MemoryEntry, f32)`; the keyword side returns
`Vec<MemoryEntry>`). Each entry inserts once
(`or_insert_with` on the keyword side avoids
overwriting the semantic-side clone). The fused list
then `filter_map`s through the lookup to recover the
bodies — items that fell out of both rankers' top-K
(impossible by construction but defensively handled)
are filtered out. No extra memory fetch; no extra
storage I/O on the recall hot path.

**Why the floor is skipped, restated.** The `rag_min_similarity`
default is `0.20` (cosine in `[-1, 1]`). An RRF score
for a position-0 item with `k=60` is `1/61 ≈ 0.0164`;
for an item in both rankings at position 0 it's
`2/61 ≈ 0.0328`. The floor `0.20` would reject every
fused hit. The correct fix is a separate `rag_hybrid_min_rrf`
knob (a documented deferral), not running the cosine
floor against incomparable RRF scores. v1 makes this
trade-off explicitly; the `rag_top_k` cap still bounds
the output size.

## Exit criteria

- [x] `[embedding].recall_hybrid: bool` (default
  `false`) — Task 2 (commit `faabd11`).
- [x] `reciprocal_rank_fusion` pure helper in
  `aivyx-channel::recall_fusion` — Task 3 (commit
  `da0ac58`).
- [x] Unit tests on the pure helper: empty / single
  ranker / two identical / two disjoint / item in
  both / `limit` truncation / deterministic on
  ties — Task 3 (commit `da0ac58`).
- [x] `SemanticMemoryContext::recall` runs both
  rankers + fuses when `recall_hybrid = true`; byte-
  identical pre-Phase-98 path with the knob off —
  Task 4 (commit `5ad87cd`).
- [x] Integration tests: hybrid-off regression, hybrid-
  on rare-term recall, hybrid-on both-rankers-agree —
  Task 4 (commit `5ad87cd`).
- [x] `docs/INSTALL.md` + `examples/aivyx.toml`
  updated — Task 5 (this commit).
- [x] ROADMAP + PRODUCT_ROADMAP + docs/README
  refreshed — Task 5 (this commit).
- [x] All four Q-block questions resolved with
  operator sign-off pre-Task 2.
- [x] DESIGN.md streak extends to forty-five.
- [x] PRODUCT.md streak extends to thirty-eight.
- [x] Production-core streak extends to forty-six
  (new record) — `lib.rs` byte-identical.
- [x] Test count delta: positive (`+16`, slightly over
  the predicted `+10-15` band by 1 — accounted for by
  the RRF module earning ~10 boundary-case tests in
  line with the project's pattern for new pure
  modules).
- [x] Zero clippy warnings (two trivial cleanups).
- [x] Zero new workspace deps.
- [x] Prediction-vs-reality block filled.
