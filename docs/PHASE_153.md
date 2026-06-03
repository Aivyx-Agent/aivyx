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

_Populated at Phase 153 exit. Predictions at
sign-off: DESIGN.md HOLD → 44; PRODUCT.md HOLD
→ 44; lib.rs HOLD → 19; zero new deps; test
count delta `+10` to `+16`; zero clippy
warnings._
