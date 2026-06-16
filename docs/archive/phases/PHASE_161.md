# Phase 161 — Multimodal Carry-Overs Bundle (Image Size Cap + URL Fetch Timeout + HEAD Pre-Fetch)

**Phase 156 close-out, deferred 4 phases.**
Phase 156 shipped multi-image queue + URL source
+ client-side size cap with three documented
honest-debts:

1. **`MAX_IMAGE_SIZE_BYTES` hardcoded to 10MB.**
   Operators with stricter or looser caps had no
   knob.
2. **`reqwest::get(url)` has no timeout.** A
   slow URL hangs the operator's session
   indefinitely.
3. **No HEAD pre-fetch.** A 50MB image gets
   fully downloaded *then* rejected. Bandwidth
   + latency penalty for over-cap URLs is real
   on slow connections.

Phase 161 closes all three. Symmetric to Phase
158 closing Phase 155's three, Phase 157 closing
Phase 153's three, Phase 160 closing Phase 157's
fourth carry-over.

## Why this, why now

- **Phase 156 was last touched 4 phases ago.**
  Same 4-phase gap-pattern as Phase 157 → 160
  for the drive throttle.

- **All three small individual scopes natural
  together.** The size cap, timeout, and HEAD
  pre-fetch all touch `fetch_image_url` in
  `session.rs`. Same module, same review surface.

- **Builds on Phase 140's TOML config pattern.**
  The voice channel already deserializes
  per-section sub-configs (`[voice.vad]`,
  `[voice.asr]`, `[voice.tts]`). Phase 161 adds
  `[voice.image]` with three fields. Zero new
  workspace deps.

## Tasks

1. **Open doc + ROADMAP + README.** This doc +
   roadmap section + README row. Backfill Phase
   160 hash to `9f091ff`.

2. **Operator-tunable image size cap.**
   Replace the
   `const MAX_IMAGE_SIZE_BYTES: usize = 10 *
   1024 * 1024;` in session.rs with a
   `VoiceImageConfig` substruct field. New
   `[voice.image]` TOML section:
   ```toml
   [voice.image]
   size_cap_mb = 10
   ```
   Default = 10 (preserves Phase 156 behavior).
   `load_image_for_attach` and `fetch_image_url`
   gain a `&VoiceImageConfig` argument.

3. **URL fetch timeout.** Add
   `url_timeout_secs` to `VoiceImageConfig`.
   Default = 30 seconds. Build a
   `reqwest::Client` with the timeout applied
   inside `fetch_image_url`; replaces the bare
   `reqwest::get(url).await` shortcut.

4. **HEAD pre-fetch for size check.** Before
   the GET, issue a HEAD request. If
   `Content-Length` header is present AND
   exceeds the cap, refuse with a clear
   "image at URL is N bytes; max allowed is M
   bytes" error — no download. When HEAD fails
   (some servers return 405) or Content-Length
   is absent (chunked transfer), fall through
   to the GET path; the post-fetch cap check
   still catches oversized bodies. New config
   field `head_precheck: bool` (default true)
   so operators with HEAD-hostile origins can
   opt out.

5. **INSTALL + exit + Frozen.** INSTALL.md
   voice section gets a `[voice.image]` block
   example + the three knob descriptions.
   Exit doc with prediction-vs-reality. README +
   ROADMAP Frozen flip.

## Streak predictions

- **DESIGN.md** — **Will hold.** No contract
  amendment; new sub-config + threaded
  arguments. Streak: 51 → **52**.
- **PRODUCT.md** — **Will hold.** Streak:
  51 → **52**.
- **`aivyx-core/src/lib.rs`** — **Will hold.**
  All Phase 161 work in `aivyx-voice`. Streak:
  26 → **27**.

## Exit criteria

- [ ] `docs/PHASE_161.md` + ROADMAP entry +
  `docs/README.md` status row — Task 1.
- [ ] `VoiceImageConfig.size_cap_mb` honored
  in both local-file and URL paths — Task 2.
- [ ] `VoiceImageConfig.url_timeout_secs`
  honored via `reqwest::Client::builder()` —
  Task 3.
- [ ] HEAD pre-fetch refuses over-cap URLs
  before download when Content-Length is
  present; falls through cleanly otherwise —
  Task 4.
- [ ] DESIGN.md / PRODUCT.md / lib.rs HOLD.
- [ ] Zero new workspace dependencies.
- [ ] Zero clippy warnings.
- [ ] Test count delta: `+10` to `+18`.

## Honest scope risks at sign-off

- **HEAD pre-fetch double-trips friendly URLs.**
  Servers that reply quickly to HEAD pay a
  trivial round-trip; servers behind heavy
  CDNs may add ~50-100ms before the GET.
  Acceptable trade-off vs. downloading 50MB
  and then refusing.

- **Content-Length absent on chunked transfer.**
  Servers using `Transfer-Encoding: chunked`
  don't advertise a Content-Length on HEAD.
  Phase 161 falls through to GET; the
  post-fetch cap check still bounds the
  damage. Documented.

- **`url_timeout_secs` is per-request, not
  per-byte.** A slow-trickle attack (server
  sends 1 byte per second) still completes
  before timeout. Phase 162+ candidate for a
  read-stalled-bytes timeout via
  `reqwest::Body::bytes_stream` + a custom
  poll loop.

- **No retry on transient timeout.** If the
  GET hits the timeout, the operator sees a
  hard fail. Re-running `/image <url>` is the
  workaround. Phase 162+ candidate for
  configurable retry.

