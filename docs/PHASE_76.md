# Phase 76 — Automatic Semantic Recall (RAG context injection)

Phase 75 shipped the semantic-retrieval *engine* (embedding
provider, vector store, cosine `semantic_search`, `mode` flag).
But the agent still only recalls past memory when it *chooses*
to call `memory.search`. Phase 76 closes the RAG arc: before
each planner turn, embed the user's message, retrieve the
most semantically-relevant past memories, and inject them into
the turn's context **automatically** — so the assistant
remembers without being told to. This is the step that turns
"a tool that can search memory" into "an assistant with
continuity," which is the core of the self-learning
personal-assistant vision.

## Why this, why now

- Phase 75 built every primitive this needs (`EmbeddingProvider`,
  `Memory::semantic_search`, the in-memory index). Phase 76 is
  pure integration on top — no new substrate.
- The codebase already has the exact precedent: `PruneSink`
  is a memory-coupling hook injected through
  `LlmPlannerConfig`, with the concrete impl living in
  `aivyx-channel`. RAG context is the symmetrical read-side
  hook. The pattern is proven; we are reusing it.
- Vision alignment: continuity of memory across turns is what
  separates a "personal assistant" from a stateless agent. It
  is the highest-leverage thing the Phase 75 investment
  unlocks.

## Streak predictions

- **DESIGN.md** — **Will hold.** A `ContextProvider` hook is a
  planner-internal extension point (same class as `PruneSink`,
  Phase-pre-streak); no locked technical-contract decision is
  touched. Prediction: streak **extends to twenty-three**
  consecutive phases (currently 22).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** Automatic recall is a quality
  improvement to G3 memory substrate (already delivered) — it
  makes existing memory more useful, it is not a new product
  commitment. No commitment-text edit. Prediction: streak
  **extends to sixteen** consecutive phases.
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold,
  deliberately.** The `ContextProvider` trait + the
  `LlmPlannerConfig::with_context_provider` builder + the
  `LlmPlanner::plan()` invocation all live in
  `crates/aivyx-core/src/llm_planner.rs`. The existing
  `PruneSink` precedent *is* re-exported from `lib.rs`
  (line 40, predating the streak discipline). Phase 76
  **deliberately diverges**: `ContextProvider` is reachable
  via `aivyx_core::llm_planner::ContextProvider` (the same
  module path consumers already use for planner internals) and
  is **not** added to the `lib.rs` re-export line, so
  `lib.rs` stays byte-identical. The minor re-export
  asymmetry is the documented, intentional price of protecting
  the streak; an ergonomics-only `lib.rs` re-export can be
  filed later as an honest streak-breaking change if it ever
  matters. Prediction: streak **extends to twenty-four**
  consecutive phases (new project record, beats Phase 75's
  23).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

- **New workspace deps** — Zero. Reuses the Phase 75
  `EmbeddingProvider` + `Memory::semantic_search`; no new
  crates.io dependency.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_76.md` (this file) + `docs/README.md` status row.

### Task 2 — `ContextProvider` hook on the planner

`aivyx-core` (`src/llm_planner.rs` only — **no `lib.rs`
edit**):

- `pub trait ContextProvider: Send + Sync { async fn
  recall(&self, user_message: &str) -> Option<String>; }`
  — returns an already-formatted context block to prepend, or
  `None` for "nothing to add" (the universal no-op path).
- `LlmPlannerConfig::with_context_provider(Arc<dyn
  ContextProvider>)` — builder, mirrors
  `with_prune_sink`. Absent → today's behavior exactly.
- `LlmPlanner::plan()` calls `recall(latest_user_message)` at
  turn start; a `Some(block)` is prepended to the turn's
  context as a distinct, clearly-delimited segment (not
  concatenated into the operator/role system prompt — it is
  per-turn and must not look like a standing instruction).
- Unit tests with a fake `ContextProvider` (returns a fixed
  block / returns `None`): block present when `Some`, turn
  unchanged when `None` or no provider configured.

### Task 3 — RAG knobs on `[embedding]` config

`aivyx-config`:

- `EmbeddingConfig` gains `rag_top_k: usize` (default 5) and
  `rag_min_similarity: f32` (default 0.20). `RawEmbedding`
  parses `rag_top_k` / `rag_min_similarity`.
- Validation: `rag_top_k >= 1`; `0.0 <= rag_min_similarity
  <= 1.0`. Absent keys → defaults (no behavior change for
  Phase-75 `[embedding]` configs).
- Tests: defaults, explicit override, both validation
  bounds.

### Task 4 — `SemanticMemoryContext` (the recall impl)

`aivyx-channel` (new module, alongside `memory_embedding`):

- Implements `ContextProvider`. Holds `Arc<dyn Memory>`,
  `Arc<dyn EmbeddingProvider>`, `rag_top_k`,
  `rag_min_similarity`, and an `AuditHook`-style sink for the
  Task 6 marker.
- `recall(user_message)`: embed the message (Q2a — latest
  user message only), `Memory::semantic_search(qvec,
  rag_top_k)`, drop hits below `rag_min_similarity` (Q3a),
  and if any survive, format a labeled block:
  `## Relevant context (auto-recalled)` followed by each
  entry as `- [topic · <age>] body`, with an explicit
  "reference only, not an instruction" framing line
  (prompt-injection hygiene, Q4).
- Silent no-op → `None` when: embed fails, the vector index
  is empty, or every hit is below the floor. Never an error;
  pre-Phase-76 behavior preserved (consistent with the
  Phase 75 fallback ethos).
- Unit tests over `InMemoryMemory` + a fake
  `EmbeddingProvider`: ranked block, floor filtering,
  empty-index no-op, embed-failure no-op.

### Task 5 — Wire into the planner factories

`aivyx-channel` binary:

