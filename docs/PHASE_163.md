# Phase 163 — `ContentPart::Document` for PDF Routing

**First DESIGN.md amendment in 54 phases.**
Phase 162's just-shipped honest-debt: PDFs flow
through `ContentPart::Image` and Anthropic
returns 400 because PDFs aren't image blocks.
Phase 163 adds a `Document` variant across the
content-block stack so PDFs route as document
blocks at the Anthropic API and skip-and-warn
on providers that don't support them.

This is the first phase since the
Phase-99→Phase-109 substrate-tool-count
amendment cluster that requires a DESIGN.md
amendment under `docs/amendments/`. The
multi-crate touch (aivyx-core, aivyx-llm, four
provider modules, aivyx-voice) makes it a larger
phase than the typical close-out bundle.

## Why this, why now

- **Phase 162 just exposed it.** Phase 162
  added `application/pdf` media-type inference;
  Phase 163 makes the inferred type actually
  route correctly. Without 163, the Phase 162
  PDF support is functionally cosmetic.

- **The fix sits at the contract layer.**
  Phase 162's exit doc named "PDF document-
  block plumbing in core" as a Phase 163+
  candidate, calling out the DESIGN.md
  amendment dependency. The user explicitly
  picked that surface (non-Recommended over
  the drive.recent_activity bundle).

- **Locked-contract amendment is precedent-
  setting for the project.** The last
  amendment (A12, Phase 109) added two new
  substrate tools. Phase 163's amendment
  (A13) extends the multimodal-content
  contract — a different shape of change,
  but the same `docs/amendments/` discipline
  applies.

## Tasks

1. **Open doc + Amendment A13 + ROADMAP +
   README.** This doc + the amendment file at
   `docs/amendments/2026-06-04-content-part-
   document.md` + the roadmap section + the
   README row. Backfill Phase 162 hash to
   `82bc141`.

2. **`aivyx-core`: `ContentPart::Document` +
   `MessageContent::Document` variants.**
   Both enums gain a `Document { media_type:
   String, data: Vec<u8> }` variant. Add
   `Message::document(session_id, media_type,
   data)` constructor. Update every match arm
   in `lib.rs` (test code) and `llm_planner.rs`
   to handle the new variant. Tests pin the
   new constructor and variant.

3. **`aivyx-llm`:
   `ContentBlock::DocumentBase64` + provider
   mappings.**
   - `aivyx-llm/src/lib.rs` adds the variant
     + `ContentBlock::document_from_bytes`
     constructor + updates `is_image` (stays
     `false` for documents) + adds
     `is_document` companion.
   - **Anthropic provider** (full document
     block emission): `provider.rs:207`
     match adds a `DocumentBase64` arm
     emitting
     `{"type":"document","source":{"type":"base64","media_type":...,"data":...}}`.
   - **OpenAI / Ollama / mistral_rs** (skip-
     and-warn): each provider's match arm
     adds a `DocumentBase64` arm that logs
     `tracing::warn!("provider X: skipping
     document block …")` and emits nothing.
   - Tests cover the Anthropic shape + the
     skip-and-warn behavior for each of the
     other three.

4. **Wire `aivyx-voice` PDF routing.**
   `aivyx-voice/src/session.rs` —
   when `pending_image`'s media_type is
   `application/pdf`, build
   `ContentPart::Document` instead of
   `ContentPart::Image`. SVG/TIFF keep routing
   as Image (visual content; provider
   rejection surfaces independently).

5. **DESIGN.md update + INSTALL + exit +
   Frozen.** Append the amendment reference to
   DESIGN.md's "Status" section. INSTALL.md
   voice section: PDF caveat from Phase 162
   updated to "works on Anthropic, skip-and-
   warn elsewhere." Exit doc with prediction-
   vs-reality. README + ROADMAP Frozen flip.

## Streak predictions

- **DESIGN.md** — **Will break.** The amendment
  reference lands in the "Status" section.
  Streak: 53 → **broken; reset to 0** at
  Phase 163's commit.
- **PRODUCT.md** — **Will hold.** No commitment
  change. Streak: 53 → **54**.
- **`aivyx-core/src/lib.rs`** — **Will break.**
  New variant + new constructor. Streak: 28 →
  **broken; reset to 0**.

This is the second pair of streaks broken at
the contract-amendment layer (A12 in Phase
109 reset DESIGN.md). It's the expected cost
of a locked-contract amendment — and the
project's audit trail is that the amendment
file goes through the same discipline.

## Exit criteria

- [ ] `docs/PHASE_163.md` + `docs/amendments/
  2026-06-04-content-part-document.md` +
  ROADMAP entry + `docs/README.md` status row
  — Task 1.
- [ ] `ContentPart::Document` and
  `MessageContent::Document` exist; every
  match site in `aivyx-core` handles them —
  Task 2.
- [ ] `ContentBlock::DocumentBase64` exists;
  four providers handle it (one full, three
  skip-and-warn) — Task 3.
- [ ] `aivyx-voice` routes PDFs to
  `ContentPart::Document` — Task 4.
- [ ] DESIGN.md "Status" section references
  amendment A13; PRODUCT.md HOLD; lib.rs
  RESET expected.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+15` to `+30` (larger
  than typical close-out because the multi-
  crate touch needs coverage at each layer).

## Honest scope risks at sign-off

- **Anthropic API may surface model-version
  errors.** Document blocks require Claude
  3.5+. Operators on older models (Claude 3
  Haiku, Opus 3) will see a model-side error
  from the API rather than from aivyx. Phase
  163 doesn't gate by model; operator
  responsibility.

- **Three of four providers drop documents
  silently (warn-only).** Operators using
  OpenAI / Ollama / mistral_rs who type
  `/image foo.pdf` will see the PDF skipped
  with a log warning. Acceptable; documented
  in INSTALL.md.

- **No streaming-document support.** Phase
  163 emits the full base64-encoded blob in
  one content block. Large PDFs (hundreds of
  MB) hit the 10MB cap from Phase 156/161,
  not Phase 163. Out-of-scope.

- **No provider-side content-validation
  pre-check.** If Anthropic adds a PDF size
  cap or page-count cap at the API layer,
  aivyx surfaces that as a provider error
  post-fact. Phase 164+ candidate for an
  inline pre-check.

- **DESIGN.md streak resets.** Documented
  cost of any contract amendment. Same
  posture as A12 / Phase 109.

- **`aivyx-core/src/lib.rs` streak resets.**
  Same.

- **Fifty-second consecutive deferral of the
  Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 163

After Phase 163, PDFs work end-to-end on
Anthropic. Phase 164+ candidates:

1. **drive.recent_activity actor / target
   filters.** (Phase 159 carry-over.)
2. **drive.recent_activity consolidation knob.**
   (Phase 159 carry-over.)
3. **drive.recent_activity + parent_folder_id
   composition.** (Phase 159 carry-over.)
4. **Provider-side PDF content-validation
   pre-check.** (New Phase 163 carry-over.)
5. **Streaming document blocks** for large
   PDFs.
6. **DOCX / TXT document support** once
   provider APIs add the block shape.
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
    still held intentionally; 52nd
    consecutive deferral at Phase 163 open.

## Prediction vs reality

_Populated at Phase 163 exit. Predictions at
sign-off: DESIGN.md RESET (amendment A13);
PRODUCT.md HOLD → 54; `aivyx-core/src/lib.rs`
RESET; zero new deps; test count delta
`+15` to `+30`; zero clippy warnings._
