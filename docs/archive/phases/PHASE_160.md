# Phase 160 — Throttle the Drive Walk: `walk_folder_tree` max_concurrent

**Phase 157 close-out, deferred 2 phases.**
Phase 157 shipped level-parallel BFS in
`walk_folder_tree` with one documented honest-
debt: **the parallel fan-out is unthrottled.** A
10-wide level fires 10 simultaneous
`/files?q=...` queries; a 50-wide level fires
50. Rate-limited operators see 429s; even
well-behaved deployments add unnecessary peak
load at the Google quota edge.

Phase 160 adds the throttle. Single honest-debt,
not a bundle. Symmetric to Phase 155 adding
`max_concurrent` to `calendar.upcoming` after
Phase 151 shipped its parallel fan-out.

## Why this, why now

- **Phase 157 was the freshest unbundled
  honest-debt.** Phase 158 closed Phase 155's
  three; Phase 159 shipped a brand-new tool;
  Phase 160 closes Phase 157's outstanding
  throttle gap before it stales further.

- **Pattern already validated.** Phase 155 used
  the exact same `tokio::sync::Semaphore` shape
  on `calendar.upcoming`. Phase 160 lifts the
  pattern into `aivyx-drive` via the
  `walk_folder_tree` substrate.

- **Zero new workspace deps.** `tokio::sync` is
  already a `tokio` workspace feature; no
  manifest change required.

## Tasks

1. **Open doc + ROADMAP + README.** This doc +
   the roadmap section + the README row.
   Backfill Phase 159 hash to `2fc8bca`.

2. **`walk_folder_tree` max_concurrent
   semaphore.** Add a
   `max_concurrent: Option<usize>` param to the
   `walk_folder_tree` substrate. Within each
   depth level, wrap each `children_of` future
   with a semaphore-permit acquisition so at
   most N requests run simultaneously. When the
   caller passes `None`, permits =
   `current_level.len()` (equivalent to
   unlimited — every future holds a permit at
   once, matching pre-Phase-160 behavior).

3. **recent_files + recent_changes
   `walk_max_concurrent` input.** Add the
   operator-facing knob on both tools. Cap 32
   (above 16 — Drive's typical per-second
   quota tolerates a bit more concurrency than
   the calendar-side `max_concurrent` cap of
   none, but a documented bound keeps the
   blast radius explicit). Validation via the
   existing `parse_recursive_cap` substrate
   pattern. None = unchanged unlimited.

4. **INSTALL + exit + Frozen.** INSTALL.md
   `drive.recent_files` and
   `drive.recent_changes` rows pick up the new
   knob; exit doc with prediction-vs-reality;
   README + ROADMAP Frozen flip.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment; substrate param + additive input
  fields. Streak: 50 → **51**.
- **PRODUCT.md** — **Will hold.** Streak:
  50 → **51**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 160 work in `aivyx-drive`. Streak:
  25 → **26**.

## Exit criteria

- [ ] `docs/PHASE_160.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `walk_folder_tree` accepts optional
  `max_concurrent: Option<usize>` and wraps
  per-folder futures in a Semaphore — Task 2.
- [ ] `walk_max_concurrent` input honored on
  both recent_* tools — Task 3.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+4` to `+10`.

## Honest scope risks at sign-off

- **No `min_concurrent` companion.** Phase 158
  added `min_concurrent` to
  `calendar.upcoming` — Phase 160 ships only
  `max_concurrent` for the drive walk. The
  drive-side use case for a floor knob is even
  less surfaced than the calendar-side one.
  Phase 161+ candidate if asked.

- **No early-abort on permit starvation.**
  When `max_concurrent` is small (e.g. 2) and
  `max_folders` is large (e.g. 1000), the walk
  can take many minutes. No timeout; the caller
  must accept the latency or pick a smaller
  `recursive_max_folders` (Phase 157 knob).

- **`walk_max_concurrent` upper bound 32.**
  Drive's per-user-per-second quota is
  ~1000 req/s; 32 concurrent reqs is well
  under that. Operators with corner-case
  quota ceilings can pick smaller values.

