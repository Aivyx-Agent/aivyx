# Phase 145 — Drive: `recent_files` + `recent_changes`

**Pivot from toolkit.** Phases 141-144 ran four
consecutive Chapter F/G phases (calendar.upcoming,
multi-calendar, budget tracking, budget CRUD parity).
Phase 145 pivots to **Drive** — the Phase 129
seven-tool surface that's been quiet since it
shipped. Drive already has `search`, `get_metadata`,
`list_folder`, `download_file`, `upload_file`,
`create_folder`, `delete_file`. Phase 145 adds the
LLM-ergonomic complements that mirror Phase 141's
`calendar.upcoming` pattern: **recent_files** and
**recent_changes**.

## Why this, why now

- **Variety after 4 consecutive toolkit/calendar
  phases.** Drive's been quiet since Phase 129.
  Operator-utility ceiling for "what did I work
  on this week" / "what changed today" is high
  and currently unmet.

- **Mirrors Phase 141's pattern.** The agent can
  already call `drive.search` with an arbitrary
  query DSL, but it has to construct
  `modifiedTime > 'X'` + `'me' in owners` itself.
  `drive.recent_files(window_days)` is the
  LLM-ergonomic shape — same architectural choice
  Phase 141 made over `calendar.list_events`.

- **Two tools for two cognitive shapes.**
  - `drive.recent_files(window_days)` →
    "what did *I* work on" (owner = me).
  - `drive.recent_changes(window_hours)` →
    "what changed in my Drive" (any accessible
    file).
  Different prompts surface naturally to
  different tools.

- **Reuses existing substrate.** Both tools call
  the same `/files` endpoint `drive.search` uses,
  with `q` strings the operator-facing tool
  composes. `file_summary` is already
  `pub(crate)` in `search.rs` from Phase 129.
  Zero new substrate; the helpers just need
  invocation.

- **Zero new workspace deps.** `chrono` already
  in workspace; aivyx-drive adds it as a crate
  dep, matching Phase 141's aivyx-calendar
  posture.

## Tasks

1. **Open doc + ROADMAP + README** — this doc +
   the roadmap section + the README row.
   Backfill Phase 144 hash to `bb28671`.

2. **`drive.recent_files` tool (owned-files
   semantics).** Add `chrono` dep to
   aivyx-drive. New `tools/recent_files.rs`
   with `DriveRecentFiles`:
   - Input: `{window_days? default 7,
     max_results? default 25, include_trashed?
     default false}`.
   - q string: `'me' in owners and
     modifiedTime > '<now - window_days>' and
     trashed = false` (the trashed clause
     drops when include_trashed=true).
   - orderBy=modifiedTime desc.
   - Reuses `file_summary` from
     `tools::search`.
   - Capability: `drive.read`.
   - Unit tests on the q-string composition
     with deterministic now.

3. **`drive.recent_changes` tool (any-files
   semantics).** New
   `tools/recent_changes.rs` with
   `DriveRecentChanges`:
   - Input: `{window_hours? default 24,
     max_results? default 25, include_trashed?
     default false}`.
   - q string: `modifiedTime > '<now -
     window_hours>' and trashed = false`
     (no owners filter).
   - Otherwise mirrors recent_files in shape.
   - Capability: `drive.read`.
   - Unit tests parallel to recent_files'.

4. **Main.rs wire + INSTALL + exit + Frozen.**
   Register both tools in `tools/mod.rs` and
   `main.rs`. Drive surface 7 → 9 tools.
   INSTALL.md drive section gains rows for
   both. Phase 145 exit doc with
   prediction-vs-reality.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment; pure tool addition. Streak:
  35 → **36**.
- **PRODUCT.md** — **Will hold.** Operator-
  utility expansion reinforces personal-
  assistant framing. Streak: 35 → **36**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 145 work in `aivyx-drive`. Core
  untouched. Streak: 10 → **11**.

## Exit criteria

