//! Aivyx Studio — a Dioxus (Rust→WASM) browser client (Chapter M; reskinned to
//! the **Stitch** design system in Chapter R).
//!
//! Two live views over the daemon's `/ws` bridge, speaking the protocol through
//! the shared [`aivyx_ipc`] types (the browser sends `serde_json(FrontendMessage)`
//! and receives `serde_json(DaemonEnvelope)` — what `web_ui.rs` relays):
//!
//! - **Missions**: a live `TeamMissionList` feed, start-a-mission, approve/reject
//!   of human gates — the Mission-Orchestration look.
//! - **Chat**: submit a turn, render streamed events, resolve the single-agent
//!   gate — the Terminal look.
//!
//! Chapter R is **presentation-only**: the shell (Sidebar / Topbar / StatusBar),
//! the Stitch token CSS, self-hosted fonts and brand icons, and a small component
//! kit. The WebSocket task, the `aivyx_ipc` data flow, and every handler are
//! unchanged from Chapter M. Stitch tokens are the single source of truth
//! (`aivyx-brand/design-tokens.md`); see `docs/FRONTEND.md`.

use aivyx_ipc::protocol::{
    AuditEntrySummary, DaemonEnvelope, FrontendMessage, MemoryEntrySummary, QueryPayload,
    QueryResponsePayload, StreamEventPayload,
};
use aivyx_ipc::{TeamMissionPhase, TeamMissionView};

/// How many recent audit entries the Command Center feed shows.
const AUDIT_FEED_N: u32 = 8;
/// Page size for memory topic-entry and search queries.
const MEMORY_LIMIT: u32 = 50;

use dioxus::prelude::*;
use futures_util::{SinkExt, StreamExt};
use gloo_net::websocket::{futures::WebSocket, Message};
use gloo_timers::future::TimeoutFuture;

const POLL_INTERVAL_MS: u32 = 1500;

// ── Bundled assets (every asset goes through `asset!()` so it lands in the
//    bundle and is served offline by the daemon — no CDN). ──────────────────
const STITCH_CSS: Asset = asset!("/assets/stitch.css");
const FAVICON: Asset = asset!("/assets/logos/aivyx-favicon.svg");
const LOGOMARK: Asset = asset!("/assets/logos/aivyx-logomark.svg");
const FONT_DISPLAY: Asset = asset!("/assets/fonts/space-grotesk-var.woff2");
const FONT_BODY: Asset = asset!("/assets/fonts/inter-var.woff2");
const FONT_MONO: Asset = asset!("/assets/fonts/jetbrains-mono-var.woff2");
const ICON_COMMAND: Asset = asset!("/assets/icons/command-center.svg");
const ICON_CHAT: Asset = asset!("/assets/icons/chat.svg");
const ICON_MISSIONS: Asset = asset!("/assets/icons/missions.svg");
const ICON_TEAMS: Asset = asset!("/assets/icons/teams.svg");
const ICON_AGENTS: Asset = asset!("/assets/icons/agents.svg");
const ICON_MEMORY: Asset = asset!("/assets/icons/memory.svg");
const ICON_SETTINGS: Asset = asset!("/assets/icons/settings.svg");
const ICON_THEME: Asset = asset!("/assets/icons/theme-toggle.svg");

/// The shared WebSocket-sender handle (poll loop + UI handlers send to it).
type Sender = Coroutine<FrontendMessage>;

/// Top-level view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Command,
    Missions,
    Chat,
    Memory,
}

/// Memory browser state — read-only snapshots fanned in by `ws_task`.
#[derive(Clone, Default, PartialEq)]
struct MemoryState {
    topics: Vec<String>,
    entries: Vec<MemoryEntrySummary>,
    /// True when a semantic search was transparently served by the keyword path.
    fell_back: bool,
}

/// Command Center dashboard state — read-only snapshots fanned in by `ws_task`.
#[derive(Clone, Default, PartialEq)]
struct Dashboard {
    audit_entries: Vec<AuditEntrySummary>,
    audit_total: u64,
    chain_ok: Option<bool>,
    assistant_name: Option<String>,
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

/// Set `data-theme` on `<html>` so the `[data-theme="light"]` token overrides
/// cascade to `:root` + `body` (dark is the default — no attribute).
fn apply_theme(light: bool) {
    if let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.document_element())
    {
        let _ = el.set_attribute("data-theme", if light { "light" } else { "dark" });
    }
}

