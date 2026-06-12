//! Aivyx — a Dioxus (Rust→WASM) browser client (Chapter M).
//!
//! Two views over the daemon's `/ws` bridge, speaking the protocol through the
//! shared [`aivyx_ipc`] types (the browser sends `serde_json(FrontendMessage)`
//! and receives `serde_json(DaemonEnvelope)` — what `web_ui.rs` relays):
//!
//! - **Missions** (the headline): a live `TeamMissionList` feed, start-a-mission,
//!   and approve/reject of human gates.
//! - **Chat** (M.6, for parity with the retired single-file page): submit a
//!   turn, render the streamed events, and resolve the single-agent gate.
//!
//! The WebSocket lives in one [`use_coroutine`] task; the poll loop and every UI
//! handler `.send()` a `FrontendMessage` to it, and a sibling read task fans the
//! inbound envelopes into the view signals. Browser behavior is verified when
//! served (M.5) — the in-repo proof is `cargo build --target wasm32` + clippy.

use aivyx_ipc::protocol::{
    DaemonEnvelope, FrontendMessage, QueryPayload, QueryResponsePayload, StreamEventPayload,
};
use aivyx_ipc::{TeamMissionPhase, TeamMissionView};

use dioxus::prelude::*;
use futures_util::{SinkExt, StreamExt};
use gloo_net::websocket::{futures::WebSocket, Message};
use gloo_timers::future::TimeoutFuture;

const POLL_INTERVAL_MS: u32 = 1500;

/// The shared WebSocket-sender handle (poll loop + UI handlers send to it).
type Sender = Coroutine<FrontendMessage>;

/// Top-level view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Missions,
    Chat,
}

/// One rendered chat transcript line.
#[derive(Clone, PartialEq)]
struct ChatLine {
    role: Role,
    text: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Role {
    Operator,
    Assistant,
    System,
    Error,
}

impl ChatLine {
    fn operator(text: String) -> Self {
        Self { role: Role::Operator, text }
    }
    fn assistant(text: String) -> Self {
        Self { role: Role::Assistant, text }
    }
    fn system(text: String) -> Self {
        Self { role: Role::System, text }
    }
    fn error(text: String) -> Self {
        Self { role: Role::Error, text }
    }
    fn class(&self) -> &'static str {
        match self.role {
            Role::Operator => "line op",
            Role::Assistant => "line asst",
            Role::System => "line sys",
            Role::Error => "line err",
        }
    }
}

/// A pending single-agent approval gate.
#[derive(Clone, PartialEq)]
struct GateInfo {
    mission_id: String,
    gate_id: String,
    reason: String,
}

fn main() {
    console_error_panic_hook::set_once();
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    let view = use_signal(|| View::Missions);
    let connected = use_signal(|| false);
    let missions = use_signal(Vec::<TeamMissionView>::new);
    // Chat state, shared with the read task + the Chat view (via context).
    let session = use_signal(|| None::<String>);
    let transcript = use_signal(Vec::<ChatLine>::new);
    let streaming = use_signal(String::new);
    let gate = use_signal(|| None::<GateInfo>);

    let ws: Sender =
        use_coroutine(move |rx| ws_task(rx, missions, connected, session, transcript, streaming, gate));
    use_context_provider(|| ws);
    use_context_provider(|| session);
    use_context_provider(|| transcript);
    use_context_provider(|| streaming);
    use_context_provider(|| gate);

    // Poll the mission feed on the interval through the same socket.
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
            Topbar { view, connected: connected() }
            match view() {
                View::Missions => rsx! { MissionsPanel { missions: missions() } },
                View::Chat => rsx! { ChatPanel {} },
            }
        }
    }
}

