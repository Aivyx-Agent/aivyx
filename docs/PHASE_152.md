# Phase 152 — Voice Carry-Overs Bundle

**Pivot back to voice after 5 consecutive non-
voice phases.** Voice was last touched in Phase
146 (mid-synthesis abort UX). Phases 147-151
delivered Chapter G health closure, drive +
budget + calendar refinement. Voice carry-overs
have been piling up; Phase 152 closes three of
the smallest.

The three deliverables, mirroring Phase 151's
calendar debt-cleanup bundle pattern:

1. **Aggressive voice abort** (Phase 146 #1).
   Phase 146 abort uses rodio's `Player::clear()`
   which calls `sleep_until_end()` internally —
   the operator hears the current sample finish
   before silence. Phase 152 replaces this with
   instant drop of the cpal stream (audio stops
   within OS buffer time, typically ~10ms).

2. **Partial-text preservation on abort** (Phase
   146 #2). When the operator aborts mid-reply,
   the partial response_text is currently lost
   when the streaming-turn future drops. Phase
   152 surfaces it so the agent can paraphrase
   what was said or include it in audit.

3. **VAD config bounded-range validation**
   (Phase 140 carry-over). `VoiceVadConfig`
   deserialization currently accepts any numeric
   value — `threshold_rms = -1.0` or
   `dwell_secs = 0` produces nonsense behavior at
   runtime. Phase 152 adds a `validate()` method
   that rejects out-of-range values at PTT loop
   entry with a clear error.

## Why this, why now

- **Voice has been quiet for 5 phases.** Phase
  151 closed Phase 142's three calendar honest-
  debts (carried 9 phases); Phase 152 closes
  three voice honest-debts (carried 6 phases
  for Phase 146 #1+#2, 12 phases for the Phase
  140 #2).

- **Symmetric to Phase 144 / 148 / 151 pattern.**
  Three Phase-N close-outs bundled into one
  Phase-N+M cleanup. Operator-readable progress
  on the debt list without sprawling phase
  scope.

- **All three are small individual scopes
  natural together.** Aggressive abort is a
  one-line behavior change. Partial-text
  preservation is ~30 lines of Arc<Mutex<String>>
  threading. VAD validation is ~30 lines of
  bound-check helpers. Bundling them keeps
  INSTALL coherent ("voice abort feels snappier
  AND the agent remembers what it said AND
  bad VAD config errors clearly").

- **Zero new workspace deps.**

## Tasks

1. **Open doc + ROADMAP + README** — this doc +
   the roadmap section + the README row.
   Backfill Phase 151 hash to `10a711f`.

2. **VAD config bounded-range validation.** Add
   `VoiceVadConfig::validate() -> Result<(),
   String>`:
   - `threshold_rms` ∈ `[0.0, 10.0]`.
   - `frame_secs` ∈ `(0.0, 1.0]`.
   - `dwell_secs > 0.0` (and reasonable upper
     bound like 60.0).
   - `min_speech_secs >= 0.0`.
   - `max_capture_secs > 0.0` (and reasonable
     upper bound like 3600.0).
   - `poll_interval_ms >= 1`.
   Call from `run_push_to_talk_loop_streaming`
   at function entry, returning an error if
   invalid. Tests cover each bound's accept +
   reject cases.

3. **Aggressive abort + partial-text
   preservation.** Two Phase 146 carry-overs in
   one task (both touch the streaming PTT
   loop):
   - **Aggressive abort:** remove
     `audio_out.stop_playback()` from the
     consumer's abort arm. AudioOut drops on
     return; cpal stream drops; audio stops at
     OS buffer time. Document the trade-off
     (instant silence vs current-sample
     graceful-end behavior). The existing
     `AudioOut::stop_playback()` method stays
     for non-abort use.
   - **Partial-text preservation:**
     `run_one_voice_turn_streaming` gains an
     `Arc<Mutex<String>>` `assembled` parameter
     (or returns a way to recover it). The
     sink writes to it. The PTT loop holds a
     clone + reads in `AbortedContinue` /
     `AbortedQuit` paths; surfaces "you said:
     ..." even on abort.

4. **INSTALL + exit + Frozen.** INSTALL.md
   voice section gains a Phase 152 paragraph
   noting the three close-outs. Phase 152
   exit doc with prediction-vs-reality.
   README + ROADMAP flip Phase 152 to Frozen.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment; UX polish + validation. Streak:
  42 → **43**.
- **PRODUCT.md** — **Will hold.** Streak:
  42 → **43**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 152 work in `aivyx-voice`. Core
  untouched. Streak: 17 → **18**.

## Exit criteria

- [ ] `docs/PHASE_152.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `VoiceVadConfig::validate` public +
  tested + called from PTT loop entry —
  Task 2.
- [ ] Aggressive abort: consumer no longer
  calls `stop_playback` on abort signal —
  Task 3.
- [ ] Partial-text preservation: abort paths
  surface assembled response_text — Task 3.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+6` to `+12` (VAD
  validation ~6-8 tests + partial-text +
  aggressive abort are operator-validation
  tier).

## Honest scope risks at sign-off

- **Aggressive abort vs current behavior is
  a UX trade-off.** Current rodio `clear()`
  lets the current sample finish (so the
  operator hears the rest of the current
  word). Phase 152 aggressive drops the cpal
  stream — the operator hears silence ~within
  the OS audio buffer time (typically <10ms).
  Some operators may prefer graceful-finish.
  Phase 153+ could add a config knob if
  surfaces.

- **Partial-text preservation only captures
  text emitted before the abort signal.** If
  the LLM is mid-token when cancelled, that
  fragment is whatever the agent's planner
  sent through `stream_event` last. The
  agent's plan may have been incomplete;
  what we capture is the operator's audible
  experience minus playback.

- **VAD validation doesn't migrate existing
  configs.** Operators with a deployed
  `aivyx.toml` containing nonsense VAD
  values now see an error on startup rather
  than silent broken behavior. Net positive
  but a one-time friction at the deploy.
  Documented in INSTALL.

- **Aggressive abort changes existing
  observable behavior.** Operators who had
  built mental models around "I press
  Enter, the current word finishes, then
  silence" now hear "I press Enter, silence
  immediately." Most operators will prefer
  this; some may not. Worth flagging in
  INSTALL.

- **No `validate()` call enforcement at
  config-load time.** Validation runs at
  PTT loop entry. If the operator runs a
  different voice tool path (e.g. a future
  Phase 153+ tool that uses VAD config
  without going through PTT), they'd skip
  the check. Acceptable for Phase 152 — the
  PTT loop is the only consumer today.

- **Forty-first consecutive deferral of
  the Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 152

After Phase 152, the longest-standing voice
debts close. Phase 153+ candidates:

1. **Voice abort UX knob** (graceful vs
   aggressive) if operators want it back.
2. **Silero ONNX VAD.**
3. **Streaming ASR.**
4. **Wake-word activation.**
5. **Multimodal output.**
6. **macOS streaming variant.**
7. **Lock-free AudioIn detector.**
8. **Calendar fuzzy dedup.**
9. **Calendar max_concurrent knob.**
10. **Calendar writable_only filter on
    upcoming.**
11. **access_role deprecation.**
12. **Budget category migration tool.**
13. **Budget currency / rust_decimal.**
14. **Multi-category trend breakdown.**
15. **Trend smoothing / moving average.**
16. **Bulk budget operations.**
17. **Recursive folder filter on drive
    recent_*.**
18. **drive_id parameter on drive recent_*.**
19. **Drive Activity API.**
20. **Proactive reminder dispatch.**
21. **Relative-time localization.**
22. **whisper-cpp-plus rehabilitation.**
23. **`build_agent_stack` substrate-tier
    promotion.**
24. **Channel Activation Milestone** —
    still held intentionally; 41st
    consecutive deferral at Phase 152
    open.

## Prediction vs reality

_Populated at Phase 152 exit. Predictions at
sign-off: DESIGN.md HOLD → 43; PRODUCT.md HOLD
→ 43; lib.rs HOLD → 18; zero new deps; test
count delta `+6` to `+12`; zero clippy
warnings._
