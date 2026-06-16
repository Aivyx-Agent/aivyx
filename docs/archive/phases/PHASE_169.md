# Phase 169 — Phase 166 + 167 Retry/Filter Refinements (Actor Filter + 503/429 Retry + Jitter)

**Three small substrate follow-ons.** Same
bundle I offered as Recommended at Phase 168
sign-off; user picked Phase 161/165 stragglers
instead. Now that those are clear, Phase 169
takes this bundle. Closes one Phase 167 carry-
over and two Phase 166 carry-overs in one
phase.

## Why this, why now

- **Phase 167's lone open carry-over (actor
  filter) is small and stale.** Post-fetch
  shaping against the enriched output's
  actor_email field is the documented honest
  path; lands cleanly.

- **Phase 166 left two retry-loop carry-overs.**
  503/429 retry classification and backoff
  jitter both touch the same `send_with_retry`
  substrate; cohesive surface.

- **Zero new workspace deps.** Jitter uses
  stdlib `SystemTime` nanos as a PRNG seed
  (no `rand` crate; not cryptographic, but
  fine for backoff randomization).

## Tasks

1. **Open doc + ROADMAP + README.** This doc +
   roadmap section + README row. Backfill
   Phase 168 hash to `92fd777`.

2. **`actor_email_filter` on drive.recent_
   activity.** New input field accepting a
   case-insensitive substring/contains
   pattern. Applied post-fetch against each
   activity's enriched `actor_email` field.
   When set, the output `count` reflects the
   post-filter count and the `activities`
   array is trimmed. Honest scope risk: the
   Activity API doesn't support actor
   predicates in its filter DSL, so the
   filter operates after page-size truncation
   — a 100-page window where only 10 entries
   match the actor returns 10 results, not
   the next 100. Documented.

3. **503 / 429 retry classification.** Extend
   `send_with_retry` to also retry when the
   response comes back with HTTP 503 (Service
   Unavailable) or 429 (Too Many Requests).
   Currently retry fires only on `Err(_)`
   from `send()`; 5xx / 4xx responses are
   `Ok(response)` with the status set.
   Refactor the loop to inspect the response
   status post-`send()`; treat 503 / 429 as
   retry-eligible when attempts remain.
   Other 4xx / 5xx still bypass retry — those
   are operator-fixable.

4. **Backoff jitter.** New cfg field
   `url_retry_jitter_ms: u64` (default 0 = no
   jitter, preserves Phase 166 deterministic
   backoff). When set, each backoff delay is
   randomized within `±jitter_ms` of the
   computed exponential value. PRNG source:
   `SystemTime::now()
   .duration_since(UNIX_EPOCH).unwrap()
   .subsec_nanos() % (2 * jitter_ms + 1)` —
   stdlib-only, not cryptographic; fine for
   thundering-herd defense.

5. **INSTALL + exit + Frozen.** INSTALL.md
   updates for all three knobs; exit doc;
   README + ROADMAP Frozen flip.

## Streak predictions

- **DESIGN.md** — **Will hold.** Streak:
  5 → **6**.
- **PRODUCT.md** — **Will hold.** Streak:
  59 → **60**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  Streak: 5 → **6**.

## Exit criteria

- [ ] `docs/PHASE_169.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `actor_email_filter` honored post-fetch
  — Task 2.
- [ ] 503 / 429 trigger retry — Task 3.
- [ ] `url_retry_jitter_ms` randomizes
  backoff — Task 4.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+12` to `+22`.

## Honest scope risks at sign-off

- **`actor_email_filter` operates post-fetch
  AFTER page-size truncation.** The Activity
  API DSL has no native actor predicate;
  Phase 169 ships the honest post-fetch
  shape. Operators wanting a guaranteed N
  matching results must request a larger
  page-size and trust the filter to keep
  pace.

- **503 / 429 retry treats all attempts as
  retry-eligible.** Some endpoints surface
  503 for permanent maintenance windows;
  retrying just delays the failure surface.
  Operators with such endpoints can set
  `url_retry_count = 0` to disable. Same
  posture as Phase 166's "operator chooses
  whether to retry."

