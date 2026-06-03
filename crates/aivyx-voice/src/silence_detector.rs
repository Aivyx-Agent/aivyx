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

/// Phase 140 — operator-tunable VAD configuration
/// surfaced as the `[voice.vad]` TOML section.
/// Carries the user-facing knobs; the AudioIn
/// constructor lowers it to a
/// [`SilenceDetectorConfig`] once the device's
/// negotiated sample rate is known.
///
/// All defaults match the Phase 139 hardcoded
/// values so operators who don't write a
/// `[voice.vad]` block get the same behavior
/// they got at Phase 139 exit.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct VoiceVadConfig {
    /// RMS amplitude below which a frame counts
    /// as silence. PCM samples are normalised to
    /// `[-1.0, 1.0]` before reaching the
    /// detector, so this is unitless on that
    /// scale. Lower = stricter (more sensitive
    /// to background noise); higher = more
    /// tolerant. Default `0.01`.
    #[serde(default = "default_threshold_rms")]
    pub threshold_rms: f32,
    /// Sustained-silence duration that triggers
    /// auto-stop. Operator pauses for this long
    /// → mic stops + dispatch. Default `1.5` s.
    #[serde(default = "default_dwell_secs")]
    pub dwell_secs: f32,
    /// Minimum recording duration before
    /// auto-stop can fire. Prevents instant
    /// dispatch on startup-silence before the
    /// operator has started talking. Default
    /// `0.5` s.
    #[serde(default = "default_min_speech_secs")]
    pub min_speech_secs: f32,
    /// Hard cap on a single recording. Protects
    /// against a stuck mic that records forever.
    /// Default `30.0` s.
    #[serde(default = "default_max_capture_secs")]
    pub max_capture_secs: f32,
    /// Per-frame RMS window size. Smaller frames
    /// = more responsive but noisier; larger =
    /// stabler but laggier. Default `0.030` s
    /// (30 ms) — common speech-processing
    /// window.
    #[serde(default = "default_frame_secs")]
    pub frame_secs: f32,
    /// PTT loop poll interval. The recording
    /// loop wakes every `poll_interval_ms` to
    /// check the detector state. Default `100`
    /// ms.
    #[serde(default = "default_poll_interval_ms")]
    pub poll_interval_ms: u64,
}

fn default_threshold_rms() -> f32 {
    0.01
}
fn default_dwell_secs() -> f32 {
    1.5
}
fn default_min_speech_secs() -> f32 {
    0.5
}
fn default_max_capture_secs() -> f32 {
    30.0
}
fn default_frame_secs() -> f32 {
    0.030
}
fn default_poll_interval_ms() -> u64 {
    100
}

impl Default for VoiceVadConfig {
    fn default() -> Self {
        VoiceVadConfig {
            threshold_rms: default_threshold_rms(),
            dwell_secs: default_dwell_secs(),
            min_speech_secs: default_min_speech_secs(),
            max_capture_secs: default_max_capture_secs(),
            frame_secs: default_frame_secs(),
            poll_interval_ms: default_poll_interval_ms(),
        }
    }
}

impl VoiceVadConfig {
    /// Lower this config into the substrate-tier
    /// [`SilenceDetectorConfig`] used by the
    /// detector itself. The detector needs the
    /// device's negotiated sample rate (passed
    /// in) to compute frame sizes.
    pub fn to_silence_detector_config(&self, sample_rate: u32) -> SilenceDetectorConfig {
        SilenceDetectorConfig {
            sample_rate,
            frame_secs: self.frame_secs,
            threshold_rms: self.threshold_rms,
        }
    }

