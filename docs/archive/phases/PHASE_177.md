# Phase 177 — Loop Loose-Ends Bundle (operator polish)

**A loose-ends bundle, in the Phase 171 mould.** The autonomous
loop arc (Phase 173–176) is feature-complete — three caps, gate
verification, the progress log — but four phases of fast-moving
work left a few small operator-facing rough edges. Phase 177
bundles the three highest-value ones into one focused phase. No
new substrate, no new capability scope, no new tool — just
polish.

## The three touch-ups

### 1. Live `tokens used` in `aivyx loop status`

Phase 176 added the `max_run_tokens` budget, and the driver
already computes the run-window token total
(`read_run_tokens`) every iteration to feed `decide()` — but it
**throws the number away** after the decision. An operator
watching a run with a budget cap wants to see *how close* it
is. This surfaces it: the driver records the latest total into
`LoopRunState`, which `aivyx loop status` already returns, so
the status renders `tokens used: N` (and `N / cap` when a
budget is set) for the current or last run.

### 2. `aivyx loop skip <story-id>`

Today the agent can `loop.complete` a story but nobody can
**skip** one: a stuck or no-longer-wanted backlog story sits
`Pending` forever, and the loop will keep re-attempting it. The
backlog substrate has had `mark_skipped` since Phase 173 — it
just was never exposed. This adds the operator command
(`LoopSkip` IPC → `mark_skipped` → reuse the `LoopControl`
response), so operators can prune the backlog without editing
storage by hand.

### 3. Progress-note de-duplication

Phase 175 flagged it at sign-off: the progress log has **no
de-duplication**, so an agent that records the same learning on
two iterations fills the injected `## Progress so far` block
with repeats. This makes `loop.note` skip a write whose text
exactly matches the most-recent progress note (trimmed), and
report it as deduped — cheap, and it keeps the injected context
clean over a long run.

## Tasks

1. **Open doc + README.** This doc, README Active row, backfill
   Phase 176's frozen hash (`2857c06`).

2. **Live tokens-used.** Add `tokens_used: u64` to
   `LoopRunState`; the driver records the run-window total each
   iteration; reset to `0` on `request_start`. `aivyx loop
   status` renders it. (Flows through the existing `LoopStatus`
   IPC for free — `state` already carries the whole
   `LoopRunState`.)

3. **`aivyx loop skip <id>`.** New `LoopSkip { story_id }` IPC
   query → handler calls `PersistentLoopBacklog::mark_skipped`,
   reusing the `LoopControl { ok, message }` response. Client
   fn + `aivyx loop skip <story-id>` CLI subcommand + help.

4. **Progress-note de-dup.** `LoopNoteTool` reads the
   most-recent progress note before writing; an exact (trimmed)
   match is skipped and reported (`{ "noted": false,
   "deduped": true }`). Tests pin the dedup + the
   distinct-note-still-writes path.

5. **INSTALL + exit + Frozen.** INSTALL notes for the three
   touch-ups; exit doc with prediction-vs-reality; README
   Frozen flip.

## Streak predictions

- **DESIGN.md** — **Will hold.** Streak: 13 → **14**. No new
  capability scope and no new tool — `loop skip` is an operator
  IPC command (like `loop start/stop`), dedup is a behaviour
  tweak to an existing tool, tokens-used is a struct field. No
  A3 amendment; the thirteen-tool core untouched.
- **PRODUCT.md** — **Will hold.** Streak: 67 → **68**. Polish
  on an existing capability.
- **`aivyx-core/src/lib.rs`** — **Will hold.** Streak: 13 →
  **14**. Work lands in `aivyx-channel`; `aivyx-core` untouched.

## Exit criteria

- [ ] `docs/PHASE_177.md` + README row + Phase 176 backfill —
  Task 1.
- [ ] `LoopRunState.tokens_used` recorded by the driver +
  rendered in `aivyx loop status` — Task 2.
- [ ] `aivyx loop skip <id>` marks a pending story skipped —
  Task 3.
- [ ] `loop.note` de-dups an exact-match most-recent note —
  Task 4.
