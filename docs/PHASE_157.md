# Phase 157 — Drive Recursive Walk Close-Out Bundle (Parallel + drive_id Scope + Tunable Caps)

**Phase 153 close-out, deferred 4 phases.**
Phase 153 shipped recursive folder filter via
`walk_folder_tree` with three documented honest-
debts:

1. **Sequential walk.** 100-folder trees took
   ~30-60 seconds in series. Phase 154+
   parallel-via-`tokio::join_all` candidate.
2. **Walk uses operator's default corpus, not
   drive_id scope.** Operators wanting "recursive
   recent in *this* Team Drive's /Projects folder"
   couldn't combine recursive + drive_id.
3. **`max_depth=5` and `max_folders=100` hardcoded.**
   Operators with deeper or wider trees hit the
   cap with no recourse.

Phase 157 closes all three in one phase.
Symmetric to Phase 148/151/152/153/155/156
bundle pattern.

## Why this, why now

- **Phase 153 was last touched 4 phases ago.**
  Same gap as Phase 155 closing Phase 151's
  follow-ons.

- **All three small individual scopes natural
  together.** Parallel walk via level-BFS
  + join_all (Phase 151's pattern). drive_id
  param threads through one extra arg.
  Tunable caps are optional input fields.

- **Builds on Phase 151 + 153 substrate.**
  futures-util is already an aivyx-calendar
  dep (Phase 151) AND just got added to
  aivyx-drive in Task 2 here. The walk_folder_tree
  + tools/mod.rs helper layer is exactly
  where the parallelization belongs.

- **Zero new workspace deps.** futures-util
  already in workspace.

## Tasks

1. **Open doc + ROADMAP + README** — this doc +
   the roadmap section + the README row.
   Backfill Phase 156 hash to `302c142`.

2. **Parallel walk_folder_tree via level-BFS.**
   Replace the sequential VecDeque-based BFS
   with level-parallel BFS:
   - At each depth, fire all per-folder
     children-queries via
     `futures_util::future::join_all`.
   - Collect all child folder IDs (subject to
     `max_folders` cap).
   - Advance to next depth (or stop if
     `max_depth` hit or `max_folders`
     reached).
   - On any per-folder query error, propagate
     up — the whole walk fails with the
     erroring folder's identifier.
   futures-util added as crate dep to
   aivyx-drive (already workspace dep). Tests
   on the level-BFS bookkeeping (no live HTTP);
   the actual fan-out is operator-validation
   tier.

3. **drive_id scope + tunable caps.** Two
   follow-ons in one task:
   - `walk_folder_tree` gains optional
     `drive_id: Option<&str>` param. When
     present, each per-folder child query
     appends `corpora=drive` + `driveId` +
     `includeItemsFromAllDrives=true` +
     `supportsAllDrives=true` to the API
     call. recent_files + recent_changes
     execute() thread `parsed.drive_id`
     through.
   - recent_files + recent_changes input
     schemas gain optional
     `recursive_max_depth: u32` (cap 20) and
     `recursive_max_folders: u32` (cap 1000).
     When present, override the
     `RECURSIVE_MAX_DEPTH` /
     `RECURSIVE_MAX_FOLDERS` constants.
   Tests cover input parsing + clamps.

4. **INSTALL + exit + Frozen.** INSTALL.md
   drive section: recent_files + recent_changes
   rows note the new knobs + the drive_id-
   scoped recursive walk. Phase 157 exit doc
   with prediction-vs-reality. README +
   ROADMAP flip Phase 157 to Frozen.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment; substrate refactor + additive
  input fields. Streak: 47 → **48**.
- **PRODUCT.md** — **Will hold.** Streak:
  47 → **48**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 157 work in `aivyx-drive`. Core
  untouched. Streak: 22 → **23**.

## Exit criteria

- [ ] `docs/PHASE_157.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `walk_folder_tree` uses level-BFS via
  `join_all` — Task 2.
- [ ] `walk_folder_tree` accepts optional
  `drive_id` param + threads through to
  child-folder queries — Task 3.
- [ ] `recursive_max_depth` +
  `recursive_max_folders` inputs honored on
  recent_* — Task 3.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+8` to `+14`.

## Honest scope risks at sign-off

- **Parallel level-BFS increases peak API
  load.** A 10-wide level fires 10
  simultaneous requests; rate-limited
  operators might surface 429s. Phase 158+
  candidate: max_concurrent throttle on
  the walk (matches Phase 155's
  calendar.upcoming pattern).

- **No early-abort on cap-hit mid-level.**
  When a level's queries are firing in
  parallel, all complete before the
  max_folders cap is enforced. The final
  count may briefly exceed the cap, then
  gets trimmed. Acceptable; documented.

- **`recursive_max_depth` upper bound 20.**
  Operators with deeper hierarchies hit the
  upper cap. 20 levels covers >99% of
  realistic project structures.

- **`recursive_max_folders` upper bound
  1000.** Operators with very wide trees
  hit the upper cap. The combined
  `20 * 1000` worst case sends 1000 API
  calls; at parallel rate this is ~few
  seconds. Phase 158+ candidate for harder
  caps if surfaces.

- **`drive_id` scope on the walk requires
  the folder hierarchy to also be in the
  same drive.** When operator passes
  `drive_id` + `parent_folder_id` + `recursive`,
  Phase 157 assumes the parent_folder_id
  is also in the drive. If not, the walk
  yields nothing.

- **Forty-sixth consecutive deferral of the
  Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 157

After Phase 157, Phase 153's three carry-overs
clear. Phase 158+ candidates:

1. **max_concurrent throttle on
   walk_folder_tree** (matches Phase 155
   calendar pattern).
2. **Operator-tunable image size cap.**
3. **URL fetch timeout.**
4. **HEAD pre-fetch for size check.**
5. **Authenticated URL fetch.**
6. **PDF / SVG / TIFF media type support.**
7. **Mid-recording or mid-reply /image
   command.**
8. **Clipboard-based image source.**
9. **Sliding-window fuzzy time** for
   adjacent-bucket merging.
10. **calendarList session caching.**
11. **min_concurrent knob.**
12. **Voice abort UX knob.**
13. **Silero ONNX VAD.**
14. **Streaming ASR.**
15. **Wake-word activation.**
16. **macOS streaming variant.**
17. **Lock-free AudioIn detector.**
18. **access_role deprecation.**
19. **Budget category migration tool.**
20. **Budget currency / rust_decimal.**
21. **Multi-category trend breakdown.**
22. **Trend smoothing / moving average.**
23. **Bulk budget operations.**
24. **Drive Activity API.**
25. **Proactive reminder dispatch.**
26. **Relative-time localization.**
27. **whisper-cpp-plus rehabilitation.**
28. **`build_agent_stack` substrate-tier
    promotion.**
29. **Channel Activation Milestone** —
    still held intentionally; 46th
    consecutive deferral at Phase 157
    open.

## Prediction vs reality

_Populated at Phase 157 exit. Predictions at
sign-off: DESIGN.md HOLD → 48; PRODUCT.md HOLD
→ 48; lib.rs HOLD → 23; zero new deps; test
count delta `+8` to `+14`; zero clippy
warnings._
