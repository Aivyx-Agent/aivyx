# Phase 164 — Phase 163 Cleanups (Anthropic Model Guard + DOCX Inference)

**Phase 163 just-shipped follow-ons.** Phase 163
(amendment A13) landed the `Document` variant
end-to-end. Phase 164 picks up two of its named
honest-debts:

1. **No provider-side PDF content-validation
   pre-check.** Phase 163's exit doc named this
   as a Phase 164+ candidate: when an operator
   attaches a PDF and their selected Anthropic
   model is pre-Claude-3.5, the API surfaces a
   400 from the model side. Phase 164 catches
   this client-side with a clear error before
   the API call.
2. **DOCX (and other Office formats) not yet
   inferred.** Phase 162 added PDF/SVG/TIFF
   extension matchers but not DOCX. Phase 164
   adds the inference and routes DOCX via the
   new `Document` variant. Provider rejection
   on Anthropic surfaces as a 400 — same
   posture as Phase 162's SVG/TIFF treatment;
   the surface area exists for when provider
   support widens.

Smaller scope than the typical 3-debt bundle.
Phase 163's other named carry-over ("remove
the now-dead `Message::text_with_image`
constructor") turned out to be **not dead** —
Telegram + channel still call it. Sub-task
dropped honestly.

## Why this, why now

- **Phase 163 just landed amendment A13.**
  The fresh substrate is warm; the two
  cleanups land cleanly against it.

- **Both touch a single provider module +
  voice module.** Anthropic model guard in
  `aivyx-llm/src/anthropic/provider.rs`; DOCX
  inference in `aivyx-voice/src/session.rs`.
  Small surface, low coupling, easy review.

- **Zero new workspace deps.** Both are string
  matching against existing values.

## Tasks

1. **Open doc + ROADMAP + README.** This doc +
   the roadmap section + the README row.
   Backfill Phase 163 hash to `d7b7c24`.

2. **Anthropic document model-version guard.**
   When a user message contains
   `ContentBlock::DocumentBase64` blocks AND
   the operator's configured model isn't in
   the document-supporting set, emit a clear
   pre-flight `LlmError::InvalidInput` (or
   nearest equivalent) instead of letting the
   request go to the API and surfacing a 400.
   Detection: model string prefix match
   against the known document-capable set
   (claude-3-5-, claude-3-7-,
   claude-opus-4-, claude-sonnet-4-,
   claude-haiku-4-, and any newer hyphenated
   prefix). Older models (claude-3-haiku-,
   claude-3-opus-, claude-3-sonnet- without
   the -5 suffix) fail closed.

3. **DOCX media type inference.**
   `aivyx-voice/src/session.rs` —
   `infer_image_media_type` adds `.docx` →
   `application/vnd.openxmlformats-officedocument.wordprocessingml.document`;
   `media_type_from_content_type` adds the
   same MIME string. `is_document_media_type`
   classifier extends to match the DOCX MIME
   in addition to `application/pdf`. The
   `content_part_for_attachment` helper then
   routes DOCX through
   `ContentPart::Document`.

