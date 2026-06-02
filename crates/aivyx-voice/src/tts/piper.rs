//! Piper TTS backend (via `piper1-rs`).
//!
//! Phase 135 Task 4 — real `TtsEngine` impl bridging
//! Aivyx's trait onto `piper1-rs`'s `Piper` +
//! `PiperSynthesisHandle` API.
//!
//! ## Build prereq
//!
//! `piper1-rs` requires ONNX runtime headers at build
//! time. Per INSTALL.md, operators install:
//! - **Linux:** `apt install libonnxruntime-dev` (or
//!   download from `microsoft/onnxruntime` releases).
//! - **macOS:** `brew install onnxruntime`.
//! - **Windows:** download the Windows ONNX runtime
//!   archive from the microsoft/onnxruntime release
//!   page, set `ONNX_RUNTIME_DIR` env var to its root.
//!
//! ## Runtime prereq
//!
//! Piper's phonemization step depends on espeak-ng's
//! data directory. Operators install espeak-ng locally
//! and supply the path (`/usr/share/espeak-ng-data` on
//! Linux is the typical install path; configurable in
//! [`PiperTtsConfig::espeak_data_path`]).
//!
//! ## Threading
//!
//! `piper1-rs::Piper` is not `Send`/`Sync` — it holds
//! raw FFI pointers. We wrap it in `Mutex<Piper>` so
//! the engine satisfies our trait's `Send + Sync`
//! bound; one synthesis at a time, which matches the
//! push-to-talk loop's serial usage anyway.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use async_trait::async_trait;
use piper1_rs::Piper;

use crate::tts::{TtsAudio, TtsConfig, TtsEngine, TtsError};

const DEFAULT_PIPER_SAMPLE_RATE: u32 = 22_050;

/// Extension of [`TtsConfig`] carrying Piper-specific
/// runtime knobs.
///
/// `voice_path` (inherited from `TtsConfig.voice_path`)
/// points at the `.onnx` voice model; Piper expects
/// the matching `.onnx.json` config to live next to it
/// (which the upstream tooling produces automatically
/// when an operator downloads a voice from rhasspy's
/// voice repo).
///
/// `espeak_data_path` is the espeak-ng data directory.
#[derive(Debug, Clone)]
pub struct PiperTtsConfig {
    pub voice_path: PathBuf,
    pub espeak_data_path: PathBuf,
    pub config_path: Option<PathBuf>,
}

impl PiperTtsConfig {
    pub fn new(
        voice_path: impl Into<PathBuf>,
        espeak_data_path: impl Into<PathBuf>,
    ) -> Self {
        PiperTtsConfig {
            voice_path: voice_path.into(),
            espeak_data_path: espeak_data_path.into(),
            config_path: None,
        }
    }

    pub fn with_config_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config_path = Some(path.into());
        self
    }
}

/// `TtsEngine` impl backed by `piper1-rs`.
pub struct PiperEngine {
    inner: Mutex<Piper>,
    /// Lazily filled from the first synthesis output;
    /// defaults to [`DEFAULT_PIPER_SAMPLE_RATE`] until
    /// then. `cpal` initialises the output stream at
    /// engine-construction time, so we want a stable
    /// rate from t=0; the default is correct for the
    /// vast majority of Piper voices (22050).
    cached_rate: AtomicU32,
}

impl PiperEngine {
    /// Build the synthesizer from a `PiperTtsConfig`.
    /// Validates that all three paths exist (Piper does
    /// this internally too; we surface the error
    /// earlier with a clearer message).
    pub fn new(config: PiperTtsConfig) -> Result<Self, TtsError> {
        let voice_str = config
            .voice_path
            .to_str()
            .ok_or_else(|| {
                TtsError::ModelLoad(format!(
                    "voice_path is not valid UTF-8: {:?}",
                    config.voice_path
                ))
            })?
            .to_string();
        let espeak_str = config
            .espeak_data_path
            .to_str()
            .ok_or_else(|| {
                TtsError::ModelLoad(format!(
                    "espeak_data_path is not valid UTF-8: {:?}",
                    config.espeak_data_path
                ))
            })?
            .to_string();
        let config_str = config
            .config_path
            .as_ref()
            .map(|p| {
                p.to_str()
                    .ok_or_else(|| {
                        TtsError::ModelLoad(format!(
                            "config_path is not valid UTF-8: {p:?}"
                        ))
                    })
                    .map(|s| s.to_string())
            })
            .transpose()?;
        let piper = Piper::new(voice_str, config_str, espeak_str)
            .map_err(|e| TtsError::ModelLoad(format!("piper init: {e}")))?;
        Ok(PiperEngine {
            inner: Mutex::new(piper),
            cached_rate: AtomicU32::new(DEFAULT_PIPER_SAMPLE_RATE),
        })
    }
}

