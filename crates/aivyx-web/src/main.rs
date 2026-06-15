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
    AuditEntrySummary, DaemonEnvelope, DocEntry, DocFile, EffectivePersonaSummary, FrontendMessage,
    MemoryEntrySummary, PersonaDeltaSummary, PersonaProposalResolution, PersonaProposalSummary,
    PersonaSeedWire, ProfileSummary, QueryPayload, QueryResponsePayload, SeedSkillWire,
    SettingsSnapshot, StreamEventPayload, VoiceSettingsSnapshot,
};
use aivyx_ipc::{
    ProposedPersonaDelta, TeamConfig, TeamMember, TeamMissionPhase, TeamMissionView, TrustTier,
};

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
const ICON_DOCUMENTS: Asset = asset!("/assets/icons/documents.svg");
const ICON_VOICE: Asset = asset!("/assets/icons/voice.svg");
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
    Settings,
    Agents,
    Teams,
    Documents,
    Voice,
}

/// Memory browser state — read-only snapshots fanned in by `ws_task`.
#[derive(Clone, Default, PartialEq)]
struct MemoryState {
    topics: Vec<String>,
    entries: Vec<MemoryEntrySummary>,
    /// True when a semantic search was transparently served by the keyword path.
    fell_back: bool,
}

/// Settings screen state — the on-disk config snapshot + the last write outcome.
/// Chapter U: the first **write** surface, so it also carries a notice banner
/// and the "restart to apply" flag (config is load-time).
#[derive(Clone, Default, PartialEq)]
struct SettingsState {
    snapshot: Option<SettingsSnapshot>,
    /// Last write outcome: `(ok, message)`. `None` until the first write.
    notice: Option<(bool, String)>,
    /// True after a successful write — a write updates aivyx.toml but the
    /// running daemon won't pick it up until it restarts.
    restart_required: bool,
}

/// Agents screen state — Chapter V. The operator-declared Profile half (V.3)
/// plus the self-learned Persona-governance half (V.4): the folded effective
/// persona, the pending proposals the operator gates, and the approved delta
/// chain the operator can revert.
#[derive(Clone, Default, PartialEq)]
struct AgentsState {
    profile: Option<ProfileSummary>,
    /// The folded effective persona (read-only viewer). `None` until first load.
    persona: Option<EffectivePersonaSummary>,
    /// Pending persona proposals awaiting the operator's gate.
    proposals: Vec<PersonaProposalSummary>,
    /// The approved persona delta chain (newest first), each revertable.
    deltas: Vec<PersonaDeltaSummary>,
    /// Last write/action outcome: `(ok, message)`. `None` until the first one.
    notice: Option<(bool, String)>,
    /// True after a successful **Profile** write — `aivyx.toml` is updated but
    /// the running daemon won't pick it up until restart (Profile is load-time).
    /// Persona actions are live (the daemon recomputes runtime state), so they
    /// never set this.
    restart_required: bool,
    /// Bumped on each persona resolve/revert ack so the panel re-queries the
    /// proposals + deltas + effective persona (the live-refresh signal).
    refresh_tick: u64,
    /// X.3 — the latest LLM-drafted seed (the onboarding card fills its form
    /// from this); `None` until a draft arrives or after a failed draft.
    seed_draft: Option<PersonaSeedWire>,
    /// X.3 — bumped on every `DraftPersonaSeed` response (success or failure) so
    /// the onboarding card can clear its "Drafting…" state and re-seed its form.
    seed_draft_resp: u64,
}

/// Voice config screen state — Chapter Voice. The on-disk `[voice]` snapshot
/// (+ readiness) plus the last write outcome + the load-time restart flag.
#[derive(Clone, Default, PartialEq)]
struct VoiceState {
    snapshot: Option<VoiceSettingsSnapshot>,
    notice: Option<(bool, String)>,
    restart_required: bool,
}

/// Documents browser state — Chapter Z. The active root + path + the current
/// directory listing, the open file (if any), and the last error notice. All
/// read-only; the data is fanned in by `ws_task`.
#[derive(Clone, Default, PartialEq)]
struct DocumentsState {
    /// `"workspace"` | `"fs"` — empty until the first load (then `"workspace"`).
    root: String,
    /// Current directory, relative to `root`.
    path: String,
    /// The current directory's listing (dirs first).
    entries: Vec<DocEntry>,
    /// The open file in the viewer, or `None` when showing the listing.
    file: Option<DocFile>,
    /// Last error (e.g. a denied path / read failure).
    notice: Option<String>,
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
    let settings = use_signal(SettingsState::default);
    let agents = use_signal(AgentsState::default);
    let roster = use_signal(|| None::<TeamConfig>);
    let documents = use_signal(DocumentsState::default);
    let voice = use_signal(VoiceState::default);
    // Chat state, shared with the read task + the Chat view (via context).
    let session = use_signal(|| None::<String>);
    let transcript = use_signal(Vec::<ChatLine>::new);
    let streaming = use_signal(String::new);
    let gate = use_signal(|| None::<GateInfo>);

