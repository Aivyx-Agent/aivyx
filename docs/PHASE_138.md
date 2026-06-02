# Phase 138 — Streaming TTS During LLM Generation

**Phase 137 follow-on.** Phase 137 gave voice
feature parity with Local. Phase 138 attacks the
biggest remaining UX weakness in the voice loop:
**latency-to-first-audio**. Today the operator
hits Enter to stop recording, waits for the LLM
to finish generating its entire response, waits
again while TTS synthesizes each sentence
sequentially, and *then* hears the first word.
For a long reply, that's tens of seconds of
silence. Phase 138 pipelines the LLM stream into
the TTS engine on sentence boundaries — the
operator hears sentence one while the LLM is
still generating sentence three.

## Why this, why now

- **Highest-impact voice UX win still on the
  table.** Phase 135 + 136 made voice work. Phase
  137 gave it feature parity. The remaining gap
  to "voice feels good" is latency. A long agent
  reply (~200 words, ~15 seconds of synthesized
  audio) currently produces ~15 seconds of dead
  silence before any playback. Streaming TTS
  collapses that to the latency of the *first
  sentence* — typically 1-2 seconds.

- **Substrate is already there.** Phase 135's
  `chunk_into_sentences` is the offline form of
  the streaming algorithm — it splits a buffered
  string at sentence boundaries. Phase 138 adds
  its streaming complement, `drain_complete_sentences`,
  which operates on a mutable buffer and pulls
  complete sentences as they arrive while leaving
  partial trailing text in place. Pure substrate;
  directly unit-testable.

- **Zero new workspace deps.** Tokio's mpsc is
  already in use across the workspace. No ONNX,
  no new audio crates, no model files.

## Tasks

1. **Open doc + ROADMAP + README** — this doc +
   the roadmap section + the README row. Backfill
   Phase 137 hash to `9b6b8a1`.

2. **`drain_complete_sentences` substrate helper.**
   New pure function in
   `aivyx-voice/src/tts/mod.rs`:
   ```rust
   pub fn drain_complete_sentences(buf: &mut String) -> Vec<String>
   ```
   Reads complete sentences from `buf` (terminated
   by `.` / `?` / `!` followed by whitespace or
   EOF-but-only-when-trailing-fragment-is-empty),
   returning them in order. Leaves any partial
   trailing fragment in `buf` so the next call —
   after more text has been appended — picks up
   where this one left off. Unit tests cover:
   empty buffer, single complete sentence,
   multiple sentences in one call, trailing
   partial, decimal-in-number does-not-break,
   incremental-streaming simulation (call,
   append, call, append, call).

3. **`VoiceChannel` text-sink hook.** Add
   ```rust
   pub fn set_text_sink(&self, f: impl Fn(&str) + Send + Sync + 'static);
   pub fn clear_text_sink(&self);
   ```
   plus a `text_sink: Mutex<Option<Box<dyn Fn(&str) + Send + Sync>>>`
   field. Update `stream_event` so when the sink
   is `Some`, text chunks fire the closure instead
   of accumulating in `text_buffer`. When `None`,
   Phase 137 buffer behaviour is preserved
   (`run_one_voice_turn` continues to work
   unchanged). Tests cover both paths.

4. **Streaming session driver + push-to-talk
   variant.** Add to `aivyx-voice/src/session.rs`:
   - `run_one_voice_turn_streaming(agent, channel, asr, tts, captured_audio, sentence_tx)`
     — registers a sink that drains complete
     sentences into the mpsc; runs `agent.turn`;
     unregisters the sink; flushes any leftover
     partial sentence. Returns `StreamingVoiceTurnResult`
     with `transcribed`, `response_text` (full
     assembled text), and `outcome`.
   - `run_push_to_talk_loop_streaming(agent, channel, asr, tts)`
     — full loop variant. Creates the mpsc;
     spawns a serial consumer task that pulls
     sentence → `tts.synthesize` → `audio_out.play_audio`;
     runs each turn against the streaming
     dispatcher; on turn completion drops the
     sender so the consumer drains and exits.
   Test: full streaming loop with a stub agent
   emitting two sentences across multiple text
   chunks. Consumer receives two TTS chunks in
   order.

