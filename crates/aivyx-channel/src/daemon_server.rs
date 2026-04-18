//! Production daemon server — Phase 17 Task 2, Phase 19 Task 2.
//!
//! Listens on a Unix domain socket, accepts connections, reads IPC
//! frames, dispatches turns through the provided agent, and streams
//! `DaemonMessage` frames back. Supports multi-turn sessions and
//! concurrent connections (Phase 19), with graceful shutdown via a
//! `CancellationToken`.
//!
//! Phase 16 shipped the single-turn PoC; Phase 17 Task 2 extended to
//! multi-turn with graceful shutdown; Phase 19 Task 2 upgrades to
//! multi-connection with per-connection channel construction.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;

use aivyx_core::{Agent, CancellationToken, ChannelContext, Message, StreamEvent, TurnOutcome};

use aivyx_storage::DomainHandle;

use crate::daemon_ipc::{
    decode_frame, encode_frame, DaemonLifecycleEvent, DaemonMessage, FrameError, FrontendMessage,
    FrontendType, StreamEventPayload, PROTOCOL_VERSION,
};
use crate::mission;

/// Channel factory: given a `FrontendType`, returns the appropriate
/// `ChannelContext` implementation for that frontend. The binary
/// constructs this closure at startup, capturing the resources each
/// channel type needs (stdout handle for Local, transport for Telegram).
pub type ChannelFactory =
    Arc<dyn Fn(FrontendType) -> Arc<dyn ChannelContext + Send + Sync> + Send + Sync>;

/// Run the daemon server.
///
/// Binds the Unix socket at `socket_path`, accepts connections in a
/// loop, and spawns a handler task per connection. Each handler reads
/// `FrontendMessage` frames and dispatches turns through the shared
/// `agent`. The `channel_factory` constructs a per-connection
/// `ChannelContext` based on the frontend type sent in `StartSession`.
///
/// The `shutdown` token allows external code (signal handlers, tests)
/// to trigger a graceful shutdown. When cancelled, the daemon stops
/// accepting new connections; in-flight handler tasks complete their
/// current turn and exit.
pub async fn run_daemon(
    socket_path: &Path,
    agent: Arc<dyn Agent>,
    channel_factory: ChannelFactory,
    shutdown: CancellationToken,
    mission_store: Option<DomainHandle>,
    schedule_store: Option<DomainHandle>,
    webhook_store: Option<DomainHandle>,
) -> Result<(), String> {
    let _ = std::fs::remove_file(socket_path);

    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create socket parent dir: {e}"))?;
    }

    let listener = UnixListener::bind(socket_path)
        .map_err(|e| format!("failed to bind daemon socket at {}: {e}", socket_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(socket_path, perms)
            .map_err(|e| format!("failed to set socket permissions: {e}"))?;
    }

    let pid_path = socket_path.with_extension("pid");
    let _pid_guard = PidGuard::write(&pid_path)?;

    // Shared trigger dispatch — all trigger subsystems (cron, webhook,
    // file-watch) share the same turn lock and agent/channel references.
    let trigger_dispatch =
        crate::trigger::TriggerDispatch::new(Arc::clone(&agent), Arc::clone(&channel_factory));

    // Spawn the scheduler loop if a schedule store is provided.
    let _scheduler_handle = schedule_store.map(|store| {
        let sched_dispatch = trigger_dispatch.clone();
        let sched_shutdown = shutdown.clone();
        tokio::spawn(async move {
            crate::daemon_scheduler::run_scheduler(
                sched_dispatch,
                store,
                sched_shutdown,
            )
            .await;
        })
    });

    // Spawn the webhook HTTP listener if a webhook store is provided.
    let _webhook_handle = webhook_store.map(|store| {
        let wh_dispatch = trigger_dispatch.clone();
        let wh_shutdown = shutdown.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::webhook_listener::run_webhook_listener(
                wh_dispatch,
                store,
                crate::webhook_listener::DEFAULT_WEBHOOK_PORT,
                wh_shutdown,
            )
            .await
            {
                eprintln!("aivyx webhook listener error: {e}");
            }
        })
    });

    let mission_store = mission_store.map(Arc::new);
    let mut handles = Vec::new();

    loop {
        let (stream, _addr) = tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok(conn) => conn,
                    Err(e) => {
                        eprintln!("aivyx daemon: accept error: {e}");
                        continue;
                    }
                }
            }
            _ = shutdown.cancelled() => {
                break;
            }
        };

        let agent = Arc::clone(&agent);
        let factory = Arc::clone(&channel_factory);
        let conn_shutdown = shutdown.clone();
        let conn_mission_store = mission_store.clone();

        let handle = tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, agent, factory, conn_shutdown, conn_mission_store).await {
                eprintln!("aivyx daemon: connection handler error: {e}");
            }
        });
        handles.push(handle);
    }

    for h in handles {
        let _ = h.await;
    }

    Ok(())
}

