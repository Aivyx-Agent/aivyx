# Phase 156 — Multimodal Close-Out Bundle (Multi-Image + URL Source + Size Cap)

**Phase 154 close-out, deferred 2 phases.**
Phase 154 shipped multimodal image attachment
via voice but with three documented honest-
debts:

1. **One image per turn.** `pending_image:
   Mutex<Option<(String, Vec<u8>)>>` —
   subsequent `set_pending_image` replaced
   prior. Operators wanting "describe these
   three screenshots" passed them across
   three turns.
2. **No client-side size cap.** A 50MB PNG
   got accepted at attach time; the LLM
   provider surfaced its own size error.
3. **Path-only source.** Operators with images
   at URLs had to download manually.

Phase 156 closes all three in one phase.
Symmetric to Phase 148/151/152/153/155 bundle
pattern.

## Why this, why now

- **Phase 154 ships a real feature; Phase 156
  rounds the edges.** Phase 154 was the
  Phase-N new feature; Phase 156 is the
  Phase-N+M close-out. Same shape as Phase
  143-144 budget CRUD or Phase 145-148 drive
  recent_*.

- **All three small individual scopes natural
  together.** Multi-image queue is a Vec
  swap. URL source is one new code path
  alongside file load. Size cap is one
  comparison + const.

- **`reqwest` already in workspace.**
  aivyx-voice doesn't have it as a crate dep
  yet; Phase 156 adds it (matching Phase 141's
  chrono pattern). Workspace dep count
  unchanged.

## Tasks

1. **Open doc + ROADMAP + README** — this doc +
   the roadmap section + the README row.
   Backfill Phase 155 hash to `ad84b49`.

2. **Multi-image queue substrate + size cap.**
   Two changes:
   - VoiceChannel `pending_image:
     Mutex<Option<(String, Vec<u8>)>>` becomes
     `pending_images: Mutex<Vec<(String,
     Vec<u8>)>>`. Methods rename:
     `set_pending_image` →
     `append_pending_image`;
     `take_pending_image` →
     `take_pending_images` (returns Vec).
   - `run_one_voice_turn_streaming` consumes
     via `take_pending_images()`; on non-empty
     Vec, constructs `MessageContent::Mixed`
     directly with one `ContentPart::Text` +
     N `ContentPart::Image` entries. On empty
     Vec, falls back to `Message::text` (Phase
     138-153 behavior).
   - Add `MAX_IMAGE_SIZE_BYTES = 10 *
     1024 * 1024` (10MB) const. Apply in
     `load_image_for_attach` after read +
     before queueing. Operator sees a clear
     "image too large" error.
   Tests: append + take Vec, multi-image
   round-trip, size-cap rejection.

3. **URL-based image source via reqwest.**
   Add `reqwest = { workspace = true }` to
   aivyx-voice Cargo.toml. In
   `load_image_for_attach`, detect `http://`
   or `https://` prefix on the argument:
   - When present, fetch via blocking reqwest
     (or short tokio::runtime::Handle since we
     already have async context — the streaming
     PTT loop is async). Infer media type
     from the `Content-Type` response header
     (`image/png`, `image/jpeg`, etc.).
     Fall back to extension inference if
     the header is absent/unrecognized.
     Apply size cap. Return bytes.
   - When absent, existing file-read path
     stays unchanged.
   Tests: URL detection + media-type-from-
   header substrate (no live HTTP).

4. **INSTALL + exit + Frozen.** INSTALL.md
   voice section updates: /image command
   now documents URL source + multi-image
   queue + size cap. Phase 156 exit doc with
   prediction-vs-reality. README + ROADMAP
   flip Phase 156 to Frozen.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment; substrate refinement + URL
  source. Streak: 46 → **47**.
- **PRODUCT.md** — **Will hold.** Streak:
  46 → **47**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 156 work in `aivyx-voice`. Core
  untouched. Streak: 21 → **22**.

## Exit criteria

- [ ] `docs/PHASE_156.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `VoiceChannel::append_pending_image` +
  `take_pending_images` public + tested;
  Message::text_with_image plumbing in
  run_one_voice_turn_streaming uses
  MessageContent::Mixed for multi-image —
  Task 2.
- [ ] `MAX_IMAGE_SIZE_BYTES` enforced in
  load_image_for_attach — Task 2.
- [ ] URL source path + reqwest crate dep +
  Content-Type media type inference —
  Task 3.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies
  (reqwest already in workspace).
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+5` to `+12`.

## Honest scope risks at sign-off

- **API break: append_pending_image +
  take_pending_images.** Phase 154's
  set/take methods are gone. Any external
  caller would break. No external callers
  today; the renames + Vec promotion are
  the cleanest shape.

- **`MAX_IMAGE_SIZE_BYTES = 10MB` is
  opinionated.** Matches the existing Drive
  inline cap (Phase 129's
  `CONTENT_INLINE_CAP_BYTES`). Phase 157+
  candidate for operator-tunable cap if
  surfaces.

- **URL fetch has no auth.** Phase 156 only
  works for public URLs (or URLs the
  operator's outbound network has
  pre-configured auth for, e.g. cookies).
  Authenticated image URLs (Drive sharing
  links, S3 signed URLs without query
  params) won't work. Phase 157+ candidate.

- **URL fetch has no timeout.** Operator
  attempting `/image
  https://slow-host/image.png` may hang the
  PTT loop. Phase 157+ candidate for
  configurable timeout.

- **No HEAD pre-fetch for size check.**
  Phase 156 fetches the body first, then
  checks size. A 100MB URL still pulls
  100MB over the wire before the size
  rejection fires. Phase 157+ optimization.

- **Content-Type header trusted.** A server
  could return `Content-Type: image/png`
  for non-image content; reqwest doesn't
  validate magic bytes. The LLM provider
  may surface its own error downstream.
  Acceptable.

- **Forty-fifth consecutive deferral of the
  Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 156

After Phase 156, multimodal input shape
matures. Phase 157+ candidates:

1. **Operator-tunable image size cap.**
2. **URL fetch timeout.**
3. **HEAD pre-fetch for size check.**
4. **Authenticated URL fetch.**
5. **PDF / SVG / TIFF media type support.**
6. **Mid-recording or mid-reply /image
   command.**
7. **Clipboard-based image source.**
8. **Sliding-window fuzzy time** for
   adjacent-bucket merging.
9. **calendarList session caching.**
10. **min_concurrent knob.**
11. **Parallel walk_folder_tree.**
12. **Recursive walk within drive_id
    scope.**
13. **Operator-tunable recursive caps.**
14. **Voice abort UX knob.**
15. **Silero ONNX VAD.**
16. **Streaming ASR.**
17. **Wake-word activation.**
18. **macOS streaming variant.**
19. **Lock-free AudioIn detector.**
20. **access_role deprecation.**
21. **Budget category migration tool.**
22. **Budget currency / rust_decimal.**
23. **Multi-category trend breakdown.**
24. **Trend smoothing / moving average.**
25. **Bulk budget operations.**
26. **Drive Activity API.**
27. **Proactive reminder dispatch.**
28. **Relative-time localization.**
29. **whisper-cpp-plus rehabilitation.**
30. **`build_agent_stack` substrate-tier
    promotion.**
31. **Channel Activation Milestone** —
    still held intentionally; 45th
    consecutive deferral at Phase 156
    open.

## Prediction vs reality

_Populated at Phase 156 exit. Predictions at
sign-off: DESIGN.md HOLD → 47; PRODUCT.md HOLD
→ 47; lib.rs HOLD → 22; zero new deps; test
count delta `+5` to `+12`; zero clippy
warnings._
