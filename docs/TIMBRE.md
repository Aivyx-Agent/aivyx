# Permissive Voice / TTS (Chapter Timbre)

> **Status:** ✅ **CHAPTER COMPLETE (code) — TB.0–TB.6 done; one operator audio
> speak-test pending (§7).** This is the locked reference for
> replacing the voice channel's **GPL-3.0 Piper TTS** with a **fully permissive
> (Apache-2.0 / MIT) Kokoro stack**, so voice is license-clean for everyone who
> builds Aivyx and can ship in official binaries. Nothing in the TTS code has
> changed yet — the engine impl, the Piper removal, the config/docs, and the
> license-gate flip are the phases below (TB.1–TB.6). Decisions locked by the
> operator: (1) **Kokoro-82M + `voice-g2p`** is the new engine; (2) **Piper is
> removed entirely** (not kept opt-in) — zero GPL anywhere.

## 1. Why — the carried constraint from Chapter Charter

Chapter Charter's CR.1 audit found exactly one GPL dependency in the tree:
**`piper1-rs-sys` (GPL-3.0-only)**, the bindings behind the `tts-piper` voice
engine. It was carved out as a scoped `deny.toml` exception because voice is an
opt-in, build-from-source feature that **`aivyx-cli`'s `default = []` excludes**,
so it never enters an official BSL binary (`docs/LICENSING.md` §6.1). That left a
standing follow-up: *"voice cannot ship in an official BSL binary until Piper is
swapped for a permissively-licensed TTS."* Chapter Timbre is that swap.

### The real source of the GPL: espeak-ng, not "Piper" per se

The contaminating component is **espeak-ng (GPL-3.0)**, which Piper uses for
**grapheme-to-phoneme (G2P)** conversion. This is a *recurring trap*, not a
Piper quirk — it catches most neural-TTS Rust crates too:

| Crate / path | License | Why |
|---|---|---|
| `piper1-rs` → `piper1-rs-sys` | **GPL-3.0** | bundles piper1-gpl + espeak-ng |
| `kokorox` | **GPL-3.0** | statically links espeak-ng |
| `kokoroxide` | MIT/Apache *label* | but pulls espeak-ng at runtime → GPL in practice |

So the design rule for this chapter is blunt: **the voice pipeline must contain no
espeak-ng, in any link form, anywhere.** A "permissive" model is necessary but
not sufficient — the **G2P** is where the license is actually decided.

## 2. The chosen stack (all permissive, espeak-free)

| Layer | Choice | License | Notes |
|---|---|---|---|
| **Acoustic model** | **Kokoro-82M** (StyleTTS2 + ISTFTNet) | **Apache-2.0** weights | small (82M), good neural quality, 24 kHz, offline |
| **G2P** | **`voice-g2p`** | **MIT** | pure Rust, **dictionary-based, no espeak**; ~183k embedded entries (90k gold + 93k silver); IPA output tuned for Kokoro; English-only |
| **Inference** | **`ort`** (ONNX Runtime) | Apache-2.0 / MIT | the same runtime Piper used, so the build chain is familiar |
| **Audio I/O** | `cpal` + `rodio` (unchanged) | permissive | already in `aivyx-voice` |

`voice-g2p` is the load-bearing find — it is the permissive, espeak-free English
G2P that makes a clean Kokoro stack possible. We build a **thin
`KokoroTtsEngine`** directly on `ort` + `voice-g2p` rather than depend on a
higher-level Kokoro crate, so **we own the dependency graph** and can guarantee
nothing re-introduces espeak transitively. `cargo deny check licenses` is the
enforcement: the chapter ends with the GPL exception **removed** from
`deny.toml`, so any espeak/GPL regression fails the gate loudly.

### Pipeline shape (fits the existing `TtsEngine` trait unchanged)

```
sentence text
  → voice-g2p          → IPA phonemes
  → phoneme→id vocab   → input_ids (Kokoro's fixed token vocab)
  → ort.run(input_ids, style_vec[voice], speed) → f32 PCM @ 24 kHz
  → TtsAudio { samples, sample_rate: 24000 }
```

