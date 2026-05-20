# Phase 89 — Topic Canonicalization (sharper learning through sharper bookkeeping)

For 88 phases the assistant has accumulated topic-keyed
signal: the Phase 7 memory, the Phase 77 recall-feedback log,
the Phase 82 helpfulness ledger, the Phase 83 co-occurrence
ledger, the Phase 87 consolidate-pair proposal IDs. Every one
of those layers keys by **the operator's typed topic string,
verbatim**. That means `deploy`, `Deploy`, `deploys`, and
`deploying` are four distinct topics across every accumulator
— and the signal that should add up across them is silently
fragmented.

Phase 89 closes this long-standing Phase 82 deferral (carried
forward six times through Phases 83 / 84 / 85 / 86 / 87 / 88)
with the smallest substrate fix that touches every consumer
at once: **canonicalize the topic string at the
`Memory::put` boundary**. Every downstream signal (recall log,
ledgers, co-occurrence pairs, consolidate-pair proposals) is
keyed off of values that came through that single seam, so a
fix there propagates to the whole stack with no per-consumer
plumbing.

This is the first phase since the "act on durable learning"
arc closed (Phases 84+87 build, 85+88 retire) that **sharpens
what we already have** rather than adding a new capability —
the natural infrastructure pause before the next big surface.

## Why this, why now

- It is the longest-standing learning-stack deferral, six
  phases old. Every subsequent phase that consumed a ledger
  (84, 85, 87, 88) paid the fragmentation tax silently.
- The seam is small and proven: `Memory::put` is the single
  upstream source of every topic-keyed signal in the stack
  (recall events derive from `MemoryEntry`; helpfulness +
  co-occurrence ledgers fold over recall events; Persona
  `recall-fb:` and `consolidate-pair:` provenance derives
  from ledger topics). Fix the seam, everything downstream
  gets clean signal **without per-consumer plumbing**.
- The fix is contained: one canonicalization function in
  `aivyx-memory`, one call site, one config knob. Zero new
  workspace deps; zero `aivyx-core/src/lib.rs` touch.
- The "existing data decays naturally" choice (Q2a) means
  no migration risk. The Phase 82/83 ledgers have a ~60-day
  half-life; the Phase 77 recall log has a ~30-day retention.
  Within a quarter, the past converges to clean — no
  rewriting of MAC-signed Persona chain entries.

## Streak predictions

- **DESIGN.md** — **Will hold.** A normalization function at
  an existing trait boundary touches no locked technical-
  contract decision. Memory's contract has always been
  `(topic, body) → seq`; canonicalizing `topic` is an
  implementation detail under that contract.
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.
  Prediction: streak **extends to thirty-six** (currently 35).

