# Phase 103 — External Tool Ergonomics (`aivyx tool init`) (Chapter B)

Three Chapter B phases shipped the *operator's* tool experience:
the surface widened (Phase 100), the calls got more reliable
(Phase 101), and the activity became visible (Phase 102).
Phase 103 closes the chapter on the *third-party tool author's*
experience — the operator who wants to ship a tool the agent
hasn't shipped already.

That author's path today is "read `docs/TOOL_SDK.md`, then copy
`examples/python-tool/` and adapt." That works, but it has two
edges. The example is Python; an author already in the Rust
ecosystem who wants the type-safety and tooling of the
substrate has nothing to copy. And "copy this directory and
edit" is implicit ceremony — it works, but `aivyx init` (Phase
44) and `aivyx init --template <name>` (Phase 66) set the
project's convention that scaffolding is an explicit
subcommand, not an instruction.

Phase 103 adds `aivyx tool init <path>` — the third entry in
the `init` family. It writes a runnable Rust tool-process
project to `<path>`: a `Cargo.toml` that pulls in the existing
`aivyx-tool` crate (which already re-exports the wire types
and framing — no new SDK helper to add), a `src/main.rs` with
the handshake + invocation main loop and a handler stub the
author replaces, a `README.md` explaining how to wire it into
`aivyx.toml`, a conformance test, and a `[[tool_process]]`
snippet to paste into the operator's config. The author edits
one function body; the protocol scaffolding is done.

## Why this, why now

- It is the Chapter B "external tool ergonomics" item — the
  last named entry in the chapter, the one that closes it.
- The substrate is already there. `aivyx-tool` re-exports
  `frame::{read_frame, write_frame, …}` and
  `wire::{DaemonToTool, ToolToDaemon, ToolDescriptor,
  Verification, TOOL_PROTOCOL_VERSION}` — every type a tool
  process needs to speak the protocol. Phase 103 ships a
  *project* that depends on those, not a *new helper*.
- It is the convention the project already set. `aivyx init`
  scaffolds an operator config (Phase 44); `aivyx init
  --template <name>` scaffolds a named profile (Phase 66);
  `aivyx tool init` is the same idea for the other side of
  the SDK contract.
- Rust over Python. Q2 resolved to Rust because the existing
  `examples/python-tool/` already covers the stdlib-Python
  case, and the missing scaffold is for an author who wants
  type-checked wire structs (the `wire.rs` enums) and the
  `cargo` toolchain. Python remains a first-class option via
  the existing example.
- The change is additive — a new CLI subcommand, no daemon
  IPC, no existing path altered.

## Streak predictions

- **DESIGN.md** — **Will hold.** `aivyx tool init` is an
  operator-side scaffolding command; it touches no locked
  technical-contract decision and adds no daemon-IPC variant.
  Hash at entry:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Prediction: streak **extends to fifty** — a project
  milestone, fifty consecutive DESIGN.md-byte-identical
  phases.

- **PRODUCT.md** — **Will hold.** A tool-author scaffolding
  command is third-party-tool ergonomics, not a substrate-
  tool addition; P10's enumerated ten-tool list stays exact,
  P11 (SDK contract) and P12 (tool-process IPC) are
  unchanged. Hash at entry:
  `9f0a515c9076544866aa955d6835763ee15beb0d009b4c335f5710b6d9ba61d3`.
  Prediction: streak **extends to three** (currently 2).

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold.**
  All of Phase 103 lives in `aivyx-channel`'s binary (a new
  CliMode variant, a parse block, a `tool_init` module that
  writes files from embedded template strings); `aivyx-core`
  is not touched. Hash at entry:
  `ab3f9730c692917023239bbdd7c375497459e2a7fb3bbf08c007b5c945c6210d`.
  Prediction: streak **extends to three** (currently 2).

- **New workspace deps** — Zero. The scaffolding command
  writes files from `include_str!`-style embedded constants
  and uses `std::fs`; the *generated* project pulls
  `aivyx-tool`, but that is a downstream-project dependency,
  not a workspace one.

