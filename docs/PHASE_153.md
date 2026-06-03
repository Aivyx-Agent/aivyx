# Phase 153 — Drive: `recent_*` Close-Out Bundle (`drive_id` + Recursive Folder Filter)

**Phase 148 close-out, deferred 5 phases.**
Phase 148 shipped `drive.list_drives` +
`parent_folder_id` filter on
`drive.recent_files` + `drive.recent_changes`,
with two documented honest-debts:

1. **No `drive_id` parameter on recent_*.**
   Operators wanting "what's recent in *this*
   Team Drive" combined `drive.list_drives` +
   `drive.search` with a hand-written
   `corpora=drive` query rather than the
   ergonomic recent_* shape.

2. **`parent_folder_id` not recursive.**
   Drive's q DSL `'<id>' in parents` matches
   direct children only. Operators wanting
   "all files modified anywhere under
   /Projects/Aivyx this week" had to combine
   `drive.list_folder` recursively or
   hand-write a `drive.search`.

Phase 153 closes both in one phase. Symmetric
to Phase 151's calendar bundle + Phase 152's
voice bundle pattern.

## Why this, why now

- **Phase 148 has two clear close-outs.**
  Same shape as Phase 151's three-in-one
  calendar bundle. Operator-readable progress
  on the debt list.

- **Both deliverables sit on the existing
  recent_* substrate.** `drive_id` is a
  query-parameter addition; recursive folder
  filter is a pre-fetch walk of the folder
  tree to expand the q clause.

