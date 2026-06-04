# Phase 162 — Phase 156 + 161 Stragglers (PDF/SVG/TIFF + Authenticated URL Fetch)

**Phase 156's last two carry-overs.** Phase 156
shipped multi-image + URL source + size cap with
five honest-debts at exit. Phase 161 closed three
(size cap tunable, URL timeout, HEAD pre-fetch).
Phase 162 closes the remaining two:

1. **PDF / SVG / TIFF media type support.**
   Phase 156's `infer_image_media_type` and
   `media_type_from_content_type` accept only
   png/jpg/jpeg/gif/webp. Operators with PDF
   documents or SVG diagrams pass the file
   path / URL and hit `"unsupported image
   extension"` — even though the downstream
   `ContentPart::Image::media_type` field is an
   opaque string.
2. **Authenticated URL fetch.** Phase 156's
   `fetch_image_url` does a bare GET — no
   authorization headers, no cookies. Operators
   wanting to attach an image behind an API
   token, a session cookie, or a custom origin
   header had no path.

Phase 162 closes both. Smaller surface than the
typical 3-honest-debt bundle (only 2 left), but
the same close-out shape.

## Why this, why now

- **Phase 156's debt ledger goes to zero.**
  After Phase 161 dropped from 5 to 2, Phase
  162 takes it to 0. Symmetric to the
  Phase-156-closes-Phase-154 / Phase-161-
  closes-Phase-156 cadence.

- **Both touch the same module.** PDF/SVG/TIFF
  inference extends `infer_image_media_type`
  and `media_type_from_content_type` in
  `session.rs`. The url_headers map extends
  `VoiceImageConfig` + `fetch_image_url` in the
  same file. One module, one review surface.

- **Zero new workspace deps.** `HashMap<String,
  String>` is stdlib; `reqwest::header` API is
  already used; serde already deserializes
  TOML tables.

## Tasks

1. **Open doc + ROADMAP + README.** This doc +
   roadmap section + README row. Backfill
   Phase 161 hash to `7d8c293`.

2. **PDF / SVG / TIFF media type support.**
   - `infer_image_media_type` adds branches
     for `.pdf` → `application/pdf`, `.svg` →
     `image/svg+xml`, `.tif` / `.tiff` →
     `image/tiff`.
   - `media_type_from_content_type` adds
     branches for `application/pdf`,
     `image/svg+xml`, and `image/tiff`.
   - Tests pin both surfaces.
   - Schema doc updated to list the new
     supported types + an honest caveat that
     the LLM provider may reject anything
     beyond png/jpg/gif/webp depending on
     model and content-block shape (PDFs in
     particular typically need a document
     block, not an image block — Phase 162
     passes the media_type through opaquely
     and lets the provider's response surface
     the error).

3. **Authenticated URL fetch via TOML header
   map.**
   - `VoiceImageConfig` gains `url_headers:
     std::collections::HashMap<String,
     String>` (defaults to empty).
   - `fetch_image_url` applies these headers
     to both the HEAD pre-check and the GET.
   - Empty map = unchanged behavior (no
     headers). Operators add:
     ```toml
     [voice.image.url_headers]
     Authorization = "Bearer xxx"
     Cookie        = "session=yyy"
     Origin        = "https://example.com"
     ```
   - Sanity: header names containing invalid
     ASCII characters get surfaced as a clear
     error rather than panicked over.
   - Honest scope risk: the TOML config file
     is the only place these credentials
     live. Operators must protect file
     permissions; documented in INSTALL.md.

4. **INSTALL + exit + Frozen.** INSTALL.md
   voice section: extend the Phase 161
   `[voice.image]` block example with
   `[voice.image.url_headers]`, document the
   provider-caveat for PDF/SVG/TIFF, document
   the file-permission caveat for url_headers.
   Exit doc + README + ROADMAP Frozen flip.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment; substrate additions only. Streak:
  52 → **53**.
- **PRODUCT.md** — **Will hold.** Streak:
  52 → **53**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  `ContentPart::Image::media_type` is already
  an opaque String; no core change needed.
  Streak: 27 → **28**.

## Exit criteria

- [ ] `docs/PHASE_162.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] PDF / SVG / TIFF inferred from both
  extension and Content-Type — Task 2.
- [ ] `url_headers` map honored on HEAD + GET
  — Task 3.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+6` to `+12`.

## Honest scope risks at sign-off

- **PDF as image block may 400 at the LLM
  provider.** Anthropic's API has separate
  document blocks for PDFs (Claude 3.5+).
  Phase 162 passes PDFs as
  `ContentPart::Image::media_type =
  "application/pdf"`; the provider may reject.
  Operators who hit this take the error as
  feedback that PDF-document-block plumbing
  is a Phase 163+ candidate (would touch
  core's `ContentPart` enum).

- **SVG / TIFF provider support varies.** Most
  vision LLMs accept PNG/JPG/GIF/WebP only.
  SVG and TIFF will likely return a 400
  "unsupported media type" from the provider.
  Documented; same posture as PDF.

- **url_headers credentials live in plaintext
  TOML.** Aivyx config files are operator-
  managed; secret-store integration is a
  larger Phase 165+ candidate. INSTALL.md
  documents the `chmod 600` recommendation.

- **No per-URL header overrides.** All
  `/image <url>` calls in a session use the
  same `[voice.image.url_headers]` block.
  Operators with multiple authenticated
  origins must pick one. Phase 163+
  candidate if surfaces.

- **Header value sanity is limited.** We
  surface `reqwest::header::InvalidHeaderName`
  / `InvalidHeaderValue` errors as build-time
  failures during config load; we don't strip
  control characters or validate against
  scoped header names. Operators with
  malformed headers see a clear error from
  reqwest.

- **Fifty-first consecutive deferral of the
  Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 162

After Phase 162, the Phase 156 multimodal debt
ledger reaches zero. Phase 163+ candidates:

1. **drive.recent_activity actor / target
   filters.** (Phase 159 carry-over.)
2. **drive.recent_activity consolidation knob.**
   (Phase 159 carry-over.)
3. **drive.recent_activity + parent_folder_id
   composition.** (Phase 159 carry-over.)
4. **PDF document-block plumbing in core.**
   (New Phase 162 carry-over: ContentPart enum
   gains `Document { media_type, data }` so
   PDFs route to Anthropic's document block.)
5. **Per-URL header overrides** for
   `/image <url>`.
6. **Read-stalled-bytes timeout (slow-trickle
   defense).** (Phase 161 carry-over.)
7. **Retry on transient image-fetch timeout.**
   (Phase 161 carry-over.)
8. **Mid-recording or mid-reply /image
   command.**
9. **Clipboard-based image source.**
10. **Voice abort UX knob.**
11. **Silero ONNX VAD.**
12. **Streaming ASR.**
13. **Wake-word activation.**
14. **macOS streaming variant.**
15. **Lock-free AudioIn detector.**
16. **calendarList cache TTL knob.**
17. **drive walk min_concurrent companion.**
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
    still held intentionally; 51st
    consecutive deferral at Phase 162 open.

## Prediction vs reality

_Populated at Phase 162 exit. Predictions at
sign-off: DESIGN.md HOLD → 53; PRODUCT.md HOLD
→ 53; lib.rs HOLD → 28; zero new deps; test
count delta `+6` to `+12`; zero clippy
warnings._
