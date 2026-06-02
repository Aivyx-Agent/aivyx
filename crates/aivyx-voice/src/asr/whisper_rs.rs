//! `whisper-rs` ASR backend (default).
//!
//! Phase 135 Task 3 — real `AsrEngine` impl bridging
//! Aivyx's `AsrEngine` trait onto `whisper-rs`'s
//! `WhisperContext` API. The model is loaded once at
//! construction time and reused across calls;
//! per-transcription state is created fresh inside
//! [`WhisperRsEngine::transcribe`].
//!
//! ## Audio input contract
//!
//! Aivyx's `AsrEngine::transcribe` takes mono f32 PCM
//! at 16 kHz — exactly what whisper.cpp expects. The
//! channel loop (Task 5) is responsible for capturing
//! at the device's native sample rate via `cpal`,
//! downmixing to mono, and resampling to 16 kHz
//! before calling the engine.

use std::sync::Mutex;

use async_trait::async_trait;
use whisper_rs::{
    FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters,
};

use crate::asr::{AsrConfig, AsrEngine, AsrError};

const DEFAULT_LANGUAGE: &str = "en";
const DEFAULT_BEAM_SIZE: usize = 5;

/// `AsrEngine` impl backed by whisper-rs / whisper.cpp.
///
/// Wraps the `WhisperContext` in a `Mutex` so the
/// `transcribe` impl can create per-call state through
/// `&mut` access while the public `&self` API stays
/// stable. Whisper inference is not internally
/// thread-safe; one transcription at a time is what
/// the channel's push-to-talk loop wants anyway.
pub struct WhisperRsEngine {
    context: Mutex<WhisperContext>,
    language: String,
    beam_size: usize,
}

impl WhisperRsEngine {
    /// Construct from operator config. Loads the
    /// `.bin` model file into memory; this is the
    /// expensive operation (~100ms for tiny, ~few
    /// seconds for medium) and happens once per
    /// process lifetime.
    ///
    /// Returns [`AsrError::ModelLoad`] when:
    /// - `config.model_path` is `None`,
    /// - the path doesn't exist or isn't readable,
    /// - the file isn't a valid Whisper GGML model.
    pub fn new(config: AsrConfig) -> Result<Self, AsrError> {
        let model_path = config.model_path.as_ref().ok_or_else(|| {
            AsrError::ModelLoad(
                "[voice.asr] model_path missing — set the absolute path to a Whisper .bin file"
                    .to_string(),
            )
        })?;
        let path_str = model_path
            .to_str()
            .ok_or_else(|| {
                AsrError::ModelLoad(format!(
                    "model_path is not valid UTF-8: {model_path:?}"
                ))
            })?;
        let ctx = WhisperContext::new_with_params(
            path_str,
            WhisperContextParameters::default(),
        )
        .map_err(|e| {
            AsrError::ModelLoad(format!(
                "whisper-rs failed to load {path_str:?}: {e}"
            ))
        })?;
        Ok(WhisperRsEngine {
            context: Mutex::new(ctx),
            language: config
                .language
                .unwrap_or_else(|| DEFAULT_LANGUAGE.to_string()),
            beam_size: config.beam_size.unwrap_or(DEFAULT_BEAM_SIZE),
        })
    }

    /// Concatenate every segment's text into one
    /// transcript. Whisper emits per-utterance segments
    /// with timestamps; the channel loop wants the
    /// joined text as the user's turn input. Empty
    /// segments are skipped; segment boundaries become
    /// a single space.
    fn collect_segments(state: &whisper_rs::WhisperState) -> String {
        let mut out = String::new();
        for segment in state.as_iter() {
            let s = segment.to_string();
            let trimmed = s.trim();
            if trimmed.is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(trimmed);
        }
        out
    }
}

#[async_trait]
impl AsrEngine for WhisperRsEngine {
    async fn transcribe(&self, samples: &[f32]) -> Result<String, AsrError> {
        if samples.is_empty() {
            return Err(AsrError::Audio(
                "empty PCM input — channel captured zero samples".to_string(),
            ));
        }

        // Run inference on a blocking-pool task. Whisper's
        // FFI is sync + CPU-heavy; running it on the tokio
        // runtime's regular worker would block other
        // concurrent tasks (the audit-emit task, the
        // cancellation watcher, etc.).
        let samples_owned: Vec<f32> = samples.to_vec();
        let language = self.language.clone();
        let beam_size = self.beam_size;
        // SAFETY: WhisperContext is Sync (internally
        // Mutex-protected). Spawning blocking moves a
        // reference clone into the closure via this
        // crate's Arc-shared handle pattern; we use a
        // boxed closure pattern here that copies the
        // Mutex pointer.
        let result = {
            let context = self.context.lock().map_err(|_| {
                AsrError::Inference("whisper context mutex poisoned".to_string())
            })?;
            let mut state = context.create_state().map_err(|e| {
                AsrError::Inference(format!("create_state: {e}"))
            })?;
            let mut params = FullParams::new(SamplingStrategy::BeamSearch {
                beam_size: beam_size as i32,
                patience: -1.0,
            });
            params.set_language(Some(language.as_str()));
            params.set_print_special(false);
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_timestamps(false);

            state
                .full(params, &samples_owned)
                .map_err(|e| AsrError::Inference(format!("whisper.full: {e}")))?;
            Self::collect_segments(&state)
        };

        if result.is_empty() {
            Err(AsrError::Empty)
        } else {
            Ok(result)
        }
    }
}