- **Jitter PRNG isn't cryptographic.**
  `SystemTime::subsec_nanos()` is
  predictable at sub-second resolution
  but fine for thundering-herd defense
  (any randomization breaks the
  synchronization pattern). Phase 170+
  candidate to swap in a real PRNG if a
  use case demands it.

- **`url_retry_jitter_ms = 0` means
  "no jitter," not "zero variance."**
  Same posture as Phase 166's
  `url_retry_count = 0` and Phase 168's
  `url_read_stall_secs = 0`.

- **Fifty-eighth consecutive deferral of
  the Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 169

After Phase 169, Phase 166 + 167 carry-over
ledgers reach zero. Phase 170+ candidates:

1. **Streaming document blocks** for large
   PDFs.
2. **Mid-recording or mid-reply /image
   command.**
3. **Clipboard-based image source.**
4. **Voice abort UX knob.** (Phase 146,
   long-deferred.)
5. **Silero ONNX VAD.**
6. **Streaming ASR.**
7. **Wake-word activation.**
8. **macOS streaming variant.**
9. **Lock-free AudioIn detector.**
10. **calendarList cache TTL knob.**
11. **access_role deprecation.**
12. **Budget category migration tool.**
13. **Budget currency / rust_decimal.**
14. **Multi-category trend breakdown.**
15. **Trend smoothing / moving average.**
16. **Bulk budget operations.**
17. **Proactive reminder dispatch.**
18. **Relative-time localization.**
19. **whisper-cpp-plus rehabilitation.**
20. **`build_agent_stack` substrate-tier
    promotion.**
21. **Secret-store integration** for
    url_headers.
22. **Full-document-compression PDF page
    count** (would require a real PDF
    parser dep — Phase 168 carry-over).
23. **Cryptographic PRNG for jitter** —
    Phase 169 carry-over.
24. **Channel Activation Milestone** —
    still held intentionally; 58th
    consecutive deferral at Phase 169
    open.

## Prediction vs reality

| Prediction | Reality | Held? |
| --- | --- | --- |
| DESIGN.md HOLD → 6 | Untouched | ✅ |
| PRODUCT.md HOLD → 60 | Untouched | ✅ |
| `aivyx-core/src/lib.rs` HOLD → 6 | Untouched | ✅ |
| Zero new workspace deps | All work used existing primitives; PRNG via stdlib SystemTime | ✅ |
| Zero clippy warnings | `cargo clippy --workspace --all-targets -- -D warnings` clean (after two `manual_range_contains` lints handled inline) | ✅ |
| Test count delta `+12` to `+22` | `+21` (actor_email_filter +11, 503/429 +4, jitter +6) | ✅ (within band) |

All three pieces closed:

1. **actor_email_filter** (Task 2, commit
   `fcd2843`). Closes Phase 167's lone open
   carry-over. Post-fetch substring match
   against the enriched actor_email field;
   output `count` reflects post-filter
   cardinality.
2. **503 / 429 retry classification** (Task
   3, commit `1f65df9`). Closes one of Phase
   166's two open carry-overs. Response-
   status path now retries 503/429 alongside
   the existing err-path classifying timeouts
   and connect failures.
3. **Backoff jitter** (Task 4, commit
   `936fe73`). Closes the other Phase 166
   carry-over. Stdlib SystemTime nanos as
   PRNG source — no new crate.

### What landed beyond the open

Nothing functional beyond the open. Test
count landed cleanly inside the predicted
band.

### Phase 166 + 167 honest-debt status — both ledgers zero

After Phase 169:
- Phase 167's three named carry-overs all
  closed: action_type_filter (P167 itself),
  consolidation knob (P167), parent_folder_id
  (P167), actor filter (P169 actor_email_
  filter).
- Phase 166's three named carry-overs all
  closed: PDF page cap knob (P166 itself),
  URL retry (P166), drive walk floor (P166);
  + 503/429 classification (P169) + backoff
  jitter (P169).

New Phase 169 carry-overs (cryptographic PRNG
for jitter, server-side 503 maintenance-window
edge case) feed Phase 170+ list.

### Fifty-eighth deferral of Channel Activation Milestone

Per operator framing — intentional hold.
Recorded for the record.
