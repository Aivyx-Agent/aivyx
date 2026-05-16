# Phase 79 — Adaptive Persona (contextual "Soul" selection)

Today the entire accreted Persona — every `learned_context`,
`character_traits`, `communication_adaptations`, … entry the
reflection loop has ever written — is dumped verbatim into
**every** system prompt by `assemble_session_prompt`,
unbounded and identical regardless of what the turn is about.
As the Soul matures over months that section grows without
limit, dilutes its own signal, and never *responds* to
anything. Phase 79 makes the Soul **adaptive**: each turn it
surfaces the Persona facets relevant to that turn (semantic
selection over the proven Phase 75/76 embedding substrate),
while a hard invariant keeps core identity and guardrails
always present. The Soul stops being a static wall of text and
starts behaving like the lived character the P14 vision
describes — and it stays bounded as it ages.

## Why this, why now

- The self-learning loop is closed (77) and legible (78).
  Making the *output* of that learning — the Persona — adaptive
  is the natural deepening, and Phase 78's trust surface is the
  precondition that lets an adaptive Soul stay legible.
- The seam is the **same one Phase 76 already solved**: per-turn
  behaviour that needs the user message, where the system
  prompt is otherwise fixed before the planner sees it. We
  reuse that exact `begin_turn` hook pattern rather than
  inventing machinery.
- It directly fixes a real, growing defect: unbounded prompt
  bloat from an ever-accreting Persona.

## Streak predictions

