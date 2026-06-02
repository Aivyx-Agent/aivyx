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

use std::sync::{Arc, Mutex};

use aivyx_core::{Agent, Message};
use thiserror::Error;
use tokio::sync::mpsc;

use crate::asr::{AsrEngine, AsrError};
use crate::channel::VoiceChannel;
use crate::tts::{chunk_into_sentences, drain_complete_sentences, TtsEngine, TtsError};

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

/// Phase 138 — streaming variant of
/// [`run_one_voice_turn`].
///
/// Instead of waiting for the whole agent reply
/// before synthesizing, this version drains
/// complete sentences from the streaming text
/// pipeline as they arrive and pushes them
/// (one at a time, in order) into `sentence_tx`.
/// A separate consumer task (owned by
/// [`run_push_to_talk_loop_streaming`]) pulls
/// sentences from the matching receiver, calls
/// `tts.synthesize`, and queues the audio for
/// playback — all while the LLM is still
/// generating later sentences.
///
/// After `agent.turn` completes, any leftover
/// partial sentence (text that arrived but didn't
/// terminate before the agent stopped emitting)
/// is flushed as one final entry. This handles
/// the common case where the agent's last sentence
/// ends at EOF without a trailing space.
///
/// The caller is responsible for:
/// - Owning the `sentence_rx` and consuming it
///   (typically a `tokio::spawn`'d consumer task).
/// - Dropping `sentence_tx` after this returns so
///   the consumer's `recv().await` returns `None`
///   and the consumer can exit cleanly.
pub async fn run_one_voice_turn_streaming<A>(
    agent: &Arc<A>,
    channel: &Arc<VoiceChannel>,
    asr: &dyn AsrEngine,
    captured_audio: &[f32],
    sentence_tx: mpsc::UnboundedSender<String>,
) -> Result<Option<StreamingVoiceTurnResult>, VoiceSessionError>
where
    A: Agent + ?Sized + 'static,
{
    // Step 1 — transcribe.
    let transcribed = match asr.transcribe(captured_audio).await {
        Ok(text) => text,
        Err(AsrError::Empty) => return Ok(None),
        Err(e) => return Err(VoiceSessionError::Asr(e)),
    };

    // Step 2 — install the streaming sink. The
    // sink holds its own partial-sentence buffer
    // (separate from VoiceChannel's text_buffer,
    // which is for the non-streaming path) and a
    // mirror of the full assembled response so we
    // can surface `response_text` post-turn.
    use aivyx_core::ChannelContext;
    let _drained_prior = channel.take_buffered_text();
    channel.reset_cancellation();

    let pending: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let assembled: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    {
        let pending = Arc::clone(&pending);
        let assembled = Arc::clone(&assembled);
        let tx = sentence_tx.clone();
        channel.set_text_sink(move |chunk: &str| {
            // Mirror into the full-response buffer
            // unconditionally — even fragments
            // count toward what the agent "said".
            {
                let mut a = assembled.lock().expect("assembled poisoned");
                a.push_str(chunk);
            }
            // Drain complete sentences and forward.
            let mut p = pending.lock().expect("pending poisoned");
            p.push_str(chunk);
            for s in drain_complete_sentences(&mut p) {
                // If the receiver has been dropped
                // (consumer task panicked or
                // exited), forwarding fails — the
                // sink can't do anything useful
                // about it, so we drop the chunk.
                // The agent.turn keeps running;
                // the operator hears whatever
                // already made it into playback.
                let _ = tx.send(s);
            }
        });
    }

    // Step 3 — dispatch the turn. As text chunks
    // arrive, the sink fires + drains sentences +
    // forwards to the consumer.
    let message = Message::text(channel.session_id(), transcribed.clone());
    let outcome = agent.turn(message, channel.as_ref()).await;

    // Step 4 — unhook the sink. Anything still in
    // the pending buffer is the agent's final
    // partial sentence; flush it as one final
    // entry so playback isn't missing the tail.
    channel.clear_text_sink();
    let leftover = {
        let mut p = pending.lock().expect("pending poisoned");
        std::mem::take(&mut *p)
    };
    let leftover_trimmed = leftover.trim().to_string();
    if !leftover_trimmed.is_empty() {
        let _ = sentence_tx.send(leftover_trimmed);
    }

    let response_text = assembled.lock().expect("assembled poisoned").clone();
    Ok(Some(StreamingVoiceTurnResult {
        transcribed,
        response_text,
        outcome,
    }))
}