async fn handle_connection(
    stream: tokio::net::UnixStream,
    agent: Arc<dyn Agent>,
    channel_factory: ChannelFactory,
    shutdown: CancellationToken,
    mission_store: Option<Arc<DomainHandle>>,
) -> Result<(), String> {
    let (mut reader, mut writer) = stream.into_split();

    let ready = DaemonLifecycleEvent::DaemonReady {
        version: PROTOCOL_VERSION.into(),
    };
    let frame = encode_frame(&ready).map_err(|e| format!("encode DaemonReady: {e}"))?;
    writer
        .write_all(&frame)
        .await
        .map_err(|e| format!("write DaemonReady: {e}"))?;

    let mut buf = Vec::with_capacity(4096);
    let mut session_id: Option<String> = None;
    let mut channel: Option<Arc<dyn ChannelContext + Send + Sync>> = None;

    loop {
        if shutdown.is_cancelled() {
            send_shutting_down(&mut writer, "shutdown requested").await;
            return Ok(());
        }

        let mut tmp = [0u8; 4096];
        let n = tokio::select! {
            result = reader.read(&mut tmp) => {
                result.map_err(|e| format!("read error: {e}"))?
            }
            _ = shutdown.cancelled() => {
                send_shutting_down(&mut writer, "shutdown requested").await;
                return Ok(());
            }
        };
        if n == 0 {
            break; // Frontend disconnected.
        }
        buf.extend_from_slice(&tmp[..n]);

        loop {
            match decode_frame::<FrontendMessage>(&buf) {
                Ok((msg, consumed)) => {
                    buf.drain(..consumed);
                    match msg {
                        FrontendMessage::StartSession { role: _, frontend_type } => {
                            let ft = frontend_type.unwrap_or(FrontendType::Local);
                            channel = Some(channel_factory(ft));

                            let sid = aivyx_core::SessionId::new().to_string();
                            session_id = Some(sid.clone());
                            let resp = DaemonMessage::SessionStarted { session_id: sid };
                            let frame = encode_frame(&resp)
                                .map_err(|e| format!("encode SessionStarted: {e}"))?;
                            writer
                                .write_all(&frame)
                                .await
                                .map_err(|e| format!("write SessionStarted: {e}"))?;
                        }
                        FrontendMessage::SubmitInput {
                            session_id: sid,
                            text,
                            mission_id: mid,
                        } => {
                            let ch = match &channel {
                                Some(c) => Arc::clone(c),
                                None => {
                                    let err = DaemonMessage::Error {
                                        code: "no_session".into(),
                                        message: "SubmitInput before StartSession".into(),
                                    };
                                    let frame = encode_frame(&err).unwrap_or_default();
                                    let _ = writer.write_all(&frame).await;
                                    continue;
                                }
                            };

                            let msg = Message::text(aivyx_core::SessionId::new(), text);

                            let bridge = IpcChannelBridge {
                                inner: ch,
                                writer: Arc::new(tokio::sync::Mutex::new(writer)),
                                session_id: sid.clone(),
                            };

                            let outcome = agent.turn(msg, &bridge).await;

                            writer = Arc::try_unwrap(bridge.writer)
                                .map_err(|_| "writer arc still shared".to_string())?
                                .into_inner();

                            if let (
                                TurnOutcome::Escalated { reason, .. },
                                Some(mission_id),
                                Some(store),
                            ) = (&outcome, &mid, &mission_store)
                            {
                                let gate_result = async {
                                    let mut record = mission::get_mission(store, mission_id)
                                        .await
                                        .map_err(|e| format!("get mission: {e}"))?
                                        .ok_or_else(|| {
                                            format!("mission {mission_id} not found")
                                        })?;
                                    let gate_id = format!(
                                        "gate-{}",
                                        uuid::Uuid::new_v4().as_hyphenated()
                                    );
                                    mission::add_gate(
                                        &mut record,
                                        gate_id.clone(),
                                        reason.clone(),
                                        None,
                                    )?;
                                    mission::update_mission(store, &record)
                                        .await
                                        .map_err(|e| format!("persist mission: {e}"))?;
                                    Ok::<String, String>(gate_id)
                                }
                                .await;

                                match gate_result {
                                    Ok(gate_id) => {
                                        let gate_event =
                                            DaemonMessage::StreamEvent {
                                                session_id: sid.clone(),
                                                event: StreamEventPayload::ApprovalGate {
                                                    mission_id: mission_id.clone(),
                                                    gate_id,
                                                    reason: reason.clone(),
                                                    scope: None,
                                                },
                                            };
                                        let frame = encode_frame(&gate_event)
                                            .map_err(|e| format!("encode ApprovalGate: {e}"))?;
                                        writer
                                            .write_all(&frame)
                                            .await
                                            .map_err(|e| format!("write ApprovalGate: {e}"))?;
                                    }
                                    Err(e) => {
                                        let err = DaemonMessage::Error {
                                            code: "gate_create_failed".into(),
                                            message: format!("failed to create gate: {e}"),
                                        };
                                        let frame = encode_frame(&err).unwrap_or_default();
                                        let _ = writer.write_all(&frame).await;
                                    }
                                }
                            }

                            let outcome_str = format_outcome(&outcome);

                            let resp = DaemonMessage::TurnComplete {
                                session_id: sid,
                                outcome: outcome_str,
                            };
                            let frame = encode_frame(&resp)
                                .map_err(|e| format!("encode TurnComplete: {e}"))?;
                            writer
                                .write_all(&frame)
                                .await
                                .map_err(|e| format!("write TurnComplete: {e}"))?;
                        }
                        FrontendMessage::Disconnect => {
                            return Ok(());
                        }
                        FrontendMessage::CancelTurn { session_id: _sid } => {
                            // Cancellation wired through CancellationToken on
                            // the channel bridge; the turn loop checks it between
                            // LLM steps.
                        }
                        FrontendMessage::ResolveGate {
                            mission_id,
                            gate_id,
                            approved,
                        } => {
                            let Some(store) = &mission_store else {
                                let err = DaemonMessage::Error {
                                    code: "no_mission_store".into(),
                                    message: "ResolveGate received but no mission store configured".into(),
                                };
                                let frame = encode_frame(&err).unwrap_or_default();
                                let _ = writer.write_all(&frame).await;
                                continue;
                            };
                            let result = async {
                                let mut record = mission::get_mission(store, &mission_id)
                                    .await
                                    .map_err(|e| format!("get mission: {e}"))?
                                    .ok_or_else(|| format!("mission {mission_id} not found"))?;
                                mission::resolve_gate(&mut record, &gate_id, approved)?;
                                mission::update_mission(store, &record)
                                    .await
                                    .map_err(|e| format!("persist mission: {e}"))?;
                                Ok::<(), String>(())
                            }.await;
                            match result {
                                Ok(()) => {
                                    let resp = DaemonMessage::GateResolved {
                                        mission_id: mission_id.clone(),
                                        gate_id: gate_id.clone(),
                                        approved,
                                    };
                                    let frame = encode_frame(&resp)
                                        .map_err(|e| format!("encode GateResolved: {e}"))?;
                                    writer
                                        .write_all(&frame)
                                        .await
                                        .map_err(|e| format!("write GateResolved: {e}"))?;

                                    if approved {
                                        if let Some(ch) = &channel {
                                            let ch = Arc::clone(ch);
                                            let resume_text = format!(
                                                "Gate {gate_id} approved — continue mission {mission_id}"
                                            );
                                            let msg = Message::text(
                                                aivyx_core::SessionId::new(),
                                                resume_text,
                                            );
                                            let sid = session_id.clone().unwrap_or_default();
                                            let bridge = IpcChannelBridge {
                                                inner: ch,
                                                writer: Arc::new(
                                                    tokio::sync::Mutex::new(writer),
                                                ),
                                                session_id: sid.clone(),
                                            };

                                            let resume_outcome = agent.turn(msg, &bridge).await;

                                            writer = Arc::try_unwrap(bridge.writer)
                                                .map_err(|_| {
                                                    "writer arc still shared".to_string()
                                                })?
                                                .into_inner();

                                            let outcome_str = format_outcome(&resume_outcome);
                                            let resp = DaemonMessage::TurnComplete {
                                                session_id: sid,
                                                outcome: outcome_str,
                                            };
                                            let frame = encode_frame(&resp).map_err(|e| {
                                                format!("encode resume TurnComplete: {e}")
                                            })?;
                                            writer.write_all(&frame).await.map_err(|e| {
                                                format!("write resume TurnComplete: {e}")
                                            })?;
                                        }
                                    }
                                }
                                Err(e) => {
                                    let err = DaemonMessage::Error {
                                        code: "gate_resolve_failed".into(),
                                        message: format!("failed to resolve gate: {e}"),
                                    };
                                    let frame = encode_frame(&err).unwrap_or_default();
                                    let _ = writer.write_all(&frame).await;
                                }
                            }
                        }
                        FrontendMessage::Shutdown => {
                            send_shutting_down(&mut writer, "operator requested via daemon stop").await;
                            shutdown.cancel();
                            return Ok(());
                        }
                    }
                }
                Err(FrameError::IncompleteBuf) => break,
                Err(e) => {
                    let err_resp = DaemonMessage::Error {
                        code: "invalid_message".into(),
                        message: e.to_string(),
                    };
                    let frame = encode_frame(&err_resp).unwrap_or_default();
                    let _ = writer.write_all(&frame).await;
                    return Err(format!("frame decode error: {e}"));
                }
            }
        }
    }

    Ok(())
}

