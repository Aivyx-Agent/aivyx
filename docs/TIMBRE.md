# Permissive Voice / TTS (Chapter Timbre)

> **Status:** 🧭 **design contract — TB.0 ✅ + TB.1 ✅ + TB.2 ✅.** This is the locked reference for
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
| **TB.3** | **Remove Piper** | delete `tts-piper` feature, `piper1-rs` dep, `tts/piper.rs`; redefine `recommended-voice` → whisper-rs + kokoro; update `aivyx-channel` features. |
| **TB.4** | **Flip the license gate** | remove the `piper1-rs-sys` exception from `deny.toml`; `cargo deny check licenses` (all-features) green with **zero GPL**. The hard proof. |
| **TB.5** | **Docs** | rewrite `docs/INSTALL.md` voice setup (Kokoro download, no espeak/ONNX-headers); update `docs/LICENSING.md` §6.1 live-constraint + the Studio Voice screen's readiness check ([[chapter-voice]]). |
| **TB.6** | **Live-verify** | build with `channel-voice-full`, run `aivyx --channel voice`, confirm Kokoro speaks end-to-end; record the run. |

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

---

*Chapter Timbre closes the last GPL door Chapter Charter left open: it makes the
voice stack fully source-available-clean (Apache/MIT), removes the lone
`deny.toml` GPL exception, and unblocks voice in official macOS binaries — while
honestly recording that Linux-musl audio I/O remains a separate, later fight.*
