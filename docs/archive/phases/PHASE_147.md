# Phase 147 — Chapter G: `health.check.remove`

**Phase 125 final candidate close-out.** Phase
125 framed three Chapter G #2 candidates:
1. **Calendar reminders** ✓ — Phases 141 + 142.
2. **Budget tracking** ✓ — Phases 143 + 144.
3. **`health.check.remove` + alert dispatch** —
   shipping the `.remove` half in Phase 147.
   Alert dispatch deferred (would bump into
   the Channel Activation Milestone).

After Phase 147, the Phase 125 framing is
fully resolved. The remaining "alert dispatch"
piece is intentionally held alongside Channel
Activation; the agent can already compose
alerts via the recipe Phase 125 documented
(read `health.check.recent_changes` + dispatch
via `notify.send`).

## Why this, why now

- **Closes the Phase 125 list.** Three
  candidates framed; three implementations
  shipped (with the alert-dispatch piece
  intentionally held). Satisfying closure on a
  multi-phase narrative.

- **Smallest meaningful scope.** One substrate
  method + one tool. Mirrors the
  `budget.delete` and `calendar.delete_event`
  shape — idempotent, returns
  `was_already_removed: true` for missing
  names.

- **Builds on existing substrate.** Phase 125
  shipped HealthStore with add_watcher /
  list_watchers / recent_transitions_within /
  due_watchers / next_check_in / record_check.
  remove_watcher is the same pattern in
  reverse: take the lock, mutate the
  watchers Vec + states HashMap, persist via
  the existing save_to_disk wrapper.

- **Zero new workspace deps.** Pure addition.

## Tasks

1. **Open doc + ROADMAP + README** — this doc +
   the roadmap section + the README row.
   Backfill Phase 146 hash to `034ff01`.

2. **`HealthStore::remove_watcher`.** New
   method:
   ```rust
   pub async fn remove_watcher(
       &self,
       name: &str,
   ) -> Result<RemoveOutcome, HealthStoreError>;
   ```
   Idempotent: returns
   `RemoveOutcome { name, was_already_removed }`.
   On match: removes from `watchers` Vec and
   from `states` HashMap; persists. On no-match:
   returns `was_already_removed: true` without
   touching disk (no write needed). Tests cover
   present-watcher + missing-watcher +
   persistence across reopen + state-map
   cleanup.

3. **`health.check.remove` tool.** New
   `HealthCheckRemove` in
   `tools/health_check.rs`:
   - Input: `{name: String}`.
   - Output: `{name, was_already_removed}`.
   - Capability: `health.write`.
   - Idempotent shape matching `budget.delete`
     and `calendar.delete_event`.
   - Input parsing tests.

4. **Main.rs wire + INSTALL + exit + Frozen.**
   `main.rs` registers `HealthCheckRemove`
   (toolkit harness 12 → 13 tools).
   `tools/mod.rs` + lib.rs re-export.
   INSTALL.md health-monitoring section gains
   the new row + Phase 125 candidate-list
   closure note. Phase 147 exit doc with
   prediction-vs-reality.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment. Streak: 37 → **38**.
- **PRODUCT.md** — **Will hold.** Streak:
  37 → **38**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 147 work in `aivyx-toolkit`. Core
  untouched. Streak: 12 → **13**.

## Exit criteria

- [ ] `docs/PHASE_147.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `HealthStore::remove_watcher` public +
  tested — Task 2.
- [ ] `HealthCheckRemove` tool public +
  registered + tested — Task 3.
- [ ] Toolkit harness 12 → 13 tools — Task 4.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+4` to `+8`
  (remove_watcher substrate ~3 tests +
  tool input parsing ~2 tests).

## Honest scope risks at sign-off

- **No bulk-remove.** Per-name only.
  Operators wanting "remove all watchers"
  do one tool call per. Phase 148+ if
  surfaces.

- **Alert dispatch still deferred.** The
  remaining Phase 125 piece — agent
  proactively sending notify.send when
  health.check.recent_changes reports a
  flip — needs scheduled-event runner +
  channel-aware delivery. Bumps into
  Channel Activation. Intentional hold.

- **No "soft-delete with restore" UX.**
  Removed watchers are gone from disk.
  No undo. Standard delete posture; same
  as budget.delete + calendar.delete_event.

- **State map cleanup may be the
  regression boundary.** The watchers Vec
  and states HashMap are parallel; both
  must stay in sync. The remove_watcher
  test that re-opens the store + verifies
  both are gone is the regression test.

- **Thirty-sixth consecutive deferral of
  the Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 147

After Phase 147, the Phase 125 framing is
fully resolved. Phase 148+ candidates:

1. **Aggressive voice abort** — drop cpal
   stream for instant silence (Phase 146
   carry-over).
2. **Partial-text preservation on voice
   abort.**