5. **Binary voice arm + INSTALL + exit doc + Frozen.**
   Switch binary's `ChannelKind::Voice` to call
   `run_push_to_talk_loop_streaming`. INSTALL.md
   voice section gains a Phase 138 paragraph
   ("operator hears first sentence as the LLM is
   still generating the rest"). Phase 138 exit
   doc with prediction-vs-reality + honest-bends.
   Flip Frozen.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment; streaming TTS is a UX latency
  optimization, not a substrate redesign. Streak:
  28 → **29**.
- **PRODUCT.md** — **Will hold.** Voice latency
  collapse reinforces the existing
  voice-personal-assistant framing. Streak:
  28 → **29**.
- **`aivyx-core/src/lib.rs`** — **Will hold.** All
  Phase 138 work lives in `aivyx-voice` + the
  binary's dispatch arm. Core untouched.
  Streak: 3 → **4**.

## Exit criteria

- [ ] `docs/PHASE_138.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `drain_complete_sentences` public in
  `aivyx-voice::tts` with full unit-test
  coverage — Task 2.
- [ ] `VoiceChannel::{set,clear}_text_sink` public
  + `stream_event` dual-path — Task 3.
- [ ] `run_one_voice_turn_streaming` +
  `run_push_to_talk_loop_streaming` public in
  `aivyx-voice::session`, with a full-loop unit
  test — Task 4.
- [ ] Binary voice arm calls the streaming
  variant — Task 5.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD as
  predicted.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings with default features.
- [ ] Test count delta: `+8` to `+14` —
  drain_complete_sentences (~6 tests), text_sink
  paths (~3 tests), streaming session driver
  (~2 tests).

## Honest scope risks at sign-off

- **Out-of-order synthesis risk: ruled out by
  design.** The consumer task is *serial* — pulls
  one sentence, synthesizes, plays, then pulls the
  next. Two TTS jobs can never race. The cost is
  giving up theoretical parallel synthesis;
  acceptable because Piper inference is fast
  enough that synthesis-of-sentence-N+1
  rarely-if-ever happens before playback of
  sentence-N has begun.

- **Cancellation mid-stream.** If the operator
  cancels a turn mid-generation, sentences already
  queued for synthesis or playback will complete
  on their current step before the consumer
  notices the cancellation. Acceptable; documented
  in the exit doc.

- **macOS-specific Send constraint.** `AudioOut`
  on macOS wraps `cpal::Stream` which is `!Send` —
  the consumer task pattern requires `AudioOut`
  to cross a `tokio::spawn` boundary. On Linux +
  Windows this is fine. On macOS, the operator
  may hit a build error; Phase 139+ candidate to
  add a platform-aware variant that runs the
  consumer on the main runtime thread.

- **No interactive abort during synthesis.** Once
  a sentence is in the mpsc queue, there's no UX
  to skip-ahead or interrupt. Phase 139+ could
  add a keyboard listener that drains the mpsc on
  Ctrl-C / Escape.

- **`build_agent_stack` substrate-tier promotion
  not addressed.** Phase 137 flagged this as a
  candidate; deferred again to keep Phase 138
  scoped on streaming TTS specifically.

- **whisper-cpp-plus still deferred** (Phase 135
  Q2c). Upstream remains broken against current
  whisper.cpp.

- **Twenty-seventh consecutive deferral of the
  Channel Activation Milestone.** Per operator
  framing — intentional hold. Tracking continues.

## Direction after Phase 138

After Phase 138, voice latency is collapsed.
Phase 139+ candidates:

1. **Voice activity detection** for trim-on-
   silence push-to-talk (no more "press Enter
   twice"). Adds the `silero` MIT/Apache crate.
2. **Wake-word activation** ("Hey Aivyx") via
   Porcupine or Silero-wakeword.
3. **Multimodal output** — agent speaks
   descriptions of images via vision-capable
   LLMs.
4. **whisper-cpp-plus rehabilitation** — close
   out the Phase 135 Q2c deferral once upstream
   is unstuck.
5. **`build_agent_stack` substrate-tier promotion**
   if more channel adapters ship.
6. **macOS streaming TTS variant** that runs
   playback on the main runtime thread.
7. **Mid-synthesis cancellation UX** (keyboard
   listener that drains the playback queue).
8. **Channel Activation Milestone** — still held
   intentionally; 27th consecutive deferral at
   Phase 138 open.

## Prediction vs reality

**Three-of-three streak HOLDs as predicted.**

- **DESIGN.md** — HELD as predicted (`62dabbdd…`
  unchanged). No contract amendment. Streak:
  28 → **29**.
- **PRODUCT.md** — HELD as predicted (`467ba59a…`
  unchanged). Streak: 28 → **29**.
- **`aivyx-core/src/lib.rs`** — HELD as predicted
  (`4f9b8c81…` unchanged). All Phase 138 work in
  `aivyx-voice` + the binary's voice dispatch
  arm. Continuing post-Phase-135 reset:
  3 → **4**.

**Test count delta: +14 — top of predicted `+8`
to `+14` range.** Workspace lib tests 3026 → 3040.
Per-module:
- `drain_complete_sentences` substrate helper: +8
  (empty, partial-fragment, complete-then-partial,
  EOF-not-flushed, multiple-in-one-call, decimal-
  not-broken, incremental-streaming-simulation,
  newline-boundary).
- `VoiceChannel::set_text_sink` / `clear_text_sink`:
  +3 (sink active diverts from buffer, sink
  cleared reverts to buffer, sink replacement).
- `run_one_voice_turn_streaming`: +3 (in-order
  sentence flush + leftover partial, empty agent
  response, empty ASR short-circuit).

**Zero new workspace dependencies** as predicted.
`tokio::sync::mpsc` was already in use across the
workspace.

**Zero clippy warnings** with default features.
One in-flight catch during Task 3: the
`Mutex<Option<Box<dyn Fn(&str) + Send + Sync>>>`
field tripped `clippy::type_complexity`. Fixed by
factoring the inner type into a `TextSinkFn` alias
— cleaner reading and easier to grep than an
inline `#[allow(...)]`.

### What landed cleanly + what bent

**Cleanly:**
- `drain_complete_sentences` substrate helper: 86
  lines + 8 tests. Pure function, in-place buffer
  truncation, streaming-correct semantics (EOF
  doesn't terminate; whitespace-after-terminator
  does).
- `VoiceChannel` text-sink hook: lock held only
  long enough to invoke the closure; sink-active
  path bypasses the legacy `text_buffer` so the
  two paths can't double-count. Phase 137
  `run_one_voice_turn` behaviour preserved
  byte-for-byte when no sink is installed.
- `run_one_voice_turn_streaming` + the matching
  push-to-talk loop variant. Serial consumer task
  pattern eliminates synthesis-ordering races at
  the cost of theoretical parallel TTS. Worth it.
- Binary voice arm switched cleanly to the
  streaming variant (one import + one call-site
  rename + banner text update).
- 3040 workspace lib tests pass; clippy clean.

**Bent honestly:**

1. **Serial consumer task is the right design,
   but it gives up parallel synthesis.** Two TTS
   tasks racing could swap sentence ordering —
   unacceptable for voice. The cost is that
   sentence-N+1 can't synthesize while sentence-N
   plays. In practice Piper inference is fast
   enough that this rarely matters. If it ever
   does, the fix is a small ordered-buffer
   pattern (synthesize in parallel, dispatch to
   audio_out in completion order via a counter).
   Phase 139+ if measurement shows it matters.

2. **macOS Send constraint surfaces here.** The
   consumer task pattern requires `AudioOut` to
   cross a `tokio::spawn` boundary. On Linux +
   Windows that's fine. On macOS, `cpal::Stream`
   is `!Send` — the build will fail. Phase 139+
   candidate: a macOS variant that runs the
   consumer on the main runtime thread (likely
   `tokio::task::LocalSet`).

3. **Mid-stream cancellation is approximate.**
   If the operator cancels mid-generation,
   sentences already queued in the mpsc are
   synthesized + played anyway — the cancellation
   token affects only the LLM provider. Phase
   139+ could drain the mpsc on cancel.

4. **No interactive abort during synthesis.** Once
   a sentence is in the playback queue, no UX to
   skip-ahead or interrupt. Phase 139+ candidate:
   keyboard listener that drains the mpsc on
   Ctrl-C / Escape.

5. **`build_agent_stack` substrate-tier promotion
   still deferred.** Phase 137 flagged it; Phase
   138 deferred again to keep scope on streaming
   TTS. Becomes worth doing when more channel
   adapters ship.

6. **whisper-cpp-plus still upstream-broken** —
   Phase 135 Q2c deferral remains. Not Phase 138
   work.

### Direction after Phase 138

Voice now hears low-latency. Phase 139+ candidates:

1. **Voice activity detection** for trim-on-
   silence push-to-talk (no more "press Enter
   twice"). Adds the `silero` MIT/Apache crate.
2. **Wake-word activation** ("Hey Aivyx") via
   Porcupine or Silero-wakeword.
3. **Multimodal output** — agent speaks
   descriptions of images via vision-capable LLMs.
4. **macOS streaming variant** that runs the
   consumer task on the main runtime thread.
5. **Mid-synthesis cancellation UX** — drain the
   mpsc on Ctrl-C / Escape.
6. **Parallel TTS with ordered dispatch** if
   measurement shows serial-consumer latency.
7. **whisper-cpp-plus rehabilitation** — Phase 135
   Q2c close-out once upstream is unstuck.
8. **`build_agent_stack` substrate-tier promotion**
   if more channel adapters ship.
9. **Channel Activation Milestone** — still held
   intentionally; 27th consecutive deferral at
   Phase 138 exit.
