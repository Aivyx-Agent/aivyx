//! Piece B (2026-08-23) — dispatches a parsed `TeamCommand` to the daemon
//! over IPC and renders the reply text a channel sends back to the chat.
//!
//! Real-code correction vs. the original design doc: the design assumed a
//! channel adapter's command handler runs "in-process" with the daemon and
//! could call `TeamMissionService` directly. Re-verified against the real
//! architecture: every channel adapter's daemon-mode driver
//! (`telegram_daemon_frontend.rs` and its Discord/Slack siblings) is a
//! **separate OS process** from `aivyx daemon run`, talking over
//! Unix-socket IPC via `daemon_client`. This module therefore calls the
//! existing, already-tested one-shot `daemon_client::{team_mission_list,
//! team_mission_status, resolve_team_gate, abort_team_mission,
//! pause_team_mission, resume_team_mission}` free functions — the same
//! ones the CLI/TUI already use — rather than adding new `DaemonSession`
//! methods that would duplicate `send_query`'s protocol handling.

use std::path::Path;

use crate::daemon_client;
use crate::team_command::TeamCommand;
use crate::team_mission::{TeamMissionPhase, TeamMissionRecord};

/// Dispatch a parsed `/team ...` command over IPC and render the reply
/// text for the originating chat. Never panics or propagates an error —
/// every failure path (daemon unreachable, unknown mission id, daemon-side
/// `QueryError`) becomes a `✗`-prefixed reply string instead, matching the
/// existing `gate_command` dispatch convention already inline in each
/// daemon-frontend.
pub async fn dispatch(socket_path: &Path, cmd: TeamCommand) -> String {
    match cmd {
        TeamCommand::Status(None) => match daemon_client::team_mission_list(socket_path).await {
            Ok(missions) => render_team_list(&missions),
            Err(e) => format!("✗ Could not list team missions: {e}"),
        },
        TeamCommand::Status(Some(mission_id)) => {
            match daemon_client::team_mission_status(socket_path, mission_id.clone()).await {
                Ok(Some(mission)) => render_team_status(&mission),
                Ok(None) => format!("✗ No team mission found with id `{mission_id}`."),
                Err(e) => format!("✗ Could not fetch mission `{mission_id}`: {e}"),
            }
        }
        TeamCommand::ResolveGate {
            mission_id,
            step,
            approve,
        } => {
            let verb = if approve { "approve" } else { "reject" };
            let past_tense = if approve { "approved" } else { "rejected" };
            match daemon_client::resolve_team_gate(socket_path, mission_id, step, approve).await {
                Ok(phase) => format!("✓ Gate {past_tense}. Mission now {}.", phase_label(phase)),
                Err(e) => format!("✗ Gate {verb} failed: {e}"),
            }
        }
        TeamCommand::Pause { mission_id } => {
            match daemon_client::pause_team_mission(socket_path, mission_id).await {
                Ok(message) => format!("✓ {message}"),
                Err(e) => format!("✗ Pause failed: {e}"),
            }
        }
        TeamCommand::Resume { mission_id } => {
            match daemon_client::resume_team_mission(socket_path, mission_id).await {
                Ok(phase) => format!("✓ Resumed. Mission now {}.", phase_label(phase)),
                Err(e) => format!("✗ Resume failed: {e}"),
            }
        }
        TeamCommand::Abort { mission_id } => {
            match daemon_client::abort_team_mission(socket_path, mission_id).await {
                Ok(message) => format!("✓ {message}"),
                Err(e) => format!("✗ Abort failed: {e}"),
            }
        }
        TeamCommand::Usage => "✗ Usage: /team status [<id>] | /team approve|reject <id> <step> | /team pause|resume <id> | /team abort <id>".to_string(),
    }
}

fn phase_label(phase: TeamMissionPhase) -> &'static str {
    match phase {
        TeamMissionPhase::Planning => "planning",
        TeamMissionPhase::Executing => "executing",
        TeamMissionPhase::AwaitingApproval => "awaiting approval",
        TeamMissionPhase::Paused => "paused",
        TeamMissionPhase::Done => "done",
        TeamMissionPhase::Rejected => "rejected",
        TeamMissionPhase::Halted => "halted",
    }
}

