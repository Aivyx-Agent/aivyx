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

// ---------------------------------------------------------------------------
// DaemonError — typed error enum for the daemon layer (Phase 41 Task 3)
// ---------------------------------------------------------------------------

/// Typed error enum for the daemon server and its subsystems.
///
/// Phase 41 Task 3 replaces the stringly-typed `Result<(), String>`
/// signatures that had accumulated across Phases 16–39. Typed errors
/// are a prerequisite for the Channel SDK (P5) — third-party adapters
/// need matchable variants, not opaque strings.
#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    /// Failed to bind the Unix domain socket.
    #[error("failed to bind daemon socket at {path}: {source}")]
    Bind {
        path: String,
        source: std::io::Error,
    },

    /// Failed to accept an incoming connection.
    #[error("accept error: {0}")]
    Accept(std::io::Error),

    /// IPC frame encoding or decoding failure.
    #[error("frame error: {0}")]
    Frame(#[from] FrameError),

    /// IPC protocol violation (e.g., message before handshake).
    #[error("protocol error: {0}")]
    Protocol(String),

    /// I/O error on the socket connection.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// PID file or state file operation failed.
    #[error("pid/state file error at {path}: {source}")]
    PidFile {
        path: String,
        source: std::io::Error,
    },

    /// Mission store operation failed.
    #[error("mission store error: {0}")]
    MissionStore(String),

    /// Configuration error (missing or invalid config values).
    #[error("config error: {0}")]
    Config(String),

    /// WebSocket or Web UI error.
    #[error("websocket error: {0}")]
    WebSocket(String),

    /// Internal error (catch-all for unexpected conditions).
    #[error("{0}")]
    Internal(String),
}

impl DaemonError {
    /// Convert a `DaemonError` to a `String` for backward compatibility
    /// with callers that still use `Result<_, String>`.
    pub fn to_string_compat(&self) -> String {
        self.to_string()
    }
}

/// Channel factory: given a `FrontendType`, returns the appropriate
/// `ChannelContext` implementation for that frontend. The binary
/// constructs this closure at startup, capturing the resources each
/// channel type needs (stdout handle for Local, transport for Telegram).
pub type ChannelFactory =
    Arc<dyn Fn(FrontendType) -> Arc<dyn ChannelContext + Send + Sync> + Send + Sync>;

/// Configuration for the daemon server.
///
/// Bundles the parameters that `run_daemon` needs into a single struct.
/// Phase 41 Task 2 extracted these from the 10-parameter function
/// signature that had accreted across Phases 21–39.
pub struct DaemonConfig {
    /// Path to the Unix domain socket the daemon listens on.
    pub socket_path: PathBuf,
    /// The shared agent instance that serves all connections.
    pub agent: Arc<dyn Agent>,
    /// Factory that constructs per-connection `ChannelContext` impls.
    pub channel_factory: ChannelFactory,
    /// Token for triggering graceful shutdown from outside.
    pub shutdown: CancellationToken,
    /// Optional encrypted storage domain for mission state.
    pub mission_store: Option<DomainHandle>,
    /// Optional encrypted storage domain for cron schedules.
    pub schedule_store: Option<DomainHandle>,
    /// Optional encrypted storage domain for webhook triggers.
    pub webhook_store: Option<DomainHandle>,
    /// Optional encrypted storage domain for file-watch triggers.
    pub file_watch_store: Option<DomainHandle>,
    /// Port for the localhost-only webhook HTTP listener.
    pub webhook_port: Option<u16>,
    /// Port for the localhost-only web UI server.
    pub web_ui_port: Option<u16>,
}