    let ws: Sender = use_coroutine(move |rx| {
        ws_task(
            rx, missions, dashboard, memory, settings, agents, roster, documents, voice,
            connected, session, transcript, streaming, gate,
        )
    });
    use_context_provider(|| ws);
    use_context_provider(|| memory);
    use_context_provider(|| settings);
    use_context_provider(|| agents);
    use_context_provider(|| roster);
    use_context_provider(|| documents);
    use_context_provider(|| voice);
    use_context_provider(|| missions);
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
            // Dashboard shows the *running* agent's name — the active snapshot.
            payload: QueryPayload::GetProfile { from_disk: false },
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
        View::Settings => "Settings",
        View::Agents => "Agents",
        View::Teams => "Teams",
        View::Documents => "Documents",
        View::Voice => "Voice",
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
                        View::Settings => rsx! { SettingsPanel {} },
                        View::Agents => rsx! { AgentsPanel {} },
                        View::Teams => rsx! { TeamsPanel {} },
                        View::Documents => rsx! { DocumentsPanel {} },
                        View::Voice => rsx! { VoicePanel {} },
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
            NavItem { icon: ICON_SETTINGS, label: "Settings", active: view() == View::Settings,
                onclick: move |_| view.set(View::Settings) }
            NavItem { icon: ICON_AGENTS, label: "Agents", active: view() == View::Agents,
                onclick: move |_| view.set(View::Agents) }
            NavItem { icon: ICON_TEAMS, label: "Teams", active: view() == View::Teams,
                onclick: move |_| view.set(View::Teams) }
            NavItem { icon: ICON_DOCUMENTS, label: "Documents", active: view() == View::Documents,
                onclick: move |_| view.set(View::Documents) }
            NavItem { icon: ICON_VOICE, label: "Voice", active: view() == View::Voice,
                onclick: move |_| view.set(View::Voice) }
            div { class: "nav-section label-tech", "Roadmap" }
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
// Settings — the first config write surface (Chapter U)
// ---------------------------------------------------------------------------

#[component]
fn SettingsPanel() -> Element {
    let ws = use_context::<Sender>();
    let settings = use_context::<Signal<SettingsState>>();

    // Editable form state, seeded from the on-disk snapshot.
    let mut level = use_signal(String::new);
    let mut root = use_signal(String::new);
    let mut per_run = use_signal(String::new);
    let mut per_day = use_signal(String::new);
    let mut on_exceeded = use_signal(|| "deny".to_string());
    let mut alert_at = use_signal(String::new);
    let mut confirm_open = use_signal(|| false);
    // The snapshot the form was last seeded from — so a write *error* (snapshot
    // unchanged) doesn't wipe the operator's in-progress edits.
    let mut last_seed = use_signal(|| None::<SettingsSnapshot>);

    // Load the current settings when the view opens.
    use_future(move || async move {
        ws.send(get_settings_query());
    });

    // Seed the form whenever the snapshot content changes (first load + after a
    // successful write), but not on a notice-only change.
    use_effect(move || {
        let snap = settings().snapshot.clone();
        if snap != last_seed() {
            if let Some(s) = snap.as_ref() {
                level.set(s.access_level.clone());
                root.set(
                    if s.access_level == "workspace" || s.access_level == "custom" {
                        s.fs_root.clone()
                    } else {
                        String::new()
                    },
                );
                per_run.set(s.budget.per_run_usd.map(|v| v.to_string()).unwrap_or_default());
                per_day.set(s.budget.per_day_usd.map(|v| v.to_string()).unwrap_or_default());
                on_exceeded.set(s.budget.on_exceeded.clone());
                alert_at.set(s.budget.alert_at.map(|v| v.to_string()).unwrap_or_default());
            }
            last_seed.set(snap);
        }
    });

    let st = settings();
    let snap = match st.snapshot.clone() {
        Some(s) => s,
        None => {
            return rsx! {
                div { class: "settings",
                    div { class: "glass-card empty",
                        p { class: "label-tech", "Loading settings…" }
                    }
                }
            }
        }
    };

    let needs_root = level() == "workspace" || level() == "custom";
    let expanded = level() != "sandbox";

    rsx! {
        div { class: "settings",

            if st.restart_required {
                div { class: "glass-card restart-banner",
                    strong { "Saved — restart the daemon to apply." }
                    p { class: "label-tech",
                        "Settings are read once at startup. Run  "
                        code { "aivyx daemon stop && aivyx daemon run" }
                    }
                }
            }

            if let Some((ok, msg)) = st.notice.clone() {
                div { class: if ok { "notice ok" } else { "notice err" }, "{msg}" }
            }

            // ── Access level (editable, confirm-first on expansion) ──
            div { class: "glass-card settings-section",
                div { class: "panel-head",
                    h3 { "Access level" }
                    span { class: "chip", "{snap.access_level}" }
                }
                p { class: "label-tech",
                    "How far the agent can reach on disk. Expanding beyond the sandbox is confirmed first."
                }
                div { class: "field-row",
                    label { class: "label-tech", "Level" }
                    select {
                        class: "input",
                        value: "{level}",
                        onchange: move |e| level.set(e.value()),
                        option { value: "sandbox", "sandbox — ~/aivyx-sandbox" }
                        option { value: "workspace", "workspace — a chosen directory" }
                        option { value: "home", "home — your home directory" }
                        option { value: "full", "full — the whole machine" }
                        option { value: "custom", "custom — a chosen directory" }
                    }
                }
                if needs_root {
                    div { class: "field-row",
                        label { class: "label-tech", "Root" }
                        input {
                            class: "input",
                            placeholder: "/path/to/directory",
                            value: "{root}",
                            oninput: move |e| root.set(e.value()),
                        }
                    }
                }
                p { class: "label-tech sub", "Current reach: {snap.fs_root}" }
                div { class: "actions",
                    button {
                        class: "btn btn-primary",
                        onclick: move |_| {
                            if expanded {
                                confirm_open.set(true);
                            } else {
                                ws.send(set_access_query(level(), None, false));
                            }
                        },
                        "Apply access level"
                    }
                }
            }

            // ── Budget (editable) ──
            div { class: "glass-card settings-section",
                div { class: "panel-head", h3 { "Budget" } }
                p { class: "label-tech", "Dollar caps on spend. Leave a cap blank for unlimited." }
                div { class: "field-row",
                    label { class: "label-tech", "Per run ($)" }
                    input {
                        class: "input", r#type: "number", placeholder: "unlimited",
                        value: "{per_run}", oninput: move |e| per_run.set(e.value()),
                    }
                }
                div { class: "field-row",
                    label { class: "label-tech", "Per day ($)" }
                    input {
                        class: "input", r#type: "number", placeholder: "unlimited",
                        value: "{per_day}", oninput: move |e| per_day.set(e.value()),
                    }
                }
                div { class: "field-row",
                    label { class: "label-tech", "On exceeded" }
                    select {
                        class: "input", value: "{on_exceeded}",
                        onchange: move |e| on_exceeded.set(e.value()),
                        option { value: "deny", "deny — block the call" }
                        option { value: "alert", "alert — warn, proceed" }
                    }
                }
                div { class: "field-row",
                    label { class: "label-tech", "Alert at (0–1)" }
                    input {
                        class: "input", r#type: "number", placeholder: "0.8",
                        value: "{alert_at}", oninput: move |e| alert_at.set(e.value()),
                    }
                }
                div { class: "actions",
                    button {
                        class: "btn btn-primary",
                        onclick: move |_| ws.send(set_budget_query(
                            parse_opt_f64(&per_run()),
                            parse_opt_f64(&per_day()),
                            Some(on_exceeded()),
                            parse_opt_f64(&alert_at()),
                        )),
                        "Save budget"
                    }
                }
            }

            // ── Provider / model (read-only — change via `aivyx init`) ──
            div { class: "glass-card settings-section",
                div { class: "panel-head", h3 { "Model" } span { class: "chip", "read-only" } }
                div { class: "kv-grid",
                    div { span { class: "label-tech", "Provider" } div { "{snap.provider}" } }
                    div { span { class: "label-tech", "Model" } div { "{snap.model}" } }
                    div {
                        span { class: "label-tech", "Context" }
                        div { {snap.num_ctx.map(|n| n.to_string()).unwrap_or_else(|| "default".to_string())} }
                    }
                    div {
                        span { class: "label-tech", "Embeddings" }
                        div { {if snap.embeddings_available { "available" } else { "off" }} }
                    }
                }
                p { class: "label-tech sub", "Change the provider, model, or keys with  " code { "aivyx init" } }
            }
        }

        // Confirm-first modal for expanded access levels (Chapter N posture).
        if confirm_open() {
            div { class: "modal-scrim",
                div { class: "glass-card modal",
                    h3 { "Grant '{level()}' access?" }
                    p { "{confirm_blurb(&level())}" }
                    div { class: "actions",
                        button { class: "btn btn-glass", onclick: move |_| confirm_open.set(false), "Cancel" }
                        button {
                            class: "btn btn-primary",
                            onclick: move |_| {
                                let r = if needs_root { Some(root()) } else { None };
                                ws.send(set_access_query(level(), r, true));
                                confirm_open.set(false);
                            },
                            "Grant access"
                        }
                    }
                }
            }
        }
    }
}

fn get_settings_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-settings-get".to_string(),
        payload: QueryPayload::GetSettings,
    }
}

fn set_access_query(level: String, root: Option<String>, confirm: bool) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-settings-access".to_string(),
        payload: QueryPayload::SetAccessLevel { level, root, confirm },
    }
}

fn set_budget_query(
    per_run_usd: Option<f64>,
    per_day_usd: Option<f64>,
    on_exceeded: Option<String>,
    alert_at: Option<f64>,
) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-settings-budget".to_string(),
        payload: QueryPayload::SetBudget { per_run_usd, per_day_usd, on_exceeded, alert_at },
    }
}

/// Parse a numeric form field: empty ⇒ `None` (clear / unlimited), unparseable
/// ⇒ `None` (the daemon validates and reports anything truly wrong).
fn parse_opt_f64(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        t.parse().ok()
    }
}

/// The confirm-modal blurb for an expanded access level.
fn confirm_blurb(level: &str) -> String {
    match level {
        "full" => "This grants access to the ENTIRE filesystem, including system files.".to_string(),
        "home" => "This grants full read / write / shell across your home directory.".to_string(),
        "workspace" | "custom" => {
            "This grants full read / write / shell within the chosen directory.".to_string()
        }
        _ => "This reaches beyond the default sandbox.".to_string(),
    }
}

// ---------------------------------------------------------------------------
// Voice — the host voice channel, configured (Chapter Voice). A config-write
// screen for [voice] + a readiness check + the launch command. The audio loop
// (mic/ASR/TTS) is a host process; this screen never touches audio.
// ---------------------------------------------------------------------------

