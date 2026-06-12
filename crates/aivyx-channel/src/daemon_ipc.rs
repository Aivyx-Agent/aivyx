//! Daemon IPC protocol types and framing.
//!
//! Implements the wire format specified in `docs/DAEMON_IPC.md`:
//! length-prefixed JSON frames over a Unix domain socket. Three
//! top-level message envelopes (`FrontendMessage`, `DaemonMessage`,
//! `DaemonLifecycleEvent`) are serde-serializable and round-trip
//! through the `encode_frame` / `decode_frame` helpers.
//!
//! Phase 16 Task 2 — this module is the parsing substrate the PoC
//! daemon (Task 3) builds on. It deliberately owns no I/O; the
//! async read/write loops live in the daemon and frontend dispatch
//! paths.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Protocol version sent in `DaemonReady`. Phase 16 defines `"0.1"`.
pub const PROTOCOL_VERSION: &str = "0.1";

/// 16 MiB — per `docs/DAEMON_IPC.md`. A frame whose length prefix
/// exceeds this is a protocol error.
pub const MAX_PAYLOAD_SIZE: u32 = 16 * 1024 * 1024;

/// Length of the frame header (4-byte big-endian payload length).
pub const FRAME_HEADER_LEN: usize = 4;

/// Resolve the daemon socket path per `docs/DAEMON_IPC.md`:
///
/// 1. `$XDG_RUNTIME_DIR/aivyx/daemon.sock` (preferred)
/// 2. `$HOME/.local/share/aivyx/daemon.sock` (fallback)
///
/// Returns `Err` only if neither `XDG_RUNTIME_DIR` nor `HOME` is set.
pub fn default_socket_path() -> Result<PathBuf, String> {
    if let Ok(xdg) = std::env::var("XDG_RUNTIME_DIR") {
        return Ok(PathBuf::from(xdg).join("aivyx").join("daemon.sock"));
    }
    if let Ok(home) = std::env::var("HOME") {
        return Ok(PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("aivyx")
            .join("daemon.sock"));
    }
    Err("neither XDG_RUNTIME_DIR nor HOME is set; cannot determine daemon socket path".into())
}

/// Resolve the daemon PID file path — sibling of the socket file.
///
/// `$XDG_RUNTIME_DIR/aivyx/daemon.pid` (preferred) or
/// `$HOME/.local/share/aivyx/daemon.pid` (fallback).
pub fn default_pid_path() -> Result<PathBuf, String> {
    default_socket_path().map(|p| p.with_extension("pid"))
}

// ---------------------------------------------------------------------------
// Frontend type — identifies the connecting adapter.
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FrontendType {
    Local,
    Telegram,
    Web,
    /// Phase 111 — Discord adapter daemon-frontend. Mirrors the
    /// Phase 19 Telegram-over-daemon pattern; the daemon-side
    /// `discord_daemon_frontend.rs` builds an `IpcChannelBridge`
    /// when a `FrontendType::Discord` connection arrives.
    Discord,
    /// Phase 111 — Slack adapter daemon-frontend. Same shape as
    /// Discord; the daemon-side `slack_daemon_frontend.rs` builds
    /// an `IpcChannelBridge` when a `FrontendType::Slack`
    /// connection arrives.
    Slack,
}

// ---------------------------------------------------------------------------
// Phase 45 — IPC attachment for multimodal input
// ---------------------------------------------------------------------------

/// A base64-encoded file attachment sent with `SubmitInput`. The daemon
/// decodes the base64 data and constructs the appropriate `Message`
/// variant (image, text+image, or text-only if no attachments).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IpcAttachment {
    pub media_type: String,
    pub data_base64: String,
    #[serde(default)]
    pub filename: Option<String>,
}

// ---------------------------------------------------------------------------
// Phase 47 — Query/QueryResponse envelope (Web UI Phase 2)
// ---------------------------------------------------------------------------

/// Inspection-side queries the frontend sends to the daemon. Carried
/// inside [`FrontendMessage::Query`] with a correlation `id` the daemon
/// echoes back in [`DaemonMessage::QueryResponse`].
///
/// All queries are read-only by contract — mutating operations stay on
/// the existing turn-loop / gate-resolution paths.
///
/// **Authorization:** none at the query layer. The daemon IPC socket
/// is `mode 0600` owned by the operator's UID (`PRODUCT.md` P6 /
/// `docs/THREAT_MODEL.md` §4.4). Anyone who can `read(2)` the socket
/// *is* the operator, so per-query capability gating would only check
/// the operator's own role envelope against their own inspection —
/// which is not the threat model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum QueryPayload {
    /// List active session IDs tracked by the daemon.
    ListSessions,
    /// List all missions persisted under `KeyDomain::Missions`.
    ListMissions,
    /// Fetch a single mission by id, including all its gates.
    GetMission {
        mission_id: String,
    },
    /// Paginated read of the persistent audit chain. Returns at most
    /// `limit` entries starting at `from_seq`. `limit` is capped
    /// server-side at 500 (Phase 47 Q3). Read-only.
    ListAuditEntries {
        from_seq: u64,
        limit: u32,
    },
    /// Cold-verify the in-memory audit chain. Returns whether the chain
    /// hashes match, the number of entries verified, and the first
    /// error encountered if any.
    VerifyAuditChain,
    /// Phase 58 — fetch the daemon's loaded `Profile`
    /// (PRODUCT.md P13). Read-only inspection. Returns a
    /// [`ProfileSummary`] snapshot of the in-memory state; same
    /// values the daemon is using for system-prompt assembly. The
    /// CLI `aivyx profile show` reads from disk directly; this
    /// query is the Web UI counterpart.
    GetProfile,
    /// Phase 60 — fetch the daemon's current effective Persona
    /// (PRODUCT.md P14). Read-only inspection. Returns the
    /// folded state — same values the assemble_session_prompt
    /// helper uses for the "## How I have learned to communicate"
    /// section.
    GetEffectivePersona,
    /// Phase 60 — fetch the persona delta chain with pagination.
    /// Mirrors `ListAuditEntries`. The daemon caps `limit`
    /// server-side at 500 entries per response.
    ListPersonaDeltas {
        from_seq: u64,
        limit: u32,
    },
    /// Phase 64 — fetch the full Persona chain in a single response
    /// for export. Unlike `ListPersonaDeltas` (paginated summaries
    /// for the Web UI), this returns full-fidelity `DeltaExport`
    /// values that preserve every PersonaDelta field. Operator-
    /// driven; intended to feed `aivyx identity export <path>`.
    /// The daemon returns up to `MAX_EXPORT_CHAIN_ENTRIES` entries
    /// in one shot (current cap: 100,000 — enough for years of
    /// reflection-approved deltas at realistic rates).
    ExportPersonaChain,
    /// Phase 70 — list pending and resolved Persona proposals.
    /// `status_filter` is one of `"all" | "pending" | "approved"
    /// | "rejected" | "superseded"`; unknown values default to
    /// `"pending"` server-side. The daemon caps the page at
    /// `limit` entries.
    ListPersonaProposals {
        status_filter: String,
        limit: u32,
    },
    /// Phase 70 — fetch a single Persona proposal by id. Returns
    /// the proposal's current status-derived view.
    GetPersonaProposal {
        proposal_id: String,
    },
    /// Phase 73 — paginated walk of the audit chain for
    /// `AutoNotifyDispatched` events. The daemon filters by
    /// `target_filter` when set and renders each match into a
    /// `NotificationHistoryEntry`. `limit` is server-side
    /// capped (same as audit-entry queries: 500 max per page).
    ListNotificationHistory {
        from_seq: u64,
        limit: u32,
        target_filter: Option<String>,
    },
    /// Phase 74 — list every distinct memory topic. Drives the
    /// Web UI Memory pane's left-column topic list + the
    /// `aivyx memory list` CLI render.
    ListMemoryTopics,
    /// Phase 74 — fetch up to `limit` entries for a single
    /// memory topic, newest first. Mirrors `Memory::get_recent`'s
    /// shape over the wire.
    GetMemoryTopicEntries {
        topic: String,
        limit: u32,
    },
    /// Phase 74 — substring search across topics + bodies.
    /// Empty `query` returns the newest entries across every
    /// topic.
    SearchMemory {
        query: String,
        limit: u32,
        /// Phase 75 — request the embedding-ranked path.
        /// `#[serde(default)]` (= `false`, keyword) so
        /// pre-Phase-75 clients and stored frames round-trip
        /// unchanged. Falls back to keyword transparently when
        /// embedding is unavailable.
        #[serde(default)]
        semantic: bool,
    },
    /// Phase 78 — read-only learning-observability query.
    /// `window_secs = None` → the handler's default lookback.
    /// `#[serde(default)]` so older clients/frames decode.
    GetLearningInsights {
        #[serde(default)]
        window_secs: Option<u64>,
    },
    /// Phase 102 — read-only tool-observability query. Returns the
    /// daemon's registered tool set joined with audit-derived
    /// call statistics. `window_secs = None` → the whole audit
    /// chain; `Some(n)` → only `ToolCall` events from the last `n`
    /// seconds. `#[serde(default)]` so older clients/frames decode.
    GetToolStats {
        #[serde(default)]
        window_secs: Option<u64>,
    },
    /// Phase 119 Task 6 — operator-inspection dump of the
    /// Phase 116 `KeyDomain::ToolRelevanceLedger`. Returns every
    /// per-keyword-key outcome row optionally filtered to a
    /// single keyword key. `#[serde(default)]` so the filter is
    /// absent in pre-Phase-119 frames (which won't send this
    /// query at all, but the wire-compat pattern stays uniform).
    DumpToolRelevance {
        #[serde(default)]
        keyword_key_filter: Option<String>,
    },
    /// Phase 173 — add a story to the autonomous-loop backlog.
    /// `priority = None` → the `[loop].default_priority` (or the
    /// built-in default). Returns the new story's id.
    LoopAdd {
        title: String,
        #[serde(default)]
        body: String,
        #[serde(default)]
        priority: Option<u32>,
    },
    /// Phase 173 — list every backlog story (all statuses).
    LoopList,
    /// Phase 173 — start an autonomous-loop run. `max_iterations
    /// = None` → the `[loop].max_iterations` default. Fails if a
    /// run is already active or the `[loop]` section is not armed.
    LoopStart {
        #[serde(default)]
        max_iterations: Option<u32>,
    },
    /// Phase 173 — request the active run to stop (between
    /// iterations). Fails if no run is active.
    LoopStop,
    /// Phase 173 — read the loop run state + remaining backlog.
    LoopStatus,
    /// Phase 175 — read the recent loop progress-log notes
    /// (operator parity with what the driver injects). `limit =
    /// None` → a default window.
    LoopLog {
        #[serde(default)]
        limit: Option<u32>,
    },
    /// Phase 177 — mark a pending backlog story `Skipped` (the
    /// operator prunes a stuck / no-longer-wanted story). Reuses
    /// the `LoopControl` response.
    LoopSkip {
        story_id: String,
    },
    /// Chapter L (L.5) — start a daemon-run team mission from an explicit
    /// [`MissionPlan`]. `config` pins a vertical-pack team (`None` ⇒ the daemon
    /// default Nonagon). Fails if no team service is configured. Responds with
    /// [`QueryResponsePayload::TeamRunStarted`].
    TeamRun {
        plan: aivyx_team::MissionPlan,
        #[serde(default)]
        config: Option<aivyx_team::TeamConfig>,
    },
    /// Chapter L — start a mission from a free-text goal: the daemon decomposes
    /// it into a plan (one LLM planning call over the chosen team's roster) and
    /// runs it. `config` pins a vertical-pack team (`None` ⇒ the default
    /// Nonagon). Responds with [`QueryResponsePayload::TeamRunStarted`].
    TeamRunGoal {
        goal: String,
        #[serde(default)]
        config: Option<aivyx_team::TeamConfig>,
    },
    /// Chapter L (L.5) — every team mission's snapshot (the poll feed the TUI
    /// Missions panel ticks). Responds with
    /// [`QueryResponsePayload::TeamMissionList`].
    TeamMissionList,
    /// Chapter L (L.5) — one team mission's snapshot. Responds with
    /// [`QueryResponsePayload::TeamMissionStatus`] (`None` if unknown).
    TeamMissionStatus {
        mission_id: String,
    },
    /// Chapter L (L.5) — approve or reject a mission paused at a human-approval
    /// gate. Responds with [`QueryResponsePayload::TeamGateResolved`].
    ResolveTeamGate {
        mission_id: String,
        step: String,
        approve: bool,
    },
}