- **Test count** — Positive. The scaffold-fidelity test
  drives `aivyx tool init <tmp>` end-to-end, asserts the
  output tree shape, and (Q3-driven) runs `cargo check`
  against the generated project to prove it compiles against
  the real `aivyx-tool` crate. Plus CLI-parse tests. Rough
  prediction: **+5 to +10**.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_103.md` + `docs/ROADMAP.md` entry flip to
`Active` + `docs/README.md` status row.

### Task 2 — `aivyx tool init <path>` subcommand + scaffold

- A new `tool` CLI subcommand with one sub-subcommand,
  `init`, taking a target `<path>`. Refuses to overwrite a
  non-empty directory unless `--force` is passed (the
  established `aivyx init`/`aivyx identity import` pattern).
- A new `aivyx_modules/tool_init.rs` module that writes the
  scaffold from embedded template strings. The scaffold is
  the full Q3 set:
  - `Cargo.toml` — a `[package]` block + `aivyx-tool` dep
    (placeholder using a path/git form; documented at the
    top, since the Distribution milestone has not yet
    published the crate to crates.io).
  - `src/main.rs` — the handshake (`ToolHello` →
    `ToolRegister`) plus the invocation main loop
    (`InvokeTool` → handler → `ToolResult`, plus
    `CancelInvocation` / `ToolShutdown` / `ToolError`
    paths). A single `handler` function with a one-tool
    `ToolDescriptor` declared inline; the author replaces
    the body.
  - `README.md` — what it is, how to build, how to wire it
    via `[[tool_process]]`, link back to
    `docs/TOOL_SDK.md`.
  - `tests/conformance.rs` — a daemon-free conformance
    test (build a fake daemon over two channels of bytes,
    drive the handshake + one invocation, assert the
    handler's output flows back).
  - A printed `[[tool_process]]` snippet at command exit
    so the operator can paste it into `aivyx.toml`
    without opening the README.

### Task 3 — Tests + docs + exit

- **Scaffold-fidelity test.** Drives `aivyx tool init` over
  a `tempfile`-style dir, asserts the four-file output
  tree, then runs `cargo check --manifest-path <tmp>/
  Cargo.toml` against the generated project so a broken
  scaffold cannot ship green. (Phase 50's
  `p12_equivalence.rs` is the precedent for an in-tree
  `cargo`-driven integration test.)
- **CLI parse tests.** `aivyx tool init <path>` parses;
  `aivyx tool` alone reports a usable error; `--force` is
  recognised.
- **`docs/TOOL_SDK.md`** — a "Start a new Rust tool" note
  near the existing "First-party tools speak this protocol
  too" section pointing at `aivyx tool init`.
- **`docs/INSTALL.md`** — a one-paragraph mention beside
  the existing `aivyx init` documentation.
- Exit: ROADMAP frozen entry, docs/README status flip,
  prediction-vs-reality, hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Mechanism:** (a) **`aivyx tool init` scaffolding
  command.** A subcommand that writes a starter tool-process
  project to a target directory. Chosen over a templates
  registry (more flexibility at the cost of code, and one
  starter is enough this phase) and over a `validate`
  conformance checker (debug aid, not a generator — a
  candidate for a follow-on phase).
- **Q2 — Target:** (b) **Rust** (`cargo new` + `aivyx-tool`
  dep). Chosen over Python (already covered by
  `examples/python-tool/`) and over shipping both (one
  starter at a time keeps the scaffold-fidelity test
  matrix small). A future micro-phase can add `--lang
  python` if the demand surfaces.
- **Q3 — Scope:** (a) **Full runnable project.** The
  scaffold is the equivalent of `examples/python-tool/`:
  `Cargo.toml`, `src/main.rs`, `README.md`, conformance
  test, plus the `[[tool_process]]` snippet. Chosen over
  the handler-only stub: the point of scaffolding is that
  the author edits *one function body* and has a runnable
  starting point, not that they assemble the rest from
  docs.

## Deferrals

**Rolling deferrals carried into Phase 103** (Phase 103
closes none — it is net-new Chapter B ergonomics work):

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
- Phase 102 deferrals (`aivyx tools` in the Web UI;
  per-tool latency percentiles; repair-round stats;
  `tool_id` stability across restarts).

**Likely Phase 103 deferrals:**

- **Python `--lang` flag.** Scaffolded Python tool — the
  same project shape as the existing
  `examples/python-tool/`, generated on demand. A
  follow-on micro-phase if operator demand surfaces.
- **`aivyx tool validate <command>`.** The Q1
  validator/lint option, deferred. A natural sibling
  once the scaffold is in operator hands and the
  authoring-failure modes are visible.
- **Sandbox-config block in the scaffold.** The generated
  `[[tool_process]]` snippet is a minimal launch line; a
  follow-on could include a commented-out
  `[tool_process.sandbox]` block per Phase 52, so the
  author sees the option without having to consult
  `TOOL_SDK.md § 9`.
- **`aivyx-tool` on crates.io.** Once the Distribution
  milestone publishes the workspace, the scaffolded
  `Cargo.toml` can use `aivyx-tool = "0.1"` directly
  rather than the path/git placeholder this phase
  documents.

## Prediction vs. reality

*Filled at phase exit.*

## Exit criteria

- [x] `docs/PHASE_103.md` + ROADMAP entry flip + docs/README
  status row — Task 1 (this commit).
- [ ] `aivyx tool init <path>` subcommand + the
  `aivyx_modules/tool_init.rs` scaffold module + embedded
  Cargo.toml / main.rs / README.md / conformance test
  templates — Task 2.
- [ ] Scaffold-fidelity test (`cargo check` against the
  generated project) + CLI parse tests — Task 3.
- [ ] `docs/TOOL_SDK.md` "Start a new Rust tool" note +
  `docs/INSTALL.md` mention — Task 3.
- [ ] ROADMAP + docs/README refreshed at exit — Task 3.
- [ ] All three Q-block questions resolved with operator
  sign-off pre-Task 2.
- [ ] DESIGN.md streak extends to fifty (milestone).
- [ ] PRODUCT.md streak extends to three.
- [ ] Production-core `lib.rs` streak extends to three.
- [ ] Zero new workspace dependencies.
- [ ] Test count delta positive (predicted `+5`–`+10`).
- [ ] Zero clippy warnings.
- [ ] Prediction-vs-reality block filled.