#[component]
fn VoicePanel() -> Element {
    let ws = use_context::<Sender>();
    let voice = use_context::<Signal<VoiceState>>();

    let mut asr_engine = use_signal(String::new);
    let mut tts_engine = use_signal(String::new);
    let mut asr_model_path = use_signal(String::new);
    let mut asr_language = use_signal(String::new);
    let mut asr_beam_size = use_signal(String::new);
    let mut tts_voice_path = use_signal(String::new);
    let mut tts_espeak_data_path = use_signal(String::new);
    let mut input_device = use_signal(String::new);
    let mut output_device = use_signal(String::new);
    let mut last_seed = use_signal(|| None::<VoiceSettingsSnapshot>);

    use_future(move || async move {
        ws.send(get_voice_query());
    });

    // Re-seed the form from the on-disk snapshot (first load + after a write),
    // guarded so a write error doesn't wipe in-progress edits.
    use_effect(move || {
        let snap = voice().snapshot.clone();
        if snap != last_seed() {
            if let Some(s) = snap.as_ref() {
                asr_engine.set(s.asr_engine.clone().unwrap_or_default());
                tts_engine.set(s.tts_engine.clone().unwrap_or_default());
                asr_model_path.set(s.asr_model_path.clone().unwrap_or_default());
                asr_language.set(s.asr_language.clone().unwrap_or_default());
                asr_beam_size.set(s.asr_beam_size.map(|n| n.to_string()).unwrap_or_default());
                tts_voice_path.set(s.tts_voice_path.clone().unwrap_or_default());
                tts_espeak_data_path.set(s.tts_espeak_data_path.clone().unwrap_or_default());
                input_device.set(s.input_device.clone().unwrap_or_default());
                output_device.set(s.output_device.clone().unwrap_or_default());
            }
            last_seed.set(snap);
        }
    });

    let st = voice();
    let snap = match st.snapshot.clone() {
        Some(s) => s,
        None => {
            return rsx! {
                div { class: "settings voice",
                    div { class: "glass-card empty", p { class: "label-tech", "Loading voice config…" } }
                }
            }
        }
    };

    rsx! {
        div { class: "settings voice",

            if st.restart_required {
                div { class: "glass-card restart-banner",
                    strong { "Saved — restart voice to apply." }
                    p { class: "label-tech",
                        "[voice] is read when the voice channel starts. Stop and re-run  "
                        code { "aivyx --channel voice" }
                    }
                }
            }
            if let Some((ok, msg)) = st.notice.clone() {
                div { class: if ok { "notice ok" } else { "notice err" }, "{msg}" }
            }

            // ── Readiness ──
            div { class: "glass-card settings-section",
                div { class: "panel-head", h3 { "Readiness" } }
                p { class: "label-tech",
                    "Voice runs on the host (microphone + speakers). These files must exist before you launch."
                }
                div { class: "kv-grid",
                    ReadinessRow { label: "Whisper model", status: snap.asr_model_status.clone() }
                    ReadinessRow { label: "Piper voice", status: snap.tts_voice_status.clone() }
                    ReadinessRow { label: "espeak-ng data", status: snap.espeak_status.clone() }
                }
            }

            // ── Models & engines ──
            div { class: "glass-card settings-section",
                div { class: "panel-head", h3 { "Models & engines" } }
                div { class: "field-row",
                    label { class: "label-tech", "ASR engine" }
                    select { class: "input", value: "{asr_engine}", onchange: move |e| asr_engine.set(e.value()),
                        option { value: "", "default (whisper-rs)" }
                        option { value: "whisper-rs", "whisper-rs" }
                        option { value: "whisper-cpp-plus", "whisper-cpp-plus" }
                    }
                }
                div { class: "field-row",
                    label { class: "label-tech", "Whisper model" }
                    input { class: "input", placeholder: "/path/to/ggml-model.bin",
                        value: "{asr_model_path}", oninput: move |e| asr_model_path.set(e.value()) }
                }
                div { class: "field-row",
                    label { class: "label-tech", "TTS engine" }
                    select { class: "input", value: "{tts_engine}", onchange: move |e| tts_engine.set(e.value()),
                        option { value: "", "default (piper)" }
                        option { value: "piper", "piper" }
                    }
                }
                div { class: "field-row",
                    label { class: "label-tech", "Piper voice" }
                    input { class: "input", placeholder: "/path/to/voice.onnx",
                        value: "{tts_voice_path}", oninput: move |e| tts_voice_path.set(e.value()) }
                }
                div { class: "field-row",
                    label { class: "label-tech", "espeak-ng data" }
                    input { class: "input", placeholder: "/usr/share/espeak-ng-data",
                        value: "{tts_espeak_data_path}", oninput: move |e| tts_espeak_data_path.set(e.value()) }
                }
                div { class: "field-row",
                    label { class: "label-tech", "ASR language" }
                    input { class: "input", placeholder: "en (or auto)",
                        value: "{asr_language}", oninput: move |e| asr_language.set(e.value()) }
                }
                div { class: "field-row",
                    label { class: "label-tech", "Beam size" }
                    input { class: "input", r#type: "number", placeholder: "5",
                        value: "{asr_beam_size}", oninput: move |e| asr_beam_size.set(e.value()) }
                }
            }

            // ── Audio devices ──
            div { class: "glass-card settings-section",
                div { class: "panel-head", h3 { "Audio devices" } span { class: "chip muted", "optional" } }
                div { class: "field-row",
                    label { class: "label-tech", "Input" }
                    input { class: "input", placeholder: "system default",
                        value: "{input_device}", oninput: move |e| input_device.set(e.value()) }
                }
                div { class: "field-row",
                    label { class: "label-tech", "Output" }
                    input { class: "input", placeholder: "system default",
                        value: "{output_device}", oninput: move |e| output_device.set(e.value()) }
                }
            }

            // ── Launch ──
            div { class: "glass-card settings-section",
                div { class: "panel-head", h3 { "Launch" } }
                p { class: "label-tech", "Start voice as its own foreground process on this machine:" }
                pre { class: "launch-cmd", "aivyx --channel voice" }
                p { class: "label-tech sub", "The audio loop (mic → Whisper → agent → Piper → speakers) runs on the host, not in the browser." }
            }

            div { class: "actions sticky-save",
                button {
                    class: "btn btn-primary",
                    onclick: move |_| ws.send(set_voice_query(
                        opt_str(&asr_engine()), opt_str(&tts_engine()), opt_str(&asr_model_path()),
                        opt_str(&asr_language()), parse_opt_u32(&asr_beam_size()), opt_str(&tts_voice_path()),
                        opt_str(&tts_espeak_data_path()), opt_str(&input_device()), opt_str(&output_device()),
                    )),
                    "Save voice config"
                }
            }
        }
    }
}

/// One readiness row — a label + a status chip (present = sage, missing = error,
/// unset = muted). A `.kv-grid` cell.
#[component]
fn ReadinessRow(label: String, status: String) -> Element {
    let (cls, txt) = match status.as_str() {
        "present" => ("sage", "present"),
        "missing" => ("error", "missing"),
        _ => ("muted", "not set"),
    };
    rsx! {
        div {
            span { class: "label-tech", "{label}" }
            div { span { class: "chip {cls}", "{txt}" } }
        }
    }
}

fn get_voice_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-voice-get".to_string(),
        payload: QueryPayload::GetVoiceSettings,
    }
}

#[allow(clippy::too_many_arguments)]
fn set_voice_query(
    asr_engine: Option<String>,
    tts_engine: Option<String>,
    asr_model_path: Option<String>,
    asr_language: Option<String>,
    asr_beam_size: Option<u32>,
    tts_voice_path: Option<String>,
    tts_espeak_data_path: Option<String>,
    input_device: Option<String>,
    output_device: Option<String>,
) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-voice-set".to_string(),
        payload: QueryPayload::SetVoice {
            asr_engine,
            tts_engine,
            asr_model_path,
            asr_language,
            asr_beam_size,
            tts_voice_path,
            tts_espeak_data_path,
            input_device,
            output_device,
        },
    }
}

/// Parse a numeric form field to `Option<u32>` (blank / unparseable ⇒ `None`).
fn parse_opt_u32(s: &str) -> Option<u32> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        t.parse().ok()
    }
}

// ---------------------------------------------------------------------------
// Agents — the identity editor (Chapter V)
//
// V.3 ships the **Profile editor** half: the operator-declared `[profile]`
// layer (assistant name, operator context, communication style, and three
// declared lists). A save round-trips through `SetProfile`, the daemon rewrites
// `[profile]` in `aivyx.toml`, and — because Profile is read at startup — the
// screen shows the same "restart to apply" banner the Settings writes do. The
// agent's self-learned *Persona* governance panel is V.4.
// ---------------------------------------------------------------------------

