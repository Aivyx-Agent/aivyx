# Phase 86 — Conversational-Window Relevance (sharpening the whole stack's input)

For 85 phases the assistant has judged relevance off **one
line**. Phase 76 auto-recall embeds only the latest user
message; Phase 79 adaptive Persona selection embeds only the
latest user message. In a real multi-turn conversation the
topic drifts, the operator's intent spans several turns, and a
single line is a lossy proxy — so recall pulls the wrong
memories and the Soul selects the wrong facets in exactly the
usage that matters most. Both phases deferred this; it is the
single foundational input-quality gap under the entire
self-learning stack.

Phase 86 gives both consumers a **recent conversational
window**: a small, recency-ordered slice of the last few turns
(user + assistant), concatenated into the one embedding the
relevance ranking already makes. The decisive constraint —
established by investigation — is that the planner is fresh
per turn with no history and no per-session buffer exists, so
this phase adds a **new in-memory, session-keyed recent-turns
buffer**, written by the daemon turn loop and read by the
providers. It ships **opt-in and byte-identical by default**
(window = 1 = exactly today's single-message behaviour) so no
existing operator's recalled context changes on upgrade.

## Why this, why now

- It is the twice-deferred (Phase 76 *and* Phase 79)
  foundational gap, and every later phase (77 feedback, 82/83
  ledgers, 84/85 consumption) is built on a relevance signal
  that is currently a one-liner. Sharpening the input sharpens
  the whole stack.
- The seam is proven and streak-safe: both hooks live in
  `llm_planner.rs` (not `lib.rs`); `ContextProvider::recall`
  already carries `session_id`, and adding it to
  `SystemPromptRefiner::refine` is an `llm_planner.rs`-only
  change (the planner already holds `message.session_id` at
  the call site).
- Reuse is high: the window is concatenated into the
  *existing* single `embed` call (the Phase 76/79 single-
  vector model is unchanged), gated by an existing-section
  config knob, and the shared-handle threading mirrors the
  Phase 82/84 ledger precedent. No new `KeyDomain` (the
  window is ephemeral), no LLM, no `AuditTag`.

## Streak predictions

- **DESIGN.md** — **Will hold.** Widening the relevance query
  inside the existing recall/Persona hooks touches no locked
  technical-contract decision. Prediction: streak **extends to
  thirty-three** consecutive phases (currently 32).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** Better relevance input is a
  quality deepening of already-delivered G3 recall / P14
  adaptive Persona; no new product commitment, none weakened,
  and the operator-facing contract is *strengthened*
  (multi-turn intent is no longer lost). No commitment-text
  edit. Prediction: streak **extends to twenty-six**
  consecutive phases.
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold,
  by design.** The two hook traits live in `llm_planner.rs`;
  adding a `session_id` parameter to `SystemPromptRefiner` and
  threading a shared window handle are `llm_planner.rs` /
  `aivyx-channel` changes. The new buffer is in-memory session
  state in `aivyx-channel`. No new `AuditTag`, no `aivyx-core`
  type change (the Phase 76–85 streak lesson, continued).
  Prediction: streak **extends to thirty-four** consecutive
  phases (new project record, beats Phase 85's 33).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

- **New workspace deps** — Zero. Reuses the planner seam, the
  Phase 75 embedding provider, and the `[embedding]` config.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_86.md` + `docs/README.md` status row.

### Task 2 — `[embedding].recall_window_turns` knob

`aivyx-config` (extend the existing `EmbeddingConfig` — the
Phase 84/85 field-addition precedent):

- `EmbeddingConfig` gains `recall_window_turns: usize`
  alongside `rag_top_k` / `rag_min_similarity`.
  `RawEmbedding` + the embedding builder: default
  `DEFAULT_RECALL_WINDOW_TURNS = 1` (= exactly the latest
  message = byte-identical to pre-Phase-86); validation when
  `[embedding]` is present: `recall_window_turns >= 1`.
  Tests: default, explicit, `0`-is-invalid.

### Task 3 — The session window buffer + seam plumbing (the crux)

`aivyx-channel` + `aivyx-core/src/llm_planner.rs`:

- New `conversation_window` module: `ConversationWindow` — a
  bounded recency ring of `(Role, String)` turns with a hard
  turn cap **and** a hard total-char budget;
  `push(role, text)` (evicts oldest past the cap),
  `assemble(window_turns, current) -> String` (the last
  `window_turns-1` prior turns, oldest→newest, then the
  current message **last so it dominates**, char-capped).
  `SharedConversationWindows` = `Arc<…>` keyed by
  `SessionId`; `shared_conversation_windows()` ctor;
  `record_turn(session_id, user_text, assistant_text)`.
- The daemon turn loop writes each completed turn's user
  message + assistant final response into the session's
  window (best-effort; a write failure never affects the
  turn).
- `SystemPromptRefiner::refine` gains a `session_id`
  parameter (`llm_planner.rs` trait + the `begin_turn` call
  site already has `message.session_id` + `FakeRefiner`).
  Both providers gain an `Option<Arc<SharedConversationWindows>>`
  via a builder (daemon-startup injection, the Phase 82/84
  shared-handle precedent) + `DaemonConfig` + `bin/aivyx`
  wiring. Pure-module unit tests (cap, char budget, eviction,
  assembly order/recency, empty).

### Task 4 — Provider consumption (the payoff)

`aivyx-channel`:

- `SemanticMemoryContext::recall` and
  `PersonaContextRefiner::refine`: when
  `recall_window_turns > 1` and a window handle + a non-empty
  per-session window are present, build the embed query from
  `ConversationWindow::assemble(recall_window_turns,
  current_message)` instead of the bare message; otherwise
  (window = 1, no handle, empty buffer, unknown session) use
  the bare current message — **byte-identical to
  pre-Phase-86**. The existing `rag_min_similarity` /
  Persona-selection floors are unchanged (the safety net
  against a stale window dragging in noise). Integration
  tests: a windowed query recalls the in-window topic the
  bare last line would miss; `recall_window_turns = 1` is
  exactly pre-Phase-86; no handle / empty buffer → graceful
  single-message; symmetric assertions for Persona selection.

### Task 5 — Tests + docs + exit

- Tests: config, the window-buffer units, both provider
  integrations (windowed vs bare vs graceful), the
  `refine(session_id)` ripple.
- Docs: `docs/INSTALL.md` "Conversational-window relevance
  (Phase 86)" (what the window is, the opt-in
  `recall_window_turns` knob, byte-identical default, applies
  to recall + Persona selection, the similarity-floor safety
  net); `examples/aivyx.toml` document the new `[embedding]`
  knob.
- Exit: ROADMAP + PRODUCT_ROADMAP frozen entries, docs/README
  status flip, prediction-vs-reality, hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Seam:** (a) Daemon-scoped, session-keyed shared
  in-memory buffer; providers look it up by `session_id`
  (already on `ContextProvider::recall`; added to
  `SystemPromptRefiner::refine`, `llm_planner.rs`-only). The
  least-invasive streak-safe path; ephemeral (no new
  `KeyDomain`).
- **Q2 — Composition:** (a) Concatenate the last N turns
  (user + assistant, recency-ordered, char-capped) into the
  *single* existing embed call, current message dominant. The
  Phase 76/79 single-vector model is unchanged.
- **Q3 — Scope:** (a) Both auto-recall (76) and adaptive
  Persona selection (79) — same seam, same limitation, the
  deferral was from both; doing one is incoherent.
- **Q4 — Default:** (a) `[embedding].recall_window_turns`,
  default `1` = byte-identical to pre-Phase-86; windowing
  engages only when the operator raises it (the project's
  behaviour-change-is-opt-in discipline; recall context
  feeds model output).

## Deferrals

**Rolling deferrals carried into Phase 86:**

- v0.1.0 publication (Phase 61 Task 7).
- System-prompt notification-target enumeration (Phase 62).
- `turn_id` correlation refactor (Phase 67).
- Phase 68 deferrals (XOAUTH2, HTML email, attachments,
  multi-account `[email]`).
- Phase 69 deferrals (WebPush / service-worker, notification
  urgency, icons, sound).
- Phase 70 deferrals (proposal supersession, multi-window
  reflection).
- Phase 71 deferrals (per-fire role override, operator-
  customizable reflection prompt, reflection cadence
  learning).
- Phase 73 deferrals (persisted rate-limit buckets,
  operator-configurable retry-on list).
- Phase 74 deferrals (fuzzy match, edit-content Web UI,
  per-topic eviction-strategy override).
- Phase 75 deferrals (ANN index, `aivyx memory reembed`,
  hybrid keyword+semantic fusion, query-embedding cache).
- Phase 76 deferrals (heuristic recall gate, token-budget
  context sizing).
- Phase 77 deferrals (`[recall_feedback]` tuning knob,
  LLM-judged recall usefulness).
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
- Phase 82 deferrals (operator-tunable half-life/retention,
  topic canonicalization).
- Phase 83 deferrals (sequential/temporal patterns, n-ary
  clusters, operator-tunable top-K/half-life).
- Phase 84 deferrals (affinity re-ranking of existing
  candidates, operator-tunable affinity policy).
- Phase 85 deferrals (helpfulness-driven *consolidation*,
  reflection-facet decay via fuzzy embedding,
  pattern-driven Persona proposals).

**Likely Phase 86 deferrals:**

- **Token-budget context sizing.** v1 caps the window by
  turns + a char budget; a true tokenizer-aware budget
  (Phase 76's own deferral) still defers.
- **Heuristic recall gate.** Deciding *whether* to recall at
  all from the window (vs always) remains Phase 76's deferral.
- **Embed-each-and-pool windows.** v1 concatenates into one
  embed; per-message vectors + pooling defers.
- **Persisted windows.** The buffer is ephemeral per daemon
  lifetime; a restart starts fresh. Durable per-session
  windows defer.

## Prediction vs. reality

To be filled in at phase exit.

## Exit criteria

- [ ] `[embedding].recall_window_turns` (default 1,
  validation `>= 1`) — Task 2.
- [ ] `conversation_window` module: bounded ring (turn cap +
  char budget), recency assembly with current message
  dominant; `SharedConversationWindows` session-keyed —
  Task 3.
- [ ] Daemon turn loop records each completed turn
  (user + assistant) best-effort — Task 3.
- [ ] `SystemPromptRefiner::refine` gains `session_id`
  (`llm_planner.rs` + `begin_turn` + `FakeRefiner`); shared
  handle threaded to both providers — Task 3.
- [ ] Both `recall` and `refine` use the assembled window
  when `recall_window_turns > 1` + handle + non-empty buffer;
  else bare current message = byte-identical to pre-Phase-86
  — Task 4.
- [ ] Tests across config, window-buffer units, both provider
  integrations (windowed / bare / graceful) — Task 5.
- [ ] `docs/INSTALL.md` + `examples/aivyx.toml` updated —
  Task 5.
- [ ] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 5.
- [ ] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [ ] DESIGN.md streak extends to thirty-three.
- [ ] PRODUCT.md streak extends to twenty-six.
- [ ] Production-core streak extends to thirty-four (new
  record) — `lib.rs` byte-identical.
- [ ] Test count delta: positive (~+9-13; per the converged
  calibration law — one new pure module (the window ring,
  ≈ +6-8 unit tests) + a config knob on an existing section
  (≈ +1) + provider/seam integration (≈ +2-4); no new
  detector module, no new `KeyDomain`).
- [ ] Zero clippy warnings.
- [ ] Zero new workspace deps.
- [ ] Prediction-vs-reality block filled.
