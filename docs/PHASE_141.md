# Phase 141 — Chapter G: `calendar.upcoming` Tool

**Pivot from voice.** Phases 135-140 shipped six
consecutive voice phases, building a strong
end-to-end voice loop: ASR + TTS substrate, audio
I/O, feature parity with Local, streaming TTS,
energy-threshold VAD, and operator-tunable
config. The voice substrate is mature; further
voice work delivers diminishing-returns
refinement.

Phase 141 pivots to operator-facing capability
expansion. Phase 125 framed three Chapter G #2
candidates: calendar reminders, lightweight
budget tracking, and `health.check.remove` +
automatic alert dispatch. Phase 141 ships the
first: a `calendar.upcoming` tool that surfaces
imminent calendar events to the agent.

## Why this, why now

- **The agent can already read calendars
  (Phase 128).** `calendar.list_events` takes
  arbitrary time bounds. The agent has to know
  to construct "now to now+24h" + parse the
  results to figure out what's coming up.
  `calendar.upcoming` is the LLM-ergonomic
  shape for the most common case: "what's on my
  calendar soon."

- **Builds on existing OAuth + client
  substrate.** Phase 128 shipped the Google
  Calendar v3 client with refresh tokens, the
  shared `SharedCalendarClient` Arc-handle, and
  the per-tool capability bases
  (`calendar.read` / `calendar.write`). Phase
  141 adds one read-side tool that reuses
  every piece.

- **Daily-use utility.** The agent answering
  "what do I have today" or proactively
  saying "your standup is in 15 minutes" is
  the kind of thing a personal assistant
  should do. Phase 141 makes that tool surface
  natural for the LLM.

- **Smallest meaningful scope.** Phase 141
  doesn't ship proactive reminders (agent
  initiating without a user turn) — that's a
  larger architectural question about
  scheduled-event dispatch. Phase 141 just
  adds the read-side tool the agent calls in
  response to "what's coming up" prompts.
  Phase 142+ candidate: scheduled-reminder
  cron.

## Tasks

1. **Open doc + ROADMAP + README** — this doc
   + the roadmap section + the README row.
   Backfill Phase 140 hash to `45ff66e`.

2. **`relative_time` substrate module.** Add
   `chrono` dep to `aivyx-calendar` (already
   in workspace from prior work). New
   `aivyx-calendar/src/relative_time.rs` with:
   ```rust
   pub fn format_relative_time(event_iso8601: &str, now: DateTime<Utc>) -> String
   pub fn is_imminent(event_iso8601: &str, now: DateTime<Utc>, threshold_secs: i64) -> bool
   ```
   Returns "in 5 minutes" / "in 2 hours" /
   "in 3 days" / "tomorrow" / "now" / "X
   minutes ago" / similar. Pure substrate;
   `now` is parameterized for deterministic
   tests. Unit-test coverage for the full
   range: <60s, minutes, hours, "tomorrow",
   days, past events.

3. **`calendar.upcoming` tool.** Promote
   `event_summary` + `flatten_timestamp` from
   `list_events.rs` to `tools/mod.rs` as
   `pub(crate)` so the new tool can reuse them.
   New `tools/upcoming.rs` with
   `CalendarUpcoming` struct + Tool impl:
   - Input: `{window_hours? default 24,
     calendar_id? default primary, max_results?
     default 50}`.
   - Computes `time_min = now` and
     `time_max = now + window_hours`.
   - Calls the existing `events` API endpoint
     via the shared client.
   - Enriches each event with two extra
     fields: `starts_in_human` (e.g. "in 2
     hours") and `is_imminent` (boolean — true
     if event starts within 30 minutes).
   - Required scope: `calendar.read` (same as
     `list_events`).
   - Register in `tools/mod.rs`.

4. **Main.rs wiring + INSTALL + exit + Frozen.**
   Add `CalendarUpcoming` construction +
   harness registration in the calendar
   binary's `main.rs`. INSTALL.md calendar
   section gains a Phase 141 paragraph
   documenting the new tool with an example
   prompt. Phase 141 exit doc with
   prediction-vs-reality.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment. Streak: 31 → **32**.
- **PRODUCT.md** — **Will hold.** Calendar
  surface expansion reinforces the personal-
  assistant framing. Streak: 31 → **32**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 141 work in `aivyx-calendar`. Core
  untouched. Streak: 6 → **7**.

## Exit criteria

- [ ] `docs/PHASE_141.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `relative_time` module public in
  `aivyx-calendar` with full unit-test
  coverage — Task 2.
- [ ] `CalendarUpcoming` tool exists +
  registered + uses the relative-time
  substrate — Task 3.
- [ ] Calendar binary registers the new tool —
  Task 4.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies (chrono
  already in workspace).
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+8` to `+15`
  (relative_time ~8-10 tests; tool input
  parsing + output shape ~3-5 tests).

## Honest scope risks at sign-off

- **No proactive reminders.** Phase 141 ships
  the read tool; the agent calls it when the
  operator asks. Truly proactive ("ping me
  before my next meeting") needs scheduled
  event dispatch and is Phase 142+ scope.

- **Relative-time format is English-only.**
  "in 5 minutes" / "tomorrow" — no localization
  posture. The agent can paraphrase in any
  language but the substrate string is
  English. Phase 142+ candidate if operator
  language varies.

- **`is_imminent` threshold is hardcoded.**
  30 minutes baked into the tool. Operators
  can't tune; agent can't override per-call.
  Phase 142+ if it matters.

- **DST + timezone handling delegated to
  chrono.** RFC 3339 timestamps carry their
  own UTC offset; chrono normalizes correctly.
  But "tomorrow at 9am" semantics live in the
  caller's zone, not in the event's. Phase
  141 says "in 18 hours" (UTC delta), not
  "tomorrow at 9am" — simpler and unambiguous.

- **Code-share with `list_events` via
  promotion.** `event_summary` and
  `flatten_timestamp` become `pub(crate)`
  helpers. If a later tool needs different
  output fields, the helpers stay shared but
  the tool builds its own enriched dict.
  Acceptable.

- **Thirtieth consecutive deferral of the
  Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 141

After Phase 141, the agent can surface
upcoming events. Phase 142+ candidates:

1. **Proactive reminder dispatch** — a
   scheduled-event runner that fires
   reminders on its own. Big architectural
   step.
2. **Chapter G budget tracking** — Phase 125
   candidate. Add `budget.record` + `budget.summary`
   tools.
3. **Chapter G health.check.remove + alert
   dispatch** — the other Phase 125
   candidate.
4. **Multi-calendar enumeration** —
   `calendar.list_calendars` so the agent
   can see secondary calendars.
5. **Voice continuation** — mid-synthesis
   abort, Silero VAD, streaming ASR,
   wake-word, multimodal output, macOS
   variant, lock-free detector.
6. **whisper-cpp-plus rehabilitation.**
7. **`build_agent_stack` substrate-tier
   promotion** if more channel adapters ship.
8. **Channel Activation Milestone** — still
   held intentionally; 30th consecutive
   deferral at Phase 141 open.

## Prediction vs reality

_Populated at Phase 141 exit. Predictions at
sign-off: DESIGN.md HOLD → 32; PRODUCT.md HOLD
→ 32; lib.rs HOLD → 7; zero new deps; test
count delta `+8` to `+15`; zero clippy
warnings._
