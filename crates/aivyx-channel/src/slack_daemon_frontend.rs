//! Phase 111 Task 5 — Daemon-mode Slack multi-channel pump.
//!
//! Mirrors the Phase 19 `telegram_daemon_frontend.rs` and the
//! Phase 111 Task 3 `discord_daemon_frontend.rs` patterns.
//! Drives a multi-channel Slack frontend over the daemon IPC
//! channel: one outer `next_message` loop pumps the Socket Mode
//! WebSocket (via the now-live `SlackMorphismTransport` from
//! Task 4), per-partition routing fans out to inner mailbox
//! tasks, each inner task submits turns through a
//! `DaemonSession` instead of constructing an agent.
//!
//! ## Three data points for the daemon-frontend shape
//!
//! Telegram (Phase 19), Discord (Phase 111 Task 3), and Slack
//! (Phase 111 Task 5) now all run the same daemon-frontend
//! shape. The Q-block at Phase 111 Task 2 sign-off (Q2a —
//! mirror Phase 19 exactly) anticipated this: if Slack
//! converges on the same shape, the **shared substrate
//! question** raised in the open doc gets answered
//! affirmatively at three data points. A future small
//! refactor could lift the per-route routing logic into a
//! shared helper; Phase 111 ships three siblings per the
//! adapter-pattern doc's "extract only when forced" rule.
//!
//! ## Partition key — `(team_id, channel_id)` vs. `u64` /
//! `i64`
//!
//! Slack's partition key is a string (`"{team_id}:{channel_id}"`
//! per Phase 108 Q3a) where Discord and Telegram used `u64`
//! and `i64` respectively. The routing `HashMap` is keyed
//! on `String` here; the shape is otherwise identical.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use aivyx_core::{
    CancellationToken, ChannelContext, ChannelError, ChannelPlatform, SessionId, StreamEvent,
    TurnOutcome,
};
use aivyx_slack::transport::{
    IncomingMessage, OutgoingMessage, SlackMorphismTransport, SlackTransport,
};

use crate::daemon_client::{self, DaemonSession};
use crate::daemon_ipc::{FrontendType, StreamEventPayload};
use crate::daemon_server::DaemonError;
use crate::gate_command;
use crate::team_command::{self, TeamCommand};
use crate::team_dispatch;
use crate::team_trigger_state::{
    check_and_record_trigger, parse_confirm_reply, ConfirmReply, PendingTrigger,
};
use std::time::Instant;

// ---------------------------------------------------------------------------
// SlackDaemonChannel — identity stub for the daemon's ChannelFactory
// ---------------------------------------------------------------------------

/// Lightweight `ChannelContext` stub that reports `SemiTrusted`
/// trust tier and `Slack` platform. Used by the daemon's
/// `ChannelFactory` when a `FrontendType::Slack` connection
/// arrives. Same shape as `TelegramDaemonChannel` and
/// `DiscordDaemonChannel`; `stream_event` and `finalize` are
/// no-ops because the daemon-side `IpcChannelBridge` handles
/// forwarding events over IPC.
pub struct SlackDaemonChannel {
    session: SessionId,
    /// Rotated per turn by [`reset_cancellation`] and fired by
    /// [`cancel_inflight`]. See `TelegramDaemonChannel::token` —
    /// same C1+H1 audit fix.
    token: Mutex<CancellationToken>,
}

impl SlackDaemonChannel {
    pub fn new() -> Self {
        SlackDaemonChannel {
            session: SessionId::new(),
            token: Mutex::new(CancellationToken::new()),
        }
    }
}

impl Default for SlackDaemonChannel {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl ChannelContext for SlackDaemonChannel {
    fn channel_name(&self) -> &str {
        "aivyx-slack-daemon"
    }

    fn platform(&self) -> ChannelPlatform {
        ChannelPlatform::Slack
    }

    fn trust_tier(&self) -> aivyx_capability::TrustTier {
        aivyx_capability::TrustTier::SemiTrusted
    }