/// Response payload mirroring [`QueryPayload`]. Wrapped in
/// [`DaemonMessage::QueryResponse`] with the same correlation `id`
/// the query was sent with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
// Phase 95 — `LearningInsights` accumulates ~14 optional
// stat fields across phases 78-93 plus the Phase 95 cadence
// vec. Boxing each Option<Stat> would churn serde wire
// formats for marginal benefit (the variant is heap-
// allocated in practice — most fields are `None` or short
// `Vec`s). The size disparity is an artifact of the
// wire-compat-via-additive-fields pattern the project uses.
#[allow(clippy::large_enum_variant)]
pub enum QueryResponsePayload {
    /// Response to [`QueryPayload::ListSessions`].
    ListSessions {
        sessions: Vec<SessionSummary>,
    },
    /// Response to [`QueryPayload::ListMissions`].
    ListMissions {
        missions: Vec<MissionSummary>,
    },
    /// Response to [`QueryPayload::GetMission`]. `mission` is `None`
    /// when the mission id does not exist (not an error).
    GetMission {
        mission: Option<MissionDetail>,
    },
    /// Response to [`QueryPayload::ListAuditEntries`]. `entries` is the
    /// page of summaries; `total_len` is the full chain length so the
    /// frontend can show "showing N..M of T" and know when to stop
    /// paginating.
    ListAuditEntries {
        entries: Vec<AuditEntrySummary>,
        total_len: u64,
    },
    /// Response to [`QueryPayload::VerifyAuditChain`].
    VerifyAuditChain {
        ok: bool,
        entries_verified: u64,
        /// Human-readable failure description on `ok == false`.
        error: Option<String>,
    },
    /// The daemon could not answer the query. `code` is a stable
    /// machine-readable label; `message` is human-readable.
    QueryError {
        code: String,
        message: String,
    },
    /// Response to [`QueryPayload::GetProfile`]. Phase 58 — the
    /// daemon's currently-loaded Profile snapshot. The Web UI
    /// renders this into the Profile pane mirroring `aivyx profile
    /// show`. Always populated — even on a daemon with no `[profile]`
    /// section in TOML, the synthesized default is returned (the
    /// snapshot includes `injection_enabled = false` in that case).
    GetProfile {
        profile: ProfileSummary,
    },
    /// Response to [`QueryPayload::GetEffectivePersona`]. Phase 60
    /// — the current folded Persona state. Always populated; an
    /// empty Persona returns an [`EffectivePersonaSummary`] with all
    /// fields empty / `None`.
    GetEffectivePersona {
        persona: EffectivePersonaSummary,
    },
    /// Response to [`QueryPayload::ExportPersonaChain`]. Phase 64.
    /// Full-fidelity chain in a single response for the
    /// `aivyx identity export` flow. `deltas` is the chain in
    /// order; `effective` is the folded state at export time
    /// (the export bundle embeds this as `effective_at_export`
    /// per Q5(a)).
    ExportPersonaChain {
        deltas: Vec<crate::identity_export::DeltaExport>,
        effective: crate::persona::EffectivePersona,
    },
    /// Response to [`QueryPayload::ListPersonaDeltas`]. Phase 60
    /// — paginated page of approved deltas. `total_len` is the full
    /// chain length so the frontend knows when to stop paginating.
    ListPersonaDeltas {
        entries: Vec<PersonaDeltaSummary>,
        total_len: u64,
    },
    /// Phase 70 — response to [`QueryPayload::ListPersonaProposals`].
    /// `proposals` is the filtered page; `total_len` is the total
    /// number of proposals matching the filter (not capped by
    /// `limit`).
    ListPersonaProposals {
        proposals: Vec<PersonaProposalSummary>,
        total_len: u64,
    },
    /// Phase 70 — response to [`QueryPayload::GetPersonaProposal`].
    /// `proposal` is `None` when the id does not exist.
    GetPersonaProposal {
        proposal: Option<PersonaProposalSummary>,
    },
    /// Phase 73 — response to [`QueryPayload::ListNotificationHistory`].
    /// `entries` is the filtered page; `total_len` is the total
    /// number of `AutoNotifyDispatched` audit events matching
    /// the filter (uncapped by `limit`).
    ListNotificationHistory {
        entries: Vec<NotificationHistoryEntry>,
        total_len: u64,
    },
    /// Phase 74 — response to [`QueryPayload::ListMemoryTopics`].
    /// Distinct topic names sorted ascending.
    ListMemoryTopics {
        topics: Vec<String>,
    },
    /// Phase 74 — response to [`QueryPayload::GetMemoryTopicEntries`].
    /// Newest-first paginated entries for one topic.
    GetMemoryTopicEntries {
        entries: Vec<MemoryEntrySummary>,
    },
    /// Phase 74 — response to [`QueryPayload::SearchMemory`].
    /// Matching entries newest-first.
    SearchMemory {
        matches: Vec<MemoryEntrySummary>,
        /// Phase 75 — `true` when a `semantic` request was
        /// transparently served by the keyword path (no
        /// `[embedding]` config, provider call failed, or the
        /// corpus has zero vectors). `#[serde(default)]` so
        /// older frames decode as `false`.
        #[serde(default)]
        fell_back_to_keyword: bool,
    },
    /// Phase 78 — response to
    /// [`QueryPayload::GetLearningInsights`]. The digest is the
    /// per-window operational picture; `proposals` is the
    /// reconstructed provenance for each recall-driven Persona
    /// proposal. An empty digest (zero recalls) is a valid
    /// "nothing learned yet" answer, not an error.
    LearningInsights {
        digest: crate::recall_insights::LearningDigest,
        proposals: Vec<crate::recall_insights::ProposalProvenance>,
        /// Phase 79 (Q4a) — the last turn's adaptive-Persona
        /// selection (selected/total facets), or `None` if no
        /// adaptive selection has run (no `[embedding]`, small
        /// Soul, or pre-Phase-79). `#[serde(default)]` so older
        /// frames decode.
        #[serde(default)]
        persona_selection:
            Option<crate::persona_context::PersonaSelectionStat>,
        /// Phase 80 (Q4a) — the last proactive cycle's outcome
        /// (what was surfaced + why, deduped/capped counts), or
        /// `None` if proactive has not run (off / no schedule /
        /// pre-Phase-80). `#[serde(default)]` so older frames
        /// decode.
        #[serde(default)]
        proactive:
            Option<crate::proactive_detect::ProactiveStat>,
        /// Phase 81 (Q4a) — the last persona-lifecycle cycle's
        /// outcome (what was proposed for consolidation/decay +
        /// why, deduped count), or `None` if the lifecycle pass
        /// has not run (off / no schedule / pre-Phase-81).
        /// `#[serde(default)]` so older frames decode.
        #[serde(default)]
        persona_lifecycle: Option<
            crate::persona_lifecycle::PersonaLifecycleStat,
        >,
        /// Phase 82 — the durable, decayed accumulated
        /// per-topic helpfulness (the longitudinal view Phase
        /// 78 deferred, distinct from the windowed
        /// `digest.top_helpful`). `None` if the ledger is
        /// absent / empty (no auto-recall, or pre-Phase-82).
        /// `#[serde(default)]` so older frames decode.
        #[serde(default)]
        accumulated_helpfulness: Option<
            crate::helpfulness_ledger::AccumulatedHelpfulness,
        >,
        /// Phase 83 — the durable cross-session co-occurrence
        /// patterns (topics that consistently help together).
        /// `None` if the ledger is absent / empty (no
        /// auto-recall, or pre-Phase-83). `#[serde(default)]`
        /// so older frames decode.
        #[serde(default)]
        cooccurrence: Option<
            crate::cooccurrence_ledger::CooccurrencePatterns,
        >,
        /// Phase 84 — the last turn's cluster-aware co-recall
        /// outcome (driver→sibling pairs injected). `None` if
        /// cluster expansion is off / has not run this daemon
        /// lifetime. `#[serde(default)]` so older frames
        /// decode.
        #[serde(default)]
        cluster_recall: Option<
            crate::memory_recall::RecallClusterStat,
        >,
        /// Phase 87 — the last reflection cycle's pattern-
        /// driven Persona consolidation outcome (filed pairs +
        /// the LLM-availability flag). `None` if
        /// `[persona_consolidation]` is off, no cycle has
        /// fired this daemon lifetime, or the substrate is
        /// missing. `#[serde(default)]` so older frames decode.
        #[serde(default)]
        persona_consolidation: Option<
            crate::persona_consolidation::PersonaConsolidationStat,
        >,
        /// Phase 172 — durable accumulated correction view
        /// (the topics the operator most often reworks). `None`
        /// if the correction ledger is absent / empty (no
        /// auto-recall, or pre-Phase-172). `#[serde(default)]`
        /// so older frames decode.
        #[serde(default)]
        accumulated_corrections: Option<
            crate::correction_ledger::AccumulatedCorrections,
        >,
        /// Phase 172 — the last reflection cycle's correction-
        /// driven Persona consolidation outcome (filed topics +
        /// the LLM-availability flag). `None` if
        /// `[correction_consolidation]` is off, no cycle has
        /// fired this daemon lifetime, or the substrate is
        /// missing. `#[serde(default)]` so older frames decode.
        #[serde(default)]
        correction_consolidation: Option<
            crate::correction_consolidation::CorrectionConsolidationStat,
        >,
        /// Phase 178 — last reflection cycle's correction-
        /// judgment outcome (judged / rework / praise /
        /// unrelated / structural-fallback counts). `None` when
        /// `[correction_judgment]` is off / the fold hasn't run.
        /// `#[serde(default)]` so older frames decode.
        #[serde(default)]
        correction_judgment: Option<
            crate::correction_judgment::CorrectionJudgmentStat,
        >,
        /// Phase 91 — last reflection cycle's LLM-judged
        /// recall outcome (per-classification counts +
        /// `(topic, judgment)` pairs + the
        /// `llm_unavailable` flag). `None` when the
        /// `[recall_judgment]` section is off / the pass
        /// has never run. `#[serde(default)]` so older
        /// frames decode unchanged.
        #[serde(default)]
        recall_judgment: Option<
            crate::recall_judgment::RecallJudgmentStat,
        >,
        /// Phase 95 — per-schedule accumulating cadence
        /// stats. Each entry is `(schedule_name,
        /// RecentReflectionStat { fired, skipped })`.
        /// Empty `Vec` when no schedule has ever made a
        /// cadence decision (the in-memory map starts
        /// empty; first cycle decisions populate it).
        /// `#[serde(default)]` so older frames decode
        /// unchanged.
        #[serde(default)]
        cadence: Vec<(
            String,
            crate::reflection_scheduler::RecentReflectionStat,
        )>,
    },
    /// Phase 102 — response to [`QueryPayload::GetToolStats`]. One
    /// [`ToolStat`] per tool, ordered by call count descending then
    /// name ascending. An empty `Vec` is a valid "no tools, no
    /// calls" answer, not an error.
    ToolStats {
        tools: Vec<ToolStat>,
    },
    /// Phase 119 Task 6 — response to
    /// [`QueryPayload::DumpToolRelevance`]. Flat per-row table
    /// rather than per-keyword-key nested entries: the operator's
    /// CLI renders one table row per `(keyword_key, surface_kind,
    /// identifier)` triple, so the wire shape pre-flattens.
    /// An empty `Vec` is a valid "no entries" answer, not an
    /// error. Rows are ordered ascending by
    /// `(keyword_key, surface_kind, identifier)` so the operator's
    /// table renders in a stable column order.
    ToolRelevanceDump {
        rows: Vec<ToolRelevanceDumpRow>,
    },
    /// Phase 173 — response to [`QueryPayload::LoopAdd`]. Carries
    /// the new story's id.
    LoopStoryAdded {
        story_id: String,
    },
    /// Phase 173 — response to [`QueryPayload::LoopList`]. Every
    /// backlog story, insertion order.
    LoopBacklog {
        stories: Vec<crate::loop_backlog::Story>,
    },
    /// Phase 173 — response to [`QueryPayload::LoopStart`] /
    /// [`QueryPayload::LoopStop`]. `ok` is whether the control
    /// action took effect; `message` is the operator-readable
    /// result either way.
    LoopControl {
        ok: bool,
        message: String,
    },
    /// Phase 173 — response to [`QueryPayload::LoopStatus`]. The
    /// run state plus the live remaining-pending count.
    LoopStatus {
        state: crate::loop_driver::LoopRunState,
        remaining: usize,
        /// Whether the `[loop]` section is armed (the driver was
        /// spawned). When `false`, `loop start` will fail.
        armed: bool,
        /// Phase 174 — whether driver-side gate verification is
        /// configured (`[loop].gate_command` set). `#[serde(default)]`
        /// so pre-174 frames decode.
        #[serde(default)]
        gate_enabled: bool,
        /// Phase 174 — the wall-clock cap in seconds, if any.
        /// `#[serde(default)]` so pre-174 frames decode.
        #[serde(default)]
        max_run_secs: Option<u64>,
        /// Phase 176 — the per-run token budget, if any.
        /// `#[serde(default)]` so pre-176 frames decode.
        #[serde(default)]
        max_run_tokens: Option<u64>,
        /// Chapter K (K.4.2) — the per-run dollar cap, if any. The
        /// live spend rides `state.spent_cents`; this carries the cap
        /// value so `aivyx loop status` can show "spend / cap".
        /// `#[serde(default)]` so pre-K.4.2 frames decode.
        #[serde(default)]
        max_run_usd: Option<f64>,
    },
    /// Phase 175 — response to [`QueryPayload::LoopLog`]. Recent
    /// progress notes, most-recent-first.
    LoopProgressLog {
        notes: Vec<String>,
    },
    /// Chapter L (L.5) — response to [`QueryPayload::TeamRun`]. The new
    /// mission's id; the drive runs in the background (poll `TeamMissionStatus`).
    TeamRunStarted {
        mission_id: String,
    },
    /// Chapter L (L.5) — response to [`QueryPayload::TeamMissionList`]. Every
    /// known mission's full record (plan + checkpoint + phase), as the loop's
    /// `LoopBacklog` carries `Story`s. The TUI maps these → `MissionRow`s (L.6).
    TeamMissionList {
        missions: Vec<crate::team_mission::TeamMissionRecord>,
    },
    /// Chapter L (L.5) — response to [`QueryPayload::TeamMissionStatus`].
    /// `None` when the id is unknown.
    TeamMissionStatus {
        mission: Option<crate::team_mission::TeamMissionRecord>,
    },
    /// Chapter L (L.5) — response to [`QueryPayload::ResolveTeamGate`]. The
    /// phase the decision moved the mission to (`Executing` on approve — the
    /// resume drives in the background — or `Rejected`).
    TeamGateResolved {
        mission_id: String,
        phase: crate::team_mission::TeamMissionPhase,
    },
}