- **Forty-ninth consecutive deferral of the
  Channel Activation Milestone.** Per operator
  framing — intentional hold.

## Direction after Phase 160

After Phase 160, Phase 157's last carry-over
clears. Phase 161+ candidates:

1. **drive.recent_activity actor / target
   filters.** (Phase 159 carry-over.)
2. **drive.recent_activity consolidation knob.**
   (Phase 159 carry-over.)
3. **drive.recent_activity + parent_folder_id
   composition.** (Phase 159 carry-over.)
4. **Operator-tunable image size cap.**
5. **URL fetch timeout.**
6. **HEAD pre-fetch for size check.**
7. **Authenticated URL fetch.**
8. **PDF / SVG / TIFF media type support.**
9. **Mid-recording or mid-reply /image
   command.**
10. **Clipboard-based image source.**
11. **Voice abort UX knob.**
12. **Silero ONNX VAD.**
13. **Streaming ASR.**
14. **Wake-word activation.**
15. **macOS streaming variant.**
16. **Lock-free AudioIn detector.**
17. **calendarList cache TTL knob.**
18. **drive walk min_concurrent companion.**
19. **access_role deprecation.**
20. **Budget category migration tool.**
21. **Budget currency / rust_decimal.**
22. **Multi-category trend breakdown.**
23. **Trend smoothing / moving average.**
24. **Bulk budget operations.**
25. **Proactive reminder dispatch.**
26. **Relative-time localization.**
27. **whisper-cpp-plus rehabilitation.**
28. **`build_agent_stack` substrate-tier
    promotion.**
29. **Channel Activation Milestone** —
    still held intentionally; 49th
    consecutive deferral at Phase 160
    open.

## Prediction vs reality

| Prediction | Reality | Held? |
| --- | --- | --- |
| DESIGN.md HOLD → 51 | Untouched | ✅ |
| PRODUCT.md HOLD → 51 | Untouched | ✅ |
| `aivyx-core/src/lib.rs` HOLD → 26 | Untouched | ✅ |
| Zero new workspace deps | `tokio::sync::Semaphore` already in workspace feature set; no manifest change | ✅ |
| Zero clippy warnings | `cargo clippy --workspace --all-targets -- -D warnings` clean | ✅ |
| Test count delta `+4` to `+10` | `+10` (228 → 238 in `cargo test -p aivyx-drive --lib`) | ✅ (top of band) |

All five exit criteria met. Phase 157's
remaining honest-debt is now closed:

1. **Substrate throttle** (Task 2, commit
   `42050fd`). `walk_folder_tree` accepts
   `max_concurrent: Option<usize>` and gates
   each per-folder `children_of` future on a
   `tokio::sync::Semaphore` permit
   acquisition. None preserves the pre-Phase-
   160 unlimited fan-out byte-identically.
2. **Operator-facing knob** (Task 3, commit
   `bc1cb39`). `drive.recent_files` and
   `drive.recent_changes` both gain a
   `walk_max_concurrent` input (cap 32),
   validated via the shared
   `parse_recursive_cap` substrate from Phase
   157.

### What landed beyond the open

Nothing. Test count landed exactly at the top
of the predicted `+4..+10` band — the
substrate +3 + per-tool input parsing +7 add
up to 10 cleanly.

### Phase 157 honest-debt status — all clear

The Phase 157 open doc listed three honest-
debts at sign-off. Phase 158/159 close-out
inventory marks them all closed:

- ✅ Sequential walk → parallel level-BFS
  (Phase 157 itself, Task 2).
- ✅ Walk uses operator's default corpus, not
  drive_id scope (Phase 157 itself, Task 2).
- ✅ Hardcoded max_depth / max_folders (Phase
  157 itself, Task 3 — operator-tunable
  inputs).

Phase 157's *carry-over* (parallel fan-out
unthrottled) — flagged at exit, then closed
in Phase 160. Two-phase pickup matches the
Phase 153 → 157 cadence.

### Forty-ninth deferral of Channel Activation Milestone

Per operator framing — intentional hold. Recorded
for the record.
