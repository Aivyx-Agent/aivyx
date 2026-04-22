# Phase 45 — Rich Input (Multimodal Messages)

Phase journals are working documents. They churn freely
during the phase and freeze at exit under a final Exit criteria
block. For the locked technical contract see
[`../DESIGN.md`](../DESIGN.md); for the locked product contract
see [`../PRODUCT.md`](../PRODUCT.md).

## Goal

Thread multimodal content (images) through the full message
pipeline: `MessageContent` -> `LlmMessage` -> provider
serialization, with channel-level extraction for CLI
(`/image <path>`) and Telegram (photos via Bot API). Transforms
Aivyx from a text-only agent into one that can see and reason
about visual content.

## Why now

1. **General-purpose agent.** Text-only input locks Aivyx into
   "chatbot" territory. Users can't send screenshots, photos,
   or documents for analysis.

2. **Provider readiness.** Anthropic and OpenAI APIs already
   support multimodal content blocks. The Anthropic provider
   already wraps user text in content block arrays.

3. **Phase 44 onboarding.** With `aivyx init` shipped, new
   users can get started easily. The next barrier is "I can
   only type text" — especially on Telegram where photo
   sharing is natural.

## Entry baseline

- Tests: 876
- Clippy warnings: 0
- Deferral backlog: 0
- DESIGN.md streak: 3 phases (untouched since Phase 41)
- PRODUCT.md streak: 8 phases (untouched since Phase 38)
- lib.rs streak: 1 phase (untouched in Phase 44)

## Streak predictions

| Streak target | Predicted | Notes |
|---|---|---|
| DESIGN.md | touched | MessageContent is a D3 contract type |
| PRODUCT.md | untouched (9) | Multimodal input is not a product commitment |
| lib.rs | touched | MessageContent enum lives in lib.rs |

## Tasks

### Task 1 — Phase open + ContentBlock type in aivyx-llm

Scaffold this file. Add `ContentBlock` enum. Change
`LlmMessage::User` from `String` to `Vec<ContentBlock>`.
Update all construction sites, `estimate_tokens`,
`summarise_pruned`.

### Task 2 — Provider serialization (Anthropic + OpenAI + Ollama)

Update `anthropic_message()` and `openai_message()` to
serialize `ContentBlock` image types. Ollama vision format.

### Task 3 — MessageContent multimodal variants in aivyx-core

Extend `MessageContent` with `Image` and `Mixed` variants.
Update `begin_turn()` conversion. Add `base64` dependency.

### Task 4 — CLI `/image` command

Add `/image <path>` handling to the REPL loop, media type
detection, image + caption support.

### Task 5 — Telegram photo extraction

Extract photos from Telegram updates via `get_file()` + HTTP
download. Update `IncomingMessage` and session layer.

### Task 6 — Daemon IPC multimodal + backwards compat

Extend `FrontendMessage::SubmitInput` with `attachments` field.
`IpcAttachment` struct. Daemon server decoding.

### Task 7 — Exit freeze

## Ship records

| Task | Commit | Tests after |
|---|---|---|
| 1+2 | `92210f6` | 879 |
| 3 | `c133018` | 883 |
| 4 | `ab21e9a` | 887 |
| 5 | `f94112a` | 890 |
| 6 | `0bea643` | 893 |
| 7 (exit) | `TBD` | 893 |

## Exit criteria

- [x] `ContentBlock` enum with `Text` and `ImageBase64` variants.
- [x] `LlmMessage::User` carries `Vec<ContentBlock>`.
- [x] Anthropic provider serializes image content blocks.
- [x] OpenAI provider serializes image content arrays.
- [x] `MessageContent::Image` and `MessageContent::Mixed` variants.
- [x] CLI `/image <path>` command sends images to the agent.
- [x] Telegram photo extraction via Bot API.
- [x] Daemon IPC carries attachments with backwards compat.
- [x] All tests pass with net-positive delta.
- [x] Zero clippy warnings.
- [x] PRODUCT.md untouched (streak -> 9).

## Exit stats

- Tests: 876 -> 893 (+17)
- Clippy warnings: 0
- Deferral backlog: 0

### Streak outcomes

| Streak target | Predicted | Actual | New streak |
|---|---|---|---|
| DESIGN.md | touched | untouched | 4 |
| PRODUCT.md | untouched (9) | untouched | 9 |
| lib.rs | touched | touched | 0 |

DESIGN.md prediction was wrong: we extended `MessageContent`
(a D3-adjacent type) but the D3 contract itself was not
modified. The enum extension is an additive change that does
not alter the existing D3 surface.
