# Phase 139 — Voice Activity Detection (Energy Threshold)

**Phase 138 follow-on.** Phase 138 collapsed
voice latency-to-first-audio with streaming TTS.
Phase 139 attacks the next UX irritant in the
push-to-talk loop: **the operator has to press
Enter twice** — once to start recording, once
to stop. Phase 139 replaces the second Enter with
**energy-threshold voice activity detection**.
The operator talks, then stops talking, and the
mic auto-stops on detected silence.

## Why this, why now

- **Smallest meaningful UX win still on the
  table.** Once you have streaming TTS, the
  next thing that feels wrong in voice mode is
  the manual stop-recording step. Auto-stop on
  silence is the natural complement to
  auto-play-as-you-think.

- **Pure substrate, zero new deps.** Energy-RMS
  detection is ~30 lines of pure Rust: sliding
  window over recent samples, sum-of-squares,
  threshold check. No ONNX, no model files, no
  ML framework. The ML-based alternative
  (`silero`) is more robust in noisy
  environments but introduces an ONNX runtime
  prereq that — like Phase 135's whisper-cpp-
  plus — could break under upstream churn.

- **PTT use case is naturally quiet.** The
  operator is deliberately speaking into the mic
  to dispatch a turn. Background noise is
  bounded (their own room, their own setup).
  Energy threshold fits the use case; ML VAD
  is over-engineered for the common case.

## Tasks

1. **Open doc + ROADMAP + README** — this doc +
   the roadmap section + the README row.
   Backfill Phase 138 hash to `6bf36d6`.

2. **`SilenceDetector` substrate module.** New
   `aivyx-voice/src/silence_detector.rs`:
   ```rust
   pub struct SilenceDetectorConfig {
       pub sample_rate: u32,
       pub frame_secs: f32,    // window size for RMS (default 0.03 = 30ms)
       pub threshold_rms: f32, // below = silence (default 0.01)
   }

   pub struct SilenceDetector {
       config: SilenceDetectorConfig,
       frame_size: usize,
       pending: Vec<f32>,
       consecutive_silent_samples: usize,
       total_samples: usize,
   }

   impl SilenceDetector {
       pub fn new(config: SilenceDetectorConfig) -> Self;
       pub fn observe(&mut self, samples: &[f32]);
       pub fn silence_dwell(&self) -> Duration;
       pub fn total_recorded(&self) -> Duration;
       pub fn reset(&mut self);
   }
   ```
   Pure substrate; directly unit-testable.
   `observe` accumulates samples into a
   pending-frame buffer, processes one frame at
   a time (RMS over the frame), and tracks
   consecutive silent samples since the last
   frame that exceeded the threshold. Unit tests
   cover: empty observe, sub-frame partial,
   multi-frame burst, silence-speech-silence
   (counter resets on speech), loud-then-quiet
   transition, reset clears state.

3. **AudioIn integrates SilenceDetector.** The
   cpal callbacks (f32 / i16 / u16) observe
   samples after pushing to the capture buffer.
   AudioIn exposes:
   ```rust
   pub fn silence_dwell(&self) -> Duration;
   pub fn total_recorded(&self) -> Duration;
   pub fn reset_detector(&mut self);
   ```
   `start()` calls `reset_detector()` before
   unpause so each turn starts with fresh
   silence state.

4. **PTT loop silence-detection auto-stop.**
   Replace the second `read_stdin_line_trimmed()`
   in `run_push_to_talk_loop_streaming` with a
   `tokio::time::sleep` poll loop. Each tick
   (~100ms): check `audio_in.silence_dwell()`.
   If `silence_dwell >= DWELL_SECS` (1.5s) AND
   `total_recorded >= MIN_SPEECH_SECS` (0.5s),
   break to dispatch. Hard cap at
   `MAX_CAPTURE_SECS` (30s) so a stuck mic
   doesn't record forever. Prompt text updates
   to reflect "speak, then pause to dispatch."

5. **INSTALL + exit + Frozen.** Voice section
   gains a Phase 139 paragraph. Exit doc with
   prediction-vs-reality + honest bends.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment; auto-stop is a UX layer on top of
  the existing PTT substrate. Streak: 29 → **30**.
- **PRODUCT.md** — **Will hold.** Lower-friction
  voice reinforces the existing voice-personal-
  assistant framing. Streak: 29 → **30**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 139 work in `aivyx-voice`. Core
  untouched. Streak: 4 → **5**.

## Exit criteria

