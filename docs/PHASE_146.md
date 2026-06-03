# Phase 146 — Voice Mid-Synthesis Abort UX

**Phase 138 close-out, deferred 7 phases.** Phase
138 shipped streaming TTS during LLM generation:
the operator hears sentence one while the LLM is
still generating sentence three. Phase 138's
honest-debt list called out **mid-synthesis abort
UX** — a keyboard listener that drains the TTS
playback mpsc on Ctrl-C/Escape so the operator
can interrupt long agent replies. Seven phases of
voice + integration work later, Phase 146 ships
it.

## Why this, why now

- **Pivot after 5 consecutive integration phases.**
  Phases 141-145 ran toolkit/calendar/budget/drive
  expansion. Phase 146 returns to voice for a
  small, focused close-out.

- **The longest-deferred honest-debt.** Voice has
  had 8 phases (135-140 + 146), Chapter F/G has
  had 11 phases (123-130 + 141-145). Phase 138's
  mid-synthesis abort callout has been carried in
  every subsequent voice phase's exit doc without
  resolution. Closing it now restores parity with
  Phase 140's recording-side abort.

- **Substrate is already in place.** Phase 138's
  serial-consumer-task pattern + Phase 140's
  long-lived async stdin reader give us all the
  pieces. The synthesis-phase abort is one
  `tokio::select!` racing the streaming-turn
  future against `line_rx.recv()`, plus a small
  abort-signal mpsc to ask the consumer task to
  call a new `AudioOut::stop_playback()`.

- **Zero new workspace deps.** Pure refactor of
  the existing streaming PTT loop.

## Tasks

1. **Open doc + ROADMAP + README** — this doc +
   the roadmap section + the README row.
   Backfill Phase 145 hash to `e0a0b81`.

2. **`AudioOut::stop_playback()` method.** Add
   one new method that drops queued sources and
   pauses the rodio Player via its `clear()`
   method:
   ```rust
   pub fn stop_playback(&self) {
       self.player.clear();
   }
   ```
   No state change otherwise; the AudioOut stays
   usable but the queue is empty + playback
   paused. The consumer task that owns the
   AudioOut calls this on abort signal then
   exits, dropping the AudioOut entirely (which
   releases the cpal stream).

3. **Synthesis-phase `tokio::select!` + abort
   signal pipeline.** Restructure the streaming
   PTT loop's synthesis phase:
   - Add an `mpsc::channel::<()>` for abort
     signaling.
   - Consumer task uses `tokio::select!` to
     listen for sentences from `sentence_rx` OR
     the abort signal. On abort, calls
     `audio_out.stop_playback()` and exits
     immediately (does NOT call
     `sleep_until_empty()` — the queue's already
     cleared).
   - Main loop wraps `run_one_voice_turn_streaming`
     in `tokio::select!` racing against
     `line_rx.recv()`. On line received during
     synthesis: send `abort_tx`,
     `channel.cancel_inflight()` (propagates to
     the LLM-provider cancellation token),
     await consumer.
   - Three resolution paths:
     - Normal completion → await consumer drain
       + dispatch (or `(no speech)` if empty).
     - Abort-continue (Enter) → log + iterate.
     - Abort-quit (`"quit"` mid-synthesis) →
       return Ok(()).
   - Drain stale `line_rx` events between
     recording phase and synthesis phase so
     rapid double-Enters during recording don't
     auto-abort the synthesis.

4. **INSTALL + exit + Frozen.** INSTALL.md
   voice section gains a Phase 146 paragraph at
   the top documenting the new keybind, what
   gets cancelled, and the "quit" mid-synthesis
   special case. Phase 146 exit doc with
   prediction-vs-reality.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment; pure UX layer on existing
  substrate. Streak: 36 → **37**.
- **PRODUCT.md** — **Will hold.** Recoverable
  long-reply UX reinforces the personal-
  assistant framing. Streak: 36 → **37**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 146 work in `aivyx-voice`. Core
  untouched. Streak: 11 → **12**.

