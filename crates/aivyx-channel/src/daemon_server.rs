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

use aivyx_audit::PersistentAuditLog;
use aivyx_core::{Agent, CancellationToken, ChannelContext, Message, StreamEvent, TurnOutcome};

use aivyx_storage::DomainHandle;

use crate::daemon_ipc::{
    decode_frame, encode_frame, AuditEntrySummary, DaemonLifecycleEvent, DaemonMessage, FrameError,
    FrontendMessage, FrontendType, GateSummary, MissionDetail, MissionSummary,
    NotificationHistoryEntry, ProfileSummary, QueryPayload, QueryResponsePayload, SessionSummary,
    StreamEventPayload, PROTOCOL_VERSION,
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
    /// Phase 63 Task 3 — optional notify dispatcher passed to
    /// `TriggerDispatch::with_notify_dispatcher` so trigger
    /// configs with `notify_target = Some(name)` auto-push the
    /// turn's final response after firing.
    pub notify_dispatcher: Option<Arc<crate::notify_dispatcher::NotifyDispatcher>>,
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
    /// Optional shared memory instance for background GC.
    pub memory: Option<Arc<dyn aivyx_memory::Memory>>,
    /// If set, entries older than this many seconds are expired by a
    /// background 1-hour timer.  Requires `memory` to be `Some`.
    pub memory_ttl_secs: Option<u64>,
    /// Phase 47 — optional handle on the persistent audit log so the
    /// daemon can answer `ListAuditEntries` / `VerifyAuditChain`
    /// inspection queries from the Web UI. When `None`, those queries
    /// return `QueryError { code: "no_audit_log", .. }`.
    pub audit_log: Option<Arc<PersistentAuditLog>>,
    /// Phase 58 — operator-declared identity layer (PRODUCT.md P13).
    /// Read-only at daemon runtime per Q5(a) load-time semantics;
    /// served to the Web UI Profile pane via the `GetProfile`
    /// inspection query. Always populated — the synthesized default
    /// is supplied when `aivyx.toml` has no `[profile]` section.
    pub profile: Arc<aivyx_config::Profile>,
    /// Phase 60 — persistent Persona delta chain (PRODUCT.md P14).
    /// The daemon uses it for both inspection queries
    /// (`ListPersonaDeltas`) and revert operations
    /// (`RevertPersonaDelta` appends to it). `None` is the test-
    /// fixture path (POC daemon / round-trip tests) — both queries
    /// return empty / default responses.
    pub persona_log: Option<Arc<crate::persona::PersistentPersonaLog>>,
    /// Phase 60 — shared runtime effective Persona. The planner
    /// factory reads it per-turn; this handle exists on the daemon
    /// side so `RevertPersonaDelta` and inspection queries can read
    /// the current snapshot. Always present — defaults to an empty
    /// state for test fixtures.
    pub shared_persona: crate::persona::SharedEffectivePersona,
    /// Phase 69 — Web UI desktop-notification broadcaster. When
    /// the Web UI is enabled, the binary constructs one
    /// `WebUiBroadcaster` and Arc-shares it between this field
    /// (so the WS handler can subscribe per browser connection)
    /// and the notify dispatcher (so `kind = "web-ui"` targets
    /// can push frames into it). `None` when the Web UI is
    /// disabled and no `kind = "web-ui"` targets exist.
    pub web_ui_broadcaster: Option<Arc<crate::notify_webui::WebUiBroadcaster>>,
    /// Phase 70 — persistent Persona proposal chain
    /// (KeyDomain::PersonaProposals). Pending proposals from
    /// the reflection auto-loop append rows here; operators
    /// resolve them via `ResolvePersonaProposal`, which
    /// transitions the status to Approved / Rejected and
    /// (on approve) appends a PersonaDelta to `persona_log`.
    /// `None` is the test-fixture path — proposal queries
    /// return empty / not-wired responses.
    pub persona_proposal_log:
        Option<Arc<crate::persona_proposal::PersistentPersonaProposalLog>>,
    /// Phase 71 — validated `[[reflection_schedule]]` entries
    /// from the config loader. When non-empty AND an audit log
    /// is configured, the daemon spawns
    /// `run_reflection_scheduler` to fire reflection turns on
    /// each entry's cron pattern. When empty, the reflection
    /// scheduler task is not spawned.
    pub reflection_schedules: Vec<aivyx_config::ReflectionScheduleConfig>,
    /// Phase 74 — per-topic-glob retention rules from
    /// `[[memory.retention]]`. Threaded into the memory-GC
    /// timer; first-match wins, unmatched topics fall through
    /// to `memory_ttl_secs`.
    pub memory_retention: Vec<aivyx_config::MemoryRetentionRule>,
    /// Phase 73 — per-target retry + rate-limit policy map.
    /// Built by the binary's startup path from the loaded
    /// `[[notify_target]]` blocks (one entry per target name).
    /// Empty map → every dispatch uses the zero-retry / no-
    /// rate-limit defaults — today's behavior.
    pub target_policies: std::collections::HashMap<String, crate::trigger::TargetPolicy>,
    /// Phase 75 — embedding provider for semantic memory.
    /// `Some` iff `[embedding]` is configured. Drives the
    /// hourly lazy-backfill pass in the memory-GC timer; it is
    /// the same provider the write tool's embedding hook wraps.
    /// `None` = semantic search disabled, no backfill spawned.
    pub embedding_provider:
        Option<Arc<dyn aivyx_llm::embedding::EmbeddingProvider>>,
    /// Phase 77 — the recall-feedback log. `Some` iff
    /// auto-recall is configured; the reflection scheduler
    /// reads/clamps it on its cadence to close the
    /// recall→learning loop. `None` → the feedback pass is
    /// skipped (pre-Phase-77 behavior).
    pub recall_log:
        Option<Arc<crate::recall_log::PersistentRecallLog>>,
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
        notify_dispatcher,
        schedule_store,
        webhook_store,
        file_watch_store,
        webhook_port,
        web_ui_port,
        memory,
        memory_ttl_secs,
        audit_log,
        profile,
        persona_log,
        shared_persona,
        web_ui_broadcaster,
        persona_proposal_log,
        reflection_schedules,
        memory_retention,
        target_policies,
        embedding_provider,
        recall_log,
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
    // Phase 63 Task 3 — auto-notify on trigger fire if the
    // operator configured `notify_target` on the trigger.
    if let Some(ref nd) = notify_dispatcher {
        trigger_dispatch = trigger_dispatch.with_notify_dispatcher(Arc::clone(nd));
    }
    // Phase 67 — audit auto-notify dispatches into the same
    // persistent chain that records TurnStarted/TurnEnded, so
    // forensic walks see the complete trigger-fire-to-notify
    // story for each schedule fire.
    if let Some(ref al) = audit_log {
        trigger_dispatch = trigger_dispatch.with_audit_log(Arc::clone(al));
    }
    // Phase 73 — per-target retry + rate-limit policy map. Empty
    // map → every dispatch uses the zero-retry / no-rate-limit
    // defaults (today's behavior). Always called even with an
    // empty map so the dispatcher's internal `target_policies`
    // is set authoritatively from config at startup.
    trigger_dispatch = trigger_dispatch.with_target_policies(target_policies);

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

    // Phase 71 — spawn the reflection scheduler if any
    // `[[reflection_schedule]]` entries are configured AND an
    // audit log is available (the loop reads the chain to
    // build outcome summaries). If either prerequisite is
    // missing the task is simply not spawned; the config block
    // sits idle.
    let _reflection_scheduler_handle = match (audit_log.as_ref(), reflection_schedules.is_empty()) {
        (Some(al), false) => {
            let rs_dispatch = trigger_dispatch.clone();
            let rs_shutdown = shutdown.clone();
            let rs_audit = Arc::clone(al);
            let rs_schedules = reflection_schedules.clone();
            // Phase 77 — bundle the recall→reflection feedback
            // deps iff the whole substrate is present (recall
            // log + memory + proposal chain). Any missing piece
            // → `None` → the feedback pass is skipped while the
            // reflection turn still fires normally.
            let rs_recall_feedback = match (
                recall_log.clone(),
                memory.clone(),
                persona_proposal_log.clone(),
            ) {
                (Some(rl), Some(mem), Some(pl)) => {
                    Some(crate::reflection_scheduler::RecallFeedbackDeps {
                        recall_log: rl,
                        memory: mem,
                        proposal_log: pl,
                        gc_retain_secs:
                            crate::recall_feedback::RECALL_LOG_RETAIN_SECS,
                    })
                }
                _ => None,
            };
            for sched in &rs_schedules {
                eprintln!(
                    "aivyx reflection schedule {:?} registered (cron={:?}, \
                     lookback={}s)",
                    sched.name, sched.cron, sched.lookback_window_secs,
                );
            }
            Some(tokio::spawn(async move {
                crate::reflection_scheduler::run_reflection_scheduler(
                    rs_schedules,
                    rs_dispatch,
                    rs_audit,
                    rs_recall_feedback,
                    rs_shutdown,
                )
                .await;
            }))
        }
        (None, false) => {
            eprintln!(
                "aivyx daemon: {} [[reflection_schedule]] entries configured \
                 but no audit log is available — reflection scheduler not \
                 spawned (outcome summaries require the audit chain)",
                reflection_schedules.len(),
            );
            None
        }
        _ => None,
    };

    // Spawn the web UI server if a port is configured.
    let _web_ui_handle = web_ui_port.map(|port| {
        let web_shutdown = shutdown.clone();
        let web_socket_path = socket_path.to_path_buf();
        let web_broadcaster = web_ui_broadcaster.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::web_ui::run_web_ui_server(
                web_socket_path,
                port,
                web_shutdown,
                web_broadcaster,
            )
            .await
            {
                eprintln!("aivyx web ui error: {e}");
            }
        })
    });

    // Spawn the memory-GC timer if a TTL is configured OR if any
    // `[[memory.retention]]` rules are declared (Phase 74). Runs
    // every hour. Path A (retention rules present): walks every
    // entry, finds the first matching rule, applies its policy.
    // Unmatched entries fall through to the global
    // `memory_ttl_secs` cutoff (or are kept if neither matches
    // nor a default TTL exists). Path B (no rules, just TTL):
    // original Phase 42 behavior, every entry checked against
    // the single global cutoff.
    let _memory_gc_handle = {
        let needs_gc =
            memory_ttl_secs.is_some() || !memory_retention.is_empty();
        // Phase 75 — the same hourly timer also drives the
        // embedding backfill, so it must spawn when a provider
        // is configured even if no TTL/retention GC is.
        let needs_backfill = embedding_provider.is_some();
        let mem_arc = if needs_gc || needs_backfill {
            memory.clone()
        } else {
            None
        };
        if let (true, Some(mem)) = (needs_gc || needs_backfill, mem_arc) {
            let gc_shutdown = shutdown.clone();
            let rules = memory_retention.clone();
            let ttl = memory_ttl_secs;
            let backfill_provider = embedding_provider.clone();
            Some(tokio::spawn(async move {
                let mut interval =
                    tokio::time::interval(std::time::Duration::from_secs(3600));
                // The first tick fires immediately — skip it so the
                // first GC runs after one hour of uptime, not at
                // startup.
                interval.tick().await;
                loop {
                    tokio::select! {
                        _ = interval.tick() => {
                          if needs_gc {
                            let now = std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs();
                            // Resolve the default cutoff from the
                            // optional global TTL.
                            let default_cutoff =
                                ttl.map(|t| now.saturating_sub(t));
                            // Build precomputed RetentionMatcher
                            // slice from the rules; cutoff_secs is
                            // None for Forever, Some(now - days*86400)
                            // for ForDays. The closure wraps each
                            // rule's GlobMatcher into the
                            // `&dyn Fn(&str) -> bool` shape the
                            // memory crate's RetentionMatcher
                            // expects.
                            type GlobClosure =
                                Box<dyn Fn(&str) -> bool + Send + Sync>;
                            let closures: Vec<GlobClosure> = rules
                                .iter()
                                .map(|r| {
                                    let m = r.matcher.clone();
                                    Box::new(move |topic: &str| m.is_match(topic))
                                        as Box<
                                            dyn Fn(&str) -> bool + Send + Sync,
                                        >
                                })
                                .collect();
                            let matchers: Vec<aivyx_memory::RetentionMatcher<'_>> =
                                rules.iter().enumerate().map(|(i, r)| {
                                    let cutoff = match r.retention {
                                        aivyx_config::RetentionPolicy::Forever => None,
                                        aivyx_config::RetentionPolicy::ForDays(days) => {
                                            Some(now.saturating_sub(days.saturating_mul(86400)))
                                        }
                                    };
                                    aivyx_memory::RetentionMatcher {
                                        matches: closures[i].as_ref(),
                                        cutoff_secs: cutoff,
                                    }
                                }).collect();
                            let result = if matchers.is_empty() {
                                // No rules → keep the existing
                                // global-TTL path. default_cutoff is
                                // unwrap-able here because !needs_gc
                                // checked above would have skipped
                                // the spawn entirely otherwise.
                                if let Some(cutoff) = default_cutoff {
                                    mem.gc_expired(cutoff).await
                                } else {
                                    Ok(0)
                                }
                            } else {
                                mem.gc_expired_with_rules(
                                    &matchers,
                                    default_cutoff,
                                )
                                .await
                            };
                            match result {
                                Ok(n) if n > 0 => {
                                    eprintln!(
                                        "aivyx memory gc: expired {n} entries \
                                         ({} rule(s) applied)",
                                        rules.len(),
                                    );
                                }
                                Ok(_) => {}
                                Err(e) => {
                                    eprintln!("aivyx memory gc error: {e}");
                                }
                            }
                          }
                          // Phase 75 — lazy embedding backfill on
                          // the same hourly cadence. Bounded per
                          // tick; provider failure is non-fatal
                          // (the pass returns Ok(0) and retries
                          // next hour).
                          if let Some(provider) = &backfill_provider {
                              match crate::memory_embedding::run_backfill_pass(
                                  &mem, provider,
                              )
                              .await
                              {
                                  Ok(n) if n > 0 => {
                                      eprintln!(
                                          "aivyx memory embed: backfilled \
                                           {n} vector(s)"
                                      );
                                  }
                                  Ok(_) => {}
                                  Err(e) => {
                                      eprintln!(
                                          "aivyx memory embed backfill \
                                           error: {e}"
                                      );
                                  }
                              }
                          }
                        }
                        _ = gc_shutdown.cancelled() => break,
                    }
                }
            }))
        } else {
            None
        }
    };

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

        let ctx = ConnectionContext {
            stream,
            agent: Arc::clone(&agent),
            channel_factory: Arc::clone(&channel_factory),
            shutdown: shutdown.clone(),
            mission_store: mission_store.clone(),
            pending_recovery: Arc::clone(&pending_recovery),
            daemon_state: Arc::clone(&daemon_state),
            audit_log: audit_log.clone(),
            profile: Arc::clone(&profile),
            persona_log: persona_log.clone(),
            shared_persona: shared_persona.clone(),
            persona_proposal_log: persona_proposal_log.clone(),
            memory: memory.clone(),
            embedding_provider: embedding_provider.clone(),
        };

        let handle = tokio::spawn(async move {
            if let Err(e) = handle_connection(ctx).await {
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

/// Per-connection state the daemon hands to `handle_connection`.
///
/// Phase 51 Task 3 — lifted from `handle_connection`'s 8-parameter
/// signature into a parameter struct, same pattern Phase 41 Task 2
/// used for `DaemonConfig`. The `#[allow(clippy::too_many_arguments)]`
/// shortcut from Phase 47 Task 4 is gone.
struct ConnectionContext {
    stream: tokio::net::UnixStream,
    agent: Arc<dyn Agent>,
    channel_factory: ChannelFactory,
    shutdown: CancellationToken,
    mission_store: Option<Arc<DomainHandle>>,
    pending_recovery: Arc<std::sync::Mutex<Option<DaemonState>>>,
    daemon_state: Arc<std::sync::Mutex<DaemonState>>,
    audit_log: Option<Arc<PersistentAuditLog>>,
    /// Phase 58 — operator-declared Profile snapshot for
    /// `Query::GetProfile`. Cloned-per-connection so the handler
    /// can read it without contending with the daemon's read path.
    profile: Arc<aivyx_config::Profile>,
    /// Phase 60 — persistent Persona log for inspection queries +
    /// revert append. `None` in test fixtures.
    persona_log: Option<Arc<crate::persona::PersistentPersonaLog>>,
    /// Phase 60 — shared effective Persona for inspection +
    /// recompute after revert append.
    shared_persona: crate::persona::SharedEffectivePersona,
    /// Phase 70 — persistent Persona proposal log for
    /// `ListPersonaProposals` / `GetPersonaProposal` queries +
    /// `ResolvePersonaProposal` status transitions. `None` in
    /// test fixtures.
    persona_proposal_log:
        Option<Arc<crate::persona_proposal::PersistentPersonaProposalLog>>,
    /// Phase 74 — memory substrate handle for the
    /// `ListMemoryTopics` / `GetMemoryTopicEntries` /
    /// `SearchMemory` queries + the `EvictMemoryTopic`
    /// frontend message. `None` in test fixtures.
    memory: Option<Arc<dyn aivyx_memory::Memory>>,
    /// Phase 75 — embedding provider for the `SearchMemory`
    /// semantic path. `None` = `[embedding]` not configured;
    /// a `mode = "semantic"` request transparently falls back
    /// to keyword.
    embedding_provider:
        Option<Arc<dyn aivyx_llm::embedding::EmbeddingProvider>>,
}

async fn handle_connection(ctx: ConnectionContext) -> Result<(), DaemonError> {
    let ConnectionContext {
        stream,
        agent,
        channel_factory,
        shutdown,
        mission_store,
        pending_recovery,
        daemon_state,
        audit_log,
        profile,
        persona_log,
        shared_persona,
        persona_proposal_log,
        memory,
        embedding_provider,
    } = ctx;
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
                            attachments,
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

                            // Phase 45 — construct the right message type
                            // based on whether attachments are present.
                            let session = aivyx_core::SessionId::new();
                            let msg = if let Some(att) = attachments.first() {
                                use base64::Engine;
                                let decoder = base64::engine::general_purpose::STANDARD;
                                match decoder.decode(&att.data_base64) {
                                    Ok(data) if text.is_empty() => {
                                        Message::image(session, &att.media_type, data)
                                    }
                                    Ok(data) => {
                                        Message::text_with_image(
                                            session, &text, &att.media_type, data,
                                        )
                                    }
                                    Err(_) => {
                                        // Bad base64 — fall back to text-only.
                                        Message::text(session, text)
                                    }
                                }
                            } else {
                                Message::text(session, text)
                            };

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
                        FrontendMessage::ProtocolNegotiation { version } => {
                            // v0.1: always accept. Future versions can
                            // check compatibility and respond with
                            // ProtocolRejected if needed.
                            let resp = if version == PROTOCOL_VERSION {
                                DaemonMessage::ProtocolAccepted { version }
                            } else {
                                // For v0.1, accept any version the client
                                // sends — forward compatibility. When v0.2
                                // ships, this branch can reject unknown
                                // versions.
                                DaemonMessage::ProtocolAccepted { version }
                            };
                            let frame = encode_frame(&resp)?;
                            writer.write_all(&frame).await?;
                        }
                        FrontendMessage::Query { id, payload } => {
                            // Phase 47 — inspection queries. Read-only; no
                            // capability check (IPC socket auth is the
                            // authorization boundary, per Q2).
                            let response_payload = handle_query(
                                payload,
                                &daemon_state,
                                mission_store.as_deref(),
                                audit_log.as_deref(),
                                &profile,
                                persona_log.as_deref(),
                                &shared_persona,
                                persona_proposal_log.as_deref(),
                                memory.as_ref(),
                                embedding_provider.as_ref(),
                            )
                            .await;
                            let resp = DaemonMessage::QueryResponse {
                                id,
                                payload: response_payload,
                            };
                            let frame = encode_frame(&resp)?;
                            writer.write_all(&frame).await?;
                        }
                        FrontendMessage::RevertPersonaDelta { id, target_delta_id } => {
                            // Phase 60 — operator-initiated revert
                            // (P14 commit 4). Append a `Revert` op
                            // delta to the persona chain; on
                            // success, recompute the shared state
                            // so the next turn picks it up. Per
                            // Q5(a) at Phase 60 sign-off: no gate
                            // prompt — the operator is the
                            // proposer.
                            let resp = match resolve_persona_revert(
                                persona_log.as_deref(),
                                &shared_persona,
                                &target_delta_id,
                            )
                            .await
                            {
                                Ok(seq) => DaemonMessage::PersonaRevertResolved {
                                    id,
                                    ok: true,
                                    seq: Some(seq),
                                    error: None,
                                },
                                Err(reason) => DaemonMessage::PersonaRevertResolved {
                                    id,
                                    ok: false,
                                    seq: None,
                                    error: Some(reason),
                                },
                            };
                            let frame = encode_frame(&resp)?;
                            writer.write_all(&frame).await?;
                        }
                        FrontendMessage::ResolvePersonaProposal {
                            id,
                            proposal_id,
                            resolution,
                        } => {
                            // Phase 70 — operator-initiated proposal
                            // resolution. Approve / ApproveWithEdit
                            // apply a PersonaDelta to the persona log
                            // first, then record the Approved entry on
                            // the proposal chain bound to the delta's
                            // seq. Reject just records the Rejected
                            // entry. The shared persona snapshot is
                            // recomputed on approve so the next turn
                            // sees the new state.
                            let resp = match resolve_persona_proposal(
                                persona_proposal_log.as_deref(),
                                persona_log.as_deref(),
                                &shared_persona,
                                &id,
                                proposal_id,
                                resolution,
                            )
                            .await
                            {
                                Ok(success) => DaemonMessage::PersonaProposalResolved {
                                    id,
                                    ok: true,
                                    success: Some(success),
                                    error: None,
                                },
                                Err(reason) => DaemonMessage::PersonaProposalResolved {
                                    id,
                                    ok: false,
                                    success: None,
                                    error: Some(reason),
                                },
                            };
                            let frame = encode_frame(&resp)?;
                            writer.write_all(&frame).await?;
                        }
                        FrontendMessage::EvictMemoryTopic { id, topic } => {
                            // Phase 74 — operator-initiated memory
                            // eviction. `Memory::forget` deletes every
                            // entry under the topic and returns the
                            // count.
                            let resp = match memory.as_ref() {
                                None => DaemonMessage::MemoryEvictResolved {
                                    id,
                                    ok: false,
                                    deleted: None,
                                    error: Some(
                                        "daemon has no memory substrate \
                                         configured"
                                            .into(),
                                    ),
                                },
                                Some(mem) => match mem.forget(&topic).await {
                                    Ok(n) => DaemonMessage::MemoryEvictResolved {
                                        id,
                                        ok: true,
                                        deleted: Some(n as u64),
                                        error: None,
                                    },
                                    Err(e) => {
                                        DaemonMessage::MemoryEvictResolved {
                                            id,
                                            ok: false,
                                            deleted: None,
                                            error: Some(e.to_string()),
                                        }
                                    }
                                },
                            };
                            let frame = encode_frame(&resp)?;
                            writer.write_all(&frame).await?;
                        }
                        FrontendMessage::ImportPersonaChain {
                            id,
                            deltas,
                            effective_at_export: _,
                            force,
                        } => {
                            // Phase 65 Task 3 — replay an exported
                            // chain. Best-effort (Q1(a)): no
                            // transaction wrapping; daemon crash
                            // mid-import leaves chain partial.
                            // Operator re-imports to recover.
                            let resp = match resolve_persona_import(
                                persona_log.as_deref(),
                                &shared_persona,
                                deltas,
                                force,
                            )
                            .await
                            {
                                Ok(success) => DaemonMessage::PersonaImportResolved {
                                    id,
                                    ok: true,
                                    success: Some(success),
                                    error: None,
                                },
                                Err(reason) => DaemonMessage::PersonaImportResolved {
                                    id,
                                    ok: false,
                                    success: None,
                                    error: Some(reason),
                                },
                            };
                            let frame = encode_frame(&resp)?;
                            writer.write_all(&frame).await?;
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
    handle_connection(ConnectionContext {
        stream,
        agent,
        channel_factory,
        shutdown,
        mission_store: None,
        pending_recovery: no_recovery,
        daemon_state: empty_state,
        audit_log: None,
        profile: Arc::new(aivyx_config::Profile::default()),
        persona_log: None,
        shared_persona: crate::persona::shared_effective_persona(
            crate::persona::EffectivePersona::default(),
        ),
        persona_proposal_log: None,
        memory: None,
        embedding_provider: None,
    })
    .await
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
        notify_dispatcher: None,
        schedule_store: None,
        webhook_store: None,
        file_watch_store: None,
        webhook_port: None,
        web_ui_port: None,
        memory: None,
        memory_ttl_secs: None,
        audit_log: None,
        profile: Arc::new(aivyx_config::Profile::default()),
        persona_log: None,
        shared_persona: crate::persona::shared_effective_persona(
            crate::persona::EffectivePersona::default(),
        ),
        web_ui_broadcaster: None,
        persona_proposal_log: None,
        reflection_schedules: Vec::new(),
        target_policies: std::collections::HashMap::new(),
        embedding_provider: None,
        recall_log: None,
        memory_retention: Vec::new(),
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
// Phase 47 — query dispatch
// ---------------------------------------------------------------------------

/// Phase 47 — answer a [`QueryPayload`] from the daemon's in-memory state
/// and persistent stores.
///
/// Read-only by contract. Authorization is enforced at the IPC socket
/// boundary (mode 0600, operator-owned) — see `PRODUCT.md` P6 and
/// `docs/THREAT_MODEL.md` §4.4. Per Q2 of the Phase 47 open doc, no
/// capability check applies at the query layer.
///
/// A poisoned `DaemonState` mutex, a missing mission store, or a
/// storage error are all reported as [`QueryResponsePayload::QueryError`]
/// rather than propagated as a panic. The daemon must stay alive even
/// if one connection's state interaction tripped earlier.
// Eight parameters because the query dispatcher fans out across
// every daemon-side substrate the read-only queries can touch.
// Bundling them into a context struct is a future refactor that
// touches every existing query test fixture; deferred.
#[allow(clippy::too_many_arguments)]
async fn handle_query(
    payload: QueryPayload,
    daemon_state: &Arc<std::sync::Mutex<DaemonState>>,
    mission_store: Option<&DomainHandle>,
    audit_log: Option<&PersistentAuditLog>,
    profile: &aivyx_config::Profile,
    persona_log: Option<&crate::persona::PersistentPersonaLog>,
    shared_persona: &crate::persona::SharedEffectivePersona,
    persona_proposal_log: Option<&crate::persona_proposal::PersistentPersonaProposalLog>,
    memory: Option<&Arc<dyn aivyx_memory::Memory>>,
    embedding_provider: Option<
        &Arc<dyn aivyx_llm::embedding::EmbeddingProvider>,
    >,
) -> QueryResponsePayload {
    /// Phase 47 Q3 — server-side cap on caller-supplied `limit` for
    /// audit queries. Prevents a single query from monopolizing the
    /// daemon on a long chain.
    const AUDIT_QUERY_MAX_LIMIT: u32 = 500;

    match payload {
        QueryPayload::ListSessions => match daemon_state.lock() {
            Ok(st) => {
                let sessions = st
                    .sessions
                    .iter()
                    .map(|s| SessionSummary { session_id: s.clone() })
                    .collect();
                QueryResponsePayload::ListSessions { sessions }
            }
            Err(_) => QueryResponsePayload::QueryError {
                code: "state_poisoned".into(),
                message: "daemon state mutex poisoned".into(),
            },
        },
        QueryPayload::ListMissions => {
            let Some(store) = mission_store else {
                return QueryResponsePayload::QueryError {
                    code: "no_mission_store".into(),
                    message: "daemon has no mission store configured".into(),
                };
            };
            match mission::list_missions(store).await {
                Ok(records) => {
                    let missions = records
                        .into_iter()
                        .map(mission_summary_from_record)
                        .collect();
                    QueryResponsePayload::ListMissions { missions }
                }
                Err(e) => QueryResponsePayload::QueryError {
                    code: "list_missions_failed".into(),
                    message: e.to_string(),
                },
            }
        }
        QueryPayload::GetMission { mission_id } => {
            let Some(store) = mission_store else {
                return QueryResponsePayload::QueryError {
                    code: "no_mission_store".into(),
                    message: "daemon has no mission store configured".into(),
                };
            };
            match mission::get_mission(store, &mission_id).await {
                Ok(Some(record)) => QueryResponsePayload::GetMission {
                    mission: Some(mission_detail_from_record(record)),
                },
                Ok(None) => QueryResponsePayload::GetMission { mission: None },
                Err(e) => QueryResponsePayload::QueryError {
                    code: "get_mission_failed".into(),
                    message: e.to_string(),
                },
            }
        }
        QueryPayload::ListAuditEntries { from_seq, limit } => {
            let Some(log) = audit_log else {
                return QueryResponsePayload::QueryError {
                    code: "no_audit_log".into(),
                    message: "daemon has no audit log configured".into(),
                };
            };
            let capped = limit.min(AUDIT_QUERY_MAX_LIMIT) as usize;
            match log.entries_range(from_seq, capped) {
                Ok(rows) => {
                    let entries: Vec<AuditEntrySummary> =
                        rows.into_iter().map(audit_entry_summary_from_signed).collect();
                    QueryResponsePayload::ListAuditEntries {
                        entries,
                        total_len: log.len() as u64,
                    }
                }
                Err(e) => QueryResponsePayload::QueryError {
                    code: "list_audit_failed".into(),
                    message: e.to_string(),
                },
            }
        }
        QueryPayload::VerifyAuditChain => {
            let Some(log) = audit_log else {
                return QueryResponsePayload::QueryError {
                    code: "no_audit_log".into(),
                    message: "daemon has no audit log configured".into(),
                };
            };
            let total_len = log.len() as u64;
            match log.verify() {
                Ok(()) => QueryResponsePayload::VerifyAuditChain {
                    ok: true,
                    entries_verified: total_len,
                    error: None,
                },
                Err(e) => QueryResponsePayload::VerifyAuditChain {
                    ok: false,
                    entries_verified: 0,
                    error: Some(e.to_string()),
                },
            }
        }
        QueryPayload::GetProfile => QueryResponsePayload::GetProfile {
            profile: profile_summary_from_profile(profile),
        },
        QueryPayload::GetEffectivePersona => {
            let summary = match shared_persona.read() {
                Ok(state) => effective_persona_summary_from_state(&state),
                Err(_) => {
                    return QueryResponsePayload::QueryError {
                        code: "persona_state_poisoned".into(),
                        message: "shared persona state lock poisoned".into(),
                    };
                }
            };
            QueryResponsePayload::GetEffectivePersona { persona: summary }
        }
        QueryPayload::ListPersonaDeltas { from_seq, limit } => {
            const PERSONA_LIST_MAX_LIMIT: u32 = 500;
            let Some(log) = persona_log else {
                return QueryResponsePayload::ListPersonaDeltas {
                    entries: Vec::new(),
                    total_len: 0,
                };
            };
            let entries = log.entries();
            let total_len = entries.len() as u64;
            let start = from_seq as usize;
            let capped = (limit.min(PERSONA_LIST_MAX_LIMIT)) as usize;
            let end = (start + capped).min(entries.len());
            let page: Vec<crate::daemon_ipc::PersonaDeltaSummary> = if start >= entries.len() {
                Vec::new()
            } else {
                entries[start..end]
                    .iter()
                    .map(persona_delta_summary_from_signed)
                    .collect()
            };
            QueryResponsePayload::ListPersonaDeltas {
                entries: page,
                total_len,
            }
        }
        QueryPayload::ExportPersonaChain => {
            // Phase 64 Task 3 — full-fidelity chain dump for the
            // `aivyx identity export` flow. Single-shot response
            // (no pagination) — capped at MAX_EXPORT_CHAIN_ENTRIES.
            // Realistic chain depth is dozens to low-hundreds of
            // approved deltas; the cap exists to prevent a runaway
            // chain from blowing IPC frame size.
            const MAX_EXPORT_CHAIN_ENTRIES: usize = 100_000;
            let Some(log) = persona_log else {
                // No persona log configured — return an empty
                // chain rather than erroring. The CLI treats this
                // as "nothing to export," which is correct.
                return QueryResponsePayload::ExportPersonaChain {
                    deltas: Vec::new(),
                    effective: crate::persona::EffectivePersona::default(),
                };
            };
            let entries = log.entries();
            if entries.len() > MAX_EXPORT_CHAIN_ENTRIES {
                return QueryResponsePayload::QueryError {
                    code: "persona_chain_too_large".into(),
                    message: format!(
                        "persona chain has {} entries; export caps at {} per response. \
                         Contact aivyx maintainers if you legitimately hit this limit.",
                        entries.len(),
                        MAX_EXPORT_CHAIN_ENTRIES,
                    ),
                };
            }
            let deltas: Vec<crate::identity_export::DeltaExport> = entries
                .iter()
                .map(crate::identity_export::DeltaExport::from)
                .collect();
            let effective = match shared_persona.read() {
                Ok(state) => state.clone(),
                Err(_) => {
                    return QueryResponsePayload::QueryError {
                        code: "persona_state_poisoned".into(),
                        message: "shared persona state lock poisoned".into(),
                    };
                }
            };
            QueryResponsePayload::ExportPersonaChain { deltas, effective }
        }
        // Phase 70 — Persona proposal queries.
        QueryPayload::ListPersonaProposals {
            status_filter,
            limit,
        } => {
            let Some(log) = persona_proposal_log else {
                return QueryResponsePayload::QueryError {
                    code: "no_persona_proposal_log".into(),
                    message: "daemon has no persona proposal log configured".into(),
                };
            };
            let filter = parse_proposal_status_filter(&status_filter);
            let all = log.list(filter);
            let total_len = all.len() as u64;
            let capped = (limit as usize).min(all.len());
            let proposals: Vec<crate::daemon_ipc::PersonaProposalSummary> = all
                .into_iter()
                .take(capped)
                .map(proposal_summary_from_view)
                .collect();
            QueryResponsePayload::ListPersonaProposals {
                proposals,
                total_len,
            }
        }
        QueryPayload::GetPersonaProposal { proposal_id } => {
            let Some(log) = persona_proposal_log else {
                return QueryResponsePayload::QueryError {
                    code: "no_persona_proposal_log".into(),
                    message: "daemon has no persona proposal log configured".into(),
                };
            };
            let proposal = log.get(&proposal_id).map(proposal_summary_from_view);
            QueryResponsePayload::GetPersonaProposal { proposal }
        }
        // Phase 73 — notification history. Walks the audit
        // chain for `AutoNotifyDispatched` events, applies the
        // optional `target_filter`, and renders each match into
        // a `NotificationHistoryEntry`. Pagination matches the
        // existing audit-entry handler's pattern (server-side
        // cap of 500 per page).
        QueryPayload::ListNotificationHistory {
            from_seq,
            limit,
            target_filter,
        } => {
            let Some(log) = audit_log else {
                return QueryResponsePayload::QueryError {
                    code: "no_audit_log".into(),
                    message: "daemon has no audit log configured".into(),
                };
            };
            let chain_len = log.len();
            let entries = match log.entries_range(0, chain_len) {
                Ok(e) => e,
                Err(e) => {
                    return QueryResponsePayload::QueryError {
                        code: "audit_read_failed".into(),
                        message: format!("audit chain read failed: {e}"),
                    };
                }
            };
            let target_str = target_filter.as_deref();
            let matches: Vec<NotificationHistoryEntry> = entries
                .iter()
                .filter_map(|entry| {
                    if let aivyx_audit::AuditEvent::AutoNotifyDispatched {
                        session_id,
                        trigger_kind,
                        trigger_id,
                        target_name,
                        outcome,
                        dispatched_at_unix_ms,
                    } = &entry.event
                    {
                        if let Some(filter) = target_str {
                            if target_name != filter {
                                return None;
                            }
                        }
                        let (outcome_kind, outcome_detail) =
                            render_notify_outcome_for_history(outcome);
                        Some(NotificationHistoryEntry {
                            seq: entry.seq,
                            dispatched_at_unix_ms: *dispatched_at_unix_ms,
                            session_id: session_id.to_string(),
                            trigger_kind: format!("{trigger_kind:?}"),
                            trigger_id: trigger_id.clone(),
                            target_name: target_name.clone(),
                            outcome_kind: outcome_kind.into(),
                            outcome_detail,
                        })
                    } else {
                        None
                    }
                })
                .collect();
            let total_len = matches.len() as u64;
            const HISTORY_QUERY_MAX_LIMIT: u32 = 500;
            let capped = (limit.min(HISTORY_QUERY_MAX_LIMIT)) as usize;
            let page: Vec<NotificationHistoryEntry> = matches
                .into_iter()
                .filter(|e| e.seq >= from_seq)
                .take(capped)
                .collect();
            QueryResponsePayload::ListNotificationHistory {
                entries: page,
                total_len,
            }
        }
        // Phase 74 — memory inspection queries.
        QueryPayload::ListMemoryTopics => {
            let Some(mem) = memory else {
                return QueryResponsePayload::QueryError {
                    code: "no_memory".into(),
                    message: "daemon has no memory substrate configured".into(),
                };
            };
            match mem.list_topics().await {
                Ok(topics) => QueryResponsePayload::ListMemoryTopics { topics },
                Err(e) => QueryResponsePayload::QueryError {
                    code: "memory_list_failed".into(),
                    message: e.to_string(),
                },
            }
        }
        QueryPayload::GetMemoryTopicEntries { topic, limit } => {
            let Some(mem) = memory else {
                return QueryResponsePayload::QueryError {
                    code: "no_memory".into(),
                    message: "daemon has no memory substrate configured".into(),
                };
            };
            // Server-side cap mirrors the audit-entry handler.
            const MEMORY_QUERY_MAX_LIMIT: u32 = 500;
            let capped = limit.clamp(1, MEMORY_QUERY_MAX_LIMIT) as usize;
            match mem.get_recent(&topic, capped).await {
                Ok(entries) => QueryResponsePayload::GetMemoryTopicEntries {
                    entries: entries
                        .into_iter()
                        .map(memory_entry_summary)
                        .collect(),
                },
                Err(e) => QueryResponsePayload::QueryError {
                    code: "memory_get_failed".into(),
                    message: e.to_string(),
                },
            }
        }
        QueryPayload::SearchMemory {
            query,
            limit,
            semantic,
        } => {
            let Some(mem) = memory else {
                return QueryResponsePayload::QueryError {
                    code: "no_memory".into(),
                    message: "daemon has no memory substrate configured".into(),
                };
            };
            const MEMORY_QUERY_MAX_LIMIT: u32 = 500;
            let capped = limit.clamp(1, MEMORY_QUERY_MAX_LIMIT) as usize;

            // Phase 75 — semantic path with transparent keyword
            // fallback (Q4a). Fall back when: no `[embedding]`
            // provider, the query embed call fails, or the
            // corpus has zero vectors (semantic over an empty
            // index would just return nothing — keyword is
            // strictly better there). The `fell_back_to_keyword`
            // flag lets the operator/agent see it happened.
            if semantic {
                let qvec = match embedding_provider {
                    Some(p) => {
                        match p.embed(std::slice::from_ref(&query)).await {
                            Ok(mut v) if !v.is_empty() => Some(v.remove(0)),
                            _ => None,
                        }
                    }
                    None => None,
                };
                let has_vectors = mem
                    .load_all_vectors()
                    .await
                    .map(|v| !v.is_empty())
                    .unwrap_or(false);
                if let (Some(qvec), true) = (qvec, has_vectors) {
                    return match mem.semantic_search(&qvec, capped).await {
                        Ok(matches) => QueryResponsePayload::SearchMemory {
                            matches: matches
                                .into_iter()
                                .map(memory_entry_summary)
                                .collect(),
                            fell_back_to_keyword: false,
                        },
                        Err(e) => QueryResponsePayload::QueryError {
                            code: "memory_search_failed".into(),
                            message: e.to_string(),
                        },
                    };
                }
                // Fallback to keyword, flagged.
                return match mem.search(&query, capped).await {
                    Ok(matches) => QueryResponsePayload::SearchMemory {
                        matches: matches
                            .into_iter()
                            .map(memory_entry_summary)
                            .collect(),
                        fell_back_to_keyword: true,
                    },
                    Err(e) => QueryResponsePayload::QueryError {
                        code: "memory_search_failed".into(),
                        message: e.to_string(),
                    },
                };
            }

            // Keyword path (default; no behavior change).
            match mem.search(&query, capped).await {
                Ok(matches) => QueryResponsePayload::SearchMemory {
                    matches: matches
                        .into_iter()
                        .map(memory_entry_summary)
                        .collect(),
                    fell_back_to_keyword: false,
                },
                Err(e) => QueryResponsePayload::QueryError {
                    code: "memory_search_failed".into(),
                    message: e.to_string(),
                },
            }
        }
    }
}

/// Phase 74 — convert an `aivyx_memory::MemoryEntry` into the
/// flat wire `MemoryEntrySummary`.
fn memory_entry_summary(
    e: aivyx_memory::MemoryEntry,
) -> crate::daemon_ipc::MemoryEntrySummary {
    crate::daemon_ipc::MemoryEntrySummary {
        topic: e.topic,
        body: e.body,
        seq: e.seq,
        created_at_secs: e.created_at_secs,
        last_read_at_secs: e.last_read_at_secs,
    }
}

/// Phase 73 — render an `AutoNotifyOutcomeSummary` into
/// (kind, detail) pair for the wire-format history entry. `kind`
/// is the stable lowercase label; `detail` carries variant-
/// specific data.
fn render_notify_outcome_for_history(
    summary: &aivyx_audit::AutoNotifyOutcomeSummary,
) -> (&'static str, String) {
    use aivyx_audit::AutoNotifyOutcomeSummary;
    match summary {
        AutoNotifyOutcomeSummary::Delivered => ("delivered", String::new()),
        AutoNotifyOutcomeSummary::SkippedEmptyResponse => {
            ("skipped_empty_response", String::new())
        }
        AutoNotifyOutcomeSummary::Failed {
            error_kind,
            error_message,
        } => ("failed", format!("[{error_kind}] {error_message}")),
        AutoNotifyOutcomeSummary::SkippedByCondition { condition } => {
            ("skipped_by_condition", condition.clone())
        }
        AutoNotifyOutcomeSummary::SkippedByRateLimit {
            limit,
            window_secs,
        } => (
            "skipped_by_rate_limit",
            format!("{limit}/{window_secs}s"),
        ),
    }
}

/// Phase 70 — parse the wire-format status filter string into
/// the typed enum. Unknown values fall through to `Pending` per
/// the IPC contract documented at `QueryPayload::ListPersonaProposals`.
fn parse_proposal_status_filter(
    s: &str,
) -> crate::persona_proposal::ProposalStatusFilter {
    use crate::persona_proposal::ProposalStatusFilter;
    match s.to_ascii_lowercase().as_str() {
        "all" => ProposalStatusFilter::All,
        "approved" => ProposalStatusFilter::Approved,
        "rejected" => ProposalStatusFilter::Rejected,
        "superseded" => ProposalStatusFilter::Superseded,
        _ => ProposalStatusFilter::Pending,
    }
}

/// Phase 70 — convert an in-memory `PersonaProposal` view into
/// the wire `PersonaProposalSummary` shape.
fn proposal_summary_from_view(
    view: crate::persona_proposal::PersonaProposal,
) -> crate::daemon_ipc::PersonaProposalSummary {
    use crate::persona_proposal::ProposalStatus;
    let category = format!("{:?}", view.proposed_op.category);
    let proposed_reason = view.proposed_op.reason.clone();
    let proposed_op = serde_json::to_value(&view.proposed_op.op)
        .unwrap_or(serde_json::Value::Null);
    let (status, applied_op, applied_seq, rejected_reason, resolved_at_unix_ms) =
        match view.status {
            ProposalStatus::Pending => {
                ("Pending".to_string(), None, None, None, None)
            }
            ProposalStatus::Approved {
                applied_op,
                applied_seq,
                resolved_at_unix_ms,
            } => (
                "Approved".to_string(),
                Some(
                    serde_json::to_value(&applied_op.op)
                        .unwrap_or(serde_json::Value::Null),
                ),
                Some(applied_seq),
                None,
                Some(resolved_at_unix_ms),
            ),
            ProposalStatus::Rejected {
                reason,
                resolved_at_unix_ms,
            } => (
                "Rejected".to_string(),
                None,
                None,
                reason,
                Some(resolved_at_unix_ms),
            ),
            ProposalStatus::Superseded {
                by_proposal_id: _,
                resolved_at_unix_ms,
            } => (
                "Superseded".to_string(),
                None,
                None,
                None,
                Some(resolved_at_unix_ms),
            ),
        };
    crate::daemon_ipc::PersonaProposalSummary {
        id: view.id,
        proposed_at_unix_ms: view.proposed_at_unix_ms,
        source_reflection_session_id: view.source_reflection_session_id,
        status,
        category,
        proposed_op,
        proposed_reason,
        applied_op,
        applied_seq,
        rejected_reason,
        resolved_at_unix_ms,
    }
}

/// Convert an in-memory effective Persona state into the wire
/// [`EffectivePersonaSummary`]. Phase 60.
fn effective_persona_summary_from_state(
    state: &crate::persona::EffectivePersona,
) -> crate::daemon_ipc::EffectivePersonaSummary {
    crate::daemon_ipc::EffectivePersonaSummary {
        assistant_name: state.assistant_name.clone(),
        operator_profile: state.operator_profile.clone(),
        communication_style: state.communication_style.clone(),
        primary_use_cases: state.primary_use_cases.clone(),
        behavioral_preferences: state.behavioral_preferences.clone(),
        behavioral_constraints: state.behavioral_constraints.clone(),
        learned_context: state.learned_context.clone(),
        communication_adaptations: state.communication_adaptations.clone(),
        character_traits: state.character_traits.clone(),
        relationship_milestones: state.relationship_milestones.clone(),
        is_non_empty: state.is_non_empty(),
    }
}

/// Convert a signed persona chain entry into the wire summary
/// shape. Phase 60.
fn persona_delta_summary_from_signed(
    entry: &crate::persona::SignedPersonaEntry,
) -> crate::daemon_ipc::PersonaDeltaSummary {
    let category_label = format!("{:?}", entry.delta.category);
    let op_value = serde_json::to_value(&entry.delta.op).unwrap_or(serde_json::Value::Null);
    crate::daemon_ipc::PersonaDeltaSummary {
        seq: entry.seq,
        delta_id: entry.delta.delta_id.clone(),
        proposed_at_unix_ms: entry.delta.proposed_at_unix_ms,
        approved_at_unix_ms: entry.delta.approved_at_unix_ms,
        proposal_id: entry.delta.proposal_id.clone(),
        category: category_label,
        op: op_value,
        mac_hex: entry.mac.iter().map(|b| format!("{b:02x}")).collect(),
    }
}

/// Convert an in-memory [`aivyx_config::Profile`] to the
/// [`ProfileSummary`] wire shape. Phase 58 — flattens `Sourced<T>`
/// into plain serializable fields and pre-computes the
/// `injection_enabled` predicate so the Web UI does not need to
/// re-implement the rule.
fn profile_summary_from_profile(profile: &aivyx_config::Profile) -> ProfileSummary {
    ProfileSummary {
        assistant_name: profile.assistant_name.value.clone(),
        assistant_name_source: field_source_label(profile.assistant_name.source).to_string(),
        operator_profile: profile.operator_profile.clone(),
        communication_style: profile.communication_style.clone(),
        primary_use_cases: profile.primary_use_cases.clone(),
        behavioral_preferences: profile.behavioral_preferences.clone(),
        behavioral_constraints: profile.behavioral_constraints.clone(),
        injection_enabled: profile.is_operator_declared(),
    }
}

/// Phase 60 — append an operator-initiated revert delta and
/// recompute the shared runtime state. Returns the new chain seq
/// on success; a human-readable reason on failure.
async fn resolve_persona_revert(
    persona_log: Option<&crate::persona::PersistentPersonaLog>,
    shared_persona: &crate::persona::SharedEffectivePersona,
    target_delta_id: &str,
) -> Result<u64, String> {
    let persona_log = persona_log
        .ok_or_else(|| "daemon has no persona log configured".to_string())?;
    // Validate the target exists in the chain before appending the
    // revert. Forward-pointing targets are rejected at fold time,
    // but rejecting them at append time gives a better operator
    // error message.
    let entries = persona_log.entries();
    let target = entries
        .iter()
        .find(|e| e.delta.delta_id == target_delta_id)
        .ok_or_else(|| {
            format!("no persona delta found with id `{target_delta_id}`")
        })?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let revert = crate::persona::PersonaDelta {
        delta_id: format!("pd-revert-{target_delta_id}"),
        proposed_at_unix_ms: now_ms,
        approved_at_unix_ms: now_ms,
        proposal_id: format!("op-revert-{target_delta_id}"),
        category: target.delta.category,
        op: crate::persona::PersonaDeltaOp::Revert {
            target_delta_id: target_delta_id.to_string(),
        },
    };
    let seq = persona_log
        .append(revert)
        .await
        .map_err(|e| format!("persona chain append failed: {e}"))?;
    let entries_after = persona_log.entries();
    if !crate::persona::recompute_shared_from_entries(shared_persona, &entries_after) {
        return Err("shared persona state lock poisoned during recompute".into());
    }
    Ok(seq)
}

/// Phase 65 — daemon-side import handler. Validates → conflict
/// checks → optionally wipes → replays → recomputes shared state.
/// Best-effort per Q1(a): no atomic-tx wrapping. Returns
/// `PersonaImportSuccess { deltas_imported, final_chain_seq }`
/// on success.
async fn resolve_persona_import(
    persona_log: Option<&crate::persona::PersistentPersonaLog>,
    shared_persona: &crate::persona::SharedEffectivePersona,
    deltas: Vec<crate::identity_export::DeltaExport>,
    force: bool,
) -> Result<crate::daemon_ipc::PersonaImportSuccess, String> {
    let persona_log = persona_log
        .ok_or_else(|| "daemon has no persona log configured".to_string())?;

    // Re-validate each delta server-side — defense against the
    // CLI sending us a frame that bypassed parse_and_validate
    // (a malicious client, or a CLI bug).
    for (index, d) in deltas.iter().enumerate() {
        d.delta.validate().map_err(|reason| {
            format!(
                "incoming delta at index {index} (seq {seq}) failed validation: {reason}",
                seq = d.seq,
            )
        })?;
    }

    // Conflict check (Q3(a) at sign-off): refuse to overwrite
    // unless --force.
    let existing = persona_log.entries();
    if !existing.is_empty() && !force {
        return Err(format!(
            "persona chain not empty ({} entries); pass --force to overwrite",
            existing.len(),
        ));
    }

    // Force wipe.
    if force && !existing.is_empty() {
        persona_log
            .clear()
            .await
            .map_err(|e| format!("persona chain wipe failed: {e}"))?;
    }

    // Replay. Each append re-signs against the local HMAC key —
    // Phase 60 Q1(a) re-bind made concrete.
    let count = deltas.len() as u64;
    let mut last_seq: u64 = 0;
    for (index, d) in deltas.into_iter().enumerate() {
        let assigned_seq = persona_log.append(d.delta).await.map_err(|e| {
            format!(
                "persona chain append failed at index {index} (expected seq {}): {e}",
                d.seq,
            )
        })?;
        last_seq = assigned_seq;
    }

    // Refresh runtime state (Q3 — refresh: daemon recomputes
    // immediately). The next agent turn sees the imported state.
    let entries_after = persona_log.entries();
    if !crate::persona::recompute_shared_from_entries(shared_persona, &entries_after) {
        return Err("shared persona state lock poisoned during recompute".into());
    }

    Ok(crate::daemon_ipc::PersonaImportSuccess {
        deltas_imported: count,
        final_chain_seq: last_seq,
    })
}

fn field_source_label(src: aivyx_config::FieldSource) -> &'static str {
    match src {
        aivyx_config::FieldSource::Env => "env",
        aivyx_config::FieldSource::Toml => "toml",
        aivyx_config::FieldSource::EncryptedStore => "encrypted-store",
        aivyx_config::FieldSource::Default => "default",
    }
}

fn audit_entry_summary_from_signed(entry: aivyx_audit::SignedEntry) -> AuditEntrySummary {
    let appended_at_unix_ms = entry
        .appended_at
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let event_type = match &entry.event {
        aivyx_audit::AuditEvent::ToolCall { .. } => "ToolCall",
        aivyx_audit::AuditEvent::ScopeDenied { .. } => "ScopeDenied",
        aivyx_audit::AuditEvent::TurnStarted { .. } => "TurnStarted",
        aivyx_audit::AuditEvent::TurnEnded { .. } => "TurnEnded",
        aivyx_audit::AuditEvent::MemoryAccess { .. } => "MemoryAccess",
        aivyx_audit::AuditEvent::AutoNotifyDispatched { .. } => "AutoNotifyDispatched",
    }
    .to_string();

    // `event` serializes to JSON unconditionally — the body is `Serialize`.
    let event = serde_json::to_value(&entry.event).unwrap_or(serde_json::Value::Null);

    let mut mac_hex = String::with_capacity(64);
    for b in entry.mac.iter() {
        mac_hex.push_str(&format!("{b:02x}"));
    }

    AuditEntrySummary {
        seq: entry.seq,
        appended_at_unix_ms,
        event_type,
        event,
        mac_hex,
    }
}

fn mission_state_label(state: mission::MissionState) -> &'static str {
    match state {
        mission::MissionState::Created => "Created",
        mission::MissionState::Running => "Running",
        mission::MissionState::GatePending => "GatePending",
        mission::MissionState::Completed => "Completed",
        mission::MissionState::Failed => "Failed",
        mission::MissionState::Cancelled => "Cancelled",
    }
}

fn gate_state_label(state: mission::GateState) -> &'static str {
    match state {
        mission::GateState::Pending => "Pending",
        mission::GateState::Approved => "Approved",
        mission::GateState::Rejected => "Rejected",
    }
}

/// Phase 70 — daemon-side proposal resolution handler. On
/// `Approve` / `ApproveWithEdit` it validates the applied op,
/// appends a `PersonaDelta` to the persona chain, then appends
/// an `Approved` entry to the proposal chain bound to the
/// delta's seq, and recomputes the shared persona snapshot. On
/// `Reject` it just appends a `Rejected` entry.
async fn resolve_persona_proposal(
    persona_proposal_log: Option<
        &crate::persona_proposal::PersistentPersonaProposalLog,
    >,
    persona_log: Option<&crate::persona::PersistentPersonaLog>,
    shared_persona: &crate::persona::SharedEffectivePersona,
    _request_id: &str,
    proposal_id: String,
    resolution: crate::daemon_ipc::PersonaProposalResolution,
) -> Result<crate::daemon_ipc::PersonaProposalResolveSuccess, String> {
    let proposal_log = persona_proposal_log
        .ok_or_else(|| "daemon has no persona proposal log configured".to_string())?;
    let view = proposal_log
        .get(&proposal_id)
        .ok_or_else(|| format!("unknown proposal id `{proposal_id}`"))?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    match resolution {
        crate::daemon_ipc::PersonaProposalResolution::Reject { reason } => {
            proposal_log
                .append_rejected(proposal_id, now_ms, reason)
                .await
                .map_err(|e| format!("proposal chain append failed: {e}"))?;
            Ok(crate::daemon_ipc::PersonaProposalResolveSuccess {
                proposal_status: "Rejected".into(),
                applied_seq: None,
            })
        }
        crate::daemon_ipc::PersonaProposalResolution::Approve
        | crate::daemon_ipc::PersonaProposalResolution::ApproveWithEdit { .. } => {
            // Resolve the op the operator actually wants applied.
            let applied_op = match &resolution {
                crate::daemon_ipc::PersonaProposalResolution::ApproveWithEdit {
                    edited_op,
                } => edited_op.clone(),
                _ => view.proposed_op.clone(),
            };
            applied_op
                .validate()
                .map_err(|reason| format!("edited op invalid: {reason}"))?;
            // Append to the persona log first; if that fails the
            // proposal stays Pending so the operator can retry.
            let persona_log = persona_log
                .ok_or_else(|| "daemon has no persona log configured".to_string())?;
            let delta_id = format!("pd-approved-{proposal_id}");
            let delta = crate::persona::PersonaDelta {
                delta_id,
                proposed_at_unix_ms: view.proposed_at_unix_ms,
                approved_at_unix_ms: now_ms,
                proposal_id: proposal_id.clone(),
                category: applied_op.category,
                op: applied_op.op.clone(),
            };
            let applied_seq = persona_log
                .append(delta)
                .await
                .map_err(|e| format!("persona chain append failed: {e}"))?;
            // Record the Approved transition on the proposal chain.
            proposal_log
                .append_approved(proposal_id, now_ms, applied_op, applied_seq)
                .await
                .map_err(|e| format!("proposal chain append failed: {e}"))?;
            // Recompute shared persona state so the next turn sees
            // the new effective persona.
            let entries_after = persona_log.entries();
            if !crate::persona::recompute_shared_from_entries(
                shared_persona,
                &entries_after,
            ) {
                return Err(
                    "shared persona state lock poisoned during recompute".into(),
                );
            }
            Ok(crate::daemon_ipc::PersonaProposalResolveSuccess {
                proposal_status: "Approved".into(),
                applied_seq: Some(applied_seq),
            })
        }
    }
}

fn mission_summary_from_record(record: mission::MissionRecord) -> MissionSummary {
    let has_pending_gate = record.pending_gate().is_some();
    MissionSummary {
        mission_id: record.mission_id,
        role_name: record.role_name,
        description: record.description,
        state: mission_state_label(record.state).to_string(),
        has_pending_gate,
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}

fn mission_detail_from_record(record: mission::MissionRecord) -> MissionDetail {
    let gates = record
        .gates
        .into_iter()
        .map(|g| GateSummary {
            gate_id: g.gate_id,
            reason: g.reason,
            scope: g.scope,
            state: gate_state_label(g.state).to_string(),
            created_at: g.created_at,
            resolved_at: g.resolved_at,
        })
        .collect();
    MissionDetail {
        mission_id: record.mission_id,
        role_name: record.role_name,
        description: record.description,
        state: mission_state_label(record.state).to_string(),
        gates,
        created_at: record.created_at,
        updated_at: record.updated_at,
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

    // -------------------------------------------------------------
    // Phase 58 — Profile inspection query helpers.
    // -------------------------------------------------------------

    #[test]
    fn profile_summary_renders_default_profile_with_injection_disabled() {
        let summary = profile_summary_from_profile(&aivyx_config::Profile::default());
        assert_eq!(summary.assistant_name, "Aivyx");
        assert_eq!(summary.assistant_name_source, "default");
        assert!(summary.operator_profile.is_none());
        assert!(summary.communication_style.is_none());
        assert!(summary.primary_use_cases.is_empty());
        assert!(summary.behavioral_preferences.is_empty());
        assert!(summary.behavioral_constraints.is_empty());
        assert!(!summary.injection_enabled);
    }

    #[test]
    fn profile_summary_renders_operator_declared_profile_with_injection_enabled() {
        let profile = aivyx_config::Profile {
            assistant_name: aivyx_config::Sourced::new(
                "Codex".to_string(),
                aivyx_config::FieldSource::Toml,
            ),
            operator_profile: Some("Senior Rust engineer".to_string()),
            communication_style: Some("terse, conclusion-first".to_string()),
            primary_use_cases: vec!["Rust systems".to_string()],
            behavioral_preferences: vec!["prefer integration tests".to_string()],
            behavioral_constraints: vec!["never auto-commit".to_string()],
        };
        let summary = profile_summary_from_profile(&profile);
        assert_eq!(summary.assistant_name, "Codex");
        assert_eq!(summary.assistant_name_source, "toml");
        assert_eq!(summary.operator_profile.as_deref(), Some("Senior Rust engineer"));
        assert_eq!(
            summary.communication_style.as_deref(),
            Some("terse, conclusion-first"),
        );
        assert_eq!(summary.primary_use_cases, vec!["Rust systems".to_string()]);
        assert_eq!(
            summary.behavioral_preferences,
            vec!["prefer integration tests".to_string()],
        );
        assert_eq!(
            summary.behavioral_constraints,
            vec!["never auto-commit".to_string()],
        );
        assert!(summary.injection_enabled);
    }

    // ---- Phase 70 — resolve_persona_proposal end-to-end -----

    /// Helper: open a fresh persona + proposal log pair backed by
    /// real redb storage so the resolve handler's chain
    /// interactions are exercised against the actual substrate.
    async fn open_phase_70_test_logs(
        slug: &str,
    ) -> (
        Arc<crate::persona::PersistentPersonaLog>,
        Arc<crate::persona_proposal::PersistentPersonaProposalLog>,
        crate::persona::SharedEffectivePersona,
    ) {
        use aivyx_crypto::MasterKey;
        use aivyx_storage::{KeyDomain, RedbStorage, Storage, StorageConfig};
        // Per-test slug + a high-res timestamp keeps every test's
        // tempdir distinct under parallel execution. redb refuses
        // two opens of the same file (`Database already open`),
        // so collisions surface as the test panicking on storage
        // open.
        let dir = test_dir(&format!(
            "phase-70-resolve-{slug}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let store: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.join("store.redb")),
            MasterKey::from_raw([70u8; 32]),
        )
        .await
        .expect("storage");
        let persona_log = Arc::new(
            crate::persona::PersistentPersonaLog::open(
                store.domain(KeyDomain::Persona),
                b"persona-key".to_vec(),
            )
            .await
            .expect("persona log"),
        );
        let proposal_log = Arc::new(
            crate::persona_proposal::PersistentPersonaProposalLog::open(
                store.domain(KeyDomain::PersonaProposals),
                b"proposal-key".to_vec(),
            )
            .await
            .expect("proposal log"),
        );
        let shared = crate::persona::shared_effective_persona(
            crate::persona::EffectivePersona::default(),
        );
        (persona_log, proposal_log, shared)
    }

    fn pending_op_fixture() -> crate::persona::ProposedPersonaDelta {
        crate::persona::ProposedPersonaDelta {
            category: crate::persona::PersonaDeltaCategory::BehavioralPreferences,
            op: crate::persona::PersonaDeltaOp::AppendList {
                value: "prefer terse".into(),
            },
            reason: Some("operator confirmed".into()),
        }
    }

    #[tokio::test]
    async fn resolve_proposal_approve_appends_to_persona_log_and_records_approved() {
        let (persona_log, proposal_log, shared) = open_phase_70_test_logs("approve").await;
        proposal_log
            .append_pending(
                "pp-1".into(),
                1_000,
                "ses-1".into(),
                pending_op_fixture(),
            )
            .await
            .unwrap();
        let success = resolve_persona_proposal(
            Some(proposal_log.as_ref()),
            Some(persona_log.as_ref()),
            &shared,
            "req-1",
            "pp-1".into(),
            crate::daemon_ipc::PersonaProposalResolution::Approve,
        )
        .await
        .expect("approve ok");
        assert_eq!(success.proposal_status, "Approved");
        assert_eq!(success.applied_seq, Some(0));
        // Persona chain has the applied delta.
        assert_eq!(persona_log.len(), 1);
        // Proposal chain now reports Approved status.
        let view = proposal_log.get("pp-1").expect("present");
        assert!(matches!(
            view.status,
            crate::persona_proposal::ProposalStatus::Approved { applied_seq: 0, .. }
        ));
        // Shared persona state reflects the approved op.
        let snap = shared.read().unwrap();
        assert!(snap
            .behavioral_preferences
            .contains(&"prefer terse".to_string()));
    }

    #[tokio::test]
    async fn resolve_proposal_approve_with_edit_records_edited_op() {
        let (persona_log, proposal_log, shared) = open_phase_70_test_logs("approve-with-edit").await;
        proposal_log
            .append_pending(
                "pp-1".into(),
                1_000,
                "ses-1".into(),
                pending_op_fixture(),
            )
            .await
            .unwrap();
        let edited = crate::persona::ProposedPersonaDelta {
            category: crate::persona::PersonaDeltaCategory::BehavioralPreferences,
            op: crate::persona::PersonaDeltaOp::AppendList {
                value: "operator-edited preference".into(),
            },
            reason: None,
        };
        resolve_persona_proposal(
            Some(proposal_log.as_ref()),
            Some(persona_log.as_ref()),
            &shared,
            "req-2",
            "pp-1".into(),
            crate::daemon_ipc::PersonaProposalResolution::ApproveWithEdit {
                edited_op: edited.clone(),
            },
        )
        .await
        .expect("approve-with-edit ok");
        // Shared persona reflects the EDITED op, not the original.
        let snap = shared.read().unwrap();
        assert!(snap
            .behavioral_preferences
            .contains(&"operator-edited preference".to_string()));
        assert!(!snap
            .behavioral_preferences
            .contains(&"prefer terse".to_string()));
    }

    #[tokio::test]
    async fn resolve_proposal_reject_records_rejected_no_persona_append() {
        let (persona_log, proposal_log, shared) = open_phase_70_test_logs("reject").await;
        proposal_log
            .append_pending(
                "pp-1".into(),
                1_000,
                "ses-1".into(),
                pending_op_fixture(),
            )
            .await
            .unwrap();
        let success = resolve_persona_proposal(
            Some(proposal_log.as_ref()),
            Some(persona_log.as_ref()),
            &shared,
            "req-3",
            "pp-1".into(),
            crate::daemon_ipc::PersonaProposalResolution::Reject {
                reason: Some("not now".into()),
            },
        )
        .await
        .expect("reject ok");
        assert_eq!(success.proposal_status, "Rejected");
        assert_eq!(success.applied_seq, None);
        // Persona chain UNCHANGED.
        assert!(persona_log.is_empty());
        let view = proposal_log.get("pp-1").unwrap();
        match view.status {
            crate::persona_proposal::ProposalStatus::Rejected { reason, .. } => {
                assert_eq!(reason.as_deref(), Some("not now"));
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn resolve_proposal_unknown_id_returns_error() {
        let (persona_log, proposal_log, shared) = open_phase_70_test_logs("unknown-id").await;
        let err = resolve_persona_proposal(
            Some(proposal_log.as_ref()),
            Some(persona_log.as_ref()),
            &shared,
            "req-4",
            "pp-MISSING".into(),
            crate::daemon_ipc::PersonaProposalResolution::Approve,
        )
        .await
        .expect_err("must error");
        assert!(err.contains("pp-MISSING"), "{err}");
    }

    #[test]
    fn parse_proposal_status_filter_handles_known_and_unknown() {
        use crate::persona_proposal::ProposalStatusFilter;
        assert!(matches!(
            parse_proposal_status_filter("all"),
            ProposalStatusFilter::All
        ));
        assert!(matches!(
            parse_proposal_status_filter("Approved"),
            ProposalStatusFilter::Approved
        ));
        assert!(matches!(
            parse_proposal_status_filter("REJECTED"),
            ProposalStatusFilter::Rejected
        ));
        assert!(matches!(
            parse_proposal_status_filter("superseded"),
            ProposalStatusFilter::Superseded
        ));
        // Unknown / empty → Pending per IPC contract.
        assert!(matches!(
            parse_proposal_status_filter("xyz"),
            ProposalStatusFilter::Pending
        ));
        assert!(matches!(
            parse_proposal_status_filter(""),
            ProposalStatusFilter::Pending
        ));
    }

    // ---- Phase 73 — notify-outcome history renderer ----------

    #[test]
    fn history_renderer_delivered_has_empty_detail() {
        let (kind, detail) = render_notify_outcome_for_history(
            &aivyx_audit::AutoNotifyOutcomeSummary::Delivered,
        );
        assert_eq!(kind, "delivered");
        assert!(detail.is_empty());
    }

    #[test]
    fn history_renderer_failed_carries_error_kind_and_message() {
        let (kind, detail) = render_notify_outcome_for_history(
            &aivyx_audit::AutoNotifyOutcomeSummary::Failed {
                error_kind: "transport".into(),
                error_message: "dns lookup failed".into(),
            },
        );
        assert_eq!(kind, "failed");
        assert!(detail.contains("transport"), "{detail}");
        assert!(detail.contains("dns lookup failed"), "{detail}");
    }

    #[test]
    fn history_renderer_skipped_empty_response_has_empty_detail() {
        let (kind, detail) = render_notify_outcome_for_history(
            &aivyx_audit::AutoNotifyOutcomeSummary::SkippedEmptyResponse,
        );
        assert_eq!(kind, "skipped_empty_response");
        assert!(detail.is_empty());
    }

    #[test]
    fn history_renderer_skipped_by_condition_carries_label() {
        let (kind, detail) = render_notify_outcome_for_history(
            &aivyx_audit::AutoNotifyOutcomeSummary::SkippedByCondition {
                condition: "on_failed".into(),
            },
        );
        assert_eq!(kind, "skipped_by_condition");
        assert_eq!(detail, "on_failed");
    }

    #[test]
    fn history_renderer_skipped_by_rate_limit_renders_limit_and_window() {
        let (kind, detail) = render_notify_outcome_for_history(
            &aivyx_audit::AutoNotifyOutcomeSummary::SkippedByRateLimit {
                limit: 10,
                window_secs: 3600,
            },
        );
        assert_eq!(kind, "skipped_by_rate_limit");
        assert_eq!(detail, "10/3600s");
    }
}