- [ ] `docs/PHASE_139.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `SilenceDetector` public in
  `aivyx-voice::silence_detector` with full
  unit-test coverage — Task 2.
- [ ] `AudioIn::silence_dwell` /
  `AudioIn::total_recorded` /
  `AudioIn::reset_detector` public — Task 3.
- [ ] `run_push_to_talk_loop_streaming` replaces
  Enter-to-stop with silence-detection auto-stop —
  Task 4.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+7` to `+10`
  (SilenceDetector ~6 tests; AudioIn-side
  helpers ~1-2).

## Honest scope risks at sign-off

- **Manual abort affordance regresses.** Phase
  138 PTT had two Enter prompts (start + stop);
  the operator could "press Enter again to
  abort" if they changed their mind mid-
  recording. Phase 139 auto-stop loop has no
  manual override during recording. If the
  operator wants to abandon a half-spoken
  message, they have to wait for silence-dwell
  to elapse (~1.5s) and then accept the
  partial-transcription turn. Phase 140+
  candidate: a separate keyboard listener task
  that fires a manual-abort channel.

- **Threshold tuning is hardcoded.** Phase 139
  ships with sensible defaults (frame 30ms,
  RMS threshold 0.01, dwell 1.5s, min-speech
  0.5s, max-capture 30s). Quiet rooms work
  great; noisy environments may need either
  raising the threshold or switching to ML
  VAD. Config wiring (a `[voice.vad]` section
  in `aivyx.toml`) deferred to Phase 140+ if
  operator demand surfaces.

- **Energy threshold less robust than ML.** A
  fan running, an air conditioner, a busy
  cafe — all push background RMS above the
  default threshold. ML VAD (silero) is
  trained to discriminate speech-vs-non-speech
  patterns and handles these cases better.
  Phase 140+ candidate for operators in noisy
  environments.