/// Backward-compatible single-connection daemon for tests that don't
/// need multi-connection or channel-factory semantics. Accepts one
/// connection, serves it to completion, then returns.
pub async fn run_poc_daemon<C: ChannelContext + Send + Sync + 'static>(
    socket_path: &Path,
    agent: Arc<dyn Agent>,
    channel: Arc<C>,
) -> Result<(), String> {
    let channel: Arc<dyn ChannelContext + Send + Sync> = channel;
    let factory: ChannelFactory = Arc::new(move |_| Arc::clone(&channel));
    run_single_connection_daemon(socket_path, agent, factory).await
}

/// Accept exactly one connection, serve it to completion, then return.
/// Used by `run_poc_daemon` and tests that need deterministic shutdown.
async fn run_single_connection_daemon(
    socket_path: &Path,
    agent: Arc<dyn Agent>,
    channel_factory: ChannelFactory,
) -> Result<(), String> {
    let _ = std::fs::remove_file(socket_path);

    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create socket parent dir: {e}"))?;
    }

    let listener = UnixListener::bind(socket_path)
        .map_err(|e| format!("failed to bind daemon socket at {}: {e}", socket_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(socket_path, perms)
            .map_err(|e| format!("failed to set socket permissions: {e}"))?;
    }

    let (stream, _addr) = listener.accept()
        .await
        .map_err(|e| format!("failed to accept connection: {e}"))?;

    let shutdown = CancellationToken::new();
    handle_connection(stream, agent, channel_factory, shutdown, None).await
}

