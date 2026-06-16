# Phase 151 — Calendar: Phase 142 Debt Cleanup Bundle

**Phase 142 close-out, deferred 9 phases.** Phase
142 shipped `calendar.list_calendars` +
multi-calendar `calendar.upcoming`, with three
documented honest-debts:

1. **Sequential fan-out, not parallel.** An
   operator with 5 calendars takes ~5× single-
   calendar latency.
2. **No cross-calendar dedup.** An event the
   operator has on both personal AND work
   calendars (typical cross-invite case)
   surfaces twice.
3. **`access_role` raw passthrough.** Google
   returns "owner" / "writer" / "reader" /
   "freeBusyReader"; the agent has to reason
   about each.

Phase 151 closes all three in one phase. Mirrors
Phase 148's drive cleanup pattern and Phase
144's budget CRUD close-out — three small
substrate changes bundled to close a multi-debt
list.

## Why this, why now

- **Longest-deferred substrate debt.** Calendar
  was last touched in Phase 142 (9 phases ago,
  longest gap of any feature surface in the
  current phase window). The Phase 142 exit
  doc's "honest-debt list" has been carried in
  every subsequent calendar reference without
  resolution.

- **Three small individual scopes; natural
  together.** Parallel fan-out is a one-line
  `for` → `join_all` change. Dedup is ~15
  lines of pure substrate. Access-role mapping
  is ~5 lines on the existing
  `calendar_summary` mapper. Bundling them
  keeps INSTALL coherent ("calendar now scales
  + dedupes + tells you what you can do").

- **Symmetric to Phase 148's drive cleanup.**
  Phase 148 closed Phase 145's drive
  honest-debts in one phase; Phase 151 closes
  Phase 142's calendar honest-debts in one
  phase. Consistent debt-closure pattern.

- **Zero new workspace deps.** `futures` is
  already in the workspace via other crates;
  if not pulled into aivyx-calendar, add as
  crate dep (matching Phase 141/145's chrono
  pattern).

## Tasks

1. **Open doc + ROADMAP + README** — this doc +
   the roadmap section + the README row.
   Backfill Phase 150 hash to `5945702`.

2. **Parallel fan-out via `tokio::join_all` /
   `futures::future::join_all`.** In
   `calendar.upcoming`'s execute(), replace the
   sequential `for calendar_id in
   parsed.calendar_ids` loop with a Vec of
   per-calendar futures awaited in parallel.
   On any per-calendar API error, the tool call
   fails with which calendar errored. Maintains
   the existing post-merge sort.

3. **Cross-calendar event dedup.** New pure-
   substrate `dedup_events(events: Vec<Value>) ->
   Vec<Value>` in calendar.upcoming module.
   Removes duplicates by `(summary, start)` key
   — keeps the first occurrence (which is
   typically the operator's primary calendar's
   copy when calendar_ids has primary first).
   Applied to the merged event list before the
   final sort + cap. Tests cover: no dupes
   unchanged, exact dupe one-removed, differ-
   by-summary kept, differ-by-start kept, null
   summary handled defensively.

4. **`access_role` → `can_read` / `can_write`
   mapping.** In `list_calendars.rs`'s
   `calendar_summary` mapper, add two booleans
   derived from access_role:
   - `owner` / `writer` → can_write=true.
   - `owner` / `writer` / `reader` → can_read=true.
   - `freeBusyReader` → both false.
   `access_role` still surfaces verbatim
   alongside (no breaking change to existing
   consumers; new fields are additive). Tests
   cover all four access_role variants +
   defensive null handling.

5. **INSTALL + exit + Frozen.** INSTALL.md
   calendar section updates: `calendar.upcoming`
   row notes parallel fan-out + dedup;
   `calendar.list_calendars` row notes
   can_read/can_write fields. Phase 151 exit
   doc with prediction-vs-reality. README +
   ROADMAP flip Phase 151 to Frozen.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment; pure substrate refactor +
  additive output fields. Streak: 41 → **42**.
- **PRODUCT.md** — **Will hold.** Streak:
  41 → **42**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 151 work in `aivyx-calendar`. Core
  untouched. Streak: 16 → **17**.

## Exit criteria

- [ ] `docs/PHASE_151.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `calendar.upcoming` execute() parallelizes
  via `join_all` — Task 2.
- [ ] `dedup_events` pure substrate + tested +
  integrated into upcoming's merge step —
  Task 3.
- [ ] `can_read`/`can_write` fields on
  `calendar.list_calendars` output — Task 4.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+8` to `+14`.

## Honest scope risks at sign-off

- **Parallel fan-out increases peak API load.**
  Operators with 5+ calendars previously made
  5 sequential requests over ~2-5 seconds;
  parallel makes 5 simultaneous requests in
  ~500ms. Google Calendar's per-second rate
  limit is high enough this is fine, but
  operators hitting the limit (heavy-use
  shared org calendars) might surface 429s.
  Phase 152+ could add concurrent-request
  cap (e.g. `max_concurrent: 3`) if surfaces.

- **Dedup is by exact `(summary, start)`
  match.** Events with different summaries
  ("Team standup" vs "Standup — team A") or
  different start times across calendars
  (one entered as 10:00 PT, another as
  17:00 UTC representing the same wall-clock
  moment via different timezone) won't
  dedupe. Phase 152+ could add fuzzy
  matching if surfaces.

- **Dedup keeps first occurrence, drops
  later.** Operators with the same event on
  multiple calendars see it tagged with the
  first-encountered calendar_id. Acceptable;
  operators typically pass calendar_ids
  with their primary first.

- **`access_role` raw still surfaced.** New
  `can_read`/`can_write` fields are
  additive; existing consumers reading
  `access_role` continue to work unchanged.
  Phase 152+ could deprecate the raw field
  if it ever stabilizes.

- **`freeBusyReader` maps to both false.**
  Operators can technically read free/busy
  state for those calendars but not event
  details. We chose to surface `can_read =
  false` because "read events" semantically
  means "read event content," not "read
  busy-times." Documented in the tool
  description.

- **Fortieth consecutive deferral of the
  Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 151

After Phase 151, Phase 142's debt list is
fully cleared. Phase 152+ candidates:

1. **Calendar fuzzy dedup** — match by
   normalized title + approximate time
   window.
2. **Calendar `max_concurrent` knob** for
   rate-limited operators.
3. **`access_role` deprecation in favor of
   can_read/can_write** if usage stabilizes.
4. **Budget category migration tool**
   (bulk normalize legacy entries).
5. **Budget currency / rust_decimal.**
6. **Multi-category trend breakdown.**
7. **Trend smoothing / moving average.**
8. **Bulk budget operations.**
9. **Recursive folder filter on drive
   recent_*.**
10. **drive_id parameter on drive recent_*.**
11. **Drive Activity API.**
12. **Aggressive voice abort.**
13. **Partial-text preservation on voice
    abort.**
14. **Silero ONNX VAD.**
15. **Streaming ASR.**
16. **Wake-word activation.**
17. **Multimodal output.**
18. **macOS streaming variant.**
19. **Lock-free AudioIn detector.**
20. **VAD config validation.**
21. **Proactive reminder dispatch.**
22. **Relative-time localization.**
23. **whisper-cpp-plus rehabilitation.**
24. **`build_agent_stack` substrate-tier
    promotion.**
25. **Channel Activation Milestone** —
    still held intentionally; 40th
    consecutive deferral at Phase 151 open.

## Prediction vs reality

**Three-of-three streak HOLDs as predicted.**

- **DESIGN.md** — HELD as predicted (`62dabbdd…`
  unchanged). No contract amendment. Streak:
  41 → **42**.
- **PRODUCT.md** — HELD as predicted (`467ba59a…`
  unchanged). Streak: 41 → **42**.
- **`aivyx-core/src/lib.rs`** — HELD as predicted
  (`4f9b8c81…` unchanged). All Phase 151 work in
  `aivyx-calendar`. Continuing post-Phase-135
  reset: 16 → **17**.

**Test count delta: +14 — top of predicted `+8`
to `+14` range.** Workspace lib tests 3200 →
3214. Per-module:
- `upcoming` (dedup substrate): +6 (no
  duplicates unchanged, exact-duplicate keeps
  first, differ-by-summary kept, differ-by-
  start kept, null-summary defensive, empty
  input).
- `list_calendars` (capability mapping): +8
  (owner / writer / reader / freeBusyReader /
  unknown / empty-string capability_from_access_role
  tests + writer-end-to-end + reader-end-to-end
  through calendar_summary).

**Zero new workspace dependencies** as
predicted. `futures-util` was already a
workspace dep; aivyx-calendar adds it as a
crate dep (matching Phase 141 + 145's chrono
pattern).

**Zero clippy warnings** with default features.

### What landed cleanly + what bent

**Cleanly:**
- Sequential `for calendar_id in
  parsed.calendar_ids` → `join_all` over a
  Vec of per-calendar futures. Latency
  collapses from sum-of-per-calendar-times
  to slowest-single-calendar.
- Per-calendar future Result type
  `Result<(String, Value), (String, _)>`
  preserves calendar_id identity through
  both success and error paths.
- `dedup_events` pure substrate function +
  6 tests covering every distinguishing
  key case.
- `capability_from_access_role` pure
  substrate + 6 unit tests covering every
  documented role + forward-compat unknown
  role + empty-string defensive case.
- Existing `calendar_summary` tests
  extended with can_read/can_write
  assertions; new tests cover writer +
  reader end-to-end through the mapper.
- INSTALL.md calendar tool table rows
  updated for both upcoming + list_calendars
  with Phase 151 enrichment notes.
- 3214 workspace lib tests pass; clippy clean.

**Bent honestly:**

1. **Parallel fan-out increases peak API
   load.** 5 calendars previously made 5
   sequential requests over ~2-5s; parallel
   makes 5 simultaneous requests in ~500ms.
   Google's rate limits are high enough this
   is fine in practice. Phase 152+
   max_concurrent knob if heavy-use operators
   surface 429s.

2. **Dedup is exact `(summary, start)` match.**
   "Standup" vs "Standup — team A" won't
   dedupe even at the same start. Phase 152+
   fuzzy dedup candidate.

3. **Dedup keeps first occurrence.** Operators
   typically pass calendar_ids primary-first;
   the retained copy is the primary's. If
   passed work-first, the work copy wins.
   Documented; operator-decided.

4. **`access_role` raw still surfaced.**
   can_read/can_write are additive. Existing
   consumers reading access_role keep
   working. Phase 152+ could deprecate the
   raw field if usage stabilizes.

5. **`freeBusyReader → (false, false)` is
   a semantic choice.** Operators technically
   read free/busy times for those calendars;
   we chose to surface "can_read=false"
   because "read events" semantically means
   "read event content," not "read busy
   times." Pinned by a regression-boundary
   test.

6. **No combined upcoming + capability
   pre-filter.** If the operator asks
   "what's coming up on writable calendars,"
   the agent makes two tool calls
   (list_calendars to find can_write IDs,
   then upcoming with those). Phase 152+
   could add a writable-only filter on
   upcoming if surfaces.

7. **Test count at top of predicted range.**
   +14 exactly, not the substrate-exhaustive
   overshoot pattern that's been
   characteristic of recent phases. Honest
   — capability mapping has limited input
   cases (4 documented roles + 1 unknown +
   1 empty = 6 substrate tests + 2 end-to-end).

### Direction after Phase 151

After Phase 151, Phase 142's three honest-debts
fully clear. Phase 152+ candidates:

1. **Calendar fuzzy dedup** — normalized
   title + approximate time window.
2. **Calendar `max_concurrent` knob** for
   rate-limited operators.
3. **Calendar `writable_only` filter on
   upcoming.**
4. **`access_role` deprecation** in favor of
   can_read/can_write if usage stabilizes.
5. **Budget category migration tool.**
6. **Budget currency / rust_decimal.**
7. **Multi-category trend breakdown.**
8. **Trend smoothing / moving average.**
9. **Bulk budget operations.**
10. **Recursive folder filter on drive
    recent_*.**
11. **drive_id parameter on drive recent_*.**
12. **Drive Activity API.**
13. **Aggressive voice abort.**
14. **Partial-text preservation on voice
    abort.**
15. **Silero ONNX VAD.**
16. **Streaming ASR.**
17. **Wake-word activation.**
18. **Multimodal output.**
19. **macOS streaming variant.**
20. **Lock-free AudioIn detector.**
21. **VAD config validation.**
22. **Proactive reminder dispatch.**
23. **Relative-time localization.**
24. **whisper-cpp-plus rehabilitation.**
25. **`build_agent_stack` substrate-tier
    promotion.**
26. **Channel Activation Milestone** —
    still held intentionally; 40th
    consecutive deferral at Phase 151 exit.
