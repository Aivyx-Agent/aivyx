# Phase 99 — Local Testing Setup (operator-requested; local-build posture)

Through Phase 98 the project shipped 99 phases of substrate
with the test pyramid resting entirely on `cargo test` —
1746 workspace unit and integration tests, all running
against in-process fakes and scripted transports. What it
never had was an **operator loop**: a one-command way to
build the `aivyx` binary and drive the *real* agent stack
against a *real* LLM backend on the developer's own
machine. Every "does it actually run" check was an ad-hoc
`cargo run --release --bin aivyx` with hand-set environment
variables and whatever store happened to be lying around.

Phase 99 closes that gap. It is the operator-feedback
infrastructure phase the post-Chapter-A posture has been
pointing at: *"the project benefits more from real-world
operator use than from speculative forward work."* You
cannot get real-world operator use without a real-world
run, and you cannot get a repeatable real-world run
without a launcher.

The phase ships **shell tooling, not Rust** — a local dev
launcher and a scripted verification pass under `scripts/`,
both targeting a fully local Ollama backend so the loop
has no API key, no network egress, and no per-run cost.
All state is pinned under a gitignored `.dev-run/`
directory: a sandbox FS root, an encrypted dev store, and
a throwaway dev passphrase. Nothing in the phase touches
the workspace crates.

The phase opens under an explicit **local-build posture**:
builds stay local while repo infrastructure (CI, remote
runners, public hosting) is still being decided. No CI
wiring, no remote-build config, no publication step is in
scope — those wait on the Distribution milestone and the
repo-infrastructure decision that gates it.

## Why this, why now

- The project has 99 phases of substrate and zero
  operator-facing run tooling. The single largest gap
  between "the code is correct" and "the agent works" is
  that nobody has driven the real binary against a real
  model in a repeatable way.