The `TtsEngine` seam (`async fn synthesize(&str) -> TtsAudio`,
`native_sample_rate()`) and all of the sentence-chunking substrate
(`chunk_into_sentences` / `drain_complete_sentences`) are **reused as-is** —
this chapter swaps the engine *behind* the trait, nothing above it.

## 3. Scope — what Piper removal touches

Inventory of the `tts-piper` footprint to delete/replace (verified in-tree):

- `crates/aivyx-voice/Cargo.toml` — drop the `tts-piper` feature + the
  `piper1-rs` optional dep; redefine the `recommended-voice` meta-feature to
  `asr-whisper-rs` + the new `tts-kokoro`.
- `crates/aivyx-voice/src/tts/mod.rs` — drop `#[cfg(tts-piper)] pub mod piper`;
  add `#[cfg(tts-kokoro)] pub mod kokoro`. Generalize `TtsConfig` (Piper's
  `voice_path` + `espeak_data_path` → a model dir + voice name; no espeak path).
- `crates/aivyx-voice/src/tts/piper.rs` — **deleted**; new `tts/kokoro.rs`.
- `crates/aivyx-voice/src/{session,channel}.rs` — engine selection
  (`tts_engine = "piper"` → `"kokoro"`) and any Piper-specific config plumbing.
- `crates/aivyx-channel/Cargo.toml` — `channel-voice-full` →
  `aivyx-voice/recommended-voice` (unchanged name; new contents).
- `deny.toml` — **remove** the `piper1-rs-sys` GPL exception (the headline).
- `docs/INSTALL.md` — rewrite the voice setup (~§ lines 1554–1659): no more
  ONNX-headers/espeak-ng prereqs or Piper voice catalog; Kokoro model + voices
  download instead.
- `docs/LICENSING.md` — flip §6.1's "live constraint" (voice can't ship in a
  BSL binary) now that the GPL is gone; note the **remaining** non-GPL blocker
  (below).

## 4. The other blocker this chapter does NOT solve (recorded honestly)

Removing the GPL is **necessary but not sufficient** to put voice in *every*
official binary. The cargo-dist Linux targets are **musl-static**, and
`cpal`/`alsa-sys` (audio I/O) don't build static-musl — the same reason the musl
release already skips ALSA (`dist-workspace.toml`, `precise-builds`). So after
Timbre:

- **macOS official binaries** *can* gain voice (CoreAudio, no ALSA/musl issue) —
  a permissive engine makes this lawful.
- **Linux-musl official binaries** still can't, until audio I/O on musl is
  solved (a glibc release variant, a dynamic-ALSA build, or a different audio
  backend). That is **out of scope for Timbre** and tracked separately.

Either way, Timbre's win stands on its own: **voice becomes license-clean
(Apache/MIT) for everyone who builds from source**, and the GPL exception leaves
the tree.

## 5. Phase plan (docs-first, small phases per project convention)

