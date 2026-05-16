# Phase 77 — Recall → Reflection Feedback Loop

Phase 76 made memory recall automatic. But the loop is open:
recalled memories don't yet teach the system anything. Phase
77 closes it. Each turn's recall is persisted; a structural
(non-LLM) signal correlates recalls with how the turn actually
went; and the existing cron reflection loop uses that
accumulated signal two ways — it self-tunes memory retention
(helpful memories survive, chronically-useless ones decay) and
it emits operator-gated Persona proposals when a recall
pattern is significant. This is the "self-**improving**" half
of the vision becoming real: the assistant gets better at
remembering the right things, on its own, with the operator
still the final authority on identity change.

## Why this, why now

- Phase 76 built the recall path and proved (Q4b) that the
  audit chain is streak-locked. Phase 77 deliberately routes
  the new signal *around* `AuditTag` via a dedicated
  `KeyDomain` — the Phase 76 lesson applied, not relearned.
- The reflection auto-loop (Phases 70–71) already walks a
  lookback window on a cron and emits operator-gated
  proposals. Recall feedback is a second input + a second
  actuator on a proven cadence — not new machinery.
- Vision alignment: continuity (P76) without learning is just
  a bigger cache. The feedback loop is what makes the memory
  substrate *improve* with use, which is the core promise.

## Streak predictions

- **DESIGN.md** — **Will hold.** A new `KeyDomain` variant
  (precedent: Phases 21/26/27/56/70/75), a recall-event log,
  a structural correlator, and retention/proposal actuators
  are all substrate extensions; no locked technical-contract
  decision is touched. Prediction: streak **extends to
  twenty-four** consecutive phases (currently 23).
  Hash at entry: `89dc89035f15daefa45d3e6df2c2c5327ed754707a8c8c2cdf8279fd70a94bce`.

- **PRODUCT.md** — **Will hold.** Closing the recall→learning
  loop is a quality improvement to already-delivered G3
  (memory) / P8 (reflection) / P14 (Persona) substrate — it
  makes them work together better, it is not a new product
  commitment. No commitment-text edit. Prediction: streak
  **extends to seventeen** consecutive phases.
  Hash at entry: `cd60c4f9ec39d970243ab90d8e071938eacb5bbfa9eca085e265aa339511088e`.

