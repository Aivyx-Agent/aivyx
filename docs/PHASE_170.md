# Phase 170 — Voice Polish Bundle (Mid-Recording /image + Clipboard Source + Abort UX Knob)

**Three deferred voice carry-overs in one
bundle.** Phase 170 is a round-number
milestone; rather than spend it on the long-
deferred Channel Activation Milestone, the
user picked the voice polish bundle.

## Why this, why now

- **Two carry-overs from Phase 156 + 162 are
  multimodal-attach quality-of-life.** Mid-
  recording /image and clipboard source both
  reduce friction on the operator-facing
  `/image` command surface.

- **Phase 146's abort UX has aged 24 phases.**
  A small operator-tunable knob lands.

- **Zero new workspace deps target.**
  Clipboard reads shell out to `wl-paste` /
  `xclip` / `pbpaste` based on platform —
  cross-platform `arboard` would be a new
  dep; we instead use what's already on
  most operator boxes.

## Tasks

1. **Open doc + ROADMAP + README.** This doc +
   roadmap section + README row. Backfill
   Phase 169 hash to `e1c5bf4`.

2. **Mid-recording `/image` command.** The
   voice loop's recording phase polls
   `line_rx` alongside the silence-detection
   timer. Today any line input stops the
   recording with `stopped_reason = "manual"`.
   Phase 170 changes this: when the line
   starts with `/image `, parse the path,
   queue the image into `pending_images`,
   and CONTINUE recording. Only an empty
   Enter (or `quit`) stops the recording.
   Other unknown commands still stop with
   the manual reason as before.

3. **Clipboard image source.** Support
   `/image clipboard` by shelling out to
   the platform clipboard tool:
   - Linux/Wayland: `wl-paste --type
     image/png`
   - Linux/X11: `xclip -selection clipboard
     -t image/png -o`
   - macOS: `pbpaste` (with image detection
     via `osascript -e 'the clipboard as ...'`
     fallback; honest scope risk: macOS
     clipboard image handling has historical
     surface complexity. Phase 170 attempts
     `pbpaste -Prefer raw` and falls through
     to an honest error.)
   When the operator types `/image
   clipboard`, the substrate detects the
   platform via `cfg(target_os)`, runs the
   command, infers media type from the bytes
   (PNG starts with 0x89 PNG; JPEG with
   0xFF D8). Tests pin the platform-
   selection logic; live runtime is
   operator-validation tier.

4. **Voice abort UX knob.** Phase 146's
   mid-synthesis abort fires on any single
   Enter. Some operators have requested a
   double-Enter guard so a stray Enter
   doesn't kill a long reply. Add:
   ```toml
   [voice]
   abort_requires_double_enter      = false  # Default false.
   abort_double_enter_window_ms     = 800    # Default 800ms.
   ```
   When `abort_requires_double_enter` is
   true, the abort path requires two Enter
   presses within the window. Single Enter
   becomes a no-op (still drained from
   line_rx). Default false preserves Phase
   146 single-Enter abort behavior.

5. **INSTALL + exit + Frozen.** INSTALL.md
   voice section gets three new behavior
   notes + the abort UX config block. Exit
   doc; README + ROADMAP Frozen flip.

## Streak predictions

- **DESIGN.md** — **Will hold.** Streak:
  6 → **7**.
- **PRODUCT.md** — **Will hold.** Streak:
  60 → **61**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 170 work in `aivyx-voice`.
  Streak: 6 → **7**.

## Exit criteria

- [ ] `docs/PHASE_170.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] Mid-recording `/image <path>` queues +
  continues recording — Task 2.
- [ ] `/image clipboard` reads from platform
  tool and detects PNG / JPEG — Task 3.
- [ ] `abort_requires_double_enter` cfg
  honored — Task 4.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+10` to `+20`.

## Honest scope risks at sign-off

- **Mid-recording `/image` still races
  against the silence-detection timer.**
  Parsing + loading the image takes
  measurable time. If the dwell threshold
  is hit during the load, recording stops
  anyway. Operator can re-press Enter
  after the image is queued. Acceptable
  trade-off.

- **Clipboard read is shell-out, not
  XDG-portal or native API.** Operators
  without `wl-paste` / `xclip` / `pbpaste`
  installed see a clear error pointing to
  the missing tool. Phase 171+ candidate
  for `arboard` integration if surfaces.

- **Clipboard MIME detection is byte-
  prefix-based.** PNG (0x89 50 4E 47) and
  JPEG (0xFF D8) are the only signatures
  Phase 170 recognizes. GIF (0x47 49 46),
  WebP (RIFF...WEBP) added if surfaces;
  current spec covers operator-common
  cases.

