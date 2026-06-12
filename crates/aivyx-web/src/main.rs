//! Aivyx Mission-Control — a Dioxus (Rust→WASM) browser client (Chapter M.3).
//!
//! M.3 is the **read-only skeleton**: open the daemon's `/ws` bridge, poll
//! `TeamMissionList` on an interval, and render the mission feed. It speaks the
//! daemon's protocol through the shared [`aivyx_ipc`] types — the browser sends
//! `serde_json(FrontendMessage)` and receives `serde_json(DaemonEnvelope)`,
//! exactly what the WS↔IPC bridge in `aivyx-channel/src/web_ui.rs` relays — so
//! there is no hand-written JSON to drift from the wire format.
//!
//! Interactions (new mission, approve/reject) are M.4; serving the bundle from
//! the daemon + the build/CI wiring are M.5.

use aivyx_ipc::protocol::{
    DaemonEnvelope, FrontendMessage, QueryPayload, QueryResponsePayload,
};
use aivyx_ipc::{TeamMissionPhase, TeamMissionView};

use dioxus::prelude::*;
use futures_util::{SinkExt, StreamExt};
use gloo_net::websocket::{futures::WebSocket, Message};
use gloo_timers::future::TimeoutFuture;

/// How often the feed polls the daemon (ms) — matches the TUI's 1.5s cadence.
const POLL_INTERVAL_MS: u32 = 1500;

fn main() {
    console_error_panic_hook::set_once();
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    // The live mission feed; the WS read task replaces it on each poll response.
    let missions = use_signal(Vec::<TeamMissionView>::new);
    // Connection status for the header.
    let connected = use_signal(|| false);

    // Drive the WebSocket: one read task that updates the feed, one poll task
    // that re-requests TeamMissionList on the interval.
    use_future(move || async move {
        drive_feed(missions, connected).await;
    });

    rsx! {
        style { {STYLE} }
        div { class: "app",
            header { class: "topbar",
                span { class: "brand", "▌ AIVYX" }
                span { class: "title", "Mission Control" }
                span {
                    class: if connected() { "dot ok" } else { "dot off" },
                    if connected() { "● daemon" } else { "○ connecting…" }
                }
            }
            main { class: "feed",
                MissionFeed { missions: missions() }
            }
        }
    }
}

#[component]
fn MissionFeed(missions: Vec<TeamMissionView>) -> Element {
    if missions.is_empty() {
        return rsx! {
            p { class: "empty",
                "No team missions. Start one with `aivyx team start \"<goal>\"`."
            }
        };
    }
    rsx! {
        for m in missions.iter() {
            MissionRow { mission: m.clone() }
        }
    }
}

#[component]
fn MissionRow(mission: TeamMissionView) -> Element {
    let pct = mission.progress.min(100);
    rsx! {
        div { class: "mission",
            div { class: "row1",
                span { class: "phase {phase_class(mission.phase)}", "{phase_label(mission.phase)}" }
                span { class: "goal", "{mission.goal}" }
                span { class: "lead", "{mission.lead}" }
            }
            div { class: "bar",
                div { class: "fill", style: "width: {pct}%;" }
            }
            div { class: "steps",
                for step in mission.steps.iter() {
                    span { class: "step", "{step.label}" }
                }
            }
            if let Some(gate) = mission.pending_gate.as_ref() {
                div { class: "gate", "⚑ awaiting approval: {gate}" }
            }
        }
    }
}

fn phase_label(p: TeamMissionPhase) -> &'static str {
    match p {
        TeamMissionPhase::Planning => "planning",
        TeamMissionPhase::Executing => "executing",
        TeamMissionPhase::AwaitingApproval => "awaiting approval",
        TeamMissionPhase::Done => "done",
        TeamMissionPhase::Rejected => "rejected",
    }
}

fn phase_class(p: TeamMissionPhase) -> &'static str {
    match p {
        TeamMissionPhase::AwaitingApproval => "amber",
        TeamMissionPhase::Done => "ok",
        TeamMissionPhase::Rejected => "err",
        _ => "lav",
    }
}