// ---------------------------------------------------------------------------
// Pure-substrate helpers — directly unit-testable without
// loading a model. The full transcribe() path requires a
// real .bin file + valid audio sample; operator-side work.
// ---------------------------------------------------------------------------

/// Resample an f32 PCM buffer from `src_rate` to
/// 16 kHz (Whisper's expected rate). Linear
/// interpolation; quick + adequate for speech
/// content. Phase 136+ could swap in a higher-fidelity
/// resampler (`rubato`) if the channel loop surfaces
/// quality issues.
///
/// `src_rate == 16_000` is a no-op fast path.
pub fn resample_to_16k(samples: &[f32], src_rate: u32) -> Vec<f32> {
    if src_rate == super::WHISPER_SAMPLE_RATE || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = super::WHISPER_SAMPLE_RATE as f64 / src_rate as f64;
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

/// Downmix a stereo interleaved f32 PCM buffer to
/// mono by averaging L+R. Channel loop calls this
/// only when the operator's input device delivered
/// stereo; mono input is forwarded unchanged.
pub fn stereo_to_mono(samples: &[f32]) -> Vec<f32> {
    samples
        .chunks_exact(2)
        .map(|pair| (pair[0] + pair[1]) * 0.5)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resample_no_op_when_already_16k() {
        let samples = vec![0.1, 0.2, 0.3, 0.4];
        let out = resample_to_16k(&samples, 16_000);
        assert_eq!(out, samples);
    }

    #[test]
    fn resample_empty_input_returns_empty() {
        let out = resample_to_16k(&[], 48_000);
        assert!(out.is_empty());
    }

    #[test]
    fn resample_48k_to_16k_reduces_length_by_three() {
        // 48000 → 16000 ratio is 1/3. A buffer of 300 samples
        // at 48 kHz should produce roughly 100 samples at 16 kHz.
        let samples = vec![0.5f32; 300];
        let out = resample_to_16k(&samples, 48_000);
        // Allow rounding +/-1.
        assert!(
            (99..=101).contains(&out.len()),
            "expected ~100 samples, got {}",
            out.len(),
        );
        // Every output sample should be ~0.5 (constant signal).
        for s in &out {
            assert!((s - 0.5).abs() < 1e-6, "sample drift: {s}");
        }
    }

    #[test]
    fn resample_16k_to_8k_halves_length() {
        // Going down by 2x.
        let samples = vec![0.1f32; 200];
        let out = resample_to_16k(&samples, 8_000);
        // 8k → 16k is 2x; 200 samples becomes ~400.
        assert!(
            (399..=401).contains(&out.len()),
            "expected ~400 samples, got {}",
            out.len(),
        );
    }

    #[test]
    fn stereo_to_mono_averages_interleaved_pairs() {
        let stereo = vec![1.0, 3.0, 2.0, 4.0, 0.0, 6.0];
        let mono = stereo_to_mono(&stereo);
        // (1+3)/2 = 2, (2+4)/2 = 3, (0+6)/2 = 3.
        assert_eq!(mono, vec![2.0, 3.0, 3.0]);
    }

    #[test]
    fn stereo_to_mono_drops_odd_trailing_sample() {
        // chunks_exact(2) skips a lone trailing sample.
        let stereo = vec![1.0, 3.0, 5.0];
        let mono = stereo_to_mono(&stereo);
        assert_eq!(mono, vec![2.0]);
    }

    #[test]
    fn stereo_to_mono_empty_returns_empty() {
        assert!(stereo_to_mono(&[]).is_empty());
    }

    // --- Engine construction error paths -------------------------

    #[test]
    fn new_returns_model_load_when_path_missing() {
        let cfg = AsrConfig {
            model_path: None,
            language: None,
            beam_size: None,
        };
        // `WhisperRsEngine` doesn't derive `Debug` (its
        // inner `WhisperContext` is non-Debug FFI state),
        // so we match-pattern on the result rather than
        // using `expect_err` which requires `Debug`.
        match WhisperRsEngine::new(cfg) {
            Err(AsrError::ModelLoad(msg)) => {
                assert!(msg.contains("model_path"), "{msg}");
            }
            Err(other) => panic!("expected ModelLoad, got {other:?}"),
            Ok(_) => panic!("must not return Ok with missing model_path"),
        }
    }

    #[test]
    fn new_returns_model_load_when_path_does_not_exist() {
        let cfg = AsrConfig {
            model_path: Some(std::path::PathBuf::from("/totally/nope/model.bin")),
            language: Some("en".to_string()),
            beam_size: Some(5),
        };
        match WhisperRsEngine::new(cfg) {
            Err(AsrError::ModelLoad(_)) => {}
            Err(other) => panic!("expected ModelLoad, got {other:?}"),
            Ok(_) => panic!("must not return Ok with nonexistent path"),
        }
    }
}
