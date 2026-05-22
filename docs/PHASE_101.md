# Phase 101 — Tool-Call Input Validation & Repair (Chapter B)

The Phase 99 and Phase 100 `dev-verify` runs both caught the
same thing: a local model emitting a tool call whose
*arguments* were wrong — a missing required field, a value
of the wrong type — or skipping the call shape entirely.
The planner already handles one half of tool-call
unreliability: an **unknown tool name** gets a synthetic
`unknown_tool` error appended to history and the LLM is
looped to retry (`llm_planner.rs`). The other half is
unhandled. A *known* tool called with malformed input is
dispatched blind — the tool's own `execute` rejects it with
an ad-hoc, per-tool error string (`"input must have a
string \`path\` field"` and a dozen variants), and the LLM
gets a dead-end `failed` result with no machine-readable
hint about *what shape was expected*.

Phase 101 closes that half. The planner gains a
**validate-before-dispatch** step: a known tool's call
input is checked against that tool's `input_schema()` — the
JSON Schema every `Tool` already exposes — *before* the
call reaches `execute`. On a mismatch the planner appends a
structured `invalid_input` tool-result that echoes the
expected schema and the specific violation, then loops, so
the model **repairs** the call instead of burning the
dispatch on a doomed one. A two-repair cap bounds the loop:
after two `invalid_input` rounds the call is dispatched
as-is and the tool's own `execute` validation is the floor,
exactly as today.

## Why this, why now

- Two consecutive `dev-verify` runs surfaced tool-call
  unreliability as the live operator-feedback signal. It is
  the Chapter B "tool-calling reliability" item, named at
  the Phase 99 exit and now the most-pressed gap.
- The asymmetry is the bug. Unknown *tool name* → a clean
  structured retry. Unknown *argument shape* → a blind
  dispatch and a ragged per-tool failure. Phase 101 makes
  the two halves symmetric.
- The substrate is already there. Every `Tool` exposes
  `input_schema()` (the planner already serializes it into
  the model's tool descriptors). The planner already has
  the retry loop — `LlmStepEnd::ToolCalls` partitions
  known/unknown and `continue`s on an all-error batch.
  Phase 101 adds one validation step at that exact site.
- A repaired call is strictly better than a failed one. A
  `failed` result ends the call; an `invalid_input` result
  with the schema attached is something the model can act
  on in the very next step.
- The change is additive. With every call well-formed —
  the common case — the planner is byte-identical to
  pre-Phase-101 behavior; validation only does work when a
  call would have failed anyway.

## Streak predictions

- **DESIGN.md** — **Will hold.** Validate-before-dispatch
  is additive planner behavior; it changes no locked
  technical-contract decision. D8's workspace crate-count
  lock is untouched — `jsonschema` is an external
  dependency, not a new workspace crate. Hash at entry:
  `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Prediction: streak **extends to forty-eight**
  (currently 47).

- **PRODUCT.md** — **Will hold.** Reliability is a quality
  improvement on an existing surface; no product-shape
  decision changes, no commitment text moves. The
  PRODUCT.md streak reset at Phase 100 (Amendment A11), so
  this is the first phase of a fresh streak. Hash at
  entry:
  `9f0a515c9076544866aa955d6835763ee15beb0d009b4c335f5710b6d9ba61d3`.
  Prediction: streak **re-establishes to one**.

- **Production-core `aivyx-core/src/lib.rs`** — **Will
  hold.** The validation helper and the repair loop live
  in `llm_planner.rs` itself — kept in-file so `lib.rs`
  gains no `mod` line and no re-export; the helper is an
  internal planner concern, not a public-API type. The
  `lib.rs` streak reset at Phase 100 (the `fs.*` tool
  re-exports), so this is a fresh streak. Hash at entry:
  `ab3f9730c692917023239bbdd7c375497459e2a7fb3bbf08c007b5c945c6210d`.
  Prediction: streak **re-establishes to one**.

- **New workspace deps** — **One: `jsonschema`.** This is
  the deliberate Q2 resolution — full JSON Schema spec
  coverage over a hand-rolled ~50-line checker. It is the
  first net-new workspace dependency since Phase 27's
  `notify` crate, and the phase doc owns that honestly:
  the validator the tool schemas are checked against
  should be a correct, maintained implementation, not a
  partial in-house one that drifts from the spec the
  schemas are written to.

- **Test count** — Positive. The validation helper earns a
  boundary suite (valid input; each violation class —
  missing required, wrong type, unknown-but-tolerated
  extra); the planner integration earns repair-loop tests
  (one repair then success; two-repair cap then
  dispatch-as-is; well-formed call unchanged). Rough
  prediction: **+12 to +22**.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_101.md` + `docs/ROADMAP.md` entry flip to
`Active` + `docs/README.md` status row.

### Task 2 — `jsonschema` dependency + validation helper + repair loop

Tasks 2 and 3 ship in one commit: the helper has no
non-test caller until the planner integration, so splitting
them would leave a dead-code helper mid-phase.

- Add `jsonschema` to `[workspace.dependencies]` in the
  root `Cargo.toml` (with `default-features = false` — no
  `$ref`-resolution backends, no second TLS stack) and as
  a dependency of `aivyx-core`.
- A `validate_tool_input(schema, input) -> Result<(),
  String>` helper, private to `llm_planner.rs` (kept
  in-file so `lib.rs` gains no `mod` line): compiles the
  schema, validates the input, and on failure returns a
  human-readable digest of the first few violations.
- A malformed *schema* (should never happen — tool schemas
  are authored in-tree) is treated as "valid": the helper
  fails open so a future tool with a quirky schema cannot
  brick its own dispatch.
- Unit tests: well-formed input passes; missing required
  field fails with the field named; wrong-typed field
  fails; an extra unschema'd field is tolerated (tool
  schemas do not set `additionalProperties: false`);
  malformed schema fails open.