- **macOS clipboard image flow is
  honestly fragile.** `pbpaste` doesn't
  natively pipe image bytes; we attempt
  `pbpaste -Prefer raw` (Phase 170 ships
  the path even though it may yield no
  bytes on some macOS versions —
  operator-validation tier confirms what
  works).

- **`abort_requires_double_enter` shifts
  default abort flow.** Pre-170 operators
  who relied on instant single-Enter abort
  must not flip the toggle. Default false
  preserves their muscle memory.

- **Fifty-ninth consecutive deferral of
  the Channel Activation Milestone.** Per
  operator framing — intentional hold.

## Direction after Phase 170

After Phase 170, the multimodal-attach
operator surface gets two QoL wins and Phase
146 sheds one carry-over. Phase 171+
candidates:

1. **Cryptographic PRNG for jitter.**
   (Phase 169 carry-over.)
2. **Full-document-compression PDF page
   count.** (Phase 168 carry-over, needs
   parser dep.)
3. **Streaming document blocks** for large
   PDFs.
4. **`arboard`-based cross-platform
   clipboard** (Phase 170 carry-over if
   shell-out friction surfaces).
5. **GIF / WebP clipboard byte signature
   support** (Phase 170 carry-over).
6. **Silero ONNX VAD.**
7. **Streaming ASR.**
8. **Wake-word activation.**
9. **macOS streaming variant.**
10. **Lock-free AudioIn detector.**
11. **calendarList cache TTL knob.**
12. **access_role deprecation.**
13. **Budget category migration tool.**
14. **Budget currency / rust_decimal.**
15. **Multi-category trend breakdown.**
16. **Trend smoothing / moving average.**
17. **Bulk budget operations.**
18. **Proactive reminder dispatch.**
19. **Relative-time localization.**
20. **whisper-cpp-plus rehabilitation.**
21. **`build_agent_stack` substrate-tier
    promotion.**
22. **Secret-store integration** for
    url_headers.
23. **Channel Activation Milestone** —
    still held intentionally; 59th
    consecutive deferral at Phase 170
    open.

## Prediction vs reality

| Prediction | Reality | Held? |
| --- | --- | --- |
| DESIGN.md HOLD → 7 | Untouched | ✅ |
| PRODUCT.md HOLD → 61 | Untouched | ✅ |
| `aivyx-core/src/lib.rs` HOLD → 7 | Untouched | ✅ |
| Zero new workspace deps | tokio `process` feature added to aivyx-voice's manifest (workspace dep already, transitively enabled). No new workspace crate. | ✅ |
| Zero clippy warnings | `cargo clippy --workspace --all-targets -- -D warnings` clean (after one `doc list item` lint reworded) | ✅ |
| Test count delta `+10` to `+20` | `+18` (mid-record +4, clipboard +6, abort knob +8) | ✅ (within band) |

All three pieces closed:

1. **Mid-recording `/image`** (Task 2, commit
   `f09cfb8`). `/image <path>` mid-recording
   queues the attachment via the same parse +
   preset + load_image_for_attach path; recording
   continues. Other lines (empty Enter, quit,
   unknown commands) still stop.
2. **Clipboard image source** (Task 3, commit
   `2534c4b`). `/image clipboard` shells out
   to wl-paste / xclip / pbpaste by
   `cfg(target_os)` (plus runtime WAYLAND_DISPLAY
   check on Linux). PNG and JPEG byte signatures
   recognized; clear error otherwise.
3. **Abort UX knob** (Task 4, commit
   `a62af7f`). `abort_requires_double_enter`
   gates the mid-synthesis abort flow on a
   second Enter within
   `abort_double_enter_window_ms` (default
   800ms). Single-Enter mode and `quit`
   semantics unchanged.

### What landed beyond the open

One structural change beyond the open's narrow
scope: `VoiceChannelConfig` lost its derived
`Default` impl in favor of an explicit one.
The auto-derived `u64` Default is 0, but the
Phase 170 abort window default is 800ms; an
explicit impl was the cleanest way to enforce
both serde and constructor defaults to the
same value. Other defaults flow through the
underlying type's own `Default` impl
unchanged.

### Voice carry-over status

Phase 170 closed three long-deferred voice
carry-overs:
- ✅ Mid-recording / mid-reply `/image`
- ✅ Clipboard-based image source
- ✅ Phase 146 abort UX knob

New Phase 170 carry-overs (GIF / WebP
clipboard byte signatures, `arboard`-based
clipboard) feed Phase 171+ list.

### Fifty-ninth deferral of Channel Activation Milestone

Per operator framing — intentional hold.
Recorded for the record. Phase 170's round-
number milestone passes by.
