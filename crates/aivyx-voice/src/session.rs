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
    assembled: Arc<Mutex<String>>,
) -> Result<Option<StreamingVoiceTurnResult>, VoiceSessionError>
where
    A: Agent + ?Sized + 'static,
{
    // Phase 152 — `assembled` is owned by the
    // caller now (was internal Arc in
    // Phase 138-151). The streaming PTT loop
    // holds a clone of this Arc so it can read
    // the partial response_text in the
    // mid-synthesis-abort path, when this
    // function's future is dropped before
    // returning normally.
    //
    // Tests + non-loop callers pass a fresh
    // `Arc::new(Mutex::new(String::new()))` and
    // discard their clone post-call — no
    // behavioral difference from the pre-152
    // shape.

    // Step 1 — transcribe.
    let transcribed = match asr.transcribe(captured_audio).await {
        Ok(text) => text,
        Err(AsrError::Empty) => return Ok(None),
        Err(e) => return Err(VoiceSessionError::Asr(e)),
    };

    // Step 2 — install the streaming sink. The
    // sink holds its own partial-sentence buffer
    // (separate from VoiceChannel's text_buffer,
    // which is for the non-streaming path); it
    // writes the full assembled response to the
    // externally-owned `assembled` Arc so
    // callers can read it post-turn (or
    // post-abort).
    use aivyx_core::ChannelContext;
    let _drained_prior = channel.take_buffered_text();
    channel.reset_cancellation();

    let pending: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
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
    //
    // Phase 154 — if the operator queued an image
    // via `/image <path>` at the start-of-iteration
    // prompt, fold it into the Message as
    // text_with_image so the vision-capable LLM
    // sees both the transcribed prompt and the
    // image bytes. Otherwise fall back to the
    // text-only Message::text shape.
    let message = match channel.take_pending_image() {
        Some((media_type, data)) => Message::text_with_image(
            channel.session_id(),
            transcribed.clone(),
            media_type,
            data,
        ),
        None => Message::text(channel.session_id(), transcribed.clone()),
    };
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
    // Phase 152 — reject out-of-range VAD config
    // values at function entry rather than
    // silently using nonsense values at runtime.
    if let Err(reason) = vad_cfg.validate() {
        return Err(VoiceSessionError::AudioDevice(format!(
            "configuration: {reason}"
        )));
    }
    let dwell_threshold = std::time::Duration::from_secs_f32(vad_cfg.dwell_secs);
    let min_speech = std::time::Duration::from_secs_f32(vad_cfg.min_speech_secs);
    let max_capture = std::time::Duration::from_secs_f32(vad_cfg.max_capture_secs);
    let poll_interval = std::time::Duration::from_millis(vad_cfg.poll_interval_ms);

    eprintln!();
    eprintln!("aivyx voice — push-to-talk REPL (streaming TTS + auto-stop)");
    eprintln!("  Enter             : start recording (then pause to dispatch)");
    eprintln!("  Enter mid-record  : abort the current capture");
    eprintln!("  Enter mid-reply   : abort the agent + playback");
    eprintln!("  quit + Enter      : exit (works at any time, including mid-reply)");
    eprintln!();

    // Phase 140 — long-lived async stdin reader.
    // Sends every line to the loop via an
    // unbounded mpsc. The start-of-iteration
    // prompt reads from it (replacing the
    // synchronous read_stdin_line_trimmed for
    // this loop variant) and the recording
    // tokio::select! races it against the
    // silence-detection poll so Enter mid-record
    // aborts cleanly.
    //
    // The reader task lives until the loop
    // returns; once we drop `line_rx`, the
    // reader's send fails and the task exits.
    let (line_tx, mut line_rx) = mpsc::unbounded_channel::<String>();
    let _stdin_task = tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let stdin = tokio::io::stdin();
        let mut reader = tokio::io::BufReader::new(stdin).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            if line_tx.send(line.trim().to_string()).is_err() {
                break;
            }
        }
    });

    loop {
        eprint!("[voice] press Enter to record (or `quit`, `/image <path>`): ");
        let _ = std::io::Write::flush(&mut std::io::stderr());
        let trimmed = match line_rx.recv().await {
            Some(line) => line,
            None => {
                // Reader task exited unexpectedly
                // (EOF on stdin, etc.) — treat as
                // quit.
                eprintln!("[voice] stdin closed, exiting.");
                return Ok(());
            }
        };
        if trimmed == "quit" {
            eprintln!("[voice] exiting.");
            return Ok(());
        }
        // Phase 154 — `/image <path>` queues an
        // image for the next recording iteration.
        // Operator types this instead of pressing
        // Enter; the loop loads the file +
        // infers media type from extension +
        // populates channel.pending_image, then
        // re-prompts for Enter (or another
        // command).
        if let Some(path) = trimmed.strip_prefix("/image ") {
            match load_image_for_attach(path.trim()) {
                Ok((media_type, data)) => {
                    eprintln!(
                        "[voice] image queued: {} ({} bytes, {})",
                        path.trim(),
                        data.len(),
                        media_type,
                    );
                    channel.set_pending_image(media_type, data);
                }
                Err(reason) => {
                    eprintln!("[voice] image attach failed: {reason}");
                }
            }
            continue;
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
            "[voice] recording at {} Hz / {} ch — pause for {:.1}s or press Enter to dispatch.",
            audio_in.src_rate(),
            audio_in.src_channels(),
            vad_cfg.dwell_secs,
        );
        let mut stopped_reason = "silence";
        loop {
            tokio::select! {
                _ = tokio::time::sleep(poll_interval) => {
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
                line = line_rx.recv() => {
                    // Operator pressed Enter
                    // mid-recording (or stdin
                    // closed). Either way → manual
                    // stop. We dispatch whatever
                    // samples we've collected.
                    stopped_reason = match line {
                        Some(_) => "manual",
                        None => "stdin-closed",
                    };
                    break;
                }
            }
        }
        audio_in.stop().map_err(|e| {
            VoiceSessionError::AudioCapture(format!("stop: {e}"))
        })?;
        let samples = audio_in.take_samples_for_whisper().map_err(|e| {
            VoiceSessionError::AudioCapture(format!("drain: {e}"))
        })?;
        match stopped_reason {
            "max-capture" => {
                eprintln!(
                    "[voice] hit {:.0}s max-capture cap — dispatching what we have.",
                    vad_cfg.max_capture_secs,
                );
            }
            "manual" => {
                if samples.is_empty() {
                    eprintln!("[voice] aborted with no audio — skipping turn.");
                } else {
                    eprintln!("[voice] aborted by operator — dispatching partial.");
                }
            }
            "stdin-closed" => {
                eprintln!("[voice] stdin closed mid-recording, exiting.");
                return Ok(());
            }
            _ => {} // "silence" — normal auto-stop, no extra log
        }
        drop(audio_in);

        if samples.is_empty() {
            eprintln!("[voice] no audio captured — try again.");
            continue;
        }

        // Phase 146 — drain any stale Enter
        // presses that landed between the end of
        // the recording phase and the start of
        // synthesis. Otherwise a rapid double-
        // Enter during recording would auto-abort
        // the synthesis.
        while line_rx.try_recv().is_ok() {}

        // ----- Streaming pipeline setup -----
        // sentence_tx feeds the consumer task;
        // sentence_rx pulls one sentence at a
        // time and synthesizes + plays serially.
        // Phase 146 — abort_tx/abort_rx pair lets
        // the loop signal the consumer "stop
        // immediately, don't drain" when the
        // operator interrupts mid-synthesis.
        let (sentence_tx, mut sentence_rx) = mpsc::unbounded_channel::<String>();
        let (abort_tx, mut abort_rx) = mpsc::channel::<()>(1);
        let tts_for_consumer = Arc::clone(&tts);
        // Phase 152 — assembled text Arc owned by
        // the loop; cloned into the streaming-turn
        // call. On normal completion the function
        // returns the assembled text in the
        // result; on abort the function's future
        // drops without returning, but the loop's
        // Arc clone keeps the partial text alive
        // so we can surface "you said: ..." even
        // on interrupted replies.
        let turn_assembled: Arc<Mutex<String>> =
            Arc::new(Mutex::new(String::new()));
        let consumer = tokio::spawn(async move {
            let audio_out = match AudioOut::new() {
                Ok(out) => out,
                Err(e) => {
                    return Err(VoiceSessionError::AudioDevice(format!(
                        "output: {e}"
                    )));
                }
            };
            loop {
                tokio::select! {
                    biased;
                    // Phase 152 — aggressive
                    // abort: don't call
                    // stop_playback (which uses
                    // rodio's Player::clear that
                    // calls sleep_until_end and
                    // lets the current sample
                    // finish before silence).
                    // Just return — AudioOut
                    // drops on return, the cpal
                    // Stream inside it drops,
                    // audio output dies within
                    // OS buffer time (typically
                    // ~10ms instead of the
                    // remainder-of-current-word
                    // latency Phase 146 had).
                    _ = abort_rx.recv() => {
                        return Ok::<(), VoiceSessionError>(());
                    }
                    maybe_sentence = sentence_rx.recv() => {
                        match maybe_sentence {
                            None => break,
                            Some(sentence) => {
                                match tts_for_consumer.synthesize(&sentence).await {
                                    Ok(audio) if !audio.is_empty() => {
                                        if let Err(e) = audio_out.play_audio(&audio) {
                                            return Err(VoiceSessionError::AudioPlayback(
                                                format!("queue: {e}"),
                                            ));
                                        }
                                    }
                                    Ok(_) => {} // empty synthesis — skip
                                    Err(TtsError::Input(_)) => {
                                        // Empty-after-trim — skip silently.
                                    }
                                    Err(e) => return Err(VoiceSessionError::Tts(e)),
                                }
                            }
                        }
                    }
                }
            }
            audio_out.sleep_until_empty();
            Ok::<(), VoiceSessionError>(())
        });

        // ----- Turn dispatch (streaming) with
        // ----- mid-synthesis abort race
        //
        // Phase 146 — race the streaming turn
        // future against `line_rx.recv()`. If
        // the operator presses Enter (or
        // anything that lands as a stdin line)
        // before the turn completes, we cancel
        // the agent + signal the consumer to
        // stop playback + iterate or quit.
        enum TurnResolution {
            Completed(Option<StreamingVoiceTurnResult>),
            Failed(VoiceSessionError),
            AbortedContinue,
            AbortedQuit,
        }

        let resolution = tokio::select! {
            result = run_one_voice_turn_streaming(
                &agent,
                &channel,
                asr.as_ref(),
                &samples,
                sentence_tx,
                Arc::clone(&turn_assembled),
            ) => {
                match result {
                    Ok(t) => TurnResolution::Completed(t),
                    Err(e) => TurnResolution::Failed(e),
                }
            }
            line = line_rx.recv() => {
                use aivyx_core::ChannelContext;
                channel.cancel_inflight();
                // Phase 152 — clear the channel's
                // text sink so its references to
                // the assembled Arc drop. We then
                // own the only live clone of the
                // assembled Arc and can read it
                // freely below.
                channel.clear_text_sink();
                let _ = abort_tx.send(()).await;
                let raw = line.unwrap_or_default();
                if raw == "quit" {
                    TurnResolution::AbortedQuit
                } else {
                    TurnResolution::AbortedContinue
                }
            }
        };

        // Phase 152 — read the assembled partial
        // text for use in the abort paths.
        let partial_text: String = turn_assembled
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default();

        match resolution {
            TurnResolution::Completed(Some(t)) => {
                // Wait for the consumer to finish
                // playback before re-prompting. By
                // this point sentence_tx has been
                // dropped (moved into the streaming
                // turn) so the consumer's
                // sentence_rx will close and the
                // task naturally winds down.
                match consumer.await {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        eprintln!("[voice] playback error: {e}");
                    }
                    Err(join_err) => {
                        eprintln!("[voice] consumer task panicked: {join_err}");
                    }
                }
                eprintln!("[voice] you said: {}", t.transcribed);
            }
            TurnResolution::Completed(None) => {
                eprintln!("[voice] (no speech detected, try again)");
                let _ = consumer.await;
            }
            TurnResolution::Failed(e) => {
                eprintln!("[voice] error: {e}");
                let _ = consumer.await;
            }
            TurnResolution::AbortedContinue => {
                eprintln!("[voice] aborted by operator — stopped agent + playback.");
                if !partial_text.trim().is_empty() {
                    eprintln!(
                        "[voice] agent had said: {}",
                        partial_text.trim()
                    );
                }
                let _ = consumer.await;
            }
            TurnResolution::AbortedQuit => {
                eprintln!("[voice] aborted by operator + exiting.");
                if !partial_text.trim().is_empty() {
                    eprintln!(
                        "[voice] agent had said: {}",
                        partial_text.trim()
                    );
                }
                let _ = consumer.await;
                return Ok(());
            }
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

/// Phase 154 — load + media-type-classify an
/// image file for attach. Returns the inferred
/// media type + the raw bytes on success;
/// operator-readable error string on any
/// failure (unknown extension, file not found,
/// IO error, empty file).
fn load_image_for_attach(path: &str) -> Result<(String, Vec<u8>), String> {
    if path.is_empty() {
        return Err("path must not be empty".to_string());
    }
    let media_type = infer_image_media_type(path)?;
    let data = std::fs::read(path)
        .map_err(|e| format!("read {path:?}: {e}"))?;
    if data.is_empty() {
        return Err(format!("file {path:?} is empty"));
    }
    Ok((media_type.to_string(), data))
}

/// Phase 154 — infer the image media type
/// from a file's extension. Pure substrate;
/// returns the canonical `image/<format>` MIME
/// string on a known extension, or a clear
/// error otherwise.
///
/// Phase 154 MVP covers the four
/// operator-typical formats (PNG, JPEG, GIF,
/// WebP). PDF/SVG/TIFF/etc. error out — Phase
/// 155+ candidate if surfaces.
fn infer_image_media_type(path: &str) -> Result<&'static str, String> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .ok_or_else(|| format!("no file extension in path {path:?}"))?;
    match ext.as_str() {
        "png" => Ok("image/png"),
        "jpg" | "jpeg" => Ok("image/jpeg"),
        "gif" => Ok("image/gif"),
        "webp" => Ok("image/webp"),
        other => Err(format!(
            "unsupported image extension {other:?}; \
             Phase 154 supports png / jpg / jpeg / gif / webp"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asr::AsrConfig;
    use crate::channel::VoiceChannelConfig;
    use crate::tts::{TtsAudio, TtsConfig};
    use async_trait::async_trait;

    // ---- Phase 154 — image media type inference + load ----

    #[test]
    fn infer_image_media_type_png() {
        assert_eq!(infer_image_media_type("foo.png").unwrap(), "image/png");
        assert_eq!(infer_image_media_type("/abs/path/IMG.PNG").unwrap(), "image/png");
    }

    #[test]
    fn infer_image_media_type_jpeg_variants() {
        assert_eq!(infer_image_media_type("a.jpg").unwrap(), "image/jpeg");
        assert_eq!(infer_image_media_type("b.jpeg").unwrap(), "image/jpeg");
        assert_eq!(infer_image_media_type("C.JPG").unwrap(), "image/jpeg");
    }

    #[test]
    fn infer_image_media_type_gif_and_webp() {
        assert_eq!(infer_image_media_type("a.gif").unwrap(), "image/gif");
        assert_eq!(infer_image_media_type("b.webp").unwrap(), "image/webp");
    }

    #[test]
    fn infer_image_media_type_unsupported_extension_rejects() {
        let err = infer_image_media_type("a.pdf").unwrap_err();
        assert!(err.contains("unsupported"), "{err}");
        let err = infer_image_media_type("a.svg").unwrap_err();
        assert!(err.contains("unsupported"), "{err}");
    }

    #[test]
    fn infer_image_media_type_no_extension_rejects() {
        let err = infer_image_media_type("README").unwrap_err();
        assert!(err.contains("no file extension"), "{err}");
    }

    #[test]
    fn load_image_empty_path_rejected() {
        let err = load_image_for_attach("").unwrap_err();
        assert!(err.contains("path must not be empty"), "{err}");
    }

    #[test]
    fn load_image_missing_file_rejected() {
        let err = load_image_for_attach("/nonexistent/file.png").unwrap_err();
        assert!(err.contains("read"), "{err}");
    }

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
        let assembled = Arc::new(std::sync::Mutex::new(String::new()));
        let result = run_one_voice_turn_streaming(&agent, &ch, &asr, &[0.0; 1000], tx, assembled)
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
        let assembled = Arc::new(std::sync::Mutex::new(String::new()));
        let result = run_one_voice_turn_streaming(&agent, &ch, &asr, &[0.0; 100], tx, assembled)
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
        let assembled = Arc::new(std::sync::Mutex::new(String::new()));
        let result = run_one_voice_turn_streaming(&agent, &ch, &asr, &[0.0; 100], tx, assembled)
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
