//! Aivyx Mission-Control — a Dioxus (Rust→WASM) browser client (Chapter M).
//!
//! M.3 was the read-only feed; M.4 adds **interactions** — start a mission from
//! a goal, and approve/reject a mission paused at a human gate — all over the
//! same `/ws` bridge, speaking the daemon's protocol through the shared
//! [`aivyx_ipc`] types (the browser sends `serde_json(FrontendMessage)` and
//! receives `serde_json(DaemonEnvelope)`, exactly what `web_ui.rs` relays).
//!
//! The WebSocket lives in a single [`use_coroutine`] task: the poll loop and
//! every UI handler `.send()` a `FrontendMessage` to it, and it drains them to
//! the socket; a sibling read task projects `TeamMissionList` responses into
//! the feed signal. The coroutine handle is shared via context so any row can
//! act. Chat-view parity + the read-only panels are later (M.4b/M.6).

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

/// The shared WebSocket-sender handle: the poll loop and UI handlers send
/// `FrontendMessage`s to it; the coroutine drains them to the socket.
type Sender = Coroutine<FrontendMessage>;

fn main() {
    console_error_panic_hook::set_once();
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    let missions = use_signal(Vec::<TeamMissionView>::new);
    let connected = use_signal(|| false);

    // The one WebSocket task: drains outbound FrontendMessages, drives the read
    // loop. Its handle is `Copy`, shared with the poll loop + (via context) rows.
    let ws: Sender = use_coroutine(move |rx| ws_task(rx, missions, connected));
    use_context_provider(|| ws);

    // Poll the feed on the interval by sending through the same socket.
    use_future(move || async move {
        loop {
            ws.send(FrontendMessage::Query {
                id: "mc-poll".to_string(),
                payload: QueryPayload::TeamMissionList,
            });
            TimeoutFuture::new(POLL_INTERVAL_MS).await;
        }
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
            NewMissionBar {}
            main { class: "feed",
                MissionFeed { missions: missions() }
            }
        }
    }
}

#[component]
fn NewMissionBar() -> Element {
    let ws = use_context::<Sender>();
    let mut goal = use_signal(String::new);

    // `ws` (Coroutine) + `goal` (Signal) are `Copy`, so each `move` handler
    // captures its own copy — the submit logic is inlined rather than shared
    // through one FnMut closure (which can't be moved into two handlers).
    rsx! {
        div { class: "newbar",
            input {
                class: "goal-input",
                placeholder: "new mission goal — e.g. \"audit the deps for CVEs\"",
                value: "{goal}",
                oninput: move |e| goal.set(e.value()),
                onkeydown: move |e| {
                    if e.key() == Key::Enter {
                        let g = goal().trim().to_string();
                        if !g.is_empty() {
                            ws.send(start_query(g));
                            goal.set(String::new());
                        }
                    }
                },
            }
            button {
                class: "start",
                onclick: move |_| {
                    let g = goal().trim().to_string();
                    if !g.is_empty() {
                        ws.send(start_query(g));
                        goal.set(String::new());
                    }
                },
                "Start"
            }
        }
    }
}

/// `team run "<goal>"` over the wire (default team; pack selection is later).
fn start_query(goal: String) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-start".to_string(),
        payload: QueryPayload::TeamRunGoal { goal, config: None },
    }
}

/// Approve/reject a mission's pending human gate.
fn resolve_query(mission_id: String, step: String, approve: bool) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-gate".to_string(),
        payload: QueryPayload::ResolveTeamGate { mission_id, step, approve },
    }
}

#[component]
fn MissionFeed(missions: Vec<TeamMissionView>) -> Element {
    if missions.is_empty() {
        return rsx! {
            p { class: "empty", "No team missions yet — start one above." }
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
    let awaiting = mission.phase == TeamMissionPhase::AwaitingApproval;

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
            if awaiting {
                if let Some(gate) = mission.pending_gate.clone() {
                    GateControls { mission_id: mission.id.clone(), step: gate }
                }
            }
        }
    }
}

#[component]
fn GateControls(mission_id: String, step: String) -> Element {
    let ws = use_context::<Sender>();
    // `mission_id`/`step` are `String` (not Copy) — give each handler its own
    // pair to move-capture; clone again per call (handlers are FnMut).
    let approve = (mission_id.clone(), step.clone());
    let reject = (mission_id.clone(), step.clone());
    let label = step.clone();
    rsx! {
        div { class: "gate",
            span { class: "gate-label", "⚑ awaiting approval: {label}" }
            button {
                class: "approve",
                onclick: move |_| ws.send(resolve_query(approve.0.clone(), approve.1.clone(), true)),
                "approve"
            }
            button {
                class: "reject",
                onclick: move |_| ws.send(resolve_query(reject.0.clone(), reject.1.clone(), false)),
                "reject"
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

/// The single WebSocket task: open `/ws`, run the read loop (projects
/// `TeamMissionList` responses into the feed), and drain outbound
/// `FrontendMessage`s (poll queries + UI actions) to the socket. The daemon's
/// bridge does the `StartSession(Web)` handshake server-side.
async fn ws_task(
    mut rx: UnboundedReceiver<FrontendMessage>,
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

    // Read task: inbound DaemonEnvelopes → feed signal.
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

    // Outbound: drain every FrontendMessage sent to the coroutine.
    while let Some(msg) = rx.next().await {
        match serde_json::to_string(&msg) {
            Ok(json) => {
                if write.send(Message::Text(json)).await.is_err() {
                    break;
                }
            }
            Err(_) => continue,
        }
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
:root { --bg:#0d0f12; --fg:#e6e6e6; --dim:#7a8290; --amber:#e0a458; --lav:#9b8cff; --ok:#5fd07a; --err:#e0566a; --border:#222730; --field:#161a21; }
* { box-sizing: border-box; }
body { margin:0; background:var(--bg); color:var(--fg); font: 14px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace; }
.app { max-width: 980px; margin: 0 auto; padding: 0 16px; }
.topbar { display:flex; align-items:center; gap:14px; padding:14px 0; border-bottom:1px solid var(--border); }
.brand { color:var(--amber); font-weight:700; }
.dot { margin-left:auto; font-size:12px; }
.dot.ok { color:var(--ok); } .dot.off { color:var(--dim); }
.newbar { display:flex; gap:8px; padding:14px 0; }
.goal-input { flex:1; background:var(--field); color:var(--fg); border:1px solid var(--border); border-radius:6px; padding:8px 10px; font:inherit; }
.goal-input:focus { outline:none; border-color:var(--lav); }
.start { background:var(--lav); color:#0d0f12; border:0; border-radius:6px; padding:8px 16px; font:inherit; font-weight:700; cursor:pointer; }
.feed { padding:8px 0 24px; display:flex; flex-direction:column; gap:12px; }
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
.gate { margin-top:10px; display:flex; align-items:center; gap:10px; }
.gate-label { color:var(--amber); font-size:13px; flex:1; }
.approve { background:var(--ok); color:#0d0f12; border:0; border-radius:6px; padding:5px 14px; font:inherit; cursor:pointer; }
.reject { background:transparent; color:var(--err); border:1px solid var(--err); border-radius:6px; padding:5px 14px; font:inherit; cursor:pointer; }
"#;
