# Phase 148 — Drive: `list_drives` + `parent_folder_id` Filter

**Phase 145 close-out.** Phase 145 shipped
`drive.recent_files` + `drive.recent_changes` but
called out two specific honest-debts:

1. **No `parent_folder_id` filter.** Operators
   wanting "what changed in /Projects/Aivyx
   this week" had to go back to
   `drive.search` with a hand-written query.
2. **No `drive.list_drives`** — shared drives
   (Team Drives) invisible to the agent. The
   analog of Phase 142's `calendar.list_calendars`
   that the agent uses to discover what's
   available.

Phase 148 closes both debts in one phase.

## Why this, why now

- **Symmetric to Phase 142's debt cleanup
  pattern.** Phase 142 closed the calendar
  multi-tenant question (one calendar →
  `calendar.list_calendars` + multi-calendar
  upcoming); Phase 148 closes the equivalent
  Drive question (one root → `drive.list_drives`
  + folder-scoped recent_*).

- **Small individual scopes; natural together.**
  `drive.list_drives` is ~120 lines of HTTP-
  client + JSON-normalize plumbing. The
  `parent_folder_id` filter is a one-line
  append to the existing `build_owned_recent_q`
  and `build_recent_changes_q` helpers.
  Bundling them keeps the operator-facing
  story coherent ("now you can ask about shared
  drives or specific folders").

- **Reuses Phase 129 substrate.** Same client,
  same OAuth, same capability bases. No new
  Drive-side machinery beyond a new endpoint
  call.

- **Zero new workspace deps.**

## Tasks

1. **Open doc + ROADMAP + README** — this doc +
   the roadmap section + the README row.
   Backfill Phase 147 hash to `24ced96`.

2. **`drive.list_drives` tool.** New
   `tools/list_drives.rs`:
   - Calls `GET /drives` via the shared client.
   - Output shape:
     ```json
     {
       "drives": [
         {"id": "0AAABbCcc...",
          "name": "Aivyx Working Group",
          "created_at": "2026-01-15T08:00:00Z"}
       ]
     }
     ```
   - Capability: `drive.read`.
   - Pure-substrate `drive_summary` mapper
     with unit tests on the
     `createdTime`-to-`created_at` rename + the
     defensive empty/null handling.
   - No pagination in MVP (most operators have
     <100 shared drives; Google's default page
     size is 100). Phase 149+ if surfaces.

3. **`parent_folder_id` filter on
   `recent_files` + `recent_changes`.** Extend
   both tools' input schemas with optional
   `parent_folder_id: String`. When present,
   append `'<folder_id>' in parents` to the
   composed `base_q`. Both helpers
   (`build_owned_recent_q` and
   `build_recent_changes_q`) gain a
   parameterized folder argument; pure-substrate
   tests cover the q-string composition
   with/without folder filter for each.

4. **Main.rs wire + INSTALL + exit + Frozen.**
   `main.rs` registers `DriveListDrives` (drive
   surface 9 → 10 tools). INSTALL.md drive
   section gains a row for `drive.list_drives`
   and updates the `recent_files` +
   `recent_changes` rows with
   `parent_folder_id` mention. Phase 148 exit
   doc with prediction-vs-reality.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment. Streak: 38 → **39**.
- **PRODUCT.md** — **Will hold.** Streak:
  38 → **39**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 148 work in `aivyx-drive`. Core
  untouched. Streak: 13 → **14**.

## Exit criteria

- [ ] `docs/PHASE_148.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `DriveListDrives` tool public +
  registered + tested — Task 2.
- [ ] `parent_folder_id` filter on both
  recent_* tools with substrate tests for
  q-string composition — Task 3.
- [ ] Drive harness 9 → 10 tools — Task 4.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+6` to `+12`
  (drive_summary substrate ~3 tests + folder-
  filter q-string variants ~4 tests).

## Honest scope risks at sign-off

- **`parent_folder_id` is not recursive.**
  Drive's `'<id>' in parents` matches direct
  children only, not the subtree. Operators
  wanting "all files anywhere under
  /Projects" go back to `drive.search` with a
  hand-written recursive query, or chain
  `drive.list_folder` + per-subfolder
  `recent_files`. Phase 149+ candidate:
  recursive flag that walks the folder tree.

- **No multi-folder filter.** One folder per
  call. Operators with multiple project
  folders make multiple calls. Phase 149+
  if surfaces.

- **No `drive_id` parameter on recent_*.**
  Both recent tools query the operator's
  default corpus (My Drive + accessible
  shared files). Operators wanting
  "recent files in *this* Team Drive"
  combine `drive.list_drives` + the existing
  `drive.search` with an explicit
  `corpora=drive` query. Phase 149+
  candidate: add `drive_id` to recent_* for
  Team-Drive-scoped queries.

- **No paginated `list_drives`.** Most
  operators have <50 shared drives;
  pagination is Phase 149+ if 100+-drive
  operators surface.

- **Test count overshoot risk.** Phase 145's
  substrate-exhaustive testing of every q-
  string boundary explains why predicted +6
  to +12 is conservative; this phase may
  follow Phase 145's overshoot pattern.
  Honest, not padding.

- **Thirty-seventh consecutive deferral of
  the Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 148

After Phase 148, Drive matches calendar's
post-Phase-142 LLM-ergonomic shape. Phase
149+ candidates:

1. **Recursive folder filter** on recent_*
   for whole-subtree queries.
2. **`drive_id` parameter on recent_*** for
   Team-Drive-scoped queries.
3. **Multi-folder filter** if demand
   surfaces.
4. **Paginated list_drives** for 100+-drive
   operators.
5. **Drive Activity API** — true edit
   history.
6. **Aggressive voice abort** — drop cpal
   stream for instant silence.
7. **Partial-text preservation on voice
   abort.**
8. **Silero ONNX VAD.**
9. **Streaming ASR.**
10. **Wake-word activation.**
11. **Multimodal output.**
12. **macOS streaming variant.**
13. **Lock-free AudioIn detector.**
14. **VAD config validation.**
15. **Category whitelist for budget.**
16. **Budget currency / rust_decimal.**
17. **`budget.trend`.**
18. **Bulk budget operations.**
19. **Proactive reminder dispatch.**
20. **Phase 142 debt cleanup** —
    calendar parallel fan-out + dedup +
    capability mapping.
21. **Relative-time localization.**
22. **whisper-cpp-plus rehabilitation.**
23. **`build_agent_stack` substrate-tier
    promotion.**
24. **Channel Activation Milestone** —
    still held intentionally; 37th
    consecutive deferral at Phase 148
    open.

## Prediction vs reality

**Three-of-three streak HOLDs as predicted.**

- **DESIGN.md** — HELD as predicted (`62dabbdd…`
  unchanged). No contract amendment. Streak:
  38 → **39**.
- **PRODUCT.md** — HELD as predicted (`467ba59a…`
  unchanged). Streak: 38 → **39**.
- **`aivyx-core/src/lib.rs`** — HELD as predicted
  (`4f9b8c81…` unchanged). All Phase 148 work in
  `aivyx-drive`. Continuing post-Phase-135 reset:
  13 → **14**.

**Test count delta: +16 — over predicted `+6`
to `+12` range.** Workspace lib tests 3159 →
3175. Per-module:
- `list_drives`: +4 (drive_summary maps full
  entry with extras dropped, sparse entry
  defensive null, empty entry, schema has no
  required args).
- `recent_files`: +6 (q appends folder clause,
  q omits folder when None, parse extracts
  folder, parse null → None, parse
  empty/whitespace → None, parse rejects
  single-quote).
- `recent_changes`: +6 (q appends folder clause
  + still no owner regression, q omits folder
  when None, parse extracts folder, parse
  rejects single-quote, + the existing
  build_recent_changes_q tests gained their
  third argument).

Same substrate-exhaustive testing pattern as
Phases 141 / 143-145 / 147. Honest, not
padding.

**Zero new workspace dependencies** as predicted.

**Zero clippy warnings** with default features.

### What landed cleanly + what bent

**Cleanly:**
- `DriveListDrives` stateless tool with
  pure-substrate `drive_summary` mapper.
  Renames Google's `createdTime` to canonical
  `created_at`; extras (kind, colorRgb)
  dropped.
- Both recent_* helpers gained a third
  `parent_folder_id: Option<&str>` parameter
  with consistent behaviour: append
  `'<id>' in parents` when present, omit
  otherwise.
- Quote-injection guard: parse_input rejects
  any parent_folder_id containing a single
  quote (Drive's q DSL delimiter). Clean
  parse-time error rather than a malformed
  composed query.
- Tools/mod.rs + main.rs register the new
  tool; Drive harness 9 → 10 tools.
- INSTALL.md drive section updated for both
  list_drives + the parent_folder_id mention
  on recent_*.
- 3175 workspace lib tests pass; clippy clean.

**Bent honestly:**

1. **`parent_folder_id` is not recursive.**
   Drive's q DSL's `'<id>' in parents` matches
   direct children only. Operators wanting
   whole-subtree queries combine
   `drive.list_folder` recursively or hand-write
   a `drive.search` query. Phase 149+
   candidate for a recursive flag.

2. **Quote-injection guard is conservative.**
   We reject ANY single-quote, not just
   unescaped ones. Folder IDs Google returns
   never contain quotes, so this is a
   defensive parse-time filter that never
   fires in practice. Same posture as
   `calendar.list_events_urlencode`.

3. **No `drive_id` on recent_*.** Both recent
   tools still query the operator's default
   corpus (My Drive + accessible shared
   files). For Team-Drive-scoped queries
   operators combine `drive.list_drives` +
   `drive.search` with `corpora=drive`.
   Phase 149+ candidate.

4. **No paginated list_drives.** Most
   operators have <100 shared drives;
   pagination is Phase 149+ if 100+-drive
   operators surface.

5. **Test count overshot prediction.** +16 vs
   predicted +6 to +12. Same substrate-
   exhaustive posture as preceding phases.
   Honest pattern.

### Direction after Phase 148

After Phase 148, Drive matches calendar's
post-Phase-142 LLM-ergonomic shape (modulo
parallel fan-out + dedup, which calendar
itself still owes). Phase 149+ candidates:

1. **Recursive folder filter** on recent_*.
2. **`drive_id` parameter on recent_*** for
   Team-Drive-scoped queries.
3. **Multi-folder filter.**
4. **Paginated list_drives.**
5. **Drive Activity API.**
6. **Aggressive voice abort.**
7. **Partial-text preservation on voice
   abort.**
8. **Silero ONNX VAD.**
9. **Streaming ASR.**
10. **Wake-word activation.**
11. **Multimodal output.**
12. **macOS streaming variant.**
13. **Lock-free AudioIn detector.**
14. **VAD config validation.**
15. **Category whitelist for budget.**
16. **Budget currency / rust_decimal.**
17. **`budget.trend`.**
18. **Bulk budget operations.**
19. **Proactive reminder dispatch.**
20. **Phase 142 debt cleanup** — calendar
    parallel fan-out + dedup + capability
    mapping (still outstanding).
21. **Relative-time localization.**
22. **whisper-cpp-plus rehabilitation.**
23. **`build_agent_stack` substrate-tier
    promotion.**
24. **Channel Activation Milestone** —
    still held intentionally; 37th
    consecutive deferral at Phase 148
    exit.
