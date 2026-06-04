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

**Three-of-three streak HOLDs as predicted.**

- **DESIGN.md** — HELD as predicted (`62dabbdd…`
  unchanged). No contract amendment. Streak:
  45 → **46**.
- **PRODUCT.md** — HELD as predicted (`467ba59a…`
  unchanged). Streak: 45 → **46**.
- **`aivyx-core/src/lib.rs`** — HELD as predicted
  (`4f9b8c81…` unchanged). All Phase 155 work in
  `aivyx-calendar`. Continuing post-Phase-135
  reset: 20 → **21**.

**Test count delta: +15 — one over predicted
`+8` to `+14` range.** Workspace lib tests
3249 → 3264. Per-module:
- `upcoming` (fuzzy dedup): +8 (normalize_summary
  cases, bucket_start_5min boundary truncation,
  bucket_start_5min unparseable defensive,
  identical-normalized titles dedupe regression
  boundary, within-5min-bucket dedupes,
  outside-5min-bucket kept both, differ-by-suffix
  kept both, fuzzy_dedup default true + false
  honored).
- `upcoming` (writable_only + max_concurrent):
  +7 (writable_only default false, honored,
  max_concurrent default None, honored, zero
  rejected, calendar_ids_explicit tracks
  operator choice across three input shapes
  — that test fans out into three internal
  asserts but counts as one).

Honest framing: 7 tests in the writable_only +
max_concurrent block rather than 6. One-off
from the +14 upper bound; same substrate-
exhaustive posture as preceding phases.

**Zero new workspace dependencies** as predicted.
`tokio::sync::Semaphore` was already available
via the existing tokio workspace dep.

**Zero clippy warnings** with default features.
One transient doc-lazy-continuation catch on
the `dedup_events` docstring; resolved by
collapsing the bullet list to a single
paragraph.

### What landed cleanly + what bent

**Cleanly:**
- `normalize_summary` + `bucket_start_5min` pure
  substrate helpers. Bucket helper handles
  unparseable input defensively (returns raw
  string).
- `dedup_events` gains `fuzzy: bool` param.
  Phase 151 callers updated to pass false
  explicitly; the integration call site in
  `execute()` passes `parsed.fuzzy_dedup`.
- `fuzzy_dedup` input knob (default true).
- `writable_only` input knob with internal
  list_calendars round-trip + intersection
  semantic (operator-explicit vs defaulted).
- `max_concurrent` input knob backed by
  `tokio::sync::Semaphore`. When None, permits
  = calendar_ids.len() (matches Phase 151
  unlimited behavior).
- `calendar_ids_explicit` ParsedInput field
  tracks operator choice for writable_only's
  intersection-vs-replacement decision.
- INSTALL.md calendar.upcoming row updated with
  all three new knobs documented + Phase 141 +
  142 + 151 + 155 lineage.
- 3263 workspace lib tests pass; clippy clean.

**Bent honestly:**

1. **Fuzzy dedup changes default observable
   behavior.** Operators with "Standup" +
   "STANDUP" copies previously saw two
   entries; now they see one. Most will
   prefer this. Operators wanting exact match
   set `fuzzy_dedup: false`. Documented in
   the tool description.

2. **5-minute bucket is opinionated.** Events
   at 9:59 and 10:00 fall in different buckets
   (9:55 vs 10:00). Adjacent-bucket merging
   deferred to Phase 156+ if surfaces.

3. **writable_only adds an API round-trip.**
   Operators using it pay ~100-300ms more
   latency. Phase 156+ session caching
   candidate.

4. **writable_only semantic split.** Operator-
   explicit ids → intersection (drop read-
   only). Defaulted ids → replacement (use all
   writable). Documented in the description
   + verified by the calendar_ids_explicit
   tracking test.

5. **max_concurrent gates the API request
   phase only.** Post-await processing (drop
   ordering, etc.) isn't gated. In practice
   that's microseconds vs the API round-trip,
   so the throttle behaves as operators
   expect.

6. **No tests for runtime semaphore behavior
   or writable_only round-trip.** Operator-
   validation tier (live API needed).
   Substrate parsing + the explicit-tracking
   bool exhaustively tested.

7. **No `min_concurrent` knob.** No-one's
   asked. Phase 156+ if surfaces.

### Direction after Phase 155

After Phase 155, Phase 151's three follow-on
debts clear. Phase 156+ candidates:

1. **Sliding-window fuzzy time** for
   adjacent-bucket merging.
2. **calendarList session caching** for
   writable_only latency.
3. **min_concurrent knob.**
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
    exit.
