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

#[cfg(feature = "tts-kokoro")]
pub mod kokoro;

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

/// Operator-supplied TTS configuration — the engine-neutral
/// superset; each backend's `config_from_generic` reads the
/// fields it needs.
#[derive(Debug, Clone, serde::Deserialize, Default)]
pub struct TtsConfig {
    /// Absolute path to the voice model `.onnx` file.
    /// Piper-only (removed with Piper in Chapter Timbre TB.3).
    #[serde(default)]
    pub voice_path: Option<std::path::PathBuf>,

    /// Optional speaker id for multi-speaker voice models.
    /// Piper-only (removed with Piper in TB.3).
    #[serde(default)]
    pub speaker_id: Option<u32>,

    /// Chapter Timbre — Kokoro model directory (holds the
    /// `.onnx`, `voices-*.bin`, and optional `config.json`).
    #[serde(default)]
    pub model_dir: Option<std::path::PathBuf>,

    /// Chapter Timbre — Kokoro voice name (e.g. `af_heart`).
    /// Defaults to the engine's default voice when omitted.
    #[serde(default)]
    pub voice_name: Option<String>,

    /// Chapter Timbre — Kokoro speaking-rate multiplier
    /// (1.0 = normal). Defaults to 1.0 when omitted.
    #[serde(default)]
    pub speed: Option<f32>,
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
/// Streaming complement to [`chunk_into_sentences`].
///
/// Operates on a mutable buffer: pulls every
/// **complete** sentence (terminated by `.` / `?` /
/// `!` followed by whitespace) into the returned
/// `Vec<String>`, and leaves any **partial trailing
/// fragment** in the buffer so the next call —
/// after more text has been appended — picks up
/// where this one left off.
///
/// Phase 138's streaming-TTS path calls this from
/// inside `VoiceChannel::stream_event` each time the
/// agent emits a text chunk; complete sentences
/// flush to the TTS engine immediately while the
/// in-flight sentence keeps growing in the buffer.
///
/// Key behaviour differences from
/// `chunk_into_sentences`:
///
/// - **EOF is not a sentence terminator.** A
///   sentence is only complete when its terminator
///   is followed by whitespace. Trailing fragments
///   — terminated or not — remain in the buffer.
///   The session driver's post-turn flush is the
///   one place that treats EOF as a terminator (via
///   a final `drain` of whatever's left).
/// - **In-place buffer mutation.** Truncates the
///   buffer in O(n) by shifting the leftover
///   fragment to the front.
///
/// Same boundary semantics as `chunk_into_sentences`:
/// decimals inside numbers (`3.14`) do not split,
/// because the period isn't followed by whitespace.
///
/// **Substrate code** — no async, no IO. Directly
/// unit-testable.
pub fn drain_complete_sentences(buf: &mut String) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut consumed_up_to: usize = 0;
    // Walk grapheme-naïvely via char_indices so we
    // can record byte offsets and slice the
    // remainder cleanly at the end.
    let chars: Vec<(usize, char)> = buf.char_indices().collect();
    let mut i = 0;
    while i < chars.len() {
        let (_, c) = chars[i];
        current.push(c);
        if matches!(c, '.' | '?' | '!') {
            // Look at the next char (if any). We
            // only flush when the next char is
            // whitespace — EOF leaves the fragment
            // in the buffer for the next call.
            let next = chars.get(i + 1).map(|(_, ch)| *ch);
            if let Some(n) = next {
                if n.is_whitespace() {
                    let trimmed = current.trim().to_string();
                    if !trimmed.is_empty() {
                        out.push(trimmed);
                    }
                    current.clear();
                    // Consume the whitespace too.
                    i += 1;
                    consumed_up_to = chars
                        .get(i + 1)
                        .map(|(idx, _)| *idx)
                        .unwrap_or(buf.len());
                    i += 1;
                    continue;
                }
            }
            // No follower (EOF) or non-whitespace
            // follower — keep accumulating. The
            // fragment stays in `current`.
        }
        i += 1;
    }
    // Whatever didn't get flushed lives in the
    // buffer for the next call. Truncate to the
    // last fully-consumed prefix.
    let leftover = buf[consumed_up_to..].to_string();
    buf.clear();
    buf.push_str(&leftover);
    out
}

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

    // ----- drain_complete_sentences (streaming) -------------------------

    #[test]
    fn drain_empty_buffer_yields_nothing() {
        let mut buf = String::new();
        let out = drain_complete_sentences(&mut buf);
        assert!(out.is_empty());
        assert_eq!(buf, "");
    }

    #[test]
    fn drain_partial_fragment_stays_buffered() {
        // No terminator yet — nothing to flush, all
        // stays in the buffer for the next call.
        let mut buf = String::from("Hello there, this is in");
        let out = drain_complete_sentences(&mut buf);
        assert!(out.is_empty());
        assert_eq!(buf, "Hello there, this is in");
    }

    #[test]
    fn drain_complete_then_partial_flushes_complete_only() {
        // First sentence is complete (period +
        // whitespace boundary). Second is mid-word.
        let mut buf = String::from("Sentence one. Sentence tw");
        let out = drain_complete_sentences(&mut buf);
        assert_eq!(out, vec!["Sentence one.".to_string()]);
        assert_eq!(buf, "Sentence tw");
    }

    #[test]
    fn drain_eof_terminator_is_not_flushed() {
        // Period at EOF with no follower — the
        // streaming form keeps it in the buffer
        // (operator may append more text). Contrast
        // with chunk_into_sentences which would flush.
        let mut buf = String::from("Hello world.");
        let out = drain_complete_sentences(&mut buf);
        assert!(out.is_empty(), "EOF terminator must not flush");
        assert_eq!(buf, "Hello world.");
    }

    #[test]
    fn drain_multiple_complete_sentences_in_one_call() {
        let mut buf = String::from("First. Second! Third? Fourth ");
        let out = drain_complete_sentences(&mut buf);
        assert_eq!(
            out,
            vec![
                "First.".to_string(),
                "Second!".to_string(),
                "Third?".to_string(),
            ],
        );
        // "Fourth " trails with no terminator yet.
        assert_eq!(buf, "Fourth ");
    }

    #[test]
    fn drain_decimal_in_number_does_not_break() {
        let mut buf = String::from("Pi is 3.14 and that's it. ");
        let out = drain_complete_sentences(&mut buf);
        assert_eq!(out, vec!["Pi is 3.14 and that's it.".to_string()]);
        assert_eq!(buf, "");
    }

    #[test]
    fn drain_incremental_streaming_simulation() {
        // The shape of how stream_event will use this:
        // append chunk, drain, repeat.
        let mut buf = String::new();
        let mut all: Vec<String> = Vec::new();

        // Chunk 1: partial first sentence.
        buf.push_str("Hello there");
        all.extend(drain_complete_sentences(&mut buf));
        assert_eq!(buf, "Hello there");
        assert!(all.is_empty());

        // Chunk 2: completes first sentence + starts
        // second.
        buf.push_str(". How are ");
        all.extend(drain_complete_sentences(&mut buf));
        // "Hello there." flushes. "How are " trails.
        assert_eq!(all, vec!["Hello there.".to_string()]);
        assert_eq!(buf, "How are ");

        // Chunk 3: completes the second + a full
        // third in one go.
        buf.push_str("you today? I'm great! Now ");
        all.extend(drain_complete_sentences(&mut buf));
        assert_eq!(
            all,
            vec![
                "Hello there.".to_string(),
                "How are you today?".to_string(),
                "I'm great!".to_string(),
            ],
        );
        assert_eq!(buf, "Now ");

        // Chunk 4: only whitespace, no new sentence.
        // Nothing flushes; buffer keeps growing.
        buf.push_str("what");
        all.extend(drain_complete_sentences(&mut buf));
        assert_eq!(all.len(), 3, "no new flush");
        assert_eq!(buf, "Now what");
    }

    #[test]
    fn drain_newline_boundary_treated_as_whitespace() {
        let mut buf = String::from("Line one.\nLine two ");
        let out = drain_complete_sentences(&mut buf);
        assert_eq!(out, vec!["Line one.".to_string()]);
        assert_eq!(buf, "Line two ");
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
