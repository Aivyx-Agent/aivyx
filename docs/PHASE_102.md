# Phase 102 — Tool Observability (`aivyx tools`) (Chapter B)

Phase 100 widened the tool surface; Phase 101 made tool
calls more reliable. Neither gave the operator a way to
*see* the tool layer. Today the only window onto tool
activity is `aivyx --verify-only`, which walks the audit
chain for integrity, not for insight — it reports an event
count, not "which tools the agent actually uses, and how
often they fail." The registered tool *set* is equally
opaque: `--print-role` renders a role's capability
envelope, but nothing answers the plain question "what
tools does this agent have, and what has it done with
them?"

Phase 102 adds `aivyx tools` — a read-only observability
subcommand, the sibling of `aivyx memory` and `aivyx
learning`. It answers both halves of that question in one
view: it **lists every registered tool** (name,
description, capability base) and **annotates each with
audit-derived call statistics** — total calls, the
outcome breakdown (completed / failed / denied / …), and
call timing. A `--window <secs>` flag scopes the stats to
a recent slice, the same affordance `aivyx learning`
offers. The data flows over a new daemon IPC query, so the
view reflects the live daemon's registered tool set joined
against its audit chain.

## Why this, why now

- It is the Chapter B "tool observability" item, named at
  the Phase 99 exit. With the surface (Phase 100) and
  reliability (Phase 101) work shipped, *seeing* the tool
  layer is the remaining gap.
- The data already exists. Every tool call is an
  `AuditEvent::ToolCall { tool_id, scope_used, outcome,
  duration, … }` in the chain — Phase 102 reads what is
  already written, it does not add new instrumentation.
  `scope_used.base()` is the stable, human-meaningful key
  (`fs.read`, `memory.write`), unlike the per-process
  `tool_id`.
- The pattern is established. `aivyx memory` and `aivyx
  learning` are read-only daemon-query subcommands with a
  rendering module each; `aivyx tools` is the same shape,
  and `--window` mirrors `aivyx learning --window`.
- It closes the loop on the reliability work. Phase 101's
  `invalid_input` repairs and tool failures land in the
  audit chain as `ToolCall` outcomes; `aivyx tools` is
  where an operator sees a tool that fails a lot.
- The change is additive. A new `QueryPayload` variant is
  backward-compatible under the Phase 41 protocol-
  negotiation handshake (Amendment A7); an older client
  simply never sends it.

## Streak predictions

- **DESIGN.md** — **Will hold.** A new `QueryPayload` /
  `QueryResponsePayload` variant is an additive,
  backward-compatible extension of the daemon IPC
  protocol — exactly what the protocol-negotiation
  handshake (Amendment A7) exists to absorb. No locked
  technical-contract decision changes; the protocol shape
  is documented in `docs/DAEMON_IPC.md`, not `DESIGN.md`.
  Hash at entry:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Prediction: streak **extends to forty-nine**
  (currently 48).

- **PRODUCT.md** — **Will hold.** A read-only
  observability subcommand changes no product-shape
  decision; it is the sibling of the existing `aivyx
  memory` / `aivyx learning` views. Hash at entry:
  `9f0a515c9076544866aa955d6835763ee15beb0d009b4c335f5710b6d9ba61d3`.
  Prediction: streak **extends to two** (currently 1).

- **Production-core `aivyx-core/src/lib.rs`** — **Will
  hold.** All of Phase 102 is `aivyx-channel` — the daemon
  IPC types, the query handler, the CLI subcommand, the
  rendering module. `aivyx-core` is not touched. Hash at
  entry:
  `ab3f9730c692917023239bbdd7c375497459e2a7fb3bbf08c007b5c945c6210d`.
  Prediction: streak **extends to two** (currently 1).

- **New workspace deps** — Zero. The audit-chain walk,
  the IPC frame, and the table rendering all use crates
  already in the tree.