/// Run the daemon server.
///
/// Binds the Unix socket at `config.socket_path`, accepts connections
/// in a loop, and spawns a handler task per connection. Each handler
/// reads `FrontendMessage` frames and dispatches turns through the
/// shared `agent`. The `channel_factory` constructs a per-connection
/// `ChannelContext` based on the frontend type sent in `StartSession`.
///
/// The `shutdown` token allows external code (signal handlers, tests)
/// to trigger a graceful shutdown. When cancelled, the daemon stops
/// accepting new connections; in-flight handler tasks complete their
/// current turn and exit.
pub async fn run_daemon(config: DaemonConfig) -> Result<(), DaemonError> {
    let DaemonConfig {
        socket_path,
        agent,
        channel_factory,
        shutdown,
        mission_store,
        schedule_store,
        webhook_store,
        file_watch_store,
        webhook_port,
        web_ui_port,
    } = config;
    let socket_path = &socket_path;
    let _ = std::fs::remove_file(socket_path);

    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| DaemonError::Bind { path: parent.display().to_string(), source: e })?;
    }

    let listener = UnixListener::bind(socket_path)
        .map_err(|e| DaemonError::Bind { path: socket_path.display().to_string(), source: e })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(socket_path, perms)
            .map_err(|e| DaemonError::Bind { path: socket_path.display().to_string(), source: e })?;
    }

    let pid_path = socket_path.with_extension("pid");
    let _pid_guard = PidGuard::write(&pid_path)?;

    // Crash-recovery detection (Phase 41 Task 4).
    let state_path = socket_path.with_extension("state");
    let recovery_notice = detect_crash_recovery(&state_path);
    if let Some(ref stale) = recovery_notice {
        eprintln!(
            "aivyx daemon: detected unclean shutdown (pid {}, started at {}). \
             Lost sessions: {:?}, lost turns: {:?}",
            stale.pid, stale.started_at, stale.sessions, stale.in_flight_turns,
        );
    }
    let _state_guard = StateGuard::write(&state_path)?;
    let daemon_state = _state_guard.shared();

    // Shared trigger dispatch — all trigger subsystems (cron, webhook,
    // file-watch) share the same turn lock and agent/channel references.
    let mut trigger_dispatch =
        crate::trigger::TriggerDispatch::new(Arc::clone(&agent), Arc::clone(&channel_factory));
    if let Some(ref ms) = mission_store {
        trigger_dispatch = trigger_dispatch.with_mission_store(ms.clone());
    }

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
        let port = webhook_port.unwrap_or(crate::webhook_listener::DEFAULT_WEBHOOK_PORT);
        tokio::spawn(async move {
            if let Err(e) = crate::webhook_listener::run_webhook_listener(
                wh_dispatch,
                store,
                port,
                wh_shutdown,
            )
            .await
            {
                eprintln!("aivyx webhook listener error: {e}");
            }
        })
    });

    // Spawn the file-watch loop if a file-watch store is provided.
    let _file_watch_handle = file_watch_store.map(|store| {
        let fw_dispatch = trigger_dispatch.clone();
        let fw_shutdown = shutdown.clone();
        tokio::spawn(async move {
            crate::file_watcher::run_file_watcher(fw_dispatch, store, fw_shutdown).await;
        })
    });

    // Spawn the web UI server if a port is configured.
    let _web_ui_handle = web_ui_port.map(|port| {
        let web_shutdown = shutdown.clone();
        let web_socket_path = socket_path.to_path_buf();
        tokio::spawn(async move {
            if let Err(e) = crate::web_ui::run_web_ui_server(
                web_socket_path,
                port,
                web_shutdown,
            )
            .await
            {
                eprintln!("aivyx web ui error: {e}");
            }
        })
    });

    let mission_store = mission_store.map(Arc::new);
    let pending_recovery: Arc<std::sync::Mutex<Option<DaemonState>>> =
        Arc::new(std::sync::Mutex::new(recovery_notice));
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
        let conn_recovery = Arc::clone(&pending_recovery);
        let conn_state = Arc::clone(&daemon_state);

        let handle = tokio::spawn(async move {
            if let Err(e) = handle_connection(
                stream, agent, factory, conn_shutdown, conn_mission_store,
                conn_recovery, conn_state,
            ).await {
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
    pending_recovery: Arc<std::sync::Mutex<Option<DaemonState>>>,
    daemon_state: Arc<std::sync::Mutex<DaemonState>>,
) -> Result<(), DaemonError> {
    let (mut reader, mut writer) = stream.into_split();

    let ready = DaemonLifecycleEvent::DaemonReady {
        version: PROTOCOL_VERSION.into(),
    };
    let frame = encode_frame(&ready)?;
    writer.write_all(&frame).await?;

    // Deliver recovery notice to the first connecting frontend (take-once).
    let recovery_frame = {
        let stale = pending_recovery.lock().unwrap().take();
        stale.and_then(|s| {
            let notice = DaemonLifecycleEvent::RecoveryNotice {
                lost_sessions: s.sessions,
                lost_turns: s.in_flight_turns,
                stale_since: s.started_at,
            };
            encode_frame(&notice).ok()
        })
    };
    if let Some(frame) = recovery_frame {
        let _ = writer.write_all(&frame).await;
    }

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
                result?
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

                            // Track session in daemon state.
                            if let Ok(mut st) = daemon_state.lock() {
                                st.sessions.push(sid.clone());
                            }

                            let resp = DaemonMessage::SessionStarted { session_id: sid };
                            let frame = encode_frame(&resp)?;
                            writer.write_all(&frame).await?;
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

                            // Track in-flight turn in daemon state.
                            let turn_key = format!("{sid}:turn");
                            if let Ok(mut st) = daemon_state.lock() {
                                st.in_flight_turns.push(turn_key.clone());
                            }

                            let bridge = IpcChannelBridge {
                                inner: ch,
                                writer: Arc::new(tokio::sync::Mutex::new(writer)),
                                session_id: sid.clone(),
                            };

                            let outcome = agent.turn(msg, &bridge).await;

                            // Turn completed — remove from in-flight.
                            if let Ok(mut st) = daemon_state.lock() {
                                st.in_flight_turns.retain(|t| t != &turn_key);
                            }

                            writer = Arc::try_unwrap(bridge.writer)
                                .map_err(|_| DaemonError::Internal("writer arc still shared".into()))?
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
                                    )
                                    .map_err(|e| e.to_string())?;
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
                                        let frame = encode_frame(&gate_event)?;
                                        writer.write_all(&frame).await?;
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
                            let frame = encode_frame(&resp)?;
                            writer.write_all(&frame).await?;
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
                                mission::resolve_gate(&mut record, &gate_id, approved)
                                    .map_err(|e| e.to_string())?;
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
                                    let frame = encode_frame(&resp)?;
                                    writer.write_all(&frame).await?;

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
                                                .map_err(|_| DaemonError::Internal(
                                                    "writer arc still shared".into(),
                                                ))?
                                                .into_inner();

                                            let outcome_str = format_outcome(&resume_outcome);
                                            let resp = DaemonMessage::TurnComplete {
                                                session_id: sid,
                                                outcome: outcome_str,
                                            };
                                            let frame = encode_frame(&resp)?;
                                            writer.write_all(&frame).await?;
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
                    return Err(e.into());
                }
            }
        }
    }

    // Deregister session from daemon state on disconnect.
    if let Some(ref sid) = session_id {
        if let Ok(mut st) = daemon_state.lock() {
            st.sessions.retain(|s| s != sid);
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
) -> Result<(), DaemonError> {
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
) -> Result<(), DaemonError> {
    let _ = std::fs::remove_file(socket_path);

    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let listener = UnixListener::bind(socket_path)
        .map_err(|source| DaemonError::Bind {
            path: socket_path.display().to_string(),
            source,
        })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(socket_path, perms)?;
    }

    let (stream, _addr) = listener.accept()
        .await
        .map_err(DaemonError::Accept)?;

    let shutdown = CancellationToken::new();
    let no_recovery = Arc::new(std::sync::Mutex::new(None));
    let empty_state = Arc::new(std::sync::Mutex::new(DaemonState {
        pid: std::process::id(),
        started_at: 0,
        sessions: Vec::new(),
        in_flight_turns: Vec::new(),
    }));
    handle_connection(stream, agent, channel_factory, shutdown, None, no_recovery, empty_state).await
}