    /// Phase 152 — reject out-of-range field
    /// values at the boundary so the runtime
    /// doesn't have to special-case nonsense
    /// later. Caller (the PTT loop) runs this
    /// at function entry and surfaces the
    /// error to the operator as a configuration
    /// problem rather than silently using the
    /// nonsense values.
    ///
    /// Bounds chosen for the typical operator
    /// setup:
    /// - `threshold_rms`: `[0.0, 10.0]`. Audio
    ///   normalized to ±1.0 means a "true" RMS
    ///   over 1.0 is impossible; the upper
    ///   bound is room for future scaling
    ///   changes. Negative is meaningless.
    /// - `frame_secs`: `(0.0, 1.0]`. Zero
    ///   frames don't exist; a 1-second frame
    ///   is already a generous upper bound for
    ///   speech processing (real-world: 10-50
    ///   ms).
    /// - `dwell_secs`: `(0.0, 60.0]`. Zero
    ///   dwell would auto-stop instantly on
    ///   any silent frame. 60 seconds is the
    ///   "I left for coffee" operator
    ///   threshold.
    /// - `min_speech_secs`: `[0.0, 60.0]`.
    ///   Zero is acceptable (auto-stop right
    ///   away if dwell hits). Upper bound
    ///   matches dwell_secs.
    /// - `max_capture_secs`: `(0.0, 3600.0]`.
    ///   Hour-long capture is the operator-
    ///   leaves-recording-running edge case.
    /// - `poll_interval_ms`: `[1, 5000]`.
    ///   Zero would CPU-spin; 5 seconds is
    ///   already unreasonably laggy.
    pub fn validate(&self) -> Result<(), String> {
        if !(0.0..=10.0).contains(&self.threshold_rms) {
            return Err(format!(
                "[voice.vad] threshold_rms must be in [0.0, 10.0]; got {}",
                self.threshold_rms,
            ));
        }
        if !(self.frame_secs > 0.0 && self.frame_secs <= 1.0) {
            return Err(format!(
                "[voice.vad] frame_secs must be in (0.0, 1.0]; got {}",
                self.frame_secs,
            ));
        }
        if !(self.dwell_secs > 0.0 && self.dwell_secs <= 60.0) {
            return Err(format!(
                "[voice.vad] dwell_secs must be in (0.0, 60.0]; got {}",
                self.dwell_secs,
            ));
        }
        if !(0.0..=60.0).contains(&self.min_speech_secs) {
            return Err(format!(
                "[voice.vad] min_speech_secs must be in [0.0, 60.0]; got {}",
                self.min_speech_secs,
            ));
        }
        if !(self.max_capture_secs > 0.0 && self.max_capture_secs <= 3600.0) {
            return Err(format!(
                "[voice.vad] max_capture_secs must be in (0.0, 3600.0]; got {}",
                self.max_capture_secs,
            ));
        }
        if !(1..=5000).contains(&self.poll_interval_ms) {
            return Err(format!(
                "[voice.vad] poll_interval_ms must be in [1, 5000]; got {}",
                self.poll_interval_ms,
            ));
        }
        // Also reject NaN/infinity at any field
        // — partial-cmp above catches NaN
        // (NaN compares false against everything,
        // so the range check fails). Infinity in
        // dwell/max_capture_secs is caught by
        // the upper-bound clause.
        Ok(())
    }
}

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

    // ---- Phase 140 — VoiceVadConfig TOML tests ----

    #[test]
    fn voice_vad_config_default_matches_phase_139_constants() {
        let cfg = VoiceVadConfig::default();
        assert!((cfg.threshold_rms - 0.01).abs() < f32::EPSILON);
        assert!((cfg.dwell_secs - 1.5).abs() < f32::EPSILON);
        assert!((cfg.min_speech_secs - 0.5).abs() < f32::EPSILON);
        assert!((cfg.max_capture_secs - 30.0).abs() < f32::EPSILON);
        assert!((cfg.frame_secs - 0.030).abs() < f32::EPSILON);
        assert_eq!(cfg.poll_interval_ms, 100);
    }

    #[test]
    fn voice_vad_config_full_toml_parses() {
        let toml = r#"
threshold_rms     = 0.02
dwell_secs        = 2.0
min_speech_secs   = 0.8
max_capture_secs  = 60.0
frame_secs        = 0.040
poll_interval_ms  = 150
"#;
        let cfg: VoiceVadConfig = toml::from_str(toml).expect("parse full section");
        assert!((cfg.threshold_rms - 0.02).abs() < f32::EPSILON);
        assert!((cfg.dwell_secs - 2.0).abs() < f32::EPSILON);
        assert!((cfg.min_speech_secs - 0.8).abs() < f32::EPSILON);
        assert!((cfg.max_capture_secs - 60.0).abs() < f32::EPSILON);
        assert!((cfg.frame_secs - 0.040).abs() < f32::EPSILON);
        assert_eq!(cfg.poll_interval_ms, 150);
    }

    #[test]
    fn voice_vad_config_partial_toml_uses_defaults() {
        // Operator only tunes the threshold; other
        // fields fall back to Phase 139 defaults.
        let toml = r#"
threshold_rms = 0.05
"#;
        let cfg: VoiceVadConfig = toml::from_str(toml).expect("parse partial");
        assert!((cfg.threshold_rms - 0.05).abs() < f32::EPSILON);
        // Untouched fields default-match.
        assert!((cfg.dwell_secs - 1.5).abs() < f32::EPSILON);
        assert!((cfg.min_speech_secs - 0.5).abs() < f32::EPSILON);
        assert_eq!(cfg.poll_interval_ms, 100);
    }

    #[test]
    fn voice_vad_config_lowers_to_silence_detector_config() {
        let cfg = VoiceVadConfig {
            threshold_rms: 0.02,
            frame_secs: 0.040,
            ..Default::default()
        };
        let sd = cfg.to_silence_detector_config(48_000);
        assert_eq!(sd.sample_rate, 48_000);
        assert!((sd.frame_secs - 0.040).abs() < f32::EPSILON);
        assert!((sd.threshold_rms - 0.02).abs() < f32::EPSILON);
    }

    // ---- Phase 152 — VoiceVadConfig::validate ----

    #[test]
    fn validate_default_config_passes() {
        // Phase 139's defaults must continue to
        // pass validation — that's the regression
        // boundary.
        assert!(VoiceVadConfig::default().validate().is_ok());
    }

    #[test]
    fn validate_negative_threshold_rms_rejected() {
        let cfg = VoiceVadConfig {
            threshold_rms: -0.01,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("threshold_rms"), "{err}");
    }

    #[test]
    fn validate_oversize_threshold_rms_rejected() {
        let cfg = VoiceVadConfig {
            threshold_rms: 100.0,
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_zero_frame_secs_rejected() {
        let cfg = VoiceVadConfig {
            frame_secs: 0.0,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("frame_secs"), "{err}");
    }

    #[test]
    fn validate_oversize_frame_secs_rejected() {
        let cfg = VoiceVadConfig {
            frame_secs: 2.0,
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_zero_dwell_secs_rejected() {
        let cfg = VoiceVadConfig {
            dwell_secs: 0.0,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("dwell_secs"), "{err}");
    }

    #[test]
    fn validate_negative_min_speech_secs_rejected() {
        let cfg = VoiceVadConfig {
            min_speech_secs: -1.0,
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_oversize_max_capture_secs_rejected() {
        let cfg = VoiceVadConfig {
            max_capture_secs: 999_999.0,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("max_capture_secs"), "{err}");
    }

    #[test]
    fn validate_zero_poll_interval_ms_rejected() {
        let cfg = VoiceVadConfig {
            poll_interval_ms: 0,
            ..Default::default()
        };
        let err = cfg.validate().unwrap_err();
        assert!(err.contains("poll_interval_ms"), "{err}");
    }

    #[test]
    fn validate_nan_threshold_rms_rejected() {
        // NaN compares false against everything,
        // so the [0.0, 10.0] range check catches
        // it.
        let cfg = VoiceVadConfig {
            threshold_rms: f32::NAN,
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_infinity_max_capture_rejected() {
        let cfg = VoiceVadConfig {
            max_capture_secs: f32::INFINITY,
            ..Default::default()
        };
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_tuned_config_within_bounds_passes() {
        // Real operator-tuned values:
        // noisy room with longer pauses.
        let cfg = VoiceVadConfig {
            threshold_rms: 0.03,
            frame_secs: 0.050,
            dwell_secs: 2.5,
            min_speech_secs: 1.0,
            max_capture_secs: 120.0,
            poll_interval_ms: 200,
        };
        assert!(cfg.validate().is_ok());
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
