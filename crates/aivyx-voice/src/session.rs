//! Voice session driver — the push-to-talk loop that
//! wires mic capture → ASR → `agent.turn` →
//! sentence-boundary-buffered TTS → speaker playback.
//!
//! Phase 135 ships the loop logic + an explicit
//! "audio I/O integration is operator validation work"
//! frame. The substrate runs end-to-end on the
//! happy path: well-behaved cpal + rodio + a real
//! Whisper model + a real Piper voice. Operator-side
//! validation lands as feedback against this
//! skeleton.

use std::sync::Arc;

use aivyx_core::{Agent, Message};
use thiserror::Error;

use crate::asr::{AsrEngine, AsrError};
use crate::channel::VoiceChannel;
use crate::tts::{chunk_into_sentences, TtsEngine, TtsError};

/// Errors the push-to-talk session driver can surface.
#[derive(Debug, Error)]
pub enum VoiceSessionError {
    /// Failed to open the operator's microphone or
    /// speaker. Common causes: no audio device
    /// available (headless server, container without
    /// audio passthrough), device-permission denied,
    /// or the operator selected a non-existent device
    /// name in `[voice] input_device` / `output_device`.
    #[error("audio device error: {0}")]
    AudioDevice(String),

    /// Mic capture failed mid-recording.
    #[error("audio capture error: {0}")]
    AudioCapture(String),

    /// Playback failed mid-utterance.
    #[error("audio playback error: {0}")]
    AudioPlayback(String),

    /// ASR engine error other than `Empty` (which the
    /// loop translates to "no input, prompt again").
    #[error("ASR error: {0}")]
    Asr(#[from] AsrError),

    /// TTS engine error.
    #[error("TTS error: {0}")]
    Tts(#[from] TtsError),

    /// Phase 135 — the integration layer for the
    /// audio I/O loop (cpal mic capture + rodio
    /// playback) is not yet implemented. The
    /// substrate seam `run_one_voice_turn` is
    /// complete and unit-tested; operators with
    /// audio I/O experience can build the loop
    /// against it locally. Phase 136+ ships the
    /// loop here.
    #[error(
        "Phase 135 ships the voice substrate but the cpal/rodio audio I/O loop \
         is operator-validation work. The substrate seam \
         `aivyx_voice::run_one_voice_turn(agent, channel, asr, tts, captured_audio)` \
         is complete and unit-tested; build the mic-capture + speaker-playback wrapper \
         against it locally. See PHASE_135.md + INSTALL.md voice section for the \
         wiring sketch."
    )]
    LoopNotYetImplemented,
}

/// One iteration of the push-to-talk loop.
///
/// Returns `Ok(Some(reply))` when the operator spoke,
/// the agent responded, and the response was
/// synthesized + queued for playback. Returns
/// `Ok(None)` when the operator's utterance produced
/// no transcription (silence, garbled audio) — the
/// session driver re-prompts. Returns `Err(_)` for
/// hardware / engine failures the operator must
/// address.
///
/// This function is the **integration substrate**:
/// it accepts the captured audio + the agent + the
/// channel + the two engines and produces the
/// agent's reply text. Real audio I/O (cpal stream
/// for mic, rodio sink for speakers) is the caller's
/// responsibility — see [`run_push_to_talk_loop`]
/// for the full wiring; this lower-level seam exists
/// so the integration is testable end-to-end with a
/// synthesized PCM buffer instead of a real mic.
pub async fn run_one_voice_turn<A>(
    agent: &Arc<A>,
    channel: &Arc<VoiceChannel>,
    asr: &dyn AsrEngine,
    tts: &dyn TtsEngine,
    captured_audio: &[f32],
) -> Result<Option<VoiceTurnResult>, VoiceSessionError>
where
    A: Agent + ?Sized + 'static,
{
    // Step 1 — transcribe the operator's utterance.
    let transcribed = match asr.transcribe(captured_audio).await {
        Ok(text) => text,
        Err(AsrError::Empty) => return Ok(None),
        Err(e) => return Err(VoiceSessionError::Asr(e)),
    };

    // Step 2 — clear any leftover buffered text from a
    // prior turn (defensive; `take_buffered_text` should
    // have done this), reset the cancellation token so
    // a previous turn's cancel doesn't pre-cancel this
    // one, and dispatch.
    let _drained_prior = channel.take_buffered_text();
    use aivyx_core::ChannelContext;
    channel.reset_cancellation();

    let message = Message::text(channel.session_id(), transcribed.clone());
    let outcome = agent.turn(message, channel.as_ref()).await;

    // Step 3 — synthesize the buffered response.
    // `take_buffered_text` empties the buffer so the
    // next turn starts clean.
    let response_text = channel.take_buffered_text();
    let mut audio_chunks: Vec<crate::tts::TtsAudio> = Vec::new();
    if !response_text.trim().is_empty() {
        for sentence in chunk_into_sentences(&response_text) {
            match tts.synthesize(&sentence).await {
                Ok(audio) if !audio.is_empty() => audio_chunks.push(audio),
                Ok(_) => {} // empty synthesis (e.g. punctuation only)
                Err(TtsError::Input(_)) => {
                    // Empty after trim — skip silently.
                }
                Err(e) => return Err(VoiceSessionError::Tts(e)),
            }
        }
    }

    Ok(Some(VoiceTurnResult {
        transcribed,
        response_text,
        audio_chunks,
        outcome,
    }))
}

