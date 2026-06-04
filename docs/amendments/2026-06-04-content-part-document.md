# Amendment A13 — `ContentPart::Document` for PDF Routing

**Date:** 2026-06-04
**Phase:** 163
**Supersedes:** Extends D1's `MessageContent` /
`ContentPart` enums (the multimodal contract
Phase 45 amended in via the implicit Phase
0 → 1 transition; not previously called out
in an amendment because the original Text /
Image / Mixed shape was treated as the
multimodal MVP).
**Implementing phase:** 163

---

## What changed

The user-message content enums in `aivyx-core`
gain a `Document` variant:

```rust
pub enum MessageContent {
    Text(String),
    Image { media_type: String, data: Vec<u8> },
    Document { media_type: String, data: Vec<u8> },  // NEW
    Mixed(Vec<ContentPart>),
}

pub enum ContentPart {
    Text(String),
    Image { media_type: String, data: Vec<u8> },
    Document { media_type: String, data: Vec<u8> }, // NEW
}
```

A parallel variant lands in `aivyx-llm`'s
provider-facing content block enum:

```rust
pub enum ContentBlock {
    Text { text: String },
    ImageBase64 { media_type: String, data: String },
    DocumentBase64 { media_type: String, data: String }, // NEW
}
```

The `LlmPlanner` maps the new core variant to
the new llm variant; the four provider
integrations consume the new variant per their
own capability:

- **Anthropic** emits the document content
  block shape
  (`{"type": "document", "source": {"type":
  "base64", "media_type": ..., "data": ...}}`).
  Anthropic supports `application/pdf` only
  (Claude 3.5+).
- **OpenAI / Ollama / mistral_rs** log a
  one-line warning and skip the block. None of
  these have a native document content block
  shape in their chat-completion APIs (OpenAI
  has a separate Files API; Ollama and
  mistral_rs are text/image only).

---

## Design decisions

### Why a new variant rather than extend `Image`

`MessageContent::Image::media_type` is a free-
form `String`. Phase 162 demonstrated that
operators can stuff `"application/pdf"` into
that field through `aivyx-voice`; the value
threads through to `ContentBlock::ImageBase64`
and lands at the provider as an image block.
Anthropic returns 400 — PDFs are not image
blocks.

Phase 163 routes the type signal at the variant
level so:
1. The compiler enforces explicit handling at
   every match site (no silent fall-through).
2. Providers that don't support documents
   skip-and-warn rather than try to encode a
   PDF as an image and confuse the API.
3. Future document types (DOCX via document
   blocks once Anthropic adds support, etc.)
   route cleanly through the same plumbing.

### Why no `Message::document_with_text` constructor

Phase 163 ships only `Message::document(...)`
(parallel to `Message::image(...)`). Operators
who want text + document together construct
`MessageContent::Mixed` explicitly. The
constructor surface stays minimal.

### Provider-side graceful degradation

Three of the four providers (OpenAI, Ollama,
mistral_rs) drop document blocks with a
warning log. This matches the substrate's
posture for unsupported scopes: surface the
limitation without panicking, let the operator
notice from logs that their PDF didn't reach
the model. An alternative would be to error
the whole turn; that's strictly noisier and the
recovery (drop the doc, retry text-only) is
not always what the operator wants.

### `aivyx-voice` routing

Phase 162 maps `.pdf` / `application/pdf` to a
media_type string. Phase 163 makes voice route
those to `ContentPart::Document` rather than
`ContentPart::Image`. The other Phase 162
additions (`.svg`, `.tif`) still route as
`Image` because they ARE images
(visual content), even if specific providers
reject the media type.

---

## What this does not change

- Existing `Image` / text behavior is byte-
  identical.
- The four core scopes and capability surface
  are untouched.
- The substrate-only principle (P10) holds —
  no new tool is introduced.
- The `aivyx-core` crate gains a variant but
  no breaking removal; downstream consumers
  who don't match Document get a compile error
  pointing at the gap.

---

## Tests

- Unit tests pin the new variants and
  constructors in `aivyx-core`.
- Provider conversion tests verify the
  Anthropic document-block shape and the
  three skip-and-warn paths.
- An end-to-end test from `aivyx-voice`'s
  `load_image_for_attach` through to the
  Anthropic content-block shape is operator-
  validation tier (would need a live model
  round-trip).

---

## Implementing phase

Phase 163. See [`docs/PHASE_163.md`](../PHASE_163.md).