/// Result of one streaming push-to-talk iteration.
///
/// Unlike [`VoiceTurnResult`], there are no
/// `audio_chunks` — the streaming consumer task
/// has already synthesized + played each sentence
/// as it arrived. The caller uses `response_text`
/// for diagnostic logging or audit only.
#[derive(Debug)]
pub struct StreamingVoiceTurnResult {
    /// What the operator said (Whisper output).
    pub transcribed: String,
    /// What the agent replied — the full assembled
    /// text reconstructed from streaming chunks.
    /// Diagnostic only; audio playback already
    /// happened.
    pub response_text: String,
    /// The full `TurnOutcome` from the agent.
    pub outcome: aivyx_core::TurnOutcome,
}

/// Phase 138 — streaming push-to-talk loop.
///
/// Like [`run_push_to_talk_loop`] but pipelines
/// the LLM stream into TTS on sentence boundaries.
/// The operator hears sentence one of the agent's
/// reply while the LLM is still generating later
/// sentences — typically a 5-10x latency-to-first-
/// audio win on long replies.
///
/// ## Loop shape
///
/// Same prompt/record/dispatch shape as the
/// non-streaming variant. Per iteration:
///
/// 1. Prompt + record (synchronous prelude).
/// 2. Create a fresh `tokio::mpsc::UnboundedChannel`
///    for sentences.
/// 3. Spawn a **serial consumer task** that loops
///    on `sentence_rx.recv().await`, synthesizes
///    each sentence with `tts.synthesize`, and
///    plays via a per-iteration `AudioOut`.
/// 4. Dispatch to [`run_one_voice_turn_streaming`].
///    As the agent emits text chunks, complete
///    sentences flush to the consumer; the
///    consumer synthesizes + plays them in order.
/// 5. After the turn returns, drop the
///    `sentence_tx` so the consumer's recv()
///    returns `None`; await the consumer task
///    so playback drains before the next
///    iteration prompts.
/// 6. Loop.
///
/// ## Why serial consumer
///
/// Tempting to spawn one TTS task per sentence
/// for parallelism, but synthesis would race with
/// playback ordering. A serial consumer guarantees
/// in-order playback at the cost of theoretical
/// parallel synthesis. In practice Piper inference
/// is fast enough that synthesis-of-sentence-N+1
/// rarely happens before playback of
/// sentence-N has begun.
///
/// ## Send constraints
///
/// `AudioOut` on Linux + Windows is `Send` and
/// crosses the `tokio::spawn` boundary cleanly.
/// On macOS, `cpal::Stream` is `!Send` — the
/// consumer task on that platform will fail to
/// compile. Phase 139+ candidate: a macOS-specific
/// variant that runs the consumer on the main
/// runtime thread.
pub async fn run_push_to_talk_loop_streaming<A>(
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

    // Phase 140 — auto-stop thresholds now sourced
    // from `channel.config().vad`, operator-tunable
    // via the `[voice.vad]` TOML section. Defaults
    // match Phase 139's previously-hardcoded values
    // so operators who don't set the section keep
    // the same behavior.
    let vad_cfg = channel.config().vad.clone();
    let dwell_threshold = std::time::Duration::from_secs_f32(vad_cfg.dwell_secs);
    let min_speech = std::time::Duration::from_secs_f32(vad_cfg.min_speech_secs);
    let max_capture = std::time::Duration::from_secs_f32(vad_cfg.max_capture_secs);
    let poll_interval = std::time::Duration::from_millis(vad_cfg.poll_interval_ms);

    eprintln!();
    eprintln!("aivyx voice — push-to-talk REPL (streaming TTS + auto-stop)");
    eprintln!("  Enter        : start recording (then pause to dispatch)");
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

        // ----- Capture phase — async polling for auto-stop -----
        // AudioIn must NOT cross an .await boundary
        // (cpal::Stream is !Send on macOS). We
        // build it, drive the polling loop in
        // place, then drain — all inside one
        // synchronous block punctuated by short
        // tokio::time::sleep awaits.
        //
        // The cpal callback runs on the audio
        // thread and feeds both the capture buffer
        // and the silence detector in lockstep; we
        // just observe the detector state.
        let mut audio_in =
            AudioIn::new_with_vad_config(input_device.as_deref(), &vad_cfg).map_err(|e| {
                VoiceSessionError::AudioDevice(format!("input: {e}"))
            })?;
        audio_in.start().map_err(|e| {
            VoiceSessionError::AudioCapture(format!("start: {e}"))
        })?;
        eprintln!(
            "[voice] recording at {} Hz / {} ch — pause for {:.1}s to dispatch.",
            audio_in.src_rate(),
            audio_in.src_channels(),
            vad_cfg.dwell_secs,
        );
        let mut stopped_reason = "silence";
        loop {
            tokio::time::sleep(poll_interval).await;
            let total = audio_in.total_recorded();
            let dwell = audio_in.silence_dwell();
            if total >= max_capture {
                stopped_reason = "max-capture";
                break;
            }
            if total >= min_speech && dwell >= dwell_threshold {
                break;
            }
        }
        audio_in.stop().map_err(|e| {
            VoiceSessionError::AudioCapture(format!("stop: {e}"))
        })?;
        let samples = audio_in.take_samples_for_whisper().map_err(|e| {
            VoiceSessionError::AudioCapture(format!("drain: {e}"))
        })?;
        if stopped_reason == "max-capture" {
            eprintln!(
                "[voice] hit {:.0}s max-capture cap — dispatching what we have.",
                vad_cfg.max_capture_secs,
            );
        }
        drop(audio_in);

        if samples.is_empty() {
            eprintln!("[voice] no audio captured — try again.");
            continue;
        }

        // ----- Streaming pipeline setup -----
        // sentence_tx feeds the consumer task;
        // sentence_rx pulls one sentence at a
        // time and synthesizes + plays serially.
        let (sentence_tx, mut sentence_rx) = mpsc::unbounded_channel::<String>();
        let tts_for_consumer = Arc::clone(&tts);
        let consumer = tokio::spawn(async move {
            let audio_out = match AudioOut::new() {
                Ok(out) => out,
                Err(e) => {
                    return Err(VoiceSessionError::AudioDevice(format!(
                        "output: {e}"
                    )));
                }
            };
            while let Some(sentence) = sentence_rx.recv().await {
                match tts_for_consumer.synthesize(&sentence).await {
                    Ok(audio) if !audio.is_empty() => {
                        if let Err(e) = audio_out.play_audio(&audio) {
                            return Err(VoiceSessionError::AudioPlayback(format!(
                                "queue: {e}"
                            )));
                        }
                    }
                    Ok(_) => {} // empty synthesis — skip
                    Err(TtsError::Input(_)) => {
                        // Empty-after-trim — skip silently.
                    }
                    Err(e) => return Err(VoiceSessionError::Tts(e)),
                }
            }
            audio_out.sleep_until_empty();
            Ok::<(), VoiceSessionError>(())
        });

        // ----- Turn dispatch (streaming) -----
        let turn = match run_one_voice_turn_streaming(
            &agent,
            &channel,
            asr.as_ref(),
            &samples,
            sentence_tx,
        )
        .await
        {
            Ok(Some(t)) => Some(t),
            Ok(None) => {
                eprintln!("[voice] (no speech detected, try again)");
                // The sentence_tx is already dropped
                // (moved into the call) — the
                // consumer drains and exits.
                let _ = consumer.await;
                continue;
            }
            Err(e) => {
                eprintln!("[voice] error: {e}");
                let _ = consumer.await;
                continue;
            }
        };

        // Wait for the consumer to finish playback
        // before re-prompting. By this point
        // sentence_tx has been dropped (it was
        // moved into run_one_voice_turn_streaming
        // and out of scope), so recv() will return
        // None once the queue drains.
        match consumer.await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                eprintln!("[voice] playback error: {e}");
                continue;
            }
            Err(join_err) => {
                eprintln!("[voice] consumer task panicked: {join_err}");
                continue;
            }
        }

        if let Some(t) = turn {
            eprintln!("[voice] you said: {}", t.transcribed);
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

    // -------- Phase 138 — streaming variant --------

    #[tokio::test]
    async fn run_one_voice_turn_streaming_flushes_sentences_in_order() {
        // Agent emits the reply across multiple
        // text chunks, with sentence terminators
        // landing in the middle of chunks (the
        // real-world shape). The streaming driver
        // must forward complete sentences to the
        // consumer in order; the final partial
        // sentence (no trailing space) must flush
        // as a leftover after agent.turn returns.
        let agent = agent_replying(vec![
            "Hello there",       // partial
            ". How are you",     // completes 1, starts 2 (partial)
            " today? I'm",       // completes 2, starts 3 (partial)
            " great! Bye now",   // completes 3, starts 4 (partial — leftover)
        ]);
        let ch = channel();
        let asr = StubAsr::ok("hi");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let collected: Arc<std::sync::Mutex<Vec<String>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let collected_for_task = Arc::clone(&collected);
        let consumer = tokio::spawn(async move {
            while let Some(s) = rx.recv().await {
                collected_for_task.lock().unwrap().push(s);
            }
        });
        let result = run_one_voice_turn_streaming(&agent, &ch, &asr, &[0.0; 1000], tx)
            .await
            .unwrap()
            .expect("non-empty transcription");
        consumer.await.unwrap();

        let got = collected.lock().unwrap().clone();
        assert_eq!(
            got,
            vec![
                "Hello there.".to_string(),
                "How are you today?".to_string(),
                "I'm great!".to_string(),
                "Bye now".to_string(), // leftover partial
            ],
            "streaming consumer must see sentences in order, plus the final partial fragment as leftover",
        );
        assert_eq!(result.transcribed, "hi");
        assert_eq!(
            result.response_text, "Hello there. How are you today? I'm great! Bye now",
            "response_text mirrors the full assembled text",
        );
    }

    #[tokio::test]
    async fn run_one_voice_turn_streaming_empty_response_yields_no_sentences() {
        // Agent emits no text. The streaming
        // pipeline never forwards anything; the
        // consumer's rx closes empty.
        let agent = agent_replying(vec![]);
        let ch = channel();
        let asr = StubAsr::ok("status");
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let collected: Arc<std::sync::Mutex<Vec<String>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let collected_for_task = Arc::clone(&collected);
        let consumer = tokio::spawn(async move {
            while let Some(s) = rx.recv().await {
                collected_for_task.lock().unwrap().push(s);
            }
        });
        let result = run_one_voice_turn_streaming(&agent, &ch, &asr, &[0.0; 100], tx)
            .await
            .unwrap()
            .expect("non-empty transcription");
        consumer.await.unwrap();

        assert!(
            collected.lock().unwrap().is_empty(),
            "no text chunks → no sentences forwarded"
        );
        assert_eq!(result.response_text, "");
    }

    #[tokio::test]
    async fn run_one_voice_turn_streaming_empty_asr_returns_none() {
        // Same as the non-streaming variant: an
        // empty ASR transcription short-circuits
        // and the streaming pipeline is never
        // installed.
        let agent = agent_replying(vec!["should not be reached"]);
        let ch = channel();
        let asr = StubAsr::empty();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let collected: Arc<std::sync::Mutex<Vec<String>>> =
            Arc::new(std::sync::Mutex::new(Vec::new()));
        let collected_for_task = Arc::clone(&collected);
        let consumer = tokio::spawn(async move {
            while let Some(s) = rx.recv().await {
                collected_for_task.lock().unwrap().push(s);
            }
        });
        let result = run_one_voice_turn_streaming(&agent, &ch, &asr, &[0.0; 100], tx)
            .await
            .unwrap();
        consumer.await.unwrap();
        assert!(result.is_none());
        assert!(collected.lock().unwrap().is_empty());
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
            vad: Default::default(),
        };
        assert_eq!(cfg.asr.beam_size, Some(5));
        assert_eq!(cfg.tts.speaker_id, Some(0));
    }
}