/// Phase 119 Task 6 — wire-format per-row dump entry for
/// [`QueryResponsePayload::ToolRelevanceDump`]. Flattens the
/// `(keyword_key, OutcomeRow)` pair so the CLI renders one
/// table row per entry without nested decode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolRelevanceDumpRow {
    /// The Phase 116 keyword key the outcome was recorded under.
    pub keyword_key: String,
    /// `"tool"` or `"skill"` (matches `RelevanceSurfaceKind::label()`).
    pub surface_kind: String,
    /// The tool or skill identifier (e.g. `fs.read`,
    /// `summarize-pdf`).
    pub identifier: String,
    pub success_count: u32,
    pub failure_count: u32,
    pub last_seen_unix_ms: u64,
}

/// Phase 102 — wire-format per-tool observability row for
/// [`QueryResponsePayload::ToolStats`]. One row per tool: the
/// registry-listing fields (`name`, `description`, `scope_base`,
/// `registered`) joined with the audit-derived call statistics.
/// The `aivyx tools` CLI renders one table row per `ToolStat`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolStat {
    /// Tool name as the planner advertises it (e.g. `fs.read`).
    pub name: String,
    /// One-line tool description.
    pub description: String,
    /// Capability base the tool's calls key on in the audit chain
    /// (`AuditEvent::ToolCall`'s `scope_used.base()`).
    pub scope_base: String,
    /// `true` when the tool is in the daemon's live registry. A
    /// `false` row is a base with audit history but no currently
    /// registered tool (a channel/role change, or a removed tool).
    pub registered: bool,
    /// Total `AuditEvent::ToolCall` events for this base within
    /// the requested window.
    pub calls: u64,
    /// Per-outcome counts, keyed by the stable outcome label
    /// (`completed`, `failed`, `denied`, `not_in_role`,
    /// `requires_escalation`). A key is absent when its count is
    /// zero; the present values sum to `calls`.
    pub outcomes: std::collections::BTreeMap<String, u64>,
    /// Total wall-clock duration across all `calls`, in
    /// milliseconds. The average is `total_duration_ms / calls`,
    /// derived client-side.
    pub total_duration_ms: u64,
}

/// Phase 74 — wire-format view of one memory entry. Flat shape
/// mirroring the Phase 47 audit / Phase 70 proposal summary
/// patterns; the Web UI Memory pane + `aivyx memory show` CLI
/// render against this type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryEntrySummary {
    pub topic: String,
    pub body: String,
    pub seq: u64,
    pub created_at_secs: u64,
    /// Phase 74 — `last_read_at_secs` so operators can sort the
    /// Memory pane by LRU heat. `0` means "never read since
    /// Phase 74 landed."
    pub last_read_at_secs: u64,
}

/// Phase 73 — flat wire view of one `AuditEvent::AutoNotifyDispatched`
/// entry. Mirrors the shape of `AuditEntrySummary` (Phase 47) but
/// projects the dispatch-specific fields into top-level keys so the
/// Web UI / CLI don't have to dig into a nested JSON.
///
/// `outcome_kind` is the stable string label of
/// `AutoNotifyOutcomeSummary` (`"delivered"`, `"failed"`,
/// `"skipped_empty_response"`, `"skipped_by_condition"`,
/// `"skipped_by_rate_limit"`). `outcome_detail` carries
/// variant-specific data — error message for `failed`, condition
/// label for `skipped_by_condition`, `"<limit>/<window_secs>s"`
/// for `skipped_by_rate_limit`, empty for `delivered` and
/// `skipped_empty_response`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationHistoryEntry {
    pub seq: u64,
    pub dispatched_at_unix_ms: u64,
    pub session_id: String,
    pub trigger_kind: String,
    pub trigger_id: String,
    pub target_name: String,
    pub outcome_kind: String,
    pub outcome_detail: String,
}

/// Minimal per-session metadata returned by
/// [`QueryResponsePayload::ListSessions`]. Will grow with later
/// Phase 47 tasks (mission/audit) — kept additive so older frontends
/// still deserialize new daemons.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub session_id: String,
}

/// Compact mission view for the dashboard list pane. Mirrors the
/// fields needed for a row in a table; the full record (including
/// gates) is fetched on demand via
/// [`QueryPayload::GetMission`].
///
/// `state` is the rendered string form of `MissionState`
/// (`"Created" | "Running" | "GatePending" | "Completed" |
/// "Failed" | "Cancelled"`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MissionSummary {
    pub mission_id: String,
    pub role_name: String,
    pub description: String,
    pub state: String,
    pub has_pending_gate: bool,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Full mission view including gate history. Returned by
/// [`QueryResponsePayload::GetMission`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MissionDetail {
    pub mission_id: String,
    pub role_name: String,
    pub description: String,
    pub state: String,
    pub gates: Vec<GateSummary>,
    pub created_at: u64,
    pub updated_at: u64,
}

/// One gate within a `MissionDetail`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateSummary {
    pub gate_id: String,
    pub reason: String,
    pub scope: Option<String>,
    /// `"Pending" | "Approved" | "Rejected"`.
    pub state: String,
    pub created_at: u64,
    pub resolved_at: Option<u64>,
}

/// One row of the audit log as exposed over IPC. Projected from
/// `aivyx_audit::SignedEntry` — the wire shape intentionally keeps
/// the event body as `serde_json::Value` so additions to the
/// `AuditEvent` enum do not break the IPC schema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditEntrySummary {
    pub seq: u64,
    /// Unix millis. `SystemTime` is converted at the daemon boundary
    /// so the wire format does not depend on platform clock encoding.
    pub appended_at_unix_ms: u64,
    /// String label of the `AuditEvent` variant — `"ToolCall"`,
    /// `"ScopeDenied"`, `"TurnStarted"`, `"TurnEnded"`,
    /// `"MemoryAccess"`. Stable; new variants append new labels.
    pub event_type: String,
    /// Full event payload as JSON. Schema follows `AuditEvent`'s
    /// serde repr.
    pub event: serde_json::Value,
    /// HMAC tag, hex-encoded for display.
    pub mac_hex: String,
}

/// Operator-declared Profile snapshot returned by
/// [`QueryResponsePayload::GetProfile`]. Phase 58 (PRODUCT.md P13).
///
/// Wire-shaped mirror of `aivyx_config::Profile` — flattens
/// `Sourced<T>` into plain serializable fields and pre-computes the
/// `injection_enabled` predicate (the runtime
/// `Profile::is_operator_declared()` result) so the Web UI does not
/// need to re-implement the rule.
///
/// `assistant_name_source` is the stringified [`aivyx_config::FieldSource`]:
/// `"toml"`, `"default"`, `"env"`, or `"encrypted-store"`. Other
/// fields do not carry per-field provenance — Profile fields other
/// than `assistant_name` are either declared in `[profile]` or
/// absent, never sourced from env or the encrypted store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProfileSummary {
    pub assistant_name: String,
    pub assistant_name_source: String,
    pub operator_profile: Option<String>,
    pub communication_style: Option<String>,
    pub primary_use_cases: Vec<String>,
    pub behavioral_preferences: Vec<String>,
    pub behavioral_constraints: Vec<String>,
    /// `true` when the daemon's `Profile::is_operator_declared()`
    /// returned `true` — i.e. Profile is shaping every turn's system
    /// prompt via `assemble_session_prompt`. `false` means the
    /// substrate is at its passthrough default.
    pub injection_enabled: bool,
}

/// One signed persona delta as it appears over the wire. Mirrors
/// the in-memory `aivyx_channel::persona::SignedPersonaEntry` but
/// flattens the HMAC arrays to hex strings (so the JSON wire format
/// stays uniform with other Summary types) and stringifies the
/// `category` / `op` fields for stable cross-version compatibility.
///
/// Phase 60 — used by the `ListPersonaDeltas` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaDeltaSummary {
    pub seq: u64,
    pub delta_id: String,
    pub proposed_at_unix_ms: u64,
    pub approved_at_unix_ms: u64,
    pub proposal_id: String,
    /// Stable string label of `PersonaDeltaCategory` — e.g.
    /// `"BehavioralPreferences"`, `"LearnedContext"`.
    pub category: String,
    /// Op as JSON object: `{ "kind": "SetScalar", "value": ... }`,
    /// `{ "kind": "AppendList", "value": "..." }`, etc. The wire
    /// shape mirrors `PersonaDeltaOp`'s serde repr.
    pub op: serde_json::Value,
    pub mac_hex: String,
}