- [ ] DESIGN.md / PRODUCT.md / `aivyx-core/src/lib.rs` HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+8` to `+14`. *(Using the refined
  loop-polish band from the Phase 176 retro — a bundle of three
  small CLI/behaviour touch-ups, each with a couple of tests.)*

## Honest scope risks at sign-off

- **`tokens used` is the same window-sum as the cap** (Phase
  176): it counts all turns during the run, not loop-only.
  Surfacing it inherits that framing — loop-only attribution is
  still a future refinement.
- **Dedup is most-recent-only.** A learning recorded, then a
  different one, then the first again *will* duplicate — only
  back-to-back repeats are caught. A full-history dedup would
  be O(n) per note; most-recent catches the common case (an
  agent re-stating the same thing in consecutive iterations).
- **`loop skip` skips, it doesn't delete.** The story stays in
  the HMAC-chained backlog as `Skipped` (the chain is
  append-only); it just stops being `Pending`. Consistent with
  the substrate's audit posture.
- **Sixty-sixth consecutive deferral of the Channel Activation
  Milestone** — intentional hold.

## Direction after Phase 177

With the loop arc complete + polished, the natural pivots are
the remaining loop-precision items (loop-only token
attribution, a cost model, a Web UI loop pane) **or** branching
back to the older roster: LLM-judged correction classification,
tool/topic surfacing in `OutcomeSummary`, cryptographic PRNG,
PDF full-compression page count, and the long-deferred
**Channel Activation Milestone** (66 deferrals).

## Prediction vs reality

| Prediction | Reality | Held? |
| --- | --- | --- |
| DESIGN.md HOLD → 14 | Untouched (no new scope, no new tool) | ✅ |
| PRODUCT.md HOLD → 68 | Untouched | ✅ |
| `aivyx-core/src/lib.rs` HOLD → 14 | Untouched | ✅ |
| Zero new workspace deps | All three touch-ups reused existing primitives | ✅ |
| Zero clippy warnings | `cargo clippy --workspace --all-targets -- -D warnings` clean | ✅ |
| Test count delta `+8` to `+14` | **`+6`** (tokens-used 1 + skip 4 + dedup 1); workspace ~4,104 → ~4,110 | ❌ **below band — but within my own refined prior** |

**The prediction lesson, now landing.** Phase 176's retro
*explicitly* set the loop-polish band at **`+5..+12`**. Then in
this open doc I wrote **`+8..+14`** — drifting the band back up
again — and landed `+6`, which is squarely in the `+5..+12`
prior I'd just established but below the band I actually
predicted. This is the third loop-follow-on phase I've
over-predicted (175: +11 vs +12–22; 176: +5 vs +10–16; 177: +6
vs +8–14). The fix is simple and I'm committing to it: **loop
touch-up / cap / polish phases get `+5..+12`, full stop** — no
upward drift "because this one feels bigger."

The bundle landed all three touch-ups:

1. **Live `tokens used`** (Task 2). `LoopRunState.tokens_used`,
   recorded by the driver each iteration, rendered by `aivyx
   loop status` (`N / cap` with a budget set).
2. **`aivyx loop skip <id>`** (Task 3). Exposes the backlog's
   `mark_skipped` over a new `LoopSkip` IPC + CLI; the story
   stays in the append-only chain as `Skipped`.
3. **Progress-note de-dup** (Task 4). `loop.note` skips an exact
   repeat of the most-recent note.

### Honest-debt status carried forward

- **`tokens used` inherits the window-sum framing** (all turns
  during the run, not loop-only).
- **Dedup is most-recent-only** (back-to-back repeats only).
- **`loop skip` skips, doesn't delete** (append-only chain).
- All three were flagged at entry; none changed.
- Sixty-sixth consecutive deferral of the Channel Activation
  Milestone.

### The result

The autonomous-loop arc (173–177) is now complete *and*
polished: three caps with live spend visibility, gate
verification, the cross-iteration progress log (de-duplicated),
and full operator backlog control (add / list / skip / start /
stop / status / log) — all built from existing substrate, with
no new workspace dependency and an unbroken DESIGN / PRODUCT /
`lib.rs` streak across the entire five-phase arc.