/// Pure renderer — every mission as a short chat-friendly list, most-
/// recently-updated first. Mirrors `aivyx-cli`'s own
/// `team_cli::render_mission_list` in spirit (not importable across the
/// `aivyx-cli` binary-crate boundary — see this plan's research notes),
/// simplified for a chat surface (no fixed-width padding).
fn render_team_list(missions: &[TeamMissionRecord]) -> String {
    if missions.is_empty() {
        return "No team missions running.".to_string();
    }
    let mut sorted: Vec<&TeamMissionRecord> = missions.iter().collect();
    sorted.sort_by_key(|m| std::cmp::Reverse(m.updated_at_unix_ms));

    let mut out = format!("{} team mission(s):\n", sorted.len());
    for m in sorted {
        let done = m.outputs.len();
        let total = m.plan.steps.len();
        out.push_str(&format!(
            "• {} [{}] {}/{} steps — {}\n",
            m.id,
            phase_label(m.phase),
            done,
            total,
            m.goal,
        ));
        if let Some(gate) = &m.pending_gate {
            out.push_str(&format!(
                "   ↳ `/team approve {} {gate}` or `/team reject {} {gate}`\n",
                m.id, m.id
            ));
        }
        if m.phase == TeamMissionPhase::Paused {
            out.push_str(&format!("   ↳ resume with `/team resume {}`\n", m.id));
        }
    }
    out
}

