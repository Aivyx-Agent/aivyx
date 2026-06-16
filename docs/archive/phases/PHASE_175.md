# Phase 175 — Loop Progress Log (cross-iteration learning)

**The Ralph-defining capability the loop still lacks.** Ralph's
power isn't the loop — it's that *memory persists through a
progress file*, so each fresh iteration starts knowing what the
last one learned. Phase 173 gave the loop fresh-context
iterations + a backlog; Phase 174 made it safe. But each
iteration today starts cold: the canonical prompt *tells* the
agent to "write one learning line to memory," yet nothing
guarantees that learning reaches the next iteration. Phase 175
wires the progress log: every iteration's learnings are
**unconditionally surfaced into the next fresh context**, so the
loop accumulates knowledge across a long backlog instead of
re-discovering the same gotchas.

## Why a dedicated mechanism (and not just auto-recall)

Aivyx already has auto-recall (Phase 76) — but it's
embedding-similarity-gated and best-effort: a learning surfaces
only when the next turn's text happens to embed near it. A
progress log must be **deterministic and always-in-context**
(the whole point of `progress.txt` is that it's *always* read),
so Phase 175 injects it directly rather than relying on
similarity recall.

## Design

- **A reserved progress topic in the existing memory
  substrate.** Progress notes are durable, survive across runs,
  and need no new storage domain — they live in `aivyx-memory`
  under a reserved topic (`loop:progress`). A note about the
  codebase ("the build needs `--release`", "tests live in
  `tests/`") is exactly the durable, cross-run knowledge memory
  is for.

- **A `loop.note` tool guarantees the topic.** Rather than hope
  the agent picks the right `memory.write` topic, a dedicated
  channel-tier `loop.note { text }` tool (sibling of
  `loop.next` / `loop.complete`) owns the reserved topic and
  appends the learning. Explicit, auditable, and the canonical
  prompt names it directly. The thirteen-tool core (A12) stays
  untouched; one new `loop.note` capability base.

- **The driver injects.** Before firing each iteration, the
  driver reads the last *N* progress notes
  (`[loop].progress_inject_count`, default 20) and prepends a
  rendered `## Progress so far` block to the canonical prompt,
  so every fresh context opens with what prior iterations
  learned. `N = 0` disables injection (pre-175 behaviour).

This reuses everything — the memory substrate, the loop-tool
pattern, the driver's per-iteration prompt assembly — and adds
no new storage domain and no new workspace dependency.

## Tasks

1. **Open doc + README.** This doc, README Active row, backfill
   Phase 174's frozen hash (`01d5777`).

2. **`loop.note` tool + scope.** New `LoopNoteTool` (channel-
   tier) appends `text` to memory under the reserved
   `LOOP_PROGRESS_TOPIC`. New `loop.note` capability base
   (Trusted-tier, like the other loop tools; A3 enumeration
   71 → 72). Registered in `bin/aivyx` with the daemon's memory
   handle.

3. **Progress render + config.** `[loop].progress_inject_count`
   (default 20; 0 disables). A pure `render_progress_block(notes:
   &[String]) -> String` so the injected text is unit-testable.
   Update `LOOP_SYSTEM_PROMPT` to instruct the agent to record
   learnings via `loop.note` (so it lands in the progress log
   the next iteration will read).

4. **Driver injection wiring.** Thread the memory `Arc` +
   `progress_inject_count` into `run_loop_driver`. Before each
   `fire`, read the last-N notes from the reserved topic, render
   the block, and pass `LOOP_SYSTEM_PROMPT + block` as the
   iteration's prompt (plain canonical prompt when there are no
   notes / injection is off). Best-effort: a memory read error
   degrades to no-injection, never breaking the run. Tests for
   the render + the read-cap path.

5. **CLI `loop log` + INSTALL + exit + Frozen.** `aivyx loop
   log [--limit N]` renders recent progress notes over a new IPC
   pair (operator parity with the agent's view). INSTALL
   section; exit doc; README Frozen flip.

## Streak predictions

- **DESIGN.md** — **Will hold.** Streak: 11 → **12**. A new
  concrete capability scope (added via the A3 amendment-file
  process, as `loop.next` / `calendar.write` were) + a channel-
  tier tool are not a contract change; the thirteen-tool core
  (A12) is untouched.
- **PRODUCT.md** — **Will hold.** Streak: 65 → **66**. A
  capability uplift to an existing surface, not a new
  commitment.
- **`aivyx-core/src/lib.rs`** — **Will hold.** Streak: 11 →
  **12**. Work lands in `aivyx-channel` + `aivyx-config`;
  `aivyx-core` is untouched.

## Exit criteria

- [ ] `docs/PHASE_175.md` + README row + Phase 174 backfill —
  Task 1.
- [ ] `loop.note` tool writes the reserved topic; `loop.note`
  base registered (A3 71 → 72) — Task 2.
- [ ] `[loop].progress_inject_count` + `render_progress_block`;
  `LOOP_SYSTEM_PROMPT` names `loop.note` — Task 3.
- [ ] `run_loop_driver` injects the last-N notes into each
  iteration's prompt; off / empty → plain prompt — Task 4.
- [ ] `aivyx loop log` — Task 5.
- [ ] DESIGN.md / PRODUCT.md / `aivyx-core/src/lib.rs` HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+12` to `+22`. *(Tightened from the
  Phase 174 retro: glue-heavy loop phases land low; the
  testable surface here is the tool + the pure render + config,
  with driver injection covered transitively.)*

## Honest scope risks at sign-off

- **Progress notes share the memory substrate.** Under the
  reserved `loop:progress` topic they're identifiable, but they
  also become eligible for normal auto-recall + count toward
  memory growth / GC like any entry. Arguably a feature (loop
  learnings recallable in normal sessions); documented either
  way.
- **Injection is last-N by recency, not relevance.** A long
  project may push an early-but-still-relevant learning past the
  window. `N` is operator-tunable; a relevance-ranked progress
  log is a future refinement.
- **No de-duplication.** If the agent records the same learning
  twice, both appear. A dedup pass is deferred.
- **The agent must call `loop.note`.** The canonical prompt
  directs it, but a turn that forgets records nothing — the
  loop degrades to pre-175 (cold) behaviour for that iteration,
  not an error.
- **Sixty-fourth consecutive deferral of the Channel Activation
  Milestone** — intentional hold.

## Direction after Phase 175

- **Phase 176 — loop cost control:** the token-budget per-run
  cap (sum the run's `TriggerSource::Loop` usage from the audit
  chain; stop over budget) — the last cap in the trio.
- **Web UI loop pane** (backlog + iterations + gate/cap state +
  progress log + stop control).
- The post-172 roster carries forward: LLM-judged correction
  classification, tool/topic surfacing in `OutcomeSummary`,
  cryptographic PRNG, PDF full-compression page count, and the
  Channel Activation Milestone.

## Prediction vs reality

| Prediction | Reality | Held? |
| --- | --- | --- |
| DESIGN.md HOLD → 12 | Untouched (one new scope via the A3 amendment-file process; channel-tier tool; 13-tool core untouched) | ✅ |
| PRODUCT.md HOLD → 66 | Untouched | ✅ |
| `aivyx-core/src/lib.rs` HOLD → 12 | Untouched | ✅ |
| Zero new workspace deps | Reused the memory substrate, the loop-tool pattern, the driver's prompt assembly — no new storage domain, no new dep | ✅ |
| Zero clippy warnings | `cargo clippy --workspace --all-targets -- -D warnings` clean | ✅ |
| Test count delta `+12` to `+22` | **`+11`** (loop.note 3 + config 1 + driver render/read 6 + cli 1); workspace ~4,088 → ~4,099 | ❌ **one below band** |

**Honest miss (by one).** Predicted the tightened `+12..+22`,
landed `+11` — one shy. The Phase 174 retro correctly called
that loop phases land low, but `+12..+22` was still slightly
high: the testable surface here is exactly the tool + the two
pure helpers (`render_progress_block` / `build_iteration_prompt`)
+ the `read_progress_notes` read-path + config, while the
injection's effect on a real turn is again integration glue over
`TriggerDispatch` (covered transitively). The empirical band for
these glue-heavy loop phases is now clearly **`+10..+16`** — a
better prior than I've been using.

The capability closed end-to-end:

1. **`loop.note` tool + scope** (Task 2). A channel-tier tool
   owning the reserved `loop:progress` memory topic; new
   `loop.note` Trusted-tier base (A3 71 → 72).
2. **Render + config + prompt** (Task 3).
   `[loop].progress_inject_count` (default 20; 0 disables); pure
   `render_progress_block` (oldest-first, blank-skipping) +
   `build_iteration_prompt`; `LOOP_SYSTEM_PROMPT` now names
   `loop.note`.
3. **Driver injection** (Task 4). `run_loop_driver` reads the
   last-N notes (`read_progress_notes`, best-effort) and fires
   each iteration with the progress block prepended.
4. **Surface** (Task 5). `aivyx loop log [--limit N]` over a new
   IPC pair; INSTALL section.

### Honest-debt status carried forward (Phase 176)

- **Token-budget per-run cap** — the last cap in the trio
  (max_iterations + wall-clock + spend).
- **Last-N by recency, not relevance**; **no de-duplication**;
  **the agent must call `loop.note`** — all documented at entry,
  all unchanged.
- Sixty-fourth consecutive deferral of the Channel Activation
  Milestone.

### The result

The loop now does the thing that makes Ralph work: it
**accumulates knowledge across iterations**. A learning recorded
in iteration 3 is unconditionally in front of iteration 4's
fresh context — durably, across runs — so a long backlog gets
*easier* as it goes instead of re-paying the same discovery
cost every turn. No new dependency, no broken streak.