/// Wire-format view of a Persona proposal. Phase 70 — used by
/// `ListPersonaProposals` + `GetPersonaProposal`.
///
/// `proposed_op` is the agent's original proposal (always present);
/// `applied_op` and `applied_seq` are populated only when the
/// proposal's current status is `Approved` (and may differ from
/// `proposed_op` if the operator edited before approving — Q3(a)).
/// `reason` is populated only when the current status is `Rejected`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaProposalSummary {
    pub id: String,
    pub proposed_at_unix_ms: u64,
    pub source_reflection_session_id: String,
    /// Stable string label, one of `"Pending" | "Approved" |
    /// "Rejected" | "Superseded"`.
    pub status: String,
    /// Stable string label of the proposal's category.
    pub category: String,
    /// Agent's original op as JSON; same shape as
    /// [`PersonaDeltaSummary::op`].
    pub proposed_op: serde_json::Value,
    /// Optional operator-supplied reason the agent gave for
    /// proposing this delta.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposed_reason: Option<String>,
    /// On `Approved` proposals only: the op that was actually
    /// applied (may differ from `proposed_op` per Q3(a) edit-on-
    /// approve flow).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_op: Option<serde_json::Value>,
    /// On `Approved` proposals only: seq of the resulting
    /// PersonaDelta in the persona chain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_seq: Option<u64>,
    /// On `Rejected` proposals only: operator-supplied reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejected_reason: Option<String>,
    /// Resolved-at timestamp for `Approved | Rejected |
    /// Superseded` proposals; `None` for `Pending`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at_unix_ms: Option<u64>,
    /// Phase 92 — when this proposal is one half of a
    /// linked supersession pair, the id of the other half.
    /// `None` for proposals that are not part of a
    /// supersession (the common case). Pulled up from the
    /// inner `ProposedPersonaDelta` so the Phase 94
    /// surface-side grouping helper can read it without
    /// re-parsing the embedded `proposed_op` JSON.
    /// `#[serde(default, skip_serializing_if =
    /// "Option::is_none")]` preserves IPC wire-compat — the
    /// established Phase 84 / 91 / 92 pattern.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes_proposal_id: Option<String>,
}

/// Folded effective Persona snapshot. Phase 60 — returned by
/// `GetEffectivePersona`. Mirrors `aivyx_channel::persona::EffectivePersona`
/// shape directly; serializable so the Web UI can render it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct EffectivePersonaSummary {
    pub assistant_name: Option<String>,
    pub operator_profile: Option<String>,
    pub communication_style: Option<String>,
    pub primary_use_cases: Vec<String>,
    pub behavioral_preferences: Vec<String>,
    pub behavioral_constraints: Vec<String>,
    pub learned_context: Vec<String>,
    pub communication_adaptations: Vec<String>,
    pub character_traits: Vec<String>,
    pub relationship_milestones: Vec<String>,
    /// Pre-computed flag — `true` when any field is non-empty.
    pub is_non_empty: bool,
}

// ---------------------------------------------------------------------------
// Frontend → Daemon
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FrontendMessage {
    StartSession {
        role: Option<String>,
        #[serde(default)]
        frontend_type: Option<FrontendType>,
    },
    SubmitInput {
        session_id: String,
        text: String,
        #[serde(default)]
        mission_id: Option<String>,
        /// Phase 45 — optional image/file attachments. `#[serde(default)]`
        /// ensures old clients that omit this field still deserialize.
        #[serde(default)]
        attachments: Vec<IpcAttachment>,
    },
    CancelTurn {
        session_id: String,
    },
    ResolveGate {
        mission_id: String,
        gate_id: String,
        approved: bool,
    },
    Disconnect,
    Shutdown,
    /// Protocol version negotiation (Phase 41 Task 5).
    /// Sent by the frontend after receiving `DaemonReady`.
    /// For v0.1, the daemon always accepts.
    ProtocolNegotiation {
        version: String,
    },
    /// Phase 47 — read-only inspection query. The daemon answers with
    /// [`DaemonMessage::QueryResponse`] carrying the same `id`.
    Query {
        id: String,
        payload: QueryPayload,
    },
    /// Phase 60 — operator-initiated Persona revert (PRODUCT.md P14
    /// commit 4). The daemon appends a `Revert` op delta to the
    /// persona chain referencing `target_delta_id` and recomputes
    /// the shared runtime state so the next turn picks it up.
    /// Per Q5(a) at Phase 60 sign-off: reverts are operator-only;
    /// no gate prompt since the operator initiated.
    ///
    /// Reply: [`DaemonMessage::PersonaRevertResolved`] with the
    /// same `id`.
    RevertPersonaDelta {
        id: String,
        target_delta_id: String,
    },
    /// Phase 65 — operator-driven Persona chain import (Phase 60
    /// identity-deferral closer). Replays a parsed export bundle
    /// onto the local chain. Without `force` the daemon refuses if
    /// the local chain is non-empty. With `force` the daemon wipes
    /// the chain before replaying. Each delta is re-signed against
    /// the local HMAC key during replay (Phase 60 Q1(a)).
    ///
    /// Daemon-side flow (best-effort, no atomic-tx wrapping per
    /// Phase 65 Q1(a)): re-validate → conflict check → optional
    /// wipe → per-delta append → recompute shared runtime state.
    ///
    /// Reply: [`DaemonMessage::PersonaImportResolved`] with the
    /// same `id`.
    ImportPersonaChain {
        id: String,
        /// Deltas to replay in order. Each is appended via the
        /// existing `PersistentPersonaLog::append` path so the new
        /// chain's MACs bind to the target host's key.
        deltas: Vec<crate::identity_export::DeltaExport>,
        /// Expected effective state after replay; the daemon
        /// echoes this back in the response for the CLI to verify.
        /// Already validated against the deltas at parse time by
        /// the CLI, but carried to the daemon for completeness.
        ///
        /// Boxed at Phase 118 — the two new operator-staged
        /// list fields on `EffectivePersona` (`profile_hints`,
        /// `role_drafts`) pushed the struct past the
        /// `clippy::large_enum_variant` threshold for this
        /// variant. Boxing keeps the rest of the
        /// `FrontendMessage` enum compact; the indirection is
        /// invisible to the daemon-side handler.
        effective_at_export: Box<crate::persona::EffectivePersona>,
        /// If `false` and the local chain is non-empty, refuse.
        /// If `true`, wipe and replace.
        force: bool,
    },
    /// Phase 70 — operator-initiated resolution of a pending
    /// Persona proposal (P14 self-learning closure). Per Q3(a)
    /// at Phase 70 sign-off the operator can approve as-is,
    /// approve-with-edit (the daemon applies the edited op
    /// instead of the original), or reject with an optional
    /// reason.
    ///
    /// Daemon-side flow on `Approve` / `ApproveWithEdit`:
    /// validate the applied op → append a `PersonaDelta` to
    /// the persona chain → append an `Approved` entry to the
    /// proposal chain referencing the delta's seq → recompute
    /// shared runtime state. On `Reject`: append a `Rejected`
    /// entry only.
    ///
    /// Reply: [`DaemonMessage::PersonaProposalResolved`] with
    /// the same `id`.
    ResolvePersonaProposal {
        id: String,
        proposal_id: String,
        resolution: PersonaProposalResolution,
    },
    /// Phase 74 — operator-initiated memory topic eviction.
    /// Deletes every entry under `topic`; replies with
    /// [`DaemonMessage::MemoryEvictResolved`] carrying the
    /// number of entries deleted on success.
    EvictMemoryTopic {
        id: String,
        topic: String,
    },
    /// Phase 119 — operator's act-on-approval gesture for a
    /// Phase 118 `ProfileHint` proposal. Carries the values
    /// the CLI already wrote to `aivyx.toml` via the Task 3
    /// atomic primitive; the daemon's job is to record the
    /// `AuditEvent::ProfileHintApplied` entry so forensic
    /// walks see the apply alongside the upstream
    /// `SkillAutoProposal` + `PersonaProposalResolved`.
    ///
    /// Reply: [`DaemonMessage::ProfileHintApplyAcked`].
    ApplyProfileHint {
        id: String,
        /// Source proposal id from the operator-approved
        /// `ProfileHint` chain entry.
        proposal_id: String,
        /// The declared Profile-config field the apply
        /// mutated (matches `ProfileField::label()`).
        field: String,
        /// The value written to aivyx.toml — new scalar for
        /// scalar fields, appended entry for list fields.
        applied_value: String,
    },
    /// Phase 119 — operator's act-on-approval gesture for a
    /// Phase 118 `RoleDefinitionSuggestion` proposal.
    /// Mirrors `ApplyProfileHint` for the second category.
    ///
    /// Reply: [`DaemonMessage::RoleDraftImportAcked`].
    ImportRoleDraft {
        id: String,
        /// Source proposal id from the operator-approved
        /// `RoleDefinitionSuggestion` chain entry.
        proposal_id: String,
        /// The kebab-case role name written.
        role_name: String,
        /// The parent role for inheritance (or `None` for
        /// top-level).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<String>,
    },
}

/// Phase 70 — operator resolution variants for
/// [`FrontendMessage::ResolvePersonaProposal`]. Tagged so
/// future variants (e.g. `Defer`, `RejectWithSuggestion`) can be
/// added without breaking older daemons / frontends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum PersonaProposalResolution {
    /// Approve verbatim — apply the agent's `proposed_op`
    /// unchanged.
    Approve,
    /// Approve with operator edits. The daemon validates and
    /// applies `edited_op` instead of the original
    /// `proposed_op`. Both are preserved in the proposal chain
    /// for audit.
    ApproveWithEdit {
        edited_op: crate::persona::ProposedPersonaDelta,
    },
    /// Reject the proposal. `reason` is optional and carried in
    /// the audit trail.
    Reject {
        reason: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// Daemon → Frontend (turn-loop traffic)
// ---------------------------------------------------------------------------

// `QueryResponse`'s `LearningInsights` payload legitimately
// accretes one read-only surface field per learning phase
// (79/80/81/82/83…); boxing every protocol field for a
// non-hot-path control message would harm readability for a
// marginal stack-size win that the next phase reintroduces.
// The large variant *is* the common case here.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonMessage {
    SessionStarted {
        session_id: String,
    },
    StreamEvent {
        session_id: String,
        event: StreamEventPayload,
    },
    TurnComplete {
        session_id: String,
        outcome: String,
    },
    Error {
        code: String,
        message: String,
    },
    MissionCreated {
        mission_id: String,
    },
    MissionStateChanged {
        mission_id: String,
        state: String,
    },
    GateResolved {
        mission_id: String,
        gate_id: String,
        approved: bool,
    },
    /// Protocol version accepted (Phase 41 Task 5).
    ProtocolAccepted {
        version: String,
    },
    /// Protocol version rejected — client should disconnect or retry
    /// with a supported version (Phase 41 Task 5).
    ProtocolRejected {
        supported: Vec<String>,
    },
    /// Phase 47 — response to a [`FrontendMessage::Query`]. The `id`
    /// echoes the query's correlation id so the frontend can match
    /// async responses without bookkeeping.
    QueryResponse {
        id: String,
        payload: QueryResponsePayload,
    },
    /// Phase 60 — response to [`FrontendMessage::RevertPersonaDelta`].
    /// `ok = true` on a successful append + shared-state recompute;
    /// `ok = false` with `error` populated on failure (unknown
    /// target_delta_id, storage error, lock poisoning).
    PersonaRevertResolved {
        id: String,
        ok: bool,
        /// Sequence number of the appended revert delta on success;
        /// `None` on failure.
        seq: Option<u64>,
        error: Option<String>,
    },
    /// Phase 65 — response to [`FrontendMessage::ImportPersonaChain`].
    /// On success carries `deltas_imported` (count from the
    /// request, useful for the CLI's tally output) and
    /// `final_chain_seq` (the last seq in the new chain).
    /// On failure carries `error` describing what went wrong
    /// (conflict without force, validation failure, append
    /// error mid-stream).
    PersonaImportResolved {
        id: String,
        ok: bool,
        /// On success: `{deltas_imported, final_chain_seq}` per
        /// Phase 65 Q4(a). `None` on failure.
        success: Option<PersonaImportSuccess>,
        error: Option<String>,
    },
    /// Phase 70 — response to
    /// [`FrontendMessage::ResolvePersonaProposal`]. `ok = true`
    /// on success; `success` carries `{ proposal_status,
    /// applied_seq }` on Approve / ApproveWithEdit (where
    /// `applied_seq` is the persona-chain seq of the appended
    /// delta) or `{ proposal_status: "Rejected", applied_seq:
    /// None }` on Reject. `error` is populated on failure
    /// (unknown proposal id, validation failure of edited op,
    /// invalid status transition, storage error).
    PersonaProposalResolved {
        id: String,
        ok: bool,
        success: Option<PersonaProposalResolveSuccess>,
        error: Option<String>,
    },
    /// Phase 74 — response to
    /// [`FrontendMessage::EvictMemoryTopic`]. `ok = true` with
    /// `deleted` set on success; `ok = false` with `error`
    /// populated when the substrate rejects (empty topic, etc.).
    MemoryEvictResolved {
        id: String,
        ok: bool,
        deleted: Option<u64>,
        error: Option<String>,
    },
    /// Phase 119 — ack for [`FrontendMessage::ApplyProfileHint`].
    /// `ok = true` means the daemon recorded the
    /// `AuditEvent::ProfileHintApplied` entry; `ok = false`
    /// with `error` populated means the audit-log append
    /// failed (the operator's `aivyx.toml` mutation already
    /// landed CLI-side before the IPC fired).
    ProfileHintApplyAcked {
        id: String,
        ok: bool,
        error: Option<String>,
    },
    /// Phase 119 — ack for [`FrontendMessage::ImportRoleDraft`].
    /// Same shape as `ProfileHintApplyAcked`.
    RoleDraftImportAcked {
        id: String,
        ok: bool,
        error: Option<String>,
    },
    /// Phase 69 — broadcast-style Web UI desktop notification.
    /// Fired by [`crate::notify_webui::NotifyWebUiBackend`] and
    /// relayed onto every connected Web UI WebSocket. Distinct
    /// from `StreamEvent` (which is per-session); these are
    /// per-daemon notifications without a session correlation.
    DesktopNotification {
        title: String,
        body: String,
    },
}

