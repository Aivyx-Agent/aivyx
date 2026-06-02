# Phase 135 — Voice I/O: Talk to the Agent, Agent Talks Back

**First multimodal-interaction phase.** Aivyx has shipped
Text/Image input through cloud LLMs since Phase 45, but
operator-facing input has always been keyboard-only and
agent output text-only. Phase 135 changes that: the
operator can **speak to the agent through their
microphone**, and the agent **speaks back through the
speakers**. End-to-end voice loop, fully local.

## Why this, why now

- **Direct operator request.** "The next Phase for the
  Aivyx Agent is to explore and implement Audio and
  Speech, so the End User can Talk to their Agent and
  the Agent can Talk back instead of just Text and
  Typing on Screen." Direct framing — voice is the
  next UX axis the operator wants Aivyx to cover.

- **Local-privacy posture continues end-to-end.**
  Phase 134 shipped embedded LLM inference with zero
  outbound network calls during generation. Phase 135
  extends that posture to voice: speech-to-text
  (Whisper) and text-to-speech (Piper) both run
  in-process on the operator's machine. No cloud
  speech APIs, no telemetry, no operator audio ever
  leaves the device.

- **The Rust audio ecosystem is mature.**
  `cpal` + `rodio` for cross-platform mic/speaker I/O
  (Linux ALSA/PipeWire, macOS CoreAudio, Windows
  WASAPI); `whisper-rs` for Whisper bindings;
  `piper1-rs` for Piper TTS bindings; `voice_activity_detector`
  for Silero VAD. Every piece exists; Phase 135
  composes them into one channel.

- **Channel-pattern fit.** Aivyx already ships seven
  channel adapters (Local CLI, Telegram, Discord,
  Slack, Web UI, Daemon IPC, Webhook). Voice becomes
  the eighth — same `ChannelContext` trait, same
  `agent.turn(message, &channel)` dispatch, same
  audit chain. Zero churn to the Phase 134 turn loop.

## Q-block sign-off (3 Recommended + 1 non-Recommended)

- **Q1a — New `aivyx-voice` crate** (Recommended).

  Voice becomes its own optional crate following the
  Chapter F + Phase 111 pattern (one crate per
  channel adapter). Feature-gated through the
  `aivyx-channel` binary so the substantial
  dependency footprint (whisper-rs + piper1-rs +
  cpal + rodio + ONNX runtime) is opt-in. Operators
  who don't want voice build without the feature
  and see zero binary-size impact.

