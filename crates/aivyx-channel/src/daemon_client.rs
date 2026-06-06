//! Daemon client — Phase 17 Task 4.
//!
//! Connects to a running daemon over the Unix domain socket,
//! manages a session, and supports multi-turn interaction. Includes
//! auto-spawn logic: if no daemon is listening, spawns one via
//! `aivyx daemon run` and waits for the socket to appear.
//!
//! Phase 16 shipped the single-turn PoC (`run_poc_client`).
//! Phase 17 Task 2 added multi-turn on the server side.
//! Phase 17 Task 4 adds multi-turn on the client side plus
//! auto-spawn.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use std::sync::Arc;

use crate::daemon_ipc::{
    decode_frame, encode_frame, DaemonEnvelope, EffectivePersonaSummary, FrameError,
    FrontendMessage, FrontendType, IpcAttachment, MemoryEntrySummary,
    NotificationHistoryEntry, PersonaDeltaSummary, PersonaProposalResolution,
    PersonaProposalResolveSuccess, PersonaProposalSummary, QueryPayload,
    QueryResponsePayload, StreamEventPayload,
};
use crate::daemon_server::DaemonError;

/// Result of a single PoC daemon turn (Phase 16 shape, kept for
/// backward compatibility with the existing e2e test).
#[derive(Debug)]
pub struct DaemonTurnResult {
    pub session_id: String,
    pub events: Vec<StreamEventPayload>,
    pub outcome: String,
    pub daemon_version: Option<String>,
}

/// A connected, session-aware daemon client that supports multi-turn
/// interaction. Created by [`DaemonSession::connect`].
pub struct DaemonSession {
    reader: tokio::net::unix::OwnedReadHalf,
    writer: Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>,
    buf: Vec<u8>,
    pub session_id: String,
    pub daemon_version: Option<String>,
}

impl DaemonSession {
    /// Connect to a running daemon, read `DaemonReady`, send
    /// `StartSession`, and return a session handle ready for
    /// `submit_input` calls.
    pub async fn connect(
        socket_path: &Path,
        role: Option<String>,
        frontend_type: Option<FrontendType>,
    ) -> Result<Self, DaemonError> {
        let stream = UnixStream::connect(socket_path).await?;
        let (mut reader, writer) = stream.into_split();
        let mut buf = Vec::with_capacity(4096);

        // Read DaemonReady.
        read_more(&mut reader, &mut buf).await?;
        let daemon_version: Option<String> =
            match decode_frame::<DaemonEnvelope>(&buf) {
                Ok((DaemonEnvelope::DaemonReady { version }, consumed)) => {
                    buf.drain(..consumed);
                    Some(version)
                }
                Ok((other, _)) => {
                    return Err(DaemonError::Protocol(format!(
                        "expected DaemonReady, got {other:?}"
                    )));
                }
                Err(e) => return Err(e.into()),
            };

        // Send StartSession.
        let start = FrontendMessage::StartSession { role, frontend_type };
        let frame = encode_frame(&start)?;
        let writer = Arc::new(tokio::sync::Mutex::new(writer));
        {
            let mut w = writer.lock().await;
            w.write_all(&frame).await?;
        }

        // Read SessionStarted.
        let session_id: String = loop {
            match decode_frame::<DaemonEnvelope>(&buf) {
                Ok((
                    DaemonEnvelope::SessionStarted { session_id: sid },
                    consumed,
                )) => {
                    buf.drain(..consumed);
                    break sid;
                }
                // The daemon delivers a take-once `RecoveryNotice`
                // between `DaemonReady` and `SessionStarted` to the
                // first frontend that connects after it restarted with
                // stale state (a previous instance that crashed / was
                // killed rather than shut down cleanly). It is
                // informational — skip past it and keep waiting for
                // `SessionStarted`, surfacing it so the operator knows
                // a prior session/turn was lost. Without this arm the
                // first reconnect after an unclean shutdown fails for
                // every frontend (REPL, TUI, channels all share this).
                Ok((
                    DaemonEnvelope::RecoveryNotice {
                        lost_sessions,
                        lost_turns,
                        ..
                    },
                    consumed,
                )) => {
                    buf.drain(..consumed);
                    if !lost_sessions.is_empty() || !lost_turns.is_empty() {
                        eprintln!(
                            "aivyx: daemon recovered from an unclean shutdown — \
                             {} session(s) and {} in-flight turn(s) were lost.",
                            lost_sessions.len(),
                            lost_turns.len(),
                        );
                    }
                }
                Ok((DaemonEnvelope::Error { code, message }, _)) => {
                    return Err(DaemonError::Protocol(format!(
                        "daemon error ({code}): {message}"
                    )));
                }
                Err(FrameError::IncompleteBuf) => {
                    read_more(&mut reader, &mut buf).await?;
                }
                Ok((other, _)) => {
                    return Err(DaemonError::Protocol(format!(
                        "expected SessionStarted, got {other:?}"
                    )));
                }
                Err(e) => return Err(e.into()),
            }
        };

        Ok(DaemonSession {
            reader,
            writer,
            buf,
            session_id,
            daemon_version,
        })
    }

    pub async fn submit_input_for_mission(
        &mut self,
        text: String,
        mission_id: String,
    ) -> Result<(Vec<StreamEventPayload>, String), DaemonError> {
        let submit = FrontendMessage::SubmitInput {
            session_id: self.session_id.clone(),
            text,
            mission_id: Some(mission_id),
            attachments: vec![],
        };
        self.send_and_collect(submit).await
    }

    /// Submit a turn to the daemon and collect all streamed events
    /// until `TurnComplete`. Returns the events and the outcome string.
    pub async fn submit_input(
        &mut self,
        text: String,
    ) -> Result<(Vec<StreamEventPayload>, String), DaemonError> {
        let submit = FrontendMessage::SubmitInput {
            session_id: self.session_id.clone(),
            text,
            mission_id: None,
            attachments: vec![],
        };
        self.send_and_collect(submit).await
    }

    /// Submit a turn with image attachments. Phase 45 multimodal path.
    pub async fn submit_input_with_attachments(
        &mut self,
        text: String,
        attachments: Vec<IpcAttachment>,
    ) -> Result<(Vec<StreamEventPayload>, String), DaemonError> {
        let submit = FrontendMessage::SubmitInput {
            session_id: self.session_id.clone(),
            text,
            mission_id: None,
            attachments,
        };
        self.send_and_collect(submit).await
    }