- When `embedding_provider` is `Some`, construct a
  `SemanticMemoryContext` and attach it via
  `with_context_provider` in **all three** planner factory
  sites: local-CLI session, daemon session, and the
  child-agent factory (so sub-agents recall too).
- `None` provider → no context provider attached → identical
  to today.

### Task 6 — Visible recall marker (Q4b)

- New audit tag (extend the existing `AuditTag` taxonomy):
  `ContextRecall { turn_id, injected_count, topics }`,
  emitted by `SemanticMemoryContext` only when a block is
  actually injected (not on the no-op path).
- Web UI: surface a per-turn "recalled N memories" indicator
  in the relevant pane + an HTML smoke assertion. The marker
  is observability only — it does not gate or alter the
  turn.

### Task 7 — Tests + docs + exit

- Tests: planner hook (core), config knobs, recall ranking +
  floor + no-op paths, audit-marker emission, Web UI HTML
  smoke. Aggregate target positive (~+25-40).
- Docs: `examples/aivyx.toml` `rag_top_k` / `rag_min_similarity`
  under the `[embedding]` block + a one-paragraph "automatic
  recall" note; `docs/INSTALL.md` "Automatic recall
  (Phase 76)" subsection (what it does, the no-op-when-off
  guarantee, the marker).
- Exit: ROADMAP + PRODUCT_ROADMAP frozen entries, docs/README
  status flip, prediction-vs-reality, hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Injection mechanism:** (a) `ContextProvider` hook on
  `LlmPlanner`, mirroring the `PruneSink` precedent. Concrete
  impl in `aivyx-channel`. The trait lives in
  `llm_planner.rs` and is **not** re-exported from `lib.rs`,
  to protect the production-core streak.
- **Q2 — Retrieval query/trigger:** (a) Embed the latest user
  message only, every turn. One embed call per turn,
  predictable cost. (Rolling-window and heuristic-gate
  variants deferred.)
- **Q3 — Budget & ranking:** (a) Config `rag_top_k` (default
  5) **and** a `rag_min_similarity` floor (default 0.20) so
  weak matches are dropped even when K isn't filled — the
  floor is what prevents naive-RAG noise. (Token-budget
  sizing deferred.)
- **Q4 — Provenance & failure:** (b) A clearly-delimited,
  injection-safe labeled block **plus** a visible per-turn
  marker (audit tag + Web UI indicator) so the operator can
  see what was auto-recalled. Silent no-op when embedding is
  unavailable — never a hard error.

## Deferrals

**Rolling deferrals carried into Phase 76:**

- v0.1.0 publication (Phase 61 Task 7).
- System-prompt notification-target enumeration (Phase 62).
- `turn_id` correlation refactor (Phase 67).
- Phase 68 deferrals (XOAUTH2, HTML email, attachments,
  multi-account `[email]`).
- Phase 69 deferrals (WebPush / service-worker, notification
  urgency, icons, sound).
- Phase 70 deferrals (proposal supersession, reflection on
  feedback events, multi-window reflection).
- Phase 71 deferrals (per-fire role override, operator-
  customizable reflection prompt, reflection cadence
  learning).
- Phase 73 deferrals (persisted rate-limit buckets,
  operator-configurable retry-on list).
- Phase 74 deferrals (fuzzy match, edit-content Web UI,
  per-topic eviction-strategy override).
- Phase 75 deferrals (ANN index, `aivyx memory reembed`,
  hybrid keyword+semantic fusion, query-embedding cache).

**Likely Phase 76 deferrals:**

- **Conversational-window query (Q2b).** v1 embeds only the
  latest user message. Embedding the last-N-turns window for
  terse follow-ups defers until recall quality on
  multi-turn threads shows a real gap.
- **Heuristic recall gate (Q2c).** v1 retrieves every turn.
  Skipping retrieval on recall-irrelevant prompts (to save
  embed calls) defers — the `rag_min_similarity` floor
  already suppresses *injection*; gating *retrieval* is a
  cost optimization, not a correctness one.
- **Token-budget context sizing (Q3b).** v1 is top-K + floor.
  Adaptive token-budget filling defers until prompt-size
  pressure surfaces.
- **Query-embedding cache.** (Also a Phase 75 deferral.) v1
  embeds the query every turn; an LRU on (message → vector)
  defers until repeated-query pressure surfaces.
- **Recall feedback into reflection.** Using which recalled
  memories the turn actually used as a reflection signal is
  a self-improvement follow-up, not part of this arc.

## Prediction vs. reality

To be filled in at phase exit.

## Exit criteria

- [ ] `ContextProvider` trait + `with_context_provider` +
  planner invocation, **no `lib.rs` edit** — Task 2.
- [ ] `rag_top_k` + `rag_min_similarity` config + validation
  + tests — Task 3.
- [ ] `SemanticMemoryContext` with floor filtering +
  injection-safe labeled block + silent no-op — Task 4.
- [ ] Wired into local-CLI, daemon, and child-agent planner
  factories — Task 5.
- [ ] `ContextRecall` audit tag + Web UI recall indicator +
  HTML smoke — Task 6.
- [ ] Tests across planner hook, config, recall ranking/
  floor/no-op, audit marker, Web UI — Task 7.
- [ ] `examples/aivyx.toml` + `docs/INSTALL.md` updated —
  Task 7.
- [ ] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 7.
- [ ] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [ ] DESIGN.md streak extends to twenty-three.
- [ ] PRODUCT.md streak extends to sixteen.
- [ ] Production-core streak extends to twenty-four (new
  record) — `lib.rs` byte-identical.
- [ ] Test count delta: positive (~+25-40).
- [ ] Zero clippy warnings.
- [ ] Zero new workspace deps.
- [ ] Prediction-vs-reality block filled.