#[component]
fn AgentsPanel() -> Element {
    let ws = use_context::<Sender>();
    let agents = use_context::<Signal<AgentsState>>();

    // Editable form state, seeded from the on-disk Profile snapshot.
    let mut name = use_signal(String::new);
    let mut operator = use_signal(String::new);
    let mut comm_style = use_signal(String::new);
    let use_cases = use_signal(Vec::<String>::new);
    let prefs = use_signal(Vec::<String>::new);
    let constraints = use_signal(Vec::<String>::new);
    // The snapshot the form was last seeded from — so a write *error* (snapshot
    // unchanged) doesn't wipe the operator's in-progress edits.
    let mut last_seed = use_signal(|| None::<ProfileSummary>);

    // Load the current Profile when the view opens. (The Command-Center one-shot
    // may already have populated it; re-asking is cheap and keeps this panel
    // self-contained.)
    use_future(move || async move {
        ws.send(get_profile_query());
    });

    // Persona governance load + live-refresh. The memo isolates the refresh tick
    // so this effect re-runs ONLY on mount (tick 0) and after a resolve/revert
    // ack bumps it — not on every unrelated AgentsState write (which would loop,
    // since the queries below feed AgentsState).
    let tick = use_memo(move || agents().refresh_tick);
    use_effect(move || {
        let _ = tick();
        ws.send(get_effective_persona_query());
        ws.send(list_proposals_query());
        ws.send(list_deltas_query());
    });

    // Seed the form whenever the snapshot content changes (first load + after a
    // successful write), but not on a notice-only change.
    let mut use_cases_s = use_cases;
    let mut prefs_s = prefs;
    let mut constraints_s = constraints;
    use_effect(move || {
        let snap = agents().profile.clone();
        if snap != last_seed() {
            if let Some(p) = snap.as_ref() {
                // Only seed the name when it is operator-declared; a `default`
                // source means "Aivyx" is the fallback, so leave the field blank
                // (saving blank keeps it at the default rather than re-declaring).
                name.set(if p.assistant_name_source == "toml" {
                    p.assistant_name.clone()
                } else {
                    String::new()
                });
                operator.set(p.operator_profile.clone().unwrap_or_default());
                comm_style.set(p.communication_style.clone().unwrap_or_default());
                use_cases_s.set(p.primary_use_cases.clone());
                prefs_s.set(p.behavioral_preferences.clone());
                constraints_s.set(p.behavioral_constraints.clone());
            }
            last_seed.set(snap);
        }
    });

    let st = agents();
    let profile = match st.profile.clone() {
        Some(p) => p,
        None => {
            return rsx! {
                div { class: "settings agents",
                    div { class: "glass-card empty",
                        p { class: "label-tech", "Loading profile…" }
                    }
                }
            }
        }
    };

    // X.3 — a *fresh* agent (the effective persona is loaded and empty, the
    // delta chain is empty, and nothing is pending) gets the onboarding seed
    // card instead of the (empty) governance view. Once seeded, the refresh
    // re-query flips `is_non_empty` and the governance view takes over.
    let is_fresh = st
        .persona
        .as_ref()
        .map(|p| !p.is_non_empty)
        .unwrap_or(false)
        && st.deltas.is_empty()
        && st.proposals.is_empty();

    rsx! {
        div { class: "settings agents",

            if st.restart_required {
                div { class: "glass-card restart-banner",
                    strong { "Saved — restart the daemon to apply." }
                    p { class: "label-tech",
                        "The Profile shapes every turn's system prompt at startup. Run  "
                        code { "aivyx daemon stop && aivyx daemon run" }
                    }
                }
            }

            if let Some((ok, msg)) = st.notice.clone() {
                div { class: if ok { "notice ok" } else { "notice err" }, "{msg}" }
            }

            // ── Declared identity (Profile scalars) ──
            div { class: "glass-card settings-section",
                div { class: "panel-head",
                    h3 { "Declared identity" }
                    span { class: if profile.injection_enabled { "chip" } else { "chip muted" },
                        {if profile.injection_enabled { "shaping prompts" } else { "passthrough" }}
                    }
                }
                p { class: "label-tech",
                    "What you declare about your assistant and yourself. The agent reads this at startup; leave a field blank to clear it."
                }
                div { class: "field-row",
                    label { class: "label-tech", "Assistant name" }
                    input {
                        class: "input", placeholder: "Aivyx (default)",
                        value: "{name}", oninput: move |e| name.set(e.value()),
                    }
                }
                div { class: "field-row",
                    label { class: "label-tech", "About you" }
                    textarea {
                        class: "input", rows: "2",
                        placeholder: "e.g. Indie game developer; prefers concise, technical answers",
                        value: "{operator}", oninput: move |e| operator.set(e.value()),
                    }
                }
                div { class: "field-row",
                    label { class: "label-tech", "Communication style" }
                    textarea {
                        class: "input", rows: "2",
                        placeholder: "e.g. Terse, no preamble, code-first",
                        value: "{comm_style}", oninput: move |e| comm_style.set(e.value()),
                    }
                }
            }

            // ── Declared lists ──
            ListEditor {
                title: "Primary use cases",
                hint: "What you mostly use the agent for.",
                items: use_cases,
            }
            ListEditor {
                title: "Behavioral preferences",
                hint: "How you'd like the agent to behave (soft guidance).",
                items: prefs,
            }
            ListEditor {
                title: "Behavioral constraints",
                hint: "Lines the agent should not cross.",
                items: constraints,
            }

            div { class: "actions sticky-save",
                button {
                    class: "btn btn-primary",
                    onclick: move |_| ws.send(set_profile_query(
                        opt_str(&name()),
                        opt_str(&operator()),
                        opt_str(&comm_style()),
                        opt_list(&use_cases()),
                        opt_list(&prefs()),
                        opt_list(&constraints()),
                    )),
                    "Save profile"
                }
            }

            // ── Self-learned persona (governance — never hand-edited) ──
            div { class: "section-divider label-tech", "Self-learned persona" }
            p { class: "label-tech persona-blurb",
                "The agent proposes these from reflection; you approve or revert. \
                 Changes apply on the next turn — no restart needed."
            }

            if is_fresh {
                SeedOnboardingCard {}
            } else {

            // Pending proposals — the operator's gate.
            div { class: "glass-card settings-section",
                div { class: "panel-head",
                    h3 { "Pending proposals" }
                    span { class: if st.proposals.is_empty() { "chip muted" } else { "chip amber" },
                        "{st.proposals.len()}"
                    }
                }
                if st.proposals.is_empty() {
                    p { class: "label-tech sub", "Nothing awaiting review." }
                } else {
                    div { class: "proposal-list",
                        for p in st.proposals.clone() {
                            ProposalCard { key: "{p.id}", p: p.clone() }
                        }
                    }
                }
            }

            // Effective persona — the folded, read-only view.
            div { class: "glass-card settings-section",
                div { class: "panel-head", h3 { "Effective persona" } span { class: "chip", "read-only" } }
                match st.persona.clone() {
                    Some(p) if p.is_non_empty => rsx! {
                        div { class: "persona-facets",
                            PersonaFacet { label: "Learned context", items: p.learned_context }
                            PersonaFacet { label: "Communication adaptations", items: p.communication_adaptations }
                            PersonaFacet { label: "Character traits", items: p.character_traits }
                            PersonaFacet { label: "Relationship milestones", items: p.relationship_milestones }
                            PersonaFacet { label: "Behavioral preferences", items: p.behavioral_preferences }
                            PersonaFacet { label: "Behavioral constraints", items: p.behavioral_constraints }
                        }
                    },
                    _ => rsx! {
                        p { class: "label-tech sub", "The agent hasn't learned anything yet." }
                    },
                }
            }

            // Change history — the approved delta chain, each revertable.
            div { class: "glass-card settings-section",
                div { class: "panel-head", h3 { "Change history" } span { class: "chip", "{st.deltas.len()}" } }
                if st.deltas.is_empty() {
                    p { class: "label-tech sub", "No approved changes yet." }
                } else {
                    div { class: "delta-list",
                        for d in st.deltas.clone() {
                            DeltaRow { key: "{d.delta_id}", d: d.clone() }
                        }
                    }
                }
            }

            } // end else (governance vs. onboarding seed card)
        }
    }
}