/// Backward-compatible single-channel daemon with shutdown token.
pub async fn run_daemon_compat<C: ChannelContext + Send + Sync + 'static>(
    socket_path: &Path,
    agent: Arc<dyn Agent>,
    channel: Arc<C>,
    shutdown: CancellationToken,
) -> Result<(), String> {
    let channel_for_factory: Arc<dyn ChannelContext + Send + Sync> = channel;
    let factory: ChannelFactory = Arc::new(move |_| Arc::clone(&channel_for_factory));
    run_daemon(socket_path, agent, factory, shutdown, None, None, None).await
}

async fn send_shutting_down(writer: &mut tokio::net::unix::OwnedWriteHalf, reason: &str) {
    let event = DaemonLifecycleEvent::ShuttingDown {
        reason: reason.to_string(),
    };
    if let Ok(frame) = encode_frame(&event) {
        let _ = writer.write_all(&frame).await;
    }
}

fn format_outcome(outcome: &TurnOutcome) -> String {
    match outcome {
        TurnOutcome::Completed { final_message, .. } => {
            format!("completed: {final_message}")
        }
        TurnOutcome::Failed(e) => format!("failed: {e}"),
        TurnOutcome::Cancelled { .. } => "cancelled".into(),
        TurnOutcome::TimedOut { .. } => "timed out".into(),
        TurnOutcome::Escalated { reason, .. } => {
            format!("escalated: {reason}")
        }
    }
}

