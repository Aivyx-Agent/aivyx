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
    // Phase 156 — pending_images is now a Vec
    // (was Option pre-156). Empty Vec → text-only
    // message. Length 1 → text+image. Length 2+ →
    // MessageContent::Mixed with one Text part
    // + N Image parts.
    let pending_imgs = channel.take_pending_images();
    let message = if pending_imgs.is_empty() {
        Message::text(channel.session_id(), transcribed.clone())
    } else {
        // Phase 163 / amendment A13 — any pending
        // image whose media_type is a document
        // type (currently just application/pdf)
        // routes through ContentPart::Document
        // instead of ContentPart::Image. Mixed
        // is the only shape that supports text +
        // document together, so we always build
        // Mixed when there's at least one pending
        // image (Phase 156's single-image shortcut
        // is no longer reachable without a media-
        // type check; collapsing into one path
        // keeps the routing logic local).
        use aivyx_core::{ContentPart, MessageContent, MessageId};
        use std::time::SystemTime;
        let mut parts: Vec<ContentPart> =
            Vec::with_capacity(pending_imgs.len() + 1);
        parts.push(ContentPart::Text(transcribed.clone()));
        for (media_type, data) in pending_imgs {
            parts.push(content_part_for_attachment(media_type, data));
        }
        Message {
            id: MessageId::new(),
            session_id: channel.session_id(),
            content: MessageContent::Mixed(parts),
            received_at: SystemTime::now(),
        }
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
        if let Some(rest) = trimmed.strip_prefix("/image ") {
            let global_cfg = &channel.config().image;
            let (path_or_url, preset_name) = match parse_image_command_args(rest.trim()) {
                Ok(parts) => parts,
                Err(reason) => {
                    eprintln!("[voice] /image: {reason}");
                    continue;
                }
            };

            // Phase 165 — resolve --headers <preset> to a
            // concrete HashMap by cloning the global cfg
            // and swapping in the preset's url_headers.
            // When no preset specified, use the global
            // cfg as-is.
            let effective_cfg;
            let cfg_ref: &VoiceImageConfig = match preset_name {
                None => global_cfg,
                Some(name) => match global_cfg.url_header_presets.get(name) {
                    Some(preset) => {
                        effective_cfg = VoiceImageConfig {
                            url_headers: preset.clone(),
                            ..global_cfg.clone()
                        };
                        &effective_cfg
                    }
                    None => {
                        eprintln!(
                            "[voice] /image: unknown header preset {name:?}; \
                             available presets: {:?}",
                            global_cfg
                                .url_header_presets
                                .keys()
                                .collect::<Vec<_>>()
                        );
                        continue;
                    }
                },
            };

            match load_image_for_attach(path_or_url, cfg_ref).await {
                Ok((media_type, data)) => {
                    eprintln!(
                        "[voice] image queued: {} ({} bytes, {})",
                        path_or_url,
                        data.len(),
                        media_type,
                    );
                    channel.append_pending_image(media_type, data);
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

/// Phase 156 — default client-side size cap on
/// queued images. Matches the existing Drive
/// inline cap from Phase 129
/// (`CONTENT_INLINE_CAP_BYTES`) so the agent
/// sees consistent size limits across
/// substrates. Phase 161 promotes this from a
/// hardcoded constant to the default of
/// `VoiceImageConfig::size_cap_mb`.
pub const DEFAULT_IMAGE_SIZE_CAP_MB: usize = 10;

/// Phase 161 — operator-tunable knobs for
/// `/image` attach. Threaded into
/// `load_image_for_attach` from the session
/// driver via
/// [`crate::channel::VoiceChannelConfig::image`].
///
/// Deserialized from
/// ```toml
/// [voice.image]
/// size_cap_mb = 10
/// url_timeout_secs = 30
/// head_precheck = true
/// ```
/// All three fields default to the Phase 156
/// behavior (10MB cap, 30s timeout, HEAD
/// pre-check enabled) so operators with no
/// `[voice.image]` block see unchanged behavior.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct VoiceImageConfig {
    /// Maximum image size in megabytes.
    /// Default 10. No upper bound — operator
    /// trust.
    #[serde(default = "VoiceImageConfig::default_size_cap_mb")]
    pub size_cap_mb: usize,
    /// URL-fetch request timeout in seconds.
    /// Default 30. Per-request (full body),
    /// not per-byte. Phase 162+ candidate for
    /// a stalled-read timeout.
    #[serde(default = "VoiceImageConfig::default_url_timeout_secs")]
    pub url_timeout_secs: u64,
    /// Whether to issue a HEAD request before
    /// the GET so Content-Length can be
    /// size-checked before download. Default
    /// true. Operators with HEAD-hostile
    /// origins set false.
    #[serde(default = "VoiceImageConfig::default_head_precheck")]
    pub head_precheck: bool,
    /// Phase 162 — operator-supplied HTTP
    /// headers attached to every `/image <url>`
    /// fetch (both HEAD and GET). Useful for
    /// Authorization / Cookie / Origin against
    /// authenticated origins. Defaults to an
    /// empty map (no headers) so existing
    /// behavior is preserved. Configured via:
    /// ```toml
    /// [voice.image.url_headers]
    /// Authorization = "Bearer xxx"
    /// Cookie        = "session=yyy"
    /// ```
    #[serde(default)]
    pub url_headers: std::collections::HashMap<String, String>,
    /// Phase 165 — named per-URL header
    /// presets. Operator runs
    /// `/image <url> --headers <preset-name>`
    /// to select a specific bundle instead of
    /// the global `url_headers` map.
    /// Configured via:
    /// ```toml
    /// [voice.image.url_header_presets.work]
    /// Authorization = "Bearer work-token"
    ///
    /// [voice.image.url_header_presets.personal]
    /// Cookie = "session=personal"
    /// ```
    /// Defaults to empty map; operators with
    /// no presets continue to use the global
    /// `url_headers` block.
    #[serde(default)]
    pub url_header_presets:
        std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    /// Phase 166 — number of retries on
    /// transient URL-fetch errors (timeout,
    /// connection refused). Default 0 = no
    /// retry (preserves Phase 161 behavior).
    /// Each retry doubles the backoff from
    /// `url_retry_backoff_ms`.
    #[serde(default)]
    pub url_retry_count: u32,
    /// Phase 166 — base backoff in
    /// milliseconds before the first retry.
    /// Default 500ms. Doubles per retry
    /// (exponential). Has no effect when
    /// `url_retry_count` is 0.
    #[serde(default = "VoiceImageConfig::default_url_retry_backoff_ms")]
    pub url_retry_backoff_ms: u64,
}

impl Default for VoiceImageConfig {
    fn default() -> Self {
        Self {
            size_cap_mb: Self::default_size_cap_mb(),
            url_timeout_secs: Self::default_url_timeout_secs(),
            head_precheck: Self::default_head_precheck(),
            url_headers: std::collections::HashMap::new(),
            url_header_presets: std::collections::HashMap::new(),
            url_retry_count: 0,
            url_retry_backoff_ms: Self::default_url_retry_backoff_ms(),
        }
    }
}

impl VoiceImageConfig {
    fn default_size_cap_mb() -> usize {
        DEFAULT_IMAGE_SIZE_CAP_MB
    }
    fn default_url_timeout_secs() -> u64 {
        30
    }
    fn default_head_precheck() -> bool {
        true
    }
    fn default_url_retry_backoff_ms() -> u64 {
        500
    }
    /// Bytes-form of `size_cap_mb` for the
    /// substrate-tier comparisons in
    /// `load_image_for_attach` and
    /// `fetch_image_url`.
    pub fn size_cap_bytes(&self) -> usize {
        self.size_cap_mb.saturating_mul(1024 * 1024)
    }
}

/// Phase 154 — load + media-type-classify an
/// image for attach. Returns the inferred
/// media type + the raw bytes on success;
/// operator-readable error string on any
/// failure (unknown extension, file not found,
/// IO error, empty file, oversized file).
///
/// Phase 156:
/// - Enforces the size cap (Phase 161-tunable)
///   after read.
/// - When `path_or_url` starts with `http://`
///   or `https://`, fetches via reqwest. Infers
///   media type from `Content-Type` header
///   (falling back to extension if header is
///   absent or unrecognized).
///
/// Phase 161:
/// - Accepts `&VoiceImageConfig` for the size
///   cap (Task 2), URL timeout (Task 3), and
///   HEAD pre-fetch toggle (Task 4).
async fn load_image_for_attach(
    path_or_url: &str,
    cfg: &VoiceImageConfig,
) -> Result<(String, Vec<u8>), String> {
    if path_or_url.is_empty() {
        return Err("path must not be empty".to_string());
    }
    if is_url(path_or_url) {
        return fetch_image_url(path_or_url, cfg).await;
    }
    let media_type = infer_image_media_type(path_or_url)?;
    let data = std::fs::read(path_or_url)
        .map_err(|e| format!("read {path_or_url:?}: {e}"))?;
    if data.is_empty() {
        return Err(format!("file {path_or_url:?} is empty"));
    }
    let cap = cfg.size_cap_bytes();
    if data.len() > cap {
        return Err(format!(
            "file {path_or_url:?} is {} bytes; max allowed is {} bytes ({} MB)",
            data.len(),
            cap,
            cfg.size_cap_mb,
        ));
    }
    Ok((media_type.to_string(), data))
}

/// Phase 156 — substrate URL detection. Returns
/// true when the operator's `/image` argument
/// is `http://` or `https://` prefixed.
fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

/// Phase 156 — fetch an image URL via reqwest
/// and infer media type from `Content-Type`
/// header. Falls back to extension inference
/// on the URL's path component when the header
/// is absent or doesn't match a supported
/// image type.
///
/// Phase 161:
/// - Builds a `reqwest::Client` with
///   `cfg.url_timeout_secs` applied so a slow
///   URL no longer hangs the session
///   indefinitely (Task 3).
/// - When `cfg.head_precheck` is true, issues
///   a HEAD before the GET; refuses with a
///   clear error when Content-Length exceeds
///   the cap. Falls through to GET when HEAD
///   fails or Content-Length is absent (Task
///   4).
/// - Enforces the operator-tunable
///   `cfg.size_cap_bytes()` post-fetch in
///   addition to the HEAD pre-check (Task 2).
async fn fetch_image_url(
    url: &str,
    cfg: &VoiceImageConfig,
) -> Result<(String, Vec<u8>), String> {
    let cap = cfg.size_cap_bytes();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(cfg.url_timeout_secs))
        .build()
        .map_err(|e| format!("build reqwest client: {e}"))?;
    let header_map = build_header_map(&cfg.url_headers)?;
    if cfg.head_precheck {
        if let Some(reason) =
            head_precheck_size(&client, url, cap, &header_map).await?
        {
            return Err(reason);
        }
    }
    // Phase 166 — retry on transient timeout
    // or connect failure. Exponential backoff:
    // delay = backoff_ms * 2^attempt. 4xx /
    // 5xx responses do NOT retry — those are
    // operator-fixable errors.
    let resp = send_with_retry(
        &client,
        url,
        &header_map,
        cfg.url_retry_count,
        cfg.url_retry_backoff_ms,
    )
    .await
    .map_err(|e| format!("fetch {url:?}: {e}"))?;
    let header_ct = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("read {url:?} body: {e}"))?
        .to_vec();
    if bytes.is_empty() {
        return Err(format!("url {url:?} returned empty body"));
    }
    if bytes.len() > cap {
        return Err(format!(
            "url {url:?} returned {} bytes; max allowed is {} bytes ({} MB)",
            bytes.len(),
            cap,
            cfg.size_cap_mb,
        ));
    }
    let media_type = media_type_from_content_type(header_ct.as_deref())
        .or_else(|| infer_image_media_type_for_url_path(url))
        .ok_or_else(|| {
            format!(
                "url {url:?} response has no recognizable image media type \
                 (Content-Type header missing/unsupported AND URL path lacks \
                 a supported extension); Phase 156 supports png / jpg / jpeg \
                 / gif / webp"
            )
        })?;
    Ok((media_type.to_string(), bytes))
}

/// Phase 166 — true iff a `reqwest::Error`
/// represents a transient client-side
/// condition worth retrying (timeout, connect
/// failure). HTTP-status errors (4xx / 5xx)
/// are NOT considered transient — those are
/// operator-fixable (auth, content, server
/// rate-limit) and retrying would mask the
/// real cause. Pure substrate so the
/// classification can be tested without going
/// through `reqwest::Client`.
fn is_transient_reqwest_error(e: &reqwest::Error) -> bool {
    e.is_timeout() || e.is_connect()
}

/// Phase 166 — perform the GET with retry on
/// transient transport errors. Backoff
/// doubles per attempt; `retry_count = 0`
/// means "try once" (the Phase 161 behavior).
async fn send_with_retry(
    client: &reqwest::Client,
    url: &str,
    headers: &reqwest::header::HeaderMap,
    retry_count: u32,
    backoff_ms: u64,
) -> Result<reqwest::Response, reqwest::Error> {
    let mut attempt: u32 = 0;
    loop {
        match client.get(url).headers(headers.clone()).send().await {
            Ok(resp) => return Ok(resp),
            Err(e) if attempt < retry_count && is_transient_reqwest_error(&e) => {
                let delay_ms = backoff_ms.saturating_mul(1u64 << attempt);
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms))
                    .await;
                attempt += 1;
                continue;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Phase 161 — HEAD pre-fetch for size check.
/// Returns:
/// - `Ok(None)` when the pre-check passed (or
///   the server didn't give us a usable
///   `Content-Length`) — caller falls through
///   to GET.
/// - `Ok(Some(reason))` when Content-Length
///   exceeded the cap — caller surfaces the
///   reason and skips the GET.
/// - `Err(_)` only for fatal transport errors;
///   we deliberately fall through on HTTP 405
///   ("Method Not Allowed", common for static
///   CDNs) so HEAD-hostile origins still
///   work.
async fn head_precheck_size(
    client: &reqwest::Client,
    url: &str,
    cap: usize,
    headers: &reqwest::header::HeaderMap,
) -> Result<Option<String>, String> {
    let resp = match client.head(url).headers(headers.clone()).send().await {
        Ok(r) => r,
        // Transport failure on HEAD: don't
        // block the GET attempt; some networks
        // proxy GET fine but reject HEAD.
        Err(_) => return Ok(None),
    };
    if !resp.status().is_success() {
        // 405 / 403 / 5xx on HEAD — fall
        // through. The GET will surface a
        // proper error if the URL is truly
        // broken.
        return Ok(None);
    }
    let Some(len_header) = resp.headers().get(reqwest::header::CONTENT_LENGTH)
    else {
        return Ok(None);
    };
    let Some(len_str) = len_header.to_str().ok() else {
        return Ok(None);
    };
    let Ok(len) = len_str.parse::<usize>() else {
        return Ok(None);
    };
    if len > cap {
        let cap_mb = cap / (1024 * 1024);
        Ok(Some(format!(
            "url {url:?} advertises {len} bytes via Content-Length; \
             max allowed is {cap} bytes ({cap_mb} MB) — refused before download"
        )))
    } else {
        Ok(None)
    }
}

/// Phase 165 — parse the `/image` command's
/// argument string into `(path_or_url,
/// optional preset_name)`. Accepted shapes:
/// - `foo.png` → (`"foo.png"`, None)
/// - `https://x.com/a.png` → (URL, None)
/// - `foo.png --headers work` →
///   (`"foo.png"`, Some(`"work"`))
/// - `https://x.com --headers personal` →
///   (URL, Some(`"personal"`))
///
/// Rejected shapes:
/// - empty input → "path must not be empty"
/// - `--headers` without name → "missing
///   preset name"
/// - `--headers` first then path → ambiguous;
///   keep the parser strict (rejected) to
///   avoid silently mis-routing
pub(crate) fn parse_image_command_args(
    rest: &str,
) -> Result<(&str, Option<&str>), String> {
    if rest.is_empty() {
        return Err("path must not be empty".to_string());
    }
    // Find the first occurrence of " --headers
    // " (with spaces) so we don't accidentally
    // split a URL containing the literal
    // string.
    let flag = " --headers ";
    if let Some(pos) = rest.find(flag) {
        let path = rest[..pos].trim();
        let preset = rest[pos + flag.len()..].trim();
        if path.is_empty() {
            return Err("path must not be empty before --headers".to_string());
        }
        if preset.is_empty() {
            return Err(
                "--headers requires a preset name (e.g. `--headers work`)"
                    .to_string(),
            );
        }
        if preset.contains(char::is_whitespace) {
            return Err(format!(
                "--headers preset name must not contain whitespace; got {preset:?}"
            ));
        }
        Ok((path, Some(preset)))
    } else if rest.ends_with(" --headers") || rest == "--headers" {
        Err(
            "--headers requires a preset name (e.g. `--headers work`)"
                .to_string(),
        )
    } else {
        Ok((rest, None))
    }
}

/// Phase 163 / amendment A13 — pick the right
/// `ContentPart` variant for a pending
/// attachment based on its media_type.
/// Document media types route to
/// `ContentPart::Document` (so the LLM provider
/// sees a document content block); everything
/// else routes to `ContentPart::Image`.
///
/// Document-classified types:
/// - `application/pdf`
///
/// Image-classified types (default):
/// - `image/png`, `image/jpeg`, `image/gif`,
///   `image/webp`, `image/svg+xml`,
///   `image/tiff` — and any unknown media_type
///   that lands here, since the LLM provider
///   surfaces its own error if the type is
///   unsupported.
fn content_part_for_attachment(
    media_type: String,
    data: Vec<u8>,
) -> aivyx_core::ContentPart {
    if is_document_media_type(&media_type) {
        aivyx_core::ContentPart::Document { media_type, data }
    } else {
        aivyx_core::ContentPart::Image { media_type, data }
    }
}

/// Phase 163 / amendment A13 — true iff the
/// media_type names a document-block-eligible
/// type. Pure substrate so the classification
/// can be tested without building a full
/// Message.
///
/// Phase 164 — adds DOCX. Provider rejection
/// on Anthropic surfaces as 400; the surface
/// area lands so provider widening doesn't
/// require voice-side work.
fn is_document_media_type(media_type: &str) -> bool {
    matches!(
        media_type,
        "application/pdf"
            | DOCX_MEDIA_TYPE
            | DOC_MEDIA_TYPE
            | RTF_MEDIA_TYPE
            | ODT_MEDIA_TYPE
            | PPTX_MEDIA_TYPE
            | XLSX_MEDIA_TYPE
    )
}

/// Phase 162 — build a `reqwest::header::HeaderMap`
/// from the operator's `[voice.image.url_headers]`
/// TOML table. Surfaces `InvalidHeaderName` and
/// `InvalidHeaderValue` errors as plain strings
/// so the caller can attach them to the
/// per-attach error message. Returns an empty
/// map (cheap clone) when the operator supplied
/// no headers.
fn build_header_map(
    raw: &std::collections::HashMap<String, String>,
) -> Result<reqwest::header::HeaderMap, String> {
    use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
    let mut out = HeaderMap::with_capacity(raw.len());
    for (name, value) in raw {
        let header_name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|e| format!("invalid header name {name:?}: {e}"))?;
        let header_value = HeaderValue::from_str(value)
            .map_err(|e| format!("invalid value for header {name:?}: {e}"))?;
        out.insert(header_name, header_value);
    }
    Ok(out)
}

/// Phase 156 — Map an HTTP `Content-Type`
/// header value to a canonical image media type
/// when it matches one of the supported formats.
/// Returns `None` otherwise (caller falls back
/// to URL-extension inference).
///
/// Phase 162 — adds `application/pdf`,
/// `image/svg+xml`, `image/tiff`. Honest scope
/// risk: the downstream LLM provider may reject
/// these as image-block content (PDFs typically
/// route through document blocks, SVG/TIFF
/// support varies). Phase 162 passes the
/// media_type through opaquely and lets the
/// provider's response surface the error.
fn media_type_from_content_type(ct: Option<&str>) -> Option<&'static str> {
    let raw = ct?;
    // Header may include "; charset=..." or
    // similar parameters; strip after the first
    // semicolon.
    let base = raw.split(';').next().unwrap_or(raw).trim().to_ascii_lowercase();
    match base.as_str() {
        "image/png" => Some("image/png"),
        "image/jpeg" | "image/jpg" => Some("image/jpeg"),
        "image/gif" => Some("image/gif"),
        "image/webp" => Some("image/webp"),
        // Phase 162 additions.
        "application/pdf" => Some("application/pdf"),
        "image/svg+xml" | "image/svg" => Some("image/svg+xml"),
        "image/tiff" | "image/tif" => Some("image/tiff"),
        // Phase 164 — DOCX support. Provider
        // rejection on Anthropic surfaces as a
        // 400; the inference surface lands so
        // that when provider support widens,
        // voice-side work is unnecessary.
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => {
            Some(DOCX_MEDIA_TYPE)
        }
        // Phase 165 — Phase 164 conflated
        // `application/msword` with DOCX; Phase
        // 165 routes it to DOC (the legacy
        // .doc binary format) since that's the
        // semantically correct MIME for those
        // files.
        "application/msword" => Some(DOC_MEDIA_TYPE),
        "application/rtf" | "text/rtf" => Some(RTF_MEDIA_TYPE),
        "application/vnd.oasis.opendocument.text" => Some(ODT_MEDIA_TYPE),
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => {
            Some(PPTX_MEDIA_TYPE)
        }
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => {
            Some(XLSX_MEDIA_TYPE)
        }
        _ => None,
    }
}

/// Phase 164 — canonical DOCX MIME string.
/// Pulled out so the matcher and the extension
/// branch share the same constant.
const DOCX_MEDIA_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document";

/// Phase 165 — canonical Office format MIME strings.
/// All five route through the Phase 163 Document
/// variant. Anthropic accepts PDF only today; these
/// surface a 400 from the API. The inference is
/// landing-bay for provider-side widening.
const DOC_MEDIA_TYPE: &str = "application/msword";
const RTF_MEDIA_TYPE: &str = "application/rtf";
const ODT_MEDIA_TYPE: &str =
    "application/vnd.oasis.opendocument.text";
const PPTX_MEDIA_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.presentation";
const XLSX_MEDIA_TYPE: &str =
    "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";

/// Phase 156 — Extension-fallback for URL paths.
/// Strips query string + fragment + then calls
/// `infer_image_media_type` on the remaining
/// path. Returns None (rather than Result) so
/// `fetch_image_url` can compose this with the
/// Content-Type path cleanly.
fn infer_image_media_type_for_url_path(url: &str) -> Option<&'static str> {
    let path = url
        .split(&['?', '#'][..])
        .next()
        .unwrap_or(url);
    infer_image_media_type(path).ok()
}