- **Q2c — Both `whisper-rs` and `whisper-cpp-plus`**
  (non-Recommended; operator-picked over Q2a's
  whisper-rs only or Q2b's whisper-cpp-plus only).

  Two STT engines as alternatives, same pattern
  Phase 133 used for LLM providers. `whisper-rs`
  is the default — mature, large community, same
  whisper.cpp engine that powers most Whisper
  projects globally. `whisper-cpp-plus` is the
  alternative for operators who want built-in
  Silero VAD + real-time PCM streaming without
  manual wiring. Doubles the test surface; honest
  scope risk noted below.

- **Q3a — Piper via `piper1-rs`** (Recommended).

  Piper is the strongest CPU-friendly TTS for an
  embedded local-agent: real-time on Raspberry Pi
  class hardware; 50+ voices across 30+ languages;
  Apache 2.0 licensed; mature Rust bindings. The
  operator downloads one ~20-50MB voice `.onnx`
  file. Higher-quality alternatives (Kokoro,
  F5-TTS) are heavier and lack mature Rust
  bindings; Phase 136+ work if operator demand
  surfaces.

- **Q4a — Push-to-talk MVP** (Recommended).

  The operator runs `aivyx --channel voice` and
  presses Enter to start recording, Enter again to
  stop (or VAD silence trim). Simplest
  implementation; clearest privacy posture (no
  always-listening). Wake-word activation
  ("Hey Aivyx") and continuous VAD-trimmed
  listening are Phase 136+ candidates after
  operators validate the basic loop.

## Streak predictions

- **DESIGN.md** — **Will hold.** D1 (turn loop)
  doesn't move; D2 (Open-Core) doesn't move; D3
  (Agent trait) doesn't move. Voice plugs into the
  existing `ChannelContext` trait unchanged. Current
  hash: `62dabbdd…`. Prediction: streak **extends
  to 26**.

- **PRODUCT.md** — **Will hold.** Voice is a natural
  extension of "AI personal assistant" framing; no
  product principle moves. Current hash: `467ba59a…`.
  Prediction: streak **extends to 26**.

- **`aivyx-core/src/lib.rs`** — **At risk.** The new
  `ChannelPlatform::Voice` variant lives in core's
  `ChannelPlatform` enum (it's where every existing
  platform variant lives — `Local`, `Telegram`, etc.).
  This is a one-variant addition; the streak almost
  certainly **breaks at 8 and resets to 1**.

  Operator forensics rationale: keeping the streak
  going via a workaround (string-typed platform
  field, or a separate `Voice` variant introduced
  later) would be design-uglier than just adding
  the variant. Phase 135 chooses honesty over
  streak preservation.

## Tasks

1. **Open doc + ROADMAP + README** — this doc + the
   roadmap section + the README row. Backfill
   Phase 134 hash to `f290caa`.

2. **`aivyx-voice` crate skeleton + workspace deps.**
   New crate with optional `whisper-rs`,
   `whisper-cpp-plus`, `piper1-rs` deps behind
   feature flags. Always-on deps: `cpal`, `rodio`,
   `voice_activity_detector`. Workspace member
   registration. Feature flags: `asr-whisper-rs`
   (default), `asr-whisper-cpp-plus`, `tts-piper`
   (default).

3. **ASR module.** `asr/mod.rs` defines `AsrEngine`
   trait; `asr/whisper_rs.rs` implements the
   default; `asr/whisper_cpp_plus.rs` implements
   the alternative (feature-gated). Per-engine
   config: model `.bin` path, language, beam size.

4. **TTS module.** `tts/mod.rs` defines `TtsEngine`
   trait; `tts/piper.rs` implements the default.
   Config: voice `.onnx` path, sample rate, optional
   speaker-id (for multi-speaker voices).

5. **VoiceChannel impl.** `channel.rs` defines
   `VoiceChannel` implementing `ChannelContext`.
   Push-to-talk loop: prompt operator → capture mic
   chunk via `cpal` until Enter or VAD silence →
   run ASR → construct `Message` → `agent.turn(msg,
   &voice_channel).await` → on `stream_event(Text)`
   buffer until sentence boundary → run TTS → play
   via `rodio`. Adds `ChannelPlatform::Voice` to
   `aivyx-core::ChannelPlatform`.

6. **Aivyx binary integration.** `--channel voice`
   flag in `aivyx-channel/src/bin/aivyx.rs`,
   feature-gated. Reads `[voice]` config section
   (asr_model_path, tts_voice_path, optional audio
   device overrides). Constructs and runs the
   voice channel loop. Actionable error message
   when the feature isn't built in.

7. **INSTALL.md voice section** — recommended
   models matrix (whisper base/small/medium + Piper
   voices), audio device setup per OS, honest
   scope caps. Build commands for each
   feature combination.

8. **Exit doc + prediction-vs-reality** — populate
   the prediction-vs-reality section honestly,
   especially the lib.rs streak break. Flip
   Frozen.

## Exit criteria

- [ ] `docs/PHASE_135.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `aivyx-voice` crate with feature-gated ASR +
  TTS implementations — Tasks 2-4.
- [ ] `VoiceChannel` + binary `--channel voice`
  dispatch — Tasks 5-6.
- [ ] INSTALL.md voice section — Task 7.
- [ ] DESIGN.md / PRODUCT.md HOLD as predicted.
- [ ] **lib.rs streak honestly broken** at 8;
  reset to 1.
- [ ] **Three new workspace dependencies behind
  opt-in feature gates:** `cpal`, `rodio`,
  `voice_activity_detector` (always-on inside
  `aivyx-voice`); `whisper-rs`, `piper1-rs`,
  `whisper-cpp-plus` (per-engine optional).
- [ ] Zero clippy warnings (default features and
  with `channel-voice`).
- [ ] Test count delta: `+15` to `+40`. Per-engine
  unit tests; integration tests deferred to
  operator validation.

## Honest scope risks at sign-off

- **Real audio I/O testing is operator work.**
  Phase 135 ships unit-tested conversion logic
  (PCM sample-rate conversion, sentence-boundary
  segmentation, ONNX-output mapping). The "speak
  into the mic and hear the response" validation
  needs an operator on a real machine with a real
  mic + speaker setup. Documented in the exit
  doc honestly.

- **Q2c doubles the ASR test surface.** Two STT
  engines means two adapters to maintain, two sets
  of conversion edge cases, two feature
  combinations to verify clippy-clean. Operators
  who pick one and stick with it see no difference;
  Phase 135's CI matrix grows.

- **ONNX runtime dependency.** `piper1-rs` requires
  ONNX runtime libraries at build time
  (`ONNX_RUNTIME_DIR`, `ONNX_INCLUDE_PATH`).
  Documented in INSTALL.md with per-OS install
  commands. Operators without the prereqs see
  build failures; the slim no-voice build is
  unaffected.

- **Whisper model size.** `whisper-base.bin` is
  ~150MB, `whisper-small.bin` ~500MB, `whisper-medium.bin`
  ~1.5GB. Operator downloads. INSTALL.md
  recommends `whisper-base` as the starting point
  for English; multilingual operators need
  `whisper-small` or larger.

- **No streaming TTS during generation.** Phase 135
  buffers the agent's full response, then
  synthesizes + plays. Operator perceives latency
  on long responses (10s+ for a multi-paragraph
  answer). Streaming TTS pipelined with LLM
  generation is Phase 136+ work.

- **No multimodal output.** Voice is text only;
  the agent can't yet "show me a chart" in voice
  mode. Same scope cap as every other channel
  except Web UI.

- **Twenty-fourth consecutive deferral of the
  Channel Activation Milestone.** Per operator
  framing: intentional hold. Honest tracking
  continues.

## Direction after Phase 135

After Phase 135, the candidates for Phase 136:

1. **Streaming TTS during LLM generation.** Pipeline
   text chunks from the planner into the TTS engine
   on sentence boundaries so the operator hears the
   first sentence while the LLM is still generating
   the rest. Significant latency win.
2. **Wake-word activation.** Porcupine or
   Silero-wakeword for "Hey Aivyx" style
   always-on listening. Bigger scope (new dep,
   tuning, false-positive guards).
3. **Voice channel for daemon mode.** Phase 135
   ships the standalone-process voice channel;
   wiring the daemon-side variant so the agent
   can run voice-attached as a background service
   is the next step.
4. **Multimodal output: spoken descriptions of
   images.** Once the agent can "see" via
   `ContentBlock::ImageBase64`, the voice channel
   can describe what it sees. Cross-modality
   feature.

## Prediction vs reality

_Populated at Phase 135 exit. Predictions at
sign-off: DESIGN.md HOLD → 26; PRODUCT.md HOLD →
26; **`aivyx-core/src/lib.rs` streak breaks at 8
and resets to 1** (ChannelPlatform::Voice variant
addition); three new workspace deps behind opt-in
gates; test count delta `+15` to `+40`; zero
clippy warnings._
