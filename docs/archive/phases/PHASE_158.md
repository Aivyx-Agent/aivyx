# Phase 158 — Calendar Follow-Ons Bundle (Sliding-Window Dedup + calendarList Cache + min_concurrent)

**Phase 155 close-out, deferred 3 phases.**
Phase 155 shipped fuzzy dedup, writable_only,
and max_concurrent against `calendar.upcoming`
with three documented honest-debts:

1. **Hard 5-minute bucket flooring.** Two events
   at `10:04` and `10:06` fall into `10:00`
   and `10:05` buckets and never dedupe.
   Sliding-window adjacency merging closes the
   gap.
2. **`writable_only` re-fetches `calendarList`
   every call.** A second `calendar.upcoming`
   call in the same session re-pays the round
   trip even though the writable set rarely
   churns mid-session.
3. **No `min_concurrent` knob.** Operators with
   N small (e.g. 3 calendars) couldn't say
   "always fire all of them at once" — the
   default sequential fallback kicked in.

Phase 158 closes all three. Symmetric to Phase
157 closing Phase 153's three honest-debts.

## Why this, why now

- **Phase 155 was last touched 3 phases ago.**
  Same gap-pattern as Phase 157 closing Phase
  153's debts at the 4-phase mark.

- **All three small individual scopes natural
  together.** Sliding-window dedup is a
  substrate-level swap of `bucket_start_5min` +
  `dedup_by_key` for a sweep+merge. The
  calendarList cache is an `Arc<Mutex<...>>` on
  `SharedCalendarClient`. `min_concurrent` is
  an additive input field.

- **Builds on Phase 155 substrate.** The fuzzy
  dedup substrate, writable_only flow, and
  max_concurrent semaphore landed in
  `tools/upcoming.rs` — Phase 158 modifies
  exactly the same module.

- **Zero new workspace deps.** All three closes
  use already-in-workspace primitives.

## Tasks

1. **Open doc + ROADMAP + README.** This doc,
   the roadmap section, the README row.
   Backfill Phase 157 hash to `16c666a`.

2. **Sliding-window fuzzy time dedup.** Replace
   the `(normalize_summary, bucket_start_5min)`
   hash-key dedup with a sweep+merge:
   - Sort events by `(normalize_summary, start_time)`.
   - For each group sharing the same
     normalized summary, walk the time-sorted
     list and merge consecutive events whose
     starts are within ±5 minutes of the prior
     event in the group.
   - Preserve the earliest start as the
     surviving event; cross-calendar
     enrichment (calendar names list) merges
     across the joined entries.
   Pure substrate; live API not required.
   Tests cover: no-overlap, exact-5-min
   adjacency, chain merges (3 events at
   10:00, 10:04, 10:08 fold to one),
   normalized-summary boundary (different
   summaries stay separate).

3. **calendarList session cache.** Add
   `writable_calendars_cache:
   Arc<Mutex<Option<(Instant, Vec<String>)>>>`
   to `SharedCalendarClient`. On
   `writable_only: true`, check the cache
   first; if present and < 5 min old, reuse;
   else fetch + populate. TTL is hardcoded to
   5 minutes for Phase 158; operator-tunable in
   a future phase if anyone surfaces it.

4. **min_concurrent knob.** Add
   `min_concurrent: u32` (cap 16) input on
   calendar.upcoming. Semantics: when present,
   the parallel fan-out fires at least
   `min(min_concurrent, calendar_count)`
   futures regardless of the default sequential
   fallback. Composes with `max_concurrent`
   (min ≤ max validated at parse time).

5. **INSTALL + exit + Frozen.** INSTALL.md
   calendar.upcoming row picks up the three new
   semantics. PHASE_158.md exit doc with
   prediction-vs-reality. README + ROADMAP
   flip Phase 158 to Frozen.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment; substrate refactor + additive
  input fields + per-client state. Streak:
  48 → **49**.
- **PRODUCT.md** — **Will hold.** Streak:
  48 → **49**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 158 work in `aivyx-calendar`.
  Streak: 23 → **24**.

## Exit criteria

- [ ] `docs/PHASE_158.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] Sliding-window adjacency dedup replaces
  the 5-min bucket key — Task 2.
- [ ] writable_calendars cache lives on
  `SharedCalendarClient` with 5-min TTL —
  Task 3.
- [ ] `min_concurrent` input honored with
  min ≤ max validation — Task 4.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+10` to `+16`.

## Honest scope risks at sign-off

