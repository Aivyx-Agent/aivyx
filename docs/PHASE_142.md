# Phase 142 — `calendar.list_calendars` + Multi-Calendar `upcoming`

**Phase 141 follow-on.** Phase 141 shipped
`calendar.upcoming` against a single calendar
(default `primary`). Real operators have 3-5
calendars: personal, work, shared family,
project-specific, on-call. With Phase 141, the
agent can't see anything outside `primary` —
it doesn't even know other calendars exist.

Phase 142 closes that gap with two
complementary changes:

1. **`calendar.list_calendars`** — enumerate
   the calendars the operator has access to.
   The agent calls this once (typically at the
   start of a conversation) to learn what
   calendar IDs exist.
2. **Multi-calendar `calendar.upcoming`** —
   extend the existing tool to accept a
   `calendar_ids: [String]` array; fan-out
   queries and merge results sorted by start
   time. Single-calendar callers (Phase 141
   shape) still work unchanged.

## Why this, why now

- **Natural Phase 141 follow-on.** Phase 141
  made "what's coming up" ergonomic for the
  agent — but only on `primary`. The
  operator-level question "what's coming up
  *across all my calendars*" is the same
  cognitive shape; Phase 142 makes it
  expressible with a 2-call agent flow
  (`list_calendars` → `upcoming(calendar_ids:
  [...])`).

- **Smallest meaningful scope.** No new
  substrate: list_calendars uses the existing
  HTTP client, and multi-calendar upcoming is
  a sequential fan-out over the same
  `/calendars/{id}/events` endpoint. Sorted
  merge is pure substrate.

- **Builds on Phase 128 + 141.** Same OAuth
  + client + capability bases. Same shared
  `event_summary` + `flatten_timestamp` from
  Phase 141's promotion.

## Tasks

1. **Open doc + ROADMAP + README** — this doc
   + the roadmap section + the README row.
   Backfill Phase 141 hash to `a7e0440`.

2. **`CalendarListCalendars` tool.** New
   `tools/list_calendars.rs`:
   - Calls `GET /users/me/calendarList`.
   - Output shape:
     ```json
     {
       "calendars": [
         {"id": "primary", "summary": "Personal",
          "is_primary": true, "access_role": "owner"}
       ]
     }
     ```
   - Capability: `calendar.read` (read-only
     enumeration).
   - Register in `tools/mod.rs`.
   - Pure-substrate `calendar_summary` mapper
     gets unit tests for accessRole / primary
     field normalization.

3. **Multi-calendar `calendar.upcoming`.**
   Extend the existing tool's input schema
   with optional `calendar_ids: [String]`.
   Behavior:
   - If `calendar_ids` present: sequential
     fan-out to each calendar's events
     endpoint; tag each event with its
     source `calendar_id`; merge by start
     time; apply `max_results` cap to the
     merged list.
   - If `calendar_ids` absent + `calendar_id`
     present: Phase 141 single-calendar path
     unchanged.
   - If both absent: default to
     `["primary"]`.
   - Each event in the output gains a
     `calendar_id` field for traceability.
   Tests cover legacy single path, multi-
   calendar merge ordering, backward-compat
   output shape.

4. **Main.rs wiring + INSTALL + exit + Frozen.**
   `main.rs` registers `CalendarListCalendars`
   alongside the existing six tools.
   INSTALL.md calendar tool table gains a
   Phase 142 row for `calendar.list_calendars`;
   the `calendar.upcoming` row updates to
   note multi-calendar support. Phase 142
   exit doc with prediction-vs-reality.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment. Streak: 32 → **33**.
- **PRODUCT.md** — **Will hold.** Multi-calendar
  surface reinforces personal-assistant
  framing. Streak: 32 → **33**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 142 work in `aivyx-calendar`.
  Core untouched. Streak: 7 → **8**.

## Exit criteria

- [ ] `docs/PHASE_142.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `CalendarListCalendars` tool exists +
  registered + tested — Task 2.
- [ ] `CalendarUpcoming` accepts
  `calendar_ids` array + fans out sequentially
  + merges by start time + tags events with
  source calendar — Task 3.
- [ ] Phase 141 single-calendar callers still
  pass tests unchanged — Task 3 regression
  boundary.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+6` to `+12`
  (list_calendars ~3-4 tests for the summary
  mapper; multi-calendar merge ~3-4 tests for
  the sort/cap logic; per-tool input parsing
  ~2-4 tests).

## Honest scope risks at sign-off

- **Sequential fan-out, not parallel.** If
  the operator's `calendar_ids` array is
  long (5+ calendars), latency grows
  linearly. Acceptable for Phase 142 MVP;
  Phase 143+ could parallelize with
  `tokio::join_all` if measurement shows
  latency.

- **No de-duplication across calendars.**
  An event the operator has on both
  personal AND work calendars (e.g.
  invited from work, accepted on personal)
  surfaces twice. Phase 143+ could
  dedupe by event title + start time if
  operators ask for it.

- **`max_results` cap applied AFTER merge.**
  If the operator asks for 50 events across
  5 calendars with 20 events each, we
  fetch 100 events and trim to 50. Could
  push the cap down per-calendar (50/5 =
  10 each) but that loses density-sensitive
  events. Acceptable for Phase 142 MVP;
  documented as an honest cost.

- **`access_role` surfaced verbatim.** Google
  returns "owner" / "writer" / "reader" /
  "freeBusyReader". The agent can interpret
  these; Phase 142 doesn't translate to a
  capability mapping (writer-or-better →
  can create events on this calendar). Phase
  143+ candidate.

- **Code-share with `event_summary` already
  in place** from Phase 141. Multi-calendar
  enrichment just adds a `calendar_id`
  field on top of the existing shape;
  no further substrate changes.

- **Thirty-first consecutive deferral of the
  Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 142

After Phase 142, the agent sees across all
operator calendars. Phase 143+ candidates:

1. **Parallel multi-calendar fan-out** with
   `tokio::join_all` if measurement shows
   sequential latency matters.
2. **Cross-calendar dedup** (by title + start
   time) if operators see duplicate events
   from cross-invites.
3. **Per-calendar capability mapping** —
   surface "can_write" booleans from
   accessRole, so the agent knows where it
   can create events.
4. **Proactive reminder dispatch** — the big
   Phase 142+ architectural step that
   remains.
5. **Chapter G budget tracking.**
6. **Chapter G health.check.remove + alert
   dispatch.**
7. **Voice continuation** — mid-synthesis
   abort, Silero VAD, streaming ASR,
   wake-word, multimodal output, macOS
   variant.
8. **Relative-time localization.**
9. **whisper-cpp-plus rehabilitation.**
10. **`build_agent_stack` substrate-tier
    promotion** if more channel adapters
    ship.
11. **Channel Activation Milestone** — still
    held intentionally; 31st consecutive
    deferral at Phase 142 open.

## Prediction vs reality

_Populated at Phase 142 exit. Predictions at
sign-off: DESIGN.md HOLD → 33; PRODUCT.md HOLD
→ 33; lib.rs HOLD → 8; zero new deps; test
count delta `+6` to `+12`; zero clippy
warnings._