/// Result of one push-to-talk iteration. The caller
/// (the [`run_push_to_talk_loop`] driver) hands the
/// audio chunks to rodio for playback.
#[derive(Debug)]
pub struct VoiceTurnResult {
    /// What the operator said (Whisper output).
    pub transcribed: String,
    /// What the agent replied (buffered text-chunk
    /// stream).
    pub response_text: String,
    /// Per-sentence synthesized audio buffers, ready
    /// for sequential playback.
    pub audio_chunks: Vec<crate::tts::TtsAudio>,
    /// The full `TurnOutcome` from the agent — passed
    /// through so the session driver can log, audit,
    /// or short-circuit on Escalated / TimedOut /
    /// MaxStepsExceeded.
    pub outcome: aivyx_core::TurnOutcome,
}

/// Full push-to-talk loop entry point.
///
/// Phase 136 closed out Phase 135's audio-I/O
/// deferral: this function now drives the
/// interactive loop end-to-end.
///
/// ## Loop shape
///
/// 1. Print prompt: "Press Enter to start
///    recording, Enter again to stop. Type 'quit'
///    to exit."
/// 2. Block on stdin until the operator hits Enter
///    (or types `quit`).
/// 3. Open a cpal input stream against the
///    configured (or default) mic device.
/// 4. Block on stdin until the operator hits Enter
///    again — this is the "stop recording" signal.
/// 5. Stop the stream and drain the captured
///    samples (resampled + downmixed to 16 kHz mono
///    via the substrate helpers).
/// 6. Hand off to the substrate seam
///    [`run_one_voice_turn`]. Empty transcription
///    re-prompts; agent / engine errors surface to
///    the operator and loop continues.
/// 7. For each per-sentence audio chunk, queue it
///    on a rodio sink; wait for the queue to
///    drain.
/// 8. Loop.
///
/// ## Threading notes
///
/// `cpal::Stream` is `!Send` on macOS, so the
/// `AudioIn` / `AudioOut` handles are deliberately
/// scoped to the synchronous prelude/postlude of
/// each iteration — they never cross an `.await`
/// point. The agent turn dispatch + TTS synthesis
/// happen between handle lifetimes, which keeps the
/// macOS build happy.
///
/// `std::io::stdin().read_line` blocks the tokio
/// worker briefly per prompt. Acceptable for an
/// interactive REPL where nothing else needs
/// attention; identical posture to LocalChannel's
/// REPL loop.
pub async fn run_push_to_talk_loop<A>(
    agent: Arc<A>,
    channel: Arc<VoiceChannel>,
    asr: Arc<dyn AsrEngine>,
    tts: Arc<dyn TtsEngine>,
) -> Result<(), VoiceSessionError>
where
    A: Agent + ?Sized + 'static,
{
    use crate::audio_in::AudioIn;
    use crate::audio_out::AudioOut;

    let input_device = channel.config().input_device.clone();

    eprintln!();
    eprintln!("aivyx voice — push-to-talk REPL");
    eprintln!("  Enter        : start recording (then Enter again to stop)");
    eprintln!("  quit + Enter : exit");
    eprintln!();

    loop {
        eprint!("[voice] press Enter to record (or `quit`): ");
        let _ = std::io::Write::flush(&mut std::io::stderr());
        let trimmed = read_stdin_line_trimmed();
        if trimmed == "quit" {
            eprintln!("[voice] exiting.");
            return Ok(());
        }

        // ----- Capture phase (synchronous, scoped) -----
        let samples = {
            let mut audio_in = AudioIn::new(input_device.as_deref()).map_err(|e| {
                VoiceSessionError::AudioDevice(format!("input: {e}"))
            })?;
            audio_in.start().map_err(|e| {
                VoiceSessionError::AudioCapture(format!("start: {e}"))
            })?;
            eprintln!(
                "[voice] recording at {} Hz / {} ch — press Enter to stop.",
                audio_in.src_rate(),
                audio_in.src_channels(),
            );
            let _ = read_stdin_line_trimmed();
            audio_in.stop().map_err(|e| {
                VoiceSessionError::AudioCapture(format!("stop: {e}"))
            })?;
            audio_in.take_samples_for_whisper().map_err(|e| {
                VoiceSessionError::AudioCapture(format!("drain: {e}"))
            })?
        };

        if samples.is_empty() {
            eprintln!("[voice] no audio captured — try again.");
            continue;
        }

        // ----- Turn dispatch (async; no audio handles live) -----
        let turn = match run_one_voice_turn(
            &agent,
            &channel,
            asr.as_ref(),
            tts.as_ref(),
            &samples,
        )
        .await
        {
            Ok(Some(t)) => t,
            Ok(None) => {
                eprintln!("[voice] (no speech detected, try again)");
                continue;
            }
            Err(e) => {
                eprintln!("[voice] error: {e}");
                continue;
            }
        };
        eprintln!("[voice] you said: {}", turn.transcribed);

        // ----- Playback phase (synchronous, scoped) -----
        if !turn.audio_chunks.is_empty() {
            let audio_out = AudioOut::new().map_err(|e| {
                VoiceSessionError::AudioDevice(format!("output: {e}"))
            })?;
            for chunk in &turn.audio_chunks {
                audio_out.play_audio(chunk).map_err(|e| {
                    VoiceSessionError::AudioPlayback(format!("queue: {e}"))
                })?;
            }
            audio_out.sleep_until_empty();
        }
    }
}