### Task 3 — Planner validate-before-dispatch + repair loop

`aivyx-core/src/llm_planner.rs`, in the
`LlmStepEnd::ToolCalls` arm:

- For each call whose tool name **is** known, fetch the
  tool via `ToolRegistry::get` and validate `call.input`
  against `tool.input_schema()`.
- On a validation failure, append an `LlmMessage::ToolResult`
  with `is_error: true` and content
  `{"error": "invalid_input", "message": "<summary>",
  "expected_schema": <schema>}` — the same shape family as
  the existing `unknown_tool` error — and do **not** batch
  the call.
- A `repair_rounds` counter, local to `next_step`'s loop,
  increments each time an all-error batch `continue`s for
  an invalid-input reason. Once it reaches `2`, the
  validation step is skipped: a known call dispatches
  as-is and the tool's own `execute` validation is the
  floor (the Q3 resolution).
- With every call well-formed, the arm is byte-identical
  to pre-Phase-101.
- Planner tests: well-formed call dispatches unchanged;
  one invalid round then a repaired call dispatches;
  two invalid rounds then the third dispatches as-is
  (cap); a mixed batch (one valid, one invalid) dispatches
  the valid call and errors the invalid one.

### Task 4 — docs + exit

- `docs/TOOL_SDK.md` — a note in the tool-authoring
  guidance that `input_schema()` is now load-bearing: the
  planner validates calls against it before dispatch, so a
  precise schema directly improves tool-call reliability.
- Exit: ROADMAP frozen entry, docs/README status flip,
  prediction-vs-reality, hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Mechanism:** (a) **Planner-level input validation
  + repair loop.** A known tool's call input is validated
  against its `input_schema()` before dispatch; a mismatch
  produces a structured `invalid_input` result that loops
  the model to repair the call. Chosen over a
  description-only prompt-tuning pass (no structural
  mechanism, untestable) and over post-dispatch retry of
  `Failed` calls (treats the symptom, not the malformed
  call).
- **Q2 — Validation depth:** (b) **Full JSON Schema via
  the `jsonschema` crate.** Chosen over a hand-rolled
  dep-free required-plus-type checker. The tool schemas
  are authored as JSON Schema; validating them against a
  correct, maintained implementation of that spec — rather
  than a partial in-house subset that drifts — is worth
  the one new dependency. Recorded as a deliberate
  break of the long zero-new-deps run.
- **Q3 — Repair budget:** (a) **Cap at two repairs, then
  dispatch as-is.** After two `invalid_input` rounds the
  call dispatches and the tool's own `execute` validation
  is the floor — graceful degradation to pre-Phase-101
  behavior. Chosen over hard-failing the turn (a stuck
  model loses the whole turn) and over unbounded repair (a
  stuck model burns the turn's step budget on one call).

## Deferrals

**Rolling deferrals carried into Phase 101** (Phase 101
closes none — it is net-new Chapter B reliability work):

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
  breadth — extended attributes, symlink-target
  resolution, inode/device ids).

**Likely Phase 101 deferrals:**

- **Compiled-schema cache.** Task 2 compiles each tool's
  schema on every validated call. Tool calls are not a hot
  loop, so v1 compiles per-call; a future phase could
  cache one compiled `Validator` per tool at registration.
- **`additionalProperties` tightening.** Tool schemas do
  not set `additionalProperties: false`, so an extra
  unschema'd field is tolerated. A future phase could
  tighten schemas + validation to reject unknown fields
  (catches a hallucinated argument early).