/// `@font-face` rules built from the hashed asset paths (so the fonts always
/// resolve to the bundled, offline woff2 — not a CDN).
fn font_faces() -> String {
    format!(
        "@font-face{{font-family:'Space Grotesk';src:url('{FONT_DISPLAY}') format('woff2');font-weight:300 700;font-display:swap;}}\
         @font-face{{font-family:'Inter';src:url('{FONT_BODY}') format('woff2');font-weight:100 900;font-display:swap;}}\
         @font-face{{font-family:'JetBrains Mono';src:url('{FONT_MONO}') format('woff2');font-weight:100 800;font-display:swap;}}"
    )
}

#[component]
fn App() -> Element {
    let view = use_signal(|| View::Command);
    let connected = use_signal(|| false);
    let light = use_signal(|| false);
    let missions = use_signal(Vec::<TeamMissionView>::new);
    let dashboard = use_signal(Dashboard::default);
    let memory = use_signal(MemoryState::default);
    // Chat state, shared with the read task + the Chat view (via context).
    let session = use_signal(|| None::<String>);
    let transcript = use_signal(Vec::<ChatLine>::new);
    let streaming = use_signal(String::new);
    let gate = use_signal(|| None::<GateInfo>);

    let ws: Sender = use_coroutine(move |rx| {
        ws_task(rx, missions, dashboard, memory, connected, session, transcript, streaming, gate)
    });
    use_context_provider(|| ws);
    use_context_provider(|| memory);
    use_context_provider(|| session);
    use_context_provider(|| transcript);
    use_context_provider(|| streaming);
    use_context_provider(|| gate);

    // Reflect the theme signal onto `<html data-theme>`.
    use_effect(move || apply_theme(light()));

    // Live poll: mission feed + the newest audit tail, every interval. The audit
    // window self-corrects to the newest entries once `audit_total` is known.
    use_future(move || async move {
        loop {
            ws.send(FrontendMessage::Query {
                id: "mc-poll".to_string(),
                payload: QueryPayload::TeamMissionList,
            });
            let from_seq = dashboard().audit_total.saturating_sub(AUDIT_FEED_N as u64);
            ws.send(FrontendMessage::Query {
                id: "mc-audit".to_string(),
                payload: QueryPayload::ListAuditEntries { from_seq, limit: AUDIT_FEED_N },
            });
            TimeoutFuture::new(POLL_INTERVAL_MS).await;
        }
    });

    // One-shot: the agent profile + a chain integrity check for the dashboard.
    use_future(move || async move {
        ws.send(FrontendMessage::Query {
            id: "mc-profile".to_string(),
            payload: QueryPayload::GetProfile,
        });
        ws.send(FrontendMessage::Query {
            id: "mc-verify".to_string(),
            payload: QueryPayload::VerifyAuditChain,
        });
    });

    let title = match view() {
        View::Command => "Command Center",
        View::Missions => "Mission Orchestration",
        View::Chat => "Terminal",
        View::Memory => "Memory",
    };

    rsx! {
        document::Title { "Aivyx Studio" }
        document::Link { rel: "icon", href: FAVICON }
        document::Stylesheet { href: STITCH_CSS }
        style { {font_faces()} }
        div { class: "app",
            Sidebar { view }
            div { class: "main",
                Topbar { title, connected: connected(), light }
                div { class: "view fade-in",
                    match view() {
                        View::Command => rsx! {
                            CommandPanel { missions: missions(), dashboard: dashboard(), connected: connected() }
                        },
                        View::Missions => rsx! { MissionsPanel { missions: missions() } },
                        View::Chat => rsx! { ChatPanel {} },
                        View::Memory => rsx! { MemoryPanel {} },
                    }
                }
            }
            StatusBar { connected: connected() }
        }
    }
}

// ---------------------------------------------------------------------------
// App shell — Sidebar / Topbar / StatusBar
// ---------------------------------------------------------------------------

