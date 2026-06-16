# Phase 74 — Memory Polish: Search, Retention, LRU, Web UI Pane

Phase journals are working documents. They churn freely during
the phase and freeze at exit under a final Exit criteria block.
For the locked technical contract see [`../DESIGN.md`](../../../DESIGN.md);
for the locked product contract see [`../PRODUCT.md`](../../../PRODUCT.md).

## Goal

Open the next vision-aligned arc after the Reach Milestone
closure. Today the memory substrate (Phase 5+) is the minimum-
viable shape: topic/body/seq/created_at, three methods
(`put` / `get_recent` / `forget`), and a single global
`memory_ttl_secs` for GC. Phase 74 makes memory the third leg
of "self-learning" alongside Persona (P14) and reflection
(Phases 70-71):

1. **Keyword search.** New `memory.search { query }` tool with
   case-insensitive substring matching across topics + content.
   Operators and the agent get a workhorse retrieval path
   without the embedding-model dep cost (semantic retrieval
   defers to a future RAG arc per Q1(a)).
2. **Per-topic retention policies.** New `[[memory.retention]]`
   config blocks with topic-glob patterns let operators
   declare per-topic-class retention: `topic_glob = "project/*"`
   with `retention = "forever"`, `topic_glob = "notes/*"` with
   `retention_days = 30`. The background GC walks each entry,
   matches against the first applicable pattern, and falls
   through to the global `memory_ttl_secs` for unmatched topics.
3. **LRU eviction.** Each `MemoryEntry` gains a
   `last_read_at_secs` field; `Memory::get_recent` updates it
   on every read. When a topic exceeds `memory_max_per_topic`,
   the entry with the oldest `last_read_at_secs` is evicted
   (Q3(a)). Operators see "the stuff I haven't looked at is
   the stuff I lose first."
4. **Web UI Memory pane.** Read-only browse with paginated
   topic list + per-topic entry view + inline search bar + a
   per-topic Evict button (operator-confirmed). CLI parity:
   `aivyx memory list / show / search / evict`.

After Phase 74 the self-learning triad is complete:
- Persona (P14) shipped Phase 56-60, autonomous via Phase 70.
- Reflection shipped Phase 70-71, autonomous via Phase 71.
- Memory shipped Phase 5+, with Phase 74 closing the polish
  gap that's been quietly accruing since Phase 6.

## Why now

1. **Reach Milestone closed end-to-end (Phases 62-73).** Every
   notify-axis polish item shipped. The natural pivot is the
   next vision-aligned arc.
2. **Self-learning triad needs all three legs.** Persona +
   reflection are mature; memory is the holdout. Operators
   accumulating memory over weeks need ways to find, retain,
   and prune what they've stored.
3. **Q-block fully resolved at design time.** Keyword search
   (Q1(a)), config-based retention (Q2(a)), LRU on
   last_read_at (Q3(a)), read-only-browse + evict Web UI
   (Q4(a)) all signed off pre-Task 2.

## Streak predictions

