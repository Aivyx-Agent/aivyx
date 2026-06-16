# Phase 154 — Multimodal: Image Attachment via Voice

**Pivot to a new feature surface after 3
consecutive debt-close phases (151-153).** Phase
154 wires Aivyx's voice channel to the existing
`MessageContent::Mixed` shape so the operator
can attach an image to their voice turn and the
agent — running on a vision-capable LLM — can
describe it via TTS.

The infrastructure is largely already there:
- `Message::text_with_image(session, text,
  media_type, data)` constructor exists in
  `aivyx-core` since Phase 45.
- Three LLM providers (Anthropic, Ollama,
  mistral.rs) already handle
  `ContentBlock::ImageBase64`.
- The voice loop already constructs Messages
  per turn.

What's missing is the operator-facing UX: a way
for the operator to say "include this image with
my next message." Phase 154 adds a `/image
<path>` text command — typed at the start-of-
iteration prompt instead of pressing Enter to
record — that loads the image, infers its media
type from the file extension, and queues it on
the channel. The next recording iteration's
turn includes the queued image.

## Why this, why now

- **Variety after 3 debt-close phases.**
  Phases 151 (calendar bundle), 152 (voice
  carry-overs), 153 (drive recent_*) all
  closed long-deferred debts. Phase 154
  delivers a new operator-facing capability.

- **The substrate is already there.**
  `Message::text_with_image` + three LLM
  providers handle image content already. Phase
  154 wires the voice channel to it; no new
  LLM provider work.

- **Daily-use value.** "Describe this
  screenshot" / "what's in this photo" / "what
  does this error message say" — the agent
  becomes meaningfully more useful for visual
  context. Real personal-assistant utility.

- **Operator-validation tier for the LLM round-
  trip.** The wiring is structurally correct +
  unit-testable; whether a real Qwen-VL or
  similar describes an image accurately is
  operator-side work. Same posture as Phase
  135 / 136 voice substrate validation.

- **Zero new workspace deps.**

## Tasks

1. **Open doc + ROADMAP + README** — this doc +
   the roadmap section + the README row.
   Backfill Phase 153 hash to `4410b17`.

2. **`VoiceChannel::pending_image` substrate.**
   Add field:
   ```rust
   pending_image: Mutex<Option<(String, Vec<u8>)>>
   ```
   Public methods:
   ```rust
   pub fn set_pending_image(&self, media_type: String, data: Vec<u8>);
   pub fn take_pending_image(&self) -> Option<(String, Vec<u8>)>;
   ```
   The streaming PTT loop populates via /image
   command. `run_one_voice_turn_streaming`
   consumes via take_pending_image when
   constructing the Message. Tests cover
   set+take round-trip, take-when-empty,
   set-then-set-replaces.

3. **`/image <path>` command + Message
   plumbing.** Two pieces:
   - In the streaming PTT loop's start-of-
     iteration prompt: when the operator's
     line starts with `/image `, treat the
     rest as a file path. Load via
     `std::fs::read`, infer media type from
     extension (.png → image/png, .jpg/.jpeg
     → image/jpeg, .gif → image/gif, .webp →
     image/webp; unknown extension → error).
     Call `channel.set_pending_image(...)`.
     Print confirmation. Loop back to the
     prompt.
   - In `run_one_voice_turn_streaming`: replace
     the `Message::text(...)` construction
     with: if `channel.take_pending_image()`
     yields Some, use
     `Message::text_with_image(...)`; else
     `Message::text(...)`.
   Substrate tests for the command-parse helper
   (path extraction, media-type inference).

4. **INSTALL + exit + Frozen.** INSTALL.md voice
   section gains a Phase 154 paragraph
   documenting the /image command, supported
   media types, operator-side LLM prereqs
   (Qwen-VL via mistral.rs / Ollama, or any
   vision-capable Anthropic/OpenAI model).
   Phase 154 exit doc with prediction-vs-
   reality. README + ROADMAP flip Phase 154 to
   Frozen.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment; uses existing
  `MessageContent::Mixed`. Streak: 44 → **45**.
- **PRODUCT.md** — **Will hold.** Multimodal
  output reinforces the personal-assistant
  framing. Streak: 44 → **45**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 154 work in `aivyx-voice`. Core
  untouched. Streak: 19 → **20**.

## Exit criteria

- [ ] `docs/PHASE_154.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `VoiceChannel::set/take_pending_image`
  public + tested — Task 2.