/// X.3 — the "Seed your assistant" onboarding card, shown for a fresh agent.
/// Describe the assistant → optionally let the model draft a starting set →
/// edit → plant. The seed goes onto the signed chain via `SeedPersona` (the
/// same primitive the boot-seed uses); the operator is always the author.
#[component]
fn SeedOnboardingCard() -> Element {
    let ws = use_context::<Sender>();
    let agents = use_context::<Signal<AgentsState>>();

    let mut description = use_signal(String::new);
    let traits = use_signal(Vec::<String>::new);
    let adaptations = use_signal(Vec::<String>::new);
    let mut context = use_signal(String::new);
    let mut skill_name = use_signal(String::new);
    let mut skill_trigger = use_signal(String::new);
    let mut skill_procedure = use_signal(String::new);
    let mut drafting = use_signal(|| false);
    let mut last_resp = use_signal(|| 0u64);

    // When a draft response arrives (success or failure), clear the spinner and
    // — on success — fill the form from the drafted seed. The operator edits
    // from there.
    let mut traits_s = traits;
    let mut adaptations_s = adaptations;
    use_effect(move || {
        let a = agents();
        if a.seed_draft_resp != last_resp() {
            last_resp.set(a.seed_draft_resp);
            drafting.set(false);
            if let Some(d) = a.seed_draft.as_ref() {
                traits_s.set(d.character_traits.clone());
                adaptations_s.set(d.communication_adaptations.clone());
                context.set(d.learned_context.first().cloned().unwrap_or_default());
                if let Some(s) = d.skills.first() {
                    skill_name.set(s.name.clone());
                    skill_trigger.set(s.trigger.clone());
                    skill_procedure.set(s.procedure.clone());
                }
            }
        }
    });

    let st = agents();

    rsx! {
        div { class: "glass-card settings-section seed-card",
            div { class: "panel-head",
                h3 { "Seed your assistant" }
                span { class: "chip sage", "fresh" }
            }
            p { class: "label-tech",
                "This agent hasn't learned a personality yet. Give it a head start — \
                 it keeps growing from use. Describe it, optionally let the model draft \
                 a set, edit, then plant."
            }

            if let Some((ok, msg)) = st.notice.clone() {
                div { class: if ok { "notice ok" } else { "notice err" }, "{msg}" }
            }

            div { class: "field-row",
                label { class: "label-tech", "Describe it" }
                textarea {
                    class: "input", rows: "2",
                    placeholder: "e.g. a witty, terse pair-programmer who cites sources",
                    value: "{description}", oninput: move |e| description.set(e.value()),
                }
            }
            div { class: "actions",
                button {
                    class: "btn btn-glass",
                    disabled: drafting(),
                    onclick: move |_| { drafting.set(true); ws.send(draft_seed_query(description())); },
                    {if drafting() { "Drafting…" } else { "Draft with AI" }}
                }
            }

            ListEditor { title: "Character traits", hint: "Voice properties to start with.", items: traits }
            ListEditor {
                title: "Communication adaptations",
                hint: "Refinements to how it talks (optional).",
                items: adaptations,
            }
            div { class: "field-row",
                label { class: "label-tech", "Day-one context" }
                textarea {
                    class: "input", rows: "2",
                    placeholder: "Anything it should know about you / your work (optional)",
                    value: "{context}", oninput: move |e| context.set(e.value()),
                }
            }

            div { class: "panel-head", h4 { "Starter skill (optional)" } }
            div { class: "field-row",
                label { class: "label-tech", "Name" }
                input { class: "input", placeholder: "rust-review",
                    value: "{skill_name}", oninput: move |e| skill_name.set(e.value()) }
            }
            div { class: "field-row",
                label { class: "label-tech", "When" }
                input { class: "input", placeholder: "when reviewing Rust",
                    value: "{skill_trigger}", oninput: move |e| skill_trigger.set(e.value()) }
            }
            div { class: "field-row",
                label { class: "label-tech", "Does what" }
                input { class: "input", placeholder: "check unwraps; cite file:line",
                    value: "{skill_procedure}", oninput: move |e| skill_procedure.set(e.value()) }
            }

            div { class: "actions sticky-save",
                button {
                    class: "btn btn-primary",
                    onclick: move |_| ws.send(seed_persona_query(build_seed_wire(
                        &traits(), &adaptations(), &context(),
                        &skill_name(), &skill_trigger(), &skill_procedure(),
                    ))),
                    "Plant seed"
                }
            }
        }
    }
}

/// One folded persona facet — a labeled list, rendered only when non-empty.
#[component]
fn PersonaFacet(label: String, items: Vec<String>) -> Element {
    if items.is_empty() {
        return rsx! {};
    }
    rsx! {
        div { class: "facet",
            span { class: "label-tech", "{label}" }
            ul { class: "facet-list",
                for item in items {
                    li { "{item}" }
                }
            }
        }
    }
}

/// The three review modes a proposal card can be in.
#[derive(Clone, Copy, PartialEq)]
enum ProposalMode {
    View,
    Editing,
    Rejecting,
}

/// One pending persona proposal with the operator's gate actions: approve,
/// approve-with-edit (reword the proposed value), or reject with a reason.
/// Chapter V — the self-learning human gate, in the Studio.
#[component]
fn ProposalCard(p: PersonaProposalSummary) -> Element {
    let ws = use_context::<Sender>();
    let mut mode = use_signal(|| ProposalMode::View);
    let mut draft = use_signal(String::new);

    let op_desc = render_op(&p.category, &p.proposed_op);
    let editable = op_value(&p.proposed_op);
    let pid = p.id.clone();
    let category = p.category.clone();
    let op = p.proposed_op.clone();

    rsx! {
        div { class: "glass-card proposal-card",
            div { class: "panel-head",
                h4 { "{p.category}" }
                span { class: "chip amber", "pending" }
            }
            p { class: "op-desc", "{op_desc}" }
            if let Some(reason) = p.proposed_reason.clone() {
                p { class: "label-tech reason", "“{reason}”" }
            }

            match mode() {
                ProposalMode::View => rsx! {
                    div { class: "actions",
                        {
                            let pid_a = pid.clone();
                            rsx! {
                                button {
                                    class: "btn btn-primary",
                                    onclick: move |_| ws.send(resolve_proposal_query(
                                        &pid_a, PersonaProposalResolution::Approve,
                                    )),
                                    "Approve"
                                }
                            }
                        }
                        if let Some(v) = editable.clone() {
                            button {
                                class: "btn btn-glass",
                                onclick: move |_| { draft.set(v.clone()); mode.set(ProposalMode::Editing); },
                                "Edit & approve"
                            }
                        }
                        button {
                            class: "btn btn-glass danger",
                            onclick: move |_| { draft.set(String::new()); mode.set(ProposalMode::Rejecting); },
                            "Reject"
                        }
                    }
                },
                ProposalMode::Editing => rsx! {
                    div { class: "field-row",
                        label { class: "label-tech", "Edited value" }
                        input { class: "input", value: "{draft}", oninput: move |e| draft.set(e.value()) }
                    }
                    div { class: "actions",
                        button { class: "btn btn-glass", onclick: move |_| mode.set(ProposalMode::View), "Cancel" }
                        {
                            let (pid_e, cat_e, op_e) = (pid.clone(), category.clone(), op.clone());
                            rsx! {
                                button {
                                    class: "btn btn-primary",
                                    onclick: move |_| {
                                        if let Some(res) = approve_with_edited_value(&cat_e, &op_e, &draft()) {
                                            ws.send(resolve_proposal_query(&pid_e, res));
                                            mode.set(ProposalMode::View);
                                        }
                                    },
                                    "Approve edit"
                                }
                            }
                        }
                    }
                },
                ProposalMode::Rejecting => rsx! {
                    div { class: "field-row",
                        label { class: "label-tech", "Reason (optional)" }
                        input {
                            class: "input", placeholder: "why you're rejecting…",
                            value: "{draft}", oninput: move |e| draft.set(e.value()),
                        }
                    }
                    div { class: "actions",
                        button { class: "btn btn-glass", onclick: move |_| mode.set(ProposalMode::View), "Cancel" }
                        {
                            let pid_r = pid.clone();
                            rsx! {
                                button {
                                    class: "btn btn-primary",
                                    onclick: move |_| {
                                        ws.send(resolve_proposal_query(
                                            &pid_r,
                                            PersonaProposalResolution::Reject { reason: opt_str(&draft()) },
                                        ));
                                        mode.set(ProposalMode::View);
                                    },
                                    "Confirm reject"
                                }
                            }
                        }
                    }
                },
            }
        }
    }
}

