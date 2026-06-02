//! Energy-threshold voice activity detection.
//!
//! Phase 139 — the substrate behind PTT auto-stop.
//! The detector tracks recent audio in fixed-size
//! frames, computes per-frame RMS, and counts how
//! many consecutive samples have been "silent" (RMS
//! below threshold) since the last frame that wasn't.
//!
//! The push-to-talk loop polls
//! [`SilenceDetector::silence_dwell`] every ~100ms
//! while recording; when the dwell exceeds the
//! configured threshold *and* the total recorded
//! duration is at least the minimum-speech window
//! (so we don't auto-stop before the operator has
//! started speaking), the loop stops capture and
//! dispatches the turn.
//!
//! ## Why energy threshold and not ML
//!
//! Phase 139 ships with pure-Rust math: sum-of-
//! squares per frame, threshold check, counter
//! arithmetic. Zero new dependencies, zero ONNX
//! prereq, directly unit-testable. The cost is
//! robustness in noisy environments — a fan, an
//! AC unit, or a busy room will push background
//! RMS above the default threshold and the
//! detector never auto-stops. ML VAD (Silero
//! ONNX) is the Phase 140+ candidate for those
//! cases. The PTT use case is naturally quiet
//! (operator deliberately speaking into a mic in
//! their own room) so the simpler approach fits.

use std::time::Duration;

/// Operator-tunable detector parameters. Phase 139
/// hardcodes sensible defaults; a `[voice.vad]`
/// TOML section is a Phase 140+ candidate if
/// operators hit threshold-tuning needs.
#[derive(Debug, Clone)]
pub struct SilenceDetectorConfig {
    /// Native sample rate of the incoming audio
    /// (Hz). Must match the cpal stream's
    /// `src_rate` — otherwise dwell timing is
    /// wrong.
    pub sample_rate: u32,
    /// Window size for per-frame RMS, in seconds.
    /// Smaller frames = more responsive detection
    /// but noisier RMS; larger frames = stabler
    /// RMS but laggier dwell estimate. Default
    /// 30ms is a common speech-processing window.
    pub frame_secs: f32,
    /// RMS below this value is considered silence.
    /// PCM samples are normalised to [-1.0, 1.0]
    /// upstream of the detector (cpal's i16/u16
    /// callbacks scale on the fly), so this is
    /// also unitless on that scale. Default 0.01
    /// is below conversational voice but above
    /// most room-noise floors.
    pub threshold_rms: f32,
}

impl Default for SilenceDetectorConfig {
    fn default() -> Self {
        SilenceDetectorConfig {
            sample_rate: 16_000,
            frame_secs: 0.030,
            threshold_rms: 0.01,
        }
    }
}

impl SilenceDetectorConfig {
    /// Convenience: build a config that matches
    /// a specific input rate. Other fields take
    /// defaults.
    pub fn for_sample_rate(sample_rate: u32) -> Self {
        SilenceDetectorConfig {
            sample_rate,
            ..Default::default()
        }
    }
}

/// Sliding RMS detector. See module docs.
///
/// **Not** internally thread-safe — wrap in
/// `Mutex` for shared access (AudioIn does this).
pub struct SilenceDetector {
    config: SilenceDetectorConfig,
    frame_size: usize,
    pending: Vec<f32>,
    /// Samples since the last frame whose RMS
    /// exceeded the threshold. Each completed
    /// frame either adds `frame_size` to this (if
    /// silent) or resets it to 0 (if loud).
    consecutive_silent_samples: usize,
    /// Total samples observed since the most
    /// recent `reset()`. Used by the PTT loop's
    /// `min_speech_secs` and `max_capture_secs`
    /// gates.
    total_samples: usize,
}

impl SilenceDetector {
    pub fn new(config: SilenceDetectorConfig) -> Self {
        let frame_size = ((config.sample_rate as f32) * config.frame_secs).max(1.0) as usize;
        SilenceDetector {
            config,
            frame_size,
            pending: Vec::with_capacity(frame_size * 2),
            consecutive_silent_samples: 0,
            total_samples: 0,
        }
    }