4. **INSTALL + exit + Frozen.** INSTALL.md
   voice section notes:
   - Anthropic PDFs work on Claude 3.5+ only
     (pre-flight error otherwise — operators
     see "model X doesn't support document
     blocks" instead of a 400).
   - DOCX inference lands; routes through
     Document; provider rejection if not
     supported.
   Exit doc with prediction-vs-reality.
   README + ROADMAP Frozen flip.

## Streak predictions

- **DESIGN.md** — **Will hold.** Phase 164 is
  pure substrate; no contract amendment.
  Streak: 0 (post-A13 RESET) → **1**.
- **PRODUCT.md** — **Will hold.** Streak:
  54 → **55**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 164 work in `aivyx-voice` +
  `aivyx-llm`. Streak: 0 (post-Phase-163
  RESET) → **1**.

## Exit criteria

- [ ] `docs/PHASE_164.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] Anthropic provider refuses
  `DocumentBase64` blocks pre-flight on
  non-document-capable models — Task 2.
- [ ] `.docx` extension + DOCX MIME inferred
  and routed as `Document` — Task 3.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+6` to `+12`.

## Honest scope risks at sign-off

- **Model-version prefix matching is
  hand-maintained.** A new Claude variant
  released after Phase 164 with a new prefix
  shape may fail-closed for documents until
  the substrate adds the prefix. Operators
  see a clear error and the fix is a
  one-line additive change. Documented.

- **DOCX support depends entirely on the
  provider.** Anthropic's document blocks
  currently accept PDF only; DOCX submitted
  to Anthropic surfaces as a 400. Phase 164
  ships the inference + routing so that
  WHEN provider support widens, no further
  voice-side work is needed.

- **No DOC / RTF / ODT / pptx / xlsx
  support.** Phase 164 ships just DOCX as
  the canonical Office text document. Adding
  the others is a one-line additive change
  per type — left for Phase 165+ if surfaces.

- **Pre-flight guard doesn't catch
  per-document size / page-count caps.** If
  Anthropic adds a 100-page limit on
  document blocks, the substrate doesn't
  pre-check. Same posture as Phase 161's
  "no read-stalled-bytes timeout" — bounded
  by the provider's response.

- **The dropped "Phase 163 dead-code
  cleanup" sub-task.** Honestly noted:
  `Message::text_with_image` is still used
  by Telegram + channel; only voice's
  callsite dropped. Sub-task removed from
  scope rather than smuggled in.

- **Fifty-third consecutive deferral of the
  Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 164

After Phase 164, Phase 163's named carry-overs
clear (modulo the dropped cleanup). Phase
165+ candidates:

1. **drive.recent_activity actor / target
   filters.** (Phase 159 carry-over; passed
   on five times running.)
2. **drive.recent_activity consolidation knob.**
   (Phase 159 carry-over.)
3. **drive.recent_activity + parent_folder_id
   composition.** (Phase 159 carry-over.)
4. **Provider-side PDF page-count cap.**
   (Phase 164 carry-over.)
5. **DOC / RTF / ODT / pptx / xlsx
   inference.** (Phase 164 carry-over.)
6. **Streaming document blocks** for large
   PDFs.
7. **Per-URL header overrides** for
   `/image <url>`. (Phase 162 carry-over.)
8. **Read-stalled-bytes timeout (slow-trickle
   defense).** (Phase 161 carry-over.)
9. **Retry on transient image-fetch timeout.**
   (Phase 161 carry-over.)
10. **Mid-recording or mid-reply /image
    command.**
11. **Clipboard-based image source.**
12. **Voice abort UX knob.**
13. **Silero ONNX VAD.**
14. **Streaming ASR.**
15. **Wake-word activation.**
16. **macOS streaming variant.**
17. **Lock-free AudioIn detector.**
18. **calendarList cache TTL knob.**
19. **drive walk min_concurrent companion.**
20. **access_role deprecation.**
21. **Budget category migration tool.**
22. **Budget currency / rust_decimal.**
23. **Multi-category trend breakdown.**
24. **Trend smoothing / moving average.**
25. **Bulk budget operations.**
26. **Proactive reminder dispatch.**
27. **Relative-time localization.**
28. **whisper-cpp-plus rehabilitation.**
29. **`build_agent_stack` substrate-tier
    promotion.**
30. **Secret-store integration** for
    url_headers.
31. **Channel Activation Milestone** —
    still held intentionally; 53rd
    consecutive deferral at Phase 164
    open.

## Prediction vs reality

_Populated at Phase 164 exit. Predictions at
sign-off: DESIGN.md HOLD → 1; PRODUCT.md HOLD
→ 55; lib.rs HOLD → 1; zero new deps; test
count delta `+6` to `+12`; zero clippy
warnings._
