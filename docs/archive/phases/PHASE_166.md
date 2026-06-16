# Phase 166 — Small Carry-Overs Bundle (PDF Page Cap Knob + URL Retry + Drive Walk Floor)

**Three small substrate carry-overs in one
phase.** One Phase 165 honest-debt, one Phase
161 carry-over, one Phase 160 carry-over. Each
piece touches a different crate; the bundle is
cohesive only in spirit (operator-tunable
knobs that the substrate-tier deferred to
follow-ons).

## Why this, why now

- **Phase 165's `ANTHROPIC_PDF_PAGE_CAP` is
  hardcoded.** Operators with custom Anthropic
  plans (higher per-document page support)
  can't override the 100-page cap.
- **Phase 161's URL fetch timeout has no
  retry.** Transient timeouts hard-fail; the
  operator manually retypes `/image <url>`.
- **Phase 160's `walk_max_concurrent` has no
  floor companion.** The Phase 158 calendar
  pattern shipped both `max_concurrent` AND
  `min_concurrent`; the drive walk has only
  the ceiling.

All three are operator-tunable surface
additions; no behavioral change at default.

## Tasks

1. **Open doc + ROADMAP + README.** This doc +
   the roadmap section + the README row.
   Backfill Phase 165 hash to `da63ace`.

2. **`anthropic_pdf_page_cap` config knob.**
   `AnthropicConfig` gains `pdf_page_cap: usize`
   field (default
   `ANTHROPIC_PDF_PAGE_CAP` = 100). New
   `with_pdf_page_cap(cap)` builder. Env-var
   fallback `AIVYX_ANTHROPIC_PDF_PAGE_CAP`
   checked in `AnthropicConfig::new` so
   operators can override without code
   changes. `build_request_body` reads the
   cap from the config rather than the
   constant; the constant becomes the named
   default. Tests cover the env-var fallback,
   the builder override, the default-cap
   path, and a regression that confirms the
   constant value is what builder defaults to.

3. **URL fetch retry on transient timeout.**
   `VoiceImageConfig` gains:
   - `url_retry_count: u32` (default 0 = no
     retry — preserves Phase 161 behavior).
   - `url_retry_backoff_ms: u64` (default
     500ms — base backoff; doubles per retry).
   `fetch_image_url` wraps both the HEAD
   pre-check and the GET in a loop that
   retries on `reqwest::Error::is_timeout()`
   AND `reqwest::Error::is_connect()` (both
   transient). 4xx / 5xx responses do NOT
   retry — operator content / server config
   errors aren't transient. Exponential
   backoff via `tokio::time::sleep`. Tests
   cover the parsing, the no-retry default,
   the retry count honored, the backoff
   formula.

4. **Drive walk `min_concurrent` companion.**
   Mirrors Phase 158's calendar.upcoming
   pattern. `recent_files` +
   `recent_changes` gain
   `walk_min_concurrent` input field (cap
   16, mirror calendar). Thread through
   `walk_folder_tree`'s permits clamp:
   `permits = clamp(default,
   walk_min_concurrent or 1,
   walk_max_concurrent or default)`. Tests
   cover the input parsing + the clamp
   computation.

5. **INSTALL + exit + Frozen.** INSTALL.md
   gets three updates: Anthropic page cap
   knob with env-var fallback,
   `[voice.image] url_retry_*` knobs, drive
   `walk_min_concurrent` knob. Exit doc +
   README + ROADMAP Frozen flip.

## Streak predictions

- **DESIGN.md** — **Will hold.** Streak:
  2 → **3**.
- **PRODUCT.md** — **Will hold.** Streak:
  56 → **57**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 166 work in aivyx-llm, aivyx-voice,
  aivyx-drive. Streak: 2 → **3**.

## Exit criteria

- [ ] `docs/PHASE_166.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `AnthropicConfig::pdf_page_cap` honored
  in `build_request_body`; env-var fallback
  works — Task 2.
- [ ] `VoiceImageConfig::url_retry_count` +
  `url_retry_backoff_ms` honored in
  `fetch_image_url` — Task 3.
- [ ] `walk_min_concurrent` input honored on
  both recent_* tools with min ≤ max
  validation — Task 4.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+12` to `+22`.

## Honest scope risks at sign-off

- **Env-var override is bypassable per
  process.** Operators sharing a machine
  who set
  `AIVYX_ANTHROPIC_PDF_PAGE_CAP` in a
  shared shell rc affect all aivyx
  processes. Documented; not a regression.

- **Retry loop doesn't recognize 503 / 429
  as transient.** Only client-side reqwest
  errors (timeout, connect failure) retry.
  Server-side rate limits surface as
  immediate errors; operator can pick a
  different URL or wait. Phase 167+
  candidate if surfaces.

- **Backoff is exponential without jitter.**
  Operators retrying against a single
  rate-limited origin during a flap could
  see thundering-herd patterns if multiple
  parallel attaches all retry at the same
  intervals. The voice attach is single-
  threaded today (one /image at a time),
  so this isn't load-bearing yet.