- **Test count** — Positive. The per-tool stat
  aggregation (the audit-walk fold keyed by scope base,
  with the window filter) earns a unit suite; the IPC
  round-trip and the CLI parse earn their own. Rough
  prediction: **+12 to +20**.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_102.md` + `docs/ROADMAP.md` entry flip to
`Active` + `docs/README.md` status row.

### Task 2 — IPC protocol: `GetToolStats` query + response

`aivyx-channel/src/daemon_ipc.rs`:

- `QueryPayload::GetToolStats { window_secs: Option<u64> }`
  — `None` = whole chain, `Some(n)` = the last `n`
  seconds.
- `QueryResponsePayload::ToolStats { tools: Vec<ToolStat> }`.
- A `ToolStat` struct: `name`, `description`,
  `scope_base`, `registered` (bool — present in the live
  registry), `calls`, an outcome breakdown
  (`completed` / `failed` / `denied` / `not_in_role` /
  `requires_escalation` counts), and `total_duration` /
  call → average derivable client-side.
- `serde` round-trip; the variant slots after the
  existing query variants so the wire enum stays
  append-only.
- `docs/DAEMON_IPC.md` — a `GetToolStats` addendum.

### Task 3 — Daemon-side `GetToolStats` handler

`aivyx-channel/src/daemon_server.rs`:

- Thread the registered tools' descriptors (name,
  description, capability base) to `handle_query` — the
  daemon builds the registry today but the descriptors do
  not reach the query handler; Phase 102 captures a
  lightweight descriptor list at daemon construction and
  passes it through.
- The `GetToolStats` arm walks the audit chain (the
  handler already holds `audit_log`), folds
  `AuditEvent::ToolCall` events keyed by
  `scope_used.base()` into per-tool counts + outcome
  breakdown + duration sum, applies the `window_secs`
  filter, then joins against the descriptor list so a
  registered-but-never-called tool still appears (zero
  counts) and a called-but-no-longer-registered base
  still appears (`registered: false`).

### Task 4 — `aivyx tools` CLI subcommand + renderer

- `parse_cli_args` recognizes `aivyx tools [--window
  <secs>]`, mirroring the `aivyx learning` parse.
- A new `aivyx_modules/tools.rs` rendering module (the
  sibling of `learning.rs`): connects to the daemon,
  sends `GetToolStats`, and renders a flat-text table —
  one row per tool with name, base, call count, outcome
  breakdown, and average duration.
- `--no daemon running` surfaces the same actionable
  error `aivyx memory` already gives.

### Task 5 — tests + exit

- Unit tests on the stat-aggregation fold (empty chain;
  one tool many calls; outcome breakdown; window filter
  includes/excludes by timestamp; registered-but-uncalled
  and called-but-unregistered join cases).
- IPC round-trip test for the new variant; CLI parse
  tests for `tools` / `tools --window N`.
- `docs/INSTALL.md` — an `aivyx tools` entry alongside the
  other read-only subcommands.
- Exit: ROADMAP frozen entry, docs/README status flip,
  prediction-vs-reality, hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Scope:** (c) **Both — listing + stats.** `aivyx
  tools` lists every registered tool and annotates each
  with its audit-derived call statistics. Chosen over a
  stats-only or listing-only view: the operator question
  is "what tools, and what have they done" — one view
  answers both, and a registered-but-uncalled tool
  (visible only because listing is included) is itself a
  useful signal.
- **Q2 — Data source:** (b) **Daemon query.** A new
  daemon IPC `GetToolStats` query, the same shape as
  `aivyx memory` / `aivyx learning`. Chosen over an
  offline audit-chain walk so the view reflects the
  *live* registered tool set (the listing half needs the
  running daemon's registry) rather than only what a
  cold store can reconstruct.
- **Q3 — Per-tool detail:** (c) **Counts + outcome
  breakdown + timing + a `--window <secs>` filter.** The
  fullest row: total calls, the per-outcome split,
  average duration, and a recent-slice filter mirroring
  `aivyx learning --window`. The `AuditEvent::ToolCall`
  record already carries `outcome` and `duration`, so the
  timing column is nearly free.

## Deferrals

**Rolling deferrals carried into Phase 102** (Phase 102
closes none — it is net-new Chapter B observability work):

- v0.1.0 publication (Phase 61 Task 7).
- System-prompt notification-target enumeration (Phase 62).
- `turn_id` correlation refactor (Phase 67).
- Phase 68 deferrals (XOAUTH2, HTML email, attachments,
  multi-account `[email]`).
- Phase 69 deferrals (WebPush / service-worker, notification
  urgency, icons, sound).
- Phase 70 deferrals (multi-window reflection).
- Phase 71 deferrals (per-fire role override, operator-
  customizable reflection prompt).
- Phase 73 deferrals (persisted rate-limit buckets,
  operator-configurable retry-on list).
- Phase 74 deferrals (fuzzy match, edit-content Web UI,
  per-topic eviction-strategy override).
- Phase 75 deferrals (`aivyx memory reembed`, query-
  embedding cache).
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
- Phase 86 deferrals (embed-each-and-pool windows,
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
  similarity supersession).
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
- Phase 98 deferrals (`rag_hybrid_min_rrf` floor knob;
  BM25-style keyword scoring; tokenization-aware
  substring matching; operator-tunable `recall_hybrid_k`;
  surface line for fused stats; shared embedding cache
  between rankers).
- Phase 99 deferrals (Anthropic-backend dev mode; CI /
  remote-build wiring; committed dev config / role
  templates; Telegram / web-UI channel verification;
  `cargo`-level e2e harness; `--config` flag vs. the
  stale `examples/aivyx-ollama.toml` comment).
- Phase 100 deferrals (tools for the eight reserved
  toolless scopes; `fs.list` as a first-class scope;
  recursive directory deletion; `fs.metadata` field
  breadth).
- Phase 101 deferrals (compiled-schema cache;
  `additionalProperties` tightening; skipped-call
  detection; operator-visible repair stats).

**Likely Phase 102 deferrals:**

- **`aivyx tools` in the Web UI.** Phase 102 ships the
  terminal view only. A Web UI tool-stats panel is a
  follow-on, the same way `aivyx learning` has a terminal
  view ahead of any UI surface.
- **Per-tool latency percentiles.** The row carries total
  and average duration; p50/p95 would need either a
  histogram or a full duration list — deferred until an
  operator wants tail-latency visibility.
- **Repair-round stats.** Phase 101's `invalid_input`
  repair rounds are visible in the chain as the calls
  that preceded a successful one, but `aivyx tools` does
  not yet surface a dedicated "repaired N times" column —
  a candidate refinement once the repair feature has
  real-use data.
- **`tool_id` → name stability across restarts.** Stats
  key on `scope_used.base()`, which is stable; a
  per-`tool_id` drill-down would need a stable tool
  identity across daemon restarts (related to the Phase
  67 `turn_id` correlation deferral).

## Prediction vs. reality

*Filled at phase exit.*

## Exit criteria

- [x] `docs/PHASE_102.md` + ROADMAP entry flip + docs/README
  status row — Task 1 (this commit).
- [ ] `QueryPayload::GetToolStats` + `QueryResponsePayload::
  ToolStats` + the `ToolStat` struct; `DAEMON_IPC.md`
  addendum — Task 2.
- [ ] Daemon `GetToolStats` handler — descriptor list
  threaded to `handle_query`, audit-walk fold keyed by
  scope base with the window filter, joined to the
  registry listing — Task 3.
- [ ] `aivyx tools [--window <secs>]` CLI subcommand +
  the `aivyx_modules/tools.rs` renderer — Task 4.
- [ ] Stat-aggregation, IPC round-trip, and CLI-parse
  tests — Task 5.
- [ ] `docs/INSTALL.md` `aivyx tools` entry — Task 5.
- [ ] ROADMAP + docs/README refreshed at exit — Task 5.
- [ ] All three Q-block questions resolved with operator
  sign-off pre-Task 2.
- [ ] DESIGN.md streak extends to forty-nine.
- [ ] PRODUCT.md streak extends to two.
- [ ] Production-core `lib.rs` streak extends to two.
- [ ] Zero new workspace dependencies.
- [ ] Test count delta positive (predicted `+12`–`+20`).
- [ ] Zero clippy warnings.
- [ ] Prediction-vs-reality block filled.
