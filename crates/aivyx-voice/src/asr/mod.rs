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
}