/// One approved persona delta with a two-click inline revert (reverting appends
/// an inverse delta — the chain stays append-only). Chapter V.
#[component]
fn DeltaRow(d: PersonaDeltaSummary) -> Element {
    let ws = use_context::<Sender>();
    let mut confirming = use_signal(|| false);
    let desc = render_op(&d.category, &d.op);
    let did = d.delta_id.clone();

    rsx! {
        div { class: "delta-row",
            div { class: "delta-main",
                span { class: "chip", "#{d.seq}" }
                // Mark deltas planted by the onboarding seed (W.2 sentinel) so
                // they're visibly distinct from the agent's learned deltas.
                if d.proposal_id == "genesis-seed" {
                    span { class: "chip sage", title: "Planted at first launch from [persona_seed]", "seed" }
                }
                span { class: "delta-cat label-tech", "{d.category}" }
                span { class: "op-desc", "{desc}" }
            }
            if confirming() {
                div { class: "actions",
                    button { class: "btn btn-glass", onclick: move |_| confirming.set(false), "Cancel" }
                    button {
                        class: "btn btn-primary",
                        onclick: move |_| { ws.send(revert_delta_query(&did)); confirming.set(false); },
                        "Confirm revert"
                    }
                }
            } else {
                button { class: "btn btn-glass", onclick: move |_| confirming.set(true), "Revert" }
            }
        }
    }
}

/// A small add/remove list editor over a shared `Signal<Vec<String>>`. Owns its
/// own draft-entry input; mutations flow straight back to the parent's signal
/// so the Save handler reads the live list. Chapter V.
#[component]
fn ListEditor(title: String, hint: String, items: Signal<Vec<String>>) -> Element {
    let mut items = items;
    let mut draft = use_signal(String::new);
    let add = move |_: MouseEvent| {
        let v = draft().trim().to_string();
        if !v.is_empty() {
            items.write().push(v);
            draft.set(String::new());
        }
    };
    rsx! {
        div { class: "glass-card settings-section",
            div { class: "panel-head",
                h3 { "{title}" }
                span { class: "chip", "{items().len()}" }
            }
            p { class: "label-tech", "{hint}" }
            if items().is_empty() {
                p { class: "label-tech sub", "None declared." }
            } else {
                div { class: "list-editor",
                    for (i, entry) in items().into_iter().enumerate() {
                        div { class: "chip-removable", key: "{i}",
                            span { "{entry}" }
                            button {
                                class: "chip-x",
                                title: "Remove",
                                onclick: move |_| { items.write().remove(i); },
                                "×"
                            }
                        }
                    }
                }
            }
            div { class: "add-row",
                input {
                    class: "input",
                    placeholder: "Add an entry…",
                    value: "{draft}",
                    oninput: move |e| draft.set(e.value()),
                }
                button { class: "btn btn-glass", onclick: add, "Add" }
            }
        }
    }
}

fn get_profile_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-agents-get".to_string(),
        // The editor seeds from the **on-disk** profile (what it writes), so a
        // save-before-restart followed by a reload shows the pending values —
        // not the stale running snapshot — and never clobbers a pending edit.
        payload: QueryPayload::GetProfile { from_disk: true },
    }
}

#[allow(clippy::too_many_arguments)]
fn set_profile_query(
    assistant_name: Option<String>,
    operator_profile: Option<String>,
    communication_style: Option<String>,
    primary_use_cases: Option<Vec<String>>,
    behavioral_preferences: Option<Vec<String>>,
    behavioral_constraints: Option<Vec<String>>,
) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-agents-profile".to_string(),
        payload: QueryPayload::SetProfile {
            assistant_name,
            operator_profile,
            communication_style,
            primary_use_cases,
            behavioral_preferences,
            behavioral_constraints,
        },
    }
}

/// A scalar form field → `Some(trimmed)` or `None` when blank (clear the key).
fn opt_str(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// A list field → `Some(cleaned)` of trimmed non-empty entries, or `None` when
/// empty (clear the key — "no declared entries", distinct from `[]`).
fn opt_list(v: &[String]) -> Option<Vec<String>> {
    let cleaned: Vec<String> = v
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

// ── Persona governance queries (Chapter V.4) — all over existing IPC. ──

fn get_effective_persona_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-agents-persona".to_string(),
        payload: QueryPayload::GetEffectivePersona,
    }
}

fn list_proposals_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-agents-proposals".to_string(),
        payload: QueryPayload::ListPersonaProposals { status_filter: "pending".to_string(), limit: 50 },
    }
}

fn list_deltas_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-agents-deltas".to_string(),
        payload: QueryPayload::ListPersonaDeltas { from_seq: 0, limit: 50 },
    }
}

fn resolve_proposal_query(
    proposal_id: &str,
    resolution: PersonaProposalResolution,
) -> FrontendMessage {
    FrontendMessage::ResolvePersonaProposal {
        id: format!("mc-agents-resolve-{proposal_id}"),
        proposal_id: proposal_id.to_string(),
        resolution,
    }
}

fn revert_delta_query(target_delta_id: &str) -> FrontendMessage {
    FrontendMessage::RevertPersonaDelta {
        id: format!("mc-agents-revert-{target_delta_id}"),
        target_delta_id: target_delta_id.to_string(),
    }
}

// ── X.3 — persona seed onboarding (web authoring + LLM draft). ──

fn draft_seed_query(description: String) -> FrontendMessage {
    FrontendMessage::DraftPersonaSeed {
        id: "mc-agents-draft".to_string(),
        description,
    }
}

fn seed_persona_query(seed: PersonaSeedWire) -> FrontendMessage {
    FrontendMessage::SeedPersona {
        id: "mc-agents-seed".to_string(),
        seed,
    }
}

/// Build a `PersonaSeedWire` from the onboarding form. Lists are trimmed +
/// de-blanked; the day-one context becomes a single `learned_context` entry; a
/// skill is included only when it has a name. (The daemon adds the genesis
/// milestone and refuses an empty seed.)
fn build_seed_wire(
    traits: &[String],
    adaptations: &[String],
    context: &str,
    skill_name: &str,
    skill_trigger: &str,
    skill_procedure: &str,
) -> PersonaSeedWire {
    let clean = |v: &[String]| -> Vec<String> {
        v.iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };
    let learned_context = match context.trim() {
        "" => Vec::new(),
        c => vec![c.to_string()],
    };
    let skills = if skill_name.trim().is_empty() {
        Vec::new()
    } else {
        vec![SeedSkillWire {
            name: skill_name.trim().to_string(),
            trigger: skill_trigger.trim().to_string(),
            procedure: skill_procedure.trim().to_string(),
        }]
    };
    PersonaSeedWire {
        learned_context,
        communication_adaptations: clean(adaptations),
        character_traits: clean(traits),
        relationship_milestones: Vec::new(),
        skills,
    }
}

/// Render a persona delta `op` JSON (`{kind, value}`) into a human sentence for
/// the proposal/delta cards. Mirrors `PersonaDeltaOp`'s serde repr.
fn render_op(category: &str, op: &serde_json::Value) -> String {
    let kind = op.get("kind").and_then(|k| k.as_str()).unwrap_or("?");
    let value = op
        .get("value")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    match kind {
        "SetScalar" => format!("set {category} → “{value}”"),
        "AppendList" => format!("add to {category}: “{value}”"),
        "RemoveList" => format!("remove from {category}: “{value}”"),
        "Revert" => format!("revert a prior {category} change"),
        other => format!("{category}: {other}"),
    }
}

/// The single editable string value of an op, for `SetScalar`/`AppendList`/
/// `RemoveList` — the kinds the operator can reword on approve. `None` for ops
/// with no editable string (so the "Edit & approve" affordance is hidden).
fn op_value(op: &serde_json::Value) -> Option<String> {
    match op.get("kind").and_then(|k| k.as_str()) {
        Some("SetScalar") | Some("AppendList") | Some("RemoveList") => op
            .get("value")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        _ => None,
    }
}