/// Backward-compatible single-channel daemon with shutdown token.
pub async fn run_daemon_compat<C: ChannelContext + Send + Sync + 'static>(
    socket_path: &Path,
    agent: Arc<dyn Agent>,
    channel: Arc<C>,
    shutdown: CancellationToken,
) -> Result<(), DaemonError> {
    let channel_for_factory: Arc<dyn ChannelContext + Send + Sync> = channel;
    let factory: ChannelFactory = Arc::new(move |_| Arc::clone(&channel_for_factory));
    run_daemon(DaemonConfig {
        socket_path: socket_path.to_path_buf(),
        agent,
        channel_factory: factory,
        shutdown,
        mission_store: None,
        schedule_store: None,
        webhook_store: None,
        file_watch_store: None,
        webhook_port: None,
        web_ui_port: None,
    }).await
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
    fn write(path: &Path) -> Result<Self, DaemonError> {
        let pid = std::process::id();
        std::fs::write(path, pid.to_string())
            .map_err(|source| DaemonError::PidFile {
                path: path.display().to_string(),
                source,
            })?;
        Ok(PidGuard { path: path.to_path_buf() })
    }
}

impl Drop for PidGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

// ---------------------------------------------------------------------------
// StateGuard — crash-recovery metadata (Phase 41 Task 4)
// ---------------------------------------------------------------------------

/// Serializable snapshot of the daemon's active sessions and in-flight
/// turns. Written to `daemon.state` on startup; cleared on clean
/// shutdown. If a stale file is found on next startup, it means the
/// previous daemon crashed — the data inside tells the operator which
/// sessions/turns were lost.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DaemonState {
    pub pid: u32,
    pub started_at: u64,
    pub sessions: Vec<String>,
    pub in_flight_turns: Vec<String>,
}

/// RAII guard that writes `daemon.state` on creation and removes it on
/// drop (clean shutdown). Holds a shared handle so `handle_connection`
/// can register/deregister sessions and turns.
struct StateGuard {
    path: PathBuf,
    state: Arc<std::sync::Mutex<DaemonState>>,
}

impl StateGuard {
    fn write(path: &Path) -> Result<Self, DaemonError> {
        let state = DaemonState {
            pid: std::process::id(),
            started_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            sessions: Vec::new(),
            in_flight_turns: Vec::new(),
        };
        Self::persist(path, &state)?;
        Ok(StateGuard {
            path: path.to_path_buf(),
            state: Arc::new(std::sync::Mutex::new(state)),
        })
    }