    /// Feed a new batch of audio samples (PCM-f32,
    /// already normalised to roughly [-1.0, 1.0]).
    /// Updates internal frame-RMS + silence-counter
    /// state in O(n).
    pub fn observe(&mut self, samples: &[f32]) {
        self.total_samples = self.total_samples.saturating_add(samples.len());
        self.pending.extend_from_slice(samples);

        // Process full frames. A partial trailing
        // frame stays in `pending` for the next
        // call.
        while self.pending.len() >= self.frame_size {
            let frame: Vec<f32> = self.pending.drain(..self.frame_size).collect();
            let rms = frame_rms(&frame);
            if rms < self.config.threshold_rms {
                self.consecutive_silent_samples = self
                    .consecutive_silent_samples
                    .saturating_add(self.frame_size);
            } else {
                self.consecutive_silent_samples = 0;
            }
        }
    }

    /// How long has silence been sustained since
    /// the most recent loud frame.
    pub fn silence_dwell(&self) -> Duration {
        samples_to_duration(self.consecutive_silent_samples, self.config.sample_rate)
    }

    /// Total recording duration since `reset()`.
    /// Used by the loop's min-speech-recorded and
    /// max-capture gates.
    pub fn total_recorded(&self) -> Duration {
        samples_to_duration(self.total_samples, self.config.sample_rate)
    }

    /// Reset all state for a fresh recording. Call
    /// before starting each new PTT capture.
    pub fn reset(&mut self) {
        self.pending.clear();
        self.consecutive_silent_samples = 0;
        self.total_samples = 0;
    }

    /// Expose the config for read-only access
    /// (diagnostics).
    pub fn config(&self) -> &SilenceDetectorConfig {
        &self.config
    }
}

/// RMS of a slice. Returns 0 for an empty slice.
fn frame_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples.iter().map(|s| (*s as f64) * (*s as f64)).sum();
    let mean_sq = sum_sq / (samples.len() as f64);
    mean_sq.sqrt() as f32
}

