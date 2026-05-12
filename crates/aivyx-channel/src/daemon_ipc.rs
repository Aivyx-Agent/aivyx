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
}

/// Response payload mirroring [`QueryPayload`]. Wrapped in
/// [`DaemonMessage::QueryResponse`] with the same correlation `id`
/// the query was sent with.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
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
    /// Response to [`QueryPayload::ListPersonaDeltas`]. Phase 60
    /// — paginated page of approved deltas. `total_len` is the full
    /// chain length so the frontend knows when to stop paginating.
    ListPersonaDeltas {
        entries: Vec<PersonaDeltaSummary>,
        total_len: u64,
    },
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
}

// ---------------------------------------------------------------------------
// Daemon → Frontend (turn-loop traffic)
// ---------------------------------------------------------------------------

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