3. **Silero ONNX VAD.**
4. **Streaming ASR.**
5. **Wake-word activation.**
6. **Multimodal output** — voice + vision.
7. **macOS streaming variant.**
8. **Lock-free AudioIn detector.**
9. **VAD config validation.**
10. **Drive Activity API** — true edit
    history.
11. **drive.list_drives** — shared drives
    enumeration.
12. **Drive parent_folder_id filter on
    recent_*.**
13. **Category whitelist + case-fold for
    budget.**
14. **Budget currency / rust_decimal.**
15. **`budget.trend`** — month-over-month.
16. **Bulk budget operations.**
17. **Proactive reminder dispatch** — the
    big architectural step, intentionally
    held with Channel Activation.
18. **Phase 142 debt cleanup.**
19. **Relative-time localization.**
20. **whisper-cpp-plus rehabilitation.**
21. **`build_agent_stack` substrate-tier
    promotion** if more channel adapters
    ship.
22. **Channel Activation Milestone** —
    still held intentionally; 36th
    consecutive deferral at Phase 147
    open.

## Prediction vs reality

**Three-of-three streak HOLDs as predicted.**

- **DESIGN.md** — HELD as predicted (`62dabbdd…`
  unchanged). No contract amendment. Streak:
  37 → **38**.
- **PRODUCT.md** — HELD as predicted (`467ba59a…`
  unchanged). Streak: 37 → **38**.
- **`aivyx-core/src/lib.rs`** — HELD as predicted
  (`4f9b8c81…` unchanged). All Phase 147 work in
  `aivyx-toolkit`. Continuing post-Phase-135
  reset: 12 → **13**.

**Test count delta: +7 — within predicted `+4`
to `+8` range.** Workspace lib tests 3152 →
3159. Per-module:
- `health_store`: +4 (present → was_already
  false + list confirms; missing →
  was_already true idempotent; persist across
  reopen + double-remove idempotent; state-
  map cleanup in sync — re-add starts fresh).
- `tools::health_check`: +3 (remove_schema
  requires name; remove input extracts name;
  remove input rejects missing name).

**Zero new workspace dependencies** as
predicted.

**Zero clippy warnings** with default features.

### What landed cleanly + what bent

**Cleanly:**
- `HealthStore::remove_watcher` idempotent
  substrate method with parallel-structure
  cleanup (watchers Vec + states HashMap).
- `RemoveOutcome { name, was_already_removed }`
  re-exported from lib.rs alongside other
  store types.
- `HealthCheckRemove` tool with idempotent
  output shape matching `budget.delete` +
  `calendar.delete_event`.
- Module-level doc comment in
  `tools/health_check.rs` updated to drop the
  old "deferred" note for `health.check.remove`.
- `main.rs` registers the new tool; toolkit
  harness 12 → 13 tools.
- INSTALL.md health-monitoring section gains
  the new tool docs + Phase 125 closure note.
- 3159 workspace lib tests pass; clippy clean.

**Bent honestly:**

1. **No bulk-remove.** Per-name only. Phase
   148+ if surfaces.

2. **Alert dispatch still deferred.** The
   remaining Phase 125 piece — agent
   proactively sending notify.send when
   health.check.recent_changes reports a flip
   — needs scheduled-event runner + channel-
   aware delivery. Intentionally held alongside
   Channel Activation Milestone.

3. **No soft-delete with restore.** Standard
   delete posture; same as the other delete-
   style tools.

4. **No no-op detection at the tool layer.**
   was_already_removed surfaces verbatim from
   the substrate; the agent reads the flag
   and paraphrases. Acceptable.

### Direction after Phase 147

After Phase 147, the Phase 125 framing is
fully resolved. Phase 148+ candidates:

1. **Aggressive voice abort** — drop cpal
   stream for instant silence.
2. **Partial-text preservation on voice
   abort.**
3. **Silero ONNX VAD.**
4. **Streaming ASR.**
5. **Wake-word activation.**
6. **Multimodal output.**
7. **macOS streaming variant.**
8. **Lock-free AudioIn detector.**
9. **VAD config validation.**
10. **Drive Activity API.**
11. **drive.list_drives.**
12. **Drive parent_folder_id filter on
    recent_*.**
13. **Category whitelist for budget.**
14. **Budget currency / rust_decimal.**
15. **`budget.trend`.**
16. **Bulk budget operations.**
17. **Proactive reminder dispatch** — the
    big architectural step.
18. **Phase 142 debt cleanup.**
19. **Relative-time localization.**
20. **whisper-cpp-plus rehabilitation.**
21. **`build_agent_stack` substrate-tier
    promotion** if more channel adapters
    ship.
22. **Channel Activation Milestone** —
    still held intentionally; 36th
    consecutive deferral at Phase 147
    exit.