/// Build an `ApproveWithEdit` resolution from a proposal's category + op with
/// the operator's reworded value. Reconstructs a typed `ProposedPersonaDelta`
/// by deserialization (the summary's category label equals the serde repr; the
/// op JSON is `PersonaDeltaOp`'s own serde shape), so the daemon re-validates
/// and re-signs the edited op exactly as it would a fresh proposal. Returns
/// `None` if the edited op fails to reconstruct (then the caller does nothing).
fn approve_with_edited_value(
    category: &str,
    op: &serde_json::Value,
    new_value: &str,
) -> Option<PersonaProposalResolution> {
    let mut edited_op = op.clone();
    if let Some(obj) = edited_op.as_object_mut() {
        obj.insert("value".to_string(), serde_json::Value::String(new_value.to_string()));
    }
    let pd_json = serde_json::json!({ "category": category, "op": edited_op });
    serde_json::from_value::<ProposedPersonaDelta>(pd_json)
        .ok()
        .map(|edited_op| PersonaProposalResolution::ApproveWithEdit { edited_op })
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

// ---------------------------------------------------------------------------
// Teams — the Nonagon roster (Chapter Y). Read-only; renders the daemon's
// active TeamConfig (lead + specialists, role / trust / scopes / tools / soul).
// ---------------------------------------------------------------------------

#[component]
fn TeamsPanel() -> Element {
    let ws = use_context::<Sender>();
    let roster = use_context::<Signal<Option<TeamConfig>>>();
    let missions = use_context::<Signal<Vec<TeamMissionView>>>();

    use_future(move || async move {
        ws.send(get_team_roster_query());
    });

    let team = match roster() {
        Some(t) => t,
        None => {
            return rsx! {
                div { class: "teams",
                    div { class: "glass-card empty", p { class: "label-tech", "Loading team…" } }
                }
            }
        }
    };

    let specialists = team.members.iter().filter(|m| m.name != team.lead).count();
    let active = missions()
        .iter()
        .filter(|m| !matches!(m.phase, TeamMissionPhase::Done | TeamMissionPhase::Rejected))
        .count();
    let lead = team.lead.clone();

    rsx! {
        div { class: "teams",
            // Team header.
            div { class: "glass-card settings-section",
                div { class: "panel-head",
                    h3 { "{team.name}" }
                    span { class: "chip", "{team.members.len()} members" }
                }
                if !team.description.is_empty() {
                    p { class: "label-tech", "{team.description}" }
                }
                div { class: "kv-grid",
                    div { span { class: "label-tech", "Lead" } div { "{team.lead}" } }
                    div { span { class: "label-tech", "Specialists" } div { "{specialists}" } }
                    div { span { class: "label-tech", "Active missions" } div { "{active}" } }
                }
            }

            // Roster — one card per member, lead first.
            div { class: "roster-grid",
                for m in team.members.clone() {
                    MemberCard { key: "{m.name}", is_lead: m.name == lead, m: m.clone() }
                }
            }
        }
    }
}

/// One team member — role + trust + scopes + tool count, expandable to the full
/// tool allowlist + the member's soul (system prompt).
#[component]
fn MemberCard(m: TeamMember, is_lead: bool) -> Element {
    let mut expanded = use_signal(|| false);
    let scopes = if m.capability_scopes.is_empty() {
        "—".to_string()
    } else {
        m.capability_scopes.join(", ")
    };

    rsx! {
        div { class: if is_lead { "glass-card member-card lead" } else { "glass-card member-card" },
            div { class: "panel-head",
                h4 { "{m.name}" }
                if is_lead {
                    span { class: "chip amber", "lead" }
                }
                span { class: "chip {trust_class(m.trust_ceiling)}", "{trust_label(m.trust_ceiling)}" }
            }
            p { class: "member-role", "{m.role}" }
            div { class: "member-meta",
                span { class: "label-tech", "scopes: {scopes}" }
                span { class: "label-tech", "tools: {m.tool_allowlist.len()}" }
            }
            button {
                class: "btn btn-glass btn-xs",
                onclick: move |_| expanded.toggle(),
                {if expanded() { "Hide soul ▴" } else { "Show soul ▾" }}
            }
            if expanded() {
                div { class: "member-detail",
                    if !m.tool_allowlist.is_empty() {
                        div { class: "tool-chips",
                            for t in m.tool_allowlist.clone() {
                                span { class: "chip", "{t}" }
                            }
                        }
                    }
                    pre { class: "soul", "{m.soul}" }
                }
            }
        }
    }
}

fn get_team_roster_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-teams".to_string(),
        payload: QueryPayload::GetTeamRoster,
    }
}

fn trust_label(t: TrustTier) -> &'static str {
    match t {
        TrustTier::Untrusted => "untrusted",
        TrustTier::SemiTrusted => "semi-trusted",
        TrustTier::Trusted => "trusted",
        TrustTier::Kernel => "kernel",
    }
}

/// Chip accent for a trust tier — higher trust reads sage (calm), lower amber.
fn trust_class(t: TrustTier) -> &'static str {
    match t {
        TrustTier::Trusted | TrustTier::Kernel => "sage",
        TrustTier::SemiTrusted => "amber",
        TrustTier::Untrusted => "muted",
    }
}

// ---------------------------------------------------------------------------
// Documents — the read-only file browser (Chapter Z). Two roots (workspace +
// the access-scoped fs_root); list + read, escape-guarded daemon-side.
// ---------------------------------------------------------------------------

