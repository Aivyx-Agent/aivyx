# Phase 100 — Tool-Surface Gap Closure (Chapter B opener)

The capability vocabulary in `aivyx-capability` was declared
ahead of the tools that exercise it. `fs.delete` and
`fs.metadata` are **D4-original substrate scope bases** —
present since Phase 0, enumerated in Amendment A3's base
inventory with the `path glob` qualifier kind — but neither
has ever had a first-party tool behind it. The agent can read
a file and write a file; it cannot delete one or stat one. An
everyday gap, and the most concrete one in the whole declared-
but-toolless set.

Closing it is not a free addition. **Amendment A5 locked P10**
("Substrate-Only Core") to *exactly eight first-party tools*:
`fs.read`, `fs.write`, `memory.read`, `memory.write`,
`memory.forget`, `shell.exec`, `web.fetch`, `web.post` — and
states in terms that "adding to or removing from this list
requires a `PRODUCT.md` amendment." `fs.delete` and
`fs.metadata` are unambiguously substrate (operator-facing
filesystem operations, the same category as `fs.read` /
`fs.write` — not agent self-management). So Phase 100 is, by
the project's own rules, an **amendment phase**: it files
**Amendment A6** extending P10's enumerated list from eight to
ten, then ships the two tools behind it. This is not novel
process — it is exactly the path Amendment A5 itself set when
Phase 37 added `web.post` and P10 went from seven to eight.

Phase 100 also audits the *rest* of the declared-but-toolless
delta — `shell.spawn`, `net.dns`, `audit.read`,
`config.read` / `config.write`, `display.window_close`,
`memory.gc`, `mission.gate` — and rules each one "tool now" or
"deliberately reserved," recording the verdict so the audit
never has to be redone. The expectation at entry is that all
eight stay reserved and only the two filesystem tools ship,
keeping the amendment to a clean eight-to-ten count change.

## Why this, why now

- `fs.delete` / `fs.metadata` are the only declared-but-
  toolless scopes that name an *everyday* capability the
  agent visibly lacks. The other eight gate
  self-management or rare operations; the filesystem pair
  is the real gap.
- Chapter B (opened at the Phase 99 exit) is the tooling
  arc. Its first phase should be the most concrete, ready
  item — surface breadth before the softer reliability /
  observability / SDK work. This is that item.
- The scope bases already exist. `fs.delete` and
  `fs.metadata` have been in D4's substrate inventory since
  Phase 0; the qualifier kind (`path glob`) is the same one
  `fs.read` / `fs.write` use. Phase 100 ships tools for a
  vocabulary the contract already anticipated — it does not
  invent capability surface.
- The amendment is small and precedented. A6 changes one
  word and extends one list, exactly as A5 did. The
  substrate-only *principle* is untouched; only the count
  moves.