- **DESIGN.md** — **Will hold.** A per-turn system-prompt
  refiner hook is the same class of planner extension point as
  Phase 76's `ContextProvider` / `PruneSink`; no locked
  technical-contract decision is touched. Prediction: streak
  **extends to twenty-six** consecutive phases (currently 25).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** Adaptive Persona selection
  refines *how* the already-delivered P14 Persona is applied;
  it adds no new product commitment and removes none. The
  core-identity invariant means the operator-visible contract
  ("your declared identity and constraints always apply") is
  strengthened, not changed. No commitment-text edit.
  Prediction: streak **extends to nineteen** consecutive
  phases.
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold,
  by design.** The `SystemPromptRefiner` trait + its
  `LlmPlannerConfig` builder + the `begin_turn` invocation all
  live in `crates/aivyx-core/src/llm_planner.rs` (the exact
  Phase 76 `ContextProvider` precedent — reachable via
  `aivyx_core::llm_planner::` and **not** re-exported from
  `lib.rs`). The concrete refiner, the semantic selection, the
  reduced-Persona assembly, and the observability all live in
  `aivyx-channel`, reusing `assemble_session_prompt`,
  `EmbeddingProvider`, the Phase 75 cosine helper, and the
  Phase 78 surface. Nothing needs an `aivyx-core` type change.
  Prediction: streak **extends to twenty-seven** consecutive
  phases (new project record, beats Phase 78's 26).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

- **New workspace deps** — Zero. Reuses the Phase 75/76
  embedding + cosine substrate and the Phase 78 surface.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_79.md` + `docs/README.md` status row.

### Task 2 — `SystemPromptRefiner` planner hook

`aivyx-core` (`src/llm_planner.rs` only — **no `lib.rs`
edit**), mirroring the Phase 76 `ContextProvider`:

- `pub trait SystemPromptRefiner: Send + Sync { async fn
  refine(&self, user_message: &str) -> Option<String>; }` —
  returns a replacement system prompt for this turn, or `None`
  to keep the base unchanged (the universal byte-identical
  fallback path).
- `LlmPlannerConfig::with_system_prompt_refiner(Arc<dyn …>)`,
  mirroring `with_context_provider`.
- `LlmPlanner::begin_turn` calls `refine(latest_user_message)`;
  a `Some(p)` replaces `self.config.system_prompt` **for this
  turn only**. `None` / no refiner → today's behaviour exactly.
- Unit tests with a fake refiner: prompt swapped on `Some`,
  untouched on `None` / no refiner / blank message.

### Task 3 — Reduced-Persona assembly + the core invariant

`aivyx-channel` (extends `profile_prompt`), pure + tested:

- A function that, given the full `EffectivePersona` and a
  selected subset of list-facet entries, builds a reduced
  `EffectivePersona` and re-uses the **unchanged**
  `assemble_session_prompt` to render it.
- **Hard invariant (Q2a):** scalar identity (`assistant_name`,
  `operator_profile`, `communication_style`) and
  `behavioral_constraints` are copied through **in full**,
  never subject to selection. Only the soft list categories
  (`learned_context`, `character_traits`,
  `communication_adaptations`, `relationship_milestones`,
  `primary_use_cases`, `behavioral_preferences`) are reducible.
- Tests assert the invariant holds for every reduction,
  including the empty-selection case.

### Task 4 — `PersonaContextRefiner` (the concrete refiner)

`aivyx-channel` (sibling of `memory_recall`):

- Implements `SystemPromptRefiner`. Holds `Profile`,
  `SharedEffectivePersona`, role name + role prompt,
  `Arc<dyn EmbeddingProvider>`, and thresholds.
- `refine(user_message)`:
  - **Fallback (Q3a):** if no embedding provider **or** the
    Persona's reducible-facet count is below a size threshold
    → `None` (planner keeps its base prompt, which is the
    unchanged full `assemble_session_prompt` — byte-identical
    to pre-Phase-79). The feature is invisible until the Soul
    is actually large.
  - Otherwise: embed the message, cosine-rank every reducible
    list entry, keep the top-K above a relevance floor (the
    Phase 76 pattern + cosine helper), build the reduced
    `EffectivePersona` (Task 3, core invariant enforced),
    re-assemble, return `Some`.
- Unit tests over `InMemoryMemory`-free fakes: no-embedding
  fallback, small-Persona fallback, ranking selects relevant
  facets, core/constraints always present, empty Persona.

### Task 5 — Wire into the planner factories

`aivyx-channel` binary: when `[embedding]` is configured,
construct a `PersonaContextRefiner` and attach via
`with_system_prompt_refiner` at all three factory sites
(local-CLI, daemon, child-agent — sub-agents get the adaptive
Soul too). `None` provider → not attached → identical to
today. Selection thresholds are fixed constants with sane
defaults (a `[persona]` tuning block is a documented
deferral).

### Task 6 — Observability (Q4a, Phase 78-consistent)

- A per-turn stderr breadcrumb: `aivyx persona: injected N/M
  facets`, emitted by the refiner only when selection actually
  ran (not on the fallback path).
- Extend the Phase 78 `GetLearningInsights` / `LearningInsights`
  with an optional last-selection summary (selected vs total,
  per-category), surfaced in the `aivyx learning` CLI render
  and the Web UI Learning pane. Reuses the just-built trust
  surface; HTML smoke updated.

### Task 7 — Tests + docs + exit

- Tests: refiner hook (core), reduced-Persona invariant,
  selection ranking + both fallbacks + empty, observability
  summary, IPC round-trip delta, CLI/Web UI render.
- Docs: `docs/INSTALL.md` "Adaptive Persona (Phase 79)"
  (what changes, the always-on core invariant, the
  feature-only-when-large guarantee, where to see it);
  `examples/aivyx.toml` pointer.
- Exit: ROADMAP + PRODUCT_ROADMAP frozen entries, docs/README
  status flip, prediction-vs-reality, hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Depth lever:** (a) Contextual Persona selection —
  per-turn semantic selection of Persona facets, reusing the
  Phase 76 embedding substrate. Adaptive *and* bounds prompt
  bloat; safest because it reuses de-risked machinery rather
  than abstracting a working layer.
- **Q2 — Selection + invariant:** (a) Semantic cosine ranking
  with a **hard always-inject invariant**: scalar identity +
  `behavioral_constraints` are always present in full, never
  selected away. This invariant is the phase's core safety
  property.
- **Q3 — Fallback:** (a) No `[embedding]` **or** Persona below
  a size threshold → full-injection, byte-identical to
  pre-Phase-79. The feature engages only once the Soul is big
  enough to need bounding — never a regression, never an
  error, never embedding-contingent for small Personas.
- **Q4 — Observability:** (a) Extend the Phase 78 learning
  surface (selected/total + per-category) plus a per-turn
  stderr breadcrumb. An adaptive Soul that silently picks
  which identity to apply must stay legible — the Phase 78
  posture, continued.

## Deferrals

**Rolling deferrals carried into Phase 79:**

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
- Phase 76 deferrals (conversational-window query, heuristic
  recall gate, token-budget context sizing).
- Phase 77 deferrals (`[recall_feedback]` tuning knob,
  LLM-judged recall usefulness, cross-session pattern
  learning).
- Phase 78 deferrals (per-memory-entry drill-down, history
  beyond the recall-log window, Web UI live refresh,
  actionable insights).

**Likely Phase 79 deferrals:**

- **`[persona]` tuning block.** v1 ships fixed
  size-threshold / top-K / similarity-floor constants. An
  operator knob defers until retuning is actually needed.
- **Persona consolidation / supersession / decay** (Q1b).
  v1 *selects* per turn but never *rewrites* the chain.
  Meaning-level dedupe, stale-trait decay, and
  proposal-supersession remain the maturation follow-up.
- **Behavioural Persona** (Q1c). v1 is prompt-only. Persona
  influencing tool-preference / verbosity / proactive
  thresholds is a later, separately-scoped arc.
- **Conversational-window selection query.** v1 selects
  against the latest user message only (same horizon Phase 76
  chose); a rolling-window query defers.

## Prediction vs. reality

To be filled in at phase exit.

## Exit criteria

- [ ] `SystemPromptRefiner` hook + builder + `begin_turn`
  swap, **no `lib.rs` edit** — Task 2.
- [ ] Reduced-Persona assembly + always-inject core invariant,
  pure + tested — Task 3.
- [ ] `PersonaContextRefiner` with both fallbacks + ranking +
  invariant — Task 4.
- [ ] Wired into local-CLI, daemon, and child-agent factories
  — Task 5.
- [ ] Per-turn breadcrumb + Phase 78 surface extended (CLI +
  Web UI) — Task 6.
- [ ] Tests across hook, invariant, selection, fallbacks,
  observability — Task 7.
- [ ] `docs/INSTALL.md` + `examples/aivyx.toml` updated —
  Task 7.
- [ ] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 7.
- [ ] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [ ] DESIGN.md streak extends to twenty-six.
- [ ] PRODUCT.md streak extends to nineteen.
- [ ] Production-core streak extends to twenty-seven (new
  record) — `lib.rs` byte-identical.
- [ ] Test count delta: positive (~+15-25; calibrated per the
  76/77/78 pattern — genuine new selection logic with many
  edge cases, but heavily reusing proven seams).
- [ ] Zero clippy warnings.
- [ ] Zero new workspace deps.
- [ ] Prediction-vs-reality block filled.