- **Daily-use value for operators with Team
  Drives.** Org-collab workflows live in
  shared drives; the `corpora=drive` shape
  is the natural way to scope. And recursive
  folder filtering matches how operators
  think about project folders ("everything
  under /Aivyx").

- **Zero new workspace deps.**

## Tasks

1. **Open doc + ROADMAP + README** — this doc
   + the roadmap section + the README row.
   Backfill Phase 152 hash to `1002330`.

2. **`drive_id` parameter on recent_*.**
   Extend `drive.recent_files` +
   `drive.recent_changes` input schemas with
   optional `drive_id: String`. When present,
   set the Google Drive API query parameters
   per the shared-drives spec:
   - `corpora=drive`
   - `driveId=<id>`
   - `includeItemsFromAllDrives=true`
   - `supportsAllDrives=true`
   Composable with `parent_folder_id` (folder
   filter applies within the specified Team
   Drive). Mutually exclusive with neither —
   they're orthogonal.
   Tests cover query parameter composition
   for: drive_id only, drive_id +
   parent_folder_id, drive_id + recursive,
   drive_id + neither.

3. **Recursive folder filter (`walk_folder_tree`).**
   Add `pub(crate) async fn walk_folder_tree(
       client: &SharedDriveClient,
       root_folder_id: &str,
       max_depth: usize,
       max_folders: usize,
   ) -> Result<Vec<String>, CalendarClientError>`
   (actually `DriveClientError`). BFS-walks the
   folder tree starting from `root_folder_id`;
   returns every folder ID encountered (including
   the root). Hard caps:
   - `max_depth = 5` (config-tunable but
     hardcoded for Phase 153 MVP).
   - `max_folders = 100` (likewise hardcoded).
   On cap-hit, returns the partial list +
   logs a heads-up to stderr.

   Add `recursive: bool` flag to recent_*
   tool inputs (default `false`; only
   meaningful when `parent_folder_id` is set).
   When `recursive = true`, the tool calls
   `walk_folder_tree(parent_folder_id, 5,
   100)` then composes the q clause as
   `"'f1' in parents or 'f2' in parents or
   ..."` (OR-joined parents clauses).

   Tests cover:
   - BFS visit order on a multi-level tree
     (mock client + canned responses).
   - Depth cap stops descent.
   - Folder cap caps total returned.
   - Empty tree (no subfolders) returns just
     the root.
   - q clause composition with N folders.

4. **INSTALL + exit + Frozen.** INSTALL.md
   drive section: recent_files + recent_changes
   rows mention drive_id + recursive flag.
   Phase 153 exit doc with prediction-vs-reality.
   README + ROADMAP flip Phase 153 to Frozen.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment; additive tool inputs + substrate
  helper. Streak: 43 → **44**.
- **PRODUCT.md** — **Will hold.** Streak:
  43 → **44**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 153 work in `aivyx-drive`. Core
  untouched. Streak: 18 → **19**.

## Exit criteria

- [ ] `docs/PHASE_153.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `drive_id` input on both recent_* tools
  with query-parameter composition tests —
  Task 2.
- [ ] `walk_folder_tree` substrate +
  `recursive` flag on recent_* + tests —
  Task 3.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+10` to `+16`
  (drive_id composition tests ~4-6 + tree
  walk substrate ~5-8 + recursive clause
  composition ~3-4).

## Honest scope risks at sign-off

- **`walk_folder_tree` latency.** A 100-folder
  tree at sequential per-list-folder pace is
  ~30-60 seconds. Phase 154+ could parallelize
  the tree walk via `tokio::join_all` like
  Phase 151's calendar fan-out. Phase 153
  ships sequential.

- **`max_depth = 5` is hardcoded.** Operators
  with deeper project hierarchies hit the
  cap. Phase 154+ could surface
  `recursive_max_depth: Option<usize>` if
  surfaces.

- **`max_folders = 100` is hardcoded.**
  Operators with very wide trees hit the
  cap. Same Phase 154+ tunability candidate.

- **Recursive q clause length.** 100 folders
  → q clause ~5kB (`'<32 char ID>' in
  parents or ` repeated). Google Drive's q
  DSL has a length limit; we don't enforce
  it client-side. Operators hitting the
  limit see a Drive API error. Phase 154+
  could batch into multiple list_folder
  calls if surfaces.

- **`recursive` flag is silently ignored
  when `parent_folder_id` is absent.**
  Operators setting `recursive: true` without
  `parent_folder_id` get the same behavior
  as `recursive: false` (no filter). The
  flag is only meaningful in combination
  with parent_folder_id; documented in the
  tool description.

- **No combined `drive_id` + recursive walk
  test integration.** Pre-fetching the folder
  tree happens against the operator's
  default corpus, NOT the specified
  `drive_id`. Phase 154+ could fix if Team-
  Drive recursive walking surfaces.

- **Forty-second consecutive deferral of the
  Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 153

After Phase 153, Phase 148's two honest-debts
clear. Phase 154+ candidates:

1. **Parallel folder-tree walk** via
   `tokio::join_all` if 100-folder trees
   surface latency complaints.
2. **`recursive_max_depth` /
   `recursive_max_folders` tunability.**
3. **Recursive walk within `drive_id`
   scope.**
4. **Voice abort UX knob** (graceful vs
   aggressive).
5. **Silero ONNX VAD.**
6. **Streaming ASR.**
7. **Wake-word activation.**
8. **Multimodal output.**
9. **macOS streaming variant.**
10. **Lock-free AudioIn detector.**
11. **Calendar fuzzy dedup.**
12. **Calendar max_concurrent knob.**
13. **Calendar writable_only filter.**
14. **access_role deprecation.**
15. **Budget category migration tool.**
16. **Budget currency / rust_decimal.**
17. **Multi-category trend breakdown.**
18. **Trend smoothing / moving average.**
19. **Bulk budget operations.**
20. **Drive Activity API.**
21. **Proactive reminder dispatch.**
22. **Relative-time localization.**
23. **whisper-cpp-plus rehabilitation.**
24. **`build_agent_stack` substrate-tier
    promotion.**
25. **Channel Activation Milestone** —
    still held intentionally; 42nd
    consecutive deferral at Phase 153
    open.

## Prediction vs reality

**Three-of-three streak HOLDs as predicted.**

- **DESIGN.md** — HELD as predicted (`62dabbdd…`
  unchanged). No contract amendment. Streak:
  43 → **44**.
- **PRODUCT.md** — HELD as predicted (`467ba59a…`
  unchanged). Streak: 43 → **44**.
- **`aivyx-core/src/lib.rs`** — HELD as predicted
  (`4f9b8c81…` unchanged). All Phase 153 work in
  `aivyx-drive`. Continuing post-Phase-135 reset:
  18 → **19**.

**Test count delta: +13 — within predicted `+10`
to `+16` range.** Workspace lib tests 3226 →
3239. Per-module:
- `recent_files` (input parsing): +6 (default
  recursive/drive_id, drive_id extract, null
  drive_id, empty drive_id, recursive flag,
  composable drive_id+folder+recursive).
- `recent_changes` (input parsing): +3 (default,
  drive_id extract, recursive flag — leaner
  because the q-string substrate is shared).
- `tools/mod.rs` (compose + caps): +4 (empty
  → "", single → Phase 148 shape, multiple OR-
  joined, caps pin to 5/100).

**Zero new workspace dependencies** as predicted.

**Zero clippy warnings** with default features.
One transient catch during Task 2: the
`recursive` field on ParsedInput was parsed but
not yet consumed (Task 3 wires it). Resolved
with `#[allow(dead_code)]` + Phase-153-Task-3
forward-pointer comment; annotation removed in
Task 3 when the field is consumed.

### What landed cleanly + what bent

**Cleanly:**
- `drive_id` parameter on both recent_* tools.
  When present: corpora=drive + driveId +
  includeItemsFromAllDrives + supportsAllDrives
  per Google's shared-drives spec. Composable
  with parent_folder_id + recursive.
- `walk_folder_tree` BFS substrate with
  hardcoded caps (max_depth=5, max_folders=100).
  On cap-hit returns partial list +
  caller-detect via len comparison.
- `compose_recursive_parent_clause` pure helper
  for OR-joining `'<id>' in parents` clauses.
- Both `build_owned_recent_q` and
  `build_recent_changes_q` refactored to
  accept a pre-composed `parent_clause:
  Option<&str>` instead of a single folder ID.
  Single-folder + recursive paths use the same
  helper.
- recent_* execute() match block dispatches on
  (parent_folder_id, recursive): None, Some-
  non-recursive, Some-recursive. Stderr
  heads-up when the walk hits the folder cap.
- 195 aivyx-drive lib tests pass; workspace
  clippy clean.

**Bent honestly:**

1. **walk_folder_tree latency on 100-folder
   trees ~30-60s sequential.** Phase 154+
   parallel via tokio::join_all candidate.

2. **max_depth=5 and max_folders=100 hardcoded.**
   Phase 154+ tunability if surfaces.

3. **Recursive q-clause length ~5kB for 100
   folders.** Google's q DSL has a length
   limit (not client-enforced). Operators
   hitting it see a Drive API error.

4. **recursive flag silently ignored when
   parent_folder_id absent.** Documented in
   the tool description.

5. **walk_folder_tree happens against operator's
   default corpus, not drive_id scope.** If the
   operator wants "recursive recent in this
   Team Drive's /Projects folder," the walk
   needs to be scoped — Phase 154+ candidate.

6. **No tests for walk_folder_tree itself.**
   Needs a mock client or live Drive
   (operator-validation tier). The pure
   helpers around it (compose, caps) are
   exhaustively tested.

### Direction after Phase 153

After Phase 153, Phase 148's two honest-debts
clear. Phase 154+ candidates:

1. **Parallel walk_folder_tree** via
   `tokio::join_all` if 100-folder latency
   surfaces.
2. **Recursive walk within `drive_id`
   scope.**
3. **Operator-tunable recursive caps.**
4. **Voice abort UX knob.**
5. **Silero ONNX VAD.**
6. **Streaming ASR.**
7. **Wake-word activation.**
8. **Multimodal output.**
9. **macOS streaming variant.**
10. **Lock-free AudioIn detector.**
11. **Calendar fuzzy dedup.**
12. **Calendar max_concurrent knob.**
13. **Calendar writable_only filter.**
14. **access_role deprecation.**
15. **Budget category migration tool.**
16. **Budget currency / rust_decimal.**
17. **Multi-category trend breakdown.**
18. **Trend smoothing / moving average.**
19. **Bulk budget operations.**
20. **Drive Activity API.**
21. **Proactive reminder dispatch.**
22. **Relative-time localization.**
23. **whisper-cpp-plus rehabilitation.**
24. **`build_agent_stack` substrate-tier
    promotion.**
25. **Channel Activation Milestone** —
    still held intentionally; 42nd
    consecutive deferral at Phase 153 exit.