/// Read one line from stdin, blocking until the
/// user hits Enter, and return it trimmed of
/// surrounding whitespace. Errors from stdin
/// (closed pipe, etc.) collapse to an empty string;
/// the caller treats that the same as `quit`.
fn read_stdin_line_trimmed() -> String {
    let mut buf = String::new();
    match std::io::stdin().read_line(&mut buf) {
        Ok(0) => "quit".to_string(), // EOF — treat as quit
        Ok(_) => buf.trim().to_string(),
        Err(_) => "quit".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asr::AsrConfig;
    use crate::channel::VoiceChannelConfig;
    use crate::tts::{TtsAudio, TtsConfig};
    use async_trait::async_trait;

    // Test fixtures: stub ASR + TTS engines that
    // return scripted output, so `run_one_voice_turn`
    // can be exercised end-to-end without loading a
    // real model.

    struct StubAsr {
        result: Result<String, AsrError>,
    }

    impl StubAsr {
        fn ok(text: &str) -> Self {
            StubAsr {
                result: Ok(text.to_string()),
            }
        }
        fn empty() -> Self {
            StubAsr {
                result: Err(AsrError::Empty),
            }
        }
        fn boom() -> Self {
            StubAsr {
                result: Err(AsrError::Inference("simulated".to_string())),
            }
        }
    }

    #[async_trait]
    impl AsrEngine for StubAsr {
        async fn transcribe(&self, _samples: &[f32]) -> Result<String, AsrError> {
            match &self.result {
                Ok(t) => Ok(t.clone()),
                Err(AsrError::Empty) => Err(AsrError::Empty),
                Err(AsrError::Inference(m)) => Err(AsrError::Inference(m.clone())),
                Err(other) => Err(AsrError::Inference(format!("{other:?}"))),
            }
        }
    }

    struct StubTts;

    #[async_trait]
    impl TtsEngine for StubTts {
        async fn synthesize(&self, text: &str) -> Result<TtsAudio, TtsError> {
            // Produce a synthetic buffer whose length
            // encodes the text length, so tests can
            // assert on per-sentence chunk counts.
            Ok(TtsAudio::new(vec![0.0; text.len()], 22_050))
        }
        fn native_sample_rate(&self) -> u32 {
            22_050
        }
    }

    // A minimal Agent stub that streams scripted text
    // chunks into the channel during `turn` and
    // returns Completed.
    struct StubAgent {
        reply_chunks: Vec<&'static str>,
    }

    #[async_trait]
    impl Agent for StubAgent {
        fn id(&self) -> aivyx_core::AgentId {
            aivyx_core::AgentId::new()
        }
        fn capabilities(&self) -> &aivyx_capability::CapabilitySet {
            // Static empty set — the planner-free
            // stub doesn't dispatch tools.
            static EMPTY: std::sync::OnceLock<aivyx_capability::CapabilitySet> =
                std::sync::OnceLock::new();
            EMPTY.get_or_init(aivyx_capability::CapabilitySet::empty)
        }
        async fn turn(
            &self,
            _message: Message,
            channel: &dyn aivyx_core::ChannelContext,
        ) -> aivyx_core::TurnOutcome {
            use aivyx_core::StreamEvent;
            for chunk in &self.reply_chunks {
                let _ = channel.stream_event(StreamEvent::Text(chunk)).await;
            }
            aivyx_core::TurnOutcome::Completed {
                final_message: self.reply_chunks.concat(),
                tool_calls_made: 0,
                duration: std::time::Duration::from_millis(1),
            }
        }
    }

    fn channel() -> Arc<VoiceChannel> {
        Arc::new(VoiceChannel::new(VoiceChannelConfig::default()))
    }

    fn agent_replying(chunks: Vec<&'static str>) -> Arc<StubAgent> {
        Arc::new(StubAgent {
            reply_chunks: chunks,
        })
    }

    #[tokio::test]
    async fn run_one_voice_turn_full_loop_returns_synthesis() {
        // Two separate sentences in the reply so the
        // chunker produces two audio buffers — that's
        // what the channel relays to rodio sequentially.
        let agent = agent_replying(vec!["Hello there. ", "How can I help?"]);
        let ch = channel();
        let asr = StubAsr::ok("can you say hello");
        let tts = StubTts;
        let result = run_one_voice_turn(&agent, &ch, &asr, &tts, &[0.0; 1000])
            .await
            .unwrap()
            .expect("non-empty transcription");
        assert_eq!(result.transcribed, "can you say hello");
        assert_eq!(result.response_text, "Hello there. How can I help?");
        // Two sentences → two audio chunks.
        assert_eq!(result.audio_chunks.len(), 2);
        // Buffer drained for the next turn.
        assert_eq!(ch.peek_buffered_text(), "");
    }

    #[tokio::test]
    async fn run_one_voice_turn_empty_asr_returns_none() {
        let agent = agent_replying(vec!["should not be reached"]);
        let ch = channel();
        let asr = StubAsr::empty();
        let tts = StubTts;
        let result = run_one_voice_turn(&agent, &ch, &asr, &tts, &[0.0; 100])
            .await
            .unwrap();
        assert!(result.is_none(), "empty transcription must short-circuit");
        // Agent never ran → buffer stays empty.
        assert_eq!(ch.peek_buffered_text(), "");
    }

    #[tokio::test]
    async fn run_one_voice_turn_asr_failure_propagates() {
        let agent = agent_replying(vec![]);
        let ch = channel();
        let asr = StubAsr::boom();
        let tts = StubTts;
        let result = run_one_voice_turn(&agent, &ch, &asr, &tts, &[0.0; 100]).await;
        match result {
            Err(VoiceSessionError::Asr(AsrError::Inference(msg))) => {
                assert!(msg.contains("simulated"), "{msg}");
            }
            other => panic!("expected ASR Inference error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_one_voice_turn_drains_prior_buffer_defensively() {
        // If something somehow left text in the
        // buffer between turns (a `take_buffered_text`
        // miss, a test interleaving), the driver must
        // discard it before dispatching the next turn.
        let agent = agent_replying(vec!["Fresh reply."]);
        let ch = channel();
        // Pollute the buffer.
        use aivyx_core::{ChannelContext, StreamEvent};
        ch.stream_event(StreamEvent::Text("STALE LEAK"))
            .await
            .unwrap();
        assert_eq!(ch.peek_buffered_text(), "STALE LEAK");
        let asr = StubAsr::ok("hello");
        let tts = StubTts;
        let result = run_one_voice_turn(&agent, &ch, &asr, &tts, &[0.0; 100])
            .await
            .unwrap()
            .unwrap();
        // The fresh agent reply landed alone — the
        // stale text was drained.
        assert_eq!(result.response_text, "Fresh reply.");
    }

    #[tokio::test]
    async fn run_one_voice_turn_no_response_yields_zero_chunks() {
        // Agent emits no text (e.g. silent acknowledgment
        // or a tool-only turn). TTS pipeline is skipped
        // entirely; result.audio_chunks is empty.
        let agent = agent_replying(vec![]);
        let ch = channel();
        let asr = StubAsr::ok("status");
        let tts = StubTts;
        let result = run_one_voice_turn(&agent, &ch, &asr, &tts, &[0.0; 100])
            .await
            .unwrap()
            .unwrap();
        assert_eq!(result.response_text, "");
        assert!(result.audio_chunks.is_empty());
    }

    // A tiny config-shape sanity test so we know the
    // module surface compiles against the engines'
    // public types.
    #[test]
    fn asr_and_tts_configs_compose_into_voice_channel_config() {
        let cfg = VoiceChannelConfig {
            asr_engine: Some("whisper-rs".to_string()),
            tts_engine: Some("piper".to_string()),
            asr: AsrConfig {
                model_path: Some("/m/whisper.bin".into()),
                language: Some("en".to_string()),
                beam_size: Some(5),
            },
            tts: TtsConfig {
                voice_path: Some("/m/piper.onnx".into()),
                speaker_id: Some(0),
            },
            input_device: None,
            output_device: None,
            capture_debug_path: None,
        };
        assert_eq!(cfg.asr.beam_size, Some(5));
        assert_eq!(cfg.tts.speaker_id, Some(0));
    }
}