- **Drive `walk_min_concurrent` is bounded by
  level width, not by anything provider-
  enforceable.** A floor of 8 on a level
  with 3 folders fires 3 futures (not 8).
  The semaphore can't manufacture work that
  isn't there. Documented; mirrors Phase
  158 calendar's same behavior.

- **Fifty-fifth consecutive deferral of the
  Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 166

After Phase 166, three small carry-overs
clear. Phase 167+ candidates:

1. **drive.recent_activity actor / target
   filters.** (Phase 159 carry-over; 7
   phases stale.)
2. **drive.recent_activity consolidation knob.**
3. **drive.recent_activity + parent_folder_id
   composition.**
4. **Compressed-stream-aware PDF page count.**
   (Phase 165 carry-over — needs PDF parser
   dep.)
5. **Streaming document blocks** for large
   PDFs.
6. **503 / 429 retry classification on URL
   fetch.** (Phase 166 carry-over.)
7. **Backoff jitter on URL fetch retry.**
   (Phase 166 carry-over.)
8. **Read-stalled-bytes timeout (slow-trickle
   defense).** (Phase 161 carry-over.)
9. **Mid-recording or mid-reply /image
   command.**
10. **Clipboard-based image source.**
11. **Voice abort UX knob.**
12. **Silero ONNX VAD.**
13. **Streaming ASR.**
14. **Wake-word activation.**
15. **macOS streaming variant.**
16. **Lock-free AudioIn detector.**
17. **calendarList cache TTL knob.**
18. **access_role deprecation.**
19. **Budget category migration tool.**
20. **Budget currency / rust_decimal.**
21. **Multi-category trend breakdown.**
22. **Trend smoothing / moving average.**
23. **Bulk budget operations.**
24. **Proactive reminder dispatch.**
25. **Relative-time localization.**
26. **whisper-cpp-plus rehabilitation.**
27. **`build_agent_stack` substrate-tier
    promotion.**
28. **Secret-store integration** for
    url_headers.
29. **Channel Activation Milestone** —
    still held intentionally; 55th
    consecutive deferral at Phase 166
    open.

## Prediction vs reality

| Prediction | Reality | Held? |
| --- | --- | --- |
| DESIGN.md HOLD → 3 | Untouched | ✅ |
| PRODUCT.md HOLD → 57 | Untouched | ✅ |
| `aivyx-core/src/lib.rs` HOLD → 3 | Untouched | ✅ |
| Zero new workspace deps | All work used existing primitives (stdlib env, tokio::time, reqwest::Error) | ✅ |
| Zero clippy warnings | `cargo clippy --workspace --all-targets -- -D warnings` clean | ✅ |
| Test count delta `+12` to `+22` | `+24` (anthropic +7, voice +5, drive +12) | ⚠️ (over by 2; see correction) |

All five exit criteria functionally met. Three
carry-overs from three different parent phases
closed:

1. **`anthropic_pdf_page_cap` config knob**
   (Task 2, commit `4b5d2d3`). Phase 165's
   hardcoded constant becomes operator-tunable
   via builder + env var with safe fallback on
   invalid input.
2. **URL fetch retry** (Task 3, commit
   `207e613`). Phase 161's no-retry default
   preserved; operators opt in via
   `url_retry_count`. Exponential backoff
   with saturating-mul overflow safety.
3. **Drive walk `min_concurrent` companion**
   (Task 4, commit `5e438d8`). Mirrors Phase
   158 calendar pattern; min ≤ max validated
   at parse time.

### Honest correction — test count over-band

Open band was `+12..+22`; actual is `+24`.
The over-shoot came from Task 4: closing the
min/max companion forced updating the Phase
160 substrate test, which exposed a
semantically equivalent but bookkeeping-
different formula. I updated the test +
added 4 floor cases (lifts/within-default/
with-ceiling/floor-above-ceiling) + 5 input-
parse cases in recent_files + 3 mirror cases
in recent_changes = 12 tests for the drive
piece alone, vs. the open's 6-8 estimate.
The substrate-tier coverage warranted the
depth.

### Phase 166 substrate behavior shift documented

The drive walk's permits formula changed:
pre-166 returned `Some(max_concurrent)`
verbatim (semaphore could have more permits
than tasks); post-166 clamps to
`min(level_width, max_concurrent)` (semaphore
permits match actual concurrency). Real-world
behavior is identical — the semaphore can't
manufacture work that isn't there — but the
unit test that pinned the old bookkeeping
needed updating. Explanatory comment landed
inline at the changed assertion.

### Phase 160 + 161 + 165 honest-debt status

Three named carry-overs cleared, one per
parent phase. New Phase 166 carry-overs
(server 503/429 retry classification,
backoff jitter) feed Phase 167+ list.

### Fifty-fifth deferral of Channel Activation Milestone

Per operator framing — intentional hold.
Recorded for the record.
