//! Automatic Speech Recognition (ASR) — the operator
//! side of the voice loop.
//!
//! The [`AsrEngine`] trait is the seam Aivyx's
//! `VoiceChannel` consumes. Two implementations ship
//! in Phase 135, gated by Cargo features:
//!
//! - `asr-whisper-rs` (default) → [`whisper_rs`] module.
//! - `asr-whisper-cpp-plus` → [`whisper_cpp_plus`] module.
//!
//! Concrete implementations land in Task 3. This
//! module ships the trait + config shape so the
//! channel skeleton in Task 5 can compile against it.

use async_trait::async_trait;
use thiserror::Error;

#[cfg(feature = "asr-whisper-rs")]
pub mod whisper_rs;

#[cfg(feature = "asr-whisper-cpp-plus")]
pub mod whisper_cpp_plus;

/// Errors a concrete ASR engine can surface to the
/// channel loop.
#[derive(Debug, Error)]
pub enum AsrError {
    /// Failed to load the model file from disk (missing
    /// path, IO error, unsupported format).
    #[error("ASR model load failed: {0}")]
    ModelLoad(String),

    /// The engine ran but produced no transcription. Not
    /// strictly an error — the channel loop translates
    /// this into "no input detected, prompt operator
    /// again."
    #[error("ASR produced empty transcription")]
    Empty,

    /// Inference failure (engine-specific).
    #[error("ASR inference failed: {0}")]
    Inference(String),

    /// Audio input was malformed — sample rate
    /// mismatch, wrong channel count, etc.
    #[error("ASR input audio rejected: {0}")]
    Audio(String),
}

/// The shape every ASR backend implements.
///
/// Concrete implementations own their model handle
/// internally (loaded once at construction time and
/// reused across calls). `transcribe` consumes a
/// fully-buffered audio sample as PCM f32 in mono at
/// 16 kHz — the canonical Whisper input format. The
/// channel loop is responsible for capturing,
/// resampling, and channel-downmixing before calling
/// the engine.
#[async_trait]
pub trait AsrEngine: Send + Sync {
    /// Transcribe a PCM-f32 mono 16 kHz audio buffer
    /// to text. Returns [`AsrError::Empty`] if the
    /// engine ran but produced no speech (silence,
    /// non-speech audio, etc.); the channel loop
    /// translates this into an operator prompt rather
    /// than a turn dispatch.
    async fn transcribe(&self, samples: &[f32]) -> Result<String, AsrError>;
}

/// Operator-supplied ASR configuration. Carries the
/// model path plus optional tuning knobs. Each backend
/// reads the fields it needs and ignores the rest.
#[derive(Debug, Clone, serde::Deserialize, Default)]
pub struct AsrConfig {
    /// Absolute path to the Whisper model `.bin` file.
    /// Required when the operator enables a Whisper-
    /// family backend.
    #[serde(default)]
    pub model_path: Option<std::path::PathBuf>,

    /// Language code (`"en"`, `"es"`, etc.) or `"auto"`
    /// for automatic detection. Defaults to `"en"` at
    /// engine-construction time when omitted.
    #[serde(default)]
    pub language: Option<String>,

    /// Beam search width. Higher = more accurate but
    /// slower. Defaults to 5 at engine-construction
    /// time when omitted.
    #[serde(default)]
    pub beam_size: Option<usize>,
}

/// The PCM sample rate Whisper expects. All Whisper-
/// family backends share this; non-Whisper backends
/// (Vosk, Moonshine — Phase 136+) would expose their
/// own constants.
pub const WHISPER_SAMPLE_RATE: u32 = 16_000;

// ---------------------------------------------------------------------------
// Substrate audio-format helpers — always available, no engine-feature gates.
// Moved up from asr/whisper_rs.rs in Phase 136 because audio_in.rs needs
// them and is always-on inside aivyx-voice.
// ---------------------------------------------------------------------------

/// Resample an f32 PCM buffer from `src_rate` to 16 kHz
/// (Whisper's expected rate). Linear interpolation;
/// quick + adequate for speech content. Phase 137+
/// could swap in a higher-fidelity resampler
/// (`rubato`) if the channel loop surfaces quality
/// issues.
///
/// `src_rate == 16_000` is a no-op fast path.
pub fn resample_to_16k(samples: &[f32], src_rate: u32) -> Vec<f32> {
    if src_rate == WHISPER_SAMPLE_RATE || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = WHISPER_SAMPLE_RATE as f64 / src_rate as f64;
    let out_len = ((samples.len() as f64) * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_idx_f = i as f64 / ratio;
        let src_idx = src_idx_f as usize;
        let frac = src_idx_f - src_idx as f64;
        let a = samples[src_idx.min(samples.len() - 1)];
        let b = samples[(src_idx + 1).min(samples.len() - 1)];
        out.push(a + (b - a) * frac as f32);
    }
    out
}

/// Downmix interleaved multi-channel PCM to mono by
/// averaging across channels per frame. `channels`
/// = 1 → no-op fast path; `channels` = 2 → stereo
/// L+R average; higher channel counts → average
/// across all channels.
pub fn downmix_to_mono(samples: &[f32], channels: u16) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }
    let c = channels as usize;
    samples
        .chunks_exact(c)
        .map(|frame| frame.iter().sum::<f32>() / (c as f32))
        .collect()
}