- [ ] `docs/PHASE_145.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `DriveRecentFiles` tool exists +
  registered + tested — Task 2.
- [ ] `DriveRecentChanges` tool exists +
  registered + tested — Task 3.
- [ ] Drive binary registers the new tools —
  Task 4.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies (chrono
  already in workspace).
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+8` to `+14`
  (q-string composition + input parsing for
  each tool ~4-7 tests each).

## Honest scope risks at sign-off

- **`'me' in owners` is heuristic.** It catches
  files the operator owns but misses files
  they're a regular collaborator on. The
  cognitive shape "what I worked on" includes
  shared docs the operator edited but doesn't
  own; recent_files won't surface those.
  Phase 146+ candidate: query Drive Activity
  API for the operator's actual edit history,
  not just ownership.

- **Drive query DSL parsing burden.** Phase 145
  tools compose canned `q` strings; the
  operator's only knobs are window + caps. If
  operators want "what changed in folder X
  this week", they go back to `drive.search`
  with a hand-written query. Phase 146+
  candidate: optional `parent_folder_id`
  filter on both tools.

- **No `q` parameter for further filtering.**
  Phase 145 keeps the tools simple. Operators
  who want both "recent" + "specific
  mimeType" combine `drive.search` with an
  ergonomic time bound themselves.

- **`include_trashed` default `false`.**
  Matches `drive.search`'s default. Operators
  exploring their trash explicitly opt in.

- **No pagination.** Both tools cap at the
  Drive default page (100 files) without
  surfacing `next_page_token`. Acceptable for
  the "recent" use case; if operators hit
  the cap, the window's too wide.

- **Sequential, not date-bucketed output.**
  The agent sees a flat list sorted by
  modifiedTime desc. Phase 146+ could group
  by day/week if operators want a digest
  shape.

- **Thirty-fourth consecutive deferral of
  the Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 145

After Phase 145, Drive matches calendar's
LLM-ergonomic pattern. Phase 146+ candidates:

1. **Drive Activity API** — true edit history
   not just ownership.
2. **`parent_folder_id` filter** on
   recent_files / recent_changes.
3. **`drive.list_drives`** — surface shared
   drives (Drive's "Team Drives"
   equivalent of calendar.list_calendars).
4. **Category whitelist + case-fold for
   budget** — Phase 143 #2 honest-debt.
5. **Currency field for budget.**
6. **`budget.trend`** — month-over-month
   deltas.
7. **rust_decimal switch** for budget
   amounts.
8. **Bulk budget operations.**
9. **Chapter G health.check.remove + alert
   dispatch** — Phase 125 final candidate.
10. **Proactive reminder dispatch.**
11. **Phase 142 debt cleanup** — calendar
    parallel fan-out, dedup, capability
    mapping.
12. **Voice continuation** — mid-synthesis
    abort, Silero VAD, streaming ASR,
    wake-word, multimodal output, macOS
    variant, lock-free detector.
13. **Relative-time localization.**
14. **whisper-cpp-plus rehabilitation.**
15. **`build_agent_stack` substrate-tier
    promotion** if more channel adapters
    ship.
16. **Channel Activation Milestone** — still
    held intentionally; 34th consecutive
    deferral at Phase 145 open.

## Prediction vs reality

**Three-of-three streak HOLDs as predicted.**

- **DESIGN.md** — HELD as predicted (`62dabbdd…`
  unchanged). No contract amendment. Streak:
  35 → **36**.
- **PRODUCT.md** — HELD as predicted (`467ba59a…`
  unchanged). Streak: 35 → **36**.
- **`aivyx-core/src/lib.rs`** — HELD as predicted
  (`4f9b8c81…` unchanged). All Phase 145 work in
  `aivyx-drive`. Continuing post-Phase-135 reset:
  10 → **11**.

**Test count delta: +21 — over predicted `+8` to
`+14` range.** Workspace lib tests 3131 → 3152.
Per-module:
- `recent_files`: +11 (default + explicit
  window/results + clamps × 2 + zero rejected
  + trashed + q includes me-clause + 1-day +
  365-day arithmetic + trashed wrap + passthrough).
