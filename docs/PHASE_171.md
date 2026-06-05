# Phase 171 — Loose Ends Bundle (GIF/WebP Clipboard + calendarList Cache TTL Knob)

**Capstone phase before extended review.** Two
small substrate cleanups: Phase 170's just-
shipped clipboard signature carry-over and
Phase 158's long-stale calendarList cache TTL
knob. Small surface; clean closure before the
Aivyx Agent Review artifact lands.

## Why this, why now

- **Phase 170's GIF/WebP carry-over is one
  line each.** PNG + JPEG signature detection
  shipped Phase 170 Task 3; GIF + WebP add the
  next two operator-common image formats.

- **Phase 158's calendarList cache TTL is 13
  phases stale.** The 5-minute hardcoded TTL
  has been on the carry-over list since Phase
  158 Task 3. Operators with rapidly changing
  calendar lists (workspace admin scenarios)
  can't shorten it.

- **Zero new workspace deps.** Pure substrate
  additions.

## Tasks

1. **Open doc + ROADMAP + README.** This doc +
   roadmap section + README row. Backfill
   Phase 170 hash to `7bc29db`.

2. **GIF + WebP clipboard byte signatures.**
   Extend `infer_clipboard_media_type` to
   recognize:
   - GIF: `0x47 0x49 0x46` (`GIF`)
   - WebP: `0x52 0x49 0x46 0x46` (`RIFF`)
     followed by 4-byte size and `0x57 0x45
     0x42 0x50` (`WEBP`) at offset 8
   Tests pin both signatures + the existing
   PNG/JPEG paths.

3. **calendarList cache TTL knob.** Promote
   `WRITABLE_CALENDARS_CACHE_TTL` from a
   hardcoded constant to a
   `CalendarClient::writable_calendars_cache_ttl`
   field with builder method + env var
   fallback `AIVYX_CALENDAR_CACHE_TTL_SECS`.
   Default 300 (matches Phase 158).

4. **INSTALL + exit + Frozen + Agent Review
   prep.** INSTALL.md notes both pieces; exit
   doc; README + ROADMAP Frozen flip. The
   `docs/AGENT_REVIEW_<date>.md` artifact
   lands separately after Phase 171 commits.

## Streak predictions

- **DESIGN.md** — **Will hold.** Streak:
  7 → **8**.
- **PRODUCT.md** — **Will hold.** Streak:
  61 → **62**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  Streak: 7 → **8**.

## Exit criteria

- [ ] `docs/PHASE_171.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] GIF + WebP recognized by
  `infer_clipboard_media_type` — Task 2.
- [ ] `CalendarClient::writable_calendars_
  cache_ttl` honored — Task 3.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+5` to `+10`.

## Honest scope risks at sign-off

- **Clipboard signature detection still
  rejects TIFF / BMP / HEIC etc.** Phase
  170 supports png/jpeg + Phase 171 adds
  gif/webp; everything else surfaces a
  clear "unrecognized signature" error.

- **`AIVYX_CALENDAR_CACHE_TTL_SECS=0`
  effectively disables the cache.**
  Acceptable; documented. Same posture as
  Phase 166's env-var validation
  (zero-falls-back-to-default would
  surprise operators who explicitly set 0
  to disable).

- **Sixtieth consecutive deferral of the
  Channel Activation Milestone.** Per
  operator framing — intentional hold.
  Round-number deferral count.

## Direction after Phase 171

After Phase 171, the project enters extended
review. The Aivyx Agent Review artifact
documents the current state. Phase 172+
candidates remain on the roster:

1. **Cryptographic PRNG for jitter.**
   (Phase 169 carry-over.)
2. **Full-document-compression PDF page
   count.** (Phase 168 carry-over.)
3. **Streaming document blocks** for large
   PDFs.
4. **`arboard`-based cross-platform
   clipboard** (Phase 170 carry-over).
5. **Silero ONNX VAD.**
6. **Streaming ASR.**
7. **Wake-word activation.**
8. **macOS streaming variant.**
9. **Lock-free AudioIn detector.**
10. **access_role deprecation.**
11. **Budget category migration tool.**
12. **Budget currency / rust_decimal.**
13. **Multi-category trend breakdown.**
14. **Trend smoothing / moving average.**
15. **Bulk budget operations.**
16. **Proactive reminder dispatch.**
17. **Relative-time localization.**
18. **whisper-cpp-plus rehabilitation.**
19. **`build_agent_stack` substrate-tier
    promotion.**
20. **Secret-store integration** for
    url_headers.
21. **Channel Activation Milestone** —
    still held intentionally; 60th
    consecutive deferral at Phase 171
    open.

## Prediction vs reality

_Populated at Phase 171 exit. Predictions at
sign-off: DESIGN.md HOLD → 8; PRODUCT.md HOLD
→ 62; lib.rs HOLD → 8; zero new deps; test
count delta `+5` to `+10`; zero clippy
warnings._