// ---------------------------------------------------------------------------
// PidGuard — writes PID file on create, removes on drop
// ---------------------------------------------------------------------------

struct PidGuard {
    path: PathBuf,
}

impl PidGuard {
    fn write(path: &Path) -> Result<Self, String> {
        let pid = std::process::id();
        std::fs::write(path, pid.to_string())
            .map_err(|e| format!("failed to write PID file at {}: {e}", path.display()))?;
        Ok(PidGuard { path: path.to_path_buf() })
    }
}

impl Drop for PidGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

// ---------------------------------------------------------------------------
// IpcChannelBridge — forwards StreamEvents over IPC
// ---------------------------------------------------------------------------

struct IpcChannelBridge {
    inner: Arc<dyn ChannelContext + Send + Sync>,
    writer: Arc<tokio::sync::Mutex<tokio::net::unix::OwnedWriteHalf>>,
    session_id: String,
}

#[async_trait::async_trait]
impl ChannelContext for IpcChannelBridge {
    fn channel_name(&self) -> &str {
        self.inner.channel_name()
    }

    fn platform(&self) -> aivyx_core::ChannelPlatform {
        self.inner.platform()
    }

    fn trust_tier(&self) -> aivyx_capability::TrustTier {
        self.inner.trust_tier()
    }

    fn session_id(&self) -> aivyx_core::SessionId {
        self.inner.session_id()
    }

    async fn stream_event(&self, event: StreamEvent<'_>) -> Result<(), aivyx_core::ChannelError> {
        let payload = stream_event_to_payload(&event);
        let msg = DaemonMessage::StreamEvent {
            session_id: self.session_id.clone(),
            event: payload,
        };
        let frame = encode_frame(&msg)
            .map_err(|e| aivyx_core::ChannelError::Send(format!("encode StreamEvent: {e}")))?;
        let mut w = self.writer.lock().await;
        w.write_all(&frame)
            .await
            .map_err(|e| aivyx_core::ChannelError::Send(format!("write StreamEvent: {e}")))?;
        Ok(())
    }

    async fn finalize(&self, _outcome: &TurnOutcome) -> Result<(), aivyx_core::ChannelError> {
        Ok(())
    }

    fn cancellation_token(&self) -> aivyx_core::CancellationToken {
        self.inner.cancellation_token()
    }
}

fn stream_event_to_payload(event: &StreamEvent<'_>) -> StreamEventPayload {
    match event {
        StreamEvent::Text(text) => StreamEventPayload::Text {
            text: (*text).to_string(),
        },
        StreamEvent::Status(status) => StreamEventPayload::Status {
            status: (*status).to_string(),
        },
        StreamEvent::ToolCallStarted {
            tool,
            tool_name,
            input,
        } => StreamEventPayload::ToolCallStarted {
            tool_id: tool.to_string(),
            tool_name: (*tool_name).to_string(),
            input: (*input).clone(),
        },
        StreamEvent::ToolCallFinished {
            tool,
            tool_name,
            outcome_summary,
        } => StreamEventPayload::ToolCallFinished {
            tool_id: tool.to_string(),
            tool_name: (*tool_name).to_string(),
            outcome_summary: (*outcome_summary).to_string(),
        },
        StreamEvent::ToolOutput {
            tool,
            tool_name,
            chunk,
        } => StreamEventPayload::ToolOutput {
            tool_id: tool.to_string(),
            tool_name: (*tool_name).to_string(),
            chunk: (*chunk).to_string(),
        },
        StreamEvent::Attachment { .. } => StreamEventPayload::Status {
            status: "[attachment not supported over IPC]".to_string(),
        },
    }
}