## Exit criteria

- [ ] `docs/PHASE_146.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `AudioOut::stop_playback` public —
  Task 2.
- [ ] PTT loop synthesis-phase abort race +
  abort signal pipeline — Task 3.
- [ ] Phase 140's recording-phase manual abort
  still works (regression boundary) — Task 3.
- [ ] Phase 140's silence-detection auto-stop
  still works — Task 3.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+1` to `+5`. Mid-
  synthesis abort needs a real-audio scenario
  to fully exercise; substrate-tier additions
  (stop_playback, the abort-signal mpsc shape)
  add minimal tests.

## Honest scope risks at sign-off

- **Operator-validation tier for end-to-end.**
  The `tokio::select!` race between the
  streaming-turn future and `line_rx` is
  structurally correct but needs real keyboard
  + real microphone + real audio device to
  validate the full UX. Stub-agent tests can
  exercise the abort-signal mpsc shape but not
  the rodio-playback-actually-stops behavior.

- **`Player::clear()` may not interrupt
  mid-source.** rodio drops queued sources +
  pauses, but the currently-playing sample may
  finish its current chunk before silence. For
  typical sentence lengths (1-3 seconds), the
  operator may hear the rest of the current
  word before silence. Acceptable for MVP;
  Phase 147+ could ship a more aggressive
  abort that drops the cpal stream
  immediately.

- **`channel.cancel_inflight()` doesn't
  guarantee fast LLM provider exit.** The
  cancellation token propagates through
  Aivyx's planner; depending on the provider's
  await-point density, the LLM call may return
  shortly after or take up to seconds. For
  long replies, the user-visible UX is still
  "playback stops immediately" via the
  consumer abort; the LLM call winding down
  in the background is invisible.

- **Stale line_rx draining is best-effort.**
  We drain before synthesis but a line that
  arrives between drain and select-start would
  still false-abort. Tokio's mpsc has no
  "atomic drain + start watching" primitive;
  the race window is small (microseconds) and
  acceptable.

- **No "abort but keep what we heard so far"
  UX.** Phase 146 abort cancels the agent
  turn entirely; the partial reply text
  isn't surfaced. Phase 147+ candidate if
  operators want partial-text preservation.

- **Thirty-fifth consecutive deferral of the
  Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 146

After Phase 146, both recording-side AND
synthesis-side abort UX exist. Phase 147+
candidates:

1. **Aggressive abort** — drop the cpal
   stream for instant silence, even
   mid-sample.
2. **Partial-text preservation on abort.**
3. **Silero ONNX VAD** for noisy environments.
4. **Streaming ASR** via Whisper partial-
   decode.
5. **Wake-word activation** ("Hey Aivyx").
6. **Multimodal output** — spoken image
   descriptions.
7. **macOS streaming variant** — `LocalSet`-
   based consumer task.
8. **Lock-free AudioIn detector** if audio
   glitches surface.
9. **VAD config validation** — bounded-range
   serde validation.
10. **Drive Activity API.**
11. **drive.list_drives.**
12. **Category whitelist + case-fold for
    budget.**
13. **Budget currency / decimal switch.**
14. **`budget.trend`.**
15. **Chapter G health.check.remove + alert
    dispatch.**
16. **Proactive reminder dispatch.**
17. **Phase 142 debt cleanup.**
18. **Relative-time localization.**
19. **whisper-cpp-plus rehabilitation.**
20. **`build_agent_stack` substrate-tier
    promotion** if more channel adapters
    ship.
21. **Channel Activation Milestone** — still
    held intentionally; 35th consecutive
    deferral at Phase 146 open.

## Prediction vs reality

_Populated at Phase 146 exit. Predictions at
sign-off: DESIGN.md HOLD → 37; PRODUCT.md HOLD
→ 37; lib.rs HOLD → 12; zero new deps; test
count delta `+1` to `+5`; zero clippy
warnings._