/// Phase 70 — success payload for
/// [`DaemonMessage::PersonaProposalResolved`]. `proposal_status`
/// is the new stable label after resolution (`"Approved" |
/// "Rejected"`); `applied_seq` is the persona-chain seq of the
/// appended PersonaDelta on Approve / ApproveWithEdit, or `None`
/// on Reject.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaProposalResolveSuccess {
    pub proposal_status: String,
    pub applied_seq: Option<u64>,
}

/// Phase 65 — success payload for [`DaemonMessage::PersonaImportResolved`].
/// Mirrors the operator-feedback shape requested at Q4(a) sign-off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaImportSuccess {
    /// How many deltas the daemon appended. Equals the length
    /// of the request's `deltas` array on a full import.
    pub deltas_imported: u64,
    /// The seq of the final appended delta. After a successful
    /// import, `final_chain_seq + 1` is the chain's current
    /// length (since seqs are zero-indexed).
    pub final_chain_seq: u64,
}

// ---------------------------------------------------------------------------
// Daemon → Frontend (lifecycle, separate from DaemonMessage per Q4)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonLifecycleEvent {
    DaemonReady { version: String },
    ShuttingDown { reason: String },
    RecoveryNotice {
        lost_sessions: Vec<String>,
        lost_turns: Vec<String>,
        stale_since: u64,
    },
}

// ---------------------------------------------------------------------------
// StreamEventPayload — owned, serializable mirror of core::StreamEvent<'a>
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum StreamEventPayload {
    Text {
        text: String,
    },
    Status {
        status: String,
    },
    ToolCallStarted {
        tool_id: String,
        tool_name: String,
        input: serde_json::Value,
    },
    ToolCallFinished {
        tool_id: String,
        tool_name: String,
        outcome_summary: String,
    },
    ToolOutput {
        tool_id: String,
        tool_name: String,
        chunk: String,
    },
    ApprovalGate {
        mission_id: String,
        gate_id: String,
        reason: String,
        scope: Option<String>,
    },
}

