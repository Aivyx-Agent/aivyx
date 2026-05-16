# Phase 75 — Semantic RAG: Embedding-Backed Memory Retrieval

Phase journals are working documents. They churn freely during
the phase and freeze at exit under a final Exit criteria block.
For the locked technical contract see [`../DESIGN.md`](../DESIGN.md);
for the locked product contract see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Close Phase 74's largest deferral: the RAG arc. `memory.search`
ships keyword-only today (case-insensitive substring). Phase 75
adds a `mode = "semantic"` path backed by embedding-vector
cosine similarity, so the agent can recall by *meaning* —
"what did the operator say about deployment" matches a note
titled `release/runbook` that never contains the word
"deployment".

The design holds the project's two load-bearing disciplines:

1. **Zero new workspace deps.** The embedding provider is an
   OpenAI-compatible HTTP client reusing the existing
   `aivyx-llm` transport (the same `reqwest`/transport path
   the chat providers use). No bundled model, no ONNX, no
   `candle`.
2. **Local-first stays possible.** `[embedding] base_url` is
   operator-configured. Point it at `api.openai.com` (memory
   content leaves the box — the operator's explicit choice)
   OR a local OpenAI-compatible server (ollama,
   llama.cpp, text-embeddings-inference) and everything stays
   on-device. The privacy decision is the operator's
   base_url, not forced by the implementation.

After Phase 75 the self-learning triad's memory leg gains the
retrieval quality the vision always implied — recall by
meaning, not just literal string match — without compromising
the privacy stance or the dep discipline.

## Why now

1. **Phase 74 explicitly deferred this.** Q1(a) at Phase 74
   sign-off shipped keyword-only "semantic retrieval defers
   to a future RAG arc." Phase 75 is that arc.
2. **Substrate is in place.** `memory.search` tool + IPC +
   Web UI + CLI all exist (Phase 74); Phase 75 adds a `mode`
   flag and a vector-cosine path behind the same surfaces.
   The OpenAI provider's configurable `base_url` over a
   shared transport (Phase 25) makes the embedding client a
   thin addition.
3. **Q-block fully resolved at design time.** OpenAI-compatible
   HTTP endpoint (Q1(a)), write-time + lazy backfill (Q2(a)),
   `KeyDomain::MemoryVectors` + in-memory index (Q3(a)), mode
   flag with keyword auto-fallback (Q4(a)) all signed off
   pre-Task 2.

## Streak predictions

- **DESIGN.md** — **Will hold.** Phase 75 adds a `KeyDomain`
  variant (precedent: Phases 21/26/27/56/70 all added
  variants without contract edits), an `[embedding]` config
  section, an `EmbeddingProvider` trait + impl, vector
  storage + cosine search, and a `mode` flag — none touch
  the locked technical contract. Prediction: streak
  **extends to twenty-two** consecutive phases (currently 21).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** Memory is G3 substrate
  (already delivered); semantic retrieval is a quality
  improvement, not a new commitment. No commitment-text
  edits. Prediction: streak **extends to fifteen**
  consecutive phases.
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 75 work lives in `aivyx-storage` (new KeyDomain),
  `aivyx-llm` (embedding provider), `aivyx-config`
  (`[embedding]` section), `aivyx-memory` (vector store +
  cosine), and `aivyx-channel` (wiring + mode flag).
  `aivyx-core` is untouched. Prediction: streak **extends
  to twenty-three** consecutive phases (new project record,
  beats Phase 74's 22).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

- **New workspace deps** — Zero. The embedding HTTP client
  reuses the existing `aivyx-llm` transport (`reqwest` +
  rustls, already in tree since Phase 25). Cosine similarity
  is a hand-rolled f32 dot-product / norm — no `ndarray` or
  linear-algebra crate.

## Tasks

### Task 1 — Open commit + PHASE_75.md scaffold

This file. Update `docs/README.md` to show Phase 75 as Open.

### Task 2 — `KeyDomain::MemoryVectors` storage variant

`aivyx-storage`:

- New `KeyDomain::MemoryVectors` variant. HKDF info bytes
  `b"memory-vectors"`, table `aivyx_memory_vectors_v1`.
  Bump `ALL` to `[KeyDomain; 12]`, the subkey array, and the
  `key_domain_all_covers_every_variant` tripwire. One
  isolation test (same key in `Memory` vs `MemoryVectors`
  doesn't collide).

### Task 3 — `EmbeddingProvider` trait + OpenAI-compatible HTTP impl

`aivyx-llm`:

- `pub trait EmbeddingProvider { async fn embed(&self, texts:
  &[String]) -> Result<Vec<Vec<f32>>, EmbeddingError>; fn
  model(&self) -> &str; fn dimensions(&self) -> usize; }`.
- `OpenAiEmbeddingProvider` — POST `{base_url}/v1/embeddings`
  with `{ model, input }`, parse `data[].embedding`. Reuses
  the existing transport + auth-header pattern from the
  chat provider. Configurable `base_url` (default
  `https://api.openai.com`), `model` (default
  `text-embedding-3-small`), `api_key`.
- `EmbeddingError` taxonomy mirroring the notify-error
  shape: `Transport`, `Auth`, `Rejected(u16)`, `Timeout`,
  `Malformed`.

### Task 4 — `[embedding]` config section

`aivyx-config`:

- `pub struct EmbeddingConfig { base_url: String, model:
  String, api_key: Option<SecretString>, dimensions: usize }`.
- `RawEmbedding` parses `[embedding]`; absent section →
  `None` (semantic search disabled, keyword still works).
  Validation: non-empty base_url, non-empty model,
  dimensions ≥ 1. API key may come from env
  (`AIVYX_EMBEDDING_API_KEY`) or the encrypted store
  fall-through (same two-phase pattern as the anthropic /
  openai keys).
- `AivyxConfig` gains `pub embedding: Option<EmbeddingConfig>`.

### Task 5 — Vector store + cosine search in `aivyx-memory`

`aivyx-memory`:

- `Memory` trait gains:
    - `put_vector(topic, seq, vector) -> Result<(), MemoryError>`
    - `load_all_vectors() -> Result<Vec<(String, u64, Vec<f32>)>, MemoryError>`
      (startup index build)
    - `semantic_search(query_vec, limit) -> Result<Vec<MemoryEntry>, MemoryError>`
      — cosine over the in-memory index, returns the top
      `limit` entries by similarity (decoding the matching
      `MemoryEntry` bodies via the existing entry store).
- In-memory `VectorIndex { entries: Vec<(String, u64,
  Vec<f32>)> }` built at `RedbMemory::open` from the
  `MemoryVectors` domain. `InMemoryMemory` keeps a parallel
  in-process index.
- `forget(topic)` + `evict_oldest_unread` also drop the
  corresponding vectors (consistency: a forgotten topic
  leaves no orphan vector).

### Task 6 — Write-time embed + lazy backfill

`aivyx-channel`:

- `MemoryWriteTool` (or the daemon write path) computes the
  embedding for the new body via the configured
  `EmbeddingProvider` and calls `put_vector`. Provider
  failure is non-fatal: the entry is still written; it just
  lacks a vector (keyword still finds it; backfill retries).
- Backfill pass reuses the hourly GC timer cadence: walk
  entries, find those without a vector in `MemoryVectors`,
  embed in batches, store. Bounded batch size so one tick
  doesn't stall on a huge unembedded corpus.

### Task 7 — `mode` flag + semantic path + keyword fallback

`memory.search` tool / `SearchMemory` IPC / `aivyx memory
search` CLI / Web UI search bar:

- `memory.search` input gains `mode: "keyword" | "semantic"`
  (default `keyword` — no behavior change for existing
  callers).
- Semantic path: embed the query via the provider, run
  `Memory::semantic_search`, return ranked matches.
- Auto-fallback (Q4(a)): semantic requested but no
  `[embedding]` config OR provider call fails OR the corpus
  has zero vectors → transparently fall back to keyword and
  set a `fell_back_to_keyword: true` flag in the result so
  the agent/operator knows.
- CLI: `aivyx memory search <query> [--semantic] [--limit N]`.
- Web UI: a "semantic" toggle next to the search bar.

### Task 8 — Tests + docs + exit

Tests: KeyDomain isolation, embedding provider (mocked HTTP:
success, auth-fail, malformed, timeout), config parse +
validation + env/store key fall-through, cosine ranking
correctness (orthogonal vs aligned vectors), backfill
idempotence, mode-flag fallback paths, IPC round-trip, CLI
parser + render, Web UI HTML smoke.

Docs: `examples/aivyx.toml` `[embedding]` block with both
the cloud and local-server base_url examples + the
privacy note. `docs/INSTALL.md` "Semantic memory search
(Phase 75)" section covering the base_url privacy choice,
the keyword-fallback behavior, and the backfill-on-upgrade
note.

Exit: ROADMAP + PRODUCT_ROADMAP frozen entries, docs/README
status flip, prediction-vs-reality, hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Embedding source:** (a) OpenAI-compatible HTTP
  endpoint with operator-configured `base_url`. Reuses the
  existing transport (zero new deps). Privacy is the
  operator's base_url choice — cloud API or local server.
- **Q2 — Embedding timing:** (a) Write-time inline + lazy
  backfill pass on the GC cadence. Provider outage is
  non-fatal — unembedded entries fall back to keyword and
  backfill retries.
- **Q3 — Vector storage:** (a) New `KeyDomain::MemoryVectors`
  (encrypted, persisted, keyed by topic+seq) + an in-memory
  flat index built at daemon startup for fast cosine. The
  `MemoryEntry` at-rest shape is unchanged.
- **Q4 — Search surface:** (a) `mode` flag on the existing
  `memory.search` (default keyword). Semantic auto-falls-back
  to keyword (with a result flag) when no provider is
  configured or the call fails. One tool, one CLI, one Web
  UI affordance.

## Deferrals

**Rolling deferrals carried into Phase 75:**

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

**Likely Phase 75 deferrals:**

- **Approximate-nearest-neighbour index.** v1 ships a
  brute-force flat cosine scan (fine for tens of thousands
  of entries). HNSW / IVF indexing defers until a corpus
  size makes the linear scan a real latency problem.
- **Re-embedding on model change.** If the operator swaps
  embedding models, old vectors are dimensionally
  incompatible. v1 detects a dimension mismatch and treats
  those entries as unembedded (backfill re-embeds them);
  an explicit `aivyx memory reembed` command defers.
- **Hybrid keyword+semantic fusion ranking.** v1 picks one
  mode per query. Reciprocal-rank-fusion of both is a
  follow-up if operators want it.
- **Embedding cache for repeated queries.** v1 embeds the
  query every semantic search. A small LRU on
  (query → vector) defers until pressure surfaces.

## Prediction vs. reality

**Streak — all three predictions correct.**

- **DESIGN.md → 22.** Held, byte-identical. Exit hash
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`
  == entry hash. The `KeyDomain` variant + `[embedding]`
  section + `EmbeddingProvider` trait + vector store + `mode`
  flag all landed in non-contract crates, as predicted.
- **PRODUCT.md → 15.** Held, byte-identical. Exit hash
  `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`
  == entry hash. Semantic retrieval was a G3 quality
  improvement, no commitment-text edit — as predicted.
- **Production-core `aivyx-core/src/lib.rs` → 23.** Held,
  byte-identical. Exit hash
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`
  == entry hash. **New project record (beats Phase 74's 22).**
  All work lived in `aivyx-storage`, `aivyx-llm`,
  `aivyx-config`, `aivyx-memory`, `aivyx-channel`.

**Test delta — inside prediction.** +46 (1378 → 1424),
within the predicted +35-50. Spread: storage isolation (T2),
mocked-HTTP embedding provider 9 (T3), config parse/validate/
fall-through 8 (T4), vector store + cosine 17 (T5), backfill
5 (T6), mode-flag fallback + IPC round-trip + CLI parser +
HTML smoke (T7).

**Zero clippy warnings, zero new workspace deps** — both held
(one `cloned_ref_to_slice_refs` lint surfaced and was fixed
with `std::slice::from_ref` during T7).

**Surprises / deviations (none contract-affecting):**

- The plan said "`MemoryWriteTool` (or the daemon write
  path)". To keep `aivyx-memory` free of an `aivyx-llm`
  dependency, an `EmbeddingHook` trait was introduced *in*
  `aivyx-memory` and the concrete `aivyx-llm`-backed adapter
  (`LlmEmbeddingHook`) lives in `aivyx-channel`. The write
  tool and the search tool both consume the hook; the daemon
  owns the backfill driver. Cleaner than threading a provider
  through the tool crate, and it preserves the crate
  dependency direction.
- `MemorySearchTool` (not just the IPC handler) needed the
  hook too, since the agent-facing `mode = "semantic"` path
  must embed the query. Anticipated by the plan listing the
  tool as a surface; called out here because it added a
  builder + Debug field beyond the write tool.
- The backfill reused the existing memory-GC timer by
  broadening its spawn condition to also fire when only
  `[embedding]` is configured — exactly the Q2(a) "GC
  cadence" intent, no new task loop.

## Exit criteria

- [x] `KeyDomain::MemoryVectors` + isolation test — Task 2.
- [x] `EmbeddingProvider` trait + OpenAI-compatible HTTP
  impl + error taxonomy — Task 3.
- [x] `[embedding]` config section + validation + env/store
  key fall-through — Task 4.
- [x] `Memory` vector store + in-memory index + cosine
  `semantic_search` + forget/evict drop vectors — Task 5.
- [x] Write-time embed + lazy backfill on GC cadence —
  Task 6.
- [x] `mode` flag on memory.search + semantic path +
  keyword auto-fallback with result flag — Task 7.
- [x] CLI `--semantic` flag + Web UI semantic toggle —
  Task 7.
- [x] Tests across storage, provider, config, cosine,
  backfill, fallback, IPC, CLI, HTML smoke — Task 8.
- [x] `examples/aivyx.toml` + `docs/INSTALL.md` updated —
  Task 8.
- [x] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 8.
- [x] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [x] DESIGN.md streak extends to twenty-two.
- [x] PRODUCT.md streak extends to fifteen.
- [x] Production-core streak extends to twenty-three (new
  record).
- [x] Test count delta: positive (+46, 1378 → 1424).
- [x] Zero clippy warnings.
- [x] Zero new workspace deps.
- [x] Prediction-vs-reality block filled.