    fn session_id(&self) -> SessionId {
        self.session
    }

    async fn stream_event(&self, _event: StreamEvent<'_>) -> Result<(), ChannelError> {
        Ok(())
    }

    async fn finalize(&self, _outcome: &TurnOutcome) -> Result<(), ChannelError> {
        Ok(())
    }

    fn cancellation_token(&self) -> CancellationToken {
        self.token.lock().expect("token mutex poisoned").clone()
    }

    fn reset_cancellation(&self) {
        let mut slot = self.token.lock().expect("token mutex poisoned");
        *slot = CancellationToken::new();
    }

    fn cancel_inflight(&self) {
        self.token.lock().expect("token mutex poisoned").cancel();
    }
}

// ---------------------------------------------------------------------------
// Multi-channel pump
// ---------------------------------------------------------------------------

struct PartitionRoute {
    sender: tokio::sync::mpsc::Sender<IncomingMessage>,
    handle: tokio::task::JoinHandle<Result<(), DaemonError>>,
}

/// Drive a multi-channel Slack frontend over the daemon IPC
/// channel. Mirrors `run_discord_daemon_multi_session` (Phase
/// 111 Task 3) and `run_telegram_daemon_multi_session` (Phase
/// 19); the only platform-specific surface is the partition
/// key — Slack uses `String` for `(team_id, channel_id)`
/// rather than Discord's `u64` or Telegram's `i64`.
pub async fn run_slack_daemon_multi_session(
    transport: Arc<SlackMorphismTransport>,
    socket_path: PathBuf,
    role: Option<String>,
    shutdown: CancellationToken,
    team_run_channel: bool,
    team_trigger_rate_limit: Option<u32>,
) -> Result<(), DaemonError> {
    let mut routes: HashMap<String, PartitionRoute> = HashMap::new();

    loop {
        if shutdown.is_cancelled() {
            break;
        }

        let msg = tokio::select! {
            biased;
            _ = shutdown.cancelled() => break,
            res = transport.next_message() => match res {
                Ok(m) => m,
                Err(e) => {
                    eprintln!(
                        "aivyx-slack(daemon): next_message failed ({e}); backing off 1s"
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    continue;
                }
            }
        };

        let partition = msg.partition_key();

        let route = routes.entry(partition.clone()).or_insert_with(|| {
            let (tx, rx) = tokio::sync::mpsc::channel(32);
            let transport_clone = Arc::clone(&transport);
            let sp = socket_path.clone();
            let role_clone = role.clone();
            let shutdown_clone = shutdown.clone();
            let partition_for_task = partition.clone();
            let handle = tokio::spawn(async move {
                run_slack_daemon_partition_task(
                    transport_clone,
                    partition_for_task,
                    sp,
                    role_clone,
                    rx,
                    shutdown_clone,
                    team_run_channel,
                    team_trigger_rate_limit,
                )
                .await
            });
            PartitionRoute { sender: tx, handle }
        });

        if let Err(e) = route.sender.send(msg).await {
            eprintln!(
                "aivyx-slack(daemon): partition {partition} mailbox send failed ({e}); dropping route"
            );
            routes.remove(&partition);
        }
    }

    let drained: Vec<(String, PartitionRoute)> = routes.drain().collect();
    for (_partition, PartitionRoute { sender, handle }) in drained {
        drop(sender);
        match handle.await {
            Ok(Err(e)) => eprintln!("aivyx-slack(daemon): inner task error: {e}"),
            Err(e) => eprintln!("aivyx-slack(daemon): inner task join failed: {e}"),
            Ok(Ok(())) => {}
        }
    }

    Ok(())
}

/// Outcome of [`handle_slack_team_run_message`] — mirrors
/// `telegram_daemon_frontend::TelegramChatOutcome` /
/// `discord_daemon_frontend::DiscordChatOutcome` exactly.
#[derive(Debug, PartialEq, Eq)]
enum SlackChatOutcome {
    /// Reply immediately with this text; do not forward to the LLM turn
    /// path or the generic `gate_command`/`team_command` dispatch below.
    Reply(String),
    /// Nothing here matched (not a pending confirm resolution, not a
    /// fresh `/team run`) — the caller should fall through to its own
    /// existing `gate_command`/`team_command` dispatch and, ultimately,
    /// `session.submit_input`.
    NotHandled,
}

/// Extracted from `run_slack_daemon_partition_task` specifically so the
/// ordering invariant (the pending-confirm check and `/team run`
/// recognition MUST be checked before the generic `team_command`
/// dispatch, or `/team run` becomes permanently unreachable dead code —
/// a real bug this exact branch shipped once and had to fix in all
/// three channels, see the final-review report) is directly testable
/// without a live transport or socket. Mirrors
/// `telegram_daemon_frontend::handle_telegram_team_run_message` exactly
/// (same control flow, same fall-through-overwrite semantics) — see that
/// function's own doc comment for the detailed rationale. Takes only
/// `text`, not the partition key or a live `SlackMorphismTransport`: the
/// per-partition `channel_id`/reply-sending shape stays entirely in the
/// thin caller wrapper.
async fn handle_slack_team_run_message(
    text: &str,
    socket_path: &Path,
    pending_trigger: &mut Option<PendingTrigger>,
    trigger_history: &mut Vec<Instant>,
    team_run_channel: bool,
    team_trigger_rate_limit: Option<u32>,
) -> SlackChatOutcome {
    // Piece C — a pending confirm-first prompt takes priority over
    // everything else (including a stray gate_command/team_command
    // match, though "yes"/"no" never collide with either's own
    // `/`-prefixed syntax). Must run before both the gate_command
    // check below and Piece B's own generic `team_command::parse`
    // dispatch — the latter matches every `TeamCommand` variant
    // including `Run` and would otherwise route a fresh `/team run`
    // straight into `team_dispatch::dispatch`'s deliberate "should
    // never be dispatched directly" stub reply.
    if let Some(pending) = pending_trigger.take() {
        let now = Instant::now();
        match parse_confirm_reply(text) {
            Some(ConfirmReply::Yes) if pending.is_expired(now) => {
                return SlackChatOutcome::Reply("✗ that request expired, ask again.".to_string());
            }
            Some(ConfirmReply::Yes) => {
                let reply = match daemon_client::run_team_mission_channel(
                    socket_path,
                    FrontendType::Slack,
                    pending.goal.clone(),
                )
                .await
                {
                    Ok(mission_id) => format!("✓ Started mission {mission_id}."),
                    Err(e) => format!("✗ Could not start the mission: {e}"),
                };
                return SlackChatOutcome::Reply(reply);
            }
            Some(ConfirmReply::No) => {
                return SlackChatOutcome::Reply("Cancelled.".to_string());
            }
            None => {
                // Not a yes/no reply — put the pending trigger back
                // (unless it just expired) and fall through to the
                // normal command/chat-turn handling below.
                if !pending.is_expired(now) {
                    *pending_trigger = Some(pending);
                }
            }
        }
    }

    // Piece C — `/team run <goal>` itself. Must also run before
    // Piece B's generic `team_command::parse` block below, for the
    // same reason as the pending-trigger check above.
    if let Some(TeamCommand::Run { goal }) = team_command::parse(text) {
        if !team_run_channel {
            return SlackChatOutcome::Reply(
                "✗ this channel is not authorized to start team missions.".to_string(),
            );
        }
        let allowed = match team_trigger_rate_limit {
            Some(limit) => check_and_record_trigger(trigger_history, limit, Instant::now()),
            None => true,
        };
        if !allowed {
            let limit = team_trigger_rate_limit.unwrap_or(0);
            return SlackChatOutcome::Reply(format!(
                "✗ too many mission-start requests (max {limit} per hour), \
                 try again later."
            ));
        }
        *pending_trigger = Some(PendingTrigger::new(goal.clone()));
        return SlackChatOutcome::Reply(format!(
            "Start '{goal}' on the default team? Reply yes/no."
        ));
    }

    SlackChatOutcome::NotHandled
}

/// Per-partition inner task. Mirrors
/// `run_discord_daemon_channel_task` (Phase 111 Task 3) and
/// `run_telegram_daemon_chat_task` (Phase 19) — the only
/// per-adapter difference is the channel-id type used to
/// route outbound `OutgoingMessage`s.
#[allow(clippy::too_many_arguments)]
async fn run_slack_daemon_partition_task(
    transport: Arc<SlackMorphismTransport>,
    partition: String,
    socket_path: PathBuf,
    role: Option<String>,
    mut mailbox: tokio::sync::mpsc::Receiver<IncomingMessage>,
    shutdown: CancellationToken,
    team_run_channel: bool,
    team_trigger_rate_limit: Option<u32>,
) -> Result<(), DaemonError> {
    let mut session = DaemonSession::connect(
        &socket_path,
        role,
        Some(FrontendType::Slack),
    )
    .await?;

    let mut pending_trigger: Option<PendingTrigger> = None;
    let mut trigger_history: Vec<Instant> = Vec::new();

    loop {
        let msg = tokio::select! {
            biased;
            _ = shutdown.cancelled() => break,
            maybe_msg = mailbox.recv() => match maybe_msg {
                Some(m) => m,
                None => break,
            }
        };

        if msg.text.trim() == "/cancel" {
            let _ = session.cancel_turn().await;
            continue;
        }

        // Piece C — pending confirm-first resolution and fresh `/team run`
        // recognition both live in `handle_slack_team_run_message`,
        // extracted specifically so this ordering (both MUST run before
        // Piece B's own generic `team_command::parse` dispatch below, or
        // `/team run` becomes permanently unreachable dead code) is
        // directly testable. See that function's own doc comment.
        match handle_slack_team_run_message(
            msg.text.trim(),
            &socket_path,
            &mut pending_trigger,
            &mut trigger_history,
            team_run_channel,
            team_trigger_rate_limit,
        )
        .await
        {
            SlackChatOutcome::Reply(text) => {
                transport
                    .send_message(OutgoingMessage {
                        channel_id: msg.channel_id.clone(),
                        text,
                    })
                    .await
                    .map_err(|e| {
                        DaemonError::Internal(format!(
                            "send_message to partition {partition}: {e}"
                        ))
                    })?;
                continue;
            }
            SlackChatOutcome::NotHandled => {}
        }

        if let Some(gate_cmd) = gate_command::parse(msg.text.trim()) {
            let result = session
                .resolve_gate(gate_cmd.mission_id, gate_cmd.gate_id, gate_cmd.approved)
                .await;
            let reply = match result {
                Ok(()) => {
                    let status = if gate_cmd.approved { "approved" } else { "rejected" };
                    format!("✓ Gate {status}.")
                }
                Err(e) => format!("✗ Gate resolve failed: {e}"),
            };
            transport
                .send_message(OutgoingMessage {
                    channel_id: msg.channel_id.clone(),
                    text: reply,
                })
                .await
                .map_err(|e| {
                    DaemonError::Internal(format!(
                        "send_message to partition {partition}: {e}"
                    ))
                })?;
            continue;
        }

        if let Some(team_cmd) = team_command::parse(msg.text.trim()) {
            let reply = team_dispatch::dispatch(&socket_path, team_cmd).await;
            transport
                .send_message(OutgoingMessage {
                    channel_id: msg.channel_id.clone(),
                    text: reply,
                })
                .await
                .map_err(|e| {
                    DaemonError::Internal(format!(
                        "send_message to partition {partition}: {e}"
                    ))
                })?;
            continue;
        }

        let (events, _outcome) = session.submit_input(msg.text).await?;
        let buf = render_events_for_slack(&events);

        transport
            .send_message(OutgoingMessage {
                channel_id: msg.channel_id.clone(),
                text: buf,
            })
            .await
            .map_err(|e| {
                DaemonError::Internal(format!(
                    "send_message to partition {partition}: {e}"
                ))
            })?;
    }

    let _ = session.disconnect().await;
    Ok(())
}

/// Render accumulated `StreamEventPayload` events into a
/// single Slack message. Byte-identical to
/// `render_events_for_discord` (Phase 111 Task 3) and
/// structurally identical to `render_events_for_telegram`
/// (Phase 19) — the three SemiTrusted adapters all produce
/// the same in-message UX deliberately, so an operator
/// switching between them sees no surprise UI deltas.
pub(crate) fn render_events_for_slack(events: &[StreamEventPayload]) -> String {
    let mut buf = String::new();
    for event in events {
        match event {
            StreamEventPayload::Text { text } => {
                buf.push_str(text);
            }
            StreamEventPayload::Status { status } => {
                if !buf.is_empty() && !buf.ends_with('\n') {
                    buf.push('\n');
                }
                buf.push_str("… ");
                buf.push_str(status);
                buf.push('\n');
            }
            StreamEventPayload::ToolCallStarted { tool_name, .. } => {
                if !buf.is_empty() && !buf.ends_with('\n') {
                    buf.push('\n');
                }
                buf.push_str("→ ");
                buf.push_str(tool_name);
                buf.push('\n');
            }
            StreamEventPayload::ToolCallFinished {
                tool_name,
                outcome_summary,
                ..
            } => {
                if !buf.is_empty() && !buf.ends_with('\n') {
                    buf.push('\n');
                }
                buf.push_str("← ");
                buf.push_str(tool_name);
                buf.push(' ');
                buf.push_str(outcome_summary);
                buf.push('\n');
            }
            StreamEventPayload::ToolOutput { .. } => {}
            StreamEventPayload::ApprovalGate {
                mission_id,
                gate_id,
                reason,
                ..
            } => {
                if !buf.is_empty() && !buf.ends_with('\n') {
                    buf.push('\n');
                }
                buf.push_str(&format!(
                    "⚑ APPROVAL GATE [{mission_id}/{gate_id}]: {reason}\n\
                     Reply /approve {mission_id} {gate_id}\n\
                     or    /reject  {mission_id} {gate_id}\n"
                ));
            }
        }
    }

    if buf.trim().is_empty() {
        "(no reply)".to_string()
    } else {
        buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon_ipc::StreamEventPayload;

    #[test]
    fn slack_daemon_channel_reports_correct_identity() {
        let c = SlackDaemonChannel::new();
        assert_eq!(c.channel_name(), "aivyx-slack-daemon");
        assert_eq!(c.platform(), ChannelPlatform::Slack);
        assert_eq!(c.trust_tier(), aivyx_capability::TrustTier::SemiTrusted);
    }

    // Audit C1+H1 regression — same coverage as the
    // Telegram + Discord stubs.
    #[test]
    fn cancel_inflight_then_reset_yields_fresh_token() {
        let c = SlackDaemonChannel::new();
        let stale = c.cancellation_token();
        assert!(!stale.is_cancelled());
        c.cancel_inflight();
        assert!(stale.is_cancelled(), "cancel_inflight fires the live token");
        c.reset_cancellation();
        assert!(
            !c.cancellation_token().is_cancelled(),
            "reset_cancellation installs a fresh token"
        );
        assert!(stale.is_cancelled(), "the pre-reset token stays cancelled");
    }

    #[test]
    fn render_events_text_concatenates() {
        let events = vec![
            StreamEventPayload::Text { text: "hello ".into() },
            StreamEventPayload::Text { text: "slack".into() },
        ];
        assert_eq!(render_events_for_slack(&events), "hello slack");
    }

    #[test]
    fn render_events_tool_call_arrows_render_consistently() {
        let events = vec![
            StreamEventPayload::ToolCallStarted {
                tool_id: "tid".into(),
                tool_name: "memory.read".into(),
                input: serde_json::Value::Null,
            },
            StreamEventPayload::ToolCallFinished {
                tool_id: "tid".into(),
                tool_name: "memory.read".into(),
                outcome_summary: "ok".into(),
            },
        ];
        let out = render_events_for_slack(&events);
        assert!(out.contains("→ memory.read"));
        assert!(out.contains("← memory.read ok"));
    }

    #[test]
    fn render_events_empty_payload_returns_no_reply_placeholder() {
        assert_eq!(render_events_for_slack(&[]), "(no reply)");
    }

    #[test]
    fn render_events_byte_identical_to_discord_renderer() {
        // Three-data-point sanity check: the Slack and Discord
        // renderers must produce byte-identical output for the
        // same StreamEventPayload sequence. If a future divergence
        // ships (e.g. Slack-specific markdown), this test breaks
        // and forces the divergence to be documented.
        let events = vec![
            StreamEventPayload::Text { text: "result: ".into() },
            StreamEventPayload::ToolCallStarted {
                tool_id: "tid".into(),
                tool_name: "fs.read".into(),
                input: serde_json::Value::Null,
            },
            StreamEventPayload::Status { status: "thinking".into() },
            StreamEventPayload::ApprovalGate {
                mission_id: "m-001".into(),
                gate_id: "g-abc".into(),
                scope: Some("fs.write".into()),
                reason: "write a file".into(),
            },
        ];
        let slack_out = render_events_for_slack(&events);
        let discord_out =
            crate::discord_daemon_frontend::render_events_for_discord(&events);
        assert_eq!(slack_out, discord_out);
    }

    // --- I5 (final-review) — the ordering invariant that keeps
    // `/team run` from being permanently unreachable, locked in by
    // testing `handle_slack_team_run_message` directly.

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixListener;

    /// A fake daemon that does the `StartSession` handshake then
    /// replies `TeamMissionChannelStarted` — mirrors
    /// `daemon_client.rs`'s own
    /// `run_team_mission_channel_does_the_start_session_handshake_then_sends_the_request`
    /// fixture, since `handle_slack_team_run_message`'s "yes" path calls
    /// the real `daemon_client::run_team_mission_channel`.
    async fn fake_daemon_starting_mission(
        mission_id: &str,
    ) -> (PathBuf, tokio::task::JoinHandle<()>) {
        use crate::daemon_ipc::{encode_frame, DaemonEnvelope};

        let sock = std::env::temp_dir()
            .join(format!("aivyx-slack-teamrun-{}.sock", uuid::Uuid::new_v4()));
        let listener = UnixListener::bind(&sock).expect("bind fake daemon");
        let sock_clone = sock.clone();
        let mission_id = mission_id.to_string();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let ready = encode_frame(&DaemonEnvelope::DaemonReady {
                version: "0.1".into(),
            })
            .expect("encode ready");
            stream.write_all(&ready).await.expect("write ready");

            let mut tmp = [0u8; 2048];
            let _ = stream.read(&mut tmp).await; // client's StartSession
            let started = encode_frame(&DaemonEnvelope::SessionStarted {
                session_id: "sess-1".into(),
            })
            .expect("encode started");
            stream.write_all(&started).await.expect("write started");

            let _ = stream.read(&mut tmp).await; // client's RunTeamMissionChannel
            let resp = encode_frame(&DaemonEnvelope::TeamMissionChannelStarted { mission_id })
                .expect("encode resp");
            stream.write_all(&resp).await.expect("write resp");
            let _ = stream.read(&mut tmp).await;
        });

        (sock_clone, server)
    }

    #[tokio::test]
    async fn team_run_authorized_and_under_limit_prompts_for_confirmation() {
        // This is the test that would have caught the original ordering
        // bug: if the generic `team_command`/`team_dispatch` dispatch
        // ran first, `/team run` would hit `team_dispatch::dispatch`'s
        // "should never be dispatched directly" stub instead of this
        // confirm prompt.
        let mut pending: Option<PendingTrigger> = None;
        let mut history: Vec<Instant> = Vec::new();

        let outcome = handle_slack_team_run_message(
            "/team run close the books",
            Path::new("/nonexistent/unused.sock"),
            &mut pending,
            &mut history,
            true,
            None,
        )
        .await;

        match outcome {
            SlackChatOutcome::Reply(text) => {
                assert!(
                    !text.contains("should never be dispatched directly"),
                    "must not fall through to the generic team_dispatch stub: {text}"
                );
                assert!(
                    text.contains("Reply yes/no"),
                    "must prompt for confirmation: {text}"
                );
            }
            SlackChatOutcome::NotHandled => panic!("expected a Reply, got NotHandled"),
        }
        assert!(pending.is_some(), "a pending trigger must now be recorded");
    }

    #[tokio::test]
    async fn team_run_denied_when_channel_not_opted_in() {
        let mut pending: Option<PendingTrigger> = None;
        let mut history: Vec<Instant> = Vec::new();

        let outcome = handle_slack_team_run_message(
            "/team run close the books",
            Path::new("/nonexistent/unused.sock"),
            &mut pending,
            &mut history,
            false,
            None,
        )
        .await;

        match outcome {
            SlackChatOutcome::Reply(text) => {
                assert!(text.contains("not authorized"), "got: {text}");
            }
            SlackChatOutcome::NotHandled => panic!("expected a Reply, got NotHandled"),
        }
        assert!(pending.is_none(), "an unauthorized attempt must not arm a pending trigger");
    }

    #[tokio::test]
    async fn ordinary_text_with_no_pending_trigger_is_not_handled() {
        let mut pending: Option<PendingTrigger> = None;
        let mut history: Vec<Instant> = Vec::new();

        let outcome = handle_slack_team_run_message(
            "just chatting, nothing special",
            Path::new("/nonexistent/unused.sock"),
            &mut pending,
            &mut history,
            true,
            None,
        )
        .await;

        assert_eq!(outcome, SlackChatOutcome::NotHandled);
        assert!(pending.is_none());
    }

    #[tokio::test]
    async fn pending_trigger_plus_yes_starts_the_mission_via_the_real_daemon_client_call() {
        let (sock, server) = fake_daemon_starting_mission("m-42").await;

        let mut pending = Some(PendingTrigger::new("close the books"));
        let mut history: Vec<Instant> = Vec::new();

        let outcome =
            handle_slack_team_run_message("yes", &sock, &mut pending, &mut history, true, None)
                .await;

        match outcome {
            SlackChatOutcome::Reply(text) => {
                assert!(text.contains("Started mission"), "got: {text}");
                assert!(text.contains("m-42"));
            }
            SlackChatOutcome::NotHandled => panic!("expected a Reply, got NotHandled"),
        }
        assert!(pending.is_none(), "the pending trigger is consumed on yes");

        let _ = server.await;
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn pending_trigger_plus_no_cancels() {
        let mut pending = Some(PendingTrigger::new("close the books"));
        let mut history: Vec<Instant> = Vec::new();

        let outcome = handle_slack_team_run_message(
            "no",
            Path::new("/nonexistent/unused.sock"),
            &mut pending,
            &mut history,
            true,
            None,
        )
        .await;

        match outcome {
            SlackChatOutcome::Reply(text) => {
                assert!(text.contains("Cancelled."), "got: {text}");
            }
            SlackChatOutcome::NotHandled => panic!("expected a Reply, got NotHandled"),
        }
        assert!(pending.is_none(), "a 'no' reply clears the pending trigger");
    }
}