#[component]
fn Sidebar(view: Signal<View>) -> Element {
    rsx! {
        aside { class: "sidebar",
            div { class: "brand-lockup",
                img { src: LOGOMARK, alt: "Aivyx" }
                span { class: "wordmark", "AIVYX" }
            }
            NavItem { icon: ICON_COMMAND, label: "Command", active: view() == View::Command,
                onclick: move |_| view.set(View::Command) }
            NavItem { icon: ICON_MISSIONS, label: "Missions", active: view() == View::Missions,
                onclick: move |_| view.set(View::Missions) }
            NavItem { icon: ICON_CHAT, label: "Chat", active: view() == View::Chat,
                onclick: move |_| view.set(View::Chat) }
            NavItem { icon: ICON_MEMORY, label: "Memory", active: view() == View::Memory,
                onclick: move |_| view.set(View::Memory) }
            div { class: "nav-section label-tech", "Roadmap" }
            NavItemSoon { icon: ICON_TEAMS, label: "Teams" }
            NavItemSoon { icon: ICON_AGENTS, label: "Agents" }
            NavItemSoon { icon: ICON_SETTINGS, label: "Settings" }
            div { style: "flex:1" }
            a { class: "nav-item", href: "/classic", "▸ Classic UI ↗" }
        }
    }
}

#[component]
fn NavItem(icon: Asset, label: &'static str, active: bool, onclick: EventHandler<MouseEvent>) -> Element {
    rsx! {
        button {
            class: if active { "nav-item active" } else { "nav-item" },
            onclick: move |e| onclick.call(e),
            span { class: "ico", style: "--ico: url({icon})" }
            "{label}"
        }
    }
}

#[component]
fn NavItemSoon(icon: Asset, label: &'static str) -> Element {
    rsx! {
        div { class: "nav-item disabled",
            span { class: "ico", style: "--ico: url({icon})" }
            "{label}"
            span { class: "soon", "soon" }
        }
    }
}

#[component]
fn Topbar(title: &'static str, connected: bool, light: Signal<bool>) -> Element {
    rsx! {
        header { class: "topbar",
            span { class: "title", "{title}" }
            div { class: "spacer" }
            div { class: if connected { "status-dot live" } else { "status-dot" },
                span { class: "beacon" }
                if connected { "daemon online" } else { "connecting…" }
            }
            button {
                class: "icon-btn",
                title: "Toggle theme",
                onclick: move |_| light.toggle(),
                span { class: "ico", style: "--ico: url({ICON_THEME})" }
            }
        }
    }
}

#[component]
fn StatusBar(connected: bool) -> Element {
    rsx! {
        footer { class: "statusbar label-tech",
            div { class: if connected { "seg live" } else { "seg" },
                span { class: "dot" }
                if connected { "DAEMON · CONNECTED" } else { "DAEMON · OFFLINE" }
            }
            div { class: "seg", "AGENT · NONAGON" }
            div { class: "seg", "STITCH · v0.1.0" }
        }
    }
}

// ---------------------------------------------------------------------------
// Command Center — the home dashboard (read-only)
// ---------------------------------------------------------------------------

#[component]
fn CommandPanel(missions: Vec<TeamMissionView>, dashboard: Dashboard, connected: bool) -> Element {
    let active = missions
        .iter()
        .filter(|m| {
            !matches!(m.phase, TeamMissionPhase::Done | TeamMissionPhase::Rejected)
        })
        .count();
    let chain = dashboard.chain_ok;
    rsx! {
        div { class: "stat-row",
            StatCard { icon: ICON_MISSIONS, label: "Missions", value: "{missions.len()}", tone: None }
            StatCard { icon: ICON_COMMAND, label: "Audit Events", value: "{dashboard.audit_total}", tone: None }
            StatCard { icon: ICON_AGENTS, label: "Active", value: "{active}", tone: None }
            StatCard { icon: ICON_MEMORY, label: "Chain", value: chain_label(chain).to_string(), tone: chain_tone(chain) }
        }
        div { class: "dash-grid",
            div { class: "dash-main",
                section { class: "panel",
                    div { class: "panel-head",
                        h3 { "Active Missions" }
                        span { class: "label-tech", "{missions.len()} total" }
                    }
                    if missions.is_empty() {
                        div { class: "glass-card empty", p { class: "label-tech", "No missions yet — start one from the Missions tab." } }
                    } else {
                        div { class: "feed",
                            for m in missions.iter().take(4) {
                                DashMissionRow { mission: m.clone() }
                            }
                        }
                    }
                }
                section { class: "panel",
                    div { class: "panel-head",
                        h3 { "Audit Trail" }
                        span { class: "label-tech", "newest {AUDIT_FEED_N}" }
                    }
                    AuditFeed { entries: dashboard.audit_entries.clone() }
                }
            }
            aside { class: "dash-rail",
                AgentStatus { name: dashboard.assistant_name.clone(), connected, chain_ok: chain }
            }
        }
    }
}

