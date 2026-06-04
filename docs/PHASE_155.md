# Phase 155 — Calendar: Small Follow-Ons Bundle (Fuzzy Dedup + Writable-Only + Max-Concurrent)

**Phase 151 follow-on, deferred 4 phases.**
Phase 151 closed Phase 142's three calendar
honest-debts (parallel fan-out, cross-calendar
dedup, capability mapping). Phase 151 itself
called out three follow-on candidates that
have been carried forward in every subsequent
calendar reference:

1. **Fuzzy dedup.** Phase 151 dedup matched
   `(summary, start)` exactly. Two copies of
   "Standup" on different calendars with the
   same Z-timestamp dedupe; copies with even
   trivially-different titles ("standup" vs
   "Standup") or near-identical starts
   (10:00:00 vs 10:00:30 across time-zone
   normalization) don't. Phase 155 ships
   normalize-then-bucket fuzzy matching.

2. **`writable_only` filter on
   `calendar.upcoming`.** Operators wanting
   "what's coming up that I can edit" had to
   call `list_calendars` separately, filter by
   `can_write`, then pass the filtered IDs to
   `upcoming`. Phase 155 adds a `writable_only:
   bool` knob that does the filtering
   internally.

3. **`max_concurrent` knob.** Phase 151's
   parallel fan-out fires N requests
   simultaneously where N = `calendar_ids.len()`.
   Rate-limited operators (heavy-use shared org
   calendars) may surface 429s. Phase 155 adds
   an optional `max_concurrent` cap.

Symmetric to Phase 148's drive bundle + Phase
152's voice bundle + Phase 153's drive
recent_* bundle.

## Why this, why now

- **Calendar last touched 4 phases ago.** Phase
  151 was the close-out. Phase 155 closes its
  own follow-on debts.