- **PRODUCT.md** — **Will hold.** No new product commitment,
  none weakened; the operator-facing contract is
  *strengthened* (the assistant's learning is no longer
  silently degraded by topic fragmentation, so G3 recall +
  P14 Persona both improve). No commitment-text edit.
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.
  Prediction: streak **extends to twenty-nine** (currently
  28).

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold,
  by design.** The `Memory` trait + impls live in
  `aivyx-memory`, not `aivyx-core`. The canonicalization
  function is a new pure helper in `aivyx-memory`; the
  config knob is in `aivyx-config`; the only `aivyx-core`
  touch *possible* would be a new `AuditTag`, and Phase 89
  emits none (canonicalization is a substrate sharpening,
  not a behavior event).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.
  Prediction: streak **extends to thirty-seven** consecutive
  phases (new project record, beats Phase 88's 36).

- **New workspace deps** — Zero. Hand-rolled stemmer (Q1a)
  per the 88-phase no-deps discipline.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_89.md` + `docs/README.md` status row.

### Task 2 — `[memory].canonicalize_topics` knob

`aivyx-config`:

- Extend the existing `MemoryConfig` (or add a new
  `[memory]` block if none exists) with
  `canonicalize_topics: bool`, default `false`. Validation
  is trivial — a boolean can't be invalid.
- Tests: absent section → `false`; explicit `true` →
  `true`; explicit `false` → `false`.

### Task 3 — `canonicalize_topic` (the crux)

`aivyx-memory`:

- New `pub fn canonicalize_topic(s: &str) -> String` in a
  new `canonical` module (or top-level pure helper). The
  v1 rule set (Q1a):
  1. **Lowercase** the entire string (ASCII-aware; non-
     ASCII characters pass through as-is to keep the
     stemmer scope-bounded).
  2. **Trim** leading/trailing whitespace.
  3. **Collapse internal whitespace** (one or more spaces
     / tabs → single space).
  4. **Suffix strip** the last word (the "topic head") via
     a tiny ordered ruleset:
     - `ies` (len ≥ 4) → `y` (`policies` → `policy`)
     - `es` (len ≥ 4) → drop (`tests` → `test`)
     - `s` (len ≥ 3, not preceded by `s`) → drop
       (`roles` → `role`; `tests` already caught by `es`
       rule)
     - `ing` (len ≥ 5) → drop (`testing` → `test`)
     - `ed` (len ≥ 4) → drop (`tested` → `test`)
- Idempotent: `canonicalize_topic(canonicalize_topic(s))
  == canonicalize_topic(s)` (the suffix-strip rules don't
  re-fire on already-stripped slugs).
- Unit tests on the pure function: the four rule cases
  (`policies`/`tests`/`roles`/`testing`/`tested`);
  idempotency; lowercase; whitespace folding; the
  short-string guards (`is`, `as` not stripped to empty);
  ASCII fold guard (non-ASCII characters pass through);
  identity on already-canonical input.

### Task 4 — Wire the seam + the config knob

`aivyx-memory`:

- `InMemoryMemory` + `RedbMemory` both gain a
  `canonicalize: bool` field (default `false`); the
  constructor takes it. `put(topic, body)` calls
  `canonicalize_topic(topic)` first when `canonicalize` is
  `true`, otherwise stores `topic` verbatim (byte-identical
  to pre-Phase-89).
- The topic-keyed read APIs that take a topic-string
  argument (`get_recent`, the keyword search backend,
  whatever the existing semantic-search-by-topic helper is
  called) **also** canonicalize their topic argument when
  the flag is on — otherwise you write `Deploy` (stored as
  `deploy`), then read `deploys`, and the lookup misses.
  Same flag, same function.

`bin/aivyx`:

- When constructing memory at daemon startup, pass
  `config.memory.canonicalize_topics` to the memory
  constructor. With no `[memory]` block (or the flag at
  `false`), the memory is byte-identical to pre-Phase-89;
  with `true`, the seam engages.

- Integration tests: a put with `Deploys` is found by a
  read with `deploy` (canonicalization on both sides; same
  fold); with the flag off, the same case stays distinct
  (byte-identical pre-Phase-89). The downstream
  consumers — exercise this through the existing recall-
  feedback fold so the helpfulness ledger key shows up
  canonicalized.

### Task 5 — Tests + docs + exit

- `docs/INSTALL.md` — "Topic canonicalization (Phase 89)"
  section: what fragmentation is, the operator-visible
  improvement, the opt-in knob, the existing-data-decays-
  naturally posture (no migration), the v1 rule set, the
  obvious deferrals (operator-tunable aliases; topic-by-
  topic exception list).
- `examples/aivyx.toml` — document the new
  `[memory].canonicalize_topics` knob.
- Exit: ROADMAP + PRODUCT_ROADMAP frozen entries,
  docs/README status flip, prediction-vs-reality, hash
  backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Rule set:** (a) Lowercase + small hand-rolled
  English suffix stemmer. Zero new deps; deterministic;
  catches the four real fragmentation cases (`deploy`/s/
  ing/ed; `tests`/`testing`; `roles`/`role`; `policies`/
  `policy`). Edge cases like `Deploy` (proper noun) get
  lowercased — the project's topic slugs have always been
  category labels, not entity names; the trade-off is
  positive.
- **Q2 — Scope:** (a) Write-side only. Existing fragmented
  signal decays out via the Phase 82/83 ~60-day half-life
  and the Phase 77 ~30-day recall log retention. No
  migration code, no migration risk; the past converges
  within ~quarter without intervention. The Persona chain
  keeps its original `recall-fb:` / `consolidate-pair:`
  provenance strings; future facets are clean.
- **Q3 — Seam:** (a) At the `Memory::put` boundary (and the
  matching topic-keyed read paths so writes are findable).
  Single canonicalization function, single seam, every
  downstream consumer inherits automatically.
- **Q4 — Default:** (a) Opt-in via `[memory].canonicalize_
  topics`, default `false`. Matches the project's 88-phase
  behaviour-change-is-opt-in discipline; operators
  consciously enable; with no block (or `false`) the
  memory layer is byte-identical to pre-Phase-89. A future
  phase may flip the default once operator confidence
  accrues.

## Deferrals

**Rolling deferrals carried into Phase 89:**

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
- Phase 82 deferrals (operator-tunable half-life/retention;
  **topic canonicalization — THIS PHASE**).
- Phase 83 deferrals (sequential/temporal patterns, n-ary
  clusters, operator-tunable top-K/half-life).
- Phase 84 deferrals (affinity re-ranking of existing
  candidates, operator-tunable affinity policy).
- Phase 85 deferrals (reflection-facet decay via fuzzy
  embedding).
- Phase 86 deferrals (token-budget context sizing, heuristic
  recall gate, embed-each-and-pool windows, persisted
  windows).
- Phase 87 deferrals (n-ary cluster proposals, pattern-
  driven supersession, operator-tunable LLM prompt).
- Phase 88 deferrals (pattern-driven supersession, n-ary
  cluster decay, pair-affinity hysteresis).

**Likely Phase 89 deferrals:**

- **Operator-tunable aliases.** A `[[topic_alias]]` config
  block (`from = "k8s"; to = "kubernetes"`) for operator-
  driven mappings the stemmer can't infer. Defers; the
  hand-rolled stemmer covers morphology, not synonymy.
- **Topic-by-topic exception list.** Some operators may
  legitimately want `Deploy` and `deploy` as distinct (the
  literal first is a proper noun for a tool, the second a
  verb). Defers as a `[[topic_canonical_exception]]` block;
  the v1 trade-off accepts the collision.
- **One-time migration of existing fragmented data.** Q2
  picked write-side only with natural decay; a `aivyx
  memory recanonicalize` subcommand that rewrites the
  Phase 82/83 ledgers (the Persona chain is MAC-signed —
  cannot be rewritten without breaking the chain) defers
  pending operator demand.
- **Non-ASCII / Unicode stemming.** Phase 89 stems ASCII
  English only. International topic conventions defer.

## Prediction vs. reality

To be filled in at phase exit.

## Exit criteria

- [ ] `[memory].canonicalize_topics: bool` (default
  `false`) — Task 2.
- [ ] `canonicalize_topic(&str) -> String` pure function in
  `aivyx-memory`, implementing the Q1a rule set
  (lowercase + trim + whitespace fold + the five-rule
  suffix stripper); idempotent — Task 3.
- [ ] Unit tests on the pure function: each rule case,
  idempotency, lowercase, whitespace folding, short-string
  guards, ASCII-fold guard, identity on canonical input —
  Task 3.
- [ ] `Memory::put` + matching topic-keyed read APIs apply
  the canonicalization when the flag is on; byte-identical
  to pre-Phase-89 when off — Task 4.
- [ ] `bin/aivyx` threads `config.memory.canonicalize_
  topics` to the memory constructor — Task 4.
- [ ] Integration test: write `Deploys`, read with
  `deploy`, both fold; downstream consumer (recall-feedback
  fold) inherits the canonical key — Task 4.
- [ ] `docs/INSTALL.md` + `examples/aivyx.toml` updated —
  Task 5.
- [ ] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 5.
- [ ] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [ ] DESIGN.md streak extends to thirty-six.
- [ ] PRODUCT.md streak extends to twenty-nine.
- [ ] Production-core streak extends to thirty-seven (new
  record) — `lib.rs` byte-identical.
- [ ] Test count delta: positive (~+6-10; per the
  converged calibration law — knob on an existing block
  (≈ +1) + new pure helper (≈ +4-6) + seam integration
  (≈ +1-2); no new module, no new `KeyDomain`).
- [ ] Zero clippy warnings.
- [ ] Zero new workspace deps.
- [ ] Prediction-vs-reality block filled.