/// Pure renderer — one mission's detail for a chat reply. Mirrors
/// `aivyx-cli`'s own `team_cli::render_mission_status` in spirit, same
/// non-importability rationale as `render_team_list`.
fn render_team_status(record: &TeamMissionRecord) -> String {
    let mut out = format!(
        "Mission {}\n  goal: {}\n  phase: {}\n",
        record.id,
        record.goal,
        phase_label(record.phase),
    );
    if let Some(gate) = &record.pending_gate {
        out.push_str(&format!(
            "  gate: `{gate}` awaiting your decision (`/team approve {} {gate}` / `/team reject {} {gate}`)\n",
            record.id, record.id
        ));
    }
    if record.phase == TeamMissionPhase::Halted {
        if let Some(reason) = &record.halt_reason {
            out.push_str(&format!("  reason: {reason}\n"));
        }
    }
    if record.phase == TeamMissionPhase::Paused {
        out.push_str(&format!("  ↳ resume with `/team resume {}`\n", record.id));
    }
    out.push_str(&format!(
        "  steps: {}/{} done\n",
        record.outputs.len(),
        record.plan.steps.len()
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_team::{MissionPlan, Step};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixListener;

    fn sample_mission(phase: TeamMissionPhase, pending: Option<&str>) -> TeamMissionRecord {
        let plan = MissionPlan::new(
            "ship the note",
            vec![
                Step::delegate("research", "researcher", "gather"),
                Step::human_gate("approve", "reviewer", "ok?").after(["research"]),
                Step::delegate("write", "writer", "draft").after(["approve"]),
            ],
        );
        let mut r = TeamMissionRecord::new("m-1", "ship the note", plan);
        r.phase = phase;
        r.pending_gate = pending.map(str::to_string);
        if pending.is_some() {
            r.outputs.insert("research".into(), "12 findings".into());
        }
        r
    }

    // --- pure render tests ---

    #[test]
    fn render_team_list_empty_says_no_missions() {
        assert_eq!(render_team_list(&[]), "No team missions running.");
    }

    #[test]
    fn render_team_list_shows_id_phase_progress_goal() {
        let m = sample_mission(TeamMissionPhase::Executing, None);
        let out = render_team_list(&[m]);
        assert!(out.contains("m-1"));
        assert!(out.contains("executing"));
        assert!(out.contains("0/3 steps"));
        assert!(out.contains("ship the note"));
    }

    #[test]
    fn render_team_list_shows_gate_hint_when_awaiting_approval() {
        let m = sample_mission(TeamMissionPhase::AwaitingApproval, Some("approve"));
        let out = render_team_list(&[m]);
        assert!(out.contains("/team approve m-1 approve"));
        assert!(out.contains("/team reject m-1 approve"));
    }

    #[test]
    fn render_team_list_shows_resume_hint_when_paused() {
        let m = sample_mission(TeamMissionPhase::Paused, None);
        let out = render_team_list(&[m]);
        assert!(out.contains("/team resume m-1"));
    }

    #[test]
    fn render_team_status_includes_goal_phase_and_step_progress() {
        let m = sample_mission(TeamMissionPhase::AwaitingApproval, Some("approve"));
        let out = render_team_status(&m);
        assert!(out.contains("Mission m-1"));
        assert!(out.contains("goal: ship the note"));
        assert!(out.contains("awaiting approval"));
        assert!(out.contains("gate: `approve`"));
        assert!(out.contains("steps: 1/3 done"));
    }

    #[test]
    fn render_team_status_shows_halt_reason_when_halted() {
        let mut m = sample_mission(TeamMissionPhase::Halted, None);
        m.halt_reason = Some("budget cap exceeded".to_string());
        let out = render_team_status(&m);
        assert!(out.contains("reason: budget cap exceeded"));
    }

    // --- dispatch tests: a fake daemon over a real UnixListener, mirroring
    // daemon_client.rs's own `send_query_skips_recovery_notice_before_the_response`
    // test fixture, confirming `dispatch` reaches the correct QueryPayload
    // and renders the reply from the correct QueryResponsePayload.

    async fn fake_daemon_returning(
        response: aivyx_ipc::protocol::QueryResponsePayload,
    ) -> (std::path::PathBuf, tokio::task::JoinHandle<()>) {
        use crate::daemon_ipc::{encode_frame, DaemonEnvelope};

        let sock = std::env::temp_dir()
            .join(format!("aivyx-team-dispatch-{}.sock", uuid::Uuid::new_v4()));
        let listener = UnixListener::bind(&sock).expect("bind fake daemon");
        let sock_clone = sock.clone();

        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let ready = encode_frame(&DaemonEnvelope::DaemonReady {
                version: "0.1".into(),
            })
            .expect("encode ready");
            stream.write_all(&ready).await.expect("write ready");
            let mut tmp = [0u8; 2048];
            let _ = stream.read(&mut tmp).await;
            let resp = encode_frame(&DaemonEnvelope::QueryResponse {
                id: "q".into(),
                payload: response,
            })
            .expect("encode resp");
            stream.write_all(&resp).await.expect("write resp");
            let _ = stream.read(&mut tmp).await;
        });

        (sock_clone, server)
    }

    #[tokio::test]
    async fn dispatch_status_bare_renders_list() {
        use aivyx_ipc::protocol::QueryResponsePayload;
        let m = sample_mission(TeamMissionPhase::Executing, None);
        let (sock, server) =
            fake_daemon_returning(QueryResponsePayload::TeamMissionList { missions: vec![m] })
                .await;

        let reply = dispatch(&sock, TeamCommand::Status(None)).await;
        assert!(reply.contains("m-1"));
        assert!(reply.contains("executing"));

        let _ = server.await;
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn dispatch_status_with_id_renders_status() {
        use aivyx_ipc::protocol::QueryResponsePayload;
        let m = sample_mission(TeamMissionPhase::Done, None);
        let (sock, server) = fake_daemon_returning(QueryResponsePayload::TeamMissionStatus {
            mission: Some(m),
        })
        .await;

        let reply = dispatch(&sock, TeamCommand::Status(Some("m-1".to_string()))).await;
        assert!(reply.contains("Mission m-1"));
        assert!(reply.contains("done"));

        let _ = server.await;
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn dispatch_status_with_unknown_id_reports_not_found() {
        use aivyx_ipc::protocol::QueryResponsePayload;
        let (sock, server) =
            fake_daemon_returning(QueryResponsePayload::TeamMissionStatus { mission: None })
                .await;

        let reply = dispatch(&sock, TeamCommand::Status(Some("nope".to_string()))).await;
        assert!(reply.starts_with('✗'));
        assert!(reply.contains("nope"));

        let _ = server.await;
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn dispatch_approve_renders_confirmation() {
        use aivyx_ipc::protocol::QueryResponsePayload;
        let (sock, server) = fake_daemon_returning(QueryResponsePayload::TeamGateResolved {
            mission_id: "m-1".to_string(),
            phase: TeamMissionPhase::Executing,
        })
        .await;

        let reply = dispatch(
            &sock,
            TeamCommand::ResolveGate {
                mission_id: "m-1".to_string(),
                step: "approve".to_string(),
                approve: true,
            },
        )
        .await;
        assert_eq!(reply, "✓ Gate approved. Mission now executing.");

        let _ = server.await;
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn dispatch_reject_renders_confirmation() {
        use aivyx_ipc::protocol::QueryResponsePayload;
        let (sock, server) = fake_daemon_returning(QueryResponsePayload::TeamGateResolved {
            mission_id: "m-1".to_string(),
            phase: TeamMissionPhase::Rejected,
        })
        .await;

        let reply = dispatch(
            &sock,
            TeamCommand::ResolveGate {
                mission_id: "m-1".to_string(),
                step: "approve".to_string(),
                approve: false,
            },
        )
        .await;
        assert_eq!(reply, "✓ Gate rejected. Mission now rejected.");

        let _ = server.await;
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn dispatch_pause_renders_message() {
        use aivyx_ipc::protocol::QueryResponsePayload;
        let (sock, server) = fake_daemon_returning(QueryResponsePayload::TeamMissionPaused {
            mission_id: "m-1".to_string(),
            message: "mission will pause at its next wave boundary".to_string(),
        })
        .await;

        let reply = dispatch(&sock, TeamCommand::Pause { mission_id: "m-1".to_string() }).await;
        assert_eq!(reply, "✓ mission will pause at its next wave boundary");

        let _ = server.await;
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn dispatch_resume_renders_phase() {
        use aivyx_ipc::protocol::QueryResponsePayload;
        let (sock, server) = fake_daemon_returning(QueryResponsePayload::TeamMissionResumed {
            mission_id: "m-1".to_string(),
            phase: TeamMissionPhase::Executing,
        })
        .await;

        let reply = dispatch(&sock, TeamCommand::Resume { mission_id: "m-1".to_string() }).await;
        assert!(reply.starts_with('✓'));
        assert!(reply.contains("executing"));

        let _ = server.await;
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn dispatch_abort_renders_message() {
        use aivyx_ipc::protocol::QueryResponsePayload;
        let (sock, server) = fake_daemon_returning(QueryResponsePayload::TeamMissionAborted {
            mission_id: "m-1".to_string(),
            message: "mission will halt at its next wave boundary".to_string(),
        })
        .await;

        let reply = dispatch(&sock, TeamCommand::Abort { mission_id: "m-1".to_string() }).await;
        assert_eq!(reply, "✓ mission will halt at its next wave boundary");

        let _ = server.await;
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn dispatch_daemon_error_renders_cross_prefixed_reply() {
        use aivyx_ipc::protocol::QueryResponsePayload;
        let (sock, server) = fake_daemon_returning(QueryResponsePayload::QueryError {
            code: "abort_team_mission_failed".to_string(),
            message: "mission not found".to_string(),
        })
        .await;

        let reply = dispatch(&sock, TeamCommand::Abort { mission_id: "m-1".to_string() }).await;
        assert!(reply.starts_with('✗'));
        assert!(reply.contains("Abort failed"));

        let _ = server.await;
        let _ = std::fs::remove_file(&sock);
    }

    #[tokio::test]
    async fn dispatch_usage_renders_usage_hint_without_any_daemon_call() {
        // No fake daemon needed — Usage never reaches daemon_client.
        let reply = dispatch(std::path::Path::new("/nonexistent/unused.sock"), TeamCommand::Usage).await;
        assert!(reply.starts_with('✗'));
        assert!(reply.contains("Usage"));
    }
}
