# Phase 136 — Voice Audio I/O Loop: Close-Out

**Phase 135 close-out.** Phase 135 shipped the voice
substrate — ASR + TTS adapters, channel impl, session
seam — but deferred the cpal + rodio audio I/O loop
body as operator-validation work. Operators running
`aivyx --channel voice` got an actionable error
pointing at `aivyx_voice::run_one_voice_turn`. Phase
136 fills in the loop so voice **actually works
end-to-end** the moment the binary is built with the
right features.

## Why this, why now

- **Phase 135 is half-finished without it.** From an
  operator's perspective, the voice channel is a
  feature flag that produces an error message. Closing
  the loop converts that into a real working
  push-to-talk experience.

- **Smallest scope possible.** The substrate seam
  (`run_one_voice_turn`) is fully wired and
  unit-tested with stub engines. Phase 136 implements
  exactly two pieces: cpal mic capture and rodio
  speaker playback, plus the surrounding stdin /
  Enter-driven prompt loop. The hard architectural
  decisions (Q-block from Phase 135) are already
  locked.

- **Same honest hardware-validation posture as
  Phases 134 + 135.** I can't validate end-to-end
  audio I/O on this development machine (no mic, no
  speakers in the build environment). The Phase 136
  deliverable is structurally-correct bridge code
  that compiles cleanly + unit-tested helpers where
  feasible. Operators validate the real-audio path
  on their own machines.

## Tasks

1. **Open doc + ROADMAP + README** — this doc + the
   roadmap section + the README row. Backfill
   Phase 135 hash to `7fdadbd`.

2. **`audio_in.rs` — cpal mic capture.** Build an
   input stream from the configured device or the
   system default. Stream callback pushes f32
   samples into a `Arc<Mutex<Vec<f32>>>`. Expose
   `start_capture` / `stop_capture` / `take_samples`.
   `stop_capture` resamples to 16 kHz mono via the
   existing helpers in `asr/whisper_rs` so the ASR
   engine sees its canonical input format
   regardless of what the operator's hardware
   provided.

3. **`audio_out.rs` — rodio speaker playback.**
   Hold a rodio `Sink` over the configured output
   device or the default. `play_audio(TtsAudio)`
   appends a `SamplesBuffer<f32>` at the chunk's
   declared sample rate; queue plays sequentially.
   `sleep_until_empty` blocks until the queue
   drains. Cancellable via the channel's
   `CancellationToken` so Ctrl-C aborts mid-utterance.

4. **Push-to-talk loop body.** Replace
   `VoiceSessionError::LoopNotYetImplemented` in
   `session.rs::run_push_to_talk_loop` with the real
   loop:
   1. Prompt operator: "Press Enter to start
      recording, Enter again to stop. Type 'quit' to
      exit."
   2. Read stdin line. `"quit"` → break loop.
      Empty line → start capture.
   3. Open `AudioIn`, call `start_capture`.
   4. Wait for next stdin line (Enter or Ctrl-C).
   5. `stop_capture` → samples.
   6. Call `run_one_voice_turn(agent, channel, asr,
      tts, &samples)`. On `Ok(None)` (empty
      transcription) → re-prompt without spending a
      turn. On `Err(_)` → surface to operator;
      continue.
   7. For each audio chunk in the result, append to
      the `AudioOut` sink; `sleep_until_empty`.
   8. Loop.

5. **Wire binary** — replace `ChannelKind::Voice`'s
   "not implemented" error in
   `aivyx-channel/src/bin/aivyx.rs` with the actual
   call to `run_push_to_talk_loop`. Construct
   `ConcreteAgent`, `VoiceChannel`, `WhisperRsEngine`,
   `PiperEngine` from the `[voice]` config section.

6. **INSTALL update + exit doc** — flip the "Phase
   135 reality" notice in INSTALL.md to "Phase 136
   ships the real loop." Exit doc with
   prediction-vs-reality.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment. Streak: 26 → **27**.
- **PRODUCT.md** — **Will hold.** Streak:
  26 → **27**.
- **`aivyx-core/src/lib.rs`** — **Will hold.** No
  core changes; all work lives in `aivyx-voice` +
  the binary's dispatch arm. Streak (post-Phase-135
  reset): 1 → **2**.

## Exit criteria

- [ ] `docs/PHASE_136.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `audio_in.rs` + `audio_out.rs` — Tasks 2-3.
- [ ] `run_push_to_talk_loop` returns `Ok(())` on
  clean exit instead of `LoopNotYetImplemented` —
  Task 4.
- [ ] Binary dispatch arm calls the loop and propagates
  its result — Task 5.
- [ ] DESIGN.md / PRODUCT.md / lib.rs all HOLD as
  predicted.
- [ ] Zero new workspace dependencies (cpal + rodio
  already in `aivyx-voice` from Phase 135).
- [ ] Zero clippy warnings with `--features
  aivyx-channel/channel-voice-full`.
- [ ] Test count delta: `+5` to `+15`. Helpers for
  buffer manipulation + format conversion get unit
  tests; the stream callbacks + speaker playback
  need operator validation.

## Honest scope risks at sign-off

- **Real-audio integration validation is operator
  work.** Same posture as Phases 134 + 135. The
  bridge compiles cleanly and the substrate-tier
  seam (`run_one_voice_turn`) is end-to-end unit-
  tested. The real-mic + real-speaker path needs an
  operator on a real machine. Documented in
  INSTALL.md.

- **cpal input-stream lifetime.** cpal streams are
  paused/resumed via an opaque handle that lives in
  the `AudioIn` struct. If the operator's OS audio
  layer surprises us (PulseAudio vs PipeWire vs
  ALSA differences on Linux, exclusive-mode vs
  shared-mode on Windows, etc.), the operator
  hits it first. Phase 137+ candidate: surface
  configurable timeouts + retry policy.

- **Sample format conversion.** cpal can deliver
  i16 / u16 / f32 / others depending on the
  device's native format. Phase 136 handles f32
  natively + converts i16/u16 inline; exotic
  formats (24-bit packed, f64) error out with an
  actionable message. Phase 137+ widens format
  support if operator demand surfaces.

- **No timeout on push-to-talk capture.** If the
  operator hits Enter to start recording then
  walks away, the mic captures forever (well —
  until the buffer fills the OS audio limits).
  Phase 137+ could add a configurable max
  capture duration with a polite "you've been
  recording for 60s; press Enter to dispatch the
  turn or wait for it to time out" prompt.

- **Twenty-fifth consecutive deferral of the
  Channel Activation Milestone.** Per operator
  framing — intentional hold. Tracking continues.

## Direction after Phase 136

Voice now works. Phase 137+ candidates:

1. **Streaming TTS during LLM generation.** Pipeline
   text chunks from the planner into the TTS engine
   on sentence boundaries — operator hears the
   first sentence while the LLM is still generating.
2. **Wake-word activation** ("Hey Aivyx") via
   Porcupine or Silero-wakeword.
3. **Voice activity detection** for trim-on-silence
   push-to-talk (no more "press Enter twice").
4. **Multimodal output** — agent speaks
   descriptions of images.
5. **whisper-cpp-plus rehabilitation** — close out
   the Phase 135 Q2c deferral once upstream is
   unstuck.
6. **Channel Activation Milestone** — still held
   intentionally.

## Prediction vs reality

_Populated at Phase 136 exit. Predictions at sign-
off: DESIGN.md HOLD → 27; PRODUCT.md HOLD → 27;
lib.rs HOLD → 2; zero new deps; test count delta
`+5` to `+15`; zero clippy warnings._