- `recent_changes`: +10 (same shape minus the
  me-clause assertion, with a key negative test
  that q does NOT include owner filter — the
  regression boundary between the two tools).

Same substrate-exhaustive posture as Phases 141 /
143 / 144 — every input permutation + window
arithmetic boundary gets its own test. Honest,
not padding.

**Zero new workspace dependencies** as predicted.
`chrono` already a workspace dep; aivyx-drive
adds crate dep only (mirroring Phase 141's
aivyx-calendar pattern).

**Zero clippy warnings** with default features.

### What landed cleanly + what bent

**Cleanly:**
- Two new tools sharing one architectural
  pattern: compose a `base_q` string with
  `format!("modifiedTime > '{}'", since)` and
  optional owners clause; pass through
  `build_q_string` for the standard
  `trashed = false and (...)` wrapping; sort
  by modifiedTime desc.
- `build_owned_recent_q` and
  `build_recent_changes_q` are pure substrate
  with parameterized `now` — deterministic
  arithmetic tests for 1 hour / 1 day /
  365 days / 30 days windows.
- Both tools reuse Phase 129's pub(crate)
  `file_summary` and `build_q_string` from
  `search.rs` — output stability across
  read-side Drive tools is single-sourced.
- `chrono` added to aivyx-drive matching
  Phase 141's aivyx-calendar pattern.
- main.rs registers both; Drive surface 7 →
  9 tools.
- INSTALL.md drive section gains rows for
  both new tools with cognitive-shape
  language ("what did I work on" vs "what
  changed").
- 3152 workspace lib tests pass; clippy clean.

**Bent honestly:**

1. **`'me' in owners` is heuristic.** Misses
   files the operator collaborates on without
   owning. Phase 146+ Drive Activity API
   candidate.

2. **No `q` parameter on either tool.**
   Operators wanting "recent files of type X"
   compose `drive.search` themselves. Phase
   146+ candidate if it surfaces.

3. **No `parent_folder_id` filter.** Operators
   wanting "recent files in /Projects/Aivyx"
   compose `drive.search` themselves. Phase
   146+ candidate.

4. **No pagination.** Both tools cap at the
   max_results value without surfacing
   `next_page_token` consumption strategy.
   The token IS surfaced in output but the
   agent doesn't have a follow-up tool to
   paginate further; operators hitting the
   cap should widen `max_results` or narrow
   the window.

5. **Sequential output not date-bucketed.**
   The agent sees a flat list sorted by
   modifiedTime desc. Phase 146+ could group
   by day for the "today" / "yesterday" /
   "this week" narrative shape.

6. **Test count overshot prediction.** +21
   vs predicted +8 to +14. Same substrate-
   exhaustive posture as the preceding 4
   phases. Honest, not padding.

### Direction after Phase 145

After Phase 145, Drive matches calendar's
LLM-ergonomic pattern. Phase 146+ candidates:

1. **Drive Activity API** — true edit history
   not just ownership.
2. **`parent_folder_id` filter** on recent_*.
3. **`drive.list_drives`** — shared drives
   enumeration.
4. **Category whitelist + case-fold for
   budget.**
5. **Currency field for budget.**
6. **`budget.trend`** — month-over-month.
7. **rust_decimal for budget amounts.**
8. **Bulk budget operations.**
9. **Chapter G health.check.remove + alert
   dispatch.**
10. **Proactive reminder dispatch.**
11. **Phase 142 debt cleanup** — calendar
    parallel fan-out, dedup, capability
    mapping.
12. **Voice continuation** — mid-synthesis
    abort, Silero VAD, streaming ASR,
    wake-word, multimodal output, macOS
    variant, lock-free detector.
13. **Relative-time localization.**
14. **whisper-cpp-plus rehabilitation.**
15. **`build_agent_stack` substrate-tier
    promotion** if more channel adapters
    ship.
16. **Channel Activation Milestone** — still
    held intentionally; 34th consecutive
    deferral at Phase 145 exit.
