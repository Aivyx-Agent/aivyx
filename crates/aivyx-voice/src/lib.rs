//! # aivyx-voice
//!
//! Voice I/O channel for the Aivyx agent. Phase 135 —
//! the eighth channel adapter in the workspace, joining
//! Local CLI, Telegram, Discord, Slack, Web UI, Daemon
//! IPC, and Webhook.
//!
//! ## What this crate provides
//!
//! - [`asr`] — Automatic Speech Recognition. Operator
//!   speaks into the microphone; the engine produces
//!   text. Phase 135 ships two engines behind feature
//!   flags: `whisper-rs` (default) and
//!   `whisper-cpp-plus` (alternative with built-in
//!   Silero VAD + PCM streaming).
//! - [`tts`] — Text-to-Speech. Agent emits text; the
//!   engine produces PCM samples played through the
//!   speakers. Phase 135 ships Piper via `piper1-rs`.
//! - [`channel`] — `VoiceChannel`, the
//!   `aivyx_core::ChannelContext` impl that wires the
//!   push-to-talk loop: mic capture → ASR → agent
//!   turn → buffer-until-sentence-boundary → TTS →
//!   speaker playback.
//!
//! ## Privacy posture
//!
//! Everything runs in-process on the operator's
//! machine. **Zero outbound network calls during
//! inference.** Operator audio never leaves the
//! device. Same posture as Phase 134's embedded LLM
//! inference; voice extends it end-to-end.
//!
//! ## Phase 135 scope cap
//!
//! - **Push-to-talk only.** Operator presses Enter to
//!   start/stop recording. Wake-word activation
//!   ("Hey Aivyx") and continuous VAD-trimmed
//!   listening are Phase 136+ candidates.
//! - **No streaming TTS during LLM generation.** The
//!   agent's full response is buffered, then
//!   synthesized + played. Streaming-as-it-generates
//!   is Phase 136+.
//! - **No multimodal.** Voice is text only; spoken
//!   descriptions of images are Phase 136+.
//! - **Unit-tested conversion logic only.** Real audio
//!   I/O validation is operator work (the "speak
//!   into the mic and hear the response" loop needs
//!   a real machine with a real mic + speakers).

pub mod asr;
pub mod audio_in;
pub mod audio_out;
pub mod channel;
pub mod session;
pub mod tts;

pub use audio_in::{AudioIn, AudioInError};
pub use audio_out::{AudioOut, AudioOutError};
pub use channel::{VoiceChannel, VoiceChannelConfig};
pub use session::{run_one_voice_turn, run_push_to_talk_loop, VoiceSessionError, VoiceTurnResult};
