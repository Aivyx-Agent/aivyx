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
use crate::silence_detector::VoiceVadConfig;
use crate::tts::TtsConfig;

/// Phase 138 — type alias for the streaming text-
/// chunk callback that [`VoiceChannel::set_text_sink`]
/// installs. Pulled out so the field type doesn't
/// trip clippy::type_complexity.
type TextSinkFn = Box<dyn Fn(&str) + Send + Sync>;

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
    /// Phase 140 — operator-tunable VAD knobs.
    /// Defaults match Phase 139's hardcoded
    /// values so operators with no `[voice.vad]`
    /// section get unchanged behavior.
    #[serde(default)]
    pub vad: VoiceVadConfig,
}

/// `ChannelContext` impl for voice I/O.
///
/// Holds three pieces of per-channel state:
/// 1. `session` — stable `SessionId` across all turns
///    in this voice session.
/// 2. `token` — per-turn cancellation token; rotated
///    before each `agent.turn` call so a previous
///    turn's timeout / cancel does not pre-cancel
///    turn N+1 (same pattern the four daemon-side
///    stubs use post-audit-C1+H1).
/// 3. `text_buffer` — accumulates `StreamEvent::Text`
///    chunks emitted during the in-flight turn. The
///    session driver reads + clears this buffer
///    after `agent.turn` returns, then chunks the
///    accumulated text into sentences for TTS
///    synthesis.
pub struct VoiceChannel {
    session: SessionId,
    config: VoiceChannelConfig,
    token: Mutex<CancellationToken>,
    text_buffer: Mutex<String>,
    /// Phase 138 — when `Some`, every text-chunk
    /// `StreamEvent` fires this closure instead of
    /// accumulating in `text_buffer`. The streaming
    /// session driver registers a sink that drains
    /// complete sentences into a `tokio::mpsc`
    /// pipeline so the operator hears synthesized
    /// audio while the LLM is still generating the
    /// rest of the response. When `None`, Phase 137
    /// behaviour (buffer-then-flush) is preserved.
    text_sink: Mutex<Option<TextSinkFn>>,
    /// Phase 154 — operator-attached images for
    /// the next turn. The PTT loop's `/image
    /// <path-or-url>` command populates this; the
    /// streaming turn driver consumes via
    /// [`take_pending_images`] when constructing
    /// the Message.
    ///
    /// Phase 156 — promoted from `Option` to
    /// `Vec` so the operator can queue multiple
    /// images per turn by typing `/image`
    /// repeatedly before recording.
    pending_images: Mutex<Vec<(String, Vec<u8>)>>,
}

impl VoiceChannel {
    pub fn new(config: VoiceChannelConfig) -> Self {
        VoiceChannel {
            session: SessionId::new(),
            config,
            token: Mutex::new(CancellationToken::new()),
            text_buffer: Mutex::new(String::new()),
            text_sink: Mutex::new(None),
            pending_images: Mutex::new(Vec::new()),
        }
    }

    /// Install a text-chunk sink for the current
    /// turn. When set, `stream_event` fires the
    /// closure on every `StreamEvent::Text` chunk
    /// instead of accumulating in `text_buffer`.
    /// The streaming session driver uses this to
    /// pipeline text into a sentence-boundary
    /// drainer + TTS engine while the agent is
    /// still streaming. Replaces any prior sink.
    pub fn set_text_sink<F>(&self, sink: F)
    where
        F: Fn(&str) + Send + Sync + 'static,
    {
        let mut slot = self.text_sink.lock().expect("text_sink poisoned");
        *slot = Some(Box::new(sink));
    }

    /// Remove the currently-installed text sink, if
    /// any. After this, `stream_event` reverts to
    /// the Phase 137 buffer-accumulation path.
    pub fn clear_text_sink(&self) {
        let mut slot = self.text_sink.lock().expect("text_sink poisoned");
        *slot = None;
    }

    /// Phase 154 + 156 — append an image to
    /// the queue for the next turn. The PTT
    /// loop's `/image <path-or-url>` command
    /// calls this after loading/fetching the
    /// image + inferring the media type. Phase
    /// 156 promotes from "replace prior" to
    /// "append to list" so operators can attach
    /// multiple images per turn by typing
    /// `/image` repeatedly before recording.
    pub fn append_pending_image(&self, media_type: String, data: Vec<u8>) {
        let mut slot = self
            .pending_images
            .lock()
            .expect("pending_images poisoned");
        slot.push((media_type, data));
    }