#[component]
fn Topbar(view: Signal<View>, connected: bool) -> Element {
    let tab = move |v: View, label: &'static str| {
        let active = view() == v;
        rsx! {
            button {
                class: if active { "tab active" } else { "tab" },
                onclick: move |_| view.set(v),
                "{label}"
            }
        }
    };
    rsx! {
        header { class: "topbar",
            span { class: "brand", "▌ AIVYX" }
            nav { class: "tabs",
                {tab(View::Missions, "Missions")}
                {tab(View::Chat, "Chat")}
            }
            span {
                class: if connected { "dot ok" } else { "dot off" },
                if connected { "● daemon" } else { "○ connecting…" }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Missions view
// ---------------------------------------------------------------------------

#[component]
fn MissionsPanel(missions: Vec<TeamMissionView>) -> Element {
    rsx! {
        NewMissionBar {}
        main { class: "feed",
            if missions.is_empty() {
                p { class: "empty", "No team missions yet — start one above." }
            } else {
                for m in missions.iter() {
                    MissionRow { mission: m.clone() }
                }
            }
        }
    }
}

#[component]
fn NewMissionBar() -> Element {
    let ws = use_context::<Sender>();
    let mut goal = use_signal(String::new);
    rsx! {
        div { class: "newbar",
            input {
                class: "field",
                placeholder: "new mission goal — e.g. \"audit the deps for CVEs\"",
                value: "{goal}",
                oninput: move |e| goal.set(e.value()),
                onkeydown: move |e| {
                    if e.key() == Key::Enter {
                        let g = goal().trim().to_string();
                        if !g.is_empty() { ws.send(start_query(g)); goal.set(String::new()); }
                    }
                },
            }
            button {
                class: "primary",
                onclick: move |_| {
                    let g = goal().trim().to_string();
                    if !g.is_empty() { ws.send(start_query(g)); goal.set(String::new()); }
                },
                "Start"
            }
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
            div { class: "bar", div { class: "fill", style: "width: {pct}%;" } }
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
    let approve = (mission_id.clone(), step.clone());
    let reject = (mission_id.clone(), step.clone());
    let label = step.clone();
    rsx! {
        div { class: "gate",
            span { class: "gate-label", "⚑ awaiting approval: {label}" }
            button {
                class: "ok",
                onclick: move |_| ws.send(resolve_team_query(approve.0.clone(), approve.1.clone(), true)),
                "approve"
            }
            button {
                class: "danger",
                onclick: move |_| ws.send(resolve_team_query(reject.0.clone(), reject.1.clone(), false)),
                "reject"
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Chat view (M.6)
// ---------------------------------------------------------------------------

#[component]
fn ChatPanel() -> Element {
    let ws = use_context::<Sender>();
    let session = use_context::<Signal<Option<String>>>();
    let mut transcript = use_context::<Signal<Vec<ChatLine>>>();
    let streaming = use_context::<Signal<String>>();
    let gate = use_context::<Signal<Option<GateInfo>>>();
    let mut input = use_signal(String::new);
    let ready = session().is_some();

    rsx! {
        main { class: "chat",
            div { class: "transcript",
                for line in transcript().iter() {
                    div { class: "{line.class()}", "{line.text}" }
                }
                if !streaming().is_empty() {
                    div { class: "line asst streaming", "{streaming}" }
                }
                if transcript().is_empty() && streaming().is_empty() {
                    p { class: "empty", "Send a message to start a turn." }
                }
            }
            if let Some(g) = gate() {
                GatePrompt { gate: g }
            } else {
                div { class: "composer",
                    input {
                        class: "field",
                        placeholder: if ready { "message…" } else { "connecting…" },
                        disabled: !ready,
                        value: "{input}",
                        oninput: move |e| input.set(e.value()),
                        onkeydown: move |e| {
                            if e.key() == Key::Enter
                                && let Some(sid) = session() {
                                let text = input().trim().to_string();
                                if !text.is_empty() {
                                    transcript.write().push(ChatLine::operator(text.clone()));
                                    ws.send(submit_query(sid, text));
                                    input.set(String::new());
                                }
                            }
                        },
                    }
                    button {
                        class: "primary",
                        disabled: !ready,
                        onclick: move |_| {
                            if let Some(sid) = session() {
                                let text = input().trim().to_string();
                                if !text.is_empty() {
                                    transcript.write().push(ChatLine::operator(text.clone()));
                                    ws.send(submit_query(sid, text));
                                    input.set(String::new());
                                }
                            }
                        },
                        "Send"
                    }
                }
            }
        }
    }
}

#[component]
fn GatePrompt(gate: GateInfo) -> Element {
    let ws = use_context::<Sender>();
    let mut gate_sig = use_context::<Signal<Option<GateInfo>>>();
    let approve = (gate.mission_id.clone(), gate.gate_id.clone());
    let reject = (gate.mission_id.clone(), gate.gate_id.clone());
    rsx! {
        div { class: "gateprompt",
            span { class: "gate-label", "⚑ approval needed — {gate.reason}" }
            button {
                class: "ok",
                onclick: move |_| {
                    ws.send(resolve_gate_query(approve.0.clone(), approve.1.clone(), true));
                    gate_sig.set(None);
                },
                "approve"
            }
            button {
                class: "danger",
                onclick: move |_| {
                    ws.send(resolve_gate_query(reject.0.clone(), reject.1.clone(), false));
                    gate_sig.set(None);
                },
                "reject"
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Wire helpers + the WebSocket task
// ---------------------------------------------------------------------------

fn start_query(goal: String) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-start".to_string(),
        payload: QueryPayload::TeamRunGoal { goal, config: None },
    }
}

fn resolve_team_query(mission_id: String, step: String, approve: bool) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-gate".to_string(),
        payload: QueryPayload::ResolveTeamGate { mission_id, step, approve },
    }
}

fn resolve_gate_query(mission_id: String, gate_id: String, approved: bool) -> FrontendMessage {
    FrontendMessage::ResolveGate { mission_id, gate_id, approved }
}

fn submit_query(session_id: String, text: String) -> FrontendMessage {
    FrontendMessage::SubmitInput {
        session_id,
        text,
        mission_id: None,
        attachments: Vec::new(),
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

/// The single WebSocket task: open `/ws`, run the read loop (fans inbound
/// envelopes into the view signals), and drain outbound `FrontendMessage`s.
#[allow(clippy::too_many_arguments)]
async fn ws_task(
    mut rx: UnboundedReceiver<FrontendMessage>,
    mut missions: Signal<Vec<TeamMissionView>>,
    mut connected: Signal<bool>,
    mut session: Signal<Option<String>>,
    mut transcript: Signal<Vec<ChatLine>>,
    mut streaming: Signal<String>,
    mut gate: Signal<Option<GateInfo>>,
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

    spawn(async move {
        while let Some(Ok(Message::Text(text))) = read.next().await {
            let Ok(env) = serde_json::from_str::<DaemonEnvelope>(&text) else {
                continue;
            };
            match env {
                DaemonEnvelope::SessionStarted { session_id } => session.set(Some(session_id)),
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::TeamMissionList { missions: records },
                    ..
                } => {
                    missions.set(records.iter().map(|r| r.to_view()).collect());
                }
                DaemonEnvelope::StreamEvent { event, .. } => match event {
                    StreamEventPayload::Text { text } => streaming.write().push_str(&text),
                    StreamEventPayload::Status { status } => {
                        transcript.write().push(ChatLine::system(format!("· {status}")));
                    }
                    StreamEventPayload::ToolCallStarted { tool_name, .. } => {
                        transcript.write().push(ChatLine::system(format!("→ {tool_name}")));
                    }
                    StreamEventPayload::ApprovalGate { mission_id, gate_id, reason, .. } => {
                        gate.set(Some(GateInfo { mission_id, gate_id, reason }));
                    }
                    _ => {}
                },
                DaemonEnvelope::TurnComplete { .. } => {
                    let text = streaming();
                    if !text.is_empty() {
                        transcript.write().push(ChatLine::assistant(text));
                    }
                    streaming.set(String::new());
                }
                DaemonEnvelope::Error { message, .. } => {
                    transcript.write().push(ChatLine::error(message));
                }
                _ => {}
            }
        }
        connected.set(false);
    });

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
.app { max-width: 980px; margin: 0 auto; padding: 0 16px; display:flex; flex-direction:column; height:100vh; }
.topbar { display:flex; align-items:center; gap:14px; padding:14px 0; border-bottom:1px solid var(--border); }
.brand { color:var(--amber); font-weight:700; }
.tabs { display:flex; gap:6px; }
.tab { background:transparent; color:var(--dim); border:0; padding:4px 10px; border-radius:6px; font:inherit; cursor:pointer; }
.tab.active { color:var(--fg); background:#1a1e26; }
.dot { margin-left:auto; font-size:12px; }
.dot.ok { color:var(--ok); } .dot.off { color:var(--dim); }
.newbar { display:flex; gap:8px; padding:14px 0; }
.field { flex:1; background:var(--field); color:var(--fg); border:1px solid var(--border); border-radius:6px; padding:8px 10px; font:inherit; }
.field:focus { outline:none; border-color:var(--lav); }
.primary { background:var(--lav); color:#0d0f12; border:0; border-radius:6px; padding:8px 16px; font:inherit; font-weight:700; cursor:pointer; }
.primary:disabled { opacity:.4; cursor:default; }
.feed { padding:8px 0 24px; display:flex; flex-direction:column; gap:12px; overflow-y:auto; }
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
.gate, .gateprompt { margin-top:10px; display:flex; align-items:center; gap:10px; }
.gateprompt { padding:10px; border:1px solid var(--amber); border-radius:8px; }
.gate-label { color:var(--amber); font-size:13px; flex:1; }
.ok { background:var(--ok); color:#0d0f12; border:0; border-radius:6px; padding:5px 14px; font:inherit; cursor:pointer; }
.danger { background:transparent; color:var(--err); border:1px solid var(--err); border-radius:6px; padding:5px 14px; font:inherit; cursor:pointer; }
.chat { flex:1; display:flex; flex-direction:column; min-height:0; padding:12px 0; }
.transcript { flex:1; overflow-y:auto; display:flex; flex-direction:column; gap:6px; padding-bottom:12px; }
.line { white-space:pre-wrap; }
.line.op { color:var(--fg); }
.line.op::before { content:"› "; color:var(--lav); }
.line.asst { color:var(--fg); }
.line.sys { color:var(--dim); font-size:13px; }
.line.err { color:var(--err); }
.line.streaming::after { content:"▌"; color:var(--lav); }
.composer { display:flex; gap:8px; padding-top:10px; border-top:1px solid var(--border); }
"#;
