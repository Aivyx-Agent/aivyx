//! `VoiceChannel` — Aivyx's voice `ChannelContext` impl.
//!
//! Phase 135 Task 2 ships the skeleton; the push-to-talk
//! loop wiring lands in Task 5 once the ASR + TTS
//! adapters in Tasks 3 + 4 are concrete.

use std::path::PathBuf;
use std::sync::Mutex;

use aivyx_core::{
    CancellationToken, ChannelContext, ChannelError, ChannelPlatform, SessionId, StreamEvent,
    TurnOutcome,
};

use crate::asr::AsrConfig;
use crate::tts::TtsConfig;

/// Operator-supplied config for the voice channel.
/// Threaded through `[voice]` in `aivyx.toml`.
#[derive(Debug, Clone, serde::Deserialize, Default)]
pub struct VoiceChannelConfig {
    /// ASR engine selection. Aivyx supports
    /// `"whisper-rs"` (default) and `"whisper-cpp-plus"`
    /// (alternative). Other strings yield a clear
    /// error at channel-construction time.
    #[serde(default)]
    pub asr_engine: Option<String>,
    /// TTS engine selection. Aivyx supports `"piper"`
    /// in Phase 135.
    #[serde(default)]
    pub tts_engine: Option<String>,
    /// ASR config (model path + tuning).
    #[serde(default)]
    pub asr: AsrConfig,
    /// TTS config (voice path + tuning).
    #[serde(default)]
    pub tts: TtsConfig,
    /// Optional override of the cpal input device name.
    /// When `None`, cpal picks the system default.
    #[serde(default)]
    pub input_device: Option<String>,
    /// Optional override of the cpal output device name.
    /// When `None`, cpal picks the system default.
    #[serde(default)]
    pub output_device: Option<String>,
    /// Optional path to log raw mic-capture audio for
    /// debugging. When `None` (the default), nothing
    /// is written. Operators flagging mic-quality
    /// issues use this to capture a sample for
    /// inspection.
    #[serde(default)]
    pub capture_debug_path: Option<PathBuf>,
}

/// `ChannelContext` impl for voice I/O. Phase 135 ships
/// the trait surface + identity bits; the push-to-talk
/// dispatch loop is wired in Task 5.
pub struct VoiceChannel {
    session: SessionId,
    config: VoiceChannelConfig,
    /// Rotated per turn so a previous turn's
    /// timeout / `/cancel` does not pre-cancel turn
    /// N+1. Same pattern the four daemon-side stubs
    /// (Telegram / Discord / Slack / Web) use post-
    /// audit-C1+H1.
    token: Mutex<CancellationToken>,
}

impl VoiceChannel {
    pub fn new(config: VoiceChannelConfig) -> Self {
        VoiceChannel {
            session: SessionId::new(),
            config,
            token: Mutex::new(CancellationToken::new()),
        }
    }

    pub fn config(&self) -> &VoiceChannelConfig {
        &self.config
    }
}

#[async_trait::async_trait]
impl ChannelContext for VoiceChannel {
    fn channel_name(&self) -> &str {
        "aivyx-voice"
    }

    fn platform(&self) -> ChannelPlatform {
        // Phase 135 — `ChannelPlatform::Voice` variant
        // lives in `aivyx-core` and breaks that crate's
        // streak. Wired in Task 5.
        ChannelPlatform::Voice
    }

    fn trust_tier(&self) -> aivyx_capability::TrustTier {
        // Voice runs on the operator's own machine —
        // same trust posture as `LocalChannel`.
        aivyx_capability::TrustTier::Trusted
    }

    fn session_id(&self) -> SessionId {
        self.session
    }

    async fn stream_event(&self, _event: StreamEvent<'_>) -> Result<(), ChannelError> {
        // Phase 135 Task 2 — placeholder. Task 5 wires
        // text-chunk buffering + sentence-boundary
        // chunking + TTS dispatch.
        Ok(())
    }

    async fn finalize(&self, _outcome: &TurnOutcome) -> Result<(), ChannelError> {
        // Phase 135 Task 2 — placeholder. Task 5 emits
        // a closing audio signal so the operator hears
        // the turn ended.
        Ok(())
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.token.lock().expect("token mutex poisoned").clone()
    }

    fn reset_cancellation(&self) {
        let mut slot = self.token.lock().expect("token mutex poisoned");
        *slot = CancellationToken::new();
    }

    fn cancel_inflight(&self) {
        self.token.lock().expect("token mutex poisoned").cancel();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_channel_reports_correct_identity() {
        let ch = VoiceChannel::new(VoiceChannelConfig::default());
        assert_eq!(ch.channel_name(), "aivyx-voice");
        assert_eq!(ch.platform(), ChannelPlatform::Voice);
        assert_eq!(ch.trust_tier(), aivyx_capability::TrustTier::Trusted);
    }

    #[test]
    fn voice_channel_session_id_stable_across_calls() {
        let ch = VoiceChannel::new(VoiceChannelConfig::default());
        assert_eq!(ch.session_id(), ch.session_id());
    }

    // Audit C1+H1 regression — same shape every daemon
    // stub got in the Agent Loop audit. The voice
    // channel reuses one `VoiceChannel` across turns
    // of the same session; `reset_cancellation` must
    // un-stick the token between turns, and
    // `cancel_inflight` must fire it.
    #[test]
    fn cancel_inflight_then_reset_yields_fresh_token() {
        let ch = VoiceChannel::new(VoiceChannelConfig::default());
        let stale = ch.cancellation_token();
        assert!(!stale.is_cancelled());
        ch.cancel_inflight();
        assert!(stale.is_cancelled());
        ch.reset_cancellation();
        assert!(!ch.cancellation_token().is_cancelled());
        assert!(stale.is_cancelled());
    }

    #[test]
    fn voice_channel_config_deserializes_full_section() {
        let toml = r#"
asr_engine = "whisper-rs"
tts_engine = "piper"
input_device = "USB Mic"
output_device = "Default"

[asr]
model_path = "/models/ggml-base.en.bin"
language = "en"

[tts]
voice_path = "/models/en_US-amy-medium.onnx"
"#;
        let cfg: VoiceChannelConfig = toml::from_str(toml).expect("parse");
        assert_eq!(cfg.asr_engine.as_deref(), Some("whisper-rs"));
        assert_eq!(cfg.tts_engine.as_deref(), Some("piper"));
        assert_eq!(cfg.input_device.as_deref(), Some("USB Mic"));
        assert_eq!(cfg.output_device.as_deref(), Some("Default"));
        assert!(cfg.asr.model_path.is_some());
        assert!(cfg.tts.voice_path.is_some());
    }

    #[test]
    fn voice_channel_config_default_is_all_none() {
        let cfg = VoiceChannelConfig::default();
        assert!(cfg.asr_engine.is_none());
        assert!(cfg.tts_engine.is_none());
        assert!(cfg.input_device.is_none());
        assert!(cfg.output_device.is_none());
        assert!(cfg.capture_debug_path.is_none());
    }
}
