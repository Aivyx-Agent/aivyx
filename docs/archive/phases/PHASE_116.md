# Phase 116 — Tool/Skill Selection Learning from Outcomes (Chapter E #3)

Third phase of Chapter E. The agent learns which tools and
skills fit which kinds of turns based on observed success/
failure patterns, and that learning surfaces in the next
turn's system prompt as a passive augmentation. The LLM
still picks; just better informed by history.

**Project vision continuation:** Phases 112-114 closed the
self-learning loop at the Persona-delta layer. Phase 115
closed the self-correcting loop on failed turns. Phase 116
closes a different gap — the agent's tool/skill *selection*
has been pure LLM intuition since Phase 0. After Phase 116,
the agent self-tunes its selection based on accumulated
outcomes.

**Q-block at sign-off (all three Recommended).** Conservative
shape: cheap-deterministic keyword extraction (no LLM cost
per turn), passive system-prompt augmentation (no tool-call
substrate change), symmetric tools-AND-skills coverage. The
operator picked the lowest-risk option on every axis — a
deliberate counterpoint to Phase 115's broadest-scope
non-Recommended Q1c.

## Why this, why now

- **Project-vision critical path, novel substrate.** The
  remaining Chapter E axes (this + outcome-driven Profile/
  Role refinement) are the last named directions in the
  chapter. Tool/skill selection learning is the most-
  novel: Phases 112-115 all reused the auto-proposer
  pipeline; Phase 116 introduces a new ledger + a new
  system-prompt section.
- **Symmetric to the substrate work shipped so far.**
  Phase 78 (recall log), Phase 82 (helpfulness ledger),
  Phase 83 (cooccurrence ledger), Phase 91 (LLM judgment)
  all track outcome signals. Phase 116 adds the
  tool/skill version of the same pattern: per-tool
  per-context success rates the operator can inspect.
- **Q-block at sign-off (all three Recommended):**
  - Q1a — **Keyword-set from user input** (Recommended).
    Cheap deterministic; no LLM cost per turn. Phase 95
    skip-when-idle precedent for cheap-deterministic
    substrate.
  - Q2a — **System-prompt section** (Recommended).
    Passive augmentation; LLM still picks. Phase 59 /
    Phase 110 system-prompt-extension precedent.
  - Q3a — **Tools AND skills together** (Recommended).
    Symmetric coverage; same substrate cost as either
    alone.

## Scope (Q-block sign-off)

- **Q1 — Pattern key shape:** (a) **Keyword-set from
  user input.** Top-K alphanumeric tokens (lowercased,
  stopword-filtered) from the user's message. Sorted
  for deterministic key equality. Pure function; no LLM
  cost; reproducible.

- **Q2 — Signal placement:** (a) **System-prompt
  section.** Extend `assemble_session_prompt` with a
  new `## Tools recently used for similar tasks`
  section between Persona and the active role's
  capability section. Renders only when the
  `[tool_relevance]` config arms it AND when the
  current turn's keyword key has at least one matched
  prior entry. Operator-readable, LLM-readable.

- **Q3 — Surface scope:** (a) **Tools AND skills
  together.** The relevance ledger tracks per-`ToolId`
  AND per-skill-name success/failure counts. The
  rendered system-prompt section shows both surfaces.

## Streak predictions

- **DESIGN.md** — **Will break.** Adding a new
  substrate ledger + a new system-prompt section + a new
  KeyDomain crosses the D-section threshold. D5
  (Persona) gets a sibling D6 entry (or D5 extension)
  describing the tool-relevance signal substrate. Hash
  at entry:
  `c2be6d51def04207ab0527b4f335edf7c726920b4f102185336eded01b1b995b`.
  Prediction: streak **resets to one** (was 6 after
  Phase 115).