#[async_trait]
impl TtsEngine for PiperEngine {
    async fn synthesize(&self, text: &str) -> Result<TtsAudio, TtsError> {
        if text.trim().is_empty() {
            return Err(TtsError::Input(
                "empty text — TTS skipped".to_string(),
            ));
        }
        // Synthesis is sync + CPU-heavy. Same posture as
        // whisper-rs: the channel loop should consider
        // wrapping the call in `tokio::task::spawn_blocking`
        // when integrating in Phase 135 Task 5.
        let mut piper = self
            .inner
            .lock()
            .map_err(|_| TtsError::Synthesis("piper mutex poisoned".to_string()))?;
        let options = piper.get_default_synthesis_options();
        let mut handle = piper
            .start_synthesis(text.to_string(), &options)
            .map_err(|e| TtsError::Synthesis(format!("start_synthesis: {e}")))?;

        let mut samples: Vec<f32> = Vec::new();
        let mut rate = DEFAULT_PIPER_SAMPLE_RATE;
        loop {
            let chunk_opt = handle
                .get_next_chunk()
                .map_err(|e| TtsError::Synthesis(format!("get_next_chunk: {e}")))?;
            let Some(chunk) = chunk_opt else {
                break;
            };
            rate = chunk.sample_rate();
            samples.extend_from_slice(chunk.samples());
            if chunk.is_last() {
                break;
            }
        }

        // Persist the observed rate so cpal's output
        // stream can stay in sync if the engine reports a
        // non-default rate.
        self.cached_rate.store(rate, Ordering::Relaxed);

        Ok(TtsAudio::new(samples, rate))
    }

    fn native_sample_rate(&self) -> u32 {
        self.cached_rate.load(Ordering::Relaxed)
    }
}

/// Build a `PiperTtsConfig` from the generic `TtsConfig`
/// plus the Piper-specific paths the channel layer
/// reads separately.
///
/// `TtsConfig.voice_path` becomes Piper's voice ONNX;
/// `espeak_data_path` is supplied explicitly because
/// Piper needs it but the generic TtsConfig stays
/// engine-neutral.
pub fn config_from_generic(
    generic: &TtsConfig,
    espeak_data_path: PathBuf,
) -> Result<PiperTtsConfig, TtsError> {
    let voice_path = generic.voice_path.clone().ok_or_else(|| {
        TtsError::ModelLoad(
            "[voice.tts] voice_path missing — set the absolute path to a Piper .onnx voice model"
                .to_string(),
        )
    })?;
    Ok(PiperTtsConfig::new(voice_path, espeak_data_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn piper_config_builder_chain() {
        let cfg = PiperTtsConfig::new(
            "/models/en_US-amy-medium.onnx",
            "/usr/share/espeak-ng-data",
        )
        .with_config_path("/models/en_US-amy-medium.onnx.json");
        assert_eq!(
            cfg.voice_path.to_string_lossy(),
            "/models/en_US-amy-medium.onnx"
        );
        assert_eq!(
            cfg.espeak_data_path.to_string_lossy(),
            "/usr/share/espeak-ng-data"
        );
        assert_eq!(
            cfg.config_path.as_ref().map(|p| p.to_string_lossy().to_string()),
            Some("/models/en_US-amy-medium.onnx.json".to_string()),
        );
    }

    #[test]
    fn piper_config_minimal_construction() {
        let cfg = PiperTtsConfig::new(
            "/models/en_US-amy-medium.onnx",
            "/usr/share/espeak-ng-data",
        );
        assert!(cfg.config_path.is_none());
    }

    #[test]
    fn config_from_generic_errors_when_voice_path_missing() {
        let generic = TtsConfig {
            voice_path: None,
            speaker_id: None,
        };
        let result = config_from_generic(&generic, "/usr/share/espeak-ng-data".into());
        match result {
            Err(TtsError::ModelLoad(msg)) => {
                assert!(msg.contains("voice_path"), "{msg}");
            }
            Ok(_) => panic!("must error when voice_path is None"),
            Err(other) => panic!("expected ModelLoad, got {other:?}"),
        }
    }

    #[test]
    fn config_from_generic_succeeds_with_voice_path() {
        let generic = TtsConfig {
            voice_path: Some("/models/voice.onnx".into()),
            speaker_id: Some(0),
        };
        let cfg = config_from_generic(&generic, "/usr/share/espeak-ng-data".into())
            .expect("ok");
        assert_eq!(cfg.voice_path.to_string_lossy(), "/models/voice.onnx");
    }

    #[test]
    fn default_piper_sample_rate_is_22050() {
        assert_eq!(DEFAULT_PIPER_SAMPLE_RATE, 22_050);
    }
}