impl StreamEventPayload {
    /// Render this payload to a human-readable CLI string, matching the
    /// format that `render_stream_event(RenderMode::Human, ..)` produces
    /// for the in-process path. This lets a daemon-mode frontend pipe IPC
    /// events through the same rendering code path without converting back
    /// to the borrowed `StreamEvent<'a>` type.
    pub fn render_for_cli(&self) -> String {
        match self {
            StreamEventPayload::Text { text } => text.clone(),
            StreamEventPayload::Status { status } => format!("  ⋯ {status}\n"),
            StreamEventPayload::ToolCallStarted {
                tool_name, input, ..
            } => {
                let input_oneline = serde_json::to_string(input).unwrap_or_default();
                format!("  → {tool_name} {input_oneline}\n")
            }
            StreamEventPayload::ToolCallFinished {
                tool_name,
                outcome_summary,
                ..
            } => format!("  ← {tool_name} {outcome_summary}\n"),
            StreamEventPayload::ToolOutput { chunk, .. } => chunk.clone(),
            StreamEventPayload::ApprovalGate {
                mission_id,
                gate_id,
                reason,
                scope,
            } => {
                let scope_str = scope
                    .as_deref()
                    .map(|s| format!(" (scope: {s})"))
                    .unwrap_or_default();
                format!(
                    "\n  ⚑ APPROVAL GATE [{mission_id}/{gate_id}]: \
                     {reason}{scope_str}\n"
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Framing: encode / decode
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum FrameError {
    PayloadTooLarge(u32),
    IncompleteBuf,
    Utf8(String),
    Json(String),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::PayloadTooLarge(n) => {
                write!(f, "payload size {n} exceeds max {MAX_PAYLOAD_SIZE}")
            }
            FrameError::IncompleteBuf => write!(f, "buffer too short for a complete frame"),
            FrameError::Utf8(e) => write!(f, "payload is not valid UTF-8: {e}"),
            FrameError::Json(e) => write!(f, "JSON parse error: {e}"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Encode a serializable message into a length-prefixed frame.
pub fn encode_frame<T: Serialize>(msg: &T) -> Result<Vec<u8>, FrameError> {
    let json = serde_json::to_vec(msg).map_err(|e| FrameError::Json(e.to_string()))?;
    let len: u32 = json
        .len()
        .try_into()
        .map_err(|_| FrameError::PayloadTooLarge(u32::MAX))?;
    if len > MAX_PAYLOAD_SIZE {
        return Err(FrameError::PayloadTooLarge(len));
    }
    let mut buf = Vec::with_capacity(FRAME_HEADER_LEN + json.len());
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(&json);
    Ok(buf)
}

/// Try to decode one frame from the front of `buf`. On success returns
/// the deserialized message and the number of bytes consumed (header +
/// payload). Returns `Err(IncompleteBuf)` if `buf` does not yet contain
/// a full frame — the caller should read more bytes and retry.
pub fn decode_frame<T: for<'de> Deserialize<'de>>(buf: &[u8]) -> Result<(T, usize), FrameError> {
    if buf.len() < FRAME_HEADER_LEN {
        return Err(FrameError::IncompleteBuf);
    }
    let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
    if len > MAX_PAYLOAD_SIZE {
        return Err(FrameError::PayloadTooLarge(len));
    }
    let total = FRAME_HEADER_LEN + len as usize;
    if buf.len() < total {
        return Err(FrameError::IncompleteBuf);
    }
    let payload = &buf[FRAME_HEADER_LEN..total];
    let text = std::str::from_utf8(payload).map_err(|e| FrameError::Utf8(e.to_string()))?;
    let msg: T = serde_json::from_str(text).map_err(|e| FrameError::Json(e.to_string()))?;
    Ok((msg, total))
}

/// Convenience: decode a frame where the message type is one of the
/// three IPC envelopes. Wraps `decode_frame` with the union type.
///
/// The daemon's receive loop calls `decode_frame::<FrontendMessage>`.
/// The frontend's receive loop needs to demux `DaemonMessage` vs.
/// `DaemonLifecycleEvent` — this enum carries both.
// See `DaemonMessage` — same accreting-`LearningInsights`
// rationale; this enum mirrors its variants.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum DaemonEnvelope {
    // DaemonMessage variants (flattened for serde tag dispatch)
    SessionStarted {
        session_id: String,
    },
    StreamEvent {
        session_id: String,
        event: StreamEventPayload,
    },
    TurnComplete {
        session_id: String,
        outcome: String,
    },
    Error {
        code: String,
        message: String,
    },
    // Mission variants (Phase 21)
    MissionCreated {
        mission_id: String,
    },
    MissionStateChanged {
        mission_id: String,
        state: String,
    },
    GateResolved {
        mission_id: String,
        gate_id: String,
        approved: bool,
    },
    // DaemonLifecycleEvent variants
    DaemonReady {
        version: String,
    },
    ShuttingDown {
        reason: String,
    },
    RecoveryNotice {
        lost_sessions: Vec<String>,
        lost_turns: Vec<String>,
        stale_since: u64,
    },
    // Protocol negotiation variants (Phase 41 Task 5)
    ProtocolAccepted {
        version: String,
    },
    ProtocolRejected {
        supported: Vec<String>,
    },
    // Phase 47 — inspection query response.
    QueryResponse {
        id: String,
        payload: QueryResponsePayload,
    },
    // Phase 60 — Persona revert resolution.
    PersonaRevertResolved {
        id: String,
        ok: bool,
        seq: Option<u64>,
        error: Option<String>,
    },
    // Phase 65 — Persona import resolution.
    PersonaImportResolved {
        id: String,
        ok: bool,
        success: Option<PersonaImportSuccess>,
        error: Option<String>,
    },
    // Phase 69 — Web UI desktop notification (broadcast).
    DesktopNotification {
        title: String,
        body: String,
    },
    // Phase 70 — Persona proposal resolution result.
    PersonaProposalResolved {
        id: String,
        ok: bool,
        success: Option<PersonaProposalResolveSuccess>,
        error: Option<String>,
    },
    // Phase 74 — memory eviction resolution result.
    MemoryEvictResolved {
        id: String,
        ok: bool,
        deleted: Option<u64>,
        error: Option<String>,
    },
    // Phase 119 — ProfileHint apply ack.
    ProfileHintApplyAcked {
        id: String,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    // Phase 119 — RoleDraft import ack.
    RoleDraftImportAcked {
        id: String,
        ok: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- FrontendMessage round-trip ----

    #[test]
    fn frontend_message_round_trips() {
        let cases = vec![
            FrontendMessage::StartSession {
                role: Some("coder".into()),
                frontend_type: Some(FrontendType::Local),
            },
            FrontendMessage::StartSession { role: None, frontend_type: None },
            FrontendMessage::StartSession {
                role: None,
                frontend_type: Some(FrontendType::Telegram),
            },
            FrontendMessage::StartSession {
                role: None,
                frontend_type: Some(FrontendType::Web),
            },
            FrontendMessage::SubmitInput {
                session_id: "abc-123".into(),
                text: "hello world".into(),
                mission_id: None,
                attachments: vec![],
            },
            FrontendMessage::CancelTurn {
                session_id: "abc-123".into(),
            },
            FrontendMessage::ResolveGate {
                mission_id: "m-001".into(),
                gate_id: "g-001".into(),
                approved: true,
            },
            FrontendMessage::ResolveGate {
                mission_id: "m-001".into(),
                gate_id: "g-002".into(),
                approved: false,
            },
            FrontendMessage::Disconnect,
            FrontendMessage::Shutdown,
            FrontendMessage::ProtocolNegotiation {
                version: "0.1".into(),
            },
            // Phase 47 — Query variants.
            FrontendMessage::Query {
                id: "q-001".into(),
                payload: QueryPayload::ListSessions,
            },
            FrontendMessage::Query {
                id: "q-002".into(),
                payload: QueryPayload::ListMissions,
            },
            FrontendMessage::Query {
                id: "q-003".into(),
                payload: QueryPayload::GetMission {
                    mission_id: "m-abc".into(),
                },
            },
            FrontendMessage::Query {
                id: "q-004".into(),
                payload: QueryPayload::ListAuditEntries {
                    from_seq: 0,
                    limit: 100,
                },
            },
            FrontendMessage::Query {
                id: "q-005".into(),
                payload: QueryPayload::VerifyAuditChain,
            },
            // Phase 58 — Profile inspection query.
            FrontendMessage::Query {
                id: "q-006".into(),
                payload: QueryPayload::GetProfile,
            },
            // Phase 60 — Persona inspection queries.
            FrontendMessage::Query {
                id: "q-007".into(),
                payload: QueryPayload::GetEffectivePersona,
            },
            FrontendMessage::Query {
                id: "q-008".into(),
                payload: QueryPayload::ListPersonaDeltas {
                    from_seq: 0,
                    limit: 50,
                },
            },
            // Phase 60 — operator-initiated Persona revert.
            FrontendMessage::RevertPersonaDelta {
                id: "rv-1".into(),
                target_delta_id: "pd-abc123".into(),
            },
            // Phase 70 — Persona proposal queries.
            FrontendMessage::Query {
                id: "q-100".into(),
                payload: QueryPayload::ListPersonaProposals {
                    status_filter: "pending".into(),
                    limit: 50,
                },
            },
            FrontendMessage::Query {
                id: "q-101".into(),
                payload: QueryPayload::GetPersonaProposal {
                    proposal_id: "pp-001".into(),
                },
            },
            // Phase 73 — notification history queries.
            FrontendMessage::Query {
                id: "q-200".into(),
                payload: QueryPayload::ListNotificationHistory {
                    from_seq: 0,
                    limit: 100,
                    target_filter: None,
                },
            },
            FrontendMessage::Query {
                id: "q-201".into(),
                payload: QueryPayload::ListNotificationHistory {
                    from_seq: 50,
                    limit: 25,
                    target_filter: Some("phone".into()),
                },
            },
            // Phase 74 — memory inspection queries.
            FrontendMessage::Query {
                id: "q-300".into(),
                payload: QueryPayload::ListMemoryTopics,
            },
            FrontendMessage::Query {
                id: "q-301".into(),
                payload: QueryPayload::GetMemoryTopicEntries {
                    topic: "notes".into(),
                    limit: 16,
                },
            },
            FrontendMessage::Query {
                id: "q-302".into(),
                payload: QueryPayload::SearchMemory {
                    query: "foo".into(),
                    limit: 20,
                    semantic: true,
                },
            },
            FrontendMessage::Query {
                id: "q-303".into(),
                payload: QueryPayload::GetLearningInsights {
                    window_secs: Some(86_400),
                },
            },
            // Phase 74 — operator-initiated memory eviction.
            FrontendMessage::EvictMemoryTopic {
                id: "ev-1".into(),
                topic: "stale-notes".into(),
            },
            // Phase 70 — Persona proposal resolutions.
            FrontendMessage::ResolvePersonaProposal {
                id: "rs-1".into(),
                proposal_id: "pp-001".into(),
                resolution: PersonaProposalResolution::Approve,
            },
            FrontendMessage::ResolvePersonaProposal {
                id: "rs-2".into(),
                proposal_id: "pp-002".into(),
                resolution: PersonaProposalResolution::ApproveWithEdit {
                    edited_op: crate::persona::ProposedPersonaDelta {
                        category: crate::persona::PersonaDeltaCategory::BehavioralPreferences,
                        op: crate::persona::PersonaDeltaOp::AppendList {
                            value: "operator-edited preference".into(),
                        },
                        reason: None,
                        supersedes_proposal_id: None,
                    },
                },
            },
            FrontendMessage::ResolvePersonaProposal {
                id: "rs-3".into(),
                proposal_id: "pp-003".into(),
                resolution: PersonaProposalResolution::Reject {
                    reason: Some("not safe".into()),
                },
            },
            FrontendMessage::ResolvePersonaProposal {
                id: "rs-4".into(),
                proposal_id: "pp-004".into(),
                resolution: PersonaProposalResolution::Reject { reason: None },
            },
        ];
        for msg in cases {
            let frame = encode_frame(&msg).expect("encode");
            let (decoded, consumed): (FrontendMessage, _) =
                decode_frame(&frame).expect("decode");
            assert_eq!(decoded, msg);
            assert_eq!(consumed, frame.len());
        }
    }

    // ---- DaemonMessage round-trip ----

    #[test]
    fn daemon_message_round_trips() {
        let cases = vec![
            DaemonMessage::SessionStarted {
                session_id: "s1".into(),
            },
            DaemonMessage::StreamEvent {
                session_id: "s1".into(),
                event: StreamEventPayload::Text {
                    text: "hello".into(),
                },
            },
            DaemonMessage::StreamEvent {
                session_id: "s1".into(),
                event: StreamEventPayload::ToolCallStarted {
                    tool_id: "t1".into(),
                    tool_name: "fs.read".into(),
                    input: serde_json::json!({"path": "/tmp/test"}),
                },
            },
            DaemonMessage::TurnComplete {
                session_id: "s1".into(),
                outcome: "completed".into(),
            },
            DaemonMessage::Error {
                code: "internal".into(),
                message: "something broke".into(),
            },
            DaemonMessage::MissionCreated {
                mission_id: "m-001".into(),
            },
            DaemonMessage::MissionStateChanged {
                mission_id: "m-001".into(),
                state: "Running".into(),
            },
            DaemonMessage::GateResolved {
                mission_id: "m-001".into(),
                gate_id: "g-001".into(),
                approved: true,
            },
            DaemonMessage::ProtocolAccepted {
                version: "0.1".into(),
            },
            DaemonMessage::ProtocolRejected {
                supported: vec!["0.1".into(), "0.2".into()],
            },
            // Phase 47 — QueryResponse variants.
            DaemonMessage::QueryResponse {
                id: "q-001".into(),
                payload: QueryResponsePayload::ListSessions {
                    sessions: vec![
                        SessionSummary { session_id: "s-1".into() },
                        SessionSummary { session_id: "s-2".into() },
                    ],
                },
            },
            DaemonMessage::QueryResponse {
                id: "q-002".into(),
                payload: QueryResponsePayload::ListSessions { sessions: vec![] },
            },
            DaemonMessage::QueryResponse {
                id: "q-003".into(),
                payload: QueryResponsePayload::QueryError {
                    code: "internal".into(),
                    message: "store unavailable".into(),
                },
            },
            DaemonMessage::QueryResponse {
                id: "q-004".into(),
                payload: QueryResponsePayload::ListMissions {
                    missions: vec![MissionSummary {
                        mission_id: "m-1".into(),
                        role_name: "default".into(),
                        description: "test".into(),
                        state: "Running".into(),
                        has_pending_gate: false,
                        created_at: 1,
                        updated_at: 2,
                    }],
                },
            },
            DaemonMessage::QueryResponse {
                id: "q-005".into(),
                payload: QueryResponsePayload::GetMission { mission: None },
            },
            DaemonMessage::QueryResponse {
                id: "q-007".into(),
                payload: QueryResponsePayload::ListAuditEntries {
                    entries: vec![AuditEntrySummary {
                        seq: 0,
                        appended_at_unix_ms: 1_700_000_000_000,
                        event_type: "TurnStarted".into(),
                        event: serde_json::json!({"type": "TurnStarted"}),
                        mac_hex: "deadbeef".repeat(8),
                    }],
                    total_len: 1,
                },
            },
            DaemonMessage::QueryResponse {
                id: "q-008".into(),
                payload: QueryResponsePayload::VerifyAuditChain {
                    ok: true,
                    entries_verified: 42,
                    error: None,
                },
            },
            DaemonMessage::QueryResponse {
                id: "q-009".into(),
                payload: QueryResponsePayload::VerifyAuditChain {
                    ok: false,
                    entries_verified: 0,
                    error: Some("chain broken at seq 3".into()),
                },
            },
            // Phase 58 — Profile inspection response (default snapshot,
            // injection disabled).
            DaemonMessage::QueryResponse {
                id: "q-010".into(),
                payload: QueryResponsePayload::GetProfile {
                    profile: ProfileSummary {
                        assistant_name: "Aivyx".into(),
                        assistant_name_source: "default".into(),
                        operator_profile: None,
                        communication_style: None,
                        primary_use_cases: vec![],
                        behavioral_preferences: vec![],
                        behavioral_constraints: vec![],
                        injection_enabled: false,
                    },
                },
            },
            // Phase 58 — Profile inspection response with operator-
            // declared content (injection enabled).
            DaemonMessage::QueryResponse {
                id: "q-011".into(),
                payload: QueryResponsePayload::GetProfile {
                    profile: ProfileSummary {
                        assistant_name: "Codex".into(),
                        assistant_name_source: "toml".into(),
                        operator_profile: Some("Senior Rust engineer".into()),
                        communication_style: Some("terse".into()),
                        primary_use_cases: vec!["Rust systems".into()],
                        behavioral_preferences: vec!["prefer integration tests".into()],
                        behavioral_constraints: vec!["never auto-commit".into()],
                        injection_enabled: true,
                    },
                },
            },
            // Phase 60 — Persona inspection responses.
            DaemonMessage::QueryResponse {
                id: "q-012".into(),
                payload: QueryResponsePayload::GetEffectivePersona {
                    persona: EffectivePersonaSummary::default(),
                },
            },
            DaemonMessage::QueryResponse {
                id: "q-013".into(),
                payload: QueryResponsePayload::GetEffectivePersona {
                    persona: EffectivePersonaSummary {
                        behavioral_preferences: vec!["always cite sources".into()],
                        learned_context: vec!["operator uses Vim".into()],
                        is_non_empty: true,
                        ..Default::default()
                    },
                },
            },
            DaemonMessage::QueryResponse {
                id: "q-014".into(),
                payload: QueryResponsePayload::ListPersonaDeltas {
                    entries: vec![],
                    total_len: 0,
                },
            },
            DaemonMessage::QueryResponse {
                id: "q-015".into(),
                payload: QueryResponsePayload::ListPersonaDeltas {
                    entries: vec![PersonaDeltaSummary {
                        seq: 0,
                        delta_id: "pd-abc".into(),
                        proposed_at_unix_ms: 1_715_000_000_000,
                        approved_at_unix_ms: 1_715_000_060_000,
                        proposal_id: "rp-1".into(),
                        category: "BehavioralPreferences".into(),
                        op: serde_json::json!({
                            "kind": "AppendList",
                            "value": "prefer terse"
                        }),
                        mac_hex: "0".repeat(64),
                    }],
                    total_len: 1,
                },
            },
            // Phase 60 — revert resolution responses.
            DaemonMessage::PersonaRevertResolved {
                id: "rv-1".into(),
                ok: true,
                seq: Some(2),
                error: None,
            },
            DaemonMessage::PersonaRevertResolved {
                id: "rv-2".into(),
                ok: false,
                seq: None,
                error: Some("no persona delta found with id `pd-missing`".into()),
            },
            // Phase 70 — proposal resolution responses.
            DaemonMessage::PersonaProposalResolved {
                id: "rs-1".into(),
                ok: true,
                success: Some(PersonaProposalResolveSuccess {
                    proposal_status: "Approved".into(),
                    applied_seq: Some(42),
                }),
                error: None,
            },
            DaemonMessage::PersonaProposalResolved {
                id: "rs-2".into(),
                ok: true,
                success: Some(PersonaProposalResolveSuccess {
                    proposal_status: "Rejected".into(),
                    applied_seq: None,
                }),
                error: None,
            },
            DaemonMessage::PersonaProposalResolved {
                id: "rs-3".into(),
                ok: false,
                success: None,
                error: Some("unknown proposal id `pp-missing`".into()),
            },
            // Phase 74 — memory eviction resolution.
            DaemonMessage::MemoryEvictResolved {
                id: "ev-1".into(),
                ok: true,
                deleted: Some(7),
                error: None,
            },
            DaemonMessage::MemoryEvictResolved {
                id: "ev-2".into(),
                ok: false,
                deleted: None,
                error: Some("memory topic must be non-empty".into()),
            },
            // Phase 74 — memory query responses.
            DaemonMessage::QueryResponse {
                id: "q-300".into(),
                payload: QueryResponsePayload::ListMemoryTopics {
                    topics: vec!["notes".into(), "project/x".into()],
                },
            },
            DaemonMessage::QueryResponse {
                id: "q-301".into(),
                payload: QueryResponsePayload::GetMemoryTopicEntries {
                    entries: vec![MemoryEntrySummary {
                        topic: "notes".into(),
                        body: "remember the milk".into(),
                        seq: 3,
                        created_at_secs: 1_715_000_000,
                        last_read_at_secs: 1_715_000_500,
                    }],
                },
            },
            DaemonMessage::QueryResponse {
                id: "q-302".into(),
                payload: QueryResponsePayload::SearchMemory {
                    matches: vec![MemoryEntrySummary {
                        topic: "project/x".into(),
                        body: "the foo subsystem".into(),
                        seq: 9,
                        created_at_secs: 1_715_001_000,
                        last_read_at_secs: 0,
                    }],
                    fell_back_to_keyword: true,
                },
            },
            DaemonMessage::QueryResponse {
                id: "q-303".into(),
                payload: QueryResponsePayload::LearningInsights {
                    digest: crate::recall_insights::LearningDigest {
                        window_secs: 86_400,
                        recalls_total: 12,
                        recalls_scored: 9,
                        promoted: 4,
                        not_promoted: 2,
                        top_helpful: vec![("project/x".into(), 5.0)],
                        top_unhelpful: vec![("scratch".into(), -3.0)],
                        proposals_in_window: 1,
                        judgment_signal: None,
                    },
                    proposals: vec![
                        crate::recall_insights::ProposalProvenance {
                            proposal_id: "recall-fb:project/x".into(),
                            topic: "project/x".into(),
                            status: "pending".into(),
                            net_score: 5.0,
                            reason: Some("net +5".into()),
                            contributing: vec![
                                crate::recall_insights::ContributingTurn {
                                    ts_secs: 1_715_000_000,
                                    outcome_kind: Some(
                                        "completed".into(),
                                    ),
                                    signal: Some(1.0),
                                    seqs: vec![9],
                                },
                            ],
                        },
                    ],
                    persona_selection: Some(
                        crate::persona_context::PersonaSelectionStat {
                            ts_secs: 1_715_002_000,
                            selected: 6,
                            total: 20,
                        },
                    ),
                    proactive: Some(
                        crate::proactive_detect::ProactiveStat {
                            ts_secs: 1_715_003_000,
                            surfaced: vec![
                                crate::proactive_detect::ProactiveSurfaced {
                                    kind: crate::proactive_detect::ProactiveKind::DueReminder,
                                    topic: "rem".into(),
                                    reason: "reminder in 'rem' was due 2h ago".into(),
                                },
                            ],
                            deduped: 1,
                            capped: 0,
                        },
                    ),
                    persona_lifecycle: Some(
                        crate::persona_lifecycle::PersonaLifecycleStat {
                            ts_secs: 1_715_004_000,
                            proposed: vec![
                                crate::persona_lifecycle::PersonaLifecycleProposed {
                                    kind: "consolidate".into(),
                                    category: crate::persona_lifecycle::SoftCategory::LearnedContext,
                                    value: "dup a".into(),
                                    reason: "2 near-duplicate learned_context facets (cosine > 0.92)".into(),
                                },
                            ],
                            deduped: 1,
                        },
                    ),
                    accumulated_helpfulness: Some(
                        crate::helpfulness_ledger::AccumulatedHelpfulness {
                            top_helpful: vec![
                                crate::helpfulness_ledger::TopicScore {
                                    topic: "project/x".into(),
                                    score: 12.5,
                                    samples: 7,
                                },
                            ],
                            top_unhelpful: vec![
                                crate::helpfulness_ledger::TopicScore {
                                    topic: "scratch".into(),
                                    score: -4.0,
                                    samples: 3,
                                },
                            ],
                        },
                    ),
                    cooccurrence: Some(
                        crate::cooccurrence_ledger::CooccurrencePatterns {
                            top_pairs: vec![
                                crate::cooccurrence_ledger::PairScore {
                                    a: "deploy".into(),
                                    b: "rollback".into(),
                                    score: 8.0,
                                    samples: 5,
                                },
                            ],
                        },
                    ),
                    cluster_recall: Some(
                        crate::memory_recall::RecallClusterStat {
                            ts_secs: 1_715_005_000,
                            injected: 1,
                            pairs: vec![(
                                "deploy".into(),
                                "rollback".into(),
                            )],
                        },
                    ),
                    persona_consolidation: Some(
                        crate::persona_consolidation::PersonaConsolidationStat {
                            ts_secs: 1_715_005_500,
                            filed: 1,
                            deduped: 0,
                            skipped_unhelpful: 0,
                            llm_unavailable: false,
                            pairs: vec![(
                                "deploy".into(),
                                "rollback".into(),
                            )],
                            superseded: 0,
                        },
                    ),
                    accumulated_corrections: Some(
                        crate::correction_ledger::AccumulatedCorrections {
                            top_corrected: vec![
                                crate::correction_ledger::TopicCorrections {
                                    topic: "deploy".into(),
                                    count: 3.0,
                                    samples: 2,
                                },
                            ],
                        },
                    ),
                    correction_consolidation: Some(
                        crate::correction_consolidation::CorrectionConsolidationStat {
                            ts_secs: 1_715_005_550,
                            filed: 1,
                            llm_unavailable: false,
                            topics: vec!["deploy".into()],
                        },
                    ),
                    correction_judgment: Some(
                        crate::correction_judgment::CorrectionJudgmentStat {
                            ts_secs: 1_715_005_560,
                            judged: 4,
                            rework: 2,
                            praise: 1,
                            unrelated: 1,
                            structural_fallback: 1,
                            llm_unavailable: false,
                        },
                    ),
                    recall_judgment: Some(
                        crate::recall_judgment::RecallJudgmentStat {
                            ts_secs: 1_715_005_600,
                            judged: 3,
                            used: 1,
                            irrelevant: 1,
                            hurt: 1,
                            skipped: 0,
                            llm_unavailable: false,
                            pairs: vec![
                                ("deploy".into(),
                                 crate::recall_judgment::RecallJudgment::Used),
                                ("rollback".into(),
                                 crate::recall_judgment::RecallJudgment::Irrelevant),
                                ("auth".into(),
                                 crate::recall_judgment::RecallJudgment::Hurt),
                            ],
                        },
                    ),
                    cadence: vec![
                        (
                            "nightly".into(),
                            crate::reflection_scheduler::RecentReflectionStat {
                                fired: 6,
                                skipped: 1,
                            },
                        ),
                    ],
                },
            },
            // Phase 70 — proposal query responses.
            DaemonMessage::QueryResponse {
                id: "q-100".into(),
                payload: QueryResponsePayload::ListPersonaProposals {
                    proposals: vec![PersonaProposalSummary {
                        id: "pp-001".into(),
                        proposed_at_unix_ms: 1_715_000_000_000,
                        source_reflection_session_id: "ses-abc".into(),
                        status: "Pending".into(),
                        category: "BehavioralPreferences".into(),
                        proposed_op: serde_json::json!({
                            "kind": "AppendList",
                            "value": "prefer terse",
                        }),
                        proposed_reason: Some(
                            "operator confirmed 3 turns".into(),
                        ),
                        applied_op: None,
                        applied_seq: None,
                        rejected_reason: None,
                        resolved_at_unix_ms: None,
                        supersedes_proposal_id: None,
                    }],
                    total_len: 1,
                },
            },
            DaemonMessage::QueryResponse {
                id: "q-101".into(),
                payload: QueryResponsePayload::GetPersonaProposal {
                    proposal: None,
                },
            },
            // Phase 73 — notification history response.
            DaemonMessage::QueryResponse {
                id: "q-200".into(),
                payload: QueryResponsePayload::ListNotificationHistory {
                    entries: vec![NotificationHistoryEntry {
                        seq: 42,
                        dispatched_at_unix_ms: 1_715_000_000_000,
                        session_id: "ses-abc".into(),
                        trigger_kind: "Cron".into(),
                        trigger_id: "morning-summary".into(),
                        target_name: "phone".into(),
                        outcome_kind: "delivered".into(),
                        outcome_detail: String::new(),
                    }],
                    total_len: 1,
                },
            },
            DaemonMessage::QueryResponse {
                id: "q-006".into(),
                payload: QueryResponsePayload::GetMission {
                    mission: Some(MissionDetail {
                        mission_id: "m-1".into(),
                        role_name: "default".into(),
                        description: "test".into(),
                        state: "GatePending".into(),
                        gates: vec![GateSummary {
                            gate_id: "g-1".into(),
                            reason: "approve please".into(),
                            scope: Some("shell.exec".into()),
                            state: "Pending".into(),
                            created_at: 10,
                            resolved_at: None,
                        }],
                        created_at: 1,
                        updated_at: 5,
                    }),
                },
            },
            // Phase 69 — Web UI desktop notification broadcast.
            DaemonMessage::DesktopNotification {
                title: "Build complete".into(),
                body: "aivyx-core: 1232 tests passed.".into(),
            },
            DaemonMessage::DesktopNotification {
                title: "Trigger fired".into(),
                body: String::new(),
            },
        ];
        for msg in cases {
            let frame = encode_frame(&msg).expect("encode");
            let (decoded, consumed): (DaemonMessage, _) =
                decode_frame(&frame).expect("decode");
            assert_eq!(decoded, msg);
            assert_eq!(consumed, frame.len());
        }
    }

    // ---- DaemonEnvelope must decode QueryResponse from a DaemonMessage frame ----

    #[test]
    fn daemon_envelope_decodes_query_response() {
        let msg = DaemonMessage::QueryResponse {
            id: "q-1".into(),
            payload: QueryResponsePayload::ListSessions {
                sessions: vec![SessionSummary { session_id: "abc".into() }],
            },
        };
        let frame = encode_frame(&msg).expect("encode");
        let (envelope, consumed): (DaemonEnvelope, _) =
            decode_frame(&frame).expect("decode");
        assert_eq!(consumed, frame.len());
        match envelope {
            DaemonEnvelope::QueryResponse { id, payload } => {
                assert_eq!(id, "q-1");
                match payload {
                    QueryResponsePayload::ListSessions { sessions } => {
                        assert_eq!(sessions.len(), 1);
                        assert_eq!(sessions[0].session_id, "abc");
                    }
                    other => panic!("expected ListSessions, got {other:?}"),
                }
            }
            other => panic!("expected QueryResponse, got {other:?}"),
        }
    }

    // ---- Chapter L (L.5) team-mission IPC round-trip ----

    #[test]
    fn team_mission_queries_round_trip() {
        use aivyx_team::{MissionPlan, Step};

        let plan = MissionPlan::new(
            "ship",
            vec![
                Step::delegate("a", "researcher", "go"),
                Step::human_gate("g", "reviewer", "ok?").after(["a"]),
            ],
        );
        // The request carrying a full MissionPlan survives the frame.
        let req = FrontendMessage::Query {
            id: "tr".into(),
            payload: QueryPayload::TeamRun { plan: plan.clone(), config: None },
        };
        let frame = encode_frame(&req).expect("encode");
        let (decoded, _): (FrontendMessage, _) = decode_frame(&frame).expect("decode");
        assert_eq!(decoded, req, "TeamRun round-trips with its plan");

        // The list response carrying a full record survives the frame.
        let mut record =
            crate::team_mission::TeamMissionRecord::new("m1", "ship", plan);
        record.phase = crate::team_mission::TeamMissionPhase::AwaitingApproval;
        record.pending_gate = Some("g".into());
        let resp = DaemonMessage::QueryResponse {
            id: "tr".into(),
            payload: QueryResponsePayload::TeamMissionList {
                missions: vec![record.clone()],
            },
        };
        let frame = encode_frame(&resp).expect("encode");
        let (env, _): (DaemonEnvelope, _) = decode_frame(&frame).expect("decode");
        match env {
            DaemonEnvelope::QueryResponse { payload, .. } => match payload {
                QueryResponsePayload::TeamMissionList { missions } => {
                    assert_eq!(missions, vec![record]);
                }
                other => panic!("expected TeamMissionList, got {other:?}"),
            },
            other => panic!("expected QueryResponse, got {other:?}"),
        }

        // The resolve response carries the resulting phase.
        let resolved = QueryResponsePayload::TeamGateResolved {
            mission_id: "m1".into(),
            phase: crate::team_mission::TeamMissionPhase::Rejected,
        };
        let frame = encode_frame(&resolved).expect("encode");
        let (back, _): (QueryResponsePayload, _) = decode_frame(&frame).expect("decode");
        assert_eq!(back, resolved);
    }

    // ---- DaemonLifecycleEvent round-trip ----

    #[test]
    fn daemon_lifecycle_event_round_trips() {
        let cases = vec![
            DaemonLifecycleEvent::DaemonReady {
                version: "0.1".into(),
            },
            DaemonLifecycleEvent::ShuttingDown {
                reason: "operator requested".into(),
            },
            DaemonLifecycleEvent::RecoveryNotice {
                lost_sessions: vec!["ses-1".into(), "ses-2".into()],
                lost_turns: vec!["ses-1:turn".into()],
                stale_since: 1713700000,
            },
        ];
        for msg in cases {
            let frame = encode_frame(&msg).expect("encode");
            let (decoded, consumed): (DaemonLifecycleEvent, _) =
                decode_frame(&frame).expect("decode");
            assert_eq!(decoded, msg);
            assert_eq!(consumed, frame.len());
        }
    }

    // ---- Max payload size boundary ----

    #[test]
    fn encode_rejects_oversized_payload() {
        let huge = "x".repeat(MAX_PAYLOAD_SIZE as usize + 1);
        let msg = FrontendMessage::SubmitInput {
            session_id: "s".into(),
            text: huge,
            mission_id: None,
            attachments: vec![],
        };
        let err = encode_frame(&msg).unwrap_err();
        assert!(matches!(err, FrameError::PayloadTooLarge(_)));
    }

    #[test]
    fn decode_rejects_oversized_length_prefix() {
        let mut buf = vec![0u8; 8];
        let bad_len: u32 = MAX_PAYLOAD_SIZE + 1;
        buf[0..4].copy_from_slice(&bad_len.to_be_bytes());
        let err = decode_frame::<FrontendMessage>(&buf).unwrap_err();
        assert!(matches!(err, FrameError::PayloadTooLarge(_)));
    }

    // ---- Incomplete buffer ----

    #[test]
    fn decode_returns_incomplete_for_short_buffer() {
        assert!(matches!(
            decode_frame::<FrontendMessage>(&[0, 0]),
            Err(FrameError::IncompleteBuf)
        ));
        // Header says 10 bytes but only 2 payload bytes present
        let buf = [0, 0, 0, 10, b'h', b'i'];
        assert!(matches!(
            decode_frame::<FrontendMessage>(&buf),
            Err(FrameError::IncompleteBuf)
        ));
    }

    // ---- DaemonEnvelope demux ----

    #[test]
    fn daemon_envelope_demuxes_all_variants() {
        let lifecycle = DaemonLifecycleEvent::DaemonReady {
            version: "0.1".into(),
        };
        let frame = encode_frame(&lifecycle).expect("encode lifecycle");
        let (envelope, _): (DaemonEnvelope, _) = decode_frame(&frame).expect("decode");
        assert!(matches!(envelope, DaemonEnvelope::DaemonReady { .. }));

        let turn = DaemonMessage::TurnComplete {
            session_id: "s1".into(),
            outcome: "done".into(),
        };
        let frame = encode_frame(&turn).expect("encode turn");
        let (envelope, _): (DaemonEnvelope, _) = decode_frame(&frame).expect("decode");
        assert!(matches!(envelope, DaemonEnvelope::TurnComplete { .. }));

        let accepted = DaemonMessage::ProtocolAccepted {
            version: "0.1".into(),
        };
        let frame = encode_frame(&accepted).expect("encode accepted");
        let (envelope, _): (DaemonEnvelope, _) = decode_frame(&frame).expect("decode");
        assert!(matches!(envelope, DaemonEnvelope::ProtocolAccepted { .. }));

        let rejected = DaemonMessage::ProtocolRejected {
            supported: vec!["0.1".into()],
        };
        let frame = encode_frame(&rejected).expect("encode rejected");
        let (envelope, _): (DaemonEnvelope, _) = decode_frame(&frame).expect("decode");
        assert!(matches!(envelope, DaemonEnvelope::ProtocolRejected { .. }));

        let recovery = DaemonLifecycleEvent::RecoveryNotice {
            lost_sessions: vec![],
            lost_turns: vec![],
            stale_since: 0,
        };
        let frame = encode_frame(&recovery).expect("encode recovery");
        let (envelope, _): (DaemonEnvelope, _) = decode_frame(&frame).expect("decode");
        assert!(matches!(envelope, DaemonEnvelope::RecoveryNotice { .. }));

        // Phase 69 — DesktopNotification demux from a DaemonMessage frame.
        let desktop = DaemonMessage::DesktopNotification {
            title: "hello".into(),
            body: "world".into(),
        };
        let frame = encode_frame(&desktop).expect("encode desktop");
        let (envelope, _): (DaemonEnvelope, _) = decode_frame(&frame).expect("decode");
        match envelope {
            DaemonEnvelope::DesktopNotification { title, body } => {
                assert_eq!(title, "hello");
                assert_eq!(body, "world");
            }
            other => panic!("expected DesktopNotification, got {other:?}"),
        }
    }

    // ---- StreamEventPayload covers all variants ----

    #[test]
    fn stream_event_payload_all_variants_round_trip() {
        let cases = vec![
            StreamEventPayload::Text {
                text: "hello".into(),
            },
            StreamEventPayload::Status {
                status: "thinking...".into(),
            },
            StreamEventPayload::ToolCallStarted {
                tool_id: "id-1".into(),
                tool_name: "memory.read".into(),
                input: serde_json::json!({"topic": "notes"}),
            },
            StreamEventPayload::ToolCallFinished {
                tool_id: "id-1".into(),
                tool_name: "memory.read".into(),
                outcome_summary: "3 entries".into(),
            },
            StreamEventPayload::ToolOutput {
                tool_id: "id-1".into(),
                tool_name: "web.fetch".into(),
                chunk: "<html>...".into(),
            },
            StreamEventPayload::ApprovalGate {
                mission_id: "m-001".into(),
                gate_id: "g-001".into(),
                reason: "deploy to production?".into(),
                scope: Some("shell.exec".into()),
            },
            StreamEventPayload::ApprovalGate {
                mission_id: "m-002".into(),
                gate_id: "g-010".into(),
                reason: "proceed with analysis?".into(),
                scope: None,
            },
        ];
        for payload in cases {
            let json = serde_json::to_string(&payload).expect("serialize");
            let back: StreamEventPayload = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, payload);
        }
    }

    // ---- Protocol version constant ----

    #[test]
    fn protocol_version_matches_spec() {
        assert_eq!(PROTOCOL_VERSION, "0.1");
    }

    // ---- Socket path resolver ----

    #[test]
    fn default_socket_path_uses_xdg_runtime_dir_when_set() {
        // We can't mutate env in parallel tests safely, so just verify
        // the function returns Ok when HOME is set (which it always is
        // in CI and dev). The exact path depends on the environment.
        let result = default_socket_path();
        assert!(
            result.is_ok(),
            "default_socket_path must succeed when HOME is set: {result:?}"
        );
        let path = result.unwrap();
        assert!(
            path.ends_with("daemon.sock"),
            "path must end with daemon.sock: {path:?}"
        );
    }

    #[test]
    fn default_pid_path_is_sibling_of_socket_path() {
        let pid = default_pid_path();
        assert!(
            pid.is_ok(),
            "default_pid_path must succeed when HOME is set: {pid:?}"
        );
        let path = pid.unwrap();
        assert!(
            path.ends_with("daemon.pid"),
            "path must end with daemon.pid: {path:?}"
        );
        let sock = default_socket_path().unwrap();
        assert_eq!(
            path.parent(),
            sock.parent(),
            "pid and socket paths must share the same parent directory"
        );
    }

    // ---- render_for_cli ----

    #[test]
    fn render_for_cli_text_passes_through() {
        let payload = StreamEventPayload::Text {
            text: "hello world".into(),
        };
        assert_eq!(payload.render_for_cli(), "hello world");
    }

    #[test]
    fn render_for_cli_tool_call_started_includes_arrow_and_name() {
        let payload = StreamEventPayload::ToolCallStarted {
            tool_id: "id".into(),
            tool_name: "fs.read".into(),
            input: serde_json::json!({"path": "/tmp"}),
        };
        let rendered = payload.render_for_cli();
        assert!(rendered.starts_with("  → fs.read"), "got: {rendered}");
        assert!(rendered.contains("/tmp"), "got: {rendered}");
    }

    #[test]
    fn render_for_cli_tool_call_finished_includes_arrow_and_summary() {
        let payload = StreamEventPayload::ToolCallFinished {
            tool_id: "id".into(),
            tool_name: "memory.read".into(),
            outcome_summary: "3 entries".into(),
        };
        let rendered = payload.render_for_cli();
        assert!(rendered.starts_with("  ← memory.read"), "got: {rendered}");
        assert!(rendered.contains("3 entries"), "got: {rendered}");
    }

    #[test]
    fn render_for_cli_approval_gate_with_scope() {
        let payload = StreamEventPayload::ApprovalGate {
            mission_id: "m-001".into(),
            gate_id: "g-001".into(),
            reason: "deploy to production?".into(),
            scope: Some("shell.exec".into()),
        };
        let rendered = payload.render_for_cli();
        assert!(rendered.contains("APPROVAL GATE"), "got: {rendered}");
        assert!(rendered.contains("m-001/g-001"), "got: {rendered}");
        assert!(rendered.contains("deploy to production?"), "got: {rendered}");
        assert!(rendered.contains("scope: shell.exec"), "got: {rendered}");
    }

    #[test]
    fn render_for_cli_approval_gate_without_scope() {
        let payload = StreamEventPayload::ApprovalGate {
            mission_id: "m-002".into(),
            gate_id: "g-010".into(),
            reason: "proceed?".into(),
            scope: None,
        };
        let rendered = payload.render_for_cli();
        assert!(rendered.contains("m-002/g-010"), "got: {rendered}");
        assert!(!rendered.contains("scope:"), "got: {rendered}");
    }

    // ---- Phase 45 — IpcAttachment ----

    #[test]
    fn ipc_submit_with_attachment_roundtrip() {
        let msg = FrontendMessage::SubmitInput {
            session_id: "s1".into(),
            text: "describe this".into(),
            mission_id: None,
            attachments: vec![IpcAttachment {
                media_type: "image/png".into(),
                data_base64: "iVBORw0KGgo=".into(),
                filename: Some("screenshot.png".into()),
            }],
        };
        let frame = encode_frame(&msg).expect("encode");
        let (decoded, consumed): (FrontendMessage, _) = decode_frame(&frame).expect("decode");
        assert_eq!(decoded, msg);
        assert_eq!(consumed, frame.len());
    }

    #[test]
    fn ipc_submit_no_attachment_backwards_compat() {
        // Simulate an old client that omits the `attachments` field entirely.
        let json = r#"{"type":"SubmitInput","session_id":"s1","text":"hello"}"#;
        let msg: FrontendMessage = serde_json::from_str(json).expect("parse");
        match msg {
            FrontendMessage::SubmitInput {
                text, attachments, ..
            } => {
                assert_eq!(text, "hello");
                assert!(attachments.is_empty(), "default should be empty vec");
            }
            other => panic!("expected SubmitInput, got {other:?}"),
        }
    }

    #[test]
    fn ipc_attachment_serde_roundtrip() {
        let att = IpcAttachment {
            media_type: "image/jpeg".into(),
            data_base64: "AAAA".into(),
            filename: None,
        };
        let json = serde_json::to_string(&att).expect("ser");
        let back: IpcAttachment = serde_json::from_str(&json).expect("de");
        assert_eq!(back, att);
    }
}
