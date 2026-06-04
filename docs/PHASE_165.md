# Phase 165 — Multimodal Stragglers Bundle (Office Formats + Per-URL Headers + PDF Page Cap)

**Three multimodal honest-debts in one phase.**
Phase 164 named DOC/RTF/ODT/pptx/xlsx inference
+ PDF page-count cap as candidates. Phase 162
named per-URL header overrides as a carry-over.
Phase 165 bundles all three. Symmetric to the
Phase 157/158/160/161 close-out cadence.

## Why this, why now

- **Phase 164 just shipped DOCX inference.**
  The matcher pattern is warm; adding the four
  other Office formats is a one-line-each
  extension.
- **Phase 162's per-URL header overrides have
  aged 3 phases.** Operators with multiple
  authenticated origins still pick one set of
  global headers.
- **Phase 164's Anthropic model guard sets the
  pre-flight precedent.** A document
  page-count cap fits the same shape: refuse
  client-side with a clear error before the
  API surfaces an opaque rejection.
- **Zero new workspace deps.** Each piece uses
  existing primitives. The PDF page-count
  cap uses a byte-scan rather than a real PDF
  parser — honest scope risk documented below.

## Tasks

1. **Open doc + ROADMAP + README.** This doc +
   the roadmap section + the README row.
   Backfill Phase 164 hash to `7a0de2d`.

2. **DOC / RTF / ODT / pptx / xlsx media type
   inference.** Extend the Phase 164 matchers:
   - `.doc` → `application/msword`
     (also classified as Document for routing
     parity with DOCX).
   - `.rtf` → `application/rtf` (Document).
   - `.odt` →
     `application/vnd.oasis.opendocument.text`
     (Document).
   - `.pptx` →
     `application/vnd.openxmlformats-
     officedocument.presentationml.presentation`
     (Document — slides count as document
     content for our routing).
   - `.xlsx` →
     `application/vnd.openxmlformats-
     officedocument.spreadsheetml.sheet`
     (Document).
   Add MIME constants. Extend
   `is_document_media_type`.
   Honest scope risk: Anthropic accepts PDF
   only as of writing. All five formats
   surface 400 from the API; the inference
   surface lands so provider widening doesn't
   need voice-side work — same posture as
   DOCX in Phase 164.

3. **Per-URL header overrides via TOML
   presets.** New `[voice.image.url_header_presets]`
   TOML section:
   ```toml
   [voice.image.url_header_presets.work]
   Authorization = "Bearer aaa"

   [voice.image.url_header_presets.personal]
   Cookie = "session=bbb"
   ```
   `/image <url> --headers work` selects the
   `work` preset for this attach. Without
   `--headers`, the global
   `[voice.image.url_headers]` map applies
   (Phase 162 behavior, unchanged).
   Parse the `--headers <name>` flag in the
   voice command parser; pass the resolved
   preset map into `load_image_for_attach`.

4. **Best-effort PDF page-count cap on
   Anthropic.** Two parts:
   - **Substrate.** A `count_pdf_pages(&[u8])`
     helper in `aivyx-llm/src/anthropic/`
     that does a byte-scan for `/Type` followed
     by whitespace + `/Page` followed by a
     non-`s` byte (so `/Pages` is excluded).
     Honest scope risk: this misses pages
     inside compressed object streams
     (modern PDFs commonly use FlateDecode-
     wrapped xref + object streams). Operators
     with compressed PDFs see false negatives
     — the cap doesn't fire, the document
     goes to the API, and any provider-side
     cap surfaces normally.
   - **Pre-flight check.** Anthropic provider
     `build_request_body` scans for
     `ContentBlock::DocumentBase64` with
     `media_type == "application/pdf"`,
     decodes the base64, counts pages, refuses
     if count exceeds the cap.
     New constant `ANTHROPIC_PDF_PAGE_CAP:
     usize = 100` (matches Anthropic's
     documented per-document cap). Honest
     scope risk: hardcoded; operators with
     custom Anthropic plans can't override.
     Phase 166+ candidate for a config knob.