#[component]
fn StatCard(icon: Asset, label: &'static str, value: String, tone: Option<&'static str>) -> Element {
    rsx! {
        div { class: "glass-card stat-card",
            div { class: "stat-top",
                span { class: "ico", style: "--ico: url({icon})" }
                span { class: "label-tech", "{label}" }
            }
            span {
                class: if let Some(t) = tone { "value {t}" } else { "value" },
                "{value}"
            }
        }
    }
}

#[component]
fn DashMissionRow(mission: TeamMissionView) -> Element {
    let pct = mission.progress.min(100);
    rsx! {
        div { class: "glass-card dash-mission",
            div { class: "row1",
                span { class: "chip {phase_class(mission.phase)}", "{phase_label(mission.phase)}" }
                span { class: "goal", "{mission.goal}" }
            }
            div { class: "progress", div { class: "fill", style: "width: {pct}%;" } }
        }
    }
}

#[component]
fn AuditFeed(entries: Vec<AuditEntrySummary>) -> Element {
    rsx! {
        div { class: "audit-feed",
            if entries.is_empty() {
                p { class: "label-tech empty", "No audit events yet." }
            } else {
                // entries arrive oldest→newest; show newest first.
                for e in entries.iter().rev() {
                    div { class: "audit-row",
                        span { class: "ev", "{e.event_type}" }
                        span { class: "when label-tech", "{rel_time(e.appended_at_unix_ms)}" }
                        span { class: "seq label-tech", "#{e.seq}" }
                    }
                }
            }
        }
    }
}

#[component]
fn AgentStatus(name: Option<String>, connected: bool, chain_ok: Option<bool>) -> Element {
    let agent = name.unwrap_or_else(|| "—".to_string());
    let chain_class = match chain_tone(chain_ok) {
        Some(t) => format!("v {t}"),
        None => "v".to_string(),
    };
    let chain = chain_label(chain_ok);
    rsx! {
        section { class: "glass-card agent-status",
            div { class: "panel-head", h3 { "System" } }
            div { class: "kv",
                span { class: "label-tech", "Agent" }
                span { class: "v", "{agent}" }
            }
            div { class: "kv",
                span { class: "label-tech", "Daemon" }
                span { class: if connected { "v ok" } else { "v off" },
                    if connected { "online" } else { "offline" }
                }
            }
            div { class: "kv",
                span { class: "label-tech", "Chain" }
                span { class: "{chain_class}", "{chain}" }
            }
            div { class: "kv",
                span { class: "label-tech", "Design" }
                span { class: "v", "Stitch" }
            }
        }
    }
}

/// "Secure" / "FAILED" / "…" for the chain status.
fn chain_label(ok: Option<bool>) -> &'static str {
    match ok {
        Some(true) => "Secure",
        Some(false) => "FAILED",
        None => "…",
    }
}

fn chain_tone(ok: Option<bool>) -> Option<&'static str> {
    match ok {
        Some(true) => Some("ok"),
        Some(false) => Some("off"),
        None => None,
    }
}

