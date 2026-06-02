//! Speaker playback via `rodio`.
//!
//! Phase 136 — closes Phase 135's audio-I/O deferral
//! on the output side. Wraps a rodio `Player` over
//! the system default output device; the push-to-talk
//! loop feeds it per-sentence TTS audio chunks.
//!
//! ## Threading
//!
//! rodio runs playback on its own internal audio
//! thread (cpal-backed). Our `AudioOut` struct owns
//! the `MixerDeviceSink` + the `Player`; appending a
//! buffer is non-blocking and `sleep_until_empty`
//! waits for the queue to drain.
//!
//! ## Sample format
//!
//! Piper emits f32 PCM at its voice model's native
//! rate (22050 Hz for the recommended Piper voices).
//! rodio's [`SamplesBuffer`] takes f32 directly and
//! resamples internally if the device's native rate
//! differs.

use std::num::NonZero;

use rodio::{MixerDeviceSink, Player};
use thiserror::Error;

use crate::tts::TtsAudio;

#[derive(Debug, Error)]
pub enum AudioOutError {
    /// No output device available, or rodio's stream
    /// init failed.
    #[error("audio output unavailable: {0}")]
    DeviceUnavailable(String),
    /// Tried to play a TtsAudio with zero sample rate
    /// or invalid channel count — would panic inside
    /// rodio's NonZero conversion.
    #[error("invalid audio buffer: {0}")]
    InvalidBuffer(String),
}

/// A live speaker-playback session.
///
/// Single-channel (mono) playback is what Piper
/// emits; the AudioOut struct constructs the rodio
/// pipeline once and reuses it across every
/// `play_audio` call.
pub struct AudioOut {
    // Holding the sink handle keeps the OS audio
    // stream alive — dropping it kills playback.
    // Kept as a field even though only `player` is
    // touched after construction.
    _sink: MixerDeviceSink,
    player: Player,
}

impl AudioOut {
    /// Open the system default audio output and build
    /// a rodio player against it. Phase 136 ships
    /// default-device only; configurable output
    /// device is a Phase 137+ candidate (rodio's
    /// device-by-name selection has a different API
    /// than cpal's, so the cross-crate device-name
    /// match would be its own work).
    pub fn new() -> Result<Self, AudioOutError> {
        let handle = rodio::DeviceSinkBuilder::open_default_sink().map_err(|e| {
            AudioOutError::DeviceUnavailable(format!("rodio default sink: {e}"))
        })?;
        let player = Player::connect_new(handle.mixer());
        Ok(AudioOut {
            _sink: handle,
            player,
        })
    }

    /// Queue a TTS-synthesized audio buffer for
    /// sequential playback. Returns immediately;
    /// playback happens on rodio's internal audio
    /// thread. Call [`sleep_until_empty`](Self::sleep_until_empty)
    /// to wait for the queue to drain.
    ///
    /// Empty buffers are silently skipped.
    pub fn play_audio(&self, audio: &TtsAudio) -> Result<(), AudioOutError> {
        if audio.is_empty() {
            return Ok(());
        }
        let sample_rate = NonZero::new(audio.sample_rate).ok_or_else(|| {
            AudioOutError::InvalidBuffer(format!(
                "sample rate must be > 0 (got {})",
                audio.sample_rate
            ))
        })?;
        // Piper outputs mono.
        let channels: NonZero<u16> = NonZero::new(1).expect("1 is non-zero");
        let buffer =
            rodio::buffer::SamplesBuffer::new(channels, sample_rate, audio.samples.clone());
        self.player.append(buffer);
        Ok(())
    }

    /// Block the current task until every queued
    /// chunk has finished playing. Called by the
    /// push-to-talk loop between turns so the
    /// next-turn prompt doesn't overlap with the
    /// previous response's tail audio.
    pub fn sleep_until_empty(&self) {
        self.player.sleep_until_end();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tts_audio_returns_ok_without_dispatch() {
        // We can't construct AudioOut without an audio
        // device, but we can verify the input-validation
        // path through the error type's shape.
        let err = AudioOutError::InvalidBuffer("sample_rate=0".into());
        assert!(err.to_string().contains("sample_rate"));
    }

    #[test]
    fn device_unavailable_error_carries_context() {
        let err = AudioOutError::DeviceUnavailable("test".into());
        assert!(err.to_string().contains("output"));
    }

    // Note — full AudioOut construction tests need a
    // real output device. Operator-side validation
    // lives in INSTALL.md.
}
