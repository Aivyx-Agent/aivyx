# Phase 168 — Phase 161 + 165 Stragglers (Read-Stalled-Bytes Timeout + Catalog-Aware PDF Page Count)

**Two unrelated honest-debts in one phase.**
Phase 161 named "read-stalled-bytes timeout
(slow-trickle defense)" as a follow-on. Phase
165 named "compressed-stream-aware PDF page
count" as a follow-on. Phase 168 picks both up.
Smaller bundle than typical (2-piece vs 3) and
genuinely harder than typical — both pieces
involve substrate-level depth.

## Why this, why now

- **Both are open carry-overs from active-
  surface phases.** Phase 161 (multimodal
  URL fetch) and Phase 165 (Anthropic PDF
  guard) both ship behaviors that are
  defeatable by edge cases the carry-overs
  address.

- **Neither requires DESIGN.md amendment.**
  Pure substrate additions.

- **Both touch a single function each.** Read-
  stall via `fetch_image_url`'s body-read
  path; PDF count via `count_pdf_pages_best_
  effort`. Low blast radius.

- **Zero new workspace deps target.** Phase
  165 set the precedent for the PDF piece by
  byte-scanning rather than adding a parser
  dep; Phase 168 continues that posture.

## Tasks

1. **Open doc + ROADMAP + README.** This doc +
   roadmap section + README row. Backfill
   Phase 167 hash to `75799bd`.

2. **Read-stalled-bytes timeout.** Switch
   `fetch_image_url`'s body read from
   `resp.bytes().await` (which only respects
   the overall request timeout) to a streaming
   read via `resp.bytes_stream()` wrapped in
   per-chunk `tokio::time::timeout`. New cfg
   field:
   ```toml
   [voice.image]
   url_read_stall_secs = 5  # Default 0 = disabled
                            # (preserves Phase 161 behavior).
   ```
   When set, the substrate aborts the body
   read if no bytes arrive within the
   configured window — defeating slow-trickle
   attacks where a server returns 1 byte per
   second and the per-request timeout never
   fires.
   Tests cover the configuration parsing +
   the substrate logic (mock the chunk
   stream); the live-network case stays
   operator-validation tier per Phase 161's
   precedent.

3. **Compressed-stream-aware PDF page count.**
   Augment `count_pdf_pages_best_effort` to
   also scan for `/Type /Pages /Count N`
   markers (the PDF catalog's declared total
   page count). The PDF root catalog object
   typically lives outside compressed object
   streams; reading the declared count is
   accurate for any PDF whose catalog is
   visible. Returns
   `max(per_page_scan, declared_count)` so
   uncompressed PDFs still count correctly
   AND compressed PDFs surface their declared
   total. Tests cover synthetic PDFs with
   both shapes.

4. **INSTALL + exit + Frozen.** INSTALL.md
   updates for both knobs; exit doc with
   prediction-vs-reality; README + ROADMAP
   Frozen flip.

## Streak predictions

- **DESIGN.md** — **Will hold.** Streak:
  4 → **5**.
- **PRODUCT.md** — **Will hold.** Streak:
  58 → **59**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  Phase 168 work in `aivyx-voice` +
  `aivyx-llm`. Streak: 4 → **5**.

## Exit criteria

- [ ] `docs/PHASE_168.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `url_read_stall_secs` honored in
  `fetch_image_url` — Task 2.
- [ ] `count_pdf_pages_best_effort` returns
  the max of per-page scan and declared
  catalog count — Task 3.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+8` to `+15`.

## Honest scope risks at sign-off

- **Read-stall test coverage is substrate-
  tier only.** Like Phase 161's timeout
  test, verifying the stall-detection
  against a real slow-trickle server would
  need either a new dev-dep mock server or
  multi-second test sleeps. Substrate
  logic-level tests pin the chunk-iteration
  branch; live behavior is operator-
  validation.

- **`url_read_stall_secs = 0` is "disabled",
  not "zero seconds."** A non-zero stall
  window is the only way to enable. Same
  posture as Phase 166's `url_retry_count =
  0` meaning "no retry."

- **Catalog `/Count` can be wrong on
  malformed PDFs.** Some PDF writers
  generate inconsistent metadata (declared
  Count differs from actual page tree
  size). Taking the max means we
  over-estimate, not under-estimate, which
  is the safe direction for a cap check
  (false positive on the cap = operator
  sees a clear error; false negative would
  let oversized PDFs through silently).

- **Catalog could itself be inside a
  compressed object stream.** When the
  trailer xref + catalog object are all
  packed in a compressed object stream
  (rare but legal — xref streams from PDF
  1.5+ with full-document compression),
  both per-page and declared count miss.
  This is the edge case that genuinely
  requires a PDF parser dep; Phase 168
  surfaces 0 and Anthropic's server-side
  cap handles the rest.

- **Nested Pages trees double-count.**
  Large PDFs use a tree of `/Type /Pages`
  nodes — each non-root inner node has its
  own /Count covering its subtree only.
  Phase 168 takes the max across all
  `/Count` values seen, so we land at the
  root's count (the largest). If a PDF
  ever inverts that invariant (inner
  /Count > root /Count, which is illegal
  per spec), we'd over-estimate; safe
  direction.

- **Fifty-seventh consecutive deferral of
  the Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 168

After Phase 168, Phase 161 + 165's deepest
honest-debts clear. Phase 169+ candidates:

1. **Actor email post-fetch filter** for
   `drive.recent_activity`. (Phase 167
   carry-over.)
2. **Streaming document blocks** for large
   PDFs.
3. **503 / 429 retry classification on URL
   fetch.** (Phase 166 carry-over.)
4. **Backoff jitter on URL fetch retry.**
   (Phase 166 carry-over.)
5. **Mid-recording or mid-reply /image
   command.**
6. **Clipboard-based image source.**
7. **Voice abort UX knob.**
8. **Silero ONNX VAD.**
9. **Streaming ASR.**
10. **Wake-word activation.**
11. **macOS streaming variant.**
12. **Lock-free AudioIn detector.**
13. **calendarList cache TTL knob.**
14. **access_role deprecation.**
15. **Budget category migration tool.**
16. **Budget currency / rust_decimal.**
17. **Multi-category trend breakdown.**
18. **Trend smoothing / moving average.**
19. **Bulk budget operations.**
20. **Proactive reminder dispatch.**
21. **Relative-time localization.**
22. **whisper-cpp-plus rehabilitation.**
23. **`build_agent_stack` substrate-tier
    promotion.**
24. **Secret-store integration** for
    url_headers.
25. **Channel Activation Milestone** —
    still held intentionally; 57th
    consecutive deferral at Phase 168 open.

## Prediction vs reality

_Populated at Phase 168 exit. Predictions at
sign-off: DESIGN.md HOLD → 5; PRODUCT.md HOLD
→ 59; lib.rs HOLD → 5; zero new deps; test
count delta `+8` to `+15`; zero clippy
warnings._