- **No streaming ASR.** Phase 139 still
  captures the whole utterance before
  dispatching to Whisper. Streaming ASR
  (whisper.cpp's partial-decode mode) is a
  separate effort; Phase 141+ candidate.

- **macOS Send constraint still applies.** The
  Phase 138 consumer-task pattern requires
  `AudioOut` Send. Phase 139 doesn't address
  this; the macOS variant remains a Phase 140+
  candidate.

- **Twenty-eighth consecutive deferral of the
  Channel Activation Milestone.** Per
  operator framing — intentional hold.
  Tracking continues.

## Direction after Phase 139

After Phase 139, voice loops without manual
stop. Phase 140+ candidates:

1. **`[voice.vad]` TOML config** — operator-
   tunable threshold + dwell + min-speech +
   max-capture knobs. Small substrate work;
   surfaces in Q-block if operator hits a
   noisy environment.
2. **Mid-recording manual-abort UX** — separate
   stdin listener task that races against the
   silence detector via mpsc.
3. **Silero ONNX VAD** — drop-in replacement
   for the energy detector when operator is in
   a noisy room.
4. **Mid-synthesis abort UX** — drain the TTS
   mpsc on Ctrl-C / Escape (Phase 138 carry-
   over).
5. **Streaming ASR** — Whisper partial-decode
   mode for true-streaming transcription.
6. **Wake-word activation** ("Hey Aivyx").
7. **Multimodal output** — agent speaks image
   descriptions.
8. **macOS streaming variant** — `LocalSet`-
   based consumer task.
9. **whisper-cpp-plus rehabilitation** —
   Phase 135 Q2c close-out once upstream is
   unstuck.
10. **`build_agent_stack` substrate-tier
    promotion** if more channel adapters ship.
11. **Channel Activation Milestone** — still
    held intentionally; 28th consecutive
    deferral at Phase 139 open.

## Prediction vs reality

**Three-of-three streak HOLDs as predicted.**

- **DESIGN.md** — HELD as predicted (`62dabbdd…`
  unchanged). No contract amendment. Streak:
  29 → **30**.
- **PRODUCT.md** — HELD as predicted (`467ba59a…`
  unchanged). Streak: 29 → **30**.
- **`aivyx-core/src/lib.rs`** — HELD as predicted
  (`4f9b8c81…` unchanged). All Phase 139 work in
  `aivyx-voice`. Continuing post-Phase-135
  reset: 4 → **5**.

**Test count delta: +10 — top of predicted `+7`
to `+10` range.** Workspace lib tests 3040 →
3050. Per-module:
- `silence_detector` substrate: +10 (config
  default sanity, empty observe, sub-frame
  partial, complete silent frame, loud frame
  resets counter, silence-speech-silence,
  sub-threshold noise, above-threshold loud,
  reset clears state, frame_rms math).
- AudioIn: 0 new tests — the existing
  WHISPER_SAMPLE_RATE sanity is the only
  non-hardware test, and AudioIn's new
  silence_dwell/total_recorded/reset_detector
  accessors are exercised end-to-end by the
  PTT loop (operator-validation tier; no
  CI-tier audio device).

**Zero new workspace dependencies** as predicted.
Energy-threshold detection is pure-Rust math:
sum-of-squares per frame, threshold check,
counter arithmetic.

**Zero clippy warnings** with default features
and with `--features aivyx-channel/channel-voice`.

### What landed cleanly + what bent

**Cleanly:**
- `SilenceDetector` + `SilenceDetectorConfig`
  public in `aivyx-voice::silence_detector`.
  Pure substrate; frame-RMS sliding-window
  implementation; resets-fresh on the
  silence-speech-silence sequence (operator
  mid-utterance pause case).
- AudioIn cpal callbacks (f32/i16/u16) observe
  samples through the detector under a
  separate mutex from the capture buffer. The
  i16/u16 paths convert once and share the
  f32-normalized slice between both consumers
  — no duplicate format conversion.
- `AudioIn::silence_dwell` /
  `AudioIn::total_recorded` /
  `AudioIn::reset_detector` accessors.
  `start()` calls reset_detector automatically.
- PTT loop replaces second Enter prompt with a
  `tokio::time::sleep` poll loop:
  - Auto-stop on `dwell >= 1.5s` AND
    `total >= 0.5s`.
  - Hard cap at 30s with a heads-up message.
  - 100ms poll tick.
- AudioIn still never crosses an `.await`
  boundary; the poll loop awaits sleep
  between non-async accessor calls. macOS
  Send constraint posture from Phase 136
  preserved.

**Bent honestly:**

1. **Phase 138's "press Enter to abort" goes
   away.** Replaced by "pause 1.5s to
   dispatch". Operators who want to abandon a
   half-spoken message must wait for the
   dwell window then accept the
   partial-transcription turn. Phase 140+
   candidate: a separate stdin-listener task
   that races the silence detector via a
   manual-abort mpsc.

2. **Threshold hardcoded.** Quiet rooms work
   well with the default 0.01 RMS threshold;
   noisy ones may need either raising it (more
   tolerant of background) or switching to ML
   VAD. A `[voice.vad]` TOML knob is
   straightforward to add once operator
   pressure surfaces.

3. **Cpal callback locks two mutexes per
   chunk.** Audio threads prefer lock-free
   state; at ~10ms chunk intervals and ~100ms
   polling rare contention is unlikely in
   practice, but a lock-free atomic-counter
   variant is a Phase 140+ candidate if
   measurement shows audio glitches under
   load.

4. **No AudioIn unit tests for the new
   accessors.** Like the existing
   WHISPER_SAMPLE_RATE sanity, the
   silence-detection wiring needs a real cpal
   device to exercise. The substrate-tier
   `SilenceDetector` tests are exhaustive (10
   tests covering every state transition)
   and the AudioIn glue is straightforward
   delegation. Operator-validation tier.

5. **macOS Send constraint still applies** —
   Phase 138 carry-over; not Phase 139 work.

### Direction after Phase 139

Voice now loops hands-free. Phase 140+
candidates:

1. **`[voice.vad]` TOML config** — operator-
   tunable threshold + dwell + min-speech +
   max-capture knobs. Small substrate work;
   first thing to ship if operators hit
   environment-specific tuning needs.
2. **Mid-recording manual-abort UX** — separate
   stdin listener task that races against the
   silence detector via mpsc.
3. **Silero ONNX VAD** — drop-in replacement
   when energy threshold isn't robust enough.
   Adds ONNX runtime prereq.
4. **Mid-synthesis abort UX** — drain the TTS
   mpsc on Ctrl-C / Escape (Phase 138
   carry-over).
5. **Streaming ASR** — Whisper partial-decode
   mode for true-streaming transcription. Big
   scope; multi-phase.
6. **Wake-word activation** ("Hey Aivyx") via
   Porcupine or Silero-wakeword. Builds on
   VAD substrate but adds always-on listening
   privacy posture.
7. **Multimodal output** — agent speaks image
   descriptions via vision-capable LLMs.
8. **macOS streaming variant** — `LocalSet`-
   based consumer task.
9. **Lock-free AudioIn detector** — atomic
   counter pattern if measurement shows audio
   glitches.
10. **whisper-cpp-plus rehabilitation** —
    Phase 135 Q2c close-out once upstream is
    unstuck.
11. **`build_agent_stack` substrate-tier
    promotion** if more channel adapters
    ship.
12. **Channel Activation Milestone** — still
    held intentionally; 28th consecutive
    deferral at Phase 139 exit.