- The post-Chapter-A posture (ROADMAP "Other forward
  work") explicitly names operator feedback loops as
  more valuable than speculative forward work. Phase 99
  *is* that loop being built.
- Ollama makes a fully local loop free. The
  `ProviderKind::Ollama` path (Phase 34) already exists,
  the config loader already skips the API-key
  requirement for it (`aivyx-config/src/lib.rs:4083`),
  and `examples/aivyx-ollama.toml` already documents the
  shape. Phase 99 only needs to wire the launcher.
- The change is **pure tooling**. It adds files under
  `scripts/` and one `.gitignore` line. No crate is
  touched; no workspace test is added or changed; no
  dependency moves.
- It is the prerequisite for the next phase. The
  Channel Activation Milestone is "operator verification
  across all channels" — that verification needs a
  launcher and a verification harness to run against.
  Phase 99 builds the harness; the milestone consumes
  it.

## Streak predictions

- **DESIGN.md** — **Will hold, trivially.** Phase 99
  ships shell scripts and a `.gitignore` line. It touches
  no `.rs` file and no locked technical-contract
  decision. Hash at entry:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Prediction: streak **extends to forty-six**
  (currently 45).

- **PRODUCT.md** — **Will hold, trivially.** No product-
  shape decision changes. The launcher is developer
  tooling below the product surface; it ships no
  operator-facing feature and revises no commitment.
  Hash at entry:
  `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.
  Prediction: streak **extends to thirty-nine**
  (currently 38).

- **Production-core `aivyx-core/src/lib.rs`** — **Will
  hold, by design.** Phase 99 adds no Rust whatsoever.
  Hash at entry:
  `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.
  Prediction: streak **extends to forty-seven**
  consecutive phases (new project record, beats Phase
  98's 46).

- **New workspace deps** — Zero. The scripts use only
  POSIX tooling plus `cargo`, `curl`, and `ollama`,
  none of which are Cargo dependencies.

- **Test count** — **Zero delta, by design.** Phase 99
  adds no `cargo test` tests. Its verification artifact
  is a *shell-level operator harness* (`dev-verify.sh`)
  that drives the real binary against a real Ollama
  instance — categorically a different layer from the
  workspace unit/integration suite, and not countable
  in the `cargo test` total. This is the honest shape
  of a tooling phase; Phase 54 (documentation sweep)
  set the precedent for a zero-code, zero-test-delta
  phase.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_99.md` + `docs/ROADMAP.md` entry + `docs/README.md`
status row.

### Task 2 — `scripts/dev-run.sh` local dev launcher

A one-command launcher for an interactive local session:

- Preflights the Ollama server — reachable at the
  configured base URL, and the requested model pulled —
  failing fast with the exact `ollama serve` /
  `ollama pull` command when not.
- Builds `aivyx` locally (`cargo build --bin aivyx`,
  `--release` optional).
- Execs the binary with `AIVYX_PROVIDER=ollama` and
  every path pinned under a gitignored `.dev-run/`:
  sandbox FS root, encrypted store, throwaway dev
  passphrase (stable across runs so the encrypted store
  reopens).
- Flags: `--model`, `--ollama-url`, `--release`,
  `--reset`; everything after `--` forwards verbatim to
  the binary.
- `.gitignore` gains `/.dev-run/`.

### Task 3 — `scripts/dev-verify.sh` scripted verification pass

A non-interactive battery that exercises the subsystems
the interactive launcher cannot smoke-test by hand:

- Shares the Task 2 preflight + build + `.dev-run/`
  env wiring.
- Runs a sequence of checks against the real binary and
  a real Ollama backend, each pass/fail with a clear
  line:
  - `--version` and `--print-role default` (no-session
    introspection paths).
  - `--verify-only` on a fresh store (empty-chain
    baseline).
  - A scripted stdin session driving a `memory.write` →
    `memory.read` round trip, then `--verify-only`
    again to confirm the audit chain grew past the
    baseline — the tool path the Phase 99 chat-only
    test never exercised.
  - `fs.read` / `fs.write` against the sandbox root.
  - Daemon mode: `daemon run` backgrounded, `daemon
    status`, `daemon stop`.
- Exits non-zero if any check fails; prints a summary
  count.
- `dev-run.sh --verify` delegates to this script so the
  verification pass is reachable from the same entry
  point.

### Task 4 — docs + exit

- `docs/INSTALL.md` — new "Running Aivyx locally
  (Phase 99)" section documenting the `dev-run.sh` /
  `dev-verify.sh` loop, the Ollama prerequisite, and the
  disposable `.dev-run/` state directory.
- Exit: ROADMAP frozen entry, docs/README status flip,
  prediction-vs-reality, hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Deliverable:** a **dev launcher script**.
  Operator-selected over a committed dev config, an
  automated `cargo`-level e2e harness, or a runbook
  doc. The launcher is the smallest thing that turns
  "runnable in principle" into "runnable in one
  command."
- **Q2 — Backend:** **Ollama, fully local.** No API
  key, no network, no cost. The Anthropic provider is
  explicitly out of scope for the Phase 99 loop —
  consistent with the local-build posture.
- **Q3 — Phase 99 scope after the launcher:** **extend
  with a scripted verification pass.** The interactive
  launcher proves chat turns work; a scripted pass is
  needed to cover the tool, trigger, and daemon paths a
  human cannot reliably smoke-test by hand. Kept inside
  Phase 99 (Task 3) rather than spun into a new phase.
- **Q4 — Phase ceremony:** **scaffold the formal phase
  docs now.** `PHASE_99.md` + ROADMAP entry in the
  established format, opened mid-phase (Task 2 already
  landed before the doc — recorded honestly in the
  task list).

## Deferrals

**Rolling deferrals carried into Phase 99** (Phase 99
closes no deferral — it is net-new operator-feedback
infrastructure):

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

**Likely Phase 99 deferrals:**

- **Anthropic-backend dev mode.** The launcher targets
  Ollama only. A future micro-phase could add an
  `--anthropic` mode for operators who want to verify
  against the cloud provider — gated on the operator
  wanting it, since it reintroduces an API key and
  per-run cost.
- **CI / remote-build wiring.** Held by the local-build
  posture pending the repo-infrastructure decision.
  Belongs to the Distribution milestone, not Phase 99.
- **Committed dev config / role templates.** The
  launcher configures everything via environment
  variables; roles fall through to built-in defaults.
  A `.dev-run/aivyx.toml` with named dev roles could be
  added if the verification pass needs role-switching
  coverage.
- **Telegram / web-UI channel verification.** The
  Task 3 pass covers local + daemon. Real-protocol
  channel verification stays with the Channel
  Activation Milestone (it needs BotFather credentials
  and a real network).
- **`cargo`-level e2e harness.** The verification pass
  is a shell harness against the real binary. Folding
  any of it into the workspace `cargo test` suite (so
  it gates a future CI) is deferred to whenever CI
  lands.
- **`--config` flag vs. stale example comment.**
  `examples/aivyx-ollama.toml` documents an `aivyx
  --config <path>` invocation, but the binary parses no
  `--config` flag — the TOML path is hard-coded to
  `./aivyx.toml` relative to CWD. Either add the flag or
  fix the example comment; a small, self-contained
  follow-up.

## Prediction vs. reality

**Predictions held — all three streaks correct.**

- **DESIGN.md — held.** Phase 99 shipped shell tooling
  and one `.gitignore` line; no `.rs` file and no
  locked technical-contract decision was touched.
  `DESIGN.md` is byte-identical at exit (hash still
  `89dc8903…a94bce`). Streak: **46 consecutive phases**
  (was 45).
- **PRODUCT.md — held.** No product-shape decision
  changed; the launcher is developer tooling below the
  product surface. Byte-identical at exit (hash still
  `cd60c4f9…1088e`). Streak: **39 consecutive phases**
  (was 38).
- **`aivyx-core/src/lib.rs` — held, by design.** Phase
  99 added no Rust whatsoever. Byte-identical at exit
  (hash still `69fb9af1…0c844`). Streak: **47
  consecutive phases** — new project record, beating
  Phase 98's 46.

**Test count — zero delta, as predicted.** The
workspace stays at 1746. Phase 99 added no `cargo test`
tests: its verification artifact, `dev-verify.sh`, is a
shell-level operator harness that drives the real
binary against a real Ollama instance — a different
layer from the workspace unit/integration suite. Phase
54 set the precedent for a zero-test-delta phase.

**Scope — every planned surface shipped.** The
`dev-run.sh` interactive launcher (Ollama preflight,
local build, `.dev-run/`-pinned state); the
`dev-verify.sh` scripted pass (deterministic substrate
checks + soft LLM-dependent tool probes); the
`dev-run.sh --verify` delegation; the `docs/INSTALL.md`
"Running Aivyx locally" section. Zero clippy warnings
(no Rust touched); zero new workspace deps.

**Verification-run result.** `dev-verify.sh` against
`qwen3.6:27b` reported **11 PASS, 1 WARN, 0 FAIL**. The
single WARN was the `fs.write` probe — the local model
skipped that tool call (it called `memory.write`
successfully the same run), exactly the local-model
non-determinism the soft-WARN class exists to absorb.
One real defect was found and fixed mid-Task-3: `aivyx
memory list` is a daemon client, so the harness was
reordered to start the daemon before querying it.

**Implementation note — the `--config` flag.**
`examples/aivyx-ollama.toml` documents an `aivyx
--config <path>` invocation that the binary does not
actually parse — the TOML path is hard-coded to
`./aivyx.toml` relative to CWD. Phase 99 worked around
it by exporting `AIVYX_PROVIDER` / `AIVYX_MODEL` /
`AIVYX_OPENAI_BASE_URL` and pinning CWD to `.dev-run/`.
Reconciling the stale example comment with the binary
is logged below as a deferral.

## Exit criteria

- [x] `docs/PHASE_99.md` + ROADMAP entry + docs/README
  status row — Task 1 (commit `933a13d`).
- [x] `scripts/dev-run.sh` local dev launcher against
  the Ollama backend; `.gitignore` covers `.dev-run/` —
  Task 2 (commit `07942f6`).
- [x] `scripts/dev-verify.sh` scripted verification
  pass (audit chain, daemon lifecycle, memory/fs tool
  probes); `dev-run.sh --verify` delegates — Task 3
  (commit `e3019d5`).
- [x] `docs/INSTALL.md` "Running Aivyx locally" section
  — Task 4 (commit `6340d7e`).
- [x] ROADMAP + docs/README refreshed at exit — this
  commit.
- [x] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [x] DESIGN.md streak extends to forty-six.
- [x] PRODUCT.md streak extends to thirty-nine.
- [x] Production-core streak extends to forty-seven
  (new record) — `lib.rs` byte-identical.
- [x] Test count delta: zero, by design (the
  verification artifact is a shell harness, not a
  `cargo test` test).
- [x] Zero clippy warnings (no Rust touched).
- [x] Zero new workspace deps.
- [x] Prediction-vs-reality block filled.