- **All three are small individual scopes
  natural together.** Fuzzy dedup is a
  substrate normalization upgrade; writable_only
  needs an internal list_calendars round-trip;
  max_concurrent uses tokio::sync::Semaphore.
  Bundling keeps INSTALL coherent ("calendar
  dedupes smarter AND filters by capability
  AND throttles for rate-limited operators").

- **Builds on Phase 151's substrate.** The
  parallel fan-out shape stays. The dedup shape
  stays. The capability mapping (can_read /
  can_write) on list_calendars is exactly what
  writable_only needs.

- **Zero new workspace deps.** `tokio::sync` is
  already in workspace.

## Tasks

1. **Open doc + ROADMAP + README** — this doc +
   the roadmap section + the README row.
   Backfill Phase 154 hash to `ae518a4`.

2. **Fuzzy dedup substrate.** Upgrade
   `dedup_events` in `tools/upcoming.rs`:
   - Normalize summary: lowercase + trim.
   - Bucket start time to 5-minute boundaries
     (truncate via
     `(rfc3339_to_unix_secs / 300) * 300`).
   - Same first-occurrence-wins semantic.
   Add `fuzzy_dedup: bool` input knob on
   `calendar.upcoming` (default `true` —
   strictly more aggressive than Phase 151
   exact; operators wanting the old exact
   behavior set `fuzzy_dedup: false`).
   Tests cover:
   - Identical normalized titles dedupe ("Standup"
     vs "STANDUP").
   - Time within 5-min bucket dedupes (10:00:00
     vs 10:01:30 → both → bucket 10:00).
   - Time outside bucket kept (10:00 vs 10:05
     → different buckets).
   - Differ-by-suffix kept ("Standup" vs
     "Standup — Team A" — the suffix carries
     intentional meaning).
   - fuzzy_dedup=false falls back to Phase 151
     exact behaviour.

3. **writable_only filter + max_concurrent
   knob.** Two small additions to
   `calendar.upcoming`:
   - `writable_only: bool` (default `false`).
     When `true`: internally call
     `/users/me/calendarList`, filter to entries
     with `access_role in [owner, writer]`,
     intersect with `calendar_ids` (or use them
     if absent). Adds one extra round-trip to
     the API call. If no writable calendars
     remain, return empty events.
   - `max_concurrent: Option<u32>` (default
     `None` = unlimited). When set: throttle the
     parallel fan-out via
     `tokio::sync::Semaphore` so at most N
     per-calendar requests run simultaneously.
   Tests cover input parsing for both knobs +
   substrate-tier semaphore composition.

4. **INSTALL + exit + Frozen.** INSTALL.md
   calendar section: `calendar.upcoming` row
   updated with the three new knobs. Phase 155
   exit doc with prediction-vs-reality.
   README + ROADMAP flip Phase 155 to Frozen.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment; pure tool input + substrate
  refinement. Streak: 45 → **46**.
- **PRODUCT.md** — **Will hold.** Streak:
  45 → **46**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 155 work in `aivyx-calendar`. Core
  untouched. Streak: 20 → **21**.

## Exit criteria

- [ ] `docs/PHASE_155.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `dedup_events` fuzzy substrate +
  `fuzzy_dedup` input — Task 2.
- [ ] `writable_only` + `max_concurrent`
  inputs honored in execute — Task 3.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+8` to `+14`
  (fuzzy dedup substrate ~4-5 + input
  parsing ~3-4 + Semaphore composition
  ~1-2).

## Honest scope risks at sign-off

- **Fuzzy dedup changes observable behavior.**
  Operators with "Standup" + "STANDUP" on
  different calendars previously saw two
  entries; now they see one. Most will
  prefer this. If anyone wants the old
  exact behavior, they set
  `fuzzy_dedup: false`.

- **5-minute time bucket is opinionated.**
  Events at 9:59 and 10:00 (1-minute apart)
  fall in different buckets (9:55 vs 10:00).
  Adjacent buckets aren't merged. Operators
  with this edge case file feedback; Phase
  156+ could shift to a "within ±N minutes"
  sliding window if surfaces.

- **`writable_only` adds an API round-trip.**
  `list_calendars` happens before the
  per-calendar events fan-out. Latency penalty
  is ~100-300ms (one Google API call). Phase
  156+ could cache the calendar list per
  session if surfaces.

- **`writable_only` doesn't compose with
  empty calendar_ids semantically.** Operators
  who pass `writable_only: true` without
  `calendar_ids` get "all writable
  calendars." If they pass
  `writable_only: true` AND
  `calendar_ids: ["primary", "shared@..."]`,
  the result is the intersection. The
  documentation surfaces this in the tool
  description.

- **`max_concurrent` doesn't reorder
  results.** Order of merged events is
  determined by the post-merge sort, not by
  request order. Operators don't see any
  difference whether N=3 or N=∞ for the
  same set of calendars.

- **No `min_concurrent` knob.** Operators
  who want at least N parallel requests have
  no way to enforce it. Trivial substrate;
  Phase 156+ if surfaces.

- **Forty-fourth consecutive deferral of the
  Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 155

After Phase 155, Phase 151's three follow-on
debts clear. Phase 156+ candidates:

1. **Sliding-window fuzzy time** if 5-min
   bucket surfaces edge-case complaints.
2. **calendarList session caching** for
   writable_only latency.
3. **min_concurrent knob** if anyone wants
   it.
4. **PDF / SVG / TIFF media type support
   for /image.**
5. **Client-side image size cap.**
6. **Multi-image queue.**
7. **URL-based image source.**
8. **Mid-recording or mid-reply /image
   command.**
9. **Parallel walk_folder_tree.**
10. **Recursive walk within drive_id
    scope.**
11. **Operator-tunable recursive caps.**
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
    still held intentionally; 44th
    consecutive deferral at Phase 155
    open.

## Prediction vs reality

_Populated at Phase 155 exit. Predictions at
sign-off: DESIGN.md HOLD → 46; PRODUCT.md HOLD
→ 46; lib.rs HOLD → 21; zero new deps; test
count delta `+8` to `+14`; zero clippy
warnings._