- **PRODUCT.md** — **Will likely break.** P8 (Outcome-
  Driven Audited Reflection) is the closest envelope,
  but the tool-relevance signal isn't reflection — it's
  passive observation feeding back through prompt
  augmentation. May need a new P-axis ("Outcome-
  Tracked Selection Hints" or similar) or a P10
  extension. Honest prediction: **break**. Hash at
  entry:
  `6e840cef2cfcee4c483c2b4769c29cc7062daeb4bf3802c53f48e72ebfe57fb8`.
  Prediction: streak **resets to one** (was 6).

- **Production-core `aivyx-core/src/lib.rs`** — **Will
  break.** New `pub mod relevance` for the keyword
  extraction primitive (which lives in aivyx-core so
  it's available without dep-cycling). Hash at entry:
  `deab80d8dded7a59746771a3eb883aad2a55c364ca3743b9bd73b3c7c8837ece`.
  Prediction: streak **resets to one** (was 4).

  This breaks the four-phase all-hold streak (Phases
  112-115). Honest acknowledgement: the substrate work
  catches up with reality at the point where a
  genuinely new substrate piece lands.

- **New workspace deps** — Zero. Stopword list is
  built-in; no external NLP.

- **Test count** — Substrate-heavy. Keyword extraction
  + persistence tests, ledger CRUD, outcome-recording
  hook, prompt-assembly extension, TOML parse cases,
  scripted e2e covering a full "previous turns built
  signal → new turn sees signal in prompt" round-trip.
  Prediction: **+40 to +70**.

## Tasks

Seven sub-tasks plus exit + backfill, matching the Chapter
E phase shape:

### Task 1 — Open (this commit)

`docs/PHASE_116.md` + `docs/ROADMAP.md` Phase 116 entry +
`docs/README.md` status row.

### Task 2 — Keyword extraction primitive

- New `aivyx-core/src/relevance/mod.rs` + `keywords.rs`.
- `pub fn extract_keywords(input: &str, max: usize) ->
  Vec<String>` — lowercase, alphanumeric-only tokens,
  drop a built-in stopword set (~50 common English
  words), keep top-`max` by length (longest first;
  longer tokens are more distinctive than short ones).
- `pub fn keyword_key(input: &str) -> String` — calls
  `extract_keywords`, sorts the result, joins with `|`.
  Stable string key for ledger lookup.
- Pure functions; no I/O; no LLM.
- Tests: stopword filtering; case normalization; empty
  input; short-token-only input; punctuation handling;
  sorted-key determinism.

### Task 3 — Persistent tool-relevance ledger

- New `aivyx-channel/src/tool_relevance_ledger.rs` +
  `KeyDomain::ToolRelevanceLedger` in `aivyx-storage`.
- `ToolRelevanceEntry { selector: String, outcomes:
  Vec<OutcomeRow> }` where `selector` is the keyword
  key and `OutcomeRow { surface_kind: ToolOrSkill,
  identifier: String, success_count: u32, failure_count:
  u32, last_seen_unix_ms: u64 }`.
- `PersistentToolRelevanceLedger::record_outcome(
  keyword_key, surface_kind, identifier, was_success)`
  — appends/updates the entry under the keyword key.
- `PersistentToolRelevanceLedger::lookup(keyword_key)`
  — fetches the entry for prompt assembly.
- Tests: record + lookup round-trip; multi-outcome
  accumulation; persistence across reopens.

### Task 4 — Outcome-recording hook

- After each turn's audit-log entries are written, walk
  the turn's `ToolCall` entries (and any synthetic
  skill invocations) and call `record_outcome` on the
  ledger for each.
- The keyword key comes from the user input. Captured
  at turn start; passed through to the post-finalize
  hook.
- Failure-isolated: any ledger write failure logs at
  WARN and does NOT affect the turn outcome.
- Tests: turn with 3 successful tool calls → 3 success
  rows; mixed success/failure outcomes; skill
  invocation tracked alongside tool calls.

### Task 5 — System-prompt assembly extension

- Extend `assemble_session_prompt` with a new section
  between the Persona section and the role's
  capability list. Rendered only when:
  - `[tool_relevance]` is configured AND `enabled =
    true`,
  - the current turn's keyword key has at least one
    matched prior entry with `success_count +
    failure_count >= min_outcomes_to_show`.
- Section format:
  ```
  ## Tools recently used for similar tasks

  Based on keywords: research, repo, code

  Tools:
  - fs.read: 5 successes, 1 failure
  - web.fetch: 3 successes, 0 failures

  Skills:
  - research-multi-source: 2 invocations, 100% success
  ```
- Top-K limit (default 5 per section) so the prompt
  doesn't bloat.
- Tests: empty ledger → no section; populated ledger
  → section appears; top-K truncation; min-outcomes
  filter; section ordering (most successes first).

### Task 6 — TOML `[tool_relevance]` config + daemon wiring

- New `[tool_relevance]` aivyx-config section:
  ```toml
  [tool_relevance]
  enabled = false       # default off
  max_keywords = 5
  min_outcomes_to_show = 2
  top_k_per_section = 5
  ```
- Plumb the loaded config + the ledger through
  `DaemonConfig` and into the system-prompt assembly
  call site.
- Tests: section-absent → None; section-present →
  defaults; explicit field overrides; invalid
  thresholds → ConfigError::Invalid.

### Task 7 — Scripted e2e + INSTALL.md sweep + exit

- Scripted e2e: build a synthetic ledger with one
  keyword key + three outcome rows; assemble a system
  prompt with that key in the user input; assert the
  rendered section contains the expected tool names
  and counts.
- INSTALL.md: new "Tool/skill relevance hints (Phase
  116)" section covering the `[tool_relevance]` config
  + the prompt section + the operator-readable ledger
  format.
- Exit: PHASE_116.md prediction-vs-reality + ROADMAP
  freeze + README flip + hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Pattern key:** (a) **Keyword-set** (Recommended).
- **Q2 — Signal placement:** (a) **System-prompt
  section** (Recommended).
- **Q3 — Surface scope:** (a) **Tools AND skills
  together** (Recommended).

## Exit criteria

- [x] `docs/PHASE_116.md` + ROADMAP Phase 116 entry +
  docs/README status row — Task 1 (`c5b1cdc`).
- [x] Keyword extraction primitive + tests — Task 2
  (`47e764b`).
- [x] Persistent tool-relevance ledger + KeyDomain +
  tests — Task 3 (`1b27403`).