- [ ] `/image <path>` command parsing +
  media-type inference + Message::text_with_image
  plumbing in run_one_voice_turn_streaming —
  Task 3.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+5` to `+10`
  (pending_image substrate ~3 + command-
  parse helper ~3-5 + media-type inference
  ~2-3).

## Honest scope risks at sign-off

- **Operator-validation tier for the end-to-end
  vision LLM round-trip.** The structural
  wiring (image bytes → Message → planner →
  provider) is correct + unit-testable, but
  whether a real Qwen-VL or Claude-Vision
  describes a given image accurately is
  operator-side validation. Documented in
  INSTALL.

- **Only 4 image media types supported in
  Phase 154 MVP.** PNG / JPEG / GIF / WebP
  cover the typical operator screenshot +
  photo cases. PDF / SVG / TIFF / etc. error
  out with a clear "unsupported media type"
  message. Phase 155+ candidate if surfaces.

- **No size cap.** Phase 154 doesn't reject
  multi-megabyte images at attach time. The
  LLM provider may surface its own size
  error. Phase 155+ candidate for client-
  side size cap.

- **`/image` command works at start-of-
  iteration only.** Operator can't attach
  mid-recording or mid-reply. Phase 155+
  candidate for richer command surface.

- **Image stays queued until consumed.** If
  the operator types `/image foo.png`, then
  `/image bar.png`, the second replaces the
  first (per the set_pending_image documented
  behavior). If they type `/image foo.png`
  and then `quit` before recording, the
  pending image is discarded — clean.

- **No multi-image support.** One queued
  image per turn. Operators who want "describe
  these three screenshots" pass them in three
  separate turns. Phase 155+ could shift
  pending_image to `Vec<(String, Vec<u8>)>`.

- **Forty-third consecutive deferral of the
  Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 154

After Phase 154, the agent can describe
images via voice. Phase 155+ candidates:

1. **PDF / SVG / TIFF media type support.**
2. **Client-side image size cap.**
3. **Mid-recording or mid-reply /image
   command.**
4. **Multi-image queue.**
5. **URL-based image source** (operator
   provides a URL; loop fetches).
6. **Clipboard-based image source** on
   platforms where it makes sense.
7. **Parallel walk_folder_tree** (Phase 153
   carry-over).
8. **Recursive walk within drive_id scope.**
9. **Operator-tunable recursive caps.**
10. **Voice abort UX knob.**
11. **Silero ONNX VAD.**
12. **Streaming ASR.**
13. **Wake-word activation.**
14. **macOS streaming variant.**
15. **Lock-free AudioIn detector.**
16. **Calendar fuzzy dedup.**
17. **Calendar max_concurrent knob.**
18. **Calendar writable_only filter.**
19. **access_role deprecation.**
20. **Budget category migration tool.**
21. **Budget currency / rust_decimal.**
22. **Multi-category trend breakdown.**
23. **Trend smoothing / moving average.**
24. **Bulk budget operations.**
25. **Drive Activity API.**
26. **Proactive reminder dispatch.**
27. **Relative-time localization.**
28. **whisper-cpp-plus rehabilitation.**
29. **`build_agent_stack` substrate-tier
    promotion.**
30. **Channel Activation Milestone** —
    still held intentionally; 43rd
    consecutive deferral at Phase 154 open.

## Prediction vs reality

**Three-of-three streak HOLDs as predicted.**

- **DESIGN.md** — HELD as predicted (`62dabbdd…`
  unchanged). No contract amendment. Streak:
  44 → **45**.
- **PRODUCT.md** — HELD as predicted (`467ba59a…`
  unchanged). Streak: 44 → **45**.
- **`aivyx-core/src/lib.rs`** — HELD as predicted
  (`4f9b8c81…` unchanged). All Phase 154 work in
  `aivyx-voice`. Continuing post-Phase-135
  reset: 19 → **20**.

**Test count delta: +10 — top of predicted `+5`
to `+10` range.** Workspace lib tests 3239 →
3249. Per-module:
- `channel`: +3 (take-when-empty, set+take
  round-trip, set replaces prior).
- `session`: +7 (infer_image_media_type for
  png/jpeg-variants/gif+webp, unsupported-ext
  rejected, no-ext rejected, load_image empty
  path rejected, load_image missing file
  rejected).

**Zero new workspace dependencies** as
predicted.

**Zero clippy warnings** with default features.

### What landed cleanly + what bent

**Cleanly:**
- `VoiceChannel::set_pending_image` +
  `take_pending_image` substrate methods +
  `pending_image: Mutex<Option<(String,
  Vec<u8>)>>` field.
- `set_pending_image` replaces any prior queued
  image (operator double-set behavior
  documented + regression-tested).
- `take_pending_image` clears the slot
  atomically so subsequent turns don't re-send.
- `infer_image_media_type` pure substrate with
  case-insensitive extension matching for 4
  formats (png, jpg/jpeg, gif, webp).
- `load_image_for_attach` composes
  empty-path-check + extension-inference +
  file-read + non-empty check.
- Voice loop's start-of-iteration prompt
  recognizes `/image <path>` and dispatches to
  the helpers. Banner prompt text updated to
  mention the command.
- `run_one_voice_turn_streaming` consumes
  pending_image via match block; falls back
  to text-only Message on None — Phase 138-
  153 behaviour preserved when no image
  queued.
- INSTALL.md voice section gains a Phase 154
  paragraph above the Phase 152 block
  documenting the command, supported formats,
  operator-side vision-LLM prereq + tested
  model list.
- 3249 workspace lib tests pass; clippy clean.

**Bent honestly:**

1. **Operator-validation tier for end-to-end
   LLM round-trip.** The plumbing
   (image → set_pending_image → take →
   Message::text_with_image → agent.turn →
   provider) is structurally correct and
   substrate-tested. Whether a real Qwen-VL
   accurately describes the image is operator-
   side validation. Documented in INSTALL.

2. **4 image media types only.** PNG, JPEG,
   GIF, WebP. Phase 155+ candidate for PDF /
   SVG / TIFF / etc. if surfaces.

3. **No client-side size cap.** A 50MB PNG
   gets accepted at attach time; the LLM
   provider may surface its own size error.
   Phase 155+ candidate for client-side
   guardrail.

4. **`/image` only at start-of-iteration.**
   Operator can't attach mid-recording or
   mid-reply. Phase 155+ command-surface
   widening candidate.

5. **One image per turn.** Multi-image queue
   (Vec<(String, Vec<u8>)>) is Phase 155+ if
   operators want it.

6. **No URL or clipboard source.** Path-only.
   Phase 155+ if surfaces.

7. **No new tests for the end-to-end Message
   construction.** Operator-validation tier;
   the substrate helpers are exhaustively
   covered (7 unit tests on the inference +
   load helpers + 3 on the channel
   pending_image methods).

### Direction after Phase 154

After Phase 154, the agent can describe
images via voice. Phase 155+ candidates:

1. **PDF / SVG / TIFF media type support.**
2. **Client-side image size cap.**
3. **Mid-recording or mid-reply /image
   command.**
4. **Multi-image queue.**
5. **URL-based image source.**
6. **Clipboard-based image source.**
7. **Parallel walk_folder_tree.**
8. **Recursive walk within drive_id scope.**
9. **Operator-tunable recursive caps.**
10. **Voice abort UX knob.**
11. **Silero ONNX VAD.**
12. **Streaming ASR.**
13. **Wake-word activation.**
14. **macOS streaming variant.**
15. **Lock-free AudioIn detector.**
16. **Calendar fuzzy dedup.**
17. **Calendar max_concurrent knob.**
18. **Calendar writable_only filter.**
19. **access_role deprecation.**
20. **Budget category migration tool.**
21. **Budget currency / rust_decimal.**
22. **Multi-category trend breakdown.**
23. **Trend smoothing / moving average.**
24. **Bulk budget operations.**
25. **Drive Activity API.**
26. **Proactive reminder dispatch.**
27. **Relative-time localization.**
28. **whisper-cpp-plus rehabilitation.**
29. **`build_agent_stack` substrate-tier
    promotion.**
30. **Channel Activation Milestone** —
    still held intentionally; 43rd
    consecutive deferral at Phase 154
    exit.