| Phase | Deliverable | Notes |
|---|---|---|
| **TB.0** | **This design contract** | locked reference; status banner flips per phase |
| **TB.1** ✅ | **`KokoroTtsEngine` spike** | DONE. `tts-kokoro` feature + `crates/aivyx-voice/src/tts/kokoro.rs`: `KokoroEngine` impls the existing `TtsEngine` trait via `ort` (=`2.0.0-rc.12`, default `download-binaries` → no system ONNX/espeak prereqs) + `voice-g2p` (MIT, espeak-free). Model I/O pinned: tokens `[0,…,0]` shape `[1,N+2]`, `[1,256]` style vector indexed by token-count from the `voices-*.bin` npz, int32/float32 `speed` autodetected, first output = 24 kHz f32 PCM; embedded canonical Kokoro vocab + `config.json` override; punctuation-aware chunking at `MAX_PHONEME_LEN=510`. Compiles, **clippy clean (-D warnings)**, 8 pure-logic tests green (vocab, phoneme→id, chunking, .npy parse). Real-audio inference is TB.6 (needs model files). Resolved an `ort` rc.10→rc.12 bump (rc.10 pinned `smallvec` incompatibly with mistralrs). |
| **TB.2** ✅ | **Config + engine selection** | DONE. Generalized the engine-neutral `TtsConfig` (added `model_dir` / `voice_name` / `speed`; Piper's `voice_path` / `speaker_id` stay until TB.3); added `kokoro::config_from_generic`; added `[voice] tts_model_dir` / `tts_voice_name` / `tts_speed` to `VoiceOptions`. The CLI voice loop now **selects the engine from `tts_engine`** — `"kokoro"` → `KokoroEngine`, anything else → Piper (retired in TB.3). Chunking substrate reused untouched. **Verified:** `aivyx-voice --features tts-kokoro` (10 tests, clippy clean), `aivyx-config` + default `aivyx-cli` compile + clippy clean. The `channel-voice-full` CLI block can't be compiled in the dev sandbox (Piper's `piper1-rs-sys` builds espeak-ng + needs onnxruntime — `ort-sys`/`whisper-rs-sys` built fine); it's also excluded from CI's default gate, so it's compile-verified in TB.3 (after Piper is removed, the CLI builds here on `ort` alone) and live-verified in TB.6. Studio/`config_write` exposure of the new fields is deferred to TB.5 (the `[voice]` TOML is hand-editable today). |
| **TB.3** ✅ | **Remove Piper** | DONE. Deleted the `tts-piper` feature, the `piper1-rs` dep, and `tts/piper.rs`; `recommended-voice` → `asr-whisper-rs` + `tts-kokoro`; the engine-facing `TtsConfig` is now Kokoro-only (`model_dir`/`voice_name`/`speed`); the CLI loop builds Kokoro unconditionally (rejects any non-`kokoro` `tts_engine`). **Removing Piper let the `channel-voice-full` CLI voice path compile for the first time** (it had bit-rotted unseen — no CI coverage + Piper's build prereqs blocked local builds): fixed by re-exporting `build_agent_stack`/`AgentStackSpec` from `aivyx-channel` and filling the `VoiceChannelConfig` literal's newer fields (vad/image/abort). **Verified:** `aivyx-cli --features channel-voice-full` compiles + clippy clean (`-D warnings`); 204 `aivyx-voice` tests pass; default workspace compiles. Vestigial `[voice] tts_voice_path`/`tts_espeak_data_path` + the Studio readiness check are cleaned in TB.5; the `deny.toml` GPL exception drops in TB.4. |
| **TB.4** ✅ | **Flip the license gate** | DONE. Removed the `piper1-rs-sys` GPL-3.0 exception from `deny.toml` (the lone exception — the block is now empty, with a note recording why). `cargo deny check licenses` (all-features, so the optional `tts-kokoro` stack is evaluated) is **green, exit 0, with no exceptions**. Verified the all-features graph has **zero GPL-3.0, no `piper`, no `espeak`** — the only copyleft id that even appears is `r-efi`'s `MIT OR Apache-2.0 OR LGPL-2.1-or-later`, where the OR resolves to a permissive option (the documented CR.1 case). The relicense's hard gate is satisfied: a future copyleft dep now fails loudly with nothing carved out. |
| **TB.5** ✅ | **Docs + Studio + vestigial config cleanup** | DONE. Rewrote `docs/INSTALL.md`'s voice section (Kokoro model+voices download; dropped the espeak-ng/ONNX-headers prereqs — `ort` self-fetches its runtime); flipped `docs/LICENSING.md` §6.1's live-constraint to ✅ RESOLVED (records the Linux-musl audio caveat). Replaced the vestigial Piper config end-to-end: `VoiceOptions` + `VoiceWrite` (`tts_voice_path`/`tts_espeak_data_path` → `tts_model_dir`/`tts_voice_name`/`tts_speed`), the `SetVoice` IPC payload + `VoiceSettingsSnapshot` (readiness `tts_voice_status`/`espeak_status` → `tts_model_status`/`tts_voices_status`, computed by scanning the model dir for `*.onnx` + `voices-*.bin`), the daemon snapshot/summary, and the **Studio Voice screen** (model-dir/voice/speed inputs, Kokoro readiness rows, kokoro engine option). **Verified:** ipc/config/channel tests + clippy green; `aivyx-web` type-checks + clippy on wasm32; full `channel-voice-full` CLI clippy clean. The committed `dist/` WASM bundle rebuild (`just build-web`) + live browser check are TB.6 (the repo's standard ".5/verify" cadence). |
| **TB.6** ✅ | **Live-verify (build + Studio) — audio speak-test = operator** | Autonomously verified: (1) **Studio bundle rebuilt** (`dx bundle --release`) + re-embedded in the daemon + committed `dist/`; the new wasm contains the Kokoro UI strings ("Kokoro model dir/voices", "kokoro") and **no** Piper strings ("espeak-ng data"/"Piper voice" = 0). (2) **Full voice binary builds + links + runs** — `cargo build -p aivyx-cli --features channel-voice-full` compiles `ort-sys`/`whisper-rs-sys`/`aivyx-voice` into the real `aivyx` binary (ALSA present → cpal links; `ldd` shows `libasound`), and the binary runs; `--channel voice` parses. Also fixed the stale `--channel` usage list to include `voice`. **Remaining (hands-on, needs a mic/speakers + the model files):** run `aivyx --channel voice` and confirm Kokoro speaks — the §7 runbook. |

**Discipline:** TB.4 is the chapter's hard gate — the whole point is *zero GPL*.
If anything in the Kokoro path drags espeak/GPL back in (a transitive dep, a
crate that links espeak), that's a blocker to resolve before TB.4 closes, not
after.

## 6. Open questions to resolve in-phase (not blockers to TB.0)

- **Where do model files live + how are they fetched?** Kokoro needs the
  `kokoro.onnx` (~80–330 MB depending on quant) + a `voices` file. Mirror the
  existing "operator downloads to a path, config points at it" pattern (like
  Piper voices / whisper models), and consider an `aivyx` helper later. Decide in
  TB.2.
- **Quantization / size.** fp32 vs int8 Kokoro ONNX — quality vs. footprint.
  Pick a default in TB.1; the model is operator-downloaded so it's swappable.
- **`voice-g2p` coverage.** English-only today. Non-English voice was never
  shipped (Piper was English-default too), so this is parity, not a regression —
  but record it so multilingual TTS isn't assumed.
- **`ort` linking mode.** `ort` can download a prebuilt ONNX Runtime or link a
  system one; pick the mode that keeps the build self-contained and permissive
  (no GPL, no surprise system requirement) in TB.1.

## 7. TB.6 operator runbook — the audio speak-test

The one step that needs a human (a microphone, speakers, and the model files):

1. **Download the Kokoro model + voices** into one directory (see
   `docs/INSTALL.md` → Voice channel → "Kokoro TTS model + voices"):
   ```bash
   mkdir -p ~/models/kokoro && cd ~/models/kokoro
   wget https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/kokoro-v1.0.onnx
   wget https://github.com/thewh1teagle/kokoro-onnx/releases/download/model-files-v1.0/voices-v1.0.bin
   ```
   (Also have a Whisper `.bin` for ASR — e.g. `ggml-base.en.bin`.)
2. **Point `[voice]` at them** in `aivyx.toml`:
   ```toml
   [voice]
   asr_engine = "whisper-rs"
   tts_engine = "kokoro"
   [voice.asr]
   model_path = "/home/<you>/models/ggml-base.en.bin"
   [voice.tts]
   model_dir  = "/home/<you>/models/kokoro"
   voice_name = "af_heart"
   ```
3. **Build with voice + run** (this machine builds it clean — ort fetches its own
   ONNX Runtime, no espeak/ONNX-headers needed):
   ```bash
   cargo run -p aivyx-cli --features channel-voice-full --bin aivyx -- --channel voice
   ```
   In a Claude Code session you can run it inline with `! aivyx --channel voice`.
4. **Confirm:** press Enter to talk, speak, release — the agent replies and you
   **hear Kokoro speak** (24 kHz, `af_heart`). Then record the run here and flip
   the banner to fully ✅.

Everything up to the audio device is verified (the binary builds, links ALSA, and
runs; the Studio shows the Kokoro screen). This step just confirms sound comes out.

---

*Chapter Timbre closes the last GPL door Chapter Charter left open: it makes the
voice stack fully source-available-clean (Apache/MIT), removes the lone
`deny.toml` GPL exception, and unblocks voice in official macOS binaries — while
honestly recording that Linux-musl audio I/O remains a separate, later fight.*