#[component]
fn DocumentsPanel() -> Element {
    let ws = use_context::<Sender>();
    let mut documents = use_context::<Signal<DocumentsState>>();

    // First entry → default to the workspace root.
    use_future(move || async move {
        if documents.read().root.is_empty() {
            documents.write().root = "workspace".to_string();
            ws.send(list_dir_query("workspace", ""));
        }
    });

    let d = documents();
    let root = if d.root.is_empty() { "workspace".to_string() } else { d.root.clone() };

    rsx! {
        div { class: "documents",
            // Root switcher.
            div { class: "doc-toolbar",
                button {
                    class: if root == "workspace" { "btn btn-primary btn-xs" } else { "btn btn-glass btn-xs" },
                    onclick: move |_| switch_doc_root(documents, ws, "workspace"),
                    "Workspace"
                }
                button {
                    class: if root == "fs" { "btn btn-primary btn-xs" } else { "btn btn-glass btn-xs" },
                    onclick: move |_| switch_doc_root(documents, ws, "fs"),
                    "Files"
                }
            }

            // Breadcrumb: root + each ancestor segment, clickable to ascend.
            div { class: "breadcrumb",
                {
                    let r = root.clone();
                    rsx! {
                        button { class: "crumb", onclick: move |_| ws.send(list_dir_query(&r, "")),
                            {if root == "fs" { "fs_root" } else { "workspace" }} }
                    }
                }
                for (label, prefix) in breadcrumb_segments(&d.path) {
                    {
                        let (r, p) = (root.clone(), prefix.clone());
                        rsx! {
                            span { class: "crumb-sep", "/" }
                            button { class: "crumb", onclick: move |_| ws.send(list_dir_query(&r, &p)), "{label}" }
                        }
                    }
                }
            }

            if let Some(msg) = d.notice.clone() {
                div { class: "notice err", "{msg}" }
            }

            // File viewer (when one is open) else the directory listing.
            if let Some(file) = d.file.clone() {
                FileViewer { file }
            } else {
                div { class: "glass-card doc-listing",
                    if d.entries.is_empty() {
                        p { class: "label-tech sub", "Empty directory." }
                    } else {
                        for e in d.entries.clone() {
                            {
                                let target = join_doc_path(&d.path, &e.name);
                                let (r, is_dir) = (root.clone(), e.kind == "dir");
                                rsx! {
                                    button {
                                        class: "doc-row",
                                        onclick: move |_| {
                                            if is_dir {
                                                ws.send(list_dir_query(&r, &target));
                                            } else {
                                                ws.send(read_file_query(&r, &target));
                                            }
                                        },
                                        span { class: "doc-ico", {kind_glyph(&e.kind)} }
                                        span { class: "doc-name", "{e.name}" }
                                        span { class: "doc-size label-tech",
                                            {if e.kind == "file" { fmt_size(e.size_bytes) } else { String::new() }} }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The file content pane — text in a mono `<pre>`, or a "not shown" note for
/// binary / over-cap files. A back action returns to the listing.
#[component]
fn FileViewer(file: DocFile) -> Element {
    let mut documents = use_context::<Signal<DocumentsState>>();
    rsx! {
        div { class: "glass-card doc-viewer",
            div { class: "panel-head",
                h4 { "{file.path}" }
                span { class: "chip", {fmt_size(file.size_bytes)} }
                button { class: "btn btn-glass btn-xs", onclick: move |_| documents.write().file = None, "Close" }
            }
            if file.truncated {
                div { class: "notice err", "Showing the first 256 KB of a larger file." }
            }
            match &file.content {
                Some(text) => rsx! { pre { class: "doc-text", "{text}" } },
                None => rsx! {
                    p { class: "label-tech sub",
                        {if file.binary {
                            format!("Binary file — {} not shown.", fmt_size(file.size_bytes))
                        } else {
                            "File too large to display.".to_string()
                        }}
                    }
                },
            }
        }
    }
}

/// Switch the Documents browser to `to` ("workspace" | "fs"), reset to that
/// root's top, and re-list. A free fn so both toolbar buttons can call it
/// (a shared closure can't be moved into two handlers).
fn switch_doc_root(mut documents: Signal<DocumentsState>, ws: Sender, to: &'static str) {
    {
        let mut st = documents.write();
        st.root = to.to_string();
        st.file = None;
        st.path = String::new();
    }
    ws.send(list_dir_query(to, ""));
}

fn list_dir_query(root: &str, path: &str) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-docs-list".to_string(),
        payload: QueryPayload::ListDir { root: root.to_string(), path: path.to_string() },
    }
}

fn read_file_query(root: &str, path: &str) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-docs-read".to_string(),
        payload: QueryPayload::ReadFile { root: root.to_string(), path: path.to_string() },
    }
}

/// Join a relative dir `base` with an entry `name` (slash-separated).
fn join_doc_path(base: &str, name: &str) -> String {
    if base.is_empty() {
        name.to_string()
    } else {
        format!("{base}/{name}")
    }
}

/// `(label, cumulative-prefix)` for each segment of `path`, for the breadcrumb.
fn breadcrumb_segments(path: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut prefix = String::new();
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        if prefix.is_empty() {
            prefix = seg.to_string();
        } else {
            prefix = format!("{prefix}/{seg}");
        }
        out.push((seg.to_string(), prefix.clone()));
    }
    out
}

fn kind_glyph(kind: &str) -> &'static str {
    match kind {
        "dir" => "📁",
        "symlink" => "🔗",
        "file" => "📄",
        _ => "•",
    }
}

/// Human-readable byte size (B / KB / MB).
fn fmt_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
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
    mut settings: Signal<SettingsState>,
    mut agents: Signal<AgentsState>,
    mut roster: Signal<Option<TeamConfig>>,
    mut documents: Signal<DocumentsState>,
    mut voice: Signal<VoiceState>,
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
                    payload: QueryResponsePayload::GetTeamRoster { roster: cfg },
                    ..
                } => {
                    roster.set(Some(cfg));
                }
                // Chapter Z — Documents browser: a directory listing arrived;
                // the echoed `path` is authoritative. Re-listing closes any open
                // file and clears the notice.
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::ListDir { entries, path },
                    ..
                } => {
                    let mut d = documents.write();
                    d.entries = entries;
                    d.path = path;
                    d.file = None;
                    d.notice = None;
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::ReadFile { file },
                    ..
                } => {
                    let mut d = documents.write();
                    d.file = Some(file);
                    d.notice = None;
                }
                // A denied path / read failure on the Documents screen (ids
                // prefixed `mc-docs`) → a notice, leaving the listing intact.
                DaemonEnvelope::QueryResponse {
                    id,
                    payload: QueryResponsePayload::QueryError { message, .. },
                } if id.starts_with("mc-docs") => {
                    documents.write().notice = Some(message);
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
                    id,
                    payload: QueryResponsePayload::GetProfile { profile },
                } => {
                    // Route by query id: the editor's request (`mc-agents-get`,
                    // from_disk) seeds the editor's on-disk view; every other
                    // GetProfile is the dashboard's active/running snapshot for
                    // the name chip. They carry different data now, so they must
                    // not cross-populate.
                    if id == "mc-agents-get" {
                        agents.write().profile = Some(profile);
                    } else {
                        dashboard.write().assistant_name = Some(profile.assistant_name);
                    }
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
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::GetSettings { settings: snap },
                    ..
                } => {
                    settings.write().snapshot = Some(snap);
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::SettingsApplied { settings: snap, restart_required },
                    ..
                } => {
                    let mut s = settings.write();
                    s.snapshot = Some(snap);
                    s.restart_required = restart_required;
                    s.notice = Some((true, "Saved to aivyx.toml.".to_string()));
                }
                // Chapter Voice — the [voice] config + readiness.
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::GetVoiceSettings { settings: snap },
                    ..
                } => {
                    voice.write().snapshot = Some(snap);
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::VoiceApplied { settings: snap, restart_required },
                    ..
                } => {
                    let mut v = voice.write();
                    v.snapshot = Some(snap);
                    v.restart_required = restart_required;
                    v.notice = Some((true, "Saved to aivyx.toml.".to_string()));
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::ProfileApplied { profile, restart_required },
                    ..
                } => {
                    let mut a = agents.write();
                    a.profile = Some(profile);
                    a.restart_required = restart_required;
                    a.notice = Some((true, "Saved to aivyx.toml.".to_string()));
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::GetEffectivePersona { persona },
                    ..
                } => {
                    agents.write().persona = Some(persona);
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::ListPersonaProposals { proposals, .. },
                    ..
                } => {
                    agents.write().proposals = proposals;
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::ListPersonaDeltas { mut entries, .. },
                    ..
                } => {
                    // Newest first for the change-history view.
                    entries.reverse();
                    agents.write().deltas = entries;
                }
                // Persona governance acks (live — the daemon has already
                // recomputed runtime state). Bump the refresh tick so the panel
                // re-queries proposals + deltas + the folded persona.
                DaemonEnvelope::PersonaProposalResolved { ok, success, error, .. } => {
                    let mut a = agents.write();
                    a.refresh_tick += 1;
                    a.notice = Some(if ok {
                        let status = success
                            .map(|s| s.proposal_status.to_lowercase())
                            .unwrap_or_else(|| "resolved".to_string());
                        (true, format!("Proposal {status} — effective next turn."))
                    } else {
                        (false, error.unwrap_or_else(|| "resolve failed".to_string()))
                    });
                }
                DaemonEnvelope::PersonaRevertResolved { ok, error, .. } => {
                    let mut a = agents.write();
                    a.refresh_tick += 1;
                    a.notice = Some(if ok {
                        (true, "Delta reverted — effective next turn.".to_string())
                    } else {
                        (false, error.unwrap_or_else(|| "revert failed".to_string()))
                    });
                }
                // X.3 — LLM seed draft arrived (or failed). The onboarding card
                // watches `seed_draft_resp` to clear its spinner + re-seed its
                // form from `seed_draft`.
                DaemonEnvelope::PersonaSeedDrafted { draft, error, .. } => {
                    let mut a = agents.write();
                    a.seed_draft_resp += 1;
                    let had_draft = draft.is_some();
                    a.seed_draft = draft;
                    if !had_draft {
                        a.notice = Some((
                            false,
                            error.unwrap_or_else(|| "couldn't draft a seed".to_string()),
                        ));
                    }
                }
                // X.3 — live seed planted (or refused). On success bump the
                // refresh tick so the panel re-queries and the card gives way to
                // the normal governance view.
                DaemonEnvelope::PersonaSeedResolved { ok, appended, error, .. } => {
                    let mut a = agents.write();
                    a.notice = Some(if ok {
                        a.refresh_tick += 1;
                        (true, format!("Seeded {appended} trait(s) — effective next turn."))
                    } else {
                        (false, error.unwrap_or_else(|| "seeding failed".to_string()))
                    });
                }
                // Route a Settings write/read failure to its panel (the query
                // ids are prefixed so other QueryErrors don't hijack the banner).
                DaemonEnvelope::QueryResponse {
                    id,
                    payload: QueryResponsePayload::QueryError { message, .. },
                } if id.starts_with("mc-settings") => {
                    settings.write().notice = Some((false, message));
                }
                // Same routing for a Profile write/read failure on the Agents
                // screen (ids prefixed `mc-agents`).
                DaemonEnvelope::QueryResponse {
                    id,
                    payload: QueryResponsePayload::QueryError { message, .. },
                } if id.starts_with("mc-agents") => {
                    agents.write().notice = Some((false, message));
                }
                // And a [voice] write/read failure (ids prefixed `mc-voice`).
                DaemonEnvelope::QueryResponse {
                    id,
                    payload: QueryResponsePayload::QueryError { message, .. },
                } if id.starts_with("mc-voice") => {
                    voice.write().notice = Some((false, message));
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
