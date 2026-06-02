//! Text-to-Speech (TTS) — the agent side of the voice
//! loop.
//!
//! The [`TtsEngine`] trait is the seam Aivyx's
//! `VoiceChannel` consumes. One implementation ships in
//! Phase 135 (gated by `tts-piper`):
//!
//! - `tts-piper` (default) → [`piper`] module.
//!
//! Higher-quality alternatives (Kokoro, F5-TTS,
//! Chatterbox) are Phase 136+ candidates after
//! operators validate the Piper baseline.

use async_trait::async_trait;
use thiserror::Error;

#[cfg(feature = "tts-piper")]
pub mod piper;

/// Errors a TTS backend can surface to the channel
/// loop.
#[derive(Debug, Error)]
pub enum TtsError {
    /// Failed to load the voice model from disk.
    #[error("TTS voice model load failed: {0}")]
    ModelLoad(String),

    /// Inference failure.
    #[error("TTS synthesis failed: {0}")]
    Synthesis(String),

    /// The text input was malformed (typically empty
    /// after sentence trimming). Channel loop treats
    /// this as a no-op (skip TTS for the chunk).
    #[error("TTS input rejected: {0}")]
    Input(String),
}

/// A synthesized audio buffer ready for the speaker
/// queue. Sample rate is whatever the engine declared
/// at construction time; the channel loop reads
/// [`Self::sample_rate`] and passes it to `cpal` /
/// `rodio` so playback fidelity matches the synthesis.
#[derive(Debug, Clone)]
pub struct TtsAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl TtsAudio {
    pub fn new(samples: Vec<f32>, sample_rate: u32) -> Self {
        TtsAudio { samples, sample_rate }
    }

    /// Returns true when the synthesizer produced no
    /// samples. Channel loop skips speaker dispatch
    /// for empty buffers.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Duration of this buffer at its declared sample
    /// rate, in seconds.
    pub fn duration_secs(&self) -> f32 {
        if self.sample_rate == 0 {
            return 0.0;
        }
        self.samples.len() as f32 / self.sample_rate as f32
    }
}

/// The shape every TTS backend implements.
#[async_trait]
pub trait TtsEngine: Send + Sync {
    /// Synthesize a text fragment into PCM-f32 audio.
    /// The channel loop chunks the agent's response
    /// at sentence boundaries before calling this — the
    /// engine sees one sentence at a time, which keeps
    /// inference latency bounded and lets the operator
    /// hear the first sentence before the last is
    /// synthesized.
    async fn synthesize(&self, text: &str) -> Result<TtsAudio, TtsError>;

    /// The native sample rate of the configured voice
    /// model. Channel loop reads this once at
    /// construction so the speaker stream is
    /// initialized with the right rate.
    fn native_sample_rate(&self) -> u32;
}

/// Operator-supplied TTS configuration.
#[derive(Debug, Clone, serde::Deserialize, Default)]
pub struct TtsConfig {
    /// Absolute path to the voice model `.onnx` file.
    /// Required when the operator enables Piper.
    #[serde(default)]
    pub voice_path: Option<std::path::PathBuf>,

    /// Optional speaker id for multi-speaker voice
    /// models. Defaults to 0 when omitted.
    #[serde(default)]
    pub speaker_id: Option<u32>,
}

// ---------------------------------------------------------------------------
// Text chunking — pure substrate, shared across every TTS backend.
// ---------------------------------------------------------------------------