/// Relative time from a unix-ms timestamp, using the browser clock.
fn rel_time(ms: u64) -> String {
    let now = js_sys::Date::now() as u64;
    if ms == 0 || ms >= now {
        return "now".to_string();
    }
    let secs = (now - ms) / 1000;
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

// ---------------------------------------------------------------------------
// Missions view — the orchestration look
// ---------------------------------------------------------------------------

#[component]
fn MissionsPanel(missions: Vec<TeamMissionView>) -> Element {
    rsx! {
        NewMissionBar {}
        div { class: "feed",
            if missions.is_empty() {
                div { class: "empty card",
                    p { "No team missions yet." }
                    p { class: "label-tech", "Start one above to dispatch the Nonagon." }
                }
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
                class: "input",
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
                class: "btn btn-primary",
                onclick: move |_| {
                    let g = goal().trim().to_string();
                    if !g.is_empty() { ws.send(start_query(g)); goal.set(String::new()); }
                },
                "Run"
            }
        }
    }
}

#[component]
fn MissionRow(mission: TeamMissionView) -> Element {
    let pct = mission.progress.min(100);
    let awaiting = mission.phase == TeamMissionPhase::AwaitingApproval;
    rsx! {
        div { class: "glass-card mission",
            div { class: "row1",
                span { class: "chip {phase_class(mission.phase)}", "{phase_label(mission.phase)}" }
                span { class: "goal", "{mission.goal}" }
                span { class: "lead label-tech", "{mission.lead}" }
            }
            div { class: "progress", div { class: "fill", style: "width: {pct}%;" } }
            div { class: "steps",
                for step in mission.steps.iter() {
                    span { class: "step label-tech", "{step.label}" }
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
            span { class: "gate-label", "⚑ awaiting approval — {label}" }
            button {
                class: "btn btn-sage",
                onclick: move |_| ws.send(resolve_team_query(approve.0.clone(), approve.1.clone(), true)),
                "Approve Sequence"
            }
            button {
                class: "btn btn-ghost-danger",
                onclick: move |_| ws.send(resolve_team_query(reject.0.clone(), reject.1.clone(), false)),
                "Reject"
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Chat view — the terminal look
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
        div { class: "chat",
            div { class: "transcript",
                for line in transcript().iter() {
                    div { class: "{line.class()}", "{line.text}" }
                }
                if !streaming().is_empty() {
                    div { class: "line asst streaming", "{streaming}" }
                }
                if transcript().is_empty() && streaming().is_empty() {
                    p { class: "empty label-tech", "Send a message to start a turn." }
                }
            }
            if let Some(g) = gate() {
                GatePrompt { gate: g }
            } else {
                div { class: "composer",
                    input {
                        class: "input",
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
                        class: "btn btn-primary",
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
        div { class: "glass-card gateprompt",
            span { class: "gate-label", "⚑ approval needed — {gate.reason}" }
            button {
                class: "btn btn-sage",
                onclick: move |_| {
                    ws.send(resolve_gate_query(approve.0.clone(), approve.1.clone(), true));
                    gate_sig.set(None);
                },
                "Approve"
            }
            button {
                class: "btn btn-ghost-danger",
                onclick: move |_| {
                    ws.send(resolve_gate_query(reject.0.clone(), reject.1.clone(), false));
                    gate_sig.set(None);
                },
                "Reject"
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Memory view — the self-learning knowledge browser (read-only)
// ---------------------------------------------------------------------------

#[component]
fn MemoryPanel() -> Element {
    let ws = use_context::<Sender>();
    let memory = use_context::<Signal<MemoryState>>();
    let mut query = use_signal(String::new);
    let mut semantic = use_signal(|| false);
    // Active scope label: "recent" | "topic:<t>" | "search:<q>".
    let mut scope = use_signal(|| "recent".to_string());

    // Load topics + the recent-across-all default each time the view opens.
    use_future(move || async move {
        ws.send(mem_topics_query());
        ws.send(mem_search_query(String::new(), false));
    });

    let m = memory();
    rsx! {
        div { class: "mem",
            aside { class: "mem-rail",
                div { class: "panel-head", h3 { "Topics" } span { class: "label-tech", "{m.topics.len()}" } }
                button {
                    class: if scope() == "recent" { "mem-topic active" } else { "mem-topic" },
                    onclick: move |_| {
                        scope.set("recent".to_string());
                        ws.send(mem_search_query(String::new(), semantic()));
                    },
                    "Recent · all"
                }
                for t in m.topics.iter() {
                    {
                        let topic = t.clone();
                        let label = t.clone();
                        let sel = scope() == format!("topic:{t}");
                        rsx! {
                            button {
                                class: if sel { "mem-topic active" } else { "mem-topic" },
                                onclick: move |_| {
                                    scope.set(format!("topic:{topic}"));
                                    ws.send(mem_topic_query(topic.clone()));
                                },
                                "{label}"
                            }
                        }
                    }
                }
            }
            div { class: "mem-main",
                div { class: "mem-search",
                    input {
                        class: "input",
                        placeholder: "search memory…",
                        value: "{query}",
                        oninput: move |e| query.set(e.value()),
                        onkeydown: move |e| {
                            if e.key() == Key::Enter {
                                let q = query();
                                scope.set(format!("search:{q}"));
                                ws.send(mem_search_query(q, semantic()));
                            }
                        },
                    }
                    button {
                        class: if semantic() { "btn btn-glass on" } else { "btn btn-glass" },
                        title: "Toggle keyword / semantic search",
                        onclick: move |_| semantic.toggle(),
                        if semantic() { "semantic" } else { "keyword" }
                    }
                    button {
                        class: "btn btn-primary",
                        onclick: move |_| {
                            let q = query();
                            scope.set(format!("search:{q}"));
                            ws.send(mem_search_query(q, semantic()));
                        },
                        "Search"
                    }
                }
                div { class: "panel-head",
                    h3 { "{scope_label(&scope())}" }
                    if m.fell_back {
                        span { class: "chip amber", "keyword fallback" }
                    }
                }
                if m.entries.is_empty() {
                    div { class: "glass-card empty",
                        p { class: "label-tech", "No memory here yet — the agent writes memories as it learns what matters to you." }
                    }
                } else {
                    div { class: "mem-entries",
                        for e in m.entries.iter() {
                            MemoryEntry { entry: e.clone() }
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn MemoryEntry(entry: MemoryEntrySummary) -> Element {
    rsx! {
        div { class: "glass-card mem-entry",
            div { class: "mem-entry-head",
                span { class: "chip", "{entry.topic}" }
                span { class: "when label-tech", "{rel_time_secs(entry.created_at_secs)}" }
                span { class: "seq label-tech", "#{entry.seq}" }
            }
            p { class: "mem-body", "{entry.body}" }
        }
    }
}

fn mem_topics_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-mem-topics".to_string(),
        payload: QueryPayload::ListMemoryTopics,
    }
}

fn mem_topic_query(topic: String) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-mem-topic".to_string(),
        payload: QueryPayload::GetMemoryTopicEntries { topic, limit: MEMORY_LIMIT },
    }
}

fn mem_search_query(query: String, semantic: bool) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-mem-search".to_string(),
        payload: QueryPayload::SearchMemory { query, limit: MEMORY_LIMIT, semantic },
    }
}

/// Human label for the active memory scope.
fn scope_label(scope: &str) -> String {
    if scope == "recent" {
        "Recent · all topics".to_string()
    } else if let Some(t) = scope.strip_prefix("topic:") {
        format!("Topic · {t}")
    } else if let Some(q) = scope.strip_prefix("search:") {
        if q.is_empty() {
            "Recent · all topics".to_string()
        } else {
            format!("Search · \"{q}\"")
        }
    } else {
        scope.to_string()
    }
}

/// `rel_time` for a unix-**seconds** timestamp (memory entries store seconds).
fn rel_time_secs(secs: u64) -> String {
    rel_time(secs.saturating_mul(1000))
}

// ---------------------------------------------------------------------------
// Wire helpers + the WebSocket task (unchanged from Chapter M)
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
        headless: false,
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
        TeamMissionPhase::Done => "sage",
        TeamMissionPhase::Rejected => "error",
        _ => "",
    }
}

/// The single WebSocket task: open `/ws`, run the read loop (fans inbound
/// envelopes into the view signals), and drain outbound `FrontendMessage`s.
#[allow(clippy::too_many_arguments)]
async fn ws_task(
    mut rx: UnboundedReceiver<FrontendMessage>,
    mut missions: Signal<Vec<TeamMissionView>>,
    mut dashboard: Signal<Dashboard>,
    mut memory: Signal<MemoryState>,
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
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::ListAuditEntries { entries, total_len },
                    ..
                } => {
                    let mut d = dashboard.write();
                    d.audit_entries = entries;
                    d.audit_total = total_len;
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::VerifyAuditChain { ok, .. },
                    ..
                } => {
                    dashboard.write().chain_ok = Some(ok);
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::GetProfile { profile },
                    ..
                } => {
                    dashboard.write().assistant_name = Some(profile.assistant_name);
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::ListMemoryTopics { topics },
                    ..
                } => {
                    memory.write().topics = topics;
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::GetMemoryTopicEntries { entries },
                    ..
                } => {
                    let mut m = memory.write();
                    m.entries = entries;
                    m.fell_back = false;
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::SearchMemory { matches, fell_back_to_keyword },
                    ..
                } => {
                    let mut m = memory.write();
                    m.entries = matches;
                    m.fell_back = fell_back_to_keyword;
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