- **Sliding-window merging is order-sensitive.**
  The merge walks sorted starts; an out-of-
  order input could chain-merge events that
  shouldn't share a bucket. Sort guarantees
  stable behavior but a malformed RFC3339
  upstream value could land mid-sort. The
  Phase 155 normalizer already trims; Phase
  158 adds a parse-then-sort step.

- **5-minute TTL on the calendarList cache.**
  An operator who adds a new calendar mid-
  session won't see writable_only widen for
  up to 5 minutes. Acceptable; documented.

- **`min_concurrent` upper bound 16.**
  Phase 155 capped `max_concurrent` at 16.
  Phase 158 mirrors. An operator with 30
  calendars and `min_concurrent: 16` gets 16
  parallel + the rest sequential.

- **min ≤ max validation is parse-time.**
  An operator passing min=8, max=4 gets a
  validation error rather than the tool
  silently downgrading. Trades one error for
  one footgun.

- **Forty-seventh consecutive deferral of the
  Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 158

After Phase 158, Phase 155's three carry-overs
clear. Phase 159+ candidates:

1. **max_concurrent throttle on
   walk_folder_tree** (Phase 157 honest-debt;
   parallel level-BFS still un-throttled).
2. **Operator-tunable image size cap.**
3. **URL fetch timeout.**
4. **HEAD pre-fetch for size check.**
5. **Authenticated URL fetch.**
6. **PDF / SVG / TIFF media type support.**
7. **Mid-recording or mid-reply /image
   command.**
8. **Clipboard-based image source.**
9. **Voice abort UX knob.**
10. **Silero ONNX VAD.**
11. **Streaming ASR.**
12. **Wake-word activation.**
13. **macOS streaming variant.**
14. **Lock-free AudioIn detector.**
15. **Drive Activity API.**
16. **calendarList cache TTL knob.**
17. **access_role deprecation.**
18. **Budget category migration tool.**
19. **Budget currency / rust_decimal.**
20. **Multi-category trend breakdown.**
21. **Trend smoothing / moving average.**
22. **Bulk budget operations.**
23. **Proactive reminder dispatch.**
24. **Relative-time localization.**
25. **whisper-cpp-plus rehabilitation.**
26. **`build_agent_stack` substrate-tier
    promotion.**
27. **Channel Activation Milestone** —
    still held intentionally; 47th
    consecutive deferral at Phase 158
    open.

## Prediction vs reality

| Prediction | Reality | Held? |
| --- | --- | --- |
| DESIGN.md HOLD → 49 | Untouched | ✅ |
| PRODUCT.md HOLD → 49 | Untouched | ✅ |
| `aivyx-core/src/lib.rs` HOLD → 24 | Untouched | ✅ |
| Zero new workspace deps | Zero; everything used (chrono, tokio, futures-util) was already in workspace + crate manifest | ✅ |
| Zero clippy warnings | `cargo clippy --workspace --all-targets -- -D warnings` clean (after one mid-task `type_complexity` flag handled via type alias) | ✅ |
| Test count delta `+10` to `+16` | `+18` (190 → 208 net in `cargo test -p aivyx-calendar --lib`, after removing 2 obsolete `bucket_start_5min` cases) | ⚠️ (over by 2; see correction note below) |

All five exit criteria met. Three Phase 155 honest-
debts closed in one phase:

1. **Hard 5-min bucket flooring → sliding-window
   adjacency merge.** Closed in Task 2 (commit
   `906020a`). 10:04+10:06 now merge; chains like
   10:00→10:04→10:08 fold to one cluster.
2. **calendarList re-fetch on every call → 5-min
   session cache.** Closed in Task 3 (commit
   `05f0040`). Cache lives on `CalendarClient` and
   is shared across all tools that consume the
   client.
3. **No min_concurrent knob → `min_concurrent`
   input (cap 16) + parse-time min ≤ max
   validation.** Closed in Task 4 (commit
   `79d1855`).

### Test count delta correction

Open doc band was `+10` to `+16`. Actual delta is
`+18`:

- Task 2: -2 (removed `bucket_start_5min_*` cases)
  +6 (sliding-window cases) = net +4.
- Task 3: +5 (cache TTL pin + 4 cache-behavior).
- Task 4: +9 (4 min_concurrent parse + 2 cross-
  knob validation + 3 permit-clamp logic).

Total: +18, over the ceiling by 2. Honest correction.
The over-shoot came from Task 4 — the permit-
clamp logic warranted three direct-computation
tests in addition to the input-parse cases,
which the open doc's `+10..+16` band hadn't
budgeted for.

### What landed beyond the open

Nothing functional beyond the open. The over-band
test count is the only deviation, called out
above.

### Forty-seventh deferral of Channel Activation Milestone

Per operator framing — intentional hold. Recorded
for the record.