    /// Phase 154 + 156 — consume the queued
    /// images. The streaming turn driver calls
    /// this once when building the Message; on
    /// non-empty Vec it constructs a
    /// `MessageContent::Mixed` with one Text
    /// part + N Image parts. The slot is
    /// cleared so subsequent turns start fresh.
    pub fn take_pending_images(&self) -> Vec<(String, Vec<u8>)> {
        let mut slot = self
            .pending_images
            .lock()
            .expect("pending_images poisoned");
        std::mem::take(&mut *slot)
    }

    pub fn config(&self) -> &VoiceChannelConfig {
        &self.config
    }

    /// Drain the buffered turn text and reset the
    /// buffer to empty. Called by the session driver
    /// immediately after `agent.turn` returns; the
    /// returned string is passed to
    /// `chunk_into_sentences` then to the TTS engine.
    pub fn take_buffered_text(&self) -> String {
        let mut buf = self.text_buffer.lock().expect("text_buffer poisoned");
        std::mem::take(&mut *buf)
    }

    /// Read the buffered turn text without clearing it.
    /// Test-only — production code uses
    /// [`take_buffered_text`].
    #[cfg(test)]
    pub fn peek_buffered_text(&self) -> String {
        self.text_buffer
            .lock()
            .expect("text_buffer poisoned")
            .clone()
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

    async fn stream_event(&self, event: StreamEvent<'_>) -> Result<(), ChannelError> {
        // Phase 135 Task 5 — buffer text-chunk events so
        // the session driver can synthesize the full
        // response after `agent.turn` returns.
        //
        // We deliberately ignore non-text events
        // (ToolCallStarted, ToolCallFinished, Status,
        // ToolOutput, Attachment): voice doesn't have a
        // good way to speak "tool call: fs.read" inline.
        // The operator hears the agent's natural
        // response; tool calls happen silently. Phase
        // 136+ could surface tool activity via short
        // chimes or a separate channel.
        if let StreamEvent::Text(s) = event {
            // Phase 138 — if a streaming sink is
            // installed, dispatch the chunk through
            // it instead of buffering. The sink
            // typically accumulates into its own
            // partial-sentence buffer and forwards
            // complete sentences to a TTS pipeline.
            // The session driver clears the sink
            // after the turn completes and flushes
            // any leftover partial fragment.
            //
            // We hold the sink lock only long enough
            // to read the Option + call the closure;
            // we don't hold it across the call's
            // body to avoid blocking concurrent
            // stream_event invocations on the same
            // channel (none today, but a posture
            // we may want later).
            let sink_present = {
                let slot = self
                    .text_sink
                    .lock()
                    .map_err(|_| ChannelError::Send("text_sink poisoned".to_string()))?;
                if let Some(sink) = slot.as_ref() {
                    sink(s);
                    true
                } else {
                    false
                }
            };
            if !sink_present {
                let mut buf = self
                    .text_buffer
                    .lock()
                    .map_err(|_| ChannelError::Send("text_buffer poisoned".to_string()))?;
                buf.push_str(s);
            }
        }
        Ok(())
    }

    async fn finalize(&self, _outcome: &TurnOutcome) -> Result<(), ChannelError> {
        // No-op for the voice channel. The session
        // driver handles post-turn synthesis +
        // playback after this returns, reading the
        // buffered text via `take_buffered_text`.
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
    use std::sync::Arc;

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
    fn voice_channel_config_parses_vad_subsection() {
        // Phase 140 — operators tune VAD by adding
        // a `[voice.vad]` block. Verify the
        // nesting works under the existing
        // `VoiceChannelConfig` shape.
        let toml = r#"
asr_engine = "whisper-rs"
tts_engine = "piper"

[vad]
threshold_rms = 0.02
dwell_secs    = 2.5

[asr]
model_path = "/m/w.bin"

[tts]
voice_path = "/m/p.onnx"
"#;
        let cfg: VoiceChannelConfig = toml::from_str(toml).expect("parse");
        assert!((cfg.vad.threshold_rms - 0.02).abs() < f32::EPSILON);
        assert!((cfg.vad.dwell_secs - 2.5).abs() < f32::EPSILON);
        // Untouched VAD fields default-match.
        assert!((cfg.vad.min_speech_secs - 0.5).abs() < f32::EPSILON);
        assert_eq!(cfg.vad.poll_interval_ms, 100);
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

    // -------- text buffering for the turn loop --------

    #[tokio::test]
    async fn stream_event_text_accumulates_in_buffer() {
        let ch = VoiceChannel::new(VoiceChannelConfig::default());
        ch.stream_event(StreamEvent::Text("Hello, ")).await.unwrap();
        ch.stream_event(StreamEvent::Text("world.")).await.unwrap();
        assert_eq!(ch.peek_buffered_text(), "Hello, world.");
    }

    #[tokio::test]
    async fn stream_event_ignores_non_text_variants() {
        use aivyx_core::ToolId;
        let ch = VoiceChannel::new(VoiceChannelConfig::default());
        let tool_id = ToolId::new();
        let empty_input = serde_json::json!({});
        ch.stream_event(StreamEvent::ToolCallStarted {
            tool: tool_id,
            tool_name: "fs.read",
            input: &empty_input,
        })
        .await
        .unwrap();
        ch.stream_event(StreamEvent::Status("thinking..."))
            .await
            .unwrap();
        assert_eq!(
            ch.peek_buffered_text(),
            "",
            "non-text stream events must not pollute the TTS buffer"
        );
    }

    #[tokio::test]
    async fn take_buffered_text_drains_and_resets() {
        let ch = VoiceChannel::new(VoiceChannelConfig::default());
        ch.stream_event(StreamEvent::Text("First turn response."))
            .await
            .unwrap();
        let first = ch.take_buffered_text();
        assert_eq!(first, "First turn response.");
        assert_eq!(
            ch.peek_buffered_text(),
            "",
            "buffer must be empty after take"
        );
        // Next turn fills it again from a clean state.
        ch.stream_event(StreamEvent::Text("Second turn."))
            .await
            .unwrap();
        assert_eq!(ch.take_buffered_text(), "Second turn.");
    }

    // -------- Phase 138 — streaming text sink --------

    #[tokio::test]
    async fn text_sink_active_diverts_chunks_away_from_buffer() {
        use std::sync::Mutex as StdMutex;
        let ch = VoiceChannel::new(VoiceChannelConfig::default());
        let captured: Arc<StdMutex<Vec<String>>> = Arc::new(StdMutex::new(Vec::new()));
        let captured_for_sink = Arc::clone(&captured);
        ch.set_text_sink(move |s| {
            captured_for_sink
                .lock()
                .expect("captured")
                .push(s.to_string());
        });
        ch.stream_event(StreamEvent::Text("Hello, ")).await.unwrap();
        ch.stream_event(StreamEvent::Text("world.")).await.unwrap();
        // Sink saw each chunk in order.
        let got = captured.lock().unwrap().clone();
        assert_eq!(got, vec!["Hello, ".to_string(), "world.".to_string()]);
        // Buffer stayed empty — the sink intercepts.
        assert_eq!(
            ch.peek_buffered_text(),
            "",
            "active sink must not also buffer"
        );
    }

    #[tokio::test]
    async fn text_sink_cleared_reverts_to_buffer() {
        use std::sync::Mutex as StdMutex;
        let ch = VoiceChannel::new(VoiceChannelConfig::default());
        let captured: Arc<StdMutex<Vec<String>>> = Arc::new(StdMutex::new(Vec::new()));
        let captured_for_sink = Arc::clone(&captured);
        ch.set_text_sink(move |s| {
            captured_for_sink
                .lock()
                .expect("captured")
                .push(s.to_string());
        });
        ch.stream_event(StreamEvent::Text("during streaming "))
            .await
            .unwrap();
        // After the streaming turn ends, the driver
        // clears the sink. Subsequent events fall
        // back to buffer accumulation (Phase 137
        // path).
        ch.clear_text_sink();
        ch.stream_event(StreamEvent::Text("after the turn"))
            .await
            .unwrap();
        // Sink captured only the streaming-phase
        // chunk.
        assert_eq!(
            captured.lock().unwrap().clone(),
            vec!["during streaming ".to_string()],
        );
        // Buffer captured only the post-clear chunk.
        assert_eq!(ch.peek_buffered_text(), "after the turn");
    }

    #[tokio::test]
    async fn text_sink_replaces_prior_sink() {
        use std::sync::Mutex as StdMutex;
        let ch = VoiceChannel::new(VoiceChannelConfig::default());
        let first: Arc<StdMutex<Vec<String>>> = Arc::new(StdMutex::new(Vec::new()));
        let second: Arc<StdMutex<Vec<String>>> = Arc::new(StdMutex::new(Vec::new()));
        let first_for_sink = Arc::clone(&first);
        let second_for_sink = Arc::clone(&second);
        ch.set_text_sink(move |s| {
            first_for_sink.lock().unwrap().push(s.to_string());
        });
        ch.stream_event(StreamEvent::Text("for-first"))
            .await
            .unwrap();
        // Replace the sink mid-flight (e.g. a
        // hypothetical "swap engines" scenario).
        ch.set_text_sink(move |s| {
            second_for_sink.lock().unwrap().push(s.to_string());
        });
        ch.stream_event(StreamEvent::Text("for-second"))
            .await
            .unwrap();
        assert_eq!(first.lock().unwrap().clone(), vec!["for-first".to_string()]);
        assert_eq!(
            second.lock().unwrap().clone(),
            vec!["for-second".to_string()]
        );
    }

    #[tokio::test]
    async fn finalize_does_not_drain_buffer() {
        // The session driver — not finalize — drains
        // the buffer so the driver can synthesize the
        // full response after agent.turn returns. If
        // finalize drained, the driver would see empty.
        let ch = VoiceChannel::new(VoiceChannelConfig::default());
        ch.stream_event(StreamEvent::Text("Buffered response."))
            .await
            .unwrap();
        ch.finalize(&TurnOutcome::Completed {
            final_message: "Buffered response.".to_string(),
            tool_calls_made: 0,
            duration: std::time::Duration::from_secs(1),
        })
        .await
        .unwrap();
        assert_eq!(ch.peek_buffered_text(), "Buffered response.");
    }

    // ---- Phase 154 + 156 — pending_images ----

    #[test]
    fn pending_images_take_when_empty_returns_empty_vec() {
        let ch = VoiceChannel::new(VoiceChannelConfig::default());
        assert!(ch.take_pending_images().is_empty());
    }

    #[test]
    fn pending_images_append_then_take_round_trip() {
        let ch = VoiceChannel::new(VoiceChannelConfig::default());
        ch.append_pending_image("image/png".to_string(), vec![1, 2, 3, 4]);
        let taken = ch.take_pending_images();
        assert_eq!(taken.len(), 1);
        assert_eq!(taken[0].0, "image/png");
        assert_eq!(taken[0].1, vec![1, 2, 3, 4]);
        // Subsequent take returns empty — the
        // queue was cleared.
        assert!(ch.take_pending_images().is_empty());
    }

    #[test]
    fn pending_images_phase_156_append_accumulates_multiple() {
        // Phase 156 — operator types /image
        // foo.png then /image bar.png before
        // recording; the queue accumulates both.
        // (Pre-156 set_pending_image replaced;
        // post-156 append accumulates.)
        let ch = VoiceChannel::new(VoiceChannelConfig::default());
        ch.append_pending_image("image/png".to_string(), vec![1]);
        ch.append_pending_image("image/jpeg".to_string(), vec![2, 3]);
        ch.append_pending_image("image/gif".to_string(), vec![4, 5, 6]);
        let taken = ch.take_pending_images();
        assert_eq!(taken.len(), 3);
        assert_eq!(taken[0].0, "image/png");
        assert_eq!(taken[1].0, "image/jpeg");
        assert_eq!(taken[2].0, "image/gif");
        // Order is append-order — first /image
        // shows up first in the Vec.
        assert_eq!(taken[2].1, vec![4, 5, 6]);
    }
}