fn samples_to_duration(samples: usize, sample_rate: u32) -> Duration {
    if sample_rate == 0 {
        return Duration::ZERO;
    }
    let secs = (samples as f64) / (sample_rate as f64);
    Duration::from_secs_f64(secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detector_16k() -> SilenceDetector {
        SilenceDetector::new(SilenceDetectorConfig::for_sample_rate(16_000))
    }

    #[test]
    fn config_default_is_sane() {
        let c = SilenceDetectorConfig::default();
        assert_eq!(c.sample_rate, 16_000);
        assert!((c.frame_secs - 0.030).abs() < f32::EPSILON);
        assert!((c.threshold_rms - 0.01).abs() < f32::EPSILON);
    }

    #[test]
    fn empty_observe_advances_no_state() {
        let mut det = detector_16k();
        det.observe(&[]);
        assert_eq!(det.silence_dwell(), Duration::ZERO);
        assert_eq!(det.total_recorded(), Duration::ZERO);
    }

    #[test]
    fn sub_frame_partial_does_not_advance_silence_counter() {
        // 16kHz × 30ms = 480 samples per frame.
        // Feeding 100 silent samples should not
        // increment consecutive_silent_samples
        // (no full frame has been processed yet).
        let mut det = detector_16k();
        det.observe(&vec![0.0; 100]);
        assert_eq!(
            det.silence_dwell(),
            Duration::ZERO,
            "partial frame must not trigger silence counter"
        );
        // total_recorded still increases — it
        // tracks samples observed, not frames.
        assert!(det.total_recorded() > Duration::ZERO);
    }

    #[test]
    fn complete_silent_frame_advances_counter() {
        // One full frame of zeros at 16kHz × 30ms.
        let mut det = detector_16k();
        det.observe(&vec![0.0; 480]);
        // One frame of silence → 480 / 16000 = 30ms
        // of dwell.
        let dwell = det.silence_dwell();
        assert!(
            dwell >= Duration::from_millis(29) && dwell <= Duration::from_millis(31),
            "dwell after one silent frame: {dwell:?}"
        );
    }

    #[test]
    fn loud_frame_resets_silence_counter() {
        let mut det = detector_16k();
        // Two silent frames first.
        det.observe(&vec![0.0; 480 * 2]);
        let dwell_before = det.silence_dwell();
        assert!(
            dwell_before >= Duration::from_millis(50),
            "expected ~60ms dwell, got {dwell_before:?}"
        );
        // One loud frame (amplitude well above the
        // 0.01 threshold).
        det.observe(&vec![0.5; 480]);
        assert_eq!(
            det.silence_dwell(),
            Duration::ZERO,
            "loud frame must reset silence dwell"
        );
    }

    #[test]
    fn silence_speech_silence_resumes_counter_from_zero() {
        // Phase 139's exact target shape: operator
        // pauses briefly mid-utterance, then keeps
        // talking, then pauses again. The detector
        // must NOT carry over silence from before
        // the speech.
        let mut det = detector_16k();
        det.observe(&vec![0.0; 480 * 5]); // 150ms silence
        let dwell_first = det.silence_dwell();
        assert!(dwell_first >= Duration::from_millis(140));
        det.observe(&vec![0.5; 480 * 3]); // 90ms speech
        assert_eq!(det.silence_dwell(), Duration::ZERO);
        det.observe(&vec![0.0; 480 * 3]); // 90ms silence
        let dwell_third = det.silence_dwell();
        assert!(
            dwell_third >= Duration::from_millis(80) && dwell_third <= Duration::from_millis(100),
            "post-speech silence must start fresh: {dwell_third:?}"
        );
    }

    #[test]
    fn quiet_below_threshold_is_treated_as_silence() {
        // PCM near zero but not exactly zero — room
        // tone, mic noise floor. Below the 0.01
        // default threshold so it counts as
        // silence.
        let mut det = detector_16k();
        det.observe(&vec![0.005; 480]); // RMS = 0.005 < 0.01
        let dwell = det.silence_dwell();
        assert!(
            dwell >= Duration::from_millis(29),
            "low-amplitude noise must register as silence: {dwell:?}"
        );
    }

    #[test]
    fn loud_above_threshold_breaks_silence() {
        let mut det = detector_16k();
        det.observe(&vec![0.02; 480]); // RMS = 0.02 > 0.01
        assert_eq!(
            det.silence_dwell(),
            Duration::ZERO,
            "RMS above threshold must register as speech"
        );
    }

    #[test]
    fn reset_clears_all_state() {
        let mut det = detector_16k();
        det.observe(&vec![0.0; 480 * 3]);
        det.observe(&vec![0.05; 100]); // partial loud frame
        assert!(det.silence_dwell() > Duration::ZERO);
        assert!(det.total_recorded() > Duration::ZERO);
        det.reset();
        assert_eq!(det.silence_dwell(), Duration::ZERO);
        assert_eq!(det.total_recorded(), Duration::ZERO);
    }

    #[test]
    fn frame_rms_matches_manual_calc() {
        // sqrt((0.5^2 + 0.5^2 + 0.5^2 + 0.5^2) / 4) = 0.5
        assert!((frame_rms(&[0.5, 0.5, 0.5, 0.5]) - 0.5).abs() < 1e-6);
        // sqrt((0 + 0 + 1.0 + 1.0) / 4) = sqrt(0.5) ≈ 0.707
        assert!((frame_rms(&[0.0, 0.0, 1.0, 1.0]) - 0.5_f32.sqrt()).abs() < 1e-6);
        assert_eq!(frame_rms(&[]), 0.0);
    }
}