/// Open `/ws`, then run the read loop (updates `missions`) and the poll loop
/// (sends `TeamMissionList` every [`POLL_INTERVAL_MS`]) concurrently. The
/// daemon's bridge performs the `StartSession(Web)` handshake server-side, so
/// the client only opens the socket and relays JSON.
async fn drive_feed(
    mut missions: Signal<Vec<TeamMissionView>>,
    mut connected: Signal<bool>,
) {
    let ws = match WebSocket::open(&ws_url()) {
        Ok(ws) => ws,
        Err(_) => {
            connected.set(false);
            return;
        }
    };
    connected.set(true);
    let (mut write, mut read) = ws.split();

    // Read task: parse inbound DaemonEnvelopes; on a TeamMissionList response,
    // project the records to views and replace the feed.
    spawn(async move {
        while let Some(Ok(Message::Text(text))) = read.next().await {
            let Ok(env) = serde_json::from_str::<DaemonEnvelope>(&text) else {
                continue;
            };
            if let DaemonEnvelope::QueryResponse { payload, .. } = env
                && let QueryResponsePayload::TeamMissionList { missions: records } = payload
            {
                missions.set(records.iter().map(|r| r.to_view()).collect());
            }
        }
        connected.set(false);
    });

    // Poll loop: request the feed on the interval until the socket closes.
    let mut seq: u64 = 0;
    loop {
        let query = FrontendMessage::Query {
            id: format!("mc-{seq}"),
            payload: QueryPayload::TeamMissionList,
        };
        match serde_json::to_string(&query) {
            Ok(json) => {
                if write.send(Message::Text(json)).await.is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
        seq += 1;
        TimeoutFuture::new(POLL_INTERVAL_MS).await;
    }
}

/// Build the `ws[s]://<host>/ws` URL from the page's location.
fn ws_url() -> String {
    let location = web_sys::window().expect("window").location();
    let scheme = match location.protocol().as_deref() {
        Ok("https:") => "wss",
        _ => "ws",
    };
    let host = location.host().unwrap_or_else(|_| "127.0.0.1".to_string());
    format!("{scheme}://{host}/ws")
}

const STYLE: &str = r#"
:root { --bg:#0d0f12; --fg:#e6e6e6; --dim:#7a8290; --amber:#e0a458; --lav:#9b8cff; --ok:#5fd07a; --err:#e0566a; --border:#222730; }
* { box-sizing: border-box; }
body { margin:0; background:var(--bg); color:var(--fg); font: 14px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace; }
.app { max-width: 980px; margin: 0 auto; padding: 0 16px; }
.topbar { display:flex; align-items:center; gap:14px; padding:14px 0; border-bottom:1px solid var(--border); }
.brand { color:var(--amber); font-weight:700; }
.title { color:var(--fg); }
.dot { margin-left:auto; font-size:12px; }
.dot.ok { color:var(--ok); } .dot.off { color:var(--dim); }
.feed { padding:16px 0; display:flex; flex-direction:column; gap:12px; }
.empty { color:var(--dim); }
.mission { border:1px solid var(--border); border-radius:8px; padding:12px; }
.row1 { display:flex; gap:12px; align-items:baseline; }
.phase { font-size:12px; text-transform:uppercase; }
.phase.amber{color:var(--amber);} .phase.ok{color:var(--ok);} .phase.err{color:var(--err);} .phase.lav{color:var(--lav);}
.goal { flex:1; }
.lead { color:var(--dim); font-size:12px; }
.bar { height:6px; background:#1a1e26; border-radius:3px; margin:8px 0; overflow:hidden; }
.fill { height:100%; background:var(--lav); }
.steps { display:flex; flex-wrap:wrap; gap:8px; }
.step { font-size:12px; color:var(--dim); }
.gate { margin-top:8px; color:var(--amber); font-size:13px; }
"#;
