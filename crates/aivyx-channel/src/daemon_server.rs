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

use aivyx_audit::{AuditWriter, PersistentAuditLog};
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
    /// Phase 82 — the durable helpfulness ledger. `Some` iff
    /// the recall substrate is configured (zero-config, built
    /// alongside the recall log); the reflection recall-feedback
    /// pass folds each window into it. `None` → no fold (a
    /// passive add-on; recall-feedback is unaffected).
    pub helpfulness_ledger: Option<
        Arc<crate::helpfulness_ledger::PersistentHelpfulnessLedger>,
    >,
    /// Phase 83 — the durable cross-session co-occurrence
    /// ledger. `Some` iff the recall substrate is configured
    /// (zero-config, built alongside the recall log); the
    /// reflection pass folds each window's pairs into it.
    /// `None` → no fold (a passive add-on).
    pub cooccurrence_ledger: Option<
        Arc<
            crate::cooccurrence_ledger::PersistentCooccurrenceLedger,
        >,
    >,
    /// Phase 172 — the durable correction ledger. `Some` iff
    /// the recall substrate is configured (zero-config, built
    /// alongside the recall log); the reflection recall-feedback
    /// pass folds each window's per-topic correction counts into
    /// it, and the consolidation pass reads it. `None` → no fold
    /// (a passive add-on).
    pub correction_ledger: Option<
        Arc<crate::correction_ledger::PersistentCorrectionLedger>,
    >,
    /// Phase 79 (Q4a) — shared last-Persona-selection stat the
    /// adaptive refiner writes and `GetLearningInsights` reads.
    /// `None` → adaptive Persona not configured (the surface
    /// reports no selection).
    pub persona_selection_stat:
        Option<crate::persona_context::SharedPersonaSelectionStat>,
    /// Phase 84 (Q4a) — shared last-turn cluster-recall stat
    /// the recall provider writes and `GetLearningInsights`
    /// reads. `None` → cluster expansion not armed (the
    /// surface reports none).
    pub recall_cluster_stat:
        Option<crate::memory_recall::SharedRecallClusterStat>,
    /// Phase 80 — `[proactive]` config. `None` (no section) →
    /// proactive surfacing is off; even `Some` no-ops unless
    /// `enabled`.
    pub proactive_config: Option<aivyx_config::ProactiveConfig>,
    /// Phase 80 — proactive dedup log. `Some` iff proactive is
    /// armed; the reflection cron pass uses it for cross-cycle
    /// dedup + the per-window cap.
    pub proactive_log:
        Option<Arc<crate::proactive_log::PersistentProactiveLog>>,
    /// Phase 80 (Q4a) — shared last-proactive-cycle stat the
    /// pass writes and `GetLearningInsights` reads. `None` →
    /// proactive not armed (the surface reports none).
    pub proactive_stat:
        Option<crate::proactive_detect::SharedProactiveStat>,
    /// Phase 81 — `[persona_lifecycle]` config. `None` (no
    /// section) → the Persona never self-consolidates or
    /// decays; even `Some` no-ops unless `enabled`.
    pub persona_lifecycle_config:
        Option<aivyx_config::PersonaLifecycleConfig>,
    /// Phase 81 (Q4a) — shared last-lifecycle-cycle stat the
    /// pass writes and `GetLearningInsights` reads. `None` →
    /// the lifecycle pass is not armed (the surface reports
    /// none).
    pub persona_lifecycle_stat: Option<
        crate::persona_lifecycle::SharedPersonaLifecycleStat,
    >,
    /// Phase 86 — daemon-scoped, per-session conversation
    /// windows. `Some` iff the recall substrate is configured
    /// (built alongside the recall log at daemon startup, same
    /// life cycle as the shared persona-selection / recall-
    /// cluster stats). The daemon turn loop writes a `(user,
    /// assistant)` pair into the matching session's ring on each
    /// `TurnOutcome::Completed`; both relevance providers read
    /// via `assemble_for` when `recall_window_turns` is greater
    /// than 1. `None` → the Phase 86 window is off (every
    /// recall query is byte-identical to pre-Phase-86).
    pub conversation_windows:
        Option<crate::conversation_window::SharedConversationWindows>,
    /// Phase 87 — `[persona_consolidation]` config. `None` (no
    /// section) → pattern-driven proposals are off; even
    /// `Some` no-ops unless `enabled`. The reflection pass
    /// reads this alongside the co-occurrence + helpfulness
    /// ledgers + the proposal chain.
    pub persona_consolidation_config:
        Option<aivyx_config::PersonaConsolidationConfig>,
    /// Phase 87 (Q4a) — shared last-cycle consolidation stat
    /// the pass writes and `GetLearningInsights` reads. `None`
    /// → consolidation not armed (the surface reports none).
    pub persona_consolidation_stat: Option<
        crate::persona_consolidation::SharedPersonaConsolidationStat,
    >,
    /// Phase 87 — production `PairPhraser` for the
    /// LLM-summarized facet phrasing (Q2b). `None` → the pass
    /// has no LLM access and skips the cycle (the actuator
    /// stays best-effort).
    pub persona_consolidation_phraser: Option<
        std::sync::Arc<
            dyn crate::persona_consolidation::PairPhraser,
        >,
    >,
    /// Phase 172 — `[correction_consolidation]` config. `None`
    /// (no section) → correction-driven proposals are off (the
    /// correction ledger still accumulates passively); `Some`
    /// arms the reflection-cron pass only when `enabled = true`.
    pub correction_consolidation_config:
        Option<aivyx_config::CorrectionConsolidationConfig>,
    /// Phase 172 — shared last-cycle correction-consolidation
    /// stat the pass writes and `GetLearningInsights` reads.
    /// `None` → not armed (the surface reports none).
    pub correction_consolidation_stat: Option<
        crate::correction_consolidation::SharedCorrectionConsolidationStat,
    >,
    /// Phase 172 — production `TopicPhraser` for the correction
    /// facet phrasing. `None` → the pass has no LLM access and
    /// skips the cycle (the actuator stays best-effort).
    pub correction_consolidation_phraser: Option<
        std::sync::Arc<
            dyn crate::correction_consolidation::TopicPhraser,
        >,
    >,
    /// Phase 91 — `[recall_judgment]` config. `None` (no
    /// section) → LLM-judged recall is off; `Some` arms the
    /// reflection-cron pass only when `enabled = true`.
    pub recall_judgment_config:
        Option<aivyx_config::RecallJudgmentConfig>,
    /// Phase 91 (Q4a) — shared last-cycle judgment stat the
    /// pass writes and `GetLearningInsights` reads. `None` →
    /// the pass has not run this daemon lifetime.
    pub recall_judgment_stat: Option<
        crate::recall_judgment::SharedRecallJudgmentStat,
    >,
    /// Phase 91 — production `RecallJudge` for the
    /// LLM-judged classification (Q2a). `None` → the pass
    /// has no LLM access and skips every cycle (the actuator
    /// stays best-effort).
    pub recall_judge: Option<
        std::sync::Arc<dyn crate::recall_judgment::RecallJudge>,
    >,
    /// Phase 93 — `[recall_feedback]` config. `None` (no
    /// section) → `correlate_detailed` runs with the
    /// pre-Phase-93 structural-only behaviour. `Some` with
    /// `use_judgment_signal = true` flips the correlator to
    /// per-hit judgment override (un-judged hits keep the
    /// structural fallback).
    pub recall_feedback_config:
        Option<aivyx_config::RecallFeedbackConfig>,
    /// Phase 102 — a static snapshot of the registered tool set,
    /// captured from the `ToolRegistry` at daemon construction.
    /// The `GetToolStats` query joins it against the audit chain
    /// so a registered-but-uncalled tool still appears. Empty for
    /// test fixtures / a daemon built without a registry.
    pub tool_descriptors: Vec<ToolDescriptor>,

    /// Phase 112 — Skill Auto-Proposer dependency bundle.
    /// `None` disables the feature entirely; `Some(ctx)` wires
    /// the post-finalize hook so every conversational turn
    /// fires `run_auto_propose_pipeline` in a detached
    /// `tokio::spawn` (Q2b inline-at-turn-boundary). The
    /// pipeline reads the audit log, persona log, persona
    /// proposal log, and shared persona handle that already
    /// live on this struct — only the LLM provider and the
    /// proposer config are bundled here.
    pub skill_auto_proposer:
        Option<Arc<crate::skill_auto_proposer::SkillAutoProposerContext>>,

    /// Phase 116 — Tool/skill relevance ledger handle.
    /// `None` disables the feature; `Some(handle)` wires the
    /// daemon's post-finalize hook to record per-turn tool
    /// outcomes (Phase 116 Task 4) and the system-prompt
    /// assembly to render the `## Tools recently used for
    /// similar tasks` section (Phase 116 Task 5).
    pub tool_relevance_ledger:
        Option<Arc<crate::tool_relevance_ledger::PersistentToolRelevanceLedger>>,
    /// Phase 173 — the autonomous-loop backlog (zero-config,
    /// always built when storage is configured) for the loop
    /// IPC handlers + the driver.
    pub loop_backlog:
        Option<Arc<crate::loop_backlog::PersistentLoopBacklog>>,
    /// Phase 173 — shared loop run state. `Some` only when the
    /// `[loop]` section is armed; the daemon spawns the loop
    /// driver and the IPC `loop start/stop/status` handlers
    /// flip / read this handle.
    pub loop_state: Option<crate::loop_driver::SharedLoopState>,
    /// Phase 173 — `[loop]` config (priority default +
    /// max-iterations ceiling). `None` when the section is
    /// absent.
    pub loop_config: Option<aivyx_config::LoopConfig>,
}

