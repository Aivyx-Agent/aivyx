//! Microphone capture via `cpal`.
//!
//! Phase 136 — closes Phase 135's audio-I/O deferral
//! on the input side. Builds a cpal input stream
//! against the operator's configured (or default)
//! microphone device, accumulates f32 PCM samples
//! into a shared buffer, and exposes a
//! resample-to-16k-mono helper so the substrate
//! seam (`run_one_voice_turn`) sees its canonical
//! Whisper input format regardless of the device's
//! native rate or channel count.
//!
//! ## Threading
//!
//! cpal stream callbacks run on an OS audio thread;
//! they can't capture non-`Send` state. The capture
//! buffer is `Arc<Mutex<Vec<f32>>>` so the callback
//! pushes samples and the main thread reads them.
//!
//! ## Platform notes (cpal)
//!
//! - **Linux:** ALSA or PipeWire-via-ALSA-shim.
//!   Operators on PipeWire-only setups should
//!   install `pipewire-alsa`.
//! - **macOS:** CoreAudio. Stream is `!Send`; we
//!   keep the AudioIn struct local to the calling
//!   thread (which is what the push-to-talk loop
//!   does anyway).
//! - **Windows:** WASAPI shared mode. Exclusive mode
//!   requires manual config; Phase 137+ candidate.

use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use thiserror::Error;

use crate::asr::{stereo_to_mono_into_16k, WHISPER_SAMPLE_RATE};

/// Errors `AudioIn` can surface.
#[derive(Debug, Error)]
pub enum AudioInError {
    /// No input device available (headless server,
    /// container without audio passthrough, operator
    /// selected a non-existent device name).
    #[error("audio input device unavailable: {0}")]
    DeviceUnavailable(String),
    /// cpal stream construction failed.
    #[error("audio stream build failed: {0}")]
    StreamBuild(String),
    /// cpal stream play() / pause() failed.
    #[error("audio stream control failed: {0}")]
    StreamControl(String),
    /// The device delivered a sample format we don't
    /// support (24-bit packed, f64, etc.). Common
    /// formats (f32, i16, u16) flow through inline.
    #[error("unsupported sample format from device: {0}")]
    UnsupportedFormat(String),
    /// The capture buffer mutex was poisoned by a
    /// panic in another thread — defensive; shouldn't
    /// happen in practice.
    #[error("capture buffer mutex poisoned")]
    BufferPoisoned,
}

/// A live microphone capture session.
///
/// Construct with [`AudioIn::new`] (which selects the
/// device + builds the stream but leaves it paused),
/// then call [`start`](Self::start) and
/// [`stop`](Self::stop) to bracket a recording.
/// [`take_samples`](Self::take_samples) consumes the
/// accumulated audio and returns it normalised to
/// the format Whisper expects (16 kHz mono f32 PCM).
///
/// One capture session per push-to-talk turn; drop
/// the struct between turns or call `clear` to reuse.
pub struct AudioIn {
    stream: cpal::Stream,
    buffer: Arc<Mutex<Vec<f32>>>,
    src_rate: u32,
    src_channels: u16,
}