- **DESIGN.md** — **Will hold.** Phase 74 extends the
  `Memory` trait (new required methods on the substrate),
  adds a config section, adds an audit-less in-memory path,
  and adds IPC envelopes — none touched the locked technical
  contract. Prediction: streak **extends to twenty-one**
  consecutive phases (currently at 20).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** No commitment-text edits;
  memory is the substrate for G3 (Reflection Layer commitment)
  which is already considered delivered. Prediction: streak
  **extends to fourteen** consecutive phases.
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 74 work lives in `aivyx-memory` (trait + impl
  extensions), `aivyx-config` (retention section),
  `aivyx-channel` (IPC + Web UI + CLI), and `aivyx-tool` or
  similar (the search tool). `aivyx-core` is untouched.
  Prediction: streak **extends to twenty-two** consecutive
  phases (new record, beats Phase 73's 21).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

- **New workspace deps** — Zero expected. `globset` (used for
  topic-glob retention patterns) is already in workspace as a
  transitive dep. Substring/fuzzy matching uses straight
  `str::contains` for v1; if operator pressure for fuzzy
  surfaces we'll add `strsim` or similar in a follow-up.

## Tasks

### Task 1 — Open commit + PHASE_74.md scaffold

This file. Update `docs/README.md` to show Phase 74 as Open.

### Task 2 — MemoryEntry + Memory trait extensions

`aivyx-memory`:

- `MemoryEntry` gains `pub last_read_at_secs: u64`
  (`#[serde(default)]` for backwards-compat — pre-Phase-74
  entries deserialize with `0`, which is the "never read,
  most-eligible-for-eviction" sentinel).
- `Memory` trait gains four new methods:
    - `search(query, limit) -> Result<Vec<MemoryEntry>, MemoryError>`
      — case-insensitive substring match across all topics +
      bodies; returns up to `limit` matches, newest first.
    - `list_topics() -> Result<Vec<String>, MemoryError>`
      — distinct topic names across the substrate, sorted.
    - `topic_entries(topic, limit) -> Result<Vec<MemoryEntry>, MemoryError>`
      — alias for `get_recent` exposed under a clearer name
      for the Web UI / CLI consumption path.
    - `evict_oldest_unread(topic, keep) -> Result<usize, MemoryError>`
      — when a topic has more than `keep` entries, evicts
      entries with the smallest `last_read_at_secs` until
      the count reaches `keep`. Returns the number evicted.
- `get_recent` updates `last_read_at_secs` to the current
  time on every entry it returns (Q3(a) LRU semantics).
  In-memory + redb impls both implement the new methods.

### Task 3 — Config: `[[memory.retention]]` blocks

`aivyx-config`:

- New `pub struct MemoryRetentionRule { topic_glob: String,
  retention: RetentionPolicy }`.
- `pub enum RetentionPolicy { Forever, ForDays(u64) }`.
- `RawMemoryRetention` parses `topic_glob = "..."` (required),
  `retention = "forever"` (literal string) or
  `retention_days = N` (numeric). Exactly one of these must
  be set per block.
- Loader validates each `topic_glob` compiles via
  `globset::GlobBuilder`. Empty glob, malformed glob, or
  partial retention config rejects at load-time.
- `AivyxConfig` gains `pub memory_retention: Vec<MemoryRetentionRule>`.

### Task 4 — GC integration: retention + LRU eviction

`aivyx-channel` (the existing memory-GC timer at startup):

- The hourly GC pass walks every entry. For each entry,
  finds the first matching retention rule (first-match wins,
  so operators put narrower globs first); applies
  `Forever` (skip eviction) or `ForDays(N)` (evict if
  `created_at_secs + N*86400 < now`).
- Topics with no matching rule fall through to the existing
  `memory_ttl_secs` default behavior (no change for
  pre-Phase-74 configs).
- LRU eviction triggers when `memory_max_per_topic` is
  exceeded post-write. Calls `Memory::evict_oldest_unread`.

### Task 5 — `memory.search` tool

`aivyx-channel`:

- New `MemorySearchTool` registered via the existing tool
  surface. Capability: `memory.read` (reuses the existing
  scope; search is "reading the substrate broadly").
- Input schema: `{ query: string, limit?: integer (default 20) }`.
- Output: `{ matches: [{ topic, body, seq, created_at_secs,
  last_read_at_secs }] }`.
- Delegates to `Memory::search`.

### Task 6 — IPC envelopes + daemon-side handlers

`aivyx-channel/src/daemon_ipc.rs`:

- `QueryPayload::ListMemoryTopics`
- `QueryPayload::GetMemoryTopicEntries { topic, limit }`
- `QueryPayload::SearchMemory { query, limit }`
- `FrontendMessage::EvictMemoryTopic { id, topic }` +
  `DaemonMessage::MemoryEvictResolved { id, ok, evicted, error }`

`daemon_server.rs`:

- `handle_query` arms for the three Query variants delegate
  to the Memory trait.
- `EvictMemoryTopic` calls `Memory::forget(topic)` (or a new
  per-topic-delete; depends on Task 2's surface) and emits a
  resolution.

### Task 7 — Web UI Memory pane

`web_ui_static.html`:

- New tab `data-pane="memory"` between Notifications and
  the rest.
- Topic list view (left column) + topic detail view (right
  column) with entries.
- Inline search bar drives `SearchMemory` query; results
  populate the detail view.
- Per-topic Evict button (confirm modal) sends
  `EvictMemoryTopic`.
- HTML smoke test: tab presence, three query labels,
  EvictMemoryTopic envelope construction.

### Task 8 — CLI + tests + docs + exit

CLI surface:

- `aivyx memory list` — print all topics with entry counts.
- `aivyx memory show <topic>` — print entries for a topic.
- `aivyx memory search <query> [--limit N]` — keyword search.
- `aivyx memory evict <topic> [--yes]` — operator-driven
  delete; `--yes` skips the confirm prompt.

Tests across: trait method semantics (search filtering, LRU
selection, list_topics ordering, evict_oldest_unread
correctness), config retention parsing + validation, GC
integration (retention rule first-match, LRU on overflow),
IPC round-trips, Web UI HTML smoke, CLI parser + render.

Docs: `examples/aivyx.toml` gains a commented
`[[memory.retention]]` block; `docs/INSTALL.md` gets a
"Memory subsystem (Phase 74)" section explaining search,
retention, LRU, and the Web UI pane.

Exit: ROADMAP frozen entry, PRODUCT_ROADMAP memory polish
note, docs/README status flip, prediction-vs-reality fill,
hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Search shape:** (a) Keyword search only (substring,
  case-insensitive). No embedding dep cost; predictable
  latency. Semantic retrieval defers to a future RAG arc.
- **Q2 — Retention expression:** (a) Config-based
  `[[memory.retention]]` blocks with topic-glob patterns.
  Greppable, version-controlled, validated at config-load
  time. First-match wins so operators put narrower globs
  first.
- **Q3 — Eviction strategy:** (a) LRU on `last_read_at_secs`.
  Stored on every `MemoryEntry`; updated on every
  `get_recent`. Most operator-aligned: "least-recently-read
  goes first."
- **Q4 — Web UI scope:** (a) Read-only browse + search +
  per-topic evict. No edit-content path — the Web UI is not
  a write surface against agent memory. Operators who need
  to modify content do it via the existing memory tools
  through agent interaction.

## Deferrals

**Rolling deferrals carried into Phase 74:**

- v0.1.0 publication (Phase 61 Task 7).
- System-prompt notification-target enumeration (Phase 62).
- `turn_id` correlation refactor (Phase 67).
- Phase 68 deferrals (XOAUTH2, HTML email, attachments,
  multi-account `[email]`).
- Phase 69 deferrals (WebPush / service-worker, notification
  urgency, icons, sound).
- Phase 70 deferrals (proposal supersession, reflection on
  feedback events, multi-window reflection, memory/role
  proposal flows).
- Phase 71 deferrals (per-fire role override, operator-
  customizable reflection prompt, reflection cadence
  learning).
- Phase 73 deferrals (persisted rate-limit buckets,
  operator-configurable retry-on list).

**Likely Phase 74 deferrals:**

- **Semantic retrieval (RAG arc).** Q1(b)/(c) — embeddings
  via the existing LLM or a local model. Real engineering
  (embedding cache, vector index, per-query latency budget).
  Defers until operator pressure or a concrete RAG use case
  surfaces.
- **Fuzzy matching.** v1 ships straight substring match;
  `strsim` or similar for typo-tolerant queries lands as a
  small follow-up if operators surface real need.
- **Edit-content Web UI path.** Q4(c). Defers per
  threat-model preference — the Web UI is a read-only
  surface against agent memory in v1.
- **Per-topic eviction strategy override.** Q3(c)'s glob-
  configurable LRU/FIFO/etc. Defers until operators surface
  need; v1 ships LRU universally.

## Prediction vs. reality

**All three streak predictions correct.**

- **DESIGN.md** — Held. Hash at exit:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`
  (byte-identical to entry). Streak extends to **twenty-one**
  consecutive phases as predicted. Phase 74 added Memory
  trait methods, a config section, IPC envelopes, a Web UI
  pane, and a CLI subcommand — none touched the locked
  technical contract.
- **PRODUCT.md** — Held. Hash at exit:
  `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`
  (byte-identical to entry). Streak extends to **fourteen**
  consecutive phases. Memory is G3 substrate (already
  delivered); Phase 74 polishes it without commitment-text
  edits.
- **Production-core `aivyx-core/src/lib.rs`** — Held. Hash
  at exit:
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`
  (byte-identical to entry). Streak extends to **twenty-two**
  consecutive phases — **new project record**, beating Phase
  73's 21. All work lived in `aivyx-memory`, `aivyx-config`,
  and `aivyx-channel`.
- **Workspace deps** — Zero new as predicted. `globset` was
  already a transitive workspace dep through
  `aivyx-capability`; substring matching used straight
  `str::contains`.
- **Tests** — +45 (1333 → 1378), above the +30-40
  prediction. Breakdown: 10 memory-trait tests (search,
  list_topics, evict_oldest_unread, get_recent LRU stamp) +
  3 retention-aware-GC tests, 9 config tests (retention
  parse + validation), 5 memory.search tool tests, 8 IPC
  round-trip cases, 1 Web UI HTML smoke, 16 CLI parser +
  render-helper tests.
- **Clippy** — Zero warnings across the workspace.
- **Q-block** — All four resolutions held in implementation:
  - **Q1(a)** — Keyword search only. `Memory::search` is a
    case-insensitive substring match; `memory.search` tool +
    `SearchMemory` IPC + `aivyx memory search` CLI + Web UI
    search bar all delegate to it. No embedding dep.
  - **Q2(a)** — Config-based `[[memory.retention]]` with
    topic-glob patterns. First-match wins in the GC pass;
    unmatched topics fall through to the global `ttl_secs`.
    `globset::GlobMatcher` compiled + validated at
    config-load.
  - **Q3(a)** — LRU on `last_read_at_secs`. The field is on
    every `MemoryEntry` (serde-defaulted for backwards
    compat); `get_recent` stamps it on both substrate impls;
    `evict_oldest_unread` ranks `(last_read ASC, seq ASC)`.
  - **Q4(a)** — Read-only browse + search + evict Web UI. No
    edit-content path. The pane is a two-column topic-list +
    detail layout with a confirm-gated per-topic Evict
    button.

After Phase 74 the self-learning triad is complete: Persona
(P14, autonomous via Phase 70), reflection (autonomous via
Phase 71), and memory (search + retention + LRU + operator
surfaces via Phase 74).

## Exit criteria

- [x] `MemoryEntry::last_read_at_secs` + serde-default
  backwards compat — Task 2.
- [x] `Memory` trait gains `search` / `list_topics` /
  `topic_entries` / `evict_oldest_unread` — Task 2.
- [x] `get_recent` updates `last_read_at_secs` on every
  read — Task 2.
- [x] `[[memory.retention]]` config section parses +
  validates topic_glob + retention discriminant — Task 3.
- [x] GC pass respects retention rules first-match + falls
  through to `memory_ttl_secs` default — Task 4.
- [x] LRU eviction kicks in when `memory_max_per_topic` is
  exceeded — Task 4.
- [x] `memory.search` tool registered + delegates to the
  substrate — Task 5.
- [x] `ListMemoryTopics` / `GetMemoryTopicEntries` /
  `SearchMemory` / `EvictMemoryTopic` IPC envelopes +
  daemon-side handlers — Task 6.
- [x] Web UI Memory pane: topic list, detail view, inline
  search, per-topic Evict — Task 7.
- [x] CLI: `aivyx memory list / show / search / evict` —
  Task 8.
- [x] Tests across trait semantics, config validation, GC
  integration, IPC, HTML smoke, CLI — Task 8.
- [x] `examples/aivyx.toml` + `docs/INSTALL.md` updated —
  Task 8.
- [x] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 8.
- [x] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [x] DESIGN.md streak extends to twenty-one.
- [x] PRODUCT.md streak extends to fourteen.
- [x] Production-core streak extends to twenty-two (new
  record).
- [x] Test count delta: positive (~+30-40).
- [x] Zero clippy warnings.
- [x] Zero new workspace deps.
- [x] Prediction-vs-reality block filled.
