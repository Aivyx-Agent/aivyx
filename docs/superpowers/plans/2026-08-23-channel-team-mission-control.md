# Channel-Triggered Team-Mission Monitoring/Control (Piece B) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give Telegram, Discord, and Slack operators a native `/team ...` command set — `status`, `approve`/`reject`, `pause`/`resume`, `abort` — that controls already-running Nonagon team missions from the same chat surface they already use, without adding any new capability or mission-starting path.

**Architecture:** A new pure parser (`team_command.rs`, mirroring the existing `gate_command.rs` exactly) recognizes `/team ...` text before it falls through to the normal chat-turn path. A new async dispatcher (`team_dispatch.rs`) turns a parsed command into a chat-ready reply string by calling the daemon over IPC. Each of the three channel daemon-frontends (`telegram_daemon_frontend.rs`, `discord_daemon_frontend.rs`, `slack_daemon_frontend.rs`) gets an identical ~10-line wiring block inserted at the same point their existing `gate_command::parse` check already lives.

**Tech Stack:** Rust, tokio, the existing `aivyx-channel` daemon-frontend/IPC machinery (`daemon_client`, `aivyx-ipc`'s `QueryPayload`/`QueryResponsePayload`).

## Global Constraints

- **No new capability base and no new mission-starting path.** Piece B is monitoring/control of already-running missions only — `status`, `approve`/`reject`, `pause`/`resume`, `abort`. Piece C (channel-triggered new mission starts) is a separate, later plan.
- **All 3 channels in the same pass** (Telegram, Discord, Slack) — per the earlier-approved scope decision.
- **`/team `-prefixed commands only**, to avoid colliding with the existing bare `/approve` / `/reject` (handled by `gate_command.rs`, targets the *old* single-agent Mission system, not Nonagon team missions) and `/cancel` (per-turn cancellation) commands each channel already recognizes.
- **Reuse the existing, already-tested one-shot `daemon_client::{team_mission_list, team_mission_status, resolve_team_gate, abort_team_mission, pause_team_mission, resume_team_mission}` free functions** — do not add new `DaemonSession` methods. See "Research corrections" below for why.
- **No capability-scope check needed.** Re-verified against current `daemon_server.rs`: `TeamMissionList`/`TeamMissionStatus`/`ResolveTeamGate`/`AbortTeamMission`/`PauseTeamMission`/`ResumeTeamMission` are all plain daemon-internal calls with only an `Option<&TeamMissionService>` presence check (`no_team_missions()` on `None`) — no `CapabilitySet`/trust-tier check exists on any of these six handlers today. If a future change adds one, that's out of this plan's scope to discover, but nothing in this plan should be built to route around a check that doesn't exist.
- **Every failure path becomes a `✗`-prefixed chat reply, never silence or a panic** — daemon unreachable, unknown mission id, and daemon-side `QueryError` all render as a reply string.

## Research corrections (re-verified against real code, 2026-08-23)

The design doc (`docs/superpowers/specs/2026-08-23-team-mission-triggers-design.md`) was written before Piece A shipped and before this plan's own re-verification pass. Two of its claims about the real code turned out to be wrong; this plan is built on the corrected facts:

1. **"A channel adapter's own message-handling code already runs inside the trusted daemon process."** False. Every channel (`--channel telegram|discord|slack`) runs its **daemon-mode driver in a separate OS process** from `aivyx daemon run`, connected over Unix-socket IPC (`DaemonSession::connect`, `daemon_client.rs`). `TeamMissionService` is constructed only inside `aivyx.rs`'s `CliMode::DaemonRun` block and never reaches the channel-dispatch code path. A channel command must go over IPC, not call `TeamMissionService` directly.
2. **The IPC path does not need new protocol code.** `daemon_client.rs` already has one-shot free functions for exactly the six operations Piece B needs (`team_mission_list`, `team_mission_status`, `resolve_team_gate`, `abort_team_mission`, `pause_team_mission`, `resume_team_mission`, lines 1216-1343), each already calling `send_query` (which opens a fresh `UnixStream`, does the `DaemonReady` handshake, sends `FrontendMessage::Query`, and awaits `DaemonEnvelope::QueryResponse`) and already mapping `QueryResponsePayload::QueryError` to `Err(DaemonError::Protocol(...))`. These are the same functions the CLI/TUI use today. Piece B calls them directly instead of adding new `DaemonSession` methods that would duplicate this protocol handling.
3. **The existing `gate_command.rs`/`/approve`/`/reject` mechanism cannot be reused for team missions.** It routes to the *old* single-agent `mission::*` store (`FrontendMessage::ResolveGate` → `KeyDomain::Missions`), a completely different system from Nonagon team missions (`QueryPayload::ResolveTeamGate` → `KeyDomain::TeamMissions`). Only its *shape* (a shared, pure, tested parser module) is reused — the parsing/dispatch logic itself is new.
4. **All three channels' daemon-frontends are structurally identical at the exact insertion point Piece B needs.** Confirmed by direct comparison: `run_telegram_daemon_chat_task`, `run_discord_daemon_channel_task`, and `run_slack_daemon_partition_task` all follow the identical sequence `DaemonSession::connect(...)` → loop → `/cancel` check → `gate_command::parse` check (identical reply-formatting `format!("✓ Gate {status}.")` / `format!("✗ Gate resolve failed: {e}")`, differing only in the `OutgoingMessage` field name (`chat_id` for Telegram, `channel_id` for Discord/Slack)) → fallthrough to `session.submit_input(msg.text)`. A shared parser + shared dispatcher, with one thin per-channel wiring block each, is the right shape — not three parallel implementations.

## File Structure

- **Create** `crates/aivyx-channel/src/team_command.rs` — pure `TeamCommand` enum + `parse(text: &str) -> Option<TeamCommand>`. No I/O. Mirrors `gate_command.rs`'s scope exactly.
- **Create** `crates/aivyx-channel/src/team_dispatch.rs` — `pub async fn dispatch(socket_path: &Path, cmd: TeamCommand) -> String`, calling the six `daemon_client` free functions and rendering the reply (including list/status rendering, mirroring `aivyx-cli`'s `team_cli::render_mission_list`/`render_mission_status` in spirit — not importable across the `aivyx-cli` binary-crate boundary).
- **Modify** `crates/aivyx-channel/src/lib.rs` — register the two new modules.
- **Modify** `crates/aivyx-channel/src/telegram_daemon_frontend.rs` — wire the new commands into `run_telegram_daemon_chat_task`.
- **Modify** `crates/aivyx-channel/src/discord_daemon_frontend.rs` — wire the new commands into `run_discord_daemon_channel_task`.
- **Modify** `crates/aivyx-channel/src/slack_daemon_frontend.rs` — wire the new commands into `run_slack_daemon_partition_task`.

---

### Task 1: `team_command.rs` — the shared parser

**Files:**
- Create: `crates/aivyx-channel/src/team_command.rs`
- Modify: `crates/aivyx-channel/src/lib.rs:181` (insert `pub mod team_command;` immediately after the existing `pub mod gate_command;` line)

**Interfaces:**
- Produces: `pub enum TeamCommand { Status(Option<String>), ResolveGate { mission_id: String, step: String, approve: bool }, Pause { mission_id: String }, Resume { mission_id: String }, Abort { mission_id: String } }` and `pub fn parse(text: &str) -> Option<TeamCommand>`, both in `crate::team_command`. Task 2 consumes both.

- [ ] **Step 1: Write the failing tests**

Create `crates/aivyx-channel/src/team_command.rs` with just the type and a stub, then add the test module:

```rust
//! Piece B (2026-08-23) — shared team-mission text-command parser.
//!
//! Mirrors `gate_command.rs`'s own shape and scope exactly: pure parsing
//! only, no I/O, no daemon dispatch (dispatch lives in `team_dispatch.rs`).
//! Telegram, Discord, and Slack daemon-frontends all call `parse` with the
//! same whitespace-trimmed message text and treat `None` as "fall through
//! to the normal chat-turn path."
//!
//! ## What this parses
//!
//! `/team `-prefixed commands, deliberately namespaced so they never
//! collide with the existing bare `/approve` / `/reject` (handled by
//! `gate_command.rs`, which targets the *old* single-agent Mission system,
//! not Nonagon team missions) or `/cancel` (per-turn cancellation, handled
//! separately by each daemon-frontend):
//!
//! - `/team status` — every mission.
//! - `/team status <mission_id>` — one mission's detail.
//! - `/team approve <mission_id> <step>` / `/team reject <mission_id> <step>`
//! - `/team pause <mission_id>` / `/team resume <mission_id>`
//! - `/team abort <mission_id>`
//!
//! Anything else — including bare `/status`, `/approve` without the `team`
//! token, wrong argument counts, or unknown subcommands — returns `None`.

/// A parsed `/team ...` command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TeamCommand {
    /// `/team status` (`None`) or `/team status <id>` (`Some(id)`).
    Status(Option<String>),
    /// `/team approve <id> <step>` (`approve: true`) or
    /// `/team reject <id> <step>` (`approve: false`).
    ResolveGate {
        mission_id: String,
        step: String,
        approve: bool,
    },
    /// `/team pause <id>`.
    Pause { mission_id: String },
    /// `/team resume <id>`.
    Resume { mission_id: String },
    /// `/team abort <id>`.
    Abort { mission_id: String },
}

/// Parse a `/team ...` command. Returns `None` for anything else. Input is
/// expected to already be whitespace-trimmed; adapters that need
/// mention-stripping (Slack `<@U...>`, Discord `<@!...>`) do so before
/// calling, same contract as `gate_command::parse`.
pub fn parse(text: &str) -> Option<TeamCommand> {
    let parts: Vec<&str> = text.split_whitespace().collect();
    if parts.first() != Some(&"/team") {
        return None;
    }
    match parts.as_slice() {
        ["/team", "status"] => Some(TeamCommand::Status(None)),
        ["/team", "status", id] => Some(TeamCommand::Status(Some((*id).to_string()))),
        ["/team", "approve", id, step] => Some(TeamCommand::ResolveGate {
            mission_id: (*id).to_string(),
            step: (*step).to_string(),
            approve: true,
        }),
        ["/team", "reject", id, step] => Some(TeamCommand::ResolveGate {
            mission_id: (*id).to_string(),
            step: (*step).to_string(),
            approve: false,
        }),
        ["/team", "pause", id] => Some(TeamCommand::Pause {
            mission_id: (*id).to_string(),
        }),
        ["/team", "resume", id] => Some(TeamCommand::Resume {
            mission_id: (*id).to_string(),
        }),
        ["/team", "abort", id] => Some(TeamCommand::Abort {
            mission_id: (*id).to_string(),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_status_bare_lists_all() {
        assert_eq!(parse("/team status"), Some(TeamCommand::Status(None)));
    }

    #[test]
    fn parse_status_with_id() {
        assert_eq!(
            parse("/team status m-001"),
            Some(TeamCommand::Status(Some("m-001".to_string())))
        );
    }

    #[test]
    fn parse_approve_canonical() {
        assert_eq!(
            parse("/team approve m-001 approve-step"),
            Some(TeamCommand::ResolveGate {
                mission_id: "m-001".to_string(),
                step: "approve-step".to_string(),
                approve: true,
            })
        );
    }

    #[test]
    fn parse_reject_canonical() {
        assert_eq!(
            parse("/team reject m-002 review"),
            Some(TeamCommand::ResolveGate {
                mission_id: "m-002".to_string(),
                step: "review".to_string(),
                approve: false,
            })
        );
    }

    #[test]
    fn parse_pause_canonical() {
        assert_eq!(
            parse("/team pause m-003"),
            Some(TeamCommand::Pause { mission_id: "m-003".to_string() })
        );
    }

    #[test]
    fn parse_resume_canonical() {
        assert_eq!(
            parse("/team resume m-004"),
            Some(TeamCommand::Resume { mission_id: "m-004".to_string() })
        );
    }

    #[test]
    fn parse_abort_canonical() {
        assert_eq!(
            parse("/team abort m-005"),
            Some(TeamCommand::Abort { mission_id: "m-005".to_string() })
        );
    }

    #[test]
    fn parse_rejects_missing_team_token() {
        assert!(parse("/status").is_none());
        assert!(parse("/approve m-001 g-abc").is_none());
        assert!(parse("status").is_none());
    }

    #[test]
    fn parse_rejects_wrong_arg_counts() {
        assert!(parse("/team approve m-001").is_none());
        assert!(parse("/team approve m-001 step extra").is_none());
        assert!(parse("/team pause").is_none());
        assert!(parse("/team pause m-001 extra").is_none());
        assert!(parse("/team status a b").is_none());
    }

    #[test]
    fn parse_rejects_unknown_subcommand() {
        assert!(parse("/team frobnicate m-001").is_none());
        assert!(parse("/team").is_none());
    }

    #[test]
    fn parse_is_case_sensitive() {
        assert!(parse("/TEAM status").is_none());
        assert!(parse("/team STATUS").is_none());
        assert!(parse("/team Approve m-001 s").is_none());
    }

    #[test]
    fn parse_accepts_multiple_spaces_between_tokens() {
        let cmd = parse("/team   approve   m-001   g-abc").unwrap();
        assert_eq!(
            cmd,
            TeamCommand::ResolveGate {
                mission_id: "m-001".to_string(),
                step: "g-abc".to_string(),
                approve: true,
            }
        );
    }

    #[test]
    fn parse_empty_and_unrelated_text_returns_none() {
        assert!(parse("").is_none());
        assert!(parse("   ").is_none());
        assert!(parse("hello team status").is_none());
    }
}
```

- [ ] **Step 2: Register the module**

In `crates/aivyx-channel/src/lib.rs`, immediately after line 181 (`pub mod gate_command;`), add:

```rust
pub mod team_command;
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p aivyx-channel team_command:: -- --test-threads=1`
Expected: all 13 tests in `team_command::tests` PASS (the implementation above is complete, not a stub — this step confirms it compiles and behaves as specified, not a red/green cycle on a stub).

- [ ] **Step 4: Commit**

```bash
git add crates/aivyx-channel/src/team_command.rs crates/aivyx-channel/src/lib.rs
git commit -m "feat(channel): add shared /team command parser (Piece B Task 1)"
```

---

### Task 2: `team_dispatch.rs` — IPC dispatch + reply rendering

**Files:**
- Create: `crates/aivyx-channel/src/team_dispatch.rs`
- Modify: `crates/aivyx-channel/src/lib.rs` (insert `pub mod team_dispatch;` immediately after the `pub mod team_command;` line added in Task 1)

**Interfaces:**
- Consumes: Task 1's `crate::team_command::TeamCommand` (all 5 variants).
- Consumes: existing `crate::daemon_client::{team_mission_list, team_mission_status, resolve_team_gate, abort_team_mission, pause_team_mission, resume_team_mission}` (all `async fn(&Path, ...) -> Result<_, DaemonError>`, signatures reproduced in Step 1 below) and `crate::team_mission::{TeamMissionPhase, TeamMissionRecord}` (re-exports of `aivyx_ipc::team_mission`).
- Produces: `pub async fn dispatch(socket_path: &std::path::Path, cmd: TeamCommand) -> String`, in `crate::team_dispatch`. Tasks 3-5 consume this.

- [ ] **Step 1: Write the failing tests**

Create `crates/aivyx-channel/src/team_dispatch.rs`:

```rust
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
            match daemon_client::resolve_team_gate(socket_path, mission_id, step, approve).await {
                Ok(phase) => format!("✓ Gate {verb}d. Mission now {}.", phase_label(phase)),
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
        assert!(reply.starts_with('✓'));
        assert!(reply.contains("approved"));
        assert!(reply.contains("executing"));

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
}
```

Note: the test module references `crate::daemon_ipc` and `uuid` — both are already dependencies of `aivyx-channel` (used by `daemon_client.rs`'s own existing tests, which this fixture mirrors verbatim) and `aivyx_team` (already a regular, non-dev dependency per `Cargo.toml:116`), so no `Cargo.toml` changes are needed.

- [ ] **Step 2: Register the module**

In `crates/aivyx-channel/src/lib.rs`, immediately after the `pub mod team_command;` line added in Task 1, add:

```rust
pub mod team_dispatch;
```

- [ ] **Step 3: Run the tests**

Run: `cargo test -p aivyx-channel team_dispatch:: -- --test-threads=1`
Expected: all tests in `team_dispatch::tests` PASS (14 tests: 6 pure-render + 8 dispatch). If a dispatch test hangs, check the fake-daemon task drains the client's `Query` write before answering (a full 2048-byte buffer read via `stream.read` before writing the response) — this mirrors the exact shape of `daemon_client.rs`'s own `send_query_skips_recovery_notice_before_the_response` test.

- [ ] **Step 4: Commit**

```bash
git add crates/aivyx-channel/src/team_dispatch.rs crates/aivyx-channel/src/lib.rs
git commit -m "feat(channel): add /team command IPC dispatch + reply rendering (Piece B Task 2)"
```

---

### Task 3: Wire `/team ...` into the Telegram daemon-frontend

**Files:**
- Modify: `crates/aivyx-channel/src/telegram_daemon_frontend.rs:25` (imports), `:229` (insertion point, inside `run_telegram_daemon_chat_task`)

**Interfaces:**
- Consumes: Task 1's `crate::team_command::parse`, Task 2's `crate::team_dispatch::dispatch`.

- [ ] **Step 1: Add the imports**

In `crates/aivyx-channel/src/telegram_daemon_frontend.rs`, immediately after line 25 (`use crate::gate_command;`), add:

```rust
use crate::team_command;
use crate::team_dispatch;
```

- [ ] **Step 2: Insert the command check**

In `run_telegram_daemon_chat_task`, the existing code (lines ~229-250) reads:

```rust
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
                .send_message(OutgoingMessage { chat_id, text: reply })
                .await
                .map_err(|e| DaemonError::Internal(format!(
                    "send_message to chat {chat_id}: {e}"
                )))?;
            continue;
        }
```

Immediately after this block (still before the `// Phase 45 — forward image data...` comment), insert:

```rust
        if let Some(team_cmd) = team_command::parse(msg.text.trim()) {
            let reply = team_dispatch::dispatch(&socket_path, team_cmd).await;
            transport
                .send_message(OutgoingMessage { chat_id, text: reply })
                .await
                .map_err(|e| DaemonError::Internal(format!(
                    "send_message to chat {chat_id}: {e}"
                )))?;
            continue;
        }
```

`socket_path` is already the function's own parameter (`socket_path: PathBuf`, bound at the top of `run_telegram_daemon_chat_task`) — no new parameter needed.

- [ ] **Step 3: Run the full crate test suite to confirm no regressions**

Run: `cargo test -p aivyx-channel -- --test-threads=1`
Expected: PASS, same count as before this task plus the 27 new tests from Tasks 1-2 (no test count regression, no new failures). This task adds no new tests of its own — the wiring is a thin, mechanical two-call sequence already covered end-to-end by Task 2's dispatch tests and Task 1's parser tests; there is no existing per-file harness for testing the async loop itself (confirmed: `discord_daemon_frontend.rs`'s own `mod tests` only unit-tests pure helpers like `render_events_for_discord`, never the loop function), so this task does not invent one.

- [ ] **Step 4: Commit**

```bash
git add crates/aivyx-channel/src/telegram_daemon_frontend.rs
git commit -m "feat(channel): wire /team commands into the Telegram daemon-frontend (Piece B Task 3)"
```

---

### Task 4: Wire `/team ...` into the Discord daemon-frontend

**Files:**
- Modify: `crates/aivyx-channel/src/discord_daemon_frontend.rs:34` (imports), `:229` (insertion point, inside `run_discord_daemon_channel_task`)

**Interfaces:**
- Consumes: Task 1's `crate::team_command::parse`, Task 2's `crate::team_dispatch::dispatch`.

- [ ] **Step 1: Add the imports**

In `crates/aivyx-channel/src/discord_daemon_frontend.rs`, immediately after line 34 (`use crate::gate_command;`), add:

```rust
use crate::team_command;
use crate::team_dispatch;
```

- [ ] **Step 2: Insert the command check**

In `run_discord_daemon_channel_task`, the existing code reads:

```rust
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
                .send_message(OutgoingMessage { channel_id, text: reply })
                .await
                .map_err(|e| {
                    DaemonError::Internal(format!(
                        "send_message to channel {channel_id}: {e}"
                    ))
                })?;
            continue;
        }
```

Immediately after this block, insert:

```rust
        if let Some(team_cmd) = team_command::parse(msg.text.trim()) {
            let reply = team_dispatch::dispatch(&socket_path, team_cmd).await;
            transport
                .send_message(OutgoingMessage { channel_id, text: reply })
                .await
                .map_err(|e| {
                    DaemonError::Internal(format!(
                        "send_message to channel {channel_id}: {e}"
                    ))
                })?;
            continue;
        }
```

`socket_path` is already `run_discord_daemon_channel_task`'s own parameter — no new parameter needed.

- [ ] **Step 3: Run the full crate test suite to confirm no regressions**

Run: `cargo test -p aivyx-channel -- --test-threads=1`
Expected: PASS, same rationale as Task 3 Step 3 — no new tests added by this task.

- [ ] **Step 4: Commit**

```bash
git add crates/aivyx-channel/src/discord_daemon_frontend.rs
git commit -m "feat(channel): wire /team commands into the Discord daemon-frontend (Piece B Task 4)"
```

---

### Task 5: Wire `/team ...` into the Slack daemon-frontend

**Files:**
- Modify: `crates/aivyx-channel/src/slack_daemon_frontend.rs:48` (imports), `:246` (insertion point, inside `run_slack_daemon_partition_task`)

**Interfaces:**
- Consumes: Task 1's `crate::team_command::parse`, Task 2's `crate::team_dispatch::dispatch`.

- [ ] **Step 1: Add the imports**

In `crates/aivyx-channel/src/slack_daemon_frontend.rs`, immediately after line 48 (`use crate::gate_command;`), add:

```rust
use crate::team_command;
use crate::team_dispatch;
```

- [ ] **Step 2: Insert the command check**

In `run_slack_daemon_partition_task`, the existing code reads:

```rust
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
```

Immediately after this block, insert:

```rust
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
```

`socket_path` is already `run_slack_daemon_partition_task`'s own parameter — no new parameter needed.

- [ ] **Step 3: Run the full crate test suite to confirm no regressions**

Run: `cargo test -p aivyx-channel -- --test-threads=1`
Expected: PASS, same rationale as Task 3 Step 3 — no new tests added by this task.

- [ ] **Step 4: Commit**

```bash
git add crates/aivyx-channel/src/slack_daemon_frontend.rs
git commit -m "feat(channel): wire /team commands into the Slack daemon-frontend (Piece B Task 5)"
```

---

## Final Verification

After all 5 tasks:

1. `cargo build --workspace` — clean build.
2. `cargo test -p aivyx-channel -- --test-threads=1` — full crate suite passes (this repo's own established practice: `aivyx-sandbox`-adjacent hangs under full parallelism have shown up before in this workspace's history; single-threaded is the safe default here).
3. `cargo clippy -p aivyx-channel --all-targets -- -D warnings` — check for new warnings. If `trigger.rs:223`'s pre-existing `clippy::result_unit_err` finding (predates this and every other branch in this lineage) is the *only* one, that is not a regression and not this plan's to fix.
4. Grep for any remaining `TODO`/`unimplemented!()` introduced by this plan's files — should be none.
5. Manually trace one full command end-to-end by reading (not running) the final diff: `/team status` typed in Telegram → `telegram_daemon_frontend.rs`'s new block → `team_command::parse` → `team_dispatch::dispatch` → `daemon_client::team_mission_list` → `send_query` → real daemon's `handle_query`'s `QueryPayload::TeamMissionList` arm (unmodified by this plan, already re-verified in this plan's research section) → back through `render_team_list` → `transport.send_message`.