impl AudioIn {
    /// Build the input stream against the operator's
    /// configured device or the system default. The
    /// stream is built but **not yet playing**; the
    /// caller invokes `start` to begin capture.
    pub fn new(device_name: Option<&str>) -> Result<Self, AudioInError> {
        let host = cpal::default_host();
        let device = match device_name {
            Some(name) => {
                let mut found = None;
                let devices = host.input_devices().map_err(|e| {
                    AudioInError::DeviceUnavailable(format!("enumerate inputs: {e}"))
                })?;
                for d in devices {
                    if let Ok(n) = d.name()
                        && n == name
                    {
                        found = Some(d);
                        break;
                    }
                }
                found.ok_or_else(|| {
                    AudioInError::DeviceUnavailable(format!(
                        "no input device named {name:?}"
                    ))
                })?
            }
            None => host.default_input_device().ok_or_else(|| {
                AudioInError::DeviceUnavailable(
                    "no default input device — check OS audio settings".to_string(),
                )
            })?,
        };

        let config = device
            .default_input_config()
            .map_err(|e| {
                AudioInError::DeviceUnavailable(format!("default_input_config: {e}"))
            })?;
        // cpal 0.17 — SampleRate is a type alias for u32.
        let src_rate: u32 = config.sample_rate();
        let src_channels = config.channels();
        let sample_format = config.sample_format();
        let stream_config: cpal::StreamConfig = config.into();

        let buffer: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));
        let buffer_for_cb = Arc::clone(&buffer);
        let err_fn = |e| eprintln!("audio input stream error: {e}");

        let stream = match sample_format {
            cpal::SampleFormat::F32 => device
                .build_input_stream(
                    &stream_config,
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        if let Ok(mut buf) = buffer_for_cb.lock() {
                            buf.extend_from_slice(data);
                        }
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| AudioInError::StreamBuild(format!("f32: {e}")))?,
            cpal::SampleFormat::I16 => device
                .build_input_stream(
                    &stream_config,
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        if let Ok(mut buf) = buffer_for_cb.lock() {
                            buf.extend(data.iter().map(|&s| {
                                // Map i16 to f32 in [-1.0, 1.0].
                                (s as f32) / (i16::MAX as f32)
                            }));
                        }
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| AudioInError::StreamBuild(format!("i16: {e}")))?,
            cpal::SampleFormat::U16 => device
                .build_input_stream(
                    &stream_config,
                    move |data: &[u16], _: &cpal::InputCallbackInfo| {
                        if let Ok(mut buf) = buffer_for_cb.lock() {
                            buf.extend(data.iter().map(|&s| {
                                // Map u16 [0, 65535] to f32 [-1.0, 1.0].
                                ((s as f32) / (u16::MAX as f32)) * 2.0 - 1.0
                            }));
                        }
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| AudioInError::StreamBuild(format!("u16: {e}")))?,
            other => {
                return Err(AudioInError::UnsupportedFormat(format!("{other:?}")));
            }
        };

        Ok(AudioIn {
            stream,
            buffer,
            src_rate,
            src_channels,
        })
    }

    /// Begin capturing. Clears the buffer so a fresh
    /// recording starts from zero.
    pub fn start(&mut self) -> Result<(), AudioInError> {
        self.clear()?;
        self.stream
            .play()
            .map_err(|e| AudioInError::StreamControl(format!("play: {e}")))
    }

    /// Stop capturing. The stream stays alive (paused);
    /// `start` reuses it.
    pub fn stop(&mut self) -> Result<(), AudioInError> {
        self.stream
            .pause()
            .map_err(|e| AudioInError::StreamControl(format!("pause: {e}")))
    }

    /// Discard any accumulated samples without
    /// stopping the stream.
    pub fn clear(&mut self) -> Result<(), AudioInError> {
        self.buffer
            .lock()
            .map_err(|_| AudioInError::BufferPoisoned)?
            .clear();
        Ok(())
    }

    /// Drain the captured samples and normalise to
    /// 16 kHz mono f32 PCM — Whisper's input format.
    /// The buffer is emptied; subsequent calls return
    /// `Ok(Vec::new())` until the next `start`.
    pub fn take_samples_for_whisper(&mut self) -> Result<Vec<f32>, AudioInError> {
        let raw: Vec<f32> = {
            let mut buf = self
                .buffer
                .lock()
                .map_err(|_| AudioInError::BufferPoisoned)?;
            std::mem::take(&mut *buf)
        };
        Ok(stereo_to_mono_into_16k(&raw, self.src_rate, self.src_channels))
    }

    /// The device's native sample rate (Hz). Surfaced
    /// for diagnostics — operators can read this to
    /// confirm the right device is selected.
    pub fn src_rate(&self) -> u32 {
        self.src_rate
    }

    /// The device's native channel count (1 = mono,
    /// 2 = stereo, etc.). Surfaced for diagnostics.
    pub fn src_channels(&self) -> u16 {
        self.src_channels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whisper_sample_rate_constant_matches_asr_module() {
        // Sanity: the rate we resample to here is the
        // same rate the ASR module exports as its
        // canonical Whisper input rate.
        assert_eq!(WHISPER_SAMPLE_RATE, 16_000);
    }

    // Note — full AudioIn construction tests would need
    // a real audio device. CI / dev environments
    // without one panic at host.default_input_device().
    // Operator-side validation lives in INSTALL.md.
}