/// Phase 154 — infer the image media type
/// from a file's extension. Pure substrate;
/// returns the canonical `image/<format>` MIME
/// string on a known extension, or a clear
/// error otherwise.
///
/// Phase 154 MVP covered the four
/// operator-typical formats (PNG, JPEG, GIF,
/// WebP). Phase 162 extends with PDF / SVG /
/// TIFF — note that the downstream LLM
/// provider may reject the non-image
/// formats as image-block content (PDFs in
/// particular typically route through
/// document blocks, not image blocks).
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
        // Phase 162 additions.
        "pdf" => Ok("application/pdf"),
        "svg" => Ok("image/svg+xml"),
        "tif" | "tiff" => Ok("image/tiff"),
        // Phase 164 — DOCX.
        "docx" => Ok(DOCX_MEDIA_TYPE),
        // Phase 165 — additional Office formats.
        "doc" => Ok(DOC_MEDIA_TYPE),
        "rtf" => Ok(RTF_MEDIA_TYPE),
        "odt" => Ok(ODT_MEDIA_TYPE),
        "pptx" => Ok(PPTX_MEDIA_TYPE),
        "xlsx" => Ok(XLSX_MEDIA_TYPE),
        other => Err(format!(
            "unsupported image extension {other:?}; \
             supported: png / jpg / jpeg / gif / webp / pdf / svg / tif / tiff / \
             docx / doc / rtf / odt / pptx / xlsx"
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

    // ---- Phase 162 — PDF / SVG / TIFF support ----

    #[test]
    fn infer_image_media_type_pdf() {
        assert_eq!(
            infer_image_media_type("doc.pdf").unwrap(),
            "application/pdf"
        );
        assert_eq!(
            infer_image_media_type("/abs/path/FILE.PDF").unwrap(),
            "application/pdf"
        );
    }

    #[test]
    fn infer_image_media_type_svg() {
        assert_eq!(
            infer_image_media_type("diagram.svg").unwrap(),
            "image/svg+xml"
        );
    }

    #[test]
    fn infer_image_media_type_tiff_variants() {
        assert_eq!(infer_image_media_type("a.tif").unwrap(), "image/tiff");
        assert_eq!(infer_image_media_type("b.tiff").unwrap(), "image/tiff");
        assert_eq!(infer_image_media_type("C.TIFF").unwrap(), "image/tiff");
    }

    #[test]
    fn infer_image_media_type_truly_unsupported_extension_rejects() {
        // .heic / .bmp / .ico are not in Phase
        // 162's set; rejection error lists the
        // supported types.
        let err = infer_image_media_type("a.heic").unwrap_err();
        assert!(err.contains("unsupported"), "{err}");
        assert!(err.contains("pdf"), "{err}");
        assert!(err.contains("svg"), "{err}");
    }

    #[test]
    fn media_type_from_content_type_pdf() {
        assert_eq!(
            media_type_from_content_type(Some("application/pdf")),
            Some("application/pdf")
        );
        assert_eq!(
            media_type_from_content_type(Some("APPLICATION/PDF; charset=binary")),
            Some("application/pdf")
        );
    }

    #[test]
    fn media_type_from_content_type_svg_xml_and_legacy() {
        assert_eq!(
            media_type_from_content_type(Some("image/svg+xml")),
            Some("image/svg+xml")
        );
        // Some old servers respond `image/svg`
        // without the `+xml`; we normalize to
        // the canonical form.
        assert_eq!(
            media_type_from_content_type(Some("image/svg")),
            Some("image/svg+xml")
        );
    }

    #[test]
    fn media_type_from_content_type_tiff_and_tif() {
        assert_eq!(
            media_type_from_content_type(Some("image/tiff")),
            Some("image/tiff")
        );
        assert_eq!(
            media_type_from_content_type(Some("image/tif")),
            Some("image/tiff")
        );
    }

    #[test]
    fn infer_image_media_type_no_extension_rejects() {
        let err = infer_image_media_type("README").unwrap_err();
        assert!(err.contains("no file extension"), "{err}");
    }

    #[tokio::test]
    async fn load_image_empty_path_rejected() {
        let cfg = VoiceImageConfig::default();
        let err = load_image_for_attach("", &cfg).await.unwrap_err();
        assert!(err.contains("path must not be empty"), "{err}");
    }

    #[tokio::test]
    async fn load_image_missing_file_rejected() {
        let cfg = VoiceImageConfig::default();
        let err = load_image_for_attach("/nonexistent/file.png", &cfg)
            .await
            .unwrap_err();
        assert!(err.contains("read"), "{err}");
    }

    // ---- Phase 156 — size cap + URL substrate ----

    #[test]
    fn default_image_size_cap_is_ten_megabytes() {
        // Pin the default. Same as Phase 129's
        // Drive inline cap. Phase 161 promoted
        // this from a hardcoded constant to the
        // default of `VoiceImageConfig::size_cap_mb`.
        assert_eq!(DEFAULT_IMAGE_SIZE_CAP_MB, 10);
        let cfg = VoiceImageConfig::default();
        assert_eq!(cfg.size_cap_bytes(), 10 * 1024 * 1024);
    }

    #[tokio::test]
    async fn load_image_oversized_rejected() {
        // Write a 10MB + 1 byte file and verify
        // load_image_for_attach rejects it with
        // a clear error.
        let cfg = VoiceImageConfig::default();
        let tmp = std::env::temp_dir().join("phase156-oversized.png");
        let oversized = vec![0u8; cfg.size_cap_bytes() + 1];
        std::fs::write(&tmp, &oversized).expect("write tmp");
        let err = load_image_for_attach(tmp.to_str().expect("utf8"), &cfg)
            .await
            .unwrap_err();
        assert!(
            err.contains("max allowed"),
            "expected size-cap error, got: {err}"
        );
        std::fs::remove_file(&tmp).ok();
    }

    // ---- Phase 161 — VoiceImageConfig knobs ----

    #[test]
    fn voice_image_config_defaults_match_phase_156_behavior() {
        let cfg = VoiceImageConfig::default();
        assert_eq!(cfg.size_cap_mb, 10);
        assert_eq!(cfg.url_timeout_secs, 30);
        assert!(cfg.head_precheck);
        // Phase 162 — url_headers defaults to
        // empty map.
        assert!(cfg.url_headers.is_empty());
    }

    #[test]
    fn voice_image_config_size_cap_bytes_scales_correctly() {
        let cfg = VoiceImageConfig {
            size_cap_mb: 5,
            url_timeout_secs: 30,
            head_precheck: true,
            url_headers: Default::default(),
            url_header_presets: Default::default(),
            url_retry_count: 0,
            url_retry_backoff_ms: 500,
        };
        assert_eq!(cfg.size_cap_bytes(), 5 * 1024 * 1024);
    }

    #[test]
    fn voice_image_config_deserializes_from_full_section() {
        let toml = r#"
size_cap_mb = 25
url_timeout_secs = 60
head_precheck = false
"#;
        let cfg: VoiceImageConfig = toml::from_str(toml).expect("parse");
        assert_eq!(cfg.size_cap_mb, 25);
        assert_eq!(cfg.url_timeout_secs, 60);
        assert!(!cfg.head_precheck);
    }

    #[test]
    fn voice_image_config_deserializes_partial_with_defaults() {
        let toml = r#"
size_cap_mb = 50
"#;
        let cfg: VoiceImageConfig = toml::from_str(toml).expect("parse");
        assert_eq!(cfg.size_cap_mb, 50);
        // Untouched defaults preserved.
        assert_eq!(cfg.url_timeout_secs, 30);
        assert!(cfg.head_precheck);
        assert!(cfg.url_headers.is_empty());
    }

    // ---- Phase 162 — url_headers + build_header_map ----

    #[test]
    fn voice_image_config_deserializes_url_headers_block() {
        let toml = r#"
size_cap_mb = 10

[url_headers]
Authorization = "Bearer xyz"
"X-Origin" = "aivyx-test"
"#;
        let cfg: VoiceImageConfig = toml::from_str(toml).expect("parse");
        assert_eq!(cfg.url_headers.len(), 2);
        assert_eq!(
            cfg.url_headers.get("Authorization").map(String::as_str),
            Some("Bearer xyz")
        );
        assert_eq!(
            cfg.url_headers.get("X-Origin").map(String::as_str),
            Some("aivyx-test")
        );
    }

    #[test]
    fn build_header_map_empty_returns_empty() {
        let map = std::collections::HashMap::new();
        let hm = build_header_map(&map).expect("ok on empty");
        assert!(hm.is_empty());
    }

    #[test]
    fn build_header_map_populates_authorization_and_custom_headers() {
        let mut map = std::collections::HashMap::new();
        map.insert("Authorization".to_string(), "Bearer abc".to_string());
        map.insert("X-Custom".to_string(), "hello".to_string());
        let hm = build_header_map(&map).expect("ok");
        assert_eq!(hm.len(), 2);
        assert_eq!(
            hm.get(reqwest::header::AUTHORIZATION)
                .and_then(|v| v.to_str().ok()),
            Some("Bearer abc")
        );
        assert_eq!(
            hm.get("x-custom").and_then(|v| v.to_str().ok()),
            Some("hello")
        );
    }

    #[test]
    fn build_header_map_rejects_invalid_header_name() {
        let mut map = std::collections::HashMap::new();
        // Header names can't contain spaces or
        // non-token characters.
        map.insert("Bad Header Name".to_string(), "x".to_string());
        let err = build_header_map(&map).unwrap_err();
        assert!(err.contains("invalid header name"), "{err}");
    }

    #[test]
    fn build_header_map_rejects_invalid_header_value() {
        let mut map = std::collections::HashMap::new();
        // Newlines aren't allowed in header
        // values per HTTP/1.1.
        map.insert("X-Bad".to_string(), "first\r\nInjected: yes".to_string());
        let err = build_header_map(&map).unwrap_err();
        assert!(err.contains("invalid value for header"), "{err}");
    }

    // ---- Phase 163 / Amendment A13 — Document routing ----

    #[test]
    fn is_document_media_type_classifies_pdf() {
        assert!(is_document_media_type("application/pdf"));
    }

    #[test]
    fn is_document_media_type_classifies_images_as_non_document() {
        assert!(!is_document_media_type("image/png"));
        assert!(!is_document_media_type("image/jpeg"));
        assert!(!is_document_media_type("image/gif"));
        assert!(!is_document_media_type("image/webp"));
        assert!(!is_document_media_type("image/svg+xml"));
        assert!(!is_document_media_type("image/tiff"));
    }

    #[test]
    fn is_document_media_type_unknown_returns_false_default_image_routing() {
        // Unknown types route as Image — the
        // provider surfaces its own error if it
        // can't handle them.
        assert!(!is_document_media_type("application/octet-stream"));
        assert!(!is_document_media_type(""));
    }

    #[test]
    fn content_part_for_attachment_routes_pdf_to_document() {
        let part = content_part_for_attachment(
            "application/pdf".to_string(),
            vec![0x25, 0x50, 0x44, 0x46],
        );
        match part {
            aivyx_core::ContentPart::Document { media_type, data } => {
                assert_eq!(media_type, "application/pdf");
                assert_eq!(data, vec![0x25, 0x50, 0x44, 0x46]);
            }
            other => panic!("expected Document for PDF, got {other:?}"),
        }
    }

    #[test]
    fn content_part_for_attachment_routes_image_to_image() {
        let part = content_part_for_attachment(
            "image/png".to_string(),
            vec![0x89, 0x50, 0x4E, 0x47],
        );
        match part {
            aivyx_core::ContentPart::Image { media_type, data } => {
                assert_eq!(media_type, "image/png");
                assert_eq!(data, vec![0x89, 0x50, 0x4E, 0x47]);
            }
            other => panic!("expected Image for PNG, got {other:?}"),
        }
    }

    #[test]
    fn content_part_for_attachment_routes_svg_as_image_not_document() {
        // SVG is visual content, not a document.
        // Even though most LLM providers reject
        // it, the voice substrate classifies it
        // as Image so it goes through the image
        // provider path.
        let part = content_part_for_attachment(
            "image/svg+xml".to_string(),
            b"<svg></svg>".to_vec(),
        );
        assert!(matches!(
            part,
            aivyx_core::ContentPart::Image { .. }
        ));
    }

    // ---- Phase 164 — DOCX inference ----

    #[test]
    fn infer_image_media_type_docx() {
        assert_eq!(
            infer_image_media_type("report.docx").unwrap(),
            DOCX_MEDIA_TYPE,
        );
        assert_eq!(
            infer_image_media_type("/abs/DRAFT.DOCX").unwrap(),
            DOCX_MEDIA_TYPE,
        );
    }

    #[test]
    fn media_type_from_content_type_docx() {
        assert_eq!(
            media_type_from_content_type(Some(
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            )),
            Some(DOCX_MEDIA_TYPE),
        );
    }

    #[test]
    fn media_type_from_content_type_doc_legacy_msword_maps_to_doc() {
        // Phase 165 correction — `application/
        // msword` is the legacy DOC binary
        // MIME, not DOCX. Phase 164 conflated
        // the two; Phase 165 separates them.
        assert_eq!(
            media_type_from_content_type(Some("application/msword")),
            Some(DOC_MEDIA_TYPE),
        );
    }

    #[test]
    fn is_document_media_type_classifies_docx() {
        assert!(is_document_media_type(DOCX_MEDIA_TYPE));
    }

    #[test]
    fn content_part_for_attachment_routes_docx_to_document() {
        let part = content_part_for_attachment(
            DOCX_MEDIA_TYPE.to_string(),
            b"PK\x03\x04 fake docx".to_vec(),
        );
        assert!(matches!(
            part,
            aivyx_core::ContentPart::Document { ref media_type, .. }
                if media_type == DOCX_MEDIA_TYPE
        ));
    }

    #[test]
    fn infer_image_media_type_truly_unsupported_after_phase_164() {
        // Phase 164's rejection error names docx
        // in the supported list.
        let err = infer_image_media_type("a.heic").unwrap_err();
        assert!(err.contains("docx"), "{err}");
    }

    // ---- Phase 165 — Office formats ----

    #[test]
    fn infer_image_media_type_doc_rtf_odt() {
        assert_eq!(infer_image_media_type("a.doc").unwrap(), DOC_MEDIA_TYPE);
        assert_eq!(infer_image_media_type("a.rtf").unwrap(), RTF_MEDIA_TYPE);
        assert_eq!(infer_image_media_type("a.odt").unwrap(), ODT_MEDIA_TYPE);
    }

    #[test]
    fn infer_image_media_type_pptx_and_xlsx() {
        assert_eq!(
            infer_image_media_type("deck.pptx").unwrap(),
            PPTX_MEDIA_TYPE
        );
        assert_eq!(
            infer_image_media_type("data.xlsx").unwrap(),
            XLSX_MEDIA_TYPE
        );
    }

    #[test]
    fn infer_image_media_type_uppercase_extensions() {
        // Case folding regression: all five
        // formats should accept upper-case
        // extensions (operators on Windows
        // often have them).
        assert_eq!(infer_image_media_type("A.DOC").unwrap(), DOC_MEDIA_TYPE);
        assert_eq!(
            infer_image_media_type("A.PPTX").unwrap(),
            PPTX_MEDIA_TYPE
        );
    }

    #[test]
    fn media_type_from_content_type_rtf_legacy_and_modern() {
        assert_eq!(
            media_type_from_content_type(Some("application/rtf")),
            Some(RTF_MEDIA_TYPE),
        );
        // Some older servers send text/rtf
        // instead of application/rtf.
        assert_eq!(
            media_type_from_content_type(Some("text/rtf")),
            Some(RTF_MEDIA_TYPE),
        );
    }

    #[test]
    fn media_type_from_content_type_odt() {
        assert_eq!(
            media_type_from_content_type(Some(
                "application/vnd.oasis.opendocument.text"
            )),
            Some(ODT_MEDIA_TYPE),
        );
    }

    #[test]
    fn media_type_from_content_type_pptx_xlsx() {
        assert_eq!(
            media_type_from_content_type(Some(
                "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            )),
            Some(PPTX_MEDIA_TYPE),
        );
        assert_eq!(
            media_type_from_content_type(Some(
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            )),
            Some(XLSX_MEDIA_TYPE),
        );
    }

    #[test]
    fn is_document_media_type_classifies_all_office_formats() {
        assert!(is_document_media_type(DOC_MEDIA_TYPE));
        assert!(is_document_media_type(RTF_MEDIA_TYPE));
        assert!(is_document_media_type(ODT_MEDIA_TYPE));
        assert!(is_document_media_type(PPTX_MEDIA_TYPE));
        assert!(is_document_media_type(XLSX_MEDIA_TYPE));
    }

    #[test]
    fn content_part_for_attachment_routes_pptx_to_document() {
        let part = content_part_for_attachment(
            PPTX_MEDIA_TYPE.to_string(),
            b"PK\x03\x04 fake pptx".to_vec(),
        );
        assert!(matches!(part, aivyx_core::ContentPart::Document { .. }));
    }

    #[test]
    fn infer_image_media_type_error_lists_phase_165_formats() {
        let err = infer_image_media_type("a.heic").unwrap_err();
        for needed in &["pptx", "xlsx", "rtf", "odt", "doc"] {
            assert!(
                err.contains(needed),
                "expected {needed} in supported list, got: {err}"
            );
        }
    }

    // ---- Phase 165 — parse_image_command_args ----

    #[test]
    fn parse_image_args_plain_path() {
        let (path, preset) =
            parse_image_command_args("foo.png").expect("ok");
        assert_eq!(path, "foo.png");
        assert_eq!(preset, None);
    }

    #[test]
    fn parse_image_args_plain_url() {
        let (path, preset) =
            parse_image_command_args("https://example.com/a.png")
                .expect("ok");
        assert_eq!(path, "https://example.com/a.png");
        assert_eq!(preset, None);
    }

    #[test]
    fn parse_image_args_path_with_preset() {
        let (path, preset) =
            parse_image_command_args("foo.png --headers work").expect("ok");
        assert_eq!(path, "foo.png");
        assert_eq!(preset, Some("work"));
    }

    #[test]
    fn parse_image_args_url_with_preset() {
        let (path, preset) =
            parse_image_command_args("https://example.com/a.png --headers personal")
                .expect("ok");
        assert_eq!(path, "https://example.com/a.png");
        assert_eq!(preset, Some("personal"));
    }

    #[test]
    fn parse_image_args_empty_rejected() {
        let err = parse_image_command_args("").unwrap_err();
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn parse_image_args_missing_preset_name_rejected() {
        let err = parse_image_command_args("foo.png --headers")
            .unwrap_err();
        assert!(err.contains("preset name"), "{err}");
    }

    #[test]
    fn parse_image_args_preset_with_whitespace_rejected() {
        let err =
            parse_image_command_args("foo.png --headers two words")
                .unwrap_err();
        assert!(err.contains("whitespace"), "{err}");
    }

    #[test]
    fn parse_image_args_just_flag_rejected() {
        let err = parse_image_command_args("--headers").unwrap_err();
        assert!(err.contains("preset name"), "{err}");
    }

    #[test]
    fn voice_image_config_deserializes_url_header_presets() {
        let toml = r#"
size_cap_mb = 10

[url_header_presets.work]
Authorization = "Bearer work-token"

[url_header_presets.personal]
Cookie = "session=personal"
"#;
        let cfg: VoiceImageConfig = toml::from_str(toml).expect("parse");
        assert_eq!(cfg.url_header_presets.len(), 2);
        let work = cfg.url_header_presets.get("work").expect("work preset");
        assert_eq!(work.get("Authorization").map(String::as_str), Some("Bearer work-token"));
        let personal =
            cfg.url_header_presets.get("personal").expect("personal preset");
        assert_eq!(personal.get("Cookie").map(String::as_str), Some("session=personal"));
    }

    #[test]
    fn voice_image_config_url_header_presets_default_empty() {
        let cfg = VoiceImageConfig::default();
        assert!(cfg.url_header_presets.is_empty());
    }

    // ---- Phase 166 — URL fetch retry ----

    #[test]
    fn voice_image_config_url_retry_defaults() {
        let cfg = VoiceImageConfig::default();
        assert_eq!(cfg.url_retry_count, 0);
        assert_eq!(cfg.url_retry_backoff_ms, 500);
    }

    #[test]
    fn voice_image_config_deserializes_url_retry_fields() {
        let toml = r#"
size_cap_mb = 10
url_retry_count = 3
url_retry_backoff_ms = 1000
"#;
        let cfg: VoiceImageConfig = toml::from_str(toml).expect("parse");
        assert_eq!(cfg.url_retry_count, 3);
        assert_eq!(cfg.url_retry_backoff_ms, 1000);
    }

    #[test]
    fn url_retry_backoff_formula_exponential() {
        // Direct-computation test of the
        // backoff math used inside
        // send_with_retry. attempt N uses
        // delay = backoff_ms * 2^N. The
        // function uses saturating_mul to
        // prevent overflow on absurdly large
        // retry counts.
        let backoff_ms: u64 = 500;
        let delays: Vec<u64> = (0..4)
            .map(|attempt| backoff_ms.saturating_mul(1u64 << attempt))
            .collect();
        // 500, 1000, 2000, 4000 (ms).
        assert_eq!(delays, vec![500, 1000, 2000, 4000]);
    }

    #[test]
    fn url_retry_backoff_saturates_on_large_attempt_counts() {
        // Pathological: an operator with
        // retry_count = 64 and a very large
        // backoff. 1u64 << 64 would panic via
        // overflow; saturating_mul keeps us at
        // u64::MAX without UB. The retry loop
        // would never actually complete that
        // many retries because tokio::sleep
        // would take centuries, but the math
        // is provably safe.
        let backoff_ms: u64 = u64::MAX / 2;
        let shifted = 1u64 << 63;
        assert_eq!(
            backoff_ms.saturating_mul(shifted),
            u64::MAX,
        );
    }

    #[test]
    fn cfg_retry_count_zero_means_one_attempt() {
        // The retry loop's `attempt < retry_count`
        // condition means retry_count = 0
        // permits zero retries — i.e. the
        // request is attempted once with no
        // retry. This matches Phase 161
        // behavior; Phase 166 is purely
        // additive when the operator leaves
        // the default in place.
        let cfg = VoiceImageConfig::default();
        assert_eq!(cfg.url_retry_count, 0);
        // The loop guard:
        let retry_count: u32 = cfg.url_retry_count;
        let attempt: u32 = 0;
        let would_retry = attempt < retry_count;
        assert!(!would_retry);
    }

    #[test]
    fn voice_image_config_size_cap_bytes_saturates_on_overflow() {
        // An operator setting an absurd cap like
        // u32::MAX MB shouldn't overflow the
        // bytes computation — saturate at
        // usize::MAX instead. Documented as
        // "no upper bound — operator trust"
        // in the open doc.
        let cfg = VoiceImageConfig {
            size_cap_mb: usize::MAX,
            url_timeout_secs: 30,
            head_precheck: true,
            url_headers: Default::default(),
            url_header_presets: Default::default(),
            url_retry_count: 0,
            url_retry_backoff_ms: 500,
        };
        assert_eq!(cfg.size_cap_bytes(), usize::MAX);
    }

    #[tokio::test]
    async fn load_image_honors_operator_size_cap_below_default() {
        // Write a 6-byte file and verify a
        // 5-byte cap rejects it (sub-default
        // operator-tighter cap).
        let cfg = VoiceImageConfig {
            size_cap_mb: 0, // 0 MB = 0 bytes
            url_timeout_secs: 30,
            head_precheck: true,
            url_headers: Default::default(),
            url_header_presets: Default::default(),
            url_retry_count: 0,
            url_retry_backoff_ms: 500,
        };
        let tmp = std::env::temp_dir().join("phase161-tightcap.png");
        std::fs::write(&tmp, b"abc").expect("write tmp");
        let err = load_image_for_attach(tmp.to_str().expect("utf8"), &cfg)
            .await
            .unwrap_err();
        assert!(
            err.contains("max allowed"),
            "expected size-cap error, got: {err}"
        );
        std::fs::remove_file(&tmp).ok();
    }

    #[test]
    fn is_url_recognizes_http_and_https() {
        assert!(is_url("http://example.com/foo.png"));
        assert!(is_url("https://example.com/foo.png"));
        assert!(!is_url("/abs/path/file.png"));
        assert!(!is_url("relative/path.png"));
        assert!(!is_url("foo.png"));
        // ftp and file are URLs but Phase 156
        // doesn't support them — they fall
        // through to the file-path branch
        // which fails on the unknown extension.
        assert!(!is_url("ftp://example.com/foo.png"));
    }

    #[test]
    fn media_type_from_content_type_recognized_image_formats() {
        assert_eq!(
            media_type_from_content_type(Some("image/png")),
            Some("image/png")
        );
        assert_eq!(
            media_type_from_content_type(Some("image/jpeg")),
            Some("image/jpeg")
        );
        // Some servers return "image/jpg"; map
        // to canonical "image/jpeg".
        assert_eq!(
            media_type_from_content_type(Some("image/jpg")),
            Some("image/jpeg")
        );
        assert_eq!(
            media_type_from_content_type(Some("image/gif")),
            Some("image/gif")
        );
        assert_eq!(
            media_type_from_content_type(Some("image/webp")),
            Some("image/webp")
        );
    }

    #[test]
    fn media_type_from_content_type_strips_parameters() {
        // Servers may return "image/png;
        // charset=binary" or similar.
        assert_eq!(
            media_type_from_content_type(Some("image/png; charset=binary")),
            Some("image/png")
        );
        assert_eq!(
            media_type_from_content_type(Some("  Image/PNG  ; extra=ignored")),
            Some("image/png")
        );
    }

    #[test]
    fn media_type_from_content_type_unsupported_returns_none() {
        // Phase 162 — `image/svg+xml` and
        // `application/pdf` are now supported.
        // Truly unsupported types like text/html
        // and image/bmp still return None.
        assert!(media_type_from_content_type(Some("text/html")).is_none());
        assert!(media_type_from_content_type(Some("image/bmp")).is_none());
        assert!(media_type_from_content_type(Some("image/heic")).is_none());
        assert!(media_type_from_content_type(None).is_none());
    }

    #[test]
    fn infer_image_media_type_for_url_path_strips_query_string() {
        // Path is foo.png?signature=abc → strip
        // the ?... part, infer from foo.png.
        assert_eq!(
            infer_image_media_type_for_url_path(
                "https://cdn.example.com/foo.png?signature=abc"
            ),
            Some("image/png")
        );
        // Also strips fragment.
        assert_eq!(
            infer_image_media_type_for_url_path(
                "https://cdn.example.com/photo.jpg#meta"
            ),
            Some("image/jpeg")
        );
        // No extension → None (caller falls
        // back).
        assert!(infer_image_media_type_for_url_path(
            "https://cdn.example.com/some-opaque-id"
        )
        .is_none());
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
            image: Default::default(),
        };
        assert_eq!(cfg.asr.beam_size, Some(5));
        assert_eq!(cfg.tts.speaker_id, Some(0));
    }
}