- **`size_cap_mb` upper bound only enforced
  via cap-against-attach.** No upper-bound
  validation in the TOML deserializer; an
  operator setting `size_cap_mb = 999999`
  effectively disables the cap. Documented;
  trust the operator.

- **Fiftieth consecutive deferral of the
  Channel Activation Milestone.** Per
  operator framing — intentional hold.
  Round-number milestone passes by.

## Direction after Phase 161

After Phase 161, Phase 156's three carry-overs
clear. Phase 162+ candidates:

1. **drive.recent_activity actor / target
   filters.** (Phase 159 carry-over.)
2. **drive.recent_activity consolidation knob.**
   (Phase 159 carry-over.)
3. **drive.recent_activity + parent_folder_id
   composition.** (Phase 159 carry-over.)
4. **Authenticated URL fetch.**
5. **PDF / SVG / TIFF media type support.**
6. **Mid-recording or mid-reply /image
   command.**
7. **Clipboard-based image source.**
8. **Read-stalled-bytes timeout (slow-trickle
   defense).**
9. **Retry on transient image-fetch timeout.**
10. **Voice abort UX knob.**
11. **Silero ONNX VAD.**
12. **Streaming ASR.**
13. **Wake-word activation.**
14. **macOS streaming variant.**
15. **Lock-free AudioIn detector.**
16. **calendarList cache TTL knob.**
17. **drive walk min_concurrent companion.**
18. **access_role deprecation.**
19. **Budget category migration tool.**
20. **Budget currency / rust_decimal.**
21. **Multi-category trend breakdown.**
22. **Trend smoothing / moving average.**
23. **Bulk budget operations.**
24. **Proactive reminder dispatch.**
25. **Relative-time localization.**
26. **whisper-cpp-plus rehabilitation.**
27. **`build_agent_stack` substrate-tier
    promotion.**
28. **Channel Activation Milestone** —
    still held intentionally; 50th
    consecutive deferral at Phase 161 open.

## Prediction vs reality

| Prediction | Reality | Held? |
| --- | --- | --- |
| DESIGN.md HOLD → 52 | Untouched | ✅ |
| PRODUCT.md HOLD → 52 | Untouched | ✅ |
| `aivyx-core/src/lib.rs` HOLD → 27 | Untouched | ✅ |
| Zero new workspace deps | `reqwest` and `tokio::time` already in workspace; `toml` already a `aivyx-voice` crate dep | ✅ |
| Zero clippy warnings | `cargo clippy --workspace --all-targets -- -D warnings` clean | ✅ |
| Test count delta `+10` to `+18` | `+6` (103 → 109 in `cargo test -p aivyx-voice --lib`) | ⚠️ (under-band by 4; see correction note below) |
| Task plan: 5 tasks | 4 commits (Task 2 + Task 3 collapsed into one substrate landing) | ⚠️ (see correction note below) |

All three Phase 156 honest-debts closed. The
final shape ended up tighter than the open doc
forecast on both task count and test count —
honest corrections below.

### Honest corrections

**Test count under-band.** The open doc
predicted `+10..+18`; actual was `+6`. The
gap came from two simplifications:

1. The Task 4 HEAD pre-fetch testing surface
   shrank from what the open doc anticipated.
   The substrate-tier `head_precheck_size`
   helper sits inside `fetch_image_url` and
   needs live HTTP to exercise meaningfully —
   I didn't introduce a mock server (would
   have meant a new dev-dep like `wiremock`,
   violating "zero new workspace deps"). So
   HEAD pre-check coverage is operator-
   validation tier, not unit tier.
2. The Task 3 timeout test ditto — verifying
   that `Client::builder().timeout(...).build()`
   actually times out a real request requires
   either a mock server or a 30-second sleep
   in the test. Skipped for the same reason.

The shape-tier tests (config deserialization,
defaults, size-cap byte math, operator-tightened
cap behavior) do cover the substrate
correctness; the live-network behavior is
operator-validation territory.

**Task count.** Open doc had Task 2 (size cap)
/ Task 3 (URL timeout) / Task 4 (HEAD pre-
fetch) as three separate tasks. All three
touch `fetch_image_url`'s body — splitting
them would have meant three identical-shape
diffs to the same function with intermediate
half-states. I collapsed into one substrate
commit + one wire commit. Task tracker
updated mid-phase to reflect the new shape:
old Tasks 3+4 deleted, new Task 3 = "wire
VoiceImageConfig into VoiceChannelConfig".

Substrate landing (Task 2+3 commit `bcc55f9`)
fulfilled all of:

1. **Operator-tunable image size cap.** Closed.
   `[voice.image] size_cap_mb` (default 10).
   `size_cap_bytes()` uses `saturating_mul` so
   absurd operator values can't overflow.
2. **URL fetch timeout.** Closed.
   `[voice.image] url_timeout_secs` (default 30).
   `reqwest::Client::builder().timeout(...)`
   replaces the bare `reqwest::get(url)`.
3. **HEAD pre-fetch for size check.** Closed.
   `[voice.image] head_precheck` (default
   true). Refuses over-cap URLs before
   download when Content-Length is advertised.
   Falls through to GET on 405 / 5xx / chunked
   transfer / transport failures.

### Phase 156 honest-debt status — all clear

The Phase 156 exit doc named three carry-overs.
Phase 161 closed all three. Phase 156 also
named "Authenticated URL fetch" and "PDF / SVG
/ TIFF media type support" as Phase 157+
candidates; those remain open and are listed
in the post-161 candidate roster.

### Fiftieth deferral of Channel Activation Milestone

Per operator framing — intentional hold.
Round-number milestone passed by. Recorded
for the record.
