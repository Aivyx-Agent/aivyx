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

**Three-of-three streak HOLDs as predicted.**

- **DESIGN.md** — HELD as predicted (`62dabbdd…`
  unchanged). No contract amendment. Streak:
  31 → **32**.
- **PRODUCT.md** — HELD as predicted (`467ba59a…`
  unchanged). Streak: 31 → **32**.
- **`aivyx-core/src/lib.rs`** — HELD as predicted
  (`4f9b8c81…` unchanged). All Phase 141 work in
  `aivyx-calendar`. Continuing post-Phase-135
  reset: 6 → **7**.

**Test count delta: +20 — over predicted `+8`
to `+15` range.** Workspace lib tests 3055 →
3075. Per-module:
- `relative_time`: +11 (unparseable, within-a-
  minute, future minutes/hours/tomorrow/days,
  past mirror, all-day date format, imminent
  inside/outside threshold, imminent on
  unparseable).
- `tools::upcoming`: +9 (default input, explicit
  window, window cap, max_results cap, zero
  window rejected, empty calendar_id rejected,
  enrich attaches fields, far-future not
  imminent, all-day enrichment).

Higher-than-predicted because the substrate
nature of relative_time encouraged exhaustive
unit tests of every phrase range; the
tool-input parsing also rendered all the
validation branches as their own tests
(matches the per-tool test density of the
other five calendar tools).

**Zero new workspace dependencies** as
predicted. `chrono` was already a workspace
dep; aivyx-calendar adds it as a crate dep,
not introducing a new workspace dep.

**Zero clippy warnings** with default features.
Two transient catches during Task 3:
1. `event_summary` test import broke when the
   helper moved to the parent module; fixed
   with `use super::super::event_summary`.
2. Unused `flatten_timestamp` import in the
   same fix; removed.

### What landed cleanly + what bent

**Cleanly:**
- `relative_time` substrate module: 11 unit
  tests with deterministic `now` fixtures
  covering every phrase range and the
  is_imminent flag math. Pure substrate, no
  external state.
- `event_summary` + `flatten_timestamp`
  promoted to `tools/mod.rs` as `pub(crate)`.
  `list_events.rs` now uses the shared
  versions; output-stability across read-side
  tools is single-sourced.
- `CalendarUpcoming` tool: 9 per-tool tests
  + integration through the Tool trait;
  same Google API path as `list_events`,
  same auth substrate, same output shape
  plus two enrichment fields.
- `main.rs` registers the new tool alongside
  the existing five.
- INSTALL.md voice tool table gains a Phase
  141 row.
- 3075 workspace lib tests pass; clippy clean.

**Bent honestly:**

1. **No proactive reminders.** Phase 141 ships
   the read tool only. Truly proactive
   ("agent pings before next meeting") needs
   a scheduled-event runner — separate
   architecture. Phase 142+.

2. **English-only phrase output.** "in 5
   minutes" / "tomorrow" are baked English.
   The agent paraphrases in any language but
   substrate is English. Phase 142+
   localization candidate if operator
   demand surfaces.

3. **`is_imminent` threshold hardcoded at 30
   min.** Phase 142+ if operators want
   per-call tuning or env-var override.

4. **Code-share via promotion, not
   abstraction.** `event_summary` /
   `flatten_timestamp` are `pub(crate)`
   helpers, not a trait. If a future read-
   tool needs different output fields, it
   builds its own. Acceptable for two
   call-sites.

5. **DST + "tomorrow at 9am" semantics
   deferred.** Phase 141 says "in 18 hours"
   (UTC delta from now), not "tomorrow at
   9am". Simpler and unambiguous; phase
   142+ could surface zone-aware phrases
   if useful.

6. **Test count overshot prediction.** +20 vs
   predicted +8 to +15. Substrate exhaustive-
   testing posture (every phrase range, every
   input-validation branch) explains the
   delta. Honest, not padding.

### Direction after Phase 141

After Phase 141, the agent can surface
upcoming events naturally. Phase 142+
candidates:

1. **Proactive reminder dispatch** —
   scheduled-event runner that fires
   reminders on its own. Big architectural
   step.
2. **Chapter G budget tracking** —
   `budget.record` + `budget.summary`.
3. **Chapter G health.check.remove + alert
   dispatch.**
4. **`calendar.list_calendars`** — secondary
   calendar enumeration.
5. **Voice continuation** — mid-synthesis
   abort, Silero VAD, streaming ASR,
   wake-word, multimodal output, macOS
   variant, lock-free detector, VAD config
   validation.
6. **Relative-time localization** if
   operators speak other languages
   primarily.
7. **whisper-cpp-plus rehabilitation.**
8. **`build_agent_stack` substrate-tier
   promotion** if more channel adapters
   ship.
9. **Channel Activation Milestone** — still
   held intentionally; 30th consecutive
   deferral at Phase 141 exit.