/// Split a buffered LLM response into TTS-sized
/// chunks at sentence boundaries.
///
/// The channel loop streams the LLM's response into a
/// `String` buffer; when the LLM emits `.`, `?`, or
/// `!` followed by a space or newline, we flush a
/// chunk to the TTS engine. This keeps perceived
/// latency low — the operator hears the first
/// sentence while the LLM is still streaming the
/// rest (post-Phase-135; Phase 135 itself buffers the
/// whole response, then chunks).
///
/// **Substrate code** — no async, no IO. Directly
/// unit-testable. Used by the channel loop, but
/// nothing engine-specific lives here.
pub fn chunk_into_sentences(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        current.push(c);
        if matches!(c, '.' | '?' | '!') {
            // Peek the next char — only break on a
            // boundary character (space, newline, or
            // end-of-string). Otherwise the period
            // belongs to an abbreviation, decimal, or
            // URL.
            match chars.peek() {
                Some(next) if next.is_whitespace() => {
                    let trimmed = current.trim().to_string();
                    if !trimmed.is_empty() {
                        out.push(trimmed);
                    }
                    current.clear();
                    // Consume the whitespace so it
                    // doesn't leak into the next
                    // chunk's leading position.
                    let _ = chars.next();
                }
                None => {
                    let trimmed = current.trim().to_string();
                    if !trimmed.is_empty() {
                        out.push(trimmed);
                    }
                    current.clear();
                }
                _ => {
                    // Non-boundary follower — keep
                    // accumulating.
                }
            }
        }
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        out.push(trimmed);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tts_audio_empty_check() {
        let a = TtsAudio::new(Vec::new(), 22050);
        assert!(a.is_empty());
        let b = TtsAudio::new(vec![0.0; 100], 22050);
        assert!(!b.is_empty());
    }

    #[test]
    fn tts_audio_duration_seconds() {
        let a = TtsAudio::new(vec![0.0; 22050], 22050);
        assert!((a.duration_secs() - 1.0).abs() < f32::EPSILON);
        let b = TtsAudio::new(vec![0.0; 11025], 22050);
        assert!((b.duration_secs() - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn tts_audio_duration_zero_rate_safe() {
        let a = TtsAudio::new(vec![0.0; 1000], 0);
        assert_eq!(a.duration_secs(), 0.0);
    }

    #[test]
    fn tts_config_deserializes_from_toml() {
        let toml = r#"
voice_path = "/models/en_US-amy-medium.onnx"
speaker_id = 0
"#;
        let cfg: TtsConfig = toml::from_str(toml).expect("parse");
        assert_eq!(
            cfg.voice_path.as_deref(),
            Some(std::path::Path::new("/models/en_US-amy-medium.onnx"))
        );
        assert_eq!(cfg.speaker_id, Some(0));
    }

    // ----- chunk_into_sentences ------------------------------------------

    #[test]
    fn chunk_empty_text_yields_no_chunks() {
        assert!(chunk_into_sentences("").is_empty());
    }

    #[test]
    fn chunk_single_sentence_no_trailing_punct() {
        let out = chunk_into_sentences("hello world");
        assert_eq!(out, vec!["hello world".to_string()]);
    }

    #[test]
    fn chunk_two_sentences_split_at_period_space() {
        let out = chunk_into_sentences("First sentence. Second sentence.");
        assert_eq!(
            out,
            vec!["First sentence.".to_string(), "Second sentence.".to_string()],
        );
    }

    #[test]
    fn chunk_question_mark_and_exclamation_break() {
        let out = chunk_into_sentences("How? Like this! Yes.");
        assert_eq!(
            out,
            vec![
                "How?".to_string(),
                "Like this!".to_string(),
                "Yes.".to_string(),
            ],
        );
    }

    #[test]
    fn chunk_decimal_inside_number_does_not_break() {
        // Decimal point not followed by whitespace.
        let out = chunk_into_sentences("Pi is about 3.14 then.");
        assert_eq!(out, vec!["Pi is about 3.14 then.".to_string()]);
    }

    #[test]
    fn chunk_newline_after_punct_counts_as_boundary() {
        let out = chunk_into_sentences("Line one.\nLine two.");
        assert_eq!(
            out,
            vec!["Line one.".to_string(), "Line two.".to_string()],
        );
    }

    #[test]
    fn chunk_trims_leading_and_trailing_whitespace_per_chunk() {
        let out = chunk_into_sentences("  Spaced. Out. ");
        assert_eq!(
            out,
            vec!["Spaced.".to_string(), "Out.".to_string()],
        );
    }

    #[test]
    fn chunk_empty_after_trim_is_dropped() {
        // Pathological input — repeated punctuation
        // with no content. The leading "..." has no
        // boundary character before EOS within the
        // accumulator so it surfaces; we then drop
        // anything that trims to empty.
        let out = chunk_into_sentences("...");
        assert_eq!(out, vec!["...".to_string()]);
    }
}