- **Production-core `aivyx-core/src/lib.rs`** — **Will hold,
  deliberately.** The recall-signal capture extends
  `ContextProvider::recall()` to receive a `SessionId`
  correlation key; that signature lives in
  `crates/aivyx-core/src/llm_planner.rs` (the Phase 76
  precedent), and `SessionId` / `Message` are merely
  *referenced*, not redefined, so `lib.rs` stays
  byte-identical. The `KeyDomain` variant lands in
  `aivyx-storage`. The structural signal reads the **existing**
  `AuditTag::TurnEnded` — explicitly **no** new audit variant
  (the Phase 76 streak lesson, respected by design — this is
  why Q1a/Q2a avoid the audit chain for storage). Prediction:
  streak **extends to twenty-five** consecutive phases (new
  project record, beats Phase 76's 24).
  Hash at entry: `69fb9af1814f3f0741baca884b8b67690533046a634e87bbc61ef00f11d0c844`.

- **New workspace deps** — Zero. Reuses the Phase 75/76
  embedding + vector + recall substrate and the Phase 70–71
  reflection loop.

## Tasks

### Task 1 — Open (this commit)

`docs/PHASE_77.md` + `docs/README.md` status row.

### Task 2 — `KeyDomain::RecallEvents` storage variant

`aivyx-storage`: add the variant via the full established
pattern (the Phase 75 `MemoryVectors` checklist): enum variant
+ `as_bytes` (`b"recall-events"`) + `table_name`
(`aivyx_recall_events_v1`) + `ALL` array + `subkeys` array
size + `derive_all_subkeys` + `subkey_for` match +
`key_domain_all_covers_every_variant` tripwire + a
domain-isolation test. Encrypted, HKDF-isolated like every
other domain.

### Task 3 — `RecallEvent` + `PersistentRecallLog`

`aivyx-channel` (sibling of `persona_proposal` /
`persona_log`):

- `RecallEvent { ts_secs, session_id, hits: Vec<RecallHit> }`
  where `RecallHit { topic, seq, score }` — what was injected
  into one turn.
- `PersistentRecallLog`: `append(event)` + a windowed read
  (`since_secs` / by session) for the reflection lookback.
  Bounded growth — a GC clamp on the oldest events mirrors the
  memory-GC discipline.

### Task 4 — Capture at recall time

- `ContextProvider::recall()` gains a `session_id: SessionId`
  parameter (in `llm_planner.rs`; `begin_turn` passes
  `message.session_id`; **no `lib.rs` edit**). The Phase 76
  fake provider + its 4 tests update for the new arg.
- `SemanticMemoryContext` holds an optional
  `Arc<PersistentRecallLog>` and appends a `RecallEvent`
  (with the surviving hits + their cosine scores) whenever it
  injects a block. Append failure is non-fatal — recall still
  works; learning just misses that turn (the established
  best-effort ethos).

### Task 5 — Structural correlation scorer

`aivyx-channel`, pure + unit-tested:

- Input: recall events in a window + the audit chain's
  `TurnEnded` summaries (already what the reflection scheduler
  reads) for the same window.
- Heuristic (Q1a, no LLM): a recall whose turn ended clean and
  was **not** followed by an immediate operator
  correction/rephrase scores weak-positive for its hit
  entries; a recall on a turn the operator immediately
  corrected scores weak-negative. Accumulate per `(topic,
  seq)` into a helpfulness tally. Coarse per-turn,
  reliable in aggregate.
- Output: a per-entry helpfulness delta map the actuators
  consume.

### Task 6 — Actuator A: memory-retention self-tuning

- Repeatedly-helpful entries are retention-promoted (shielded
  from TTL / LRU decay); chronically-recalled-but-never-helpful
  entries decay faster / are flagged for eviction. Driven
  through the existing Phase 74 retention + eviction
  machinery (no new eviction policy primitive — a helpfulness
  bias on the existing one). Mechanical, reversible, bounded.

### Task 7 — Actuator B: operator-gated Persona proposals

- When the accumulated signal shows a significant recall
  pattern, the reflection pass emits a **Pending** proposal
  into the existing `persona_proposal` chain (operator
  approves/rejects — never auto-applied; the Phase 70 P14
  authority rule holds). Reuses the proposal + gate substrate
  end to end.

### Task 8 — Piggyback the cron reflection loop

- The reflection scheduler's existing on-cron pass also: reads
  the recall window, runs the Task 5 scorer, applies Task 6
  retention bias, and feeds Task 7 proposal emission. One
  cadence, one operator-controlled timer; no new scheduler.
  When no `[embedding]` / no recall events exist, the pass is
  a no-op (pre-Phase-77 behavior).

### Task 9 — Tests + docs + exit

- Tests: storage isolation, recall-log append/window/GC,
  capture (signature + event shape + non-fatal append),
  scorer (clean/corrected/positive/negative + aggregation),
  retention actuator (promote/decay/no-op), proposal actuator
  (pattern → Pending, operator-gated), reflection-loop
  integration, no-`[embedding]` no-op.
- Docs: `examples/aivyx.toml` any new knob + a feedback-loop
  note; `docs/INSTALL.md` "Recall feedback loop (Phase 77)"
  section (what it learns, the no-LLM honesty, operator stays
  the authority on Persona).
- Exit: ROADMAP + PRODUCT_ROADMAP frozen entries, docs/README
  status flip, prediction-vs-reality, hash backfill.

## Q-block resolutions (signed off pre-Task 2)

- **Q1 — Feedback signal:** (a) Structural correlation, **no
  LLM**. Persist per-turn recall events; correlate with the
  audit chain's existing `TurnEnded` outcome + an
  operator-corrected-next-turn heuristic. Honest, cheap, not
  self-judgement; coarse per-turn but reliable in aggregate.
- **Q2 — Signal storage:** (a) New `KeyDomain::RecallEvents`
  persisted log keyed by time/session. Streak-safe (`KeyDomain`
  is not a streak file; `MemoryVectors` precedent); reflection
  gains it as a second lookback input. Deliberately **not** the
  audit chain (Phase 76 streak lesson).
- **Q3 — Actuator:** (c) **Both** — self-tuning memory
  retention **and** operator-gated Persona proposals from the
  same accumulated signal. The fullest loop closure;
  acknowledged as the largest-surface option (two actuators,
  two test matrices) and scoped accordingly across Tasks 6–7.
- **Q4 — Cadence:** (a) Piggyback the existing cron reflection
  auto-loop. Zero new scheduler; one operator-controlled
  cadence for all learning.

## Deferrals

**Rolling deferrals carried into Phase 77:**

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
  recall gate, token-budget context sizing, Web UI recall
  indicator).

**Likely Phase 77 deferrals:**

- **LLM-judged recall usefulness.** v1 is deliberately
  structural. An optional reflection-LLM assessment of recall
  quality defers until the structural signal proves
  insufficient in practice.
- **Recall-event Web UI surface.** v1 keeps the loop
  headless (log + persisted events). An operator dashboard of
  "what memory is helping" defers.
- **Tunable heuristic weights.** v1 ships fixed
  positive/negative weights + correction-detection window. A
  `[recall_feedback]` knob block defers until an operator
  needs to retune.
- **Cross-session pattern learning.** v1 correlates within
  the lookback window. Long-horizon, cross-session pattern
  mining is a later self-improvement arc.

## Prediction vs. reality

To be filled in at phase exit.

## Exit criteria

- [ ] `KeyDomain::RecallEvents` + isolation test — Task 2.
- [ ] `RecallEvent` + `PersistentRecallLog` (append + windowed
  read + GC clamp) — Task 3.
- [ ] Capture: `recall()` `SessionId` arg (**no `lib.rs`
  edit**) + non-fatal event append — Task 4.
- [ ] Structural correlation scorer (no LLM) — Task 5.
- [ ] Retention self-tuning actuator — Task 6.
- [ ] Operator-gated Persona-proposal actuator — Task 7.
- [ ] Piggybacked into the cron reflection loop; no-op when
  no recall events — Task 8.
- [ ] Tests across storage, log, capture, scorer, both
  actuators, reflection integration, no-op — Task 9.
- [ ] `examples/aivyx.toml` + `docs/INSTALL.md` updated —
  Task 9.
- [ ] ROADMAP + PRODUCT_ROADMAP + docs/README refreshed —
  Task 9.
- [ ] All four Q-block questions resolved with operator
  sign-off pre-Task 2.
- [ ] DESIGN.md streak extends to twenty-four.
- [ ] PRODUCT.md streak extends to seventeen.
- [ ] Production-core streak extends to twenty-five (new
  record) — `lib.rs` byte-identical.
- [ ] Test count delta: positive (~+35-55).
- [ ] Zero clippy warnings.
- [ ] Zero new workspace deps.
- [ ] Prediction-vs-reality block filled.