    async fn send_and_collect(
        &mut self,
        msg: FrontendMessage,
    ) -> Result<(Vec<StreamEventPayload>, String), DaemonError> {
        let frame = encode_frame(&msg)?;
        {
            let mut w = self.writer.lock().await;
            w.write_all(&frame).await?;
        }

        let mut events = Vec::new();
        loop {
            match decode_frame::<DaemonEnvelope>(&self.buf) {
                Ok((DaemonEnvelope::StreamEvent { event, .. }, consumed)) => {
                    self.buf.drain(..consumed);
                    events.push(event);
                }
                Ok((DaemonEnvelope::TurnComplete { outcome, .. }, consumed)) => {
                    self.buf.drain(..consumed);
                    return Ok((events, outcome));
                }
                Ok((DaemonEnvelope::Error { code, message }, _)) => {
                    return Err(DaemonError::Protocol(format!(
                        "daemon error ({code}): {message}"
                    )));
                }
                Ok((DaemonEnvelope::ShuttingDown { reason }, _)) => {
                    return Err(DaemonError::Protocol(format!(
                        "daemon shutting down: {reason}"
                    )));
                }
                Err(FrameError::IncompleteBuf) => {
                    read_more(&mut self.reader, &mut self.buf).await?;
                }
                Ok((other, consumed)) => {
                    self.buf.drain(..consumed);
                    return Err(DaemonError::Protocol(format!(
                        "unexpected message during turn: {other:?}"
                    )));
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Send `CancelTurn` to request cancellation of the in-flight turn.
    pub async fn cancel_turn(&mut self) -> Result<(), DaemonError> {
        let cancel = FrontendMessage::CancelTurn {
            session_id: self.session_id.clone(),
        };
        let frame = encode_frame(&cancel)?;
        let mut w = self.writer.lock().await;
        w.write_all(&frame).await?;
        Ok(())
    }

    /// Send `ResolveGate` and wait for `GateResolved` (or `Error`).
    pub async fn resolve_gate(
        &mut self,
        mission_id: String,
        gate_id: String,
        approved: bool,
    ) -> Result<(), DaemonError> {
        let msg = FrontendMessage::ResolveGate {
            mission_id,
            gate_id,
            approved,
        };
        let frame = encode_frame(&msg)?;
        {
            let mut w = self.writer.lock().await;
            w.write_all(&frame).await?;
        }

        loop {
            match decode_frame::<DaemonEnvelope>(&self.buf) {
                Ok((DaemonEnvelope::GateResolved { .. }, consumed)) => {
                    self.buf.drain(..consumed);
                    return Ok(());
                }
                Ok((DaemonEnvelope::Error { code, message }, _)) => {
                    return Err(DaemonError::Protocol(format!(
                        "gate resolve error ({code}): {message}"
                    )));
                }
                Ok((DaemonEnvelope::ShuttingDown { reason }, _)) => {
                    return Err(DaemonError::Protocol(format!(
                        "daemon shutting down: {reason}"
                    )));
                }
                Err(FrameError::IncompleteBuf) => {
                    read_more(&mut self.reader, &mut self.buf).await?;
                }
                Ok((other, consumed)) => {
                    self.buf.drain(..consumed);
                    return Err(DaemonError::Protocol(format!(
                        "unexpected message during ResolveGate: {other:?}"
                    )));
                }
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// Return a cloneable cancel handle for use from a signal handler.
    pub fn cancel_handle(&self) -> DaemonCancelHandle {
        DaemonCancelHandle {
            writer: Arc::clone(&self.writer),
            session_id: self.session_id.clone(),
        }
    }

    /// Send `Disconnect` and drop the connection cleanly.
    pub async fn disconnect(self) -> Result<(), DaemonError> {
        let frame = encode_frame(&FrontendMessage::Disconnect)?;
        let mut w = self.writer.lock().await;
        w.write_all(&frame).await?;
        Ok(())
    }
}

/// A cloneable handle for sending `CancelTurn` from a signal handler
/// without holding `&mut DaemonSession`. Created by
/// [`DaemonSession::cancel_handle`].
#[derive(Clone)]
pub struct DaemonCancelHandle {
    writer: Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>,
    session_id: String,
}

impl DaemonCancelHandle {
    /// Send `CancelTurn` to the daemon. Safe to call from any task.
    pub async fn cancel(&self) {
        let cancel = FrontendMessage::CancelTurn {
            session_id: self.session_id.clone(),
        };
        if let Ok(frame) = encode_frame(&cancel) {
            let mut w = self.writer.lock().await;
            let _ = w.write_all(&frame).await;
        }
    }
}

/// Check whether a daemon is listening at the given socket path.
/// Returns `true` if a connection succeeds, `false` otherwise.
pub async fn daemon_is_running(socket_path: &Path) -> bool {
    UnixStream::connect(socket_path).await.is_ok()
}

/// Result of a `daemon status` probe.
#[derive(Debug)]
pub struct DaemonStatusInfo {
    pub running: bool,
    pub version: Option<String>,
    pub pid: Option<u32>,
}

/// Read the PID from a daemon PID file, if it exists and contains a
/// valid u32. Returns `None` if the file is missing, empty, or
/// contains non-numeric data.
pub fn read_pid_file(pid_path: &Path) -> Option<u32> {
    std::fs::read_to_string(pid_path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

/// Probe a running daemon: connect, read `DaemonReady`, disconnect.
/// Returns status info without starting a session. The PID is read
/// from the sibling `.pid` file if it exists.
pub async fn daemon_status(socket_path: &Path) -> DaemonStatusInfo {
    let pid_path = socket_path.with_extension("pid");
    let pid = read_pid_file(&pid_path);

    let stream = match UnixStream::connect(socket_path).await {
        Ok(s) => s,
        Err(_) => return DaemonStatusInfo { running: false, version: None, pid },
    };
    let (mut reader, _writer) = stream.into_split();
    let mut buf = Vec::with_capacity(4096);
    if read_more(&mut reader, &mut buf).await.is_err() {
        return DaemonStatusInfo { running: true, version: None, pid };
    }
    match decode_frame::<DaemonEnvelope>(&buf) {
        Ok((DaemonEnvelope::DaemonReady { version }, _)) => {
            DaemonStatusInfo { running: true, version: Some(version), pid }
        }
        _ => DaemonStatusInfo { running: true, version: None, pid },
    }
}

/// Send `Shutdown` to a running daemon and wait for the `ShuttingDown`
/// lifecycle event. Returns the shutdown reason on success.
pub async fn daemon_stop(socket_path: &Path) -> Result<String, DaemonError> {
    let stream = UnixStream::connect(socket_path).await?;
    let (mut reader, mut writer) = stream.into_split();
    let mut buf = Vec::with_capacity(4096);

    // Read DaemonReady.
    read_more(&mut reader, &mut buf).await?;
    match decode_frame::<DaemonEnvelope>(&buf) {
        Ok((DaemonEnvelope::DaemonReady { .. }, consumed)) => {
            buf.drain(..consumed);
        }
        Ok((other, _)) => {
            return Err(DaemonError::Protocol(format!(
                "expected DaemonReady, got {other:?}"
            )));
        }
        Err(e) => return Err(e.into()),
    }

    // Send Shutdown.
    let frame = encode_frame(&FrontendMessage::Shutdown)?;
    writer.write_all(&frame).await?;

    // Wait for ShuttingDown.
    loop {
        match decode_frame::<DaemonEnvelope>(&buf) {
            Ok((DaemonEnvelope::ShuttingDown { reason }, _)) => {
                return Ok(reason);
            }
            Ok((other, consumed)) => {
                buf.drain(..consumed);
                return Err(DaemonError::Protocol(format!(
                    "expected ShuttingDown, got {other:?}"
                )));
            }
            Err(FrameError::IncompleteBuf) => {
                read_more(&mut reader, &mut buf).await?;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// Phase 60 — fetch the daemon's current effective Persona snapshot
/// over IPC. Used by `aivyx persona show` and by the Web UI's
/// Persona pane.
pub async fn get_effective_persona(
    socket_path: &Path,
) -> Result<EffectivePersonaSummary, DaemonError> {
    let payload = send_query(socket_path, "p-show", QueryPayload::GetEffectivePersona).await?;
    match payload {
        QueryResponsePayload::GetEffectivePersona { persona } => Ok(persona),
        QueryResponsePayload::QueryError { code, message } => {
            Err(DaemonError::Protocol(format!("{code}: {message}")))
        }
        other => Err(DaemonError::Protocol(format!(
            "expected GetEffectivePersona, got {other:?}"
        ))),
    }
}

/// Phase 60 — paginated read of the persona delta chain over IPC.
/// `from_seq` is zero-based; `limit` is capped server-side at 500.
pub async fn list_persona_deltas(
    socket_path: &Path,
    from_seq: u64,
    limit: u32,
) -> Result<(Vec<PersonaDeltaSummary>, u64), DaemonError> {
    let payload = send_query(
        socket_path,
        "p-list",
        QueryPayload::ListPersonaDeltas { from_seq, limit },
    )
    .await?;
    match payload {
        QueryResponsePayload::ListPersonaDeltas { entries, total_len } => Ok((entries, total_len)),
        QueryResponsePayload::QueryError { code, message } => {
            Err(DaemonError::Protocol(format!("{code}: {message}")))
        }
        other => Err(DaemonError::Protocol(format!(
            "expected ListPersonaDeltas, got {other:?}"
        ))),
    }
}

/// Phase 74 — list every distinct memory topic over IPC.
pub async fn list_memory_topics(
    socket_path: &Path,
) -> Result<Vec<String>, DaemonError> {
    let payload =
        send_query(socket_path, "m-topics", QueryPayload::ListMemoryTopics)
            .await?;
    match payload {
        QueryResponsePayload::ListMemoryTopics { topics } => Ok(topics),
        QueryResponsePayload::QueryError { code, message } => {
            Err(DaemonError::Protocol(format!("{code}: {message}")))
        }
        other => Err(DaemonError::Protocol(format!(
            "expected ListMemoryTopics, got {other:?}"
        ))),
    }
}

/// Phase 74 — fetch up to `limit` entries for one topic.
pub async fn get_memory_topic_entries(
    socket_path: &Path,
    topic: &str,
    limit: u32,
) -> Result<Vec<MemoryEntrySummary>, DaemonError> {
    let payload = send_query(
        socket_path,
        "m-entries",
        QueryPayload::GetMemoryTopicEntries {
            topic: topic.to_string(),
            limit,
        },
    )
    .await?;
    match payload {
        QueryResponsePayload::GetMemoryTopicEntries { entries } => Ok(entries),
        QueryResponsePayload::QueryError { code, message } => {
            Err(DaemonError::Protocol(format!("{code}: {message}")))
        }
        other => Err(DaemonError::Protocol(format!(
            "expected GetMemoryTopicEntries, got {other:?}"
        ))),
    }
}

/// Phase 74 — substring search across topics + bodies over IPC.
/// Phase 75 — `semantic` requests the embedding-ranked path;
/// the returned bool is `fell_back_to_keyword` (the daemon
/// transparently downgraded to keyword).
pub async fn search_memory(
    socket_path: &Path,
    query: &str,
    limit: u32,
    semantic: bool,
) -> Result<(Vec<MemoryEntrySummary>, bool), DaemonError> {
    let payload = send_query(
        socket_path,
        "m-search",
        QueryPayload::SearchMemory {
            query: query.to_string(),
            limit,
            semantic,
        },
    )
    .await?;
    match payload {
        QueryResponsePayload::SearchMemory {
            matches,
            fell_back_to_keyword,
        } => Ok((matches, fell_back_to_keyword)),
        QueryResponsePayload::QueryError { code, message } => {
            Err(DaemonError::Protocol(format!("{code}: {message}")))
        }
        other => Err(DaemonError::Protocol(format!(
            "expected SearchMemory, got {other:?}"
        ))),
    }
}

/// Phase 78 — read-only learning-observability query. `None`
/// window → the daemon's default lookback.
pub async fn get_learning_insights(
    socket_path: &Path,
    window_secs: Option<u64>,
) -> Result<
    (
        crate::recall_insights::LearningDigest,
        Vec<crate::recall_insights::ProposalProvenance>,
        Option<crate::persona_context::PersonaSelectionStat>,
        Option<crate::proactive_detect::ProactiveStat>,
        Option<crate::persona_lifecycle::PersonaLifecycleStat>,
        Option<crate::helpfulness_ledger::AccumulatedHelpfulness>,
        Option<crate::cooccurrence_ledger::CooccurrencePatterns>,
        Option<crate::memory_recall::RecallClusterStat>,
        Option<crate::persona_consolidation::PersonaConsolidationStat>,
        Option<crate::correction_ledger::AccumulatedCorrections>,
        Option<
            crate::correction_consolidation::CorrectionConsolidationStat,
        >,
        Option<crate::correction_judgment::CorrectionJudgmentStat>,
        Option<crate::recall_judgment::RecallJudgmentStat>,
        Vec<(
            String,
            crate::reflection_scheduler::RecentReflectionStat,
        )>,
    ),
    DaemonError,
> {
    let payload = send_query(
        socket_path,
        "l-insights",
        QueryPayload::GetLearningInsights { window_secs },
    )
    .await?;
    match payload {
        QueryResponsePayload::LearningInsights {
            digest,
            proposals,
            persona_selection,
            proactive,
            persona_lifecycle,
            accumulated_helpfulness,
            cooccurrence,
            cluster_recall,
            persona_consolidation,
            accumulated_corrections,
            correction_consolidation,
            correction_judgment,
            recall_judgment,
            cadence,
        } => Ok((
            digest,
            proposals,
            persona_selection,
            proactive,
            persona_lifecycle,
            accumulated_helpfulness,
            cooccurrence,
            cluster_recall,
            persona_consolidation,
            accumulated_corrections,
            correction_consolidation,
            correction_judgment,
            recall_judgment,
            cadence,
        )),
        QueryResponsePayload::QueryError { code, message } => {
            Err(DaemonError::Protocol(format!("{code}: {message}")))
        }
        other => Err(DaemonError::Protocol(format!(
            "expected LearningInsights, got {other:?}"
        ))),
    }
}

/// Phase 102 — fetch per-tool observability stats from the daemon.
/// Backs `aivyx tools [--window <secs>]`. `window_secs = None`
/// scopes the answer to the whole audit chain.
pub async fn get_tool_stats(
    socket_path: &Path,
    window_secs: Option<u64>,
) -> Result<Vec<crate::daemon_ipc::ToolStat>, DaemonError> {
    let payload = send_query(
        socket_path,
        "t-stats",
        QueryPayload::GetToolStats { window_secs },
    )
    .await?;
    match payload {
        QueryResponsePayload::ToolStats { tools } => Ok(tools),
        QueryResponsePayload::QueryError { code, message } => {
            Err(DaemonError::Protocol(format!("{code}: {message}")))
        }
        other => Err(DaemonError::Protocol(format!(
            "expected ToolStats, got {other:?}"
        ))),
    }
}

// ---------------------------------------------------------------------------
// Phase 173 — autonomous loop control
// ---------------------------------------------------------------------------

/// Add a story to the autonomous-loop backlog. Returns the new
/// story's id.
pub async fn loop_add(
    socket_path: &Path,
    title: String,
    body: String,
    priority: Option<u32>,
) -> Result<String, DaemonError> {
    let payload = send_query(
        socket_path,
        "loop-add",
        QueryPayload::LoopAdd {
            title,
            body,
            priority,
        },
    )
    .await?;
    match payload {
        QueryResponsePayload::LoopStoryAdded { story_id } => Ok(story_id),
        QueryResponsePayload::QueryError { code, message } => {
            Err(DaemonError::Protocol(format!("{code}: {message}")))
        }
        other => Err(DaemonError::Protocol(format!(
            "expected LoopStoryAdded, got {other:?}"
        ))),
    }
}

/// List every backlog story (all statuses).
pub async fn loop_list(
    socket_path: &Path,
) -> Result<Vec<crate::loop_backlog::Story>, DaemonError> {
    let payload =
        send_query(socket_path, "loop-list", QueryPayload::LoopList).await?;
    match payload {
        QueryResponsePayload::LoopBacklog { stories } => Ok(stories),
        QueryResponsePayload::QueryError { code, message } => {
            Err(DaemonError::Protocol(format!("{code}: {message}")))
        }
        other => Err(DaemonError::Protocol(format!(
            "expected LoopBacklog, got {other:?}"
        ))),
    }
}

/// Start an autonomous-loop run. Returns `(ok, message)`.
pub async fn loop_start(
    socket_path: &Path,
    max_iterations: Option<u32>,
) -> Result<(bool, String), DaemonError> {
    let payload = send_query(
        socket_path,
        "loop-start",
        QueryPayload::LoopStart { max_iterations },
    )
    .await?;
    match payload {
        QueryResponsePayload::LoopControl { ok, message } => Ok((ok, message)),
        QueryResponsePayload::QueryError { code, message } => {
            Err(DaemonError::Protocol(format!("{code}: {message}")))
        }
        other => Err(DaemonError::Protocol(format!(
            "expected LoopControl, got {other:?}"
        ))),
    }
}

/// Request the active loop run to stop. Returns `(ok, message)`.
pub async fn loop_stop(
    socket_path: &Path,
) -> Result<(bool, String), DaemonError> {
    let payload =
        send_query(socket_path, "loop-stop", QueryPayload::LoopStop).await?;
    match payload {
        QueryResponsePayload::LoopControl { ok, message } => Ok((ok, message)),
        QueryResponsePayload::QueryError { code, message } => {
            Err(DaemonError::Protocol(format!("{code}: {message}")))
        }
        other => Err(DaemonError::Protocol(format!(
            "expected LoopControl, got {other:?}"
        ))),
    }
}

/// Read the loop run state + remaining backlog. Returns
/// `(state, remaining, armed, gate_enabled, max_run_secs,
/// max_run_tokens)`.
#[allow(clippy::type_complexity)]
pub async fn loop_status(
    socket_path: &Path,
) -> Result<
    (
        crate::loop_driver::LoopRunState,
        usize,
        bool,
        bool,
        Option<u64>,
        Option<u64>,
    ),
    DaemonError,
> {
    let payload =
        send_query(socket_path, "loop-status", QueryPayload::LoopStatus)
            .await?;
    match payload {
        QueryResponsePayload::LoopStatus {
            state,
            remaining,
            armed,
            gate_enabled,
            max_run_secs,
            max_run_tokens,
        } => Ok((
            state,
            remaining,
            armed,
            gate_enabled,
            max_run_secs,
            max_run_tokens,
        )),
        QueryResponsePayload::QueryError { code, message } => {
            Err(DaemonError::Protocol(format!("{code}: {message}")))
        }
        other => Err(DaemonError::Protocol(format!(
            "expected LoopStatus, got {other:?}"
        ))),
    }
}

/// Read the recent loop progress-log notes (most-recent-first).
pub async fn loop_log(
    socket_path: &Path,
    limit: Option<u32>,
) -> Result<Vec<String>, DaemonError> {
    let payload = send_query(
        socket_path,
        "loop-log",
        QueryPayload::LoopLog { limit },
    )
    .await?;
    match payload {
        QueryResponsePayload::LoopProgressLog { notes } => Ok(notes),
        QueryResponsePayload::QueryError { code, message } => {
            Err(DaemonError::Protocol(format!("{code}: {message}")))
        }
        other => Err(DaemonError::Protocol(format!(
            "expected LoopProgressLog, got {other:?}"
        ))),
    }
}

/// Mark a pending backlog story `Skipped`. Returns
/// `(ok, message)`.
pub async fn loop_skip(
    socket_path: &Path,
    story_id: String,
) -> Result<(bool, String), DaemonError> {
    let payload = send_query(
        socket_path,
        "loop-skip",
        QueryPayload::LoopSkip { story_id },
    )
    .await?;
    match payload {
        QueryResponsePayload::LoopControl { ok, message } => Ok((ok, message)),
        QueryResponsePayload::QueryError { code, message } => {
            Err(DaemonError::Protocol(format!("{code}: {message}")))
        }
        other => Err(DaemonError::Protocol(format!(
            "expected LoopControl, got {other:?}"
        ))),
    }
}

/// Phase 74 — operator-initiated memory topic eviction over IPC.
/// Returns the number of entries deleted on success.
pub async fn evict_memory_topic(
    socket_path: &Path,
    topic: &str,
) -> Result<u64, DaemonError> {
    let stream = UnixStream::connect(socket_path).await?;
    let (mut reader, mut writer) = stream.into_split();
    let mut buf = Vec::with_capacity(4096);
    read_more(&mut reader, &mut buf).await?;
    match decode_frame::<DaemonEnvelope>(&buf) {
        Ok((DaemonEnvelope::DaemonReady { .. }, consumed)) => {
            buf.drain(..consumed);
        }
        Ok((other, _)) => {
            return Err(DaemonError::Protocol(format!(
                "expected DaemonReady, got {other:?}"
            )))
        }
        Err(e) => return Err(e.into()),
    }
    let req = FrontendMessage::EvictMemoryTopic {
        id: "ev-cli".into(),
        topic: topic.to_string(),
    };
    let frame = encode_frame(&req)?;
    writer.write_all(&frame).await?;
    loop {
        match decode_frame::<DaemonEnvelope>(&buf) {
            Ok((
                DaemonEnvelope::MemoryEvictResolved {
                    ok, deleted, error, ..
                },
                _,
            )) => {
                if ok {
                    return deleted.ok_or_else(|| {
                        DaemonError::Protocol(
                            "MemoryEvictResolved ok=true but deleted is None"
                                .into(),
                        )
                    });
                }
                return Err(DaemonError::Protocol(
                    error.unwrap_or_else(|| "memory evict failed".into()),
                ));
            }
            Ok((other, consumed)) => {
                buf.drain(..consumed);
                return Err(DaemonError::Protocol(format!(
                    "expected MemoryEvictResolved, got {other:?}"
                )));
            }
            Err(FrameError::IncompleteBuf) => {
                read_more(&mut reader, &mut buf).await?;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// Phase 119 — operator-CLI ApplyProfileHint over IPC. Sends the
/// apply-record request after the CLI has already mutated
/// `aivyx.toml` via the Task 3 atomic primitive; the daemon's job is
/// to record the `AuditEvent::ProfileHintApplied` entry. Returns the
/// error message on a daemon-side failure (audit log unconfigured,
/// chain append error, etc.); the caller decides whether to surface
/// the audit failure as a soft warning (the file mutation already
/// landed) or as a hard error.
pub async fn apply_profile_hint(
    socket_path: &Path,
    proposal_id: &str,
    field: &str,
    applied_value: &str,
) -> Result<(), DaemonError> {
    let stream = UnixStream::connect(socket_path).await?;
    let (mut reader, mut writer) = stream.into_split();
    let mut buf = Vec::with_capacity(4096);
    read_more(&mut reader, &mut buf).await?;
    match decode_frame::<DaemonEnvelope>(&buf) {
        Ok((DaemonEnvelope::DaemonReady { .. }, consumed)) => {
            buf.drain(..consumed);
        }
        Ok((other, _)) => {
            return Err(DaemonError::Protocol(format!(
                "expected DaemonReady, got {other:?}"
            )))
        }
        Err(e) => return Err(e.into()),
    }
    let req = FrontendMessage::ApplyProfileHint {
        id: "ah-cli".into(),
        proposal_id: proposal_id.to_string(),
        field: field.to_string(),
        applied_value: applied_value.to_string(),
    };
    let frame = encode_frame(&req)?;
    writer.write_all(&frame).await?;
    loop {
        match decode_frame::<DaemonEnvelope>(&buf) {
            Ok((
                DaemonEnvelope::ProfileHintApplyAcked { ok, error, .. },
                _,
            )) => {
                if ok {
                    return Ok(());
                }
                return Err(DaemonError::Protocol(
                    error.unwrap_or_else(|| "apply-profile-hint failed".into()),
                ));
            }
            Ok((other, consumed)) => {
                buf.drain(..consumed);
                return Err(DaemonError::Protocol(format!(
                    "expected ProfileHintApplyAcked, got {other:?}"
                )));
            }
            Err(FrameError::IncompleteBuf) => {
                read_more(&mut reader, &mut buf).await?;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// Phase 119 Task 6 — operator-CLI tool-relevance ledger dump over
/// IPC. Returns the per-row dump table (one row per
/// `(keyword_key, surface_kind, identifier)` triple) optionally
/// filtered to a single keyword key.
pub async fn dump_tool_relevance(
    socket_path: &Path,
    keyword_key_filter: Option<&str>,
) -> Result<Vec<crate::daemon_ipc::ToolRelevanceDumpRow>, DaemonError> {
    let payload = send_query(
        socket_path,
        "tr-dump",
        QueryPayload::DumpToolRelevance {
            keyword_key_filter: keyword_key_filter.map(str::to_string),
        },
    )
    .await?;
    match payload {
        QueryResponsePayload::ToolRelevanceDump { rows } => Ok(rows),
        QueryResponsePayload::QueryError { code, message } => {
            Err(DaemonError::Protocol(format!("{code}: {message}")))
        }
        other => Err(DaemonError::Protocol(format!(
            "expected ToolRelevanceDump, got {other:?}"
        ))),
    }
}

/// Phase 119 — operator-CLI ImportRoleDraft over IPC. Mirrors
/// `apply_profile_hint` for the second Phase 118 category.
pub async fn import_role_draft(
    socket_path: &Path,
    proposal_id: &str,
    role_name: &str,
    parent: Option<&str>,
) -> Result<(), DaemonError> {
    let stream = UnixStream::connect(socket_path).await?;
    let (mut reader, mut writer) = stream.into_split();
    let mut buf = Vec::with_capacity(4096);
    read_more(&mut reader, &mut buf).await?;
    match decode_frame::<DaemonEnvelope>(&buf) {
        Ok((DaemonEnvelope::DaemonReady { .. }, consumed)) => {
            buf.drain(..consumed);
        }
        Ok((other, _)) => {
            return Err(DaemonError::Protocol(format!(
                "expected DaemonReady, got {other:?}"
            )))
        }
        Err(e) => return Err(e.into()),
    }
    let req = FrontendMessage::ImportRoleDraft {
        id: "ir-cli".into(),
        proposal_id: proposal_id.to_string(),
        role_name: role_name.to_string(),
        parent: parent.map(str::to_string),
    };
    let frame = encode_frame(&req)?;
    writer.write_all(&frame).await?;
    loop {
        match decode_frame::<DaemonEnvelope>(&buf) {
            Ok((
                DaemonEnvelope::RoleDraftImportAcked { ok, error, .. },
                _,
            )) => {
                if ok {
                    return Ok(());
                }
                return Err(DaemonError::Protocol(
                    error.unwrap_or_else(|| "import-role-draft failed".into()),
                ));
            }
            Ok((other, consumed)) => {
                buf.drain(..consumed);
                return Err(DaemonError::Protocol(format!(
                    "expected RoleDraftImportAcked, got {other:?}"
                )));
            }
            Err(FrameError::IncompleteBuf) => {
                read_more(&mut reader, &mut buf).await?;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// Phase 73 — paginated notification history walk over IPC.
/// Returns the `(entries, total_len)` pair from
/// `QueryPayload::ListNotificationHistory`.
pub async fn list_notification_history(
    socket_path: &Path,
    from_seq: u64,
    limit: u32,
    target_filter: Option<&str>,
) -> Result<(Vec<NotificationHistoryEntry>, u64), DaemonError> {
    let payload = send_query(
        socket_path,
        "n-hist",
        QueryPayload::ListNotificationHistory {
            from_seq,
            limit,
            target_filter: target_filter.map(str::to_string),
        },
    )
    .await?;
    match payload {
        QueryResponsePayload::ListNotificationHistory { entries, total_len } => {
            Ok((entries, total_len))
        }
        QueryResponsePayload::QueryError { code, message } => {
            Err(DaemonError::Protocol(format!("{code}: {message}")))
        }
        other => Err(DaemonError::Protocol(format!(
            "expected ListNotificationHistory, got {other:?}"
        ))),
    }
}

/// Phase 70 — list pending and resolved Persona proposals over
/// IPC. `status_filter` is the same string the wire envelope
/// expects: `"all" | "pending" | "approved" | "rejected" |
/// "superseded"`; unknown values fall through to `"pending"`
/// server-side.
pub async fn list_persona_proposals(
    socket_path: &Path,
    status_filter: &str,
    limit: u32,
) -> Result<(Vec<PersonaProposalSummary>, u64), DaemonError> {
    let payload = send_query(
        socket_path,
        "pp-list",
        QueryPayload::ListPersonaProposals {
            status_filter: status_filter.to_string(),
            limit,
        },
    )
    .await?;
    match payload {
        QueryResponsePayload::ListPersonaProposals {
            proposals,
            total_len,
        } => Ok((proposals, total_len)),
        QueryResponsePayload::QueryError { code, message } => {
            Err(DaemonError::Protocol(format!("{code}: {message}")))
        }
        other => Err(DaemonError::Protocol(format!(
            "expected ListPersonaProposals, got {other:?}"
        ))),
    }
}

/// Phase 70 — fetch a single Persona proposal by id.
pub async fn get_persona_proposal(
    socket_path: &Path,
    proposal_id: &str,
) -> Result<Option<PersonaProposalSummary>, DaemonError> {
    let payload = send_query(
        socket_path,
        "pp-get",
        QueryPayload::GetPersonaProposal {
            proposal_id: proposal_id.to_string(),
        },
    )
    .await?;
    match payload {
        QueryResponsePayload::GetPersonaProposal { proposal } => Ok(proposal),
        QueryResponsePayload::QueryError { code, message } => {
            Err(DaemonError::Protocol(format!("{code}: {message}")))
        }
        other => Err(DaemonError::Protocol(format!(
            "expected GetPersonaProposal, got {other:?}"
        ))),
    }
}

/// Phase 70 — operator-initiated proposal resolution over IPC.
/// Sends a `ResolvePersonaProposal` frame and blocks for the
/// matching `PersonaProposalResolved` reply.
pub async fn resolve_persona_proposal(
    socket_path: &Path,
    proposal_id: &str,
    resolution: PersonaProposalResolution,
) -> Result<PersonaProposalResolveSuccess, DaemonError> {
    let stream = UnixStream::connect(socket_path).await?;
    let (mut reader, mut writer) = stream.into_split();
    let mut buf = Vec::with_capacity(4096);
    read_more(&mut reader, &mut buf).await?;
    match decode_frame::<DaemonEnvelope>(&buf) {
        Ok((DaemonEnvelope::DaemonReady { .. }, consumed)) => {
            buf.drain(..consumed);
        }
        Ok((other, _)) => {
            return Err(DaemonError::Protocol(format!(
                "expected DaemonReady, got {other:?}"
            )))
        }
        Err(e) => return Err(e.into()),
    }
    let req = FrontendMessage::ResolvePersonaProposal {
        id: "rsp-cli".into(),
        proposal_id: proposal_id.to_string(),
        resolution,
    };
    let frame = encode_frame(&req)?;
    writer.write_all(&frame).await?;
    loop {
        match decode_frame::<DaemonEnvelope>(&buf) {
            Ok((
                DaemonEnvelope::PersonaProposalResolved {
                    ok, success, error, ..
                },
                _,
            )) => {
                if ok {
                    return success.ok_or_else(|| {
                        DaemonError::Protocol(
                            "PersonaProposalResolved ok=true but success is None"
                                .into(),
                        )
                    });
                }
                return Err(DaemonError::Protocol(
                    error.unwrap_or_else(|| "proposal resolution failed".into()),
                ));
            }
            Ok((other, consumed)) => {
                buf.drain(..consumed);
                return Err(DaemonError::Protocol(format!(
                    "expected PersonaProposalResolved, got {other:?}"
                )));
            }
            Err(FrameError::IncompleteBuf) => {
                read_more(&mut reader, &mut buf).await?;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// Phase 64 Task 3 — full-fidelity Persona chain dump over IPC.
/// Single-shot response (no pagination). Used by
/// `aivyx identity export` to read the chain into memory before
/// writing the export bundle to disk. Returns the chain in order
/// and the effective state at fetch time.
pub async fn export_persona_chain(
    socket_path: &Path,
) -> Result<
    (
        Vec<crate::identity_export::DeltaExport>,
        crate::persona::EffectivePersona,
    ),
    DaemonError,
> {
    let payload = send_query(socket_path, "p-export", QueryPayload::ExportPersonaChain).await?;
    match payload {
        QueryResponsePayload::ExportPersonaChain { deltas, effective } => {
            Ok((deltas, effective))
        }
        QueryResponsePayload::QueryError { code, message } => {
            Err(DaemonError::Protocol(format!("{code}: {message}")))
        }
        other => Err(DaemonError::Protocol(format!(
            "expected ExportPersonaChain, got {other:?}"
        ))),
    }
}

/// Phase 60 — operator-initiated Persona delta revert over IPC.
/// Returns the chain sequence number of the appended Revert delta
/// on success.
pub async fn revert_persona_delta(
    socket_path: &Path,
    target_delta_id: &str,
) -> Result<u64, DaemonError> {
    let stream = UnixStream::connect(socket_path).await?;
    let (mut reader, mut writer) = stream.into_split();
    let mut buf = Vec::with_capacity(4096);
    read_more(&mut reader, &mut buf).await?;
    match decode_frame::<DaemonEnvelope>(&buf) {
        Ok((DaemonEnvelope::DaemonReady { .. }, consumed)) => {
            buf.drain(..consumed);
        }
        Ok((other, _)) => {
            return Err(DaemonError::Protocol(format!(
                "expected DaemonReady, got {other:?}"
            )))
        }
        Err(e) => return Err(e.into()),
    }
    let req = FrontendMessage::RevertPersonaDelta {
        id: "rv-cli".into(),
        target_delta_id: target_delta_id.to_string(),
    };
    let frame = encode_frame(&req)?;
    writer.write_all(&frame).await?;
    loop {
        match decode_frame::<DaemonEnvelope>(&buf) {
            Ok((
                DaemonEnvelope::PersonaRevertResolved {
                    ok, seq, error, ..
                },
                _,
            )) => {
                if ok {
                    return seq.ok_or_else(|| {
                        DaemonError::Protocol(
                            "PersonaRevertResolved ok=true but seq is None".into(),
                        )
                    });
                }
                return Err(DaemonError::Protocol(
                    error.unwrap_or_else(|| "persona revert failed".into()),
                ));
            }
            Ok((other, consumed)) => {
                buf.drain(..consumed);
                return Err(DaemonError::Protocol(format!(
                    "expected PersonaRevertResolved, got {other:?}"
                )));
            }
            Err(FrameError::IncompleteBuf) => {
                read_more(&mut reader, &mut buf).await?;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// Phase 65 — operator-driven Persona chain import over IPC.
/// Closes the Phase 60 identity-deferral end to end. Sends the
/// parsed export bundle to the daemon for replay against the
/// local store. The daemon refuses on a non-empty chain unless
/// `force` is set, then wipes and replays. On success returns
/// the daemon's `PersonaImportSuccess` payload with
/// `deltas_imported` + `final_chain_seq` per Q4(a).
pub async fn import_persona_chain(
    socket_path: &Path,
    deltas: Vec<crate::identity_export::DeltaExport>,
    effective_at_export: crate::persona::EffectivePersona,
    force: bool,
) -> Result<crate::daemon_ipc::PersonaImportSuccess, DaemonError> {
    let stream = UnixStream::connect(socket_path).await?;
    let (mut reader, mut writer) = stream.into_split();
    let mut buf = Vec::with_capacity(4096);
    read_more(&mut reader, &mut buf).await?;
    match decode_frame::<DaemonEnvelope>(&buf) {
        Ok((DaemonEnvelope::DaemonReady { .. }, consumed)) => {
            buf.drain(..consumed);
        }
        Ok((other, _)) => {
            return Err(DaemonError::Protocol(format!(
                "expected DaemonReady, got {other:?}"
            )))
        }
        Err(e) => return Err(e.into()),
    }
    let req = FrontendMessage::ImportPersonaChain {
        id: "im-cli".into(),
        deltas,
        // Phase 118 — boxed at the IPC boundary; see
        // FrontendMessage::ImportPersonaChain field doc.
        effective_at_export: Box::new(effective_at_export),
        force,
    };
    let frame = encode_frame(&req)?;
    writer.write_all(&frame).await?;
    loop {
        match decode_frame::<DaemonEnvelope>(&buf) {
            Ok((
                DaemonEnvelope::PersonaImportResolved {
                    ok, success, error, ..
                },
                _,
            )) => {
                if ok {
                    return success.ok_or_else(|| {
                        DaemonError::Protocol(
                            "PersonaImportResolved ok=true but success is None".into(),
                        )
                    });
                }
                return Err(DaemonError::Protocol(
                    error.unwrap_or_else(|| "persona import failed".into()),
                ));
            }
            Ok((other, consumed)) => {
                buf.drain(..consumed);
                return Err(DaemonError::Protocol(format!(
                    "expected PersonaImportResolved, got {other:?}"
                )));
            }
            Err(FrameError::IncompleteBuf) => {
                read_more(&mut reader, &mut buf).await?;
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// Shared helper: connect, send a `Query`, return the response
/// payload. Used by the Persona inspection helpers above.
async fn send_query(
    socket_path: &Path,
    id: &str,
    query: QueryPayload,
) -> Result<QueryResponsePayload, DaemonError> {
    let stream = UnixStream::connect(socket_path).await?;
    let (mut reader, mut writer) = stream.into_split();
    let mut buf = Vec::with_capacity(4096);
    read_more(&mut reader, &mut buf).await?;
    match decode_frame::<DaemonEnvelope>(&buf) {
        Ok((DaemonEnvelope::DaemonReady { .. }, consumed)) => buf.drain(..consumed),
        Ok((other, _)) => {
            return Err(DaemonError::Protocol(format!(
                "expected DaemonReady, got {other:?}"
            )))
        }
        Err(e) => return Err(e.into()),
    };
    let req = FrontendMessage::Query {
        id: id.to_string(),
        payload: query,
    };
    let frame = encode_frame(&req)?;
    writer.write_all(&frame).await?;
    loop {
        match decode_frame::<DaemonEnvelope>(&buf) {
            Ok((DaemonEnvelope::QueryResponse { payload, .. }, _)) => return Ok(payload),
            Ok((other, consumed)) => {
                buf.drain(..consumed);
                return Err(DaemonError::Protocol(format!(
                    "expected QueryResponse, got {other:?}"
                )));
            }
            Err(FrameError::IncompleteBuf) => read_more(&mut reader, &mut buf).await?,
            Err(e) => return Err(e.into()),
        }
    }
}

/// The auto-spawned daemon's log file: a sibling `daemon.log` next to
/// the socket (and the `daemon.pid`), so the three live together in
/// the runtime dir.
fn daemon_log_path(socket_path: &Path) -> PathBuf {
    socket_path.with_extension("log")
}

/// Open the sibling `daemon.log` for the auto-spawned daemon's
/// stdout + stderr (created if missing, appended otherwise). Falls
/// back to discarding output if the log cannot be opened, so a
/// logging problem never blocks the daemon from starting.
fn daemon_log_stdio(socket_path: &Path) -> (std::process::Stdio, std::process::Stdio) {
    let log_path = daemon_log_path(socket_path);
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        // Two independent handles so stdout and stderr can be written
        // concurrently without sharing one offset cursor.
        Ok(file) => match file.try_clone() {
            Ok(file2) => (file.into(), file2.into()),
            Err(_) => (file.into(), std::process::Stdio::null()),
        },
        Err(_) => (std::process::Stdio::null(), std::process::Stdio::null()),
    }
}

/// Spawn a daemon process in the background and wait for its socket
/// to appear. Returns the socket path on success.
///
/// Uses `tokio::process::Command` to launch `aivyx daemon run` as a
/// detached child. The daemon's stdout/stderr are redirected to a
/// sibling `daemon.log` rather than inherited: a background daemon
/// must not print onto the launching terminal — in the TUI the
/// startup banner bleeds under the alternate screen, and in the REPL
/// it interleaves with the prompt. The banner + ongoing logs stay
/// recoverable in the log file. (A direct `aivyx daemon run` is
/// unaffected — it does not go through this path and keeps writing to
/// the operator's terminal.)
pub async fn spawn_daemon_and_wait(
    socket_path: &Path,
    timeout: Duration,
) -> Result<PathBuf, DaemonError> {
    let exe = std::env::current_exe()?;

    let (stdout, stderr) = daemon_log_stdio(socket_path);
    let _child = tokio::process::Command::new(&exe)
        .args(["daemon", "run"])
        .stdin(std::process::Stdio::null())
        .stdout(stdout)
        .stderr(stderr)
        .spawn()
        .map_err(|e| DaemonError::Internal(format!("failed to spawn daemon: {e}")))?;

    // Poll for the socket to appear with exponential backoff.
    let start = tokio::time::Instant::now();
    let mut delay = Duration::from_millis(20);
    loop {
        if daemon_is_running(socket_path).await {
            return Ok(socket_path.to_path_buf());
        }
        if start.elapsed() > timeout {
            return Err(DaemonError::Internal(format!(
                "daemon did not start within {}ms — socket not found at {}",
                timeout.as_millis(),
                socket_path.display(),
            )));
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_millis(500));
    }
}

// -----------------------------------------------------------------------
// Phase 16 backward-compatible single-turn client
// -----------------------------------------------------------------------

/// Connect to the daemon, start a session, submit one input, and
/// collect all streamed events until `TurnComplete`.
pub async fn run_poc_client(
    socket_path: &Path,
    role: Option<String>,
    input_text: String,
) -> Result<DaemonTurnResult, DaemonError> {
    let mut session = DaemonSession::connect(socket_path, role, None).await?;
    let daemon_version = session.daemon_version.clone();
    let session_id = session.session_id.clone();

    let (events, outcome) = session.submit_input(input_text).await?;
    let _ = session.disconnect().await;

    Ok(DaemonTurnResult {
        session_id,
        events,
        outcome,
        daemon_version,
    })
}

async fn read_more(
    reader: &mut tokio::net::unix::OwnedReadHalf,
    buf: &mut Vec<u8>,
) -> Result<(), DaemonError> {
    let mut tmp = [0u8; 4096];
    let n = reader.read(&mut tmp).await?;
    if n == 0 {
        return Err(DaemonError::Protocol(
            "connection closed unexpectedly".into(),
        ));
    }
    buf.extend_from_slice(&tmp[..n]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon_ipc::{encode_frame, DaemonEnvelope};
    use tokio::net::UnixListener;

    /// A fake daemon that performs the lifecycle handshake while
    /// injecting a `RecoveryNotice` between `DaemonReady` and
    /// `SessionStarted` — exactly what a real daemon sends to the
    /// first frontend that connects after an unclean restart. `connect`
    /// must skip the notice and still succeed.
    #[tokio::test]
    async fn connect_skips_recovery_notice_in_the_handshake() {
        let sock = std::env::temp_dir()
            .join(format!("aivyx-recov-{}.sock", uuid::Uuid::new_v4()));
        let listener = UnixListener::bind(&sock).expect("bind fake daemon");

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            for env in [
                DaemonEnvelope::DaemonReady {
                    version: "0.1".into(),
                },
                DaemonEnvelope::RecoveryNotice {
                    lost_sessions: vec!["old-session".into()],
                    lost_turns: vec!["old-turn".into()],
                    stale_since: 42,
                },
                DaemonEnvelope::SessionStarted {
                    session_id: "sess-recovered".into(),
                },
            ] {
                let frame = encode_frame(&env).expect("encode");
                stream.write_all(&frame).await.expect("write frame");
            }
            // Drain the client's StartSession so the socket stays open
            // until the client has finished the handshake.
            let mut tmp = [0u8; 1024];
            let _ = stream.read(&mut tmp).await;
        });

        let session = DaemonSession::connect(&sock, None, None)
            .await
            .expect("connect must succeed despite the RecoveryNotice");
        assert_eq!(session.session_id, "sess-recovered");
        assert_eq!(session.daemon_version.as_deref(), Some("0.1"));

        let _ = session.disconnect().await;
        let _ = server.await;
        let _ = std::fs::remove_file(&sock);
    }

    #[test]
    fn daemon_log_path_is_sibling_of_socket_and_pid() {
        let socket = Path::new("/run/user/1000/aivyx/daemon.sock");
        let log = daemon_log_path(socket);
        assert_eq!(log, Path::new("/run/user/1000/aivyx/daemon.log"));
        // Lives alongside the pid file (same `with_extension` rule the
        // server + status probe use), so the runtime dir holds the
        // socket, pid, and log together.
        assert_eq!(log.parent(), socket.with_extension("pid").parent());
        assert_eq!(log.file_name().unwrap(), "daemon.log");
    }

    #[test]
    fn daemon_log_stdio_opens_a_log_under_a_temp_runtime_dir() {
        // The auto-spawn redirect must create the log (and its parent
        // dir if missing) so the daemon never bleeds onto the
        // launching terminal. Use a throwaway dir under the temp root.
        let base = std::env::temp_dir().join(format!(
            "aivyx-daemon-log-test-{}",
            uuid::Uuid::new_v4()
        ));
        let socket = base.join("nested").join("daemon.sock");
        assert!(!base.exists());

        // Should not panic and should materialize the log + parents.
        let _stdio = daemon_log_stdio(&socket);
        let log = daemon_log_path(&socket);
        assert!(log.exists(), "daemon.log was created: {}", log.display());

        let _ = std::fs::remove_dir_all(&base);
    }
}