/// Phase 102 — a registered tool's listing fields, snapshotted
/// from the `ToolRegistry` at daemon construction for the
/// `GetToolStats` query. Not a wire type — the daemon joins this
/// with audit stats to produce the wire-format
/// [`crate::daemon_ipc::ToolStat`].
#[derive(Debug, Clone)]
pub struct ToolDescriptor {
    /// Tool name as the planner advertises it (e.g. `fs.read`).
    pub name: String,
    /// One-line tool description.
    pub description: String,
    /// Capability base the tool's audit `ToolCall` events key on
    /// — `required_scope(..).base()`. Equals `name` for most
    /// tools but not all (`web.fetch` keys on `net.fetch`).
    pub scope_base: String,
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
        helpfulness_ledger,
        cooccurrence_ledger,
        correction_ledger,
        persona_selection_stat,
        recall_cluster_stat,
        proactive_config,
        proactive_log,
        proactive_stat,
        persona_lifecycle_config,
        persona_lifecycle_stat,
        conversation_windows,
        persona_consolidation_config,
        persona_consolidation_stat,
        persona_consolidation_phraser,
        correction_consolidation_config,
        correction_consolidation_stat,
        correction_consolidation_phraser,
        recall_judgment_config,
        recall_judgment_stat,
        recall_judge,
        recall_feedback_config,
        tool_descriptors,
        skill_auto_proposer,
        tool_relevance_ledger,
        loop_backlog,
        loop_state,
        loop_config,
    } = config;
    // Phase 102 — shared once into every per-connection
    // `ConnectionContext` so `GetToolStats` can list the tool set.
    let tool_descriptors: Arc<[ToolDescriptor]> = tool_descriptors.into();
    let socket_path = &socket_path;
    let _ = std::fs::remove_file(socket_path);

    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| DaemonError::Bind { path: parent.display().to_string(), source: e })?;
    }

    // Phase 95 — shared per-schedule cadence stats (fired /
    // skipped counts, accumulated across the daemon lifetime).
    // Created once here; cloned into both the reflection-
    // scheduler spawn (writer) and per-connection contexts
    // (reader for `GetLearningInsights`).
    let cadence_stats =
        crate::reflection_scheduler::shared_recent_reflection_stats();

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

    // Phase 173 — spawn the autonomous-loop driver iff the
    // `[loop]` section is armed (loop_state is `Some`) AND the
    // backlog is present. The driver idles (no CPU) until an
    // `aivyx loop start` flips the shared run state; it then
    // fires TriggerSource::Loop turns until the backlog drains
    // or the max-iterations cap is hit.
    let _loop_driver_handle = match (&loop_state, &loop_backlog) {
        (Some(state), Some(backlog)) => {
            let ld_dispatch = trigger_dispatch.clone();
            let ld_backlog = Arc::clone(backlog);
            let ld_state = state.clone();
            let ld_shutdown = shutdown.clone();
            // Phase 174 — build the gate runner from the armed
            // `[loop]` config (gate_command + working_dir +
            // timeout). `None` → no driver-side verification.
            let ld_gate: Option<
                Arc<dyn crate::loop_gate::GateRunner>,
            > = loop_config.as_ref().and_then(|c| {
                c.gate_command.as_ref().map(|cmd| {
                    Arc::new(crate::loop_gate::ShellGateRunner::new(
                        cmd.clone(),
                        c.working_dir.as_ref().map(std::path::PathBuf::from),
                        std::time::Duration::from_secs(c.gate_timeout_secs),
                    ))
                        as Arc<dyn crate::loop_gate::GateRunner>
                })
            });
            let ld_max_run_secs =
                loop_config.as_ref().and_then(|c| c.max_run_secs);
            // Phase 175 — the progress log: the driver reads
            // recent notes from the shared memory handle and
            // injects them into each iteration's prompt.
            let ld_memory = memory.clone();
            let ld_progress_inject = loop_config
                .as_ref()
                .map(|c| c.progress_inject_count)
                .unwrap_or(0);
            eprintln!(
                "aivyx loop: driver armed (max_iterations ceiling={}, \
                 gate={}, max_run_secs={:?}, progress_inject={})",
                loop_config
                    .as_ref()
                    .map(|c| c.max_iterations)
                    .unwrap_or(0),
                if ld_gate.is_some() { "on" } else { "off" },
                ld_max_run_secs,
                ld_progress_inject,
            );
            Some(tokio::spawn(async move {
                crate::loop_driver::run_loop_driver(
                    ld_dispatch,
                    ld_backlog,
                    ld_state,
                    ld_gate,
                    ld_max_run_secs,
                    ld_memory,
                    ld_progress_inject,
                    ld_shutdown,
                )
                .await;
            }))
        }
        _ => None,
    };

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
            let rs_cadence_stats = cadence_stats.clone();
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
                        // Phase 82 — fold each window into the
                        // durable ledger when the substrate is
                        // present (zero-config, like the recall
                        // log itself).
                        helpfulness_ledger:
                            helpfulness_ledger.clone(),
                        // Phase 83 — fold each window's pairs
                        // into the durable co-occurrence
                        // ledger (zero-config, same substrate).
                        cooccurrence_ledger:
                            cooccurrence_ledger.clone(),
                        // Phase 172 — fold each window's per-topic
                        // correction counts into the durable
                        // correction ledger (zero-config, same
                        // substrate).
                        correction_ledger: correction_ledger.clone(),
                        // Phase 93 — per-hit judgment override
                        // when `[recall_feedback].use_judgment_signal
                        // = true`. Absent section → `false`
                        // (byte-identical to pre-Phase-93).
                        use_judgment_signal: recall_feedback_config
                            .as_ref()
                            .map(|c| c.use_judgment_signal)
                            .unwrap_or(false),
                    })
                }
                _ => None,
            };
            // Phase 80 — proactive deps: armed only when the
            // section is enabled AND the substrate is present.
            let rs_proactive = match (
                proactive_config.clone(),
                memory.clone(),
                proactive_log.clone(),
                notify_dispatcher.clone(),
            ) {
                (Some(cfg), Some(mem), Some(plog), Some(nd))
                    if cfg.enabled =>
                {
                    Some(crate::reflection_scheduler::ProactiveDeps {
                        config: cfg,
                        memory: mem,
                        proactive_log: plog,
                        notify: nd,
                        recall_log: recall_log.clone(),
                        memory_ttl_secs,
                        gc_retain_secs:
                            crate::proactive_log::PROACTIVE_LOG_RETAIN_SECS,
                        stat: proactive_stat.clone(),
                    })
                }
                _ => None,
            };
            // Phase 81 — persona-lifecycle deps: armed only
            // when the section is enabled AND the persona
            // substrate (chain + proposal chain + embedding)
            // is present. Any missing piece → None → the pass
            // is skipped while reflection still fires.
            let rs_persona_lifecycle = match (
                persona_lifecycle_config.clone(),
                persona_log.clone(),
                persona_proposal_log.clone(),
                embedding_provider.clone(),
            ) {
                (Some(cfg), Some(plog), Some(pplog), Some(emb))
                    if cfg.enabled =>
                {
                    Some(
                        crate::reflection_scheduler::PersonaLifecycleDeps {
                            config: cfg,
                            persona_log: plog,
                            proposal_log: pplog,
                            embedding: emb,
                            // Phase 85 — gate decay by durable
                            // topic helpfulness when available
                            // (already wired for Phase 82).
                            helpfulness_ledger:
                                helpfulness_ledger.clone(),
                            // Phase 88 — gate decay by durable
                            // pair affinity when available
                            // (already wired for Phase 83);
                            // `None` → pure age-only fallback
                            // for `consolidate-pair:` facets.
                            cooccurrence_ledger:
                                cooccurrence_ledger.clone(),
                            stat: persona_lifecycle_stat.clone(),
                        },
                    )
                }
                _ => None,
            };
            // Phase 87 — pattern-driven Persona consolidation
            // deps: armed only when the section is enabled AND
            // every substrate is present (co-occurrence ledger
            // + helpfulness ledger + proposal chain + an LLM
            // phraser the binary builds with the existing
            // reflection LLM provider). Any missing piece →
            // None → the pass is skipped while reflection
            // still fires (byte-identical to pre-Phase-87).
            let rs_persona_consolidation = match (
                persona_consolidation_config.clone(),
                cooccurrence_ledger.clone(),
                helpfulness_ledger.clone(),
                persona_proposal_log.clone(),
                persona_consolidation_phraser.clone(),
            ) {
                (
                    Some(cfg),
                    Some(cooc),
                    Some(helps),
                    Some(plog),
                    Some(phraser),
                ) if cfg.enabled => Some(
                    crate::reflection_scheduler::PersonaConsolidationDeps {
                        config: cfg,
                        cooccurrence_ledger: cooc,
                        helpfulness_ledger: helps,
                        proposal_log: plog,
                        phraser,
                        stat: persona_consolidation_stat.clone(),
                        // Phase 92 — Persona chain handle +
                        // Phase 88 floor. The binary fills
                        // these so the supersession-detection
                        // branch (gated on
                        // `config.enable_supersession`) can
                        // walk applied `consolidate-pair:`
                        // facets. The bin/aivyx wiring uses
                        // the operator's actual
                        // `[persona_lifecycle].decay_pair_
                        // below_affinity` when present;
                        // None / 1.0 here is the default
                        // (the same default as Phase 88).
                        persona_log: persona_log.clone(),
                        pair_below_affinity: persona_lifecycle_config
                            .as_ref()
                            .map(|c| c.decay_pair_below_affinity)
                            .unwrap_or(
                                aivyx_config::DEFAULT_PL_DECAY_PAIR_BELOW_AFFINITY,
                            ),
                    },
                ),
                _ => None,
            };
            // Phase 172 — correction-consolidation deps: armed
            // only when the section is enabled AND the substrate
            // is present (correction ledger + proposal chain + an
            // LLM `TopicPhraser`). Any missing piece → None → the
            // pass is skipped (the correction ledger still
            // accumulates passively; no proposals are filed).
            let rs_correction_consolidation = match (
                correction_consolidation_config.clone(),
                correction_ledger.clone(),
                persona_proposal_log.clone(),
                correction_consolidation_phraser.clone(),
            ) {
                (Some(cfg), Some(ledger), Some(plog), Some(phraser))
                    if cfg.enabled =>
                {
                    Some(
                        crate::reflection_scheduler::CorrectionConsolidationDeps {
                            config: cfg,
                            correction_ledger: ledger,
                            proposal_log: plog,
                            phraser,
                            stat: correction_consolidation_stat
                                .clone(),
                        },
                    )
                }
                _ => None,
            };
            // Phase 91 — LLM-judged recall deps: armed only
            // when the section is enabled AND every substrate
            // is present (recall log + memory + an
            // `LlmRecallJudge` the binary built with the
            // existing reflection LLM provider). Any missing
            // piece → None → the pass is skipped (the Phase 77
            // structural signal remains the only signal,
            // byte-identical to pre-Phase-91).
            let rs_recall_judgment = match (
                recall_judgment_config.clone(),
                recall_log.clone(),
                memory.clone(),
                recall_judge.clone(),
            ) {
                (Some(cfg), Some(rlog), Some(mem), Some(judge))
                    if cfg.enabled =>
                {
                    Some(
                        crate::reflection_scheduler::RecallJudgmentDeps {
                            config: cfg,
                            recall_log: rlog,
                            memory: mem,
                            judge,
                            stat: recall_judgment_stat.clone(),
                        },
                    )
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
                    rs_proactive,
                    rs_persona_lifecycle,
                    rs_persona_consolidation,
                    rs_correction_consolidation,
                    rs_recall_judgment,
                    rs_cadence_stats,
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
            recall_log: recall_log.clone(),
            helpfulness_ledger: helpfulness_ledger.clone(),
            cooccurrence_ledger: cooccurrence_ledger.clone(),
            correction_ledger: correction_ledger.clone(),
            persona_selection_stat: persona_selection_stat.clone(),
            recall_cluster_stat: recall_cluster_stat.clone(),
            proactive_stat: proactive_stat.clone(),
            persona_lifecycle_stat: persona_lifecycle_stat.clone(),
            conversation_windows: conversation_windows.clone(),
            persona_consolidation_stat:
                persona_consolidation_stat.clone(),
            correction_consolidation_stat:
                correction_consolidation_stat.clone(),
            recall_judgment_stat: recall_judgment_stat.clone(),
            recall_feedback_config: recall_feedback_config.clone(),
            cadence_stats: cadence_stats.clone(),
            tool_descriptors: Arc::clone(&tool_descriptors),
            skill_auto_proposer: skill_auto_proposer.clone(),
            tool_relevance_ledger: tool_relevance_ledger.clone(),
            loop_backlog: loop_backlog.clone(),
            loop_state: loop_state.clone(),
            loop_config: loop_config.clone(),
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
    /// Phase 78 — recall-feedback log for the read-only
    /// `GetLearningInsights` query. `None` = no auto-recall
    /// configured (the query returns an empty digest).
    recall_log: Option<Arc<crate::recall_log::PersistentRecallLog>>,
    /// Phase 82 — durable helpfulness ledger for the read-only
    /// `GetLearningInsights` longitudinal view. `None` = no
    /// auto-recall configured (no accumulated view).
    helpfulness_ledger: Option<
        Arc<crate::helpfulness_ledger::PersistentHelpfulnessLedger>,
    >,
    /// Phase 83 — durable co-occurrence ledger for the
    /// read-only `GetLearningInsights` cross-session pattern
    /// view. `None` = no auto-recall configured.
    cooccurrence_ledger: Option<
        Arc<
            crate::cooccurrence_ledger::PersistentCooccurrenceLedger,
        >,
    >,
    /// Phase 172 — durable correction ledger for the read-only
    /// `GetLearningInsights` accumulated-corrections view.
    /// `None` = no auto-recall configured.
    correction_ledger: Option<
        Arc<crate::correction_ledger::PersistentCorrectionLedger>,
    >,
    /// Phase 79 (Q4a) — last-Persona-selection stat for the
    /// `GetLearningInsights` surface.
    persona_selection_stat:
        Option<crate::persona_context::SharedPersonaSelectionStat>,
    /// Phase 84 (Q4a) — last-turn cluster-recall stat for the
    /// `GetLearningInsights` surface.
    recall_cluster_stat:
        Option<crate::memory_recall::SharedRecallClusterStat>,
    /// Phase 80 (Q4a) — last-proactive-cycle stat for the
    /// `GetLearningInsights` surface.
    proactive_stat:
        Option<crate::proactive_detect::SharedProactiveStat>,
    /// Phase 81 (Q4a) — last-persona-lifecycle-cycle stat for
    /// the `GetLearningInsights` surface.
    persona_lifecycle_stat: Option<
        crate::persona_lifecycle::SharedPersonaLifecycleStat,
    >,
    /// Phase 86 — per-session conversation windows. `Some` →
    /// the turn loop appends `(user, assistant)` pairs on
    /// `TurnOutcome::Completed` so both relevance providers can
    /// embed a multi-turn query.
    conversation_windows:
        Option<crate::conversation_window::SharedConversationWindows>,
    /// Phase 87 (Q4a) — last-reflection-cycle pattern-driven
    /// Persona consolidation stat for the
    /// `GetLearningInsights` surface.
    persona_consolidation_stat: Option<
        crate::persona_consolidation::SharedPersonaConsolidationStat,
    >,
    /// Phase 172 (Q4a) — last-reflection-cycle correction-driven
    /// consolidation stat for the `GetLearningInsights` surface.
    correction_consolidation_stat: Option<
        crate::correction_consolidation::SharedCorrectionConsolidationStat,
    >,
    /// Phase 91 (Q4a) — last-reflection-cycle LLM-judged
    /// recall stat for the `GetLearningInsights` surface.
    recall_judgment_stat: Option<
        crate::recall_judgment::SharedRecallJudgmentStat,
    >,
    /// Phase 93 — `[recall_feedback]` config for the
    /// `GetLearningInsights` surface so the insights view
    /// reflects the same per-hit judgment override that the
    /// reflection-cron actuator is using.
    recall_feedback_config: Option<aivyx_config::RecallFeedbackConfig>,
    /// Phase 95 — per-schedule cadence stats (fired /
    /// skipped counts) the `GetLearningInsights` surface
    /// reads to render the cadence section.
    cadence_stats: crate::reflection_scheduler::SharedRecentReflectionStats,
    /// Phase 102 — registered-tool snapshot for the `GetToolStats`
    /// query. `Arc`-shared so each per-connection context is a
    /// cheap pointer clone.
    tool_descriptors: Arc<[ToolDescriptor]>,
    /// Phase 112 — Skill Auto-Proposer dependency bundle.
    /// `None` disables the post-turn auto-proposer spawn.
    skill_auto_proposer:
        Option<Arc<crate::skill_auto_proposer::SkillAutoProposerContext>>,
    /// Phase 116 — tool/skill relevance ledger handle.
    /// `None` disables the recording hook + prompt section.
    tool_relevance_ledger:
        Option<Arc<crate::tool_relevance_ledger::PersistentToolRelevanceLedger>>,
    /// Phase 173 — the autonomous-loop backlog (always `Some`
    /// when storage is configured) for the `loop add/list/status`
    /// IPC handlers.
    loop_backlog:
        Option<Arc<crate::loop_backlog::PersistentLoopBacklog>>,
    /// Phase 173 — shared loop run state for `loop start/stop/
    /// status`. `Some` only when the `[loop]` section is armed
    /// (the driver was spawned).
    loop_state: Option<crate::loop_driver::SharedLoopState>,
    /// Phase 173 — the `[loop]` config (default priority +
    /// max-iterations ceiling) for the IPC handlers.
    loop_config: Option<aivyx_config::LoopConfig>,
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
        recall_log,
        helpfulness_ledger,
        cooccurrence_ledger,
        correction_ledger,
        persona_selection_stat,
        recall_cluster_stat,
        proactive_stat,
        persona_lifecycle_stat,
        conversation_windows,
        persona_consolidation_stat,
        correction_consolidation_stat,
        recall_judgment_stat,
        recall_feedback_config,
        cadence_stats,
        tool_descriptors,
        skill_auto_proposer,
        tool_relevance_ledger,
        loop_backlog,
        loop_state,
        loop_config,
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
                            // Phase 86 — the message's `session_id` must
                            // be stable across turns of the same daemon
                            // session (sid was generated by
                            // `SessionId::new().to_string()` at
                            // StartSession). Parse it back so the
                            // per-session conversation window + the
                            // Phase 77 recall correlation key by the
                            // session the operator actually has, not a
                            // fresh-per-turn surrogate.
                            let session = sid
                                .parse::<uuid::Uuid>()
                                .map(aivyx_core::SessionId)
                                .unwrap_or_else(|_| aivyx_core::SessionId::new());
                            // Phase 86 — keep the user text for the
                            // conversation-window write site below
                            // (Message::text consumes it).
                            let user_text = text.clone();
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

                            // Audit H1 fix — rotate the channel stub's
                            // cancellation token so a previous turn's
                            // timeout or `/cancel` does not pre-cancel
                            // this turn. `CancellationToken` is
                            // monotonic; without this reset, the first
                            // timeout/cancel in a daemon session would
                            // brick every subsequent turn until the
                            // operator restarted the connection.
                            bridge.reset_cancellation();

                            // Phase 116 — capture the audit chain's
                            // pre-turn length so the post-finalize hook
                            // can read the turn's per-tool-call entries
                            // (via `entries_range(pre_len, len -
                            // pre_len)`) without locking.
                            let audit_pre_turn_len = audit_log
                                .as_ref()
                                .map(|l| l.len());

                            let outcome = agent.turn(msg, &bridge).await;

                            // Turn completed — remove from in-flight.
                            if let Ok(mut st) = daemon_state.lock() {
                                st.in_flight_turns.retain(|t| t != &turn_key);
                            }

                            // Phase 86 — record the completed turn
                            // (user input → assistant final) into the
                            // per-session conversation window so the
                            // next turn's recall + Persona selection
                            // can embed a multi-turn relevance query.
                            // Only conversational completions are
                            // recorded; trigger-fired synthetic turns
                            // (cron / webhook / file-watch) write
                            // through their own paths and are
                            // intentionally excluded. Best-effort —
                            // a missed write costs one cycle of
                            // signal, never the turn.
                            if let (
                                Some(windows),
                                TurnOutcome::Completed { final_message, .. },
                            ) = (&conversation_windows, &outcome)
                            {
                                crate::conversation_window::record_turn(
                                    windows,
                                    session,
                                    &user_text,
                                    final_message,
                                );
                            }

                            // Phase 116 — tool-relevance ledger post-
                            // finalize hook. Fires for every turn (any
                            // TurnOutcome variant) when the ledger is
                            // configured; walks the audit chain from
                            // the pre-turn snapshot to the current head
                            // and records each ToolCall's outcome
                            // against the user input's keyword key.
                            // Detached `tokio::spawn` so it never
                            // blocks the next turn. Failure-isolated.
                            if let (Some(ledger), Some(pre_len), Some(audit)) = (
                                &tool_relevance_ledger,
                                audit_pre_turn_len,
                                &audit_log,
                            ) {
                                let keyword_key =
                                    aivyx_core::relevance::keyword_key(
                                        &user_text, 5,
                                    );
                                if !keyword_key.is_empty() {
                                    let ledger_clone = Arc::clone(ledger);
                                    let audit_clone = Arc::clone(audit);
                                    tokio::spawn(async move {
                                        let head = audit_clone.len();
                                        let limit = head.saturating_sub(pre_len);
                                        if limit == 0 {
                                            return;
                                        }
                                        let entries = match audit_clone
                                            .entries_range(pre_len as u64, limit)
                                        {
                                            Ok(e) => e,
                                            Err(e) => {
                                                eprintln!(
                                                    "aivyx tool-relevance: \
                                                     audit walk failed ({e})"
                                                );
                                                return;
                                            }
                                        };
                                        let now_ms = std::time::SystemTime::now()
                                            .duration_since(
                                                std::time::UNIX_EPOCH,
                                            )
                                            .map(|d| d.as_millis() as u64)
                                            .unwrap_or(0);
                                        crate::tool_relevance_ledger::record_turn_outcomes(
                                            &ledger_clone,
                                            &keyword_key,
                                            &entries,
                                            now_ms,
                                        )
                                        .await;
                                    });
                                }
                            }

                            // Phase 112 + 115 — auto-proposer post-finalize
                            // hook. Q2b inline-at-turn-boundary firing; the
                            // pipeline runs in a detached `tokio::spawn` so
                            // it never blocks the next turn.
                            //
                            // Phase 112-114: fires only on
                            // `TurnOutcome::Completed` (positive-pattern path).
                            // Phase 115: also fires on Failed / Cancelled /
                            // TimedOut / Escalated when the operator has
                            // `from_failed_turns = true` in their TOML and the
                            // specific failure outcome is enabled in
                            // `failure_outcomes`. The negative-feedback
                            // (failure correction) path passes
                            // `ProposalSource::FailedTurn { .. }` to the
                            // pipeline.
                            if let Some(proposer_ctx) = &skill_auto_proposer {
                                use crate::skill_auto_proposer::{
                                    self as sap, FailureKind, ProposalSource,
                                };

                                // Classify the outcome into (signals, source).
                                let dispatch: Option<(
                                    sap::TurnSignals,
                                    ProposalSource,
                                )> = match &outcome {
                                    TurnOutcome::Completed {
                                        tool_calls_made,
                                        duration,
                                        ..
                                    } => Some((
                                        sap::TurnSignals {
                                            tool_calls_made: *tool_calls_made as u32,
                                            distinct_tool_id_count:
                                                (*tool_calls_made as u32).min(4),
                                            duration: *duration,
                                            had_successful_gate_resolve: false,
                                            // Phase 118 — Task 3 ships the
                                            // type substrate; the actual
                                            // ledger/audit-walk sourcing
                                            // for these signals is wired in
                                            // Task 6. Default-zero until
                                            // then.
                                            ..sap::TurnSignals::default()
                                        },
                                        ProposalSource::CompletedTurn,
                                    )),
                                    other => {
                                        // Phase 115 — non-Completed outcomes
                                        // gate on the operator's
                                        // from_failed_turns + failure_outcomes
                                        // config.
                                        let cfg = &proposer_ctx.config;
                                        if !cfg.from_failed_turns {
                                            None
                                        } else {
                                            let kind = match other {
                                                TurnOutcome::Failed(_)
                                                | TurnOutcome::MaxStepsExceeded { .. } =>
                                                    FailureKind::Failed,
                                                TurnOutcome::Cancelled { .. } =>
                                                    FailureKind::Cancelled,
                                                TurnOutcome::TimedOut { .. } =>
                                                    FailureKind::TimedOut,
                                                TurnOutcome::Escalated { .. } =>
                                                    FailureKind::Escalated,
                                                TurnOutcome::Completed { .. } =>
                                                    unreachable!(),
                                            };
                                            if !sap::is_failure_candidate(
                                                kind,
                                                &cfg.failure_outcomes,
                                            ) {
                                                None
                                            } else {
                                                // For failure paths the
                                                // signals are degenerate; the
                                                // failure heuristic + judge
                                                // are the real gates.
                                                // Phase 118 — degenerate
                                                // signals for the failure
                                                // path; the new Profile/Role
                                                // signals default to zero.
                                                let signals = sap::TurnSignals::default();
                                                let summary = match other {
                                                    TurnOutcome::Failed(e) =>
                                                        format!("planner/agent error: {e}"),
                                                    TurnOutcome::Cancelled { .. } =>
                                                        "operator cancelled mid-turn".into(),
                                                    TurnOutcome::TimedOut {
                                                        elapsed, ..
                                                    } => format!(
                                                        "exceeded turn budget after {}ms",
                                                        elapsed.as_millis()
                                                    ),
                                                    TurnOutcome::Escalated {
                                                        reason, ..
                                                    } => format!(
                                                        "agent escalated: {reason}"
                                                    ),
                                                    _ => unreachable!(),
                                                };
                                                Some((
                                                    signals,
                                                    ProposalSource::FailedTurn {
                                                        kind,
                                                        summary,
                                                    },
                                                ))
                                            }
                                        }
                                    }
                                };

                                if let Some((signals, source)) = dispatch {
                                    let summary =
                                        sap::build_turn_summary(&user_text, &outcome);
                                    let proposer_ctx = Arc::clone(proposer_ctx);
                                    let audit_clone = audit_log.clone();
                                    let persona_clone = persona_log.clone();
                                    let proposal_clone =
                                        persona_proposal_log.clone();
                                    let shared_clone = shared_persona.clone();
                                    let cancel = shutdown.clone();
                                    tokio::spawn(async move {
                                        sap::run_auto_propose_pipeline_with_source(
                                            &proposer_ctx,
                                            audit_clone.as_ref(),
                                            persona_clone.as_ref(),
                                            proposal_clone.as_ref(),
                                            &shared_clone,
                                            session,
                                            signals,
                                            summary,
                                            source,
                                            &cancel,
                                        )
                                        .await;
                                    });
                                }
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
                            // Audit C1 fix — fire the channel stub's in-flight
                            // cancellation token. Before this fix, the handler
                            // was a no-op (the comment claimed the wiring was
                            // present, but no code actually called `.cancel()`
                            // on anything). The four daemon-side stubs
                            // (Telegram/Discord/Slack/Web) override
                            // `cancel_inflight` to fire their internal token;
                            // any non-daemon channel uses the default no-op.
                            // The turn loop's mid-LLM-step cancellation check
                            // then translates the cancel into
                            // `TurnOutcome::Cancelled`.
                            if let Some(ch) = &channel {
                                ch.cancel_inflight();
                            }
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

                                            // Audit H1 fix — rotate before
                                            // resuming so a prior cancel does
                                            // not pre-cancel the resume turn.
                                            bridge.reset_cancellation();

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
                                recall_log.as_ref(),
                                helpfulness_ledger.as_ref(),
                                cooccurrence_ledger.as_ref(),
                                correction_ledger.as_ref(),
                                persona_selection_stat.as_ref(),
                                recall_cluster_stat.as_ref(),
                                proactive_stat.as_ref(),
                                persona_lifecycle_stat.as_ref(),
                                persona_consolidation_stat.as_ref(),
                                correction_consolidation_stat.as_ref(),
                                recall_judgment_stat.as_ref(),
                                recall_feedback_config.as_ref(),
                                &cadence_stats,
                                &tool_descriptors,
                                tool_relevance_ledger.as_ref(),
                                loop_backlog.as_ref(),
                                loop_state.as_ref(),
                                loop_config.as_ref(),
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
                        FrontendMessage::ApplyProfileHint {
                            id,
                            proposal_id,
                            field,
                            applied_value,
                        } => {
                            // Phase 119 — operator's act-on-approval
                            // gesture for a ProfileHint. The CLI has
                            // already mutated aivyx.toml via the Task 3
                            // atomic primitive; this handler's only
                            // job is to record the audit event so
                            // forensic walks can pair the apply with
                            // the upstream proposal.
                            let resp = match audit_log.as_ref() {
                                None => DaemonMessage::ProfileHintApplyAcked {
                                    id,
                                    ok: false,
                                    error: Some(
                                        "daemon has no audit log configured"
                                            .into(),
                                    ),
                                },
                                Some(al) => {
                                    let event = aivyx_audit::AuditEvent::ProfileHintApplied {
                                        session_id: aivyx_core::SessionId::new(),
                                        proposal_id,
                                        field,
                                        applied_value,
                                    };
                                    match al.append(event) {
                                        Ok(_) => DaemonMessage::ProfileHintApplyAcked {
                                            id,
                                            ok: true,
                                            error: None,
                                        },
                                        Err(e) => DaemonMessage::ProfileHintApplyAcked {
                                            id,
                                            ok: false,
                                            error: Some(e.to_string()),
                                        },
                                    }
                                }
                            };
                            let frame = encode_frame(&resp)?;
                            writer.write_all(&frame).await?;
                        }
                        FrontendMessage::ImportRoleDraft {
                            id,
                            proposal_id,
                            role_name,
                            parent,
                        } => {
                            // Phase 119 — operator's act-on-approval
                            // gesture for a RoleDefinitionSuggestion.
                            // Same shape as ApplyProfileHint.
                            let resp = match audit_log.as_ref() {
                                None => DaemonMessage::RoleDraftImportAcked {
                                    id,
                                    ok: false,
                                    error: Some(
                                        "daemon has no audit log configured"
                                            .into(),
                                    ),
                                },
                                Some(al) => {
                                    let event = aivyx_audit::AuditEvent::RoleDraftImported {
                                        session_id: aivyx_core::SessionId::new(),
                                        proposal_id,
                                        role_name,
                                        parent,
                                    };
                                    match al.append(event) {
                                        Ok(_) => DaemonMessage::RoleDraftImportAcked {
                                            id,
                                            ok: true,
                                            error: None,
                                        },
                                        Err(e) => DaemonMessage::RoleDraftImportAcked {
                                            id,
                                            ok: false,
                                            error: Some(e.to_string()),
                                        },
                                    }
                                }
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
        recall_log: None,
        helpfulness_ledger: None,
        cooccurrence_ledger: None,
        correction_ledger: None,
        persona_selection_stat: None,
        recall_cluster_stat: None,
        proactive_stat: None,
        persona_lifecycle_stat: None,
        conversation_windows: None,
        persona_consolidation_stat: None,
        correction_consolidation_stat: None,
        recall_judgment_stat: None,
        recall_feedback_config: None,
        cadence_stats: crate::reflection_scheduler::shared_recent_reflection_stats(),
        tool_descriptors: Arc::from(Vec::<ToolDescriptor>::new()),
        skill_auto_proposer: None,
        tool_relevance_ledger: None,
        loop_backlog: None,
        loop_state: None,
        loop_config: None,
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
        helpfulness_ledger: None,
        cooccurrence_ledger: None,
        correction_ledger: None,
        persona_selection_stat: None,
        recall_cluster_stat: None,
        proactive_config: None,
        proactive_log: None,
        proactive_stat: None,
        persona_lifecycle_config: None,
        persona_lifecycle_stat: None,
        memory_retention: Vec::new(),
        conversation_windows: None,
        persona_consolidation_config: None,
        persona_consolidation_stat: None,
        persona_consolidation_phraser: None,
        correction_consolidation_config: None,
        correction_consolidation_stat: None,
        correction_consolidation_phraser: None,
        recall_judgment_config: None,
        recall_judgment_stat: None,
        recall_judge: None,
        recall_feedback_config: None,
        tool_descriptors: Vec::new(),
        skill_auto_proposer: None,
        tool_relevance_ledger: None,
        loop_backlog: None,
        loop_state: None,
        loop_config: None,
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
        TurnOutcome::MaxStepsExceeded { max_steps, .. } => {
            format!("aborted: planner exceeded {max_steps} steps")
        }
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
    recall_log: Option<&Arc<crate::recall_log::PersistentRecallLog>>,
    helpfulness_ledger: Option<
        &Arc<crate::helpfulness_ledger::PersistentHelpfulnessLedger>,
    >,
    cooccurrence_ledger: Option<
        &Arc<
            crate::cooccurrence_ledger::PersistentCooccurrenceLedger,
        >,
    >,
    correction_ledger: Option<
        &Arc<crate::correction_ledger::PersistentCorrectionLedger>,
    >,
    persona_selection_stat: Option<
        &crate::persona_context::SharedPersonaSelectionStat,
    >,
    recall_cluster_stat: Option<
        &crate::memory_recall::SharedRecallClusterStat,
    >,
    proactive_stat: Option<
        &crate::proactive_detect::SharedProactiveStat,
    >,
    persona_lifecycle_stat: Option<
        &crate::persona_lifecycle::SharedPersonaLifecycleStat,
    >,
    persona_consolidation_stat: Option<
        &crate::persona_consolidation::SharedPersonaConsolidationStat,
    >,
    correction_consolidation_stat: Option<
        &crate::correction_consolidation::SharedCorrectionConsolidationStat,
    >,
    recall_judgment_stat: Option<
        &crate::recall_judgment::SharedRecallJudgmentStat,
    >,
    recall_feedback_config: Option<&aivyx_config::RecallFeedbackConfig>,
    cadence_stats: &crate::reflection_scheduler::SharedRecentReflectionStats,
    tool_descriptors: &[ToolDescriptor],
    tool_relevance_ledger: Option<
        &Arc<crate::tool_relevance_ledger::PersistentToolRelevanceLedger>,
    >,
    loop_backlog: Option<&Arc<crate::loop_backlog::PersistentLoopBacklog>>,
    loop_state: Option<&crate::loop_driver::SharedLoopState>,
    loop_config: Option<&aivyx_config::LoopConfig>,
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
        QueryPayload::GetToolStats { window_secs } => {
            let Some(log) = audit_log else {
                return QueryResponsePayload::QueryError {
                    code: "no_audit_log".into(),
                    message: "daemon has no audit log configured".into(),
                };
            };
            // `window_secs` → an absolute cutoff; `None` = whole
            // chain. A clock that cannot subtract `secs` (absurdly
            // large window) just yields `None` → whole chain.
            let cutoff = window_secs.and_then(|secs| {
                std::time::SystemTime::now()
                    .checked_sub(std::time::Duration::from_secs(secs))
            });
            // The whole chain is loaded — an observability query,
            // not a hot path, and the chain is bounded (Phase 53).
            match log.entries_range(0, log.len()) {
                Ok(rows) => QueryResponsePayload::ToolStats {
                    tools: fold_tool_stats(&rows, cutoff, tool_descriptors),
                },
                Err(e) => QueryResponsePayload::QueryError {
                    code: "tool_stats_failed".into(),
                    message: e.to_string(),
                },
            }
        }
        QueryPayload::DumpToolRelevance { keyword_key_filter } => {
            let Some(ledger) = tool_relevance_ledger else {
                return QueryResponsePayload::QueryError {
                    code: "no_tool_relevance_ledger".into(),
                    message:
                        "daemon has no tool-relevance ledger configured \
                         (enable `[tool_relevance]` in aivyx.toml)"
                            .into(),
                };
            };
            let entries = match ledger
                .list_all_entries(keyword_key_filter.as_deref())
                .await
            {
                Ok(e) => e,
                Err(e) => {
                    return QueryResponsePayload::QueryError {
                        code: "tool_relevance_dump_failed".into(),
                        message: e.to_string(),
                    };
                }
            };
            let mut rows: Vec<crate::daemon_ipc::ToolRelevanceDumpRow> = Vec::new();
            for (keyword_key, entry) in entries {
                for row in entry.outcomes {
                    rows.push(crate::daemon_ipc::ToolRelevanceDumpRow {
                        keyword_key: keyword_key.clone(),
                        surface_kind: row.surface_kind.label().to_string(),
                        identifier: row.identifier,
                        success_count: row.success_count,
                        failure_count: row.failure_count,
                        last_seen_unix_ms: row.last_seen_unix_ms,
                    });
                }
            }
            // Stable column ordering for the operator-facing table.
            rows.sort_by(|a, b| {
                (a.keyword_key.as_str(), a.surface_kind.as_str(), a.identifier.as_str())
                    .cmp(&(
                        b.keyword_key.as_str(),
                        b.surface_kind.as_str(),
                        b.identifier.as_str(),
                    ))
            });
            QueryResponsePayload::ToolRelevanceDump { rows }
        }
        // ---- Phase 173 — autonomous loop control ---------------
        QueryPayload::LoopAdd {
            title,
            body,
            priority,
        } => {
            let Some(backlog) = loop_backlog else {
                return QueryResponsePayload::QueryError {
                    code: "no_loop_backlog".into(),
                    message: "daemon has no loop backlog configured".into(),
                };
            };
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let priority = priority.unwrap_or_else(|| {
                loop_config
                    .map(|c| c.default_priority)
                    .unwrap_or(aivyx_config::DEFAULT_LOOP_PRIORITY)
            });
            let story_id =
                format!("ls-{}", uuid::Uuid::new_v4().as_simple());
            match backlog
                .add_story(story_id.clone(), now_ms, priority, title, body)
                .await
            {
                Ok(_) => QueryResponsePayload::LoopStoryAdded { story_id },
                Err(e) => QueryResponsePayload::QueryError {
                    code: "loop_add_failed".into(),
                    message: e.to_string(),
                },
            }
        }
        QueryPayload::LoopList => {
            let Some(backlog) = loop_backlog else {
                return QueryResponsePayload::QueryError {
                    code: "no_loop_backlog".into(),
                    message: "daemon has no loop backlog configured".into(),
                };
            };
            QueryResponsePayload::LoopBacklog {
                stories: backlog
                    .list(crate::loop_backlog::StoryStatusFilter::All),
            }
        }
        QueryPayload::LoopStart { max_iterations } => {
            let (Some(state), Some(cfg)) = (loop_state, loop_config) else {
                return QueryResponsePayload::LoopControl {
                    ok: false,
                    message: "the [loop] section is not armed (set \
                              `[loop] enabled = true` in aivyx.toml and \
                              restart the daemon)"
                        .into(),
                };
            };
            // The configured cap is the ceiling; a per-run request
            // may only lower it.
            let requested = max_iterations
                .unwrap_or(cfg.max_iterations)
                .min(cfg.max_iterations)
                .max(1);
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            if state.request_start(requested, now_ms) {
                QueryResponsePayload::LoopControl {
                    ok: true,
                    message: format!(
                        "loop run started (max_iterations={requested})"
                    ),
                }
            } else {
                QueryResponsePayload::LoopControl {
                    ok: false,
                    message: "a loop run is already active".into(),
                }
            }
        }
        QueryPayload::LoopStop => {
            let Some(state) = loop_state else {
                return QueryResponsePayload::LoopControl {
                    ok: false,
                    message: "the [loop] section is not armed".into(),
                };
            };
            if state.request_stop() {
                QueryResponsePayload::LoopControl {
                    ok: true,
                    message: "loop run stopping (after the current \
                              iteration)"
                        .into(),
                }
            } else {
                QueryResponsePayload::LoopControl {
                    ok: false,
                    message: "no loop run is active".into(),
                }
            }
        }
        QueryPayload::LoopStatus => {
            let remaining = loop_backlog
                .map(|b| b.remaining_count())
                .unwrap_or(0);
            let state = loop_state
                .map(|s| s.snapshot())
                .unwrap_or_default();
            QueryResponsePayload::LoopStatus {
                state,
                remaining,
                armed: loop_state.is_some(),
                gate_enabled: loop_config
                    .map(|c| c.gate_command.is_some())
                    .unwrap_or(false),
                max_run_secs: loop_config.and_then(|c| c.max_run_secs),
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
        QueryPayload::GetLearningInsights { window_secs } => {
            let window = window_secs.unwrap_or(
                crate::recall_feedback::RECALL_LOG_RETAIN_SECS,
            );
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            let now_secs = now_ms / 1000;

            // Phase 79 (Q4a) — last adaptive-Persona selection.
            // Independent of the recall substrate, so resolved
            // once and included in every LearningInsights return.
            let persona_selection = persona_selection_stat
                .and_then(|s| s.read().ok().and_then(|g| g.clone()));
            // Phase 84 (Q4a) — last-turn cluster-recall stat
            // (same shared-handle pattern as persona_selection).
            let cluster_recall = recall_cluster_stat
                .and_then(|s| s.read().ok().and_then(|g| g.clone()));
            let proactive = proactive_stat
                .and_then(|s| s.read().ok().and_then(|g| g.clone()));
            let persona_lifecycle = persona_lifecycle_stat
                .and_then(|s| s.read().ok().and_then(|g| g.clone()));
            // Phase 87 (Q4a) — last reflection cycle's
            // pattern-driven consolidation outcome (same
            // shared-handle pattern as persona_selection /
            // cluster_recall).
            let persona_consolidation = persona_consolidation_stat
                .and_then(|s| s.read().ok().and_then(|g| g.clone()));
            // Phase 91 (Q4a) — last reflection cycle's
            // LLM-judged recall outcome.
            let recall_judgment = recall_judgment_stat
                .and_then(|s| s.read().ok().and_then(|g| g.clone()));
            // Phase 82 — durable accumulated helpfulness (the
            // longitudinal view). Best-effort: a ledger error
            // collapses to `None`, never breaking the surface;
            // an empty ledger is reported as "none yet."
            let accumulated_helpfulness =
                match helpfulness_ledger {
                    Some(l) => l
                        .accumulated(now_secs, 5)
                        .await
                        .ok()
                        .filter(|a| {
                            !a.top_helpful.is_empty()
                                || !a.top_unhelpful.is_empty()
                        }),
                    None => None,
                };
            // Phase 83 — durable cross-session co-occurrence
            // patterns. Best-effort: ledger error → None,
            // empty → None (never breaks the surface).
            let cooccurrence = match cooccurrence_ledger {
                Some(l) => l
                    .top_affinities(now_secs, 5)
                    .await
                    .ok()
                    .filter(|p| !p.top_pairs.is_empty()),
                None => None,
            };
            // Phase 172 — durable accumulated correction view
            // (the topics the operator most often reworks).
            // Best-effort: ledger error → None, empty → None.
            let accumulated_corrections = match correction_ledger {
                Some(l) => l
                    .accumulated(now_secs, 5)
                    .await
                    .ok()
                    .filter(|a| !a.top_corrected.is_empty()),
                None => None,
            };
            // Phase 172 (Q4a) — last reflection cycle's
            // correction-driven consolidation outcome.
            let correction_consolidation =
                correction_consolidation_stat.and_then(|s| {
                    s.read().ok().and_then(|g| g.clone())
                });

            // Phase 95 — snapshot per-schedule cadence stats.
            // Sorted by schedule name for stable rendering.
            let cadence: Vec<(
                String,
                crate::reflection_scheduler::RecentReflectionStat,
            )> = cadence_stats
                .read()
                .ok()
                .map(|g| {
                    let mut v: Vec<_> = g
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    v.sort_by(|a, b| a.0.cmp(&b.0));
                    v
                })
                .unwrap_or_default();

            // No recall substrate → an empty digest is the
            // valid "nothing learned yet" answer, not an error.
            let Some(rlog) = recall_log else {
                return QueryResponsePayload::LearningInsights {
                    digest: crate::recall_insights::build_digest(
                        window,
                        &crate::recall_feedback::HelpfulnessTally::default(),
                        &[],
                        &[],
                        recall_feedback_config
                            .map(|c| c.use_judgment_signal),
                    ),
                    proposals: Vec::new(),
                    persona_selection,
                    proactive,
                    persona_lifecycle,
                    accumulated_helpfulness,
                    cooccurrence,
                    cluster_recall,
                    persona_consolidation,
                    accumulated_corrections,
                    correction_consolidation,
                    recall_judgment,
                    cadence,
                };
            };

            let since = now_secs.saturating_sub(window);
            let recalls = match rlog.events_since(since).await {
                Ok(r) => r,
                Err(e) => {
                    return QueryResponsePayload::QueryError {
                        code: "recall_log_read_failed".into(),
                        message: e.to_string(),
                    };
                }
            };
            // Outcomes from the audit chain (same builder the
            // reflection loop uses). No audit log → no scored
            // recalls (digest still reports the raw recall
            // count).
            let outcomes = match audit_log {
                Some(al) => {
                    let n = al.len();
                    match al.entries_range(0, n) {
                        Ok(es) => {
                            crate::reflection_scheduler::summarize_recent_outcomes_from_entries(
                                &es, window, now_ms,
                            )
                        }
                        Err(e) => {
                            return QueryResponsePayload::QueryError {
                                code: "audit_read_failed".into(),
                                message: format!(
                                    "audit chain read failed: {e}"
                                ),
                            };
                        }
                    }
                }
                None => Vec::new(),
            };
            let (tally, detail) =
                crate::recall_feedback::correlate_detailed(
                    &recalls,
                    &outcomes,
                    recall_feedback_config
                        .map(|c| c.use_judgment_signal)
                        .unwrap_or(false),
                );
            let proposals = persona_proposal_log
                .map(|l| {
                    l.list(
                        crate::persona_proposal::ProposalStatusFilter::All,
                    )
                })
                .unwrap_or_default();
            QueryResponsePayload::LearningInsights {
                digest: crate::recall_insights::build_digest(
                    window,
                    &tally,
                    &detail,
                    &proposals,
                    recall_feedback_config
                        .map(|c| c.use_judgment_signal),
                ),
                proposals: crate::recall_insights::build_provenance(
                    &detail, &proposals,
                ),
                persona_selection,
                proactive,
                persona_lifecycle,
                accumulated_helpfulness,
                cooccurrence,
                cluster_recall,
                persona_consolidation,
                accumulated_corrections,
                correction_consolidation,
                recall_judgment,
                cadence,
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
    // Phase 92 → Phase 94 — lift the linkage onto the
    // summary so the surface grouping helper doesn't need
    // to re-parse `proposed_op` JSON.
    let supersedes_proposal_id =
        view.proposed_op.supersedes_proposal_id.clone();
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
        supersedes_proposal_id,
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
        aivyx_audit::AuditEvent::SkillAutoProposal { .. } => "SkillAutoProposal",
        aivyx_audit::AuditEvent::SkillInvocation { .. } => "SkillInvocation",
        aivyx_audit::AuditEvent::ProfileHintApplied { .. } => "ProfileHintApplied",
        aivyx_audit::AuditEvent::RoleDraftImported { .. } => "RoleDraftImported",
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

/// Phase 102 — fold a slice of audit `SignedEntry`s into per-tool
/// statistics for the `GetToolStats` query, joined against the
/// registered tool set.
///
/// `ToolCall` events are keyed by `scope_used.base()` — the stable,
/// human-meaningful capability base (`fs.read`, `net.fetch`),
/// unlike the per-process `tool_id`. Entries appended before
/// `cutoff` (when set) are skipped. Every registered tool yields a
/// row (zero stats if never called); a base with call history but
/// no currently registered tool yields a `registered: false` row.
/// Rows are ordered by call count descending, then name ascending.
fn fold_tool_stats(
    entries: &[aivyx_audit::SignedEntry],
    cutoff: Option<std::time::SystemTime>,
    tool_descriptors: &[ToolDescriptor],
) -> Vec<crate::daemon_ipc::ToolStat> {
    use std::collections::{BTreeMap, BTreeSet};

    #[derive(Default)]
    struct Acc {
        calls: u64,
        outcomes: BTreeMap<String, u64>,
        total_duration_ms: u64,
    }
    let mut acc: BTreeMap<String, Acc> = BTreeMap::new();

    for entry in entries {
        if let Some(cut) = cutoff {
            if entry.appended_at < cut {
                continue;
            }
        }
        let aivyx_audit::AuditEvent::ToolCall {
            scope_used,
            outcome,
            duration,
            ..
        } = &entry.event
        else {
            continue;
        };
        let a = acc.entry(scope_used.base().to_string()).or_default();
        a.calls += 1;
        a.total_duration_ms += duration.as_millis() as u64;
        let label = match outcome {
            aivyx_core::ToolOutcomeSummary::Completed { .. } => "completed",
            aivyx_core::ToolOutcomeSummary::Denied => "denied",
            aivyx_core::ToolOutcomeSummary::NotInRole => "not_in_role",
            aivyx_core::ToolOutcomeSummary::RequiresEscalation => {
                "requires_escalation"
            }
            aivyx_core::ToolOutcomeSummary::Failed => "failed",
        };
        *a.outcomes.entry(label.to_string()).or_insert(0) += 1;
    }

    let mut tools: Vec<crate::daemon_ipc::ToolStat> = Vec::new();
    let mut listed: BTreeSet<String> = BTreeSet::new();
    for desc in tool_descriptors {
        listed.insert(desc.scope_base.clone());
        let a = acc.get(&desc.scope_base);
        tools.push(crate::daemon_ipc::ToolStat {
            name: desc.name.clone(),
            description: desc.description.clone(),
            scope_base: desc.scope_base.clone(),
            registered: true,
            calls: a.map_or(0, |x| x.calls),
            outcomes: a.map(|x| x.outcomes.clone()).unwrap_or_default(),
            total_duration_ms: a.map_or(0, |x| x.total_duration_ms),
        });
    }
    // Bases with audit history but no currently registered tool.
    for (base, a) in &acc {
        if listed.contains(base) {
            continue;
        }
        tools.push(crate::daemon_ipc::ToolStat {
            name: base.clone(),
            description: "(no registered tool)".to_string(),
            scope_base: base.clone(),
            registered: false,
            calls: a.calls,
            outcomes: a.outcomes.clone(),
            total_duration_ms: a.total_duration_ms,
        });
    }
    tools.sort_by(|x, y| y.calls.cmp(&x.calls).then_with(|| x.name.cmp(&y.name)));
    tools
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

    fn reset_cancellation(&self) {
        // Audit H1 fix — the daemon calls this between turns
        // to rotate the channel stub's token. Forwards to the
        // underlying daemon stub (Telegram/Discord/Slack/Web)
        // which holds the actual `Mutex<CancellationToken>`.
        self.inner.reset_cancellation();
    }

    fn cancel_inflight(&self) {
        // Audit C1 fix — the daemon calls this from the
        // `FrontendMessage::CancelTurn` handler. Forwards to
        // the underlying daemon stub.
        self.inner.cancel_inflight();
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
            supersedes_proposal_id: None,
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
            supersedes_proposal_id: None,
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

    // ---- Phase 102: fold_tool_stats ---------------------------------

    fn completed_outcome() -> aivyx_core::ToolOutcomeSummary {
        aivyx_core::ToolOutcomeSummary::Completed {
            verified: aivyx_core::VerificationSummary::NotApplicable,
        }
    }

    fn tc_entry(
        seq: u64,
        scope: &str,
        outcome: aivyx_core::ToolOutcomeSummary,
        duration_ms: u64,
        appended_at: std::time::SystemTime,
    ) -> aivyx_audit::SignedEntry {
        aivyx_audit::SignedEntry {
            seq,
            appended_at,
            event: aivyx_audit::AuditEvent::ToolCall {
                turn_id: aivyx_core::TurnId::new(),
                tool_id: aivyx_core::ToolId::new(),
                scope_used: aivyx_capability::Scope::parse(scope).unwrap(),
                input_hash: [0u8; 32],
                outcome,
                duration: std::time::Duration::from_millis(duration_ms),
                auto_corrected_from: None,
                extracted_from_text: None,
            },
            mac: [0u8; 32],
            prev_mac: [0u8; 32],
        }
    }

    fn desc(name: &str, scope_base: &str) -> ToolDescriptor {
        ToolDescriptor {
            name: name.to_string(),
            description: format!("{name} tool"),
            scope_base: scope_base.to_string(),
        }
    }

    #[test]
    fn fold_empty_chain_lists_registered_tools_with_zero_stats() {
        let descs = vec![desc("fs.read", "fs.read"), desc("fs.write", "fs.write")];
        let rows = fold_tool_stats(&[], None, &descs);
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.registered && r.calls == 0));
    }

    #[test]
    fn fold_counts_calls_and_outcomes_by_scope_base() {
        let now = std::time::SystemTime::now();
        let entries = vec![
            tc_entry(0, "fs.read:/x/**", completed_outcome(), 10, now),
            tc_entry(1, "fs.read:/y/**", completed_outcome(), 20, now),
            tc_entry(
                2,
                "fs.read:/z/**",
                aivyx_core::ToolOutcomeSummary::Failed,
                6,
                now,
            ),
        ];
        let descs = vec![desc("fs.read", "fs.read")];
        let rows = fold_tool_stats(&entries, None, &descs);
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.calls, 3);
        assert_eq!(r.scope_base, "fs.read");
        assert_eq!(r.outcomes.get("completed"), Some(&2));
        assert_eq!(r.outcomes.get("failed"), Some(&1));
        assert_eq!(r.total_duration_ms, 36);
    }

    #[test]
    fn fold_window_filter_excludes_entries_before_the_cutoff() {
        let now = std::time::SystemTime::now();
        let old = now - std::time::Duration::from_secs(7200);
        let entries = vec![
            tc_entry(0, "fs.read:/a/**", completed_outcome(), 5, old),
            tc_entry(1, "fs.read:/b/**", completed_outcome(), 5, now),
        ];
        let descs = vec![desc("fs.read", "fs.read")];
        // Cutoff one hour ago — the two-hour-old entry is excluded.
        let cutoff = now - std::time::Duration::from_secs(3600);
        let rows = fold_tool_stats(&entries, Some(cutoff), &descs);
        assert_eq!(rows[0].calls, 1, "only the in-window call counts");
    }

    #[test]
    fn fold_called_but_unregistered_base_gets_an_unregistered_row() {
        let now = std::time::SystemTime::now();
        let entries =
            vec![tc_entry(0, "shell.exec:cwd:/x/**", completed_outcome(), 9, now)];
        // No descriptor for shell.exec — only fs.read is registered.
        let descs = vec![desc("fs.read", "fs.read")];
        let rows = fold_tool_stats(&entries, None, &descs);
        let shell = rows
            .iter()
            .find(|r| r.scope_base == "shell.exec")
            .expect("a called base with no descriptor must still get a row");
        assert!(!shell.registered);
        assert_eq!(shell.calls, 1);
    }
}