- Reuse is high. The new tools are siblings of `FsReadTool`
  / `FsWriteTool` in `aivyx-core/src/tools/fs.rs`: same
  `Config`-builder shape, same canonicalize-then-scope-gate
  sandbox discipline, same `path glob` scope construction.
  No new crate (D8's workspace lock holds), no new
  dependency (`std::fs` covers delete and metadata).

## Streak predictions

- **DESIGN.md** — **Will hold.** `fs.delete` and
  `fs.metadata` are D4-original scope bases; shipping their
  tools changes no locked *technical*-contract decision.
  No new crate — D8's workspace lock stands. The one risk
  is Q1: if directory listing earns a *new* `fs.list` scope
  base rather than folding into `fs.metadata`, that extends
  D4's base inventory and would require a DESIGN.md
  amendment, breaking this streak. The entry prediction
  assumes Q1(a) — listing folds into `fs.metadata`. Hash at
  entry:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Prediction: streak **extends to forty-seven** (currently
  46), conditional on Q1(a).

- **PRODUCT.md** — **Will break, by design.** Amendment A6
  edits P10's enumerated tool list (eight → ten). That is a
  deliberate, precedented contract change — the only legal
  way to add a substrate tool. Hash at entry:
  `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.
  Prediction: streak **ends at thirty-nine** and resets;
  the next phase begins a fresh PRODUCT.md streak. This is
  the correct outcome, not a regression — cf. Phase 56's
  P13/P14 amendments and Phase 38's A5.

- **Production-core `aivyx-core/src/lib.rs`** — **Will
  break.** `FsDeleteTool` / `FsDeleteToolConfig` /
  `FsMetadataTool` / `FsMetadataToolConfig` join the
  existing `pub use tools::{FsReadTool, …}` re-export block
  at `lib.rs:44`. Hiding the new tools behind only the
  `tools` submodule path to preserve a streak would be
  streak-driven design — which the project rejects (cf.
  the deliberate Phase 51 break). Hash at entry:
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.
  Prediction: streak **ends at forty-seven** (the record it
  set) and resets.

- **New workspace deps** — Zero. `std::fs::remove_file` /
  `remove_dir_all` and `std::fs::metadata` cover both
  tools.

- **Test count** — Positive. Each new tool earns a full
  boundary suite in `fs.rs` mirroring the `FsReadTool` /
  `FsWriteTool` coverage (scope gate, sandbox-escape
  rejection, canonicalization, not-found, wrong-type).
  Rough prediction: **+25 to +40** workspace.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_100.md` + `docs/ROADMAP.md` entry flip to
`Active` + `docs/README.md` status row.

### Task 2 — Amendment A6 + scope-vs-tool-registry audit

- `docs/amendments/<date>-substrate-tool-count-ten.md` —
  Amendment A6. Narrows P10: "eight" → "ten",
  `fs.delete` and `fs.metadata` added to the enumerated
  list. Same shape as A5. Updates `PRODUCT.md` P10 text
  and Delivery Status.
- The declared-but-toolless audit: a table in this doc
  ruling each of `shell.spawn`, `net.dns`, `audit.read`,
  `config.read`, `config.write`, `display.window_close`,
  `memory.gc`, `mission.gate` either "tool — future phase"
  or "deliberately reserved," with one line of reasoning
  each.
- Resolved Q-block answers folded in.

### Task 3 — `fs.delete` tool

`aivyx-core/src/tools/fs.rs`:

- `FsDeleteToolConfig` + `FsDeleteTool`, mirroring
  `FsWriteTool`: `new(sandbox_root)` → `build() ->
  Result<_, AivyxError>` (canonicalize the root, reject a
  non-directory), `Tool::name() == "fs.delete"`, required
  scope `fs.delete:<canonical_abs>` constructed per the
  same `path glob` discipline.
- The canonicalize-then-scope-gate sandbox-escape
  defence is non-negotiable and copied verbatim from
  `FsWriteTool`.
- Per Q3: a `shell.exec`-style registration-time trust
  gate — `fs.delete` is registered for Local channels
  only and is absent entirely from a SemiTrusted dispatch
  registry. Deletion is non-recursive: `remove_file` for
  files, `remove_dir` for empty directories only; a
  non-empty directory is a clean tool error, never a
  recursive wipe.
- Full boundary test suite.

### Task 4 — `fs.metadata` tool

`aivyx-core/src/tools/fs.rs`:

- `FsMetadataToolConfig` + `FsMetadataTool` — read-only
  stat: size, file type, modified time, permissions
  (mode bits). Required scope `fs.metadata:<abs>`.
- Directory listing resolved per Q1: under Q1(a) a
  `fs.metadata` call on a directory returns its entries.
- Full boundary test suite.

### Task 5 — Binary wiring + verification + docs + exit

- `aivyx-core/src/lib.rs` + `tools/mod.rs` re-exports for
  the four new public types.
- Binary registers both tools alongside the existing
  `FsReadToolConfig` / `FsWriteToolConfig` block, granting
  `fs.delete:<root>/**` and `fs.metadata:<root>/**` so the
  agent can exercise them.
- `scripts/dev-verify.sh` gains tool-path probes for
  `fs.delete` and `fs.metadata`.
- `docs/TOOL_SDK.md` first-party-tool count refreshed;
  `docs/INSTALL.md` if operator-facing.
- Exit: ROADMAP + PRODUCT_ROADMAP + docs/README,
  prediction-vs-reality, hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Directory listing:** (a) **Fold into
  `fs.metadata`.** A `fs.metadata` call on a directory
  returns its entries. No new scope base, no second
  amendment; P10 lands at exactly ten tools and the
  DESIGN.md streak holds at 47. A dedicated `fs.list`
  scope/tool is recorded as a deferral.
- **Q2 — Amendment A6 shape:** (a) **A6 extends P10's
  enumerated list from eight to ten.** `fs.delete` and
  `fs.metadata` are added as substrate tools (operator-
  facing filesystem operations, not self-management).
  The substrate-only principle and the rest of P10 are
  untouched — only the count and the list move. The A5
  pattern (Phase 37 / `web.post`, seven → eight) repeated.
- **Q3 — `fs.delete` trust treatment:** (b) **Local-only
  registration gate, non-recursive.** `fs.delete` gets a
  registration-time trust-tier gate like `shell.exec`: it
  is absent entirely from a SemiTrusted channel's
  dispatch registry — a SemiTrusted audit chain never
  sees `fs.delete` mentioned, not even as a denial.
  Deletion is scoped to files and empty directories only
  (`std::fs::remove_file` / `remove_dir`); recursive
  `remove_dir_all` is out of scope and recorded as a
  deferral. This matches the destructive-capability
  blast-radius posture `shell.exec` already sets.
- **Q4 — The other eight toolless scopes:** (a)
  **Document the verdict only.** `shell.spawn`,
  `net.dns`, `audit.read`, `config.read`, `config.write`,
  `display.window_close`, `memory.gc`, and `mission.gate`
  are each ruled "tool — future phase" or "deliberately
  reserved" in the Task 2 audit table. Phase 100 ships no
  tool for them and removes no scope base — scope-base
  removal would be its own amendment.

## Deferrals

**Rolling deferrals carried into Phase 100** (Phase 100
closes none — it is net-new Chapter B tool-surface work):

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

**Likely Phase 100 deferrals:**

- **Tools for the other eight toolless scopes.** Per Q4,
  Phase 100 documents `shell.spawn` / `net.dns` /
  `audit.read` / `config.read` / `config.write` /
  `display.window_close` / `memory.gc` / `mission.gate`
  as reserved; any that earn a tool later are their own
  Chapter B phases.
- **`fs.list` as a first-class scope.** If Q1 resolves to
  (a), a dedicated directory-listing scope/tool is a
  documented future option (it would need its own D4
  amendment).
- **Recursive directory deletion.** If Q3 scopes
  `fs.delete` to files-and-empty-dirs, `remove_dir_all`-
  style recursive deletion is a deferred follow-up gated
  on operator need.
- **`fs.metadata` field breadth.** v1 returns size / type
  / mtime / permissions. Extended attributes, symlink
  target resolution, and inode/device identifiers defer
  to operator-feedback.

## Prediction vs. reality

*Filled at phase exit.*

## Exit criteria

- [x] `docs/PHASE_100.md` + ROADMAP entry flip + docs/README
  status row — Task 1 (this commit).
- [ ] Amendment A6 extends P10 to ten tools; `PRODUCT.md`
  updated; declared-but-toolless audit table recorded —
  Task 2.
- [ ] `fs.delete` tool in `aivyx-core` with the full
  sandbox-escape defence + boundary suite — Task 3.
- [ ] `fs.metadata` tool in `aivyx-core` with directory
  listing per Q1 + boundary suite — Task 4.
- [ ] Binary registration + `lib.rs` re-exports +
  `dev-verify.sh` probes + docs — Task 5.
- [ ] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed
  at exit — Task 5.
- [ ] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [ ] DESIGN.md streak extends to forty-seven (holds —
  conditional on Q1(a)).
- [ ] PRODUCT.md streak ends at thirty-nine and resets
  (Amendment A6 — by design).
- [ ] Production-core `lib.rs` streak ends at forty-seven
  and resets (new tool re-exports).
- [ ] Test count delta positive (predicted `+25`–`+40`).
- [ ] Zero clippy warnings.
- [ ] Zero new workspace deps.
- [ ] Prediction-vs-reality block filled.
