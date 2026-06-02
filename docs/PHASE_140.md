# Phase 140 — Close Phase 139 Debt: TOML Config + Manual-Abort UX

**Phase 139 close-out.** Phase 139 shipped energy-
threshold VAD for auto-stop PTT, but with two
documented debts:

1. **Thresholds hardcoded.** Quiet rooms work
   well with the defaults; noisy environments
   need either threshold tuning or ML VAD. No
   way for operators to tune without
   recompiling.
2. **No manual abort during recording.**
   Phase 138's "press Enter to abort"
   affordance went away. Operators wanting to
   abandon a half-spoken message had to wait
   1.5s of silence then accept the partial-
   transcription turn.

Phase 140 closes both debts in one phase.

## Why this, why now

- **Both debts are small individually and
  natural together.** TOML config is ~50 lines
  of plumbing. Manual abort is ~30 lines of
  `tokio::select!`. They share no code but
  they share the user-facing voice-loop
  context, so bundling them keeps INSTALL
  documentation coherent.

- **The honest-debt list grew through Phases
  135-139.** Five consecutive voice phases
  shipped real capabilities but each deferred
  something. Phase 140 explicitly clears two
  of the largest deferred items before the
  list compounds further.

- **Zero new workspace deps.** TOML config
  plumbs through existing serde infrastructure;
  manual abort uses `tokio::io::stdin` +
  `tokio::sync::mpsc` already in workspace.

## Tasks

1. **Open doc + ROADMAP + README** — this doc +
   the roadmap section + the README row.
   Backfill Phase 139 hash to `38734ec`.

2. **`VoiceVadConfig` TOML section.** New struct
   in `aivyx-voice` with serde defaults matching
   Phase 139's hardcoded values:
   ```toml
   [voice.vad]
   threshold_rms     = 0.01
   dwell_secs        = 1.5
   min_speech_secs   = 0.5
   max_capture_secs  = 30.0
   frame_secs        = 0.030
   poll_interval_ms  = 100
   ```
   Wires into `VoiceChannelConfig` as
   `#[serde(default)] vad: VoiceVadConfig`.
   Adds `VoiceVadConfig::to_silence_detector_config(sample_rate)`
   bridge. Unit tests cover: default parses,
   full TOML section parses, partial TOML uses
   defaults for missing fields.

3. **AudioIn + PTT loop read config from
   channel.** AudioIn gains
   `new_with_detector(device, SilenceDetectorConfig)`
   constructor; legacy `AudioIn::new` keeps
   default behavior. The streaming PTT loop
   reads `channel.config().vad`, converts to
   `SilenceDetectorConfig`, builds AudioIn with
   it. Hardcoded constants in the loop replaced
   by config-driven Duration values.

4. **Manual-abort stdin listener +
   `tokio::select!` race.** Long-lived stdin
   reader task wraps `tokio::io::stdin().lines()`;
   sends each line to a `mpsc::UnboundedSender<String>`.
   PTT loop's start-of-iteration "press Enter"
   prompt reads from the rx. During recording,
   the inner poll loop uses `tokio::select!` to
   race `tokio::time::sleep(poll_interval)`
   against `line_rx.recv()`. Enter mid-recording:
   `aborted_manually = true`, break. Aborted-but-
   non-empty captures still dispatch (operator
   may have started a thought). Aborted-and-empty
   captures skip turn with a heads-up message.

5. **INSTALL + exit + Frozen.** Voice section
   gains a Phase 140 paragraph at the top.
   Documents the `[voice.vad]` knob with a
   defaults table; describes the recovered
   manual-abort affordance. Phase 140 exit
   doc with prediction-vs-reality + honest
   bends.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment; both deliverables are UX/config
  layers on existing substrate. Streak:
  30 → **31**.
- **PRODUCT.md** — **Will hold.** Operator-
  tunable voice + recoverable abort reinforces
  the voice-personal-assistant framing.
  Streak: 30 → **31**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 140 work in `aivyx-voice`. Core
  untouched. Streak: 5 → **6**.

## Exit criteria

- [ ] `docs/PHASE_140.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `VoiceVadConfig` public in `aivyx-voice`
  with full unit-test coverage — Task 2.
- [ ] `AudioIn::new_with_detector` public; PTT
  loop reads from `VoiceVadConfig` — Task 3.
- [ ] Manual abort works mid-recording via
  Enter; aborted-non-empty captures still
  dispatch — Task 4.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+4` to `+8`
  (VoiceVadConfig ~4 tests; manual-abort
  exercised end-to-end via stub agent in
  the streaming session driver tests if
  feasible, otherwise operator-validation).

## Honest scope risks at sign-off

- **Manual abort interacts subtly with the
  serial consumer task.** If the operator
  aborts mid-recording before any speech
  registered, the dispatched turn would
  fail at ASR (empty samples) — which Phase
  138's `Ok(None)` branch already handles
  gracefully. Tested via the existing
  `run_one_voice_turn_streaming_empty_asr_returns_none`
  test.

- **Stdin reader task outlives the recording
  loop.** The long-lived stdin reader is
  spawned once per PTT loop invocation and
  lives for the whole REPL session. On
  `quit`, the function returns + the reader's
  mpsc receiver drops + the reader task's
  send fails + the task exits. Clean shutdown
  by ownership.

- **TOML config bypasses validation.** The
  serde defaults are sensible but operators
  could set `dwell_secs = -1.0` or
  `threshold_rms = 100.0` and get nonsense
  behavior. Phase 140 doesn't add validation
  bounds; operators get what they ask for.
  Phase 141+ could add `Result<()>` validation
  with bounded ranges if it surfaces as a
  support burden.

- **No streaming-pipeline visibility from the
  config.** Operators tune VAD via TOML but
  the streaming-TTS sentence-buffering
  behavior remains hardcoded. Phase 141+
  candidate if operators want per-sentence
  artificial-pause inserts or similar.

- **macOS Send constraint still applies** —
  Phase 138 carry-over; not Phase 140 work.

- **Twenty-ninth consecutive deferral of the
  Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 140

After Phase 140, Phase 139's debt closes.
Phase 141+ candidates:

1. **Mid-synthesis abort UX** — drain the TTS
   playback mpsc on Ctrl-C / Escape. Different
   feature surface from Phase 140's recording-
   side abort. Phase 138 carry-over.
2. **Silero ONNX VAD** — drop-in replacement
   when energy threshold isn't robust enough.
3. **Streaming ASR** — Whisper partial-decode
   for true-streaming transcription.
4. **Wake-word activation** ("Hey Aivyx").
5. **Multimodal output** — agent speaks image
   descriptions via vision-capable LLMs.
6. **macOS streaming variant** — `LocalSet`-
   based consumer task.
7. **Lock-free AudioIn detector** — atomic
   counter pattern if measurement shows audio
   glitches.
8. **whisper-cpp-plus rehabilitation** —
   Phase 135 Q2c close-out.
9. **`build_agent_stack` substrate-tier
   promotion** if more channel adapters ship.
10. **Pivot from voice — Chapter G toolkit
    expansion** (calendar reminders, budget
    tracking, health.check.remove).
11. **Channel Activation Milestone** — still
    held intentionally; 29th consecutive
    deferral at Phase 140 open.

## Prediction vs reality

_Populated at Phase 140 exit. Predictions at
sign-off: DESIGN.md HOLD → 31; PRODUCT.md HOLD
→ 31; lib.rs HOLD → 6; zero new deps; test
count delta `+4` to `+8`; zero clippy
warnings._