5. **INSTALL + exit + Frozen.** INSTALL.md
   notes the new MIMEs, presets, page-cap
   semantics + honest limitations. Exit doc.
   README + ROADMAP Frozen flip.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment; substrate additions only. Streak:
  1 → **2**.
- **PRODUCT.md** — **Will hold.** Streak:
  55 → **56**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 165 work in `aivyx-voice` +
  `aivyx-llm`. Streak: 1 → **2**.

## Exit criteria

- [ ] `docs/PHASE_165.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] Five Office formats inferred and routed
  as Document — Task 2.
- [ ] `--headers <preset>` flag honored on
  `/image <url>` — Task 3.
- [ ] Anthropic provider refuses PDFs with
  best-effort page-count > cap — Task 4.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+15` to `+25`.

## Honest scope risks at sign-off

- **Office formats land but Anthropic only
  accepts PDF.** Same posture as Phase 164's
  DOCX — the inference surface lands; provider
  rejection surfaces as a per-attach error.

- **PDF page-count byte-scan misses compressed
  pages.** Modern PDFs commonly use
  FlateDecode object streams; the byte-scan
  sees an opaque blob instead of `/Type
  /Page` markers. The false negative means
  the cap doesn't fire; document still goes
  to Anthropic; Anthropic's own page-count
  enforcement surfaces a 400 as before. The
  cap is *additive defense* for uncompressed
  PDFs (the common case for PDFs generated
  by older tools, scans, command-line
  utilities); modern Acrobat-generated PDFs
  may skip it.

- **`ANTHROPIC_PDF_PAGE_CAP` is hardcoded.**
  Phase 166+ candidate for a TOML knob.
  Operators with custom Anthropic plans
  (higher than 100 page support) can't
  override yet.

- **`--headers` syntax adds command-parser
  surface.** The voice `/image` command was
  argument-free (just the path/URL). Phase
  165 adds one optional flag. The parser
  needs to gracefully handle missing
  preset-name, invalid preset-name, and the
  case where the path/URL itself contains
  `--headers` (unlikely but possible).

- **Five new MIMEs widen the substrate's
  classification surface.** A new format
  added in Phase 166+ continues the
  one-line-per-format pattern; no structural
  change.

- **Fifty-fourth consecutive deferral of the
  Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 165

After Phase 165, Phase 164's stragglers + Phase
162's per-URL headers carry-over clear. Phase
166+ candidates:

1. **drive.recent_activity actor / target
   filters.** (Phase 159 carry-over; 6
   phases stale.)
2. **drive.recent_activity consolidation knob.**
   (Phase 159 carry-over.)
3. **drive.recent_activity + parent_folder_id
   composition.** (Phase 159 carry-over.)
4. **`ANTHROPIC_PDF_PAGE_CAP` TOML knob.**
   (Phase 165 carry-over.)
5. **Compressed-stream-aware PDF page count.**
   (Phase 165 carry-over — needs a real
   PDF parser dep.)
6. **Streaming document blocks** for large
   PDFs.
7. **Read-stalled-bytes timeout (slow-trickle
   defense).** (Phase 161 carry-over.)
8. **Retry on transient image-fetch timeout.**
   (Phase 161 carry-over.)
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
18. **drive walk min_concurrent companion.**
19. **access_role deprecation.**
20. **Budget category migration tool.**
21. **Budget currency / rust_decimal.**
22. **Multi-category trend breakdown.**
23. **Trend smoothing / moving average.**
24. **Bulk budget operations.**
25. **Proactive reminder dispatch.**
26. **Relative-time localization.**
27. **whisper-cpp-plus rehabilitation.**
28. **`build_agent_stack` substrate-tier
    promotion.**
29. **Secret-store integration** for
    url_headers.
30. **Channel Activation Milestone** —
    still held intentionally; 54th
    consecutive deferral at Phase 165 open.

## Prediction vs reality

_Populated at Phase 165 exit. Predictions at
sign-off: DESIGN.md HOLD → 2; PRODUCT.md HOLD
→ 56; lib.rs HOLD → 2; zero new deps; test
count delta `+15` to `+25`; zero clippy
warnings._