/// Convenience: downmix to mono then resample to
/// 16 kHz. The exact transform audio_in.rs applies
/// before handing samples to the ASR engine; lifted
/// to a single function so the channel loop calls
/// once.
pub fn stereo_to_mono_into_16k(
    samples: &[f32],
    src_rate: u32,
    channels: u16,
) -> Vec<f32> {
    let mono = downmix_to_mono(samples, channels);
    resample_to_16k(&mono, src_rate)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whisper_sample_rate_is_16k() {
        assert_eq!(WHISPER_SAMPLE_RATE, 16_000);
    }

    #[test]
    fn asr_config_defaults_are_all_none() {
        let cfg = AsrConfig::default();
        assert!(cfg.model_path.is_none());
        assert!(cfg.language.is_none());
        assert!(cfg.beam_size.is_none());
    }

    #[test]
    fn asr_config_deserializes_from_toml() {
        let toml = r#"
model_path = "/models/ggml-base.en.bin"
language = "en"
beam_size = 5
"#;
        let cfg: AsrConfig = toml::from_str(toml).expect("parse");
        assert_eq!(
            cfg.model_path.as_deref(),
            Some(std::path::Path::new("/models/ggml-base.en.bin"))
        );
        assert_eq!(cfg.language.as_deref(), Some("en"));
        assert_eq!(cfg.beam_size, Some(5));
    }

    #[test]
    fn asr_error_messages_carry_context() {
        let e = AsrError::ModelLoad("file not found".into());
        let s = e.to_string();
        assert!(s.contains("ASR model load"));
        assert!(s.contains("file not found"));
    }

    #[test]
    fn empty_error_is_distinct_variant() {
        let e = AsrError::Empty;
        assert!(e.to_string().contains("empty"));
        // Ensure callers can pattern-match against this
        // variant cleanly — the channel loop branches on
        // it for the "prompt operator again" UX.
        assert!(matches!(e, AsrError::Empty));
    }

    // --- Substrate audio-format helpers ---------------

    #[test]
    fn resample_no_op_when_already_16k() {
        let samples = vec![0.1, 0.2, 0.3, 0.4];
        assert_eq!(resample_to_16k(&samples, 16_000), samples);
    }

    #[test]
    fn resample_empty_input_returns_empty() {
        assert!(resample_to_16k(&[], 48_000).is_empty());
    }

    #[test]
    fn resample_48k_to_16k_reduces_length_by_three() {
        let samples = vec![0.5f32; 300];
        let out = resample_to_16k(&samples, 48_000);
        assert!((99..=101).contains(&out.len()), "got {}", out.len());
        for s in &out {
            assert!((s - 0.5).abs() < 1e-6);
        }
    }

    #[test]
    fn resample_16k_to_8k_doubles_length() {
        // 8k source → 16k target → 2x output length.
        let samples = vec![0.1f32; 200];
        let out = resample_to_16k(&samples, 8_000);
        assert!((399..=401).contains(&out.len()), "got {}", out.len());
    }

    #[test]
    fn downmix_mono_input_is_noop() {
        let mono = vec![0.5, -0.5, 0.25];
        assert_eq!(downmix_to_mono(&mono, 1), mono);
    }

    #[test]
    fn downmix_stereo_averages_interleaved_frames() {
        // L/R interleaved: (1+3)/2=2, (2+4)/2=3, (0+6)/2=3.
        let stereo = vec![1.0, 3.0, 2.0, 4.0, 0.0, 6.0];
        assert_eq!(downmix_to_mono(&stereo, 2), vec![2.0, 3.0, 3.0]);
    }

    #[test]
    fn downmix_5_1_averages_across_six_channels() {
        // Two frames at 6 channels each. Frame 1: all
        // 1.0 → average 1.0. Frame 2: 0,1,2,3,4,5 →
        // average 2.5.
        let surround = vec![
            1.0, 1.0, 1.0, 1.0, 1.0, 1.0, // frame 1
            0.0, 1.0, 2.0, 3.0, 4.0, 5.0, // frame 2
        ];
        assert_eq!(downmix_to_mono(&surround, 6), vec![1.0, 2.5]);
    }

    #[test]
    fn downmix_drops_short_trailing_frame() {
        // chunks_exact semantic.
        let stereo = vec![1.0, 3.0, 5.0]; // 1.5 frames
        assert_eq!(downmix_to_mono(&stereo, 2), vec![2.0]);
    }

    #[test]
    fn stereo_to_mono_into_16k_composes_downmix_and_resample() {
        // 8k stereo, 200 samples = 100 frames mono.
        // Then 8k→16k 2x → ~200 samples.
        let stereo_8k = vec![0.4f32; 200];
        let out = stereo_to_mono_into_16k(&stereo_8k, 8_000, 2);
        // 100 frames after downmix, doubled to ~200 after resample.
        assert!(
            (198..=202).contains(&out.len()),
            "got {} samples (expected ~200)",
            out.len(),
        );
    }

    #[test]
    fn stereo_to_mono_into_16k_native_format_is_passthrough() {
        // 16k mono input — no conversion needed.
        let mono_16k = vec![0.1f32; 1000];
        let out = stereo_to_mono_into_16k(&mono_16k, 16_000, 1);
        assert_eq!(out, mono_16k);
    }
}