- **Skipped-call detection.** Phase 101 repairs malformed
  calls; it does not address a model that emits prose
  instead of a tool call at all. Nudging a non-calling
  model is a separate reliability lever (description
  sharpening, or a system-prompt cue) — a later Chapter B
  item.
- **Operator-visible repair stats.** The `invalid_input`
  repair events are not surfaced in `aivyx learning`. A
  count of repair rounds per session is a candidate for
  the Chapter B observability phase.

## Prediction vs. reality

**Predictions held — all three streak calls correct.**

- **DESIGN.md — held.** Validate-before-dispatch is
  additive planner behavior; no locked technical-contract
  decision changed, and `jsonschema` is an external crate,
  not a workspace crate, so D8 held. `DESIGN.md` is
  byte-identical at exit (hash still `89dc8903…a94bce`).
  Streak: **48 consecutive phases** (was 47).
- **PRODUCT.md — held.** Reliability is a quality
  improvement on an existing surface; no commitment text
  moved. Byte-identical at exit (hash still
  `9f0a515c…ba61d3`). Streak **re-establishes to one** (it
  reset at Phase 100's Amendment A11).
- **`aivyx-core/src/lib.rs` — held.** The helper and the
  repair loop live inside `llm_planner.rs`; no new module
  file, so `lib.rs` gained no `mod` line. Byte-identical
  at exit (hash still `ab3f9730…c6210d`). Streak
  **re-establishes to one** (it reset at Phase 100's
  `fs.*` re-exports).

**New workspace deps — one, `jsonschema` 0.46, as
predicted.** `default-features = false` kept its
`$ref`-resolution backends and second TLS stack out of the
tree; it still pulls a transitive set (`num`, `fraction`,
`fancy-regex`, `referencing`, `regex-automata`) that is the
real cost of spec-correct validation — the deliberate Q2
trade.

**Test count — `+9`** (workspace `1778 → 1787`), **under
the predicted `+12–22` band by three.** The estimate was
high: the helper boundary suite landed at five tests and
the planner repair-loop suite at four, more compact than
the forecast. Coverage is complete for the surface shipped
(every violation class; well-formed, repair-then-dispatch,
two-repair cap, mixed batch) — the miss is a calibration
error in the prediction, not a coverage gap.

**Scope — shipped as planned; Tasks 2 and 3 merged into one
commit.** The `validate_tool_input` helper has no non-test
caller until the planner integration, so committing it
alone would have left a dead-code helper mid-phase; the doc
was updated at Task 2 to record the merge. The helper lives
in `llm_planner.rs` rather than a new module file — the
in-file placement is what keeps `lib.rs` byte-identical.

**`invalid_input` repair result.** A known tool called with
schema-violating input now yields `{"error":
"invalid_input", "message": "<digest>", "expected_schema":
<schema>}` — the same shape family as the pre-existing
`unknown_tool` error — and the model is looped to repair
the call. `repair_rounds` caps the loop at two; the third
round dispatches as-is, the tool's own `execute` validation
the floor. A well-formed call leaves the planner
byte-identical to pre-Phase-101.

**`docs/TOOL_SDK.md`.** The tool-process contract already
*stated* that input is validated against `input_schema()`
before a tool sees it; Phase 101 makes that planner-
enforced for every registered tool (first-party and
tool-process alike). The doc note was enhanced — not newly
added — to describe the `invalid_input` repair behavior and
why a precise schema improves reliability.

## Exit criteria

- [x] `docs/PHASE_101.md` + ROADMAP entry flip + docs/README
  status row — Task 1 (commit `78ddf26`).
- [x] `jsonschema` workspace dependency + the
  `validate_tool_input` helper with its boundary suite —
  Tasks 2–3 (commit `2b4976f`).
- [x] Planner validate-before-dispatch + `invalid_input`
  repair result + two-repair cap — Tasks 2–3 (commit
  `2b4976f`).
- [x] Planner repair-loop tests (repair-then-dispatch,
  two-repair cap, well-formed unchanged, mixed batch) —
  Tasks 2–3 (commit `2b4976f`).
- [x] `docs/TOOL_SDK.md` schema-is-load-bearing note —
  Task 4 (this commit).
- [x] ROADMAP + docs/README refreshed at exit — this
  commit.
- [x] All three Q-block questions resolved with operator
  sign-off pre-Task 2.
- [x] DESIGN.md streak extends to forty-eight.
- [x] PRODUCT.md streak re-establishes to one.
- [x] Production-core `lib.rs` streak re-establishes to
  one.
- [x] Exactly one new workspace dependency (`jsonschema`),
  as predicted.
- [x] Test count delta positive — `+9` (workspace
  `1778 → 1787`), under the predicted `+12`–`+22` band by
  three (a prediction calibration miss, not a coverage
  gap).
- [x] Zero clippy warnings.
- [x] Prediction-vs-reality block filled.