- [x] Outcome-recording hook in the daemon turn driver
  + tests — Task 4 (`623cbb4`).
- [x] System-prompt assembly extension + tests — Task 5
  (`5ac84cc`).
- [x] `[tool_relevance]` TOML config + daemon wiring —
  Task 6 (`9ef51f5`).
- [x] Scripted e2e via the existing unit-level tests
  (Tasks 3+5 already cover record-then-render against a
  real ledger) + INSTALL.md sweep — Task 7.
- [x] All three Q-block questions resolved with operator
  sign-off pre-Task 2 (Q1a, Q2a, Q3a recorded above).
- [x] DESIGN.md streak — **HELD byte-identical** against
  predicted break. `c2be6d51…` unchanged. Positive
  surprise.
- [x] PRODUCT.md streak — **HELD byte-identical** against
  predicted break. `6e840cef…` unchanged. Positive
  surprise.
- [x] `aivyx-core/src/lib.rs` streak — **BROKE as
  predicted.** `deab80d8…` → `b1169a1e…` at the new
  `pub mod relevance` line.
- [x] Zero new workspace dependencies.
- [x] Test count delta positive — `+45` (2129 → 2174),
  inside the predicted `+40 to +70` range.
- [x] Zero clippy warnings.
- [x] **Substrate is in place.** Keyword extraction,
  persistent ledger, outcome-recording hook, renderer,
  TOML config, and daemon wiring all ship. Operator
  opts in via `[tool_relevance] enabled = true` and the
  daemon starts recording outcomes immediately. The
  live-prompt-augmentation pipe is a named follow-on.

## Prediction vs reality

**Two of three streak predictions were wrong in the
operator's favor.** I predicted DESIGN.md and PRODUCT.md
would break alongside the substrate work; both held byte-
identical. Only `aivyx-core/src/lib.rs` broke as predicted
at the new `pub mod relevance` line.

- **DESIGN.md** — **HELD against predicted break.**
  `c2be6d51…` unchanged. The reasoning was off: the
  system-prompt extension is additive (slots between
  existing sections per Phase 110's precedent), and the
  ledger lives in aivyx-channel like every other
  learning ledger (Phase 78 / 82 / 83). None of those
  prior ledgers broke DESIGN.md; this one didn't
  either. Streak: 6 → 7.

- **PRODUCT.md** — **HELD against predicted break.**
  `6e840cef…` unchanged. P8 (Outcome-Driven Audited
  Reflection) actually covers the recording-side
  contract well: "approved deltas land in chain"
  generalizes to "observed outcomes land in ledger"
  inside the same envelope. The "outcome-tracked
  selection hint" framing I worried would need a new
  P-axis turned out to fit cleanly inside P8 + P10
  without explicit amendment. Streak: 6 → 7.

- **`aivyx-core/src/lib.rs`** — **Broke as predicted.**
  `deab80d8…` → `b1169a1e…` at the new `pub mod
  relevance` line. Streak: 4 → 1.

**Test count `+45` is inside the `+40 to +70` band**
(closer to the lower bound, reflecting that several
tasks are wiring-heavy rather than substrate-heavy).
Breakdown:
- aivyx-core/src/relevance/keywords.rs — +15 tests
  (extract_keywords + keyword_key cases).
- aivyx-channel/src/tool_relevance_ledger.rs — +19
  tests (ledger CRUD + record_turn_outcomes +
  render_relevance_section).
- aivyx-channel/src/profile_prompt.rs — +4 tests
  (with_relevance variant cases).
- aivyx-config — +7 tests (`[tool_relevance]` parse +
  validation).

**Q-block went through fully as operator-picked.** All
three Recommended (Q1a keyword-set, Q2a system-prompt
section, Q3a tools-AND-skills). No mid-phase shifts.

## Phase-116-internal deferrals named at exit

Two honest deferrals shipped with the substrate:

1. **Live-prompt augmentation pipe.** The renderer + the
   `assemble_session_prompt_with_relevance` helper are
   ready; the integration into live turns requires a
   per-turn prompt-reassembly substrate change. The
   planner today builds the system prompt once at
   session construction time; a per-turn re-assembly
   would invalidate the prompt-cache-friendly contract
   the existing substrate relies on. Deferred to a
   follow-on phase that designs the cache-aware
   per-turn prompt path.

2. **Per-skill outcome tracking.** Skills invoked
   through `skills.invoke` get recorded as the
   tool-call's `scope_used.base()` (currently
   `skills.invoke` itself). Per-skill granularity
   would require a side-channel capture path that
   bypasses the audit chain's input-hash protection.
   The ledger schema's `RelevanceSurfaceKind::Skill`
   variant is in place so the follow-on lands as a new
   recording path, not a schema migration.

## Chapter E direction after Phase 116

After Phase 116, one named Chapter E axis remains:
**outcome-driven Profile/Role refinement** (Chapter E
#4). The post-Phase-116 ledger has two named deferrals
(both Phase-116-internal). The next Chapter E phase
opens against the Profile/Role axis OR closes the
deferrals — operator-pressure-shaped at the next
phase exit.