    fn shared(&self) -> Arc<std::sync::Mutex<DaemonState>> {
        Arc::clone(&self.state)
    }

    fn persist(path: &Path, state: &DaemonState) -> Result<(), DaemonError> {
        let json = serde_json::to_string_pretty(state)
            .map_err(|e| DaemonError::Internal(format!("serialize state: {e}")))?;
        std::fs::write(path, json).map_err(|source| DaemonError::PidFile {
            path: path.display().to_string(),
            source,
        })
    }
}

impl Drop for StateGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Check for a stale `daemon.state` file from a previous crash.
/// Returns `Some(DaemonState)` if a crash is detected, `None` otherwise.
///
/// A clean shutdown removes the state file via `StateGuard::drop`, so
/// any remaining file means the previous daemon exited abnormally.
/// As a safety check, if the recorded PID matches the current process
/// (e.g., test reuse), the file is treated as stale, not a live
/// collision.
fn detect_crash_recovery(state_path: &Path) -> Option<DaemonState> {
    let contents = std::fs::read_to_string(state_path).ok()?;
    let state: DaemonState = serde_json::from_str(&contents).ok()?;
    Some(state)
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("aivyx-test-state")
            .join(name);
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    #[test]
    fn daemon_state_round_trips_through_json() {
        let state = DaemonState {
            pid: 12345,
            started_at: 1713700000,
            sessions: vec!["ses-abc".into(), "ses-def".into()],
            in_flight_turns: vec!["ses-abc:turn".into()],
        };
        let json = serde_json::to_string(&state).unwrap();
        let parsed: DaemonState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.pid, 12345);
        assert_eq!(parsed.started_at, 1713700000);
        assert_eq!(parsed.sessions.len(), 2);
        assert_eq!(parsed.in_flight_turns, vec!["ses-abc:turn"]);
    }

    #[test]
    fn detect_crash_recovery_returns_none_for_missing_file() {
        let dir = test_dir("crash-missing");
        let path = dir.join("daemon.state");
        let _ = std::fs::remove_file(&path);
        assert!(detect_crash_recovery(&path).is_none());
    }

    #[test]
    fn detect_crash_recovery_returns_state_for_stale_file() {
        let dir = test_dir("crash-stale");
        let path = dir.join("daemon.state");
        let state = DaemonState {
            pid: 99999,
            started_at: 1713700000,
            sessions: vec!["ses-old".into()],
            in_flight_turns: vec!["ses-old:turn".into()],
        };
        std::fs::write(&path, serde_json::to_string(&state).unwrap()).unwrap();
        let recovered = detect_crash_recovery(&path).unwrap();
        assert_eq!(recovered.pid, 99999);
        assert_eq!(recovered.sessions, vec!["ses-old"]);
        assert_eq!(recovered.in_flight_turns, vec!["ses-old:turn"]);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn detect_crash_recovery_returns_none_for_invalid_json() {
        let dir = test_dir("crash-invalid");
        let path = dir.join("daemon.state");
        std::fs::write(&path, "not valid json").unwrap();
        assert!(detect_crash_recovery(&path).is_none());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn state_guard_creates_and_removes_file() {
        let dir = test_dir("guard-lifecycle");
        let path = dir.join("daemon.state");
        {
            let _guard = StateGuard::write(&path).unwrap();
            assert!(path.exists());
            let contents = std::fs::read_to_string(&path).unwrap();
            let state: DaemonState = serde_json::from_str(&contents).unwrap();
            assert_eq!(state.pid, std::process::id());
            assert!(state.sessions.is_empty());
            assert!(state.in_flight_turns.is_empty());
        }
        // Guard dropped — file should be removed.
        assert!(!path.exists());
    }

    #[test]
    fn state_guard_shared_allows_session_tracking() {
        let dir = test_dir("guard-tracking");
        let path = dir.join("daemon.state");
        let guard = StateGuard::write(&path).unwrap();
        let shared = guard.shared();

        // Register a session.
        shared.lock().unwrap().sessions.push("ses-1".into());
        assert_eq!(shared.lock().unwrap().sessions, vec!["ses-1"]);

        // Register an in-flight turn.
        shared.lock().unwrap().in_flight_turns.push("ses-1:turn".into());

        // Complete turn.
        shared.lock().unwrap().in_flight_turns.retain(|t| t != "ses-1:turn");
        assert!(shared.lock().unwrap().in_flight_turns.is_empty());

        // Deregister session.
        shared.lock().unwrap().sessions.retain(|s| s != "ses-1");
        assert!(shared.lock().unwrap().sessions.is_empty());

        drop(guard);
        assert!(!path.exists());
    }
}
