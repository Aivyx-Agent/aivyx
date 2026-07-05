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
    McpServerStatusView, MemoryEntrySummary, MemoryGraphNode, PersonaDeltaSummary,
    PersonaProposalResolution,
    PersonaProposalSummary, PersonaSeedWire, ProfileDraftWire, ProfileSummary, QueryPayload,
    QueryResponsePayload, ScheduleView, SeedSkillWire, SettingsSnapshot, SkillView,
    StreamEventPayload, VoiceSettingsSnapshot,
};
use aivyx_ipc::{
    PairScore, ProposedPersonaDelta, TeamConfig, TeamMember, TeamMissionPhase, TeamMissionView,
    TrustTier,
};
use aivyx_ipc::wiki::{WikiPage, WikiPageSummary};
use aivyx_ipc::graph::{GraphEntity, GraphTriple};

/// End-user guide content + markdown rendering for the Guide screen.
mod guide;

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
// Distinct per-screen nav icons (UI polish): every sidebar item gets its own
// glyph instead of sharing one. `plugins`/`candle-flame` are pre-existing brand
// spares; `wiki`/`graph`/`skills`/`guide` were authored to match the set.
const ICON_WIKI: Asset = asset!("/assets/icons/wiki.svg");
const ICON_GRAPH: Asset = asset!("/assets/icons/graph.svg");
const ICON_SKILLS: Asset = asset!("/assets/icons/skills.svg");
const ICON_GUIDE: Asset = asset!("/assets/icons/guide.svg");
const ICON_PLUGINS: Asset = asset!("/assets/icons/plugins.svg");
const ICON_CREATE: Asset = asset!("/assets/icons/candle-flame.svg");

/// The shared WebSocket-sender handle (poll loop + UI handlers send to it).
type Sender = Coroutine<FrontendMessage>;

/// Top-level view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum View {
    Command,
    Missions,
    Chat,
    Memory,
    /// Chapter Codex — the knowledge-wiki: synthesized per-topic pages.
    Wiki,
    /// Chapter Lattice — the typed knowledge graph: entities + directed
    /// typed relations.
    Lattice,
    /// Chapter Repertoire — the Skills library: every skill + its
    /// effectiveness + provenance/lineage.
    Skills,
    Settings,
    Agents,
    Teams,
    Documents,
    /// Chapter Lantern — the MCP screen: each configured MCP server's
    /// last-start health (connected + tool count, or failed + reason).
    Mcp,
    Voice,
    /// The in-app end-user guide — the `docs/guide/*.md` pages rendered in the
    /// Studio (see `guide.rs`). Pure static content, no daemon IPC.
    Guide,
    /// Chapter Genesis — the guided agent-creation flow (Profile → Persona seed
    /// → access). First-run lands here when the Profile isn't yet declared.
    Onboarding,
}

impl View {
    /// Every view, in sidebar order — drives the command palette + slug lookup.
    const ALL: [View; 15] = [
        View::Command,
        View::Chat,
        View::Missions,
        View::Memory,
        View::Wiki,
        View::Lattice,
        View::Onboarding,
        View::Agents,
        View::Skills,
        View::Teams,
        View::Documents,
        View::Mcp,
        View::Voice,
        View::Settings,
        View::Guide,
    ];

    /// The URL-hash slug for this view (deep-linking: `…/#memory`).
    fn slug(self) -> &'static str {
        match self {
            View::Command => "command",
            View::Missions => "missions",
            View::Chat => "chat",
            View::Memory => "memory",
            View::Wiki => "wiki",
            View::Lattice => "graph",
            View::Skills => "skills",
            View::Settings => "settings",
            View::Agents => "agents",
            View::Teams => "teams",
            View::Documents => "documents",
            View::Mcp => "mcp",
            View::Voice => "voice",
            View::Guide => "guide",
            View::Onboarding => "create",
        }
    }

    /// Parse a slug back to a view (for reading the URL hash on load / back).
    fn from_slug(s: &str) -> Option<View> {
        View::ALL.into_iter().find(|v| v.slug() == s)
    }

    /// Human label for the command palette (matches the sidebar).
    fn label(self) -> &'static str {
        match self {
            View::Command => "Command",
            View::Missions => "Missions",
            View::Chat => "Chat",
            View::Memory => "Memory",
            View::Wiki => "Wiki",
            View::Lattice => "Graph",
            View::Skills => "Skills",
            View::Settings => "Settings",
            View::Agents => "Agents",
            View::Teams => "Teams",
            View::Documents => "Documents",
            View::Mcp => "MCP",
            View::Voice => "Voice",
            View::Guide => "Guide",
            View::Onboarding => "Create",
        }
    }
}

/// Memory browser state — read-only snapshots fanned in by `ws_task`.
#[derive(Clone, Default, PartialEq)]
struct MemoryState {
    topics: Vec<String>,
    entries: Vec<MemoryEntrySummary>,
    /// True when a semantic search was transparently served by the keyword path.
    fell_back: bool,
    /// MG — the knowledge-graph nodes (topics + entry counts).
    graph_nodes: Vec<MemoryGraphNode>,
    /// MG — the weighted co-occurrence edges (empty ⇒ a topic cloud).
    graph_edges: Vec<PairScore>,
    /// `false` until the first entries snapshot arrives — distinguishes "still
    /// loading" from "genuinely no memories yet" so the panel shows a skeleton.
    loaded: bool,
}

/// Chapter Codex — knowledge-wiki browser state. `pages` is the index
/// (compact rows); `selected` is the open page (full summary + backlinks
/// + source seqs). Read-only snapshots fanned in by `ws_task`.
#[derive(Clone, Default, PartialEq)]
struct WikiState {
    pages: Vec<WikiPageSummary>,
    selected: Option<WikiPage>,
}

/// Chapter Repertoire — Skills library state: the skill inventory (each
/// `SkillView` = a `LearnedSkill` + its WH.2 effectiveness) + the count of
/// pending skill proposals (governed in Agents). Read-only snapshot fanned
/// in by `ws_task`.
#[derive(Clone, Default, PartialEq)]
struct SkillsState {
    skills: Vec<SkillView>,
    pending_proposals: usize,
    loaded: bool,
}

/// Chapter Lattice — typed knowledge-graph state: entity nodes + the
/// directed typed edges, fanned in by `ws_task`.
#[derive(Clone, Default, PartialEq)]
struct GraphKnowledgeState {
    entities: Vec<GraphEntity>,
    edges: Vec<GraphTriple>,
}

/// Chapter Lantern — MCP screen state: each configured server's last-start
/// health + the snapshot's capture time. Read-only snapshot fanned in by
/// `ws_task`. `loaded` flips on the first `GetMcpStatus` response so the
/// panel can tell "still loading" from "genuinely no servers".
#[derive(Clone, Default, PartialEq)]
struct McpState {
    servers: Vec<McpServerStatusView>,
    captured_unix: u64,
    loaded: bool,
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

/// Teams screen state — Chapters Y (read) + Roster (RO.3, edit). The active
/// roster (re-read from disk after a save) plus the last write outcome + the
/// load-time restart flag. The editor seeds a local draft from `roster`.
#[derive(Clone, Default, PartialEq)]
struct TeamsState {
    /// The daemon's active team config. `None` until the first load.
    roster: Option<TeamConfig>,
    /// Last save outcome: `(ok, message)`. `None` until the first save.
    notice: Option<(bool, String)>,
    /// True after a successful save — the team file is written but the running
    /// daemon won't adopt it until it restarts (the team service is boot-built).
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
    /// GE.3 — the latest LLM-drafted **Profile** (the onboarding flow's step 1
    /// fills its six fields from this); `None` until a draft arrives or fails.
    profile_draft: Option<ProfileDraftWire>,
    /// GE.3 — bumped on every `DraftProfile` response (success or failure) so the
    /// onboarding flow can clear its "Drafting…" state and re-fill its form.
    profile_draft_resp: u64,
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
    /// Last outcome `(ok, message)` (a denied path / read failure / DW write).
    notice: Option<(bool, String)>,
    /// `false` until the first directory listing arrives (the `root` field is
    /// set client-side immediately, so it can't signal load) — drives the
    /// listing skeleton.
    loaded: bool,
}

/// Command Center dashboard state — read-only snapshots fanned in by `ws_task`.
#[derive(Clone, Default, PartialEq)]
struct Dashboard {
    audit_entries: Vec<AuditEntrySummary>,
    audit_total: u64,
    chain_ok: Option<bool>,
    assistant_name: Option<String>,
    /// The running agent's vitals (model / provider / context / autonomy /
    /// access) from `GetSettings` — drives the agent-vitals rail.
    settings: Option<SettingsSnapshot>,
    /// The agent's scheduled background routines (`GetSchedules`) — drives the
    /// Routines panel + stat card, the "live agent working on its own" signal.
    schedules: Vec<ScheduleView>,
    /// `false` until the first dashboard snapshot (the audit-entries response)
    /// arrives. Distinguishes "not loaded yet" from "loaded and genuinely
    /// empty" so the Command Center shows a skeleton instead of flashing zeros.
    loaded: bool,
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

/// The view named by the current URL hash (`…/#memory`), if it's a known slug.
fn hash_view() -> Option<View> {
    let h = web_sys::window()?.location().hash().ok()?;
    View::from_slug(h.trim_start_matches('#'))
}

/// The view to start on: the URL hash if it names a real screen, else Command.
/// (First-run onboarding still takes over via the no-profile redirect.)
fn initial_view() -> View {
    hash_view().unwrap_or(View::Command)
}

#[component]
fn App() -> Element {
    // Deep-linking: the active view is mirrored in the URL hash, so screens are
    // bookmarkable/shareable and survive a reload, and back/forward navigate.
    let view = use_signal(initial_view);
    // view -> URL hash.
    use_effect(move || {
        let slug = view().slug();
        if let Some(loc) = web_sys::window().map(|w| w.location()) {
            if loc.hash().unwrap_or_default().trim_start_matches('#') != slug {
                let _ = loc.set_hash(slug);
            }
        }
    });
    // URL hash -> view (back/forward, manual edits). Registered once.
    use_hook(|| {
        use wasm_bindgen::JsCast;
        let mut view = view;
        let cb = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
            if let Some(v) = hash_view() {
                if view.peek().slug() != v.slug() {
                    view.set(v);
                }
            }
        });
        if let Some(w) = web_sys::window() {
            let _ = w
                .add_event_listener_with_callback("hashchange", cb.as_ref().unchecked_ref());
        }
        cb.forget();
    });

    // Command palette (Ctrl/Cmd-K) — a global keydown listener toggles it.
    let palette_open = use_signal(|| false);
    use_hook(|| {
        use wasm_bindgen::JsCast;
        let mut palette_open = palette_open;
        let cb = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(
            move |e: web_sys::Event| {
                let Some(ke) = e.dyn_ref::<web_sys::KeyboardEvent>() else {
                    return;
                };
                if (ke.ctrl_key() || ke.meta_key()) && ke.key() == "k" {
                    e.prevent_default();
                    let now = *palette_open.peek();
                    palette_open.set(!now);
                } else if ke.key() == "Escape" && *palette_open.peek() {
                    palette_open.set(false);
                }
            },
        );
        if let Some(w) = web_sys::window() {
            let _ = w.add_event_listener_with_callback("keydown", cb.as_ref().unchecked_ref());
        }
        cb.forget();
    });

    let connected = use_signal(|| false);
    let light = use_signal(|| false);
    // Mobile drawer state: below the shell breakpoint the sidebar is off-canvas
    // and this toggles it. Ignored on desktop (the sidebar is always in-grid).
    let mut nav_open = use_signal(|| false);
    // The Guide's current page — App-owned so the topbar "?" can deep-link it.
    let guide_page = use_signal(|| 0usize);
    let missions = use_signal(Vec::<TeamMissionView>::new);
    let dashboard = use_signal(Dashboard::default);
    let memory = use_signal(MemoryState::default);
    let wiki = use_signal(WikiState::default);
    let lattice = use_signal(GraphKnowledgeState::default);
    let settings = use_signal(SettingsState::default);
    let agents = use_signal(AgentsState::default);
    let teams = use_signal(TeamsState::default);
    let documents = use_signal(DocumentsState::default);
    let voice = use_signal(VoiceState::default);
    let skills = use_signal(SkillsState::default);
    let mcp = use_signal(McpState::default);
    // Chat state, shared with the read task + the Chat view (via context).
    let session = use_signal(|| None::<String>);
    let transcript = use_signal(Vec::<ChatLine>::new);
    let streaming = use_signal(String::new);
    let gate = use_signal(|| None::<GateInfo>);

    let ws: Sender = use_coroutine(move |rx| {
        ws_task(
            rx, missions, dashboard, memory, wiki, lattice, settings, agents, teams, documents,
            voice, skills, mcp, connected, session, transcript, streaming, gate,
        )
    });
    use_context_provider(|| ws);
    // Connection state, so action surfaces (mission bar, roster save) can
    // gate on a live socket instead of sending into a zombie page.
    use_context_provider(|| connected);
    use_context_provider(|| memory);
    use_context_provider(|| wiki);
    use_context_provider(|| lattice);
    use_context_provider(|| settings);
    use_context_provider(|| agents);
    use_context_provider(|| teams);
    use_context_provider(|| documents);
    use_context_provider(|| voice);
    use_context_provider(|| skills);
    use_context_provider(|| mcp);
    // Chapter Repertoire — the Skills screen's "review in Agents" pointer
    // switches the active view.
    use_context_provider(|| view);
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
            // Vitrine final sweep — routines have live-changing fields
            // (last-fired / next-fire move as crons tick), so the
            // Routines panel polls with the missions + audit feed
            // instead of staying a page-load snapshot. The schedule
            // list is tiny; the query is cheap.
            ws.send(FrontendMessage::Query {
                id: "mc-schedules".to_string(),
                payload: QueryPayload::GetSchedules,
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
        // Agent vitals (model / provider / context / autonomy / access) +
        // the scheduled background routines — the "live, working agent" panels.
        ws.send(FrontendMessage::Query {
            id: "mc-settings".to_string(),
            payload: QueryPayload::GetSettings,
        });
        ws.send(FrontendMessage::Query {
            id: "mc-schedules".to_string(),
            payload: QueryPayload::GetSchedules,
        });
    });

    let title = match view() {
        View::Command => "Command Center",
        View::Missions => "Mission Orchestration",
        View::Chat => "Terminal",
        View::Memory => "Memory",
        View::Wiki => "Knowledge Wiki",
        View::Lattice => "Knowledge Graph",
        View::Skills => "Skills",
        View::Settings => "Settings",
        View::Agents => "Agents",
        View::Teams => "Teams",
        View::Documents => "Documents",
        View::Mcp => "MCP Servers",
        View::Voice => "Voice",
        View::Guide => "Guide",
        View::Onboarding => "Create your agent",
    };

    rsx! {
        document::Title { "Aivyx Studio" }
        document::Link { rel: "icon", href: FAVICON }
        document::Stylesheet { href: STITCH_CSS }
        style { {font_faces()} }
        div { class: if nav_open() { "app nav-open" } else { "app" },
            // Keyboard a11y: first focusable element jumps past the nav.
            a { class: "skip-link", href: "#main-content", "Skip to content" }
            Sidebar { view, nav_open }
            // Mobile-only scrim behind the open drawer; tap to dismiss.
            div { class: "nav-backdrop", onclick: move |_| nav_open.set(false) }
            div { class: "main",
                if !connected() {
                    div {
                        style: "position:sticky;top:0;z-index:1000;background:var(--danger, #b91c1c);color:#fff;text-align:center;padding:6px 12px;font-size:13px;letter-spacing:0.02em;",
                        role: "alert",
                        "Connection to the agent lost — reconnecting…"
                    }
                }
                Topbar { title, light, nav_open, view, guide_page }
                main { class: "view fade-in", id: "main-content", tabindex: "-1",
                    match view() {
                        View::Command => rsx! {
                            CommandPanel { missions: missions(), dashboard: dashboard(), connected: connected() }
                        },
                        View::Missions => rsx! { MissionsPanel { missions: missions() } },
                        View::Chat => rsx! { ChatPanel {} },
                        View::Memory => rsx! { MemoryPanel {} },
                        View::Wiki => rsx! { WikiPanel {} },
                        View::Lattice => rsx! { LatticePanel {} },
                        View::Skills => rsx! { SkillsPanel {} },
                        View::Settings => rsx! { SettingsPanel {} },
                        View::Agents => rsx! { AgentsPanel {} },
                        View::Teams => rsx! { TeamsPanel {} },
                        View::Documents => rsx! { DocumentsPanel {} },
                        View::Mcp => rsx! { McpPanel {} },
                        View::Voice => rsx! { VoicePanel {} },
                        View::Guide => rsx! { GuidePanel { page: guide_page } },
                        View::Onboarding => rsx! { OnboardingPanel { view } },
                    }
                }
            }
            StatusBar { connected: connected(), agent_name: dashboard().assistant_name.clone().unwrap_or_default() }
            if palette_open() {
                CommandPalette { view, open: palette_open }
            }
        }
    }
}

/// Ctrl/Cmd-K command palette — fuzzy-jump to any screen. Type to filter,
/// arrows to move, Enter to go, Esc/backdrop to dismiss.
#[component]
fn CommandPalette(view: Signal<View>, open: Signal<bool>) -> Element {
    let mut query = use_signal(String::new);
    let mut selected = use_signal(|| 0usize);

    // The filtered screen list (case-insensitive label contains).
    let q = query().to_lowercase();
    let results: Vec<View> = View::ALL
        .into_iter()
        .filter(|v| q.is_empty() || v.label().to_lowercase().contains(&q))
        .collect();
    let sel = selected().min(results.len().saturating_sub(1));
    let kb = results.clone(); // snapshot moved into the keydown handler

    rsx! {
        div { class: "palette-backdrop", onclick: move |_| open.set(false),
            div { class: "palette", onclick: move |e| e.stop_propagation(),
                input {
                    class: "palette-input",
                    r#type: "text",
                    autofocus: true,
                    "aria-label": "Jump to a screen",
                    placeholder: "Jump to a screen…",
                    value: "{query}",
                    oninput: move |e| { query.set(e.value()); selected.set(0); },
                    onkeydown: move |e| {
                        let n = kb.len();
                        match e.key() {
                            Key::ArrowDown => { e.prevent_default(); if n > 0 { selected.set((sel + 1) % n); } }
                            Key::ArrowUp => { e.prevent_default(); if n > 0 { selected.set((sel + n - 1) % n); } }
                            Key::Enter => {
                                if let Some(v) = kb.get(sel).copied() { view.set(v); open.set(false); }
                            }
                            Key::Escape => open.set(false),
                            _ => {}
                        }
                    },
                }
                div { class: "palette-list",
                    if results.is_empty() {
                        div { class: "palette-empty label-tech", "No matching screen" }
                    } else {
                        for (i, v) in results.iter().copied().enumerate() {
                            button {
                                key: "{v.slug()}",
                                class: if i == sel { "palette-item active" } else { "palette-item" },
                                onmouseenter: move |_| selected.set(i),
                                onclick: move |_| { view.set(v); open.set(false); },
                                span { class: "palette-item-label", "{v.label()}" }
                                span { class: "palette-item-slug label-tech", "/#{v.slug()}" }
                            }
                        }
                    }
                }
                div { class: "palette-hint label-tech", "↑↓ navigate · ↵ open · esc close" }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// App shell — Sidebar / Topbar / StatusBar
// ---------------------------------------------------------------------------

/// One sidebar nav entry: its icon, label, and the view it opens.
type NavEntry = (Asset, &'static str, View);
/// A labeled sidebar group (`""` header ⇒ no label) and its entries.
type NavGroup = (&'static str, Vec<NavEntry>);

/// The sidebar navigation, grouped into labeled sections. Data-driven so the
/// IA is one table to read/reorder, and every item closes the mobile drawer on
/// click. An empty group header (`""`) renders no label (the lone Command item).
#[component]
fn Sidebar(view: Signal<View>, nav_open: Signal<bool>) -> Element {
    let groups: Vec<NavGroup> = vec![
        ("", vec![(ICON_COMMAND, "Command", View::Command)]),
        (
            "Workspace",
            vec![
                (ICON_CHAT, "Chat", View::Chat),
                (ICON_MISSIONS, "Missions", View::Missions),
            ],
        ),
        (
            "Knowledge",
            vec![
                (ICON_MEMORY, "Memory", View::Memory),
                (ICON_WIKI, "Wiki", View::Wiki),
                (ICON_GRAPH, "Graph", View::Lattice),
            ],
        ),
        (
            "Agent",
            vec![
                (ICON_CREATE, "Create", View::Onboarding),
                (ICON_AGENTS, "Agents", View::Agents),
                (ICON_SKILLS, "Skills", View::Skills),
                (ICON_TEAMS, "Teams", View::Teams),
            ],
        ),
        (
            "System",
            vec![
                (ICON_DOCUMENTS, "Documents", View::Documents),
                (ICON_PLUGINS, "MCP", View::Mcp),
                (ICON_VOICE, "Voice", View::Voice),
                (ICON_SETTINGS, "Settings", View::Settings),
                (ICON_GUIDE, "Guide", View::Guide),
            ],
        ),
    ];

    rsx! {
        aside { class: "sidebar",
            div { class: "brand-lockup",
                img { src: LOGOMARK, alt: "Aivyx" }
                span { class: "wordmark", "AIVYX" }
            }
            nav { "aria-label": "Primary",
                for (header, items) in groups {
                    if !header.is_empty() {
                        div { class: "nav-section label-tech", "{header}" }
                    }
                    for (icon, label, v) in items {
                        NavItem { icon, label, active: view() == v,
                            onclick: move |_| { view.set(v); nav_open.set(false); } }
                    }
                }
            }
            div { style: "flex:1" }
            a { class: "nav-item nav-classic", href: "/classic",
                onclick: move |_| nav_open.set(false), "▸ Classic UI ↗" }
        }
    }
}

#[component]
fn NavItem(icon: Asset, label: &'static str, active: bool, onclick: EventHandler<MouseEvent>) -> Element {
    rsx! {
        button {
            class: if active { "nav-item active" } else { "nav-item" },
            "aria-current": if active { "page" } else { "false" },
            onclick: move |e| onclick.call(e),
            span { class: "ico", style: "--ico: url({icon})" }
            "{label}"
        }
    }
}

/// The Guide screen — a page list plus the rendered markdown body. Pure static
/// content (bundled `docs/guide/*.md`), no daemon IPC. `selected` is the index
/// into [`guide::PAGES`]; the HTML is memoized so switching an unrelated signal
/// never re-parses the markdown.
///
/// Cross-page links inside the rendered markdown are real `.md` anchors (so they
/// also work on GitHub). In-app they aren't Dioxus-managed elements — they live
/// inside `dangerous_inner_html` — so we delegate: catch clicks on the content
/// container, walk up to the clicked `<a>`, and if its `href` names a guide page
/// ([`guide::index_for_href`]) switch to it instead of letting the browser
/// navigate away. External links (`http(s)://`, …) don't match and behave
/// normally.
#[component]
fn GuidePanel(page: Signal<usize>) -> Element {
    use dioxus::web::WebEventExt;
    use wasm_bindgen::JsCast;

    // The current page is shared (App-owned) so the topbar "?" help button can
    // open the Guide directly to the page relevant to the screen you were on.
    let mut selected = page;
    let body_html = use_memo(move || {
        let idx = selected().min(guide::PAGES.len().saturating_sub(1));
        guide::render(guide::PAGES[idx].body)
    });

    let on_content_click = move |evt: Event<MouseData>| {
        let Some(web_evt) = evt.try_as_web_event() else { return };
        let Some(target) = web_evt.target() else { return };
        let Some(el) = target.dyn_ref::<web_sys::Element>() else { return };
        // Nearest enclosing anchor (the click may land on text inside the <a>).
        let Ok(Some(anchor)) = el.closest("a") else { return };
        let Some(href) = anchor.get_attribute("href") else { return };
        if let Some(idx) = guide::index_for_href(&href) {
            evt.prevent_default();
            selected.set(idx);
        }
    };

    rsx! {
        div { class: "guide",
            nav { class: "guide-nav",
                div { class: "guide-nav-head label-tech", "User guide" }
                for (i, page) in guide::PAGES.iter().enumerate() {
                    button {
                        key: "{page.id}",
                        class: if selected() == i { "guide-link active" } else { "guide-link" },
                        onclick: move |_| selected.set(i),
                        "{page.title}"
                    }
                }
            }
            article {
                class: "guide-content glass-card",
                onclick: on_content_click,
                dangerous_inner_html: body_html(),
            }
        }
    }
}

#[component]
fn Topbar(
    title: &'static str,
    light: Signal<bool>,
    nav_open: Signal<bool>,
    mut view: Signal<View>,
    mut guide_page: Signal<usize>,
) -> Element {
    rsx! {
        header { class: "topbar",
            // Hamburger — CSS shows it only below the shell breakpoint.
            button {
                class: "icon-btn nav-toggle",
                "aria-label": "Toggle navigation",
                onclick: move |_| nav_open.toggle(),
                span { class: "hamburger" }
            }
            span { class: "title", "{title}" }
            div { class: "spacer" }
            // Daemon connection status lives in the status bar (footer) as the
            // single source — the topbar no longer duplicates it.
            // Contextual help: open the Guide to the page for the current screen.
            button {
                class: "icon-btn",
                title: "Help for this screen",
                "aria-label": "Open the guide for this screen",
                onclick: move |_| {
                    guide_page.set(guide_page_for(view()));
                    view.set(View::Guide);
                },
                span { class: "ico", style: "--ico: url({ICON_GUIDE})" }
            }
            button {
                class: "icon-btn",
                title: "Toggle theme",
                "aria-label": "Toggle light/dark theme",
                onclick: move |_| light.toggle(),
                span { class: "ico", style: "--ico: url({ICON_THEME})" }
            }
        }
    }
}

/// Map a screen to the most relevant end-user guide page (index into
/// [`guide::PAGES`]) for the topbar "?" help button.
fn guide_page_for(v: View) -> usize {
    let id = match v {
        View::Chat | View::Missions => "chat-and-missions",
        View::Memory | View::Wiki | View::Lattice => "memory",
        View::Skills | View::Agents => "skills-and-persona",
        View::Teams => "teams",
        View::Settings => "access-and-settings",
        View::Onboarding => "create-your-agent",
        View::Guide => "welcome",
        // Command / Documents / Mcp / Voice → the screens overview.
        _ => "screens",
    };
    guide::PAGES.iter().position(|p| p.id == id).unwrap_or(0)
}

#[component]
fn StatusBar(connected: bool, agent_name: String) -> Element {
    // Vitrine final sweep (2026-07-05): this segment was a hardcoded
    // "AGENT · NONAGON" literal — the one static datum in the shell.
    // It now shows the RUNNING agent's name from the dashboard profile
    // snapshot (empty until the first snapshot arrives).
    let agent = if agent_name.trim().is_empty() {
        "AGENT · —".to_string()
    } else {
        format!("AGENT · {}", agent_name.to_uppercase())
    };
    rsx! {
        footer { class: "statusbar label-tech",
            div { class: if connected { "seg live" } else { "seg" },
                span { class: "dot" }
                if connected { "DAEMON · CONNECTED" } else { "DAEMON · OFFLINE" }
            }
            div { class: "seg seg-mid", "{agent}" }
            div { class: "seg seg-ver", {format!("AIVYX · v{}", env!("CARGO_PKG_VERSION"))} }
        }
    }
}

// ---------------------------------------------------------------------------
// Command Center — the home dashboard (read-only)
// ---------------------------------------------------------------------------

#[component]
fn CommandPanel(missions: Vec<TeamMissionView>, dashboard: Dashboard, connected: bool) -> Element {
    // Until the first dashboard snapshot arrives, show a skeleton instead of
    // flashing placeholder zeros (which then pop to real values on load).
    if !dashboard.loaded {
        return rsx! { CommandSkeleton {} };
    }
    let active = missions
        .iter()
        .filter(|m| !m.phase.is_terminal())
        .count();
    let chain = dashboard.chain_ok;
    let routines = dashboard.schedules.clone();
    let routines_total = routines.len();
    let routines_on = routines.iter().filter(|r| r.enabled).count();
    rsx! {
        div { class: "stat-row stat-row-5",
            StatCard { icon: ICON_MISSIONS, label: "Missions", value: "{missions.len()}", tone: None }
            StatCard { icon: ICON_AGENTS, label: "Active", value: "{active}", tone: None }
            StatCard { icon: ICON_COMMAND, label: "Routines", value: "{routines_on}/{routines_total}", tone: None }
            StatCard { icon: ICON_MEMORY, label: "Audit Events", value: "{dashboard.audit_total}", tone: None }
            StatCard { icon: ICON_SETTINGS, label: "Chain", value: chain_label(chain).to_string(), tone: chain_tone(chain) }
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
                        h3 { "Routines" }
                        span { class: "label-tech", "{routines_on} of {routines_total} active" }
                    }
                    if routines.is_empty() {
                        div { class: "glass-card empty", p { class: "label-tech", "No background routines configured." } }
                    } else {
                        div { class: "feed",
                            for r in routines.iter() {
                                RoutineRow { routine: r.clone() }
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
                AgentStatus { name: dashboard.assistant_name.clone(), connected, chain_ok: chain, settings: dashboard.settings.clone() }
            }
        }
    }
}

/// Loading skeleton for the Command Center — mirrors the real layout (4 stat
/// cards + the two-panel main + rail) so swapping in live data causes no shift.
#[component]
fn CommandSkeleton() -> Element {
    rsx! {
        div { class: "stat-row",
            for i in 0..4 {
                div { key: "{i}", class: "glass-card stat-card",
                    div { class: "stat-top",
                        span { class: "skeleton sk-ico" }
                        span { class: "skeleton sk-line sk-w40" }
                    }
                    span { class: "skeleton sk-value" }
                }
            }
        }
        div { class: "dash-grid",
            div { class: "dash-main",
                section { class: "panel",
                    div { class: "panel-head", span { class: "skeleton sk-line sk-w30" } }
                    div { class: "glass-card",
                        span { class: "skeleton sk-line sk-w70" }
                        span { class: "skeleton sk-line sk-w50" }
                    }
                }
                section { class: "panel",
                    div { class: "panel-head", span { class: "skeleton sk-line sk-w30" } }
                    div { class: "glass-card",
                        for i in 0..3 {
                            span { key: "{i}", class: "skeleton sk-line sk-w60" }
                        }
                    }
                }
            }
            aside { class: "dash-rail",
                div { class: "glass-card",
                    span { class: "skeleton sk-line sk-w50" }
                    span { class: "skeleton sk-line sk-w80" }
                    span { class: "skeleton sk-line sk-w70" }
                }
            }
        }
    }
}

/// Generic loading skeleton — `rows` shimmer cards stacked vertically. For
/// list-style screens (Memory entries, Documents files, Teams roster). Reuses
/// the `.feed` layout so the swap to real rows causes no shift.
#[component]
fn SkeletonList(rows: usize) -> Element {
    rsx! {
        div { class: "feed",
            for i in 0..rows {
                div { key: "{i}", class: "glass-card",
                    span { class: "skeleton sk-line sk-w40" }
                    span { class: "skeleton sk-line sk-w80" }
                }
            }
        }
    }
}

/// Generic loading skeleton — `cards` shimmer cards in the auto-fill grid. For
/// grid-style screens (Skills, MCP).
#[component]
fn SkeletonCards(cards: usize) -> Element {
    rsx! {
        div { class: "mcp-grid",
            for i in 0..cards {
                div { key: "{i}", class: "glass-card",
                    span { class: "skeleton sk-line sk-w50" }
                    span { class: "skeleton sk-line sk-w70" }
                    span { class: "skeleton sk-line sk-w30" }
                }
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
fn RoutineRow(routine: ScheduleView) -> Element {
    let last = routine
        .last_fired_unix_ms
        .map(rel_time)
        .unwrap_or_else(|| "never".to_string());
    let next = if routine.enabled {
        routine
            .next_fire_unix_ms
            .map(until_time)
            .unwrap_or_else(|| "—".to_string())
    } else {
        "paused".to_string()
    };
    rsx! {
        div { class: "glass-card routine-row",
            div { class: "row1",
                span { class: if routine.enabled { "dot live" } else { "dot off" } }
                span { class: "name", "{routine.name}" }
                span { class: "label-tech cron", "{routine.cron}" }
            }
            div { class: "row2 label-tech",
                span { "next " span { class: "v", "{next}" } }
                span { "last " span { class: "v", "{last}" } }
            }
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
fn AgentStatus(
    name: Option<String>,
    connected: bool,
    chain_ok: Option<bool>,
    settings: Option<SettingsSnapshot>,
) -> Element {
    let agent = name.unwrap_or_else(|| "—".to_string());
    let chain_class = match chain_tone(chain_ok) {
        Some(t) => format!("v {t}"),
        None => "v".to_string(),
    };
    let chain = chain_label(chain_ok);
    // Live agent vitals from the running snapshot (GetSettings). Precomputed so
    // the rsx stays declarative.
    let has_vitals = settings.is_some();
    let (model, provider, ctx, autonomy, access) = match &settings {
        Some(s) => {
            let ctx = match s.num_ctx {
                Some(n) if n % 1024 == 0 => format!("{}k tok", n / 1024),
                Some(n) => format!("{n} tok"),
                None => "auto".to_string(),
            };
            (
                s.model.clone(),
                s.provider.clone(),
                ctx,
                s.autonomy_level.clone(),
                s.access_level.clone(),
            )
        }
        None => (
            "—".to_string(),
            "—".to_string(),
            "—".to_string(),
            "—".to_string(),
            "—".to_string(),
        ),
    };
    rsx! {
        section { class: "glass-card agent-status",
            div { class: "panel-head", h3 { "Agent" } }
            div { class: "kv",
                span { class: "label-tech", "Name" }
                span { class: "v", "{agent}" }
            }
            div { class: "kv",
                span { class: "label-tech", "Daemon" }
                span { class: if connected { "v ok" } else { "v off" },
                    if connected { span { class: "dot live" } }
                    if connected { "online" } else { "offline" }
                }
            }
            if has_vitals {
                div { class: "kv",
                    span { class: "label-tech", "Model" }
                    span { class: "v mono", "{model}" }
                }
                div { class: "kv",
                    span { class: "label-tech", "Provider" }
                    span { class: "v", "{provider} · {ctx}" }
                }
                div { class: "kv",
                    span { class: "label-tech", "Autonomy" }
                    span { class: "v", "{autonomy}" }
                }
                div { class: "kv",
                    span { class: "label-tech", "Access" }
                    span { class: "v", "{access}" }
                }
            }
            div { class: "kv",
                span { class: "label-tech", "Chain" }
                span { class: "{chain_class}", "{chain}" }
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
    let connected = use_context::<Signal<bool>>();
    let mut goal = use_signal(String::new);
    let ready = connected();
    rsx! {
        div { class: "newbar",
            input {
                class: "input",
                "aria-label": "New mission goal",
                placeholder: if ready { "new mission goal — e.g. \"audit the deps for CVEs\"" } else { "reconnecting…" },
                disabled: !ready,
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
                disabled: !ready,
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
                        "aria-label": "Message",
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
    // MG — List ⇄ Graph view toggle (local; the graph data loads alongside).
    let mut graph_view = use_signal(|| false);

    // Load topics + the recent-across-all default + the graph each time the
    // view opens.
    use_future(move || async move {
        ws.send(mem_topics_query());
        ws.send(mem_search_query(String::new(), false));
        ws.send(mem_graph_query());
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
                        "aria-label": "Search memory",
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
                    div { class: "mem-viewtoggle",
                        button {
                            class: if graph_view() { "btn btn-glass btn-xs" } else { "btn btn-primary btn-xs" },
                            onclick: move |_| graph_view.set(false),
                            "List"
                        }
                        button {
                            class: if graph_view() { "btn btn-primary btn-xs" } else { "btn btn-glass btn-xs" },
                            onclick: move |_| graph_view.set(true),
                            "Graph"
                        }
                    }
                }
                if graph_view() {
                    if m.graph_nodes.is_empty() {
                        div { class: "glass-card empty",
                            p { class: "label-tech", "No topics to graph yet." }
                        }
                    } else {
                        MemoryGraph {
                            nodes: m.graph_nodes.clone(),
                            edges: m.graph_edges.clone(),
                            on_select: move |topic: String| {
                                graph_view.set(false);
                                scope.set(format!("topic:{topic}"));
                                ws.send(mem_topic_query(topic));
                            },
                        }
                    }
                } else if !m.loaded {
                    SkeletonList { rows: 4 }
                } else if m.entries.is_empty() {
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

fn mem_graph_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-mem-graph".to_string(),
        payload: QueryPayload::GetMemoryGraph { limit: 60 },
    }
}

// ---------------------------------------------------------------------------
// Wiki view — Chapter Codex (CX.5). The synthesized per-topic knowledge
// pages: an index rail → a page (summary + co-occurrence backlinks +
// source-entry refs). Read-only; pages are derived from memory.
// ---------------------------------------------------------------------------

#[component]
fn WikiPanel() -> Element {
    let ws = use_context::<Sender>();
    let wiki = use_context::<Signal<WikiState>>();

    // Load the page index each time the view opens.
    use_future(move || async move {
        ws.send(wiki_list_query());
    });

    let w = wiki();
    let selected_topic = w.selected.as_ref().map(|p| p.topic.clone());
    rsx! {
        div { class: "mem",
            aside { class: "mem-rail",
                div { class: "panel-head", h3 { "Pages" } span { class: "label-tech", "{w.pages.len()}" } }
                if w.pages.is_empty() {
                    p { class: "label-tech", style: "padding:8px",
                        "No pages yet. Set [memory] profile = \"smart\" (or [wiki] enabled = true) and the agent consolidates each memory topic into a page."
                    }
                }
                for p in w.pages.iter() {
                    {
                        let topic = p.topic.clone();
                        let sel = selected_topic.as_deref() == Some(p.topic.as_str());
                        rsx! {
                            button {
                                class: if sel { "mem-topic active" } else { "mem-topic" },
                                onclick: move |_| ws.send(wiki_page_query(topic.clone())),
                                "{p.topic}"
                                span { class: "label-tech", style: "float:right", "{p.entry_count}" }
                            }
                        }
                    }
                }
            }
            div { class: "mem-main",
                match w.selected.clone() {
                    Some(page) => rsx! { WikiPageView { page } },
                    None => rsx! {
                        div { class: "glass-card empty",
                            p { class: "label-tech",
                                "Select a page. Each is the agent's consolidated summary of one memory topic — what it knows, not just what it logged."
                            }
                        }
                    },
                }
            }
        }
    }
}

#[component]
fn WikiPageView(page: WikiPage) -> Element {
    let ws = use_context::<Sender>();
    rsx! {
        div { class: "glass-card",
            div { class: "mem-entry-head",
                h3 { "{page.topic}" }
                span { class: "chip", "{page.entry_count} entries" }
                span { class: "when label-tech", "updated {rel_time_secs(page.updated_at)}" }
            }
            p { class: "mem-body", style: "white-space:pre-wrap", "{page.summary}" }
            if !page.backlinks.is_empty() {
                div { class: "panel-head", h3 { class: "label-tech", "Related" } }
                div { class: "wiki-backlinks",
                    for b in page.backlinks.iter() {
                        {
                            let topic = b.topic.clone();
                            rsx! {
                                button {
                                    class: "chip",
                                    title: "affinity {b.affinity:.2} · {b.hops} hop(s)",
                                    onclick: move |_| ws.send(wiki_page_query(topic.clone())),
                                    "{b.topic}"
                                }
                            }
                        }
                    }
                }
            }
            div { class: "when label-tech", style: "margin-top:8px",
                "consolidated from {page.source_seqs.len()} memory entr",
                if page.source_seqs.len() == 1 { "y" } else { "ies" }
            }
        }
    }
}

fn wiki_list_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-wiki-list".to_string(),
        payload: QueryPayload::ListWikiPages,
    }
}

fn wiki_page_query(topic: String) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-wiki-page".to_string(),
        payload: QueryPayload::GetWikiPage { topic },
    }
}

// ---------------------------------------------------------------------------
// Skills view — Chapter Repertoire (RP.2). The agent's whole repertoire of
// skills (operator-taught, agent-authored, agent-refined) with their WH.2
// effectiveness + provenance/lineage. Read-only; governance (approve/edit/
// reject of skill proposals) stays in the Agents screen — this points there.
// ---------------------------------------------------------------------------

fn skills_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-skills".to_string(),
        payload: QueryPayload::GetSkills,
    }
}

fn forget_skill_query(name: &str) -> FrontendMessage {
    FrontendMessage::ForgetSkill {
        id: format!("mc-skill-forget-{name}"),
        name: name.to_string(),
    }
}

/// Effectiveness bucket label + bar fraction from the WH.2 EWMA + samples.
/// `samples == 0` ⇒ unmeasured.
fn skill_effectiveness(view: &SkillView) -> (&'static str, &'static str, f32) {
    if view.samples == 0 {
        return ("unmeasured", "skill-eff-unmeasured", 0.0);
    }
    // Normalize the (unbounded) EWMA into a 0..1 bar via a soft squash.
    let frac = (view.ewma_score / (view.ewma_score.abs() + 2.0) + 1.0) / 2.0;
    if view.ewma_score < 0.0 {
        ("underperforming", "skill-eff-bad", frac.clamp(0.0, 1.0))
    } else if view.ewma_score > 0.0 {
        ("helping", "skill-eff-good", frac.clamp(0.0, 1.0))
    } else {
        ("neutral", "skill-eff-neutral", 0.5)
    }
}

#[component]
fn SkillsPanel() -> Element {
    let ws = use_context::<Sender>();
    let skills = use_context::<Signal<SkillsState>>();
    // Chapter Repertoire (approve-in-place) — reuse the shared persona-
    // proposal feed + ProposalCard, filtered to skill proposals.
    let agents = use_context::<Signal<AgentsState>>();

    // Load the inventory + the pending proposals each time the view opens.
    use_future(move || async move {
        ws.send(skills_query());
        ws.send(list_proposals_query());
    });

    let s = skills();
    // Pending skill proposals (Whetstone refinements + Praxis authored) —
    // approve / edit / reject right here, via the existing ProposalCard.
    let skill_proposals: Vec<PersonaProposalSummary> = agents()
        .proposals
        .into_iter()
        .filter(|p| p.category == "LearnedSkill" && p.status == "Pending")
        .collect();
    // Effectiveness-descending, with unmeasured (samples 0) grouped last.
    let mut rows = s.skills.clone();
    rows.sort_by(|a, b| {
        let am = a.samples == 0;
        let bm = b.samples == 0;
        am.cmp(&bm).then_with(|| {
            b.ewma_score
                .partial_cmp(&a.ewma_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    });

    rsx! {
        div { class: "skills",
            div { class: "panel-head",
                h3 { "Skills" }
                span { class: "label-tech", "{s.skills.len()}" }
            }
            if !skill_proposals.is_empty() {
                div { class: "skills-proposals",
                    div { class: "panel-head",
                        h3 { class: "label-tech", "Pending proposals" }
                        span { class: "chip amber", "{skill_proposals.len()}" }
                    }
                    for p in skill_proposals.iter() {
                        { rsx! { ProposalCard { key: "{p.id}", p: p.clone() } } }
                    }
                }
            }
            if !s.loaded {
                SkeletonCards { cards: 4 }
            } else if s.skills.is_empty() {
                div { class: "glass-card empty",
                    p { class: "label-tech",
                        "No skills yet. Teach one in chat (\"learn this skill…\"), or enable [skill_authoring] so the agent writes specialized skills from what it knows."
                    }
                }
            } else {
                div { class: "skills-grid",
                    for sv in rows.iter() {
                        { rsx! { SkillCard { view: sv.clone() } } }
                    }
                }
            }
        }
    }
}

#[component]
fn SkillCard(view: SkillView) -> Element {
    let ws = use_context::<Sender>();
    let mut confirming = use_signal(|| false);
    let sk = &view.skill;
    let (eff_label, eff_class, eff_frac) = skill_effectiveness(&view);
    let agent = sk.provenance.author == aivyx_ipc::persona::SkillAuthor::Agent;
    let forget_name = sk.name.clone();
    rsx! {
        div { class: "glass-card skill-card",
            div { class: "skill-card-head",
                h3 { "{sk.name}" }
                div { class: "skill-badges",
                    span {
                        class: if agent { "chip skill-prov-agent" } else { "chip skill-prov-op" },
                        if agent { "agent" } else { "operator" }
                    }
                    if let Some(d) = sk.domain.as_ref() {
                        span { class: "chip", "{d}" }
                    }
                    span { class: "label-tech", "v{sk.version}" }
                }
            }
            p { class: "skill-trigger", "{sk.trigger}" }
            if let Some(from) = sk.refined_from.as_ref() {
                p { class: "label-tech", "refined from {from}" }
            }
            div { class: "skill-eff",
                span { class: "chip {eff_class}", "{eff_label}" }
                div { class: "skill-eff-bar",
                    div { class: "skill-eff-fill {eff_class}", style: "width:{(eff_frac*100.0) as u32}%" }
                }
                span { class: "label-tech",
                    if view.samples == 0 { "no data" } else { "score {view.ewma_score:.1} · {view.samples} sample(s)" }
                }
                span { class: "label-tech", "· invoked {view.invocations}×" }
            }
            details { class: "skill-proc",
                summary { class: "label-tech", "procedure" }
                p { class: "mem-body", style: "white-space:pre-wrap", "{sk.procedure}" }
            }
            div { class: "skill-actions",
                if confirming() {
                    span { class: "label-tech", "Forget this skill?" }
                    button { class: "btn-danger",
                        onclick: move |_| { ws.send(forget_skill_query(&forget_name)); confirming.set(false); },
                        "Confirm"
                    }
                    button { class: "btn-ghost", onclick: move |_| confirming.set(false), "Cancel" }
                } else {
                    button { class: "btn-ghost", onclick: move |_| confirming.set(true), "Forget" }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// MCP screen — Chapter Lantern (LN.3). The web port of `aivyx mcp status`:
// each configured MCP server's last-start health (connected + tool count,
// or failed + reason + captured stderr), read from the daemon's snapshot
// over GetMcpStatus. Read-only — adding/removing servers stays in
// aivyx.toml (the screen shows, it does not edit).
// ---------------------------------------------------------------------------

fn mcp_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-mcp-status".to_string(),
        payload: QueryPayload::GetMcpStatus,
    }
}

#[component]
fn McpPanel() -> Element {
    let ws = use_context::<Sender>();
    let mcp = use_context::<Signal<McpState>>();

    // Load the snapshot each time the view opens (it only changes on a
    // daemon restart, so on-open + a manual refresh is enough — no poll).
    use_future(move || async move {
        ws.send(mcp_query());
    });

    let m = mcp();
    let connected = m.servers.iter().filter(|s| s.connected).count();
    rsx! {
        div { class: "mcp",
            div { class: "panel-head",
                h3 { "MCP Servers" }
                if m.loaded && !m.servers.is_empty() {
                    span { class: "label-tech", "{connected}/{m.servers.len()} connected" }
                }
                button {
                    class: "btn-ghost",
                    onclick: move |_| ws.send(mcp_query()),
                    "Refresh"
                }
            }
            if !m.loaded {
                SkeletonCards { cards: 3 }
            } else if m.servers.is_empty() {
                div { class: "glass-card empty",
                    p { class: "label-tech",
                        "No MCP servers reported at the last daemon start. Add one with a `[[mcp_server]]` block in aivyx.toml (see docs/MCP_RECIPES.md), then restart the daemon."
                    }
                }
            } else {
                div { class: "mcp-grid",
                    for sv in m.servers.iter() {
                        { rsx! { McpServerCard { key: "{sv.name}", view: sv.clone() } } }
                    }
                }
            }
        }
    }
}

#[component]
fn McpServerCard(view: McpServerStatusView) -> Element {
    let (pill_class, pill_label) = if view.connected {
        ("chip sage", "connected")
    } else {
        ("chip error", "failed")
    };
    rsx! {
        div { class: "glass-card mcp-card",
            div { class: "mcp-card-head",
                span { class: "mcp-name", "{view.name}" }
                span { class: "label-tech", "{view.transport}" }
                span { class: pill_class, "{pill_label}" }
            }
            if view.connected {
                p { class: "label-tech", "{view.tool_count} tool(s) registered" }
            } else {
                if let Some(err) = view.error.as_ref() {
                    p { class: "mcp-error", "{err}" }
                }
                if !view.stderr_tail.is_empty() {
                    details { class: "mcp-stderr",
                        summary { class: "label-tech", "captured stderr ({view.stderr_tail.len()} line(s))" }
                        pre {
                            for line in view.stderr_tail.iter() {
                                "{line}\n"
                            }
                        }
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Lattice view — Chapter Lattice (LT.5). The typed knowledge graph:
// entity nodes (sized by degree) + DIRECTED, labeled relation edges,
// force-laid-out in-WASM. Read-only; distinct from the MG co-occurrence
// view (undirected, topics).
// ---------------------------------------------------------------------------

#[component]
fn LatticePanel() -> Element {
    let ws = use_context::<Sender>();
    let lattice = use_context::<Signal<GraphKnowledgeState>>();

    use_future(move || async move {
        ws.send(knowledge_graph_query());
    });

    let g = lattice();
    rsx! {
        div { class: "lattice",
            div { class: "panel-head",
                h3 { "Knowledge Graph" }
                span { class: "label-tech", "{g.entities.len()} entities · {g.edges.len()} relations" }
            }
            if g.entities.is_empty() {
                div { class: "glass-card empty",
                    p { class: "label-tech",
                        "No graph yet. Set [memory] profile = \"smart\" (or [graph] enabled = true) and the agent extracts typed relations — (subject)-[predicate]->(object) — from its memory."
                    }
                }
            } else {
                LatticeGraph { entities: g.entities.clone(), edges: g.edges.clone() }
            }
        }
    }
}

/// The typed knowledge graph as a directed, labeled SVG. Reuses the
/// force-directed `compute_layout` (entities → nodes, triples → edges by
/// connectivity), then draws each edge as an arrowed line with its
/// predicate label at the midpoint.
#[component]
fn LatticeGraph(entities: Vec<GraphEntity>, edges: Vec<GraphTriple>) -> Element {
    // Map to the layout types (the FR layout cares only about
    // connectivity, not direction).
    let nodes: Vec<MemoryGraphNode> = entities
        .iter()
        .map(|e| MemoryGraphNode { topic: e.name.clone(), entry_count: e.degree })
        .collect();
    let layout_edges: Vec<PairScore> = edges
        .iter()
        .map(|t| PairScore {
            a: t.subject.clone(),
            b: t.object.clone(),
            score: t.mentions.max(1) as f32,
            samples: t.mentions,
        })
        .collect();
    let pos = compute_layout(&nodes, &layout_edges);
    let idx: std::collections::HashMap<&str, usize> =
        nodes.iter().enumerate().map(|(i, nd)| (nd.topic.as_str(), i)).collect();

    rsx! {
        div { class: "glass-card mem-graph-card",
            svg {
                class: "mem-graph",
                view_box: "0 0 {GRAPH_W} {GRAPH_H}",
                defs {
                    marker {
                        id: "lattice-arrow", view_box: "0 0 10 10",
                        ref_x: "9", ref_y: "5", marker_width: "7", marker_height: "7",
                        orient: "auto-start-reverse",
                        path { d: "M 0 0 L 10 5 L 0 10 z", class: "lattice-arrowhead" }
                    }
                }
                // Directed edges (under the nodes), shortened to the target
                // node's rim so the arrowhead is visible.
                for t in edges.iter() {
                    if let (Some(&i), Some(&j)) = (idx.get(t.subject.as_str()), idx.get(t.object.as_str())) {
                        {
                            let (x1, y1) = pos[i];
                            let (x2c, y2c) = pos[j];
                            let r = node_radius(entities[j].degree) + 4.0;
                            let dx = x2c - x1; let dy = y2c - y1;
                            let d = (dx * dx + dy * dy).sqrt().max(0.01);
                            let (x2, y2) = (x2c - dx / d * r, y2c - dy / d * r);
                            let (mx, my) = ((x1 + x2) / 2.0, (y1 + y2) / 2.0);
                            let label = t.predicate.clone();
                            rsx! {
                                line {
                                    x1: "{x1}", y1: "{y1}", x2: "{x2}", y2: "{y2}",
                                    class: "lattice-edge", marker_end: "url(#lattice-arrow)",
                                }
                                text { x: "{mx}", y: "{my}", class: "lattice-edge-label", text_anchor: "middle", "{label}" }
                            }
                        }
                    }
                }
                // Entity nodes.
                for (i, ent) in entities.iter().enumerate() {
                    {
                        let (cx, cy) = pos[i];
                        let r = node_radius(ent.degree);
                        rsx! {
                            g { class: "mem-node",
                                circle { cx: "{cx}", cy: "{cy}", r: "{r}" }
                                text { x: "{cx}", y: "{cy + r + 11.0}", text_anchor: "middle", "{ent.name}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn knowledge_graph_query() -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-knowledge-graph".to_string(),
        payload: QueryPayload::GetKnowledgeGraph { limit: 80 },
    }
}

// ── MG — the knowledge-graph view (force-directed layout, in-WASM). ──

/// The graph canvas size (SVG viewBox units).
const GRAPH_W: f64 = 760.0;
const GRAPH_H: f64 = 460.0;

/// A stable, deterministic hash of a topic name — seeds the layout so the graph
/// doesn't jitter between renders.
fn stable_hash(s: &str) -> u64 {
    // FNV-1a.
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Deterministic Fruchterman–Reingold layout: repulsion between all nodes,
/// attraction along (weighted) edges, cooled over a fixed iteration count.
/// Returns one `(x, y)` per node, index-aligned with `nodes`.
fn compute_layout(nodes: &[MemoryGraphNode], edges: &[PairScore]) -> Vec<(f64, f64)> {
    let n = nodes.len();
    if n == 0 {
        return Vec::new();
    }
    let k = (GRAPH_W * GRAPH_H / n as f64).sqrt() * 0.55; // ideal edge length
    // Seed positions on a spiral from a stable per-topic hash.
    let mut pos: Vec<(f64, f64)> = nodes
        .iter()
        .map(|node| {
            let h = stable_hash(&node.topic);
            let ang = (h % 628) as f64 / 100.0; // 0..2π
            let r = 40.0 + (h / 628 % 170) as f64;
            (GRAPH_W / 2.0 + r * ang.cos(), GRAPH_H / 2.0 + r * ang.sin())
        })
        .collect();
    let idx: std::collections::HashMap<&str, usize> =
        nodes.iter().enumerate().map(|(i, nd)| (nd.topic.as_str(), i)).collect();

    let mut temp = GRAPH_W / 8.0;
    for _ in 0..220 {
        let mut disp = vec![(0.0_f64, 0.0_f64); n];
        // Repulsion (all pairs).
        for i in 0..n {
            for j in (i + 1)..n {
                let dx = pos[i].0 - pos[j].0;
                let dy = pos[i].1 - pos[j].1;
                let d = (dx * dx + dy * dy).sqrt().max(0.01);
                let f = k * k / d;
                let (ux, uy) = (dx / d * f, dy / d * f);
                disp[i].0 += ux;
                disp[i].1 += uy;
                disp[j].0 -= ux;
                disp[j].1 -= uy;
            }
        }
        // Attraction along edges (stronger for higher-affinity pairs).
        for e in edges {
            if let (Some(&i), Some(&j)) = (idx.get(e.a.as_str()), idx.get(e.b.as_str())) {
                let w = (e.score.max(0.1) as f64).min(4.0);
                let dx = pos[i].0 - pos[j].0;
                let dy = pos[i].1 - pos[j].1;
                let d = (dx * dx + dy * dy).sqrt().max(0.01);
                let f = d * d / k * (0.5 + 0.25 * w);
                let (ux, uy) = (dx / d * f, dy / d * f);
                disp[i].0 -= ux;
                disp[i].1 -= uy;
                disp[j].0 += ux;
                disp[j].1 += uy;
            }
        }
        // Apply, capped by temperature, clamped to the canvas.
        for i in 0..n {
            let dl = (disp[i].0 * disp[i].0 + disp[i].1 * disp[i].1).sqrt().max(0.01);
            let mv = dl.min(temp);
            pos[i].0 = (pos[i].0 + disp[i].0 / dl * mv).clamp(28.0, GRAPH_W - 28.0);
            pos[i].1 = (pos[i].1 + disp[i].1 / dl * mv).clamp(28.0, GRAPH_H - 28.0);
        }
        temp *= 0.965;
    }
    pos
}

/// Node radius from entry count (sqrt-scaled, clamped).
fn node_radius(entry_count: u32) -> f64 {
    (6.0 + (entry_count as f64).sqrt() * 3.0).min(24.0)
}

/// The memory knowledge graph — a force-directed SVG of topic nodes (sized by
/// entry count) + weighted co-occurrence edges. Clicking a node selects that
/// topic. Read-only. The layout is deterministic (no animation loop).
#[component]
fn MemoryGraph(
    nodes: Vec<MemoryGraphNode>,
    edges: Vec<PairScore>,
    on_select: EventHandler<String>,
) -> Element {
    let pos = compute_layout(&nodes, &edges);
    let idx: std::collections::HashMap<&str, usize> =
        nodes.iter().enumerate().map(|(i, nd)| (nd.topic.as_str(), i)).collect();
    let max_score = edges.iter().map(|e| e.score).fold(0.1_f32, f32::max);

    rsx! {
        div { class: "glass-card mem-graph-card",
            if edges.is_empty() {
                p { class: "label-tech sub", "No co-occurrence links yet — topics appear as a cloud until the agent recalls them together." }
            }
            svg {
                class: "mem-graph",
                view_box: "0 0 {GRAPH_W} {GRAPH_H}",
                // Edges first (under the nodes).
                for e in edges.iter() {
                    if let (Some(&i), Some(&j)) = (idx.get(e.a.as_str()), idx.get(e.b.as_str())) {
                        {
                            let (x1, y1) = pos[i];
                            let (x2, y2) = pos[j];
                            let frac = (e.score / max_score).clamp(0.1, 1.0) as f64;
                            let w = 0.6 + frac * 3.4;
                            let op = 0.12 + frac * 0.5;
                            rsx! {
                                line {
                                    x1: "{x1}", y1: "{y1}", x2: "{x2}", y2: "{y2}",
                                    class: "mem-edge",
                                    stroke_width: "{w}", opacity: "{op}",
                                }
                            }
                        }
                    }
                }
                // Nodes.
                for (i, node) in nodes.iter().enumerate() {
                    {
                        let (cx, cy) = pos[i];
                        let r = node_radius(node.entry_count);
                        let topic = node.topic.clone();
                        rsx! {
                            g { class: "mem-node",
                                onclick: move |_| on_select.call(topic.clone()),
                                circle { cx: "{cx}", cy: "{cy}", r: "{r}" }
                                text { x: "{cx}", y: "{cy + r + 11.0}", text_anchor: "middle", "{node.topic}" }
                            }
                        }
                    }
                }
            }
        }
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

/// Relative time to a FUTURE unix-ms timestamp ("in 6h", "in 2d"); past/now →
/// "due". Used for a routine's next scheduled fire.
fn until_time(ms: u64) -> String {
    let now = js_sys::Date::now() as u64;
    if ms == 0 || ms <= now {
        return "due".to_string();
    }
    let secs = (ms - now) / 1000;
    if secs < 60 {
        format!("in {secs}s")
    } else if secs < 3600 {
        format!("in {}m", secs / 60)
    } else if secs < 86_400 {
        format!("in {}h", secs / 3600)
    } else {
        format!("in {}d", secs / 86_400)
    }
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
    // Chapter Reins — the autonomy dial (level picker + confirm-on-autonomy).
    let mut auto_level = use_signal(String::new);
    let mut auto_confirm_open = use_signal(|| false);
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
                auto_level.set(s.autonomy_level.clone());
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
    let cycle_on = snap.cycle_detection;
    // Chapter Reins — the autonomy-granting levels confirm first (server-side too).
    let auto_grants = auto_level() == "autonomous" || auto_level() == "unleashed";

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

            // ── Autonomy (editable, confirm-first on autonomy-granting levels) ──
            div { class: "glass-card settings-section",
                div { class: "panel-head",
                    h3 { "Autonomy" }
                    span { class: "chip", "{snap.autonomy_level}" }
                }
                p { class: "label-tech",
                    "How autonomous the agent is. One dial that composes the safety \
                     knobs; supervised and above arm the autonomous loop. Granting \
                     unattended autonomy is confirmed first."
                }
                div { class: "field-row",
                    label { class: "label-tech", "Level" }
                    select {
                        class: "input",
                        value: "{auto_level}",
                        onchange: move |e| auto_level.set(e.value()),
                        option { value: "manual", "manual — confirm everything" }
                        option { value: "assisted", "assisted — reversible free, irreversible confirmed (default)" }
                        option { value: "supervised", "supervised — armed loop, a human nearby" }
                        option { value: "autonomous", "autonomous — pursues goals unattended (capped)" }
                        option { value: "unleashed", "unleashed — isolated host, eyes-open" }
                    }
                }
                p { class: "label-tech sub",
                    "Per-domain overrides and the auto-approve allowlist are edited in "
                    code { "aivyx.toml" }
                    " for now. Takes effect on the next restart."
                }
                div { class: "actions",
                    button {
                        class: "btn btn-primary",
                        onclick: move |_| {
                            if auto_grants {
                                auto_confirm_open.set(true);
                            } else {
                                ws.send(set_autonomy_query(auto_level(), false));
                            }
                        },
                        "Apply autonomy level"
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

            // ── Agent loop safety (editable) ──
            div { class: "glass-card settings-section",
                div { class: "panel-head", h3 { "Agent" } }
                p { class: "label-tech",
                    "Loop protection. Stops the agent if it falls into a repeating cycle \
                     of the same actions (e.g. A→B→A→B) instead of making progress."
                }
                div { class: "field-row",
                    label { class: "label-tech", "Cycle breaker" }
                    div { class: "toggle-line",
                        span { class: "chip", {if cycle_on { "on" } else { "off" }} }
                        button {
                            class: "btn btn-glass",
                            onclick: move |_| ws.send(set_cycle_detection_query(!cycle_on)),
                            {if cycle_on { "Disable" } else { "Enable" }}
                        }
                    }
                }
                p { class: "label-tech sub",
                    "Off by default for the interactive agent; autonomous team agents always \
                     have it on. Takes effect on the next restart."
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

        // Confirm-first modal for the autonomy-granting levels (Chapter Reins).
        if auto_confirm_open() {
            div { class: "modal-scrim",
                div { class: "glass-card modal",
                    h3 { "Set autonomy to '{auto_level()}'?" }
                    p { "{autonomy_confirm_blurb(&auto_level())}" }
                    div { class: "actions",
                        button { class: "btn btn-glass", onclick: move |_| auto_confirm_open.set(false), "Cancel" }
                        button {
                            class: "btn btn-primary",
                            onclick: move |_| {
                                ws.send(set_autonomy_query(auto_level(), true));
                                auto_confirm_open.set(false);
                            },
                            "Set autonomy"
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

fn set_cycle_detection_query(enabled: bool) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-settings-cycle".to_string(),
        payload: QueryPayload::SetCycleDetection { enabled },
    }
}

fn set_autonomy_query(level: String, confirm: bool) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-settings-autonomy".to_string(),
        payload: QueryPayload::SetAutonomyLevel { level, confirm },
    }
}

/// One-line risk blurb for the autonomy confirm modal (Chapter Reins).
fn autonomy_confirm_blurb(level: &str) -> &'static str {
    match level {
        "autonomous" => {
            "The agent will pursue goals unattended within its caps. Irreversible \
             actions are still refused without a human."
        }
        "unleashed" => {
            "Runs an armed, self-directing agent with confirm-first OFF. Intended \
             only for a dedicated, isolated host where the agent's blast radius is \
             the host."
        }
        _ => "This grants the agent unattended autonomy.",
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
    let mut tts_model_dir = use_signal(String::new);
    let mut tts_voice_name = use_signal(String::new);
    let mut tts_speed = use_signal(String::new);
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
                tts_model_dir.set(s.tts_model_dir.clone().unwrap_or_default());
                tts_voice_name.set(s.tts_voice_name.clone().unwrap_or_default());
                tts_speed.set(s.tts_speed.map(|n| n.to_string()).unwrap_or_default());
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
                    ReadinessRow { label: "Kokoro model (.onnx)", status: snap.tts_model_status.clone() }
                    ReadinessRow { label: "Kokoro voices (.bin)", status: snap.tts_voices_status.clone() }
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
                        option { value: "", "default (kokoro)" }
                        option { value: "kokoro", "kokoro" }
                    }
                }
                div { class: "field-row",
                    label { class: "label-tech", "Kokoro model dir" }
                    input { class: "input", placeholder: "/path/to/kokoro (holds the .onnx + voices-*.bin)",
                        value: "{tts_model_dir}", oninput: move |e| tts_model_dir.set(e.value()) }
                }
                div { class: "field-row",
                    label { class: "label-tech", "Kokoro voice" }
                    input { class: "input", placeholder: "af_heart",
                        value: "{tts_voice_name}", oninput: move |e| tts_voice_name.set(e.value()) }
                }
                div { class: "field-row",
                    label { class: "label-tech", "Speaking rate" }
                    input { class: "input", r#type: "number", placeholder: "1.0",
                        value: "{tts_speed}", oninput: move |e| tts_speed.set(e.value()) }
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
                p { class: "label-tech sub", "The audio loop (mic → Whisper → agent → Kokoro → speakers) runs on the host, not in the browser." }
            }

            div { class: "actions sticky-save",
                button {
                    class: "btn btn-primary",
                    onclick: move |_| ws.send(set_voice_query(
                        opt_str(&asr_engine()), opt_str(&tts_engine()), opt_str(&asr_model_path()),
                        opt_str(&asr_language()), parse_opt_u32(&asr_beam_size()), opt_str(&tts_model_dir()),
                        opt_str(&tts_voice_name()), parse_opt_f32(&tts_speed()),
                        opt_str(&input_device()), opt_str(&output_device()),
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
    tts_model_dir: Option<String>,
    tts_voice_name: Option<String>,
    tts_speed: Option<f32>,
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
            tts_model_dir,
            tts_voice_name,
            tts_speed,
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

/// Parse a numeric form field to `Option<f32>` (blank / unparseable ⇒ `None`).
fn parse_opt_f32(s: &str) -> Option<f32> {
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

/// Chapter Genesis (GE.3) — the guided agent-creation flow. Three sequenced
/// steps over existing IPC: (1) Profile — the declared identity, optionally
/// LLM-drafted via `DraftProfile`, persisted via `SetProfile`; (2) Persona seed
/// — the learned voice, via the X.3 `SeedOnboardingCard`; (3) Access — how far
/// the agent reaches, via `SetAccessLevel`. The operator authors every field;
/// the LLM only drafts. Targets a running daemon (the cold-start path is
/// `aivyx init`); Profile + access are load-time so a restart applies them.
#[component]
fn OnboardingPanel(view: Signal<View>) -> Element {
    let step = use_signal(|| 0u8);
    let ws = use_context::<Sender>();
    let settings = use_context::<Signal<SettingsState>>();

    // GE.4 — provider/model is CLI/installer-set (it must precede daemon boot),
    // so the flow only *shows* it read-only. Fetch the snapshot on mount.
    use_effect(move || {
        ws.send(get_settings_query());
    });
    let model_line = settings()
        .snapshot
        .map(|s| format!("Connected to {} · {}", s.provider, s.model));

    rsx! {
        div { class: "view-stack",
            div { class: "glass-card settings-section",
                div { class: "panel-head",
                    h3 { "Create your agent" }
                    span { class: "chip sage", "step {step() + 1} of 4" }
                }
                p { class: "muted",
                    "Shape your assistant's identity, voice, and reach. You're the author of "
                    "record — the model only drafts. Profile and access apply after a daemon restart."
                }
                if let Some(line) = model_line {
                    p { class: "muted", style: "font-size:12px;",
                        "{line} — set the provider/model with `aivyx init` or in your config."
                    }
                }
                div { class: "step-rail",
                    StepDot { n: 1, label: "Profile", active: step() == 0, done: step() > 0 }
                    StepDot { n: 2, label: "Persona", active: step() == 1, done: step() > 1 }
                    StepDot { n: 3, label: "Team", active: step() == 2, done: step() > 2 }
                    StepDot { n: 4, label: "Access", active: step() == 3, done: false }
                }
            }
            match step() {
                0 => rsx! { OnboardingProfileStep { step } },
                1 => rsx! {
                    div { class: "view-stack",
                        SeedOnboardingCard {}
                        div { class: "wizard-nav",
                            button { class: "btn ghost", onclick: move |_| { let mut s = step; s.set(0); }, "Back" }
                            button { class: "btn", onclick: move |_| { let mut s = step; s.set(2); }, "Continue →" }
                        }
                    }
                },
                2 => rsx! { OnboardingTeamStep { step, view } },
                _ => rsx! { OnboardingAccessStep { step, view } },
            }
        }
    }
}

/// One dot in the onboarding step rail.
#[component]
fn StepDot(n: u8, label: &'static str, active: bool, done: bool) -> Element {
    let cls = if active { "step-dot active" } else if done { "step-dot done" } else { "step-dot" };
    rsx! {
        div { class: "{cls}",
            span { class: "step-num", if done { "✓" } else { "{n}" } }
            span { class: "step-label", "{label}" }
        }
    }
}

/// GE.3 step 1 — the declared Profile. Four onboarding answers feed an optional
/// LLM draft (`DraftProfile`); the six Profile fields below are then editable
/// and saved via `SetProfile`.
#[component]
fn OnboardingProfileStep(step: Signal<u8>) -> Element {
    let ws = use_context::<Sender>();
    let agents = use_context::<Signal<AgentsState>>();

    // The four relationship answers (LLM draft inputs).
    let mut intent = use_signal(String::new);
    let mut role = use_signal(String::new);
    let mut tone = use_signal(String::new);
    let mut never_do = use_signal(String::new);

    // The six declared Profile fields (editable; the draft fills them).
    let mut assistant_name = use_signal(String::new);
    let mut operator_profile = use_signal(String::new);
    let mut communication_style = use_signal(String::new);
    let mut use_cases = use_signal(String::new);
    let mut prefs = use_signal(String::new);
    let mut constraints = use_signal(String::new);

    let mut drafting = use_signal(|| false);
    let mut last_resp = use_signal(|| 0u64);

    // Fill the six fields when a Profile draft arrives (success or failure).
    use_effect(move || {
        let a = agents();
        if a.profile_draft_resp != last_resp() {
            last_resp.set(a.profile_draft_resp);
            drafting.set(false);
            if let Some(d) = a.profile_draft.as_ref() {
                assistant_name.set(d.assistant_name.clone().unwrap_or_default());
                operator_profile.set(d.operator_profile.clone().unwrap_or_default());
                communication_style.set(d.communication_style.clone().unwrap_or_default());
                use_cases.set(d.primary_use_cases.join(", "));
                prefs.set(d.behavioral_preferences.join(", "));
                constraints.set(d.behavioral_constraints.join(", "));
            }
        }
    });

    let st = agents();
    let saved = st.restart_required;

    rsx! {
        div { class: "glass-card settings-section",
            div { class: "panel-head", h3 { "1 · Profile — who your assistant is" } }
            p { class: "muted", "Answer in your own words, then let the model draft a starting Profile — or fill the six fields yourself." }

            label { class: "field-label", "What do you want this assistant to be for you?" }
            textarea { class: "input", rows: "2", value: "{intent}", oninput: move |e| intent.set(e.value()) }
            label { class: "field-label", "What role should it play?" }
            input { class: "input", value: "{role}", placeholder: "collaborator / coach / assistant …", oninput: move |e| role.set(e.value()) }
            label { class: "field-label", "How should it talk?" }
            input { class: "input", value: "{tone}", placeholder: "warm but concise …", oninput: move |e| tone.set(e.value()) }
            label { class: "field-label", "What must it never do?" }
            input { class: "input", value: "{never_do}", placeholder: "never flatter; always confirm destructive actions …", oninput: move |e| never_do.set(e.value()) }

            div { class: "wizard-nav",
                button {
                    class: "btn ghost",
                    disabled: drafting(),
                    onclick: move |_| {
                        drafting.set(true);
                        ws.send(draft_profile_query(intent(), role(), tone(), never_do()));
                    },
                    if drafting() { "Drafting…" } else { "✦ Draft with AI" }
                }
            }

            hr { class: "divider" }
            div { class: "panel-head", h4 { "Your Profile" } }
            label { class: "field-label", "Assistant name" }
            input { class: "input", value: "{assistant_name}", oninput: move |e| assistant_name.set(e.value()) }
            label { class: "field-label", "About you (operator profile)" }
            textarea { class: "input", rows: "2", value: "{operator_profile}", oninput: move |e| operator_profile.set(e.value()) }
            label { class: "field-label", "Communication style" }
            input { class: "input", value: "{communication_style}", oninput: move |e| communication_style.set(e.value()) }
            label { class: "field-label", "Primary use cases (comma-separated)" }
            input { class: "input", value: "{use_cases}", oninput: move |e| use_cases.set(e.value()) }
            label { class: "field-label", "Behavioral preferences (comma-separated)" }
            input { class: "input", value: "{prefs}", oninput: move |e| prefs.set(e.value()) }
            label { class: "field-label", "Behavioral constraints (comma-separated)" }
            input { class: "input", value: "{constraints}", oninput: move |e| constraints.set(e.value()) }

            if let Some((ok, msg)) = st.notice.clone() {
                div { class: if ok { "notice ok" } else { "notice err" }, "{msg}" }
            }
            if saved {
                div { class: "notice ok", "Profile saved — it shapes every turn after the next daemon restart." }
            }

            div { class: "wizard-nav",
                button {
                    class: "btn",
                    onclick: move |_| {
                        ws.send(set_profile_query(
                            opt_str(&assistant_name()),
                            opt_str(&operator_profile()),
                            opt_str(&communication_style()),
                            csv_opt(&use_cases()),
                            csv_opt(&prefs()),
                            csv_opt(&constraints()),
                        ));
                        let mut s = step; s.set(1);
                    },
                    "Save & continue →"
                }
                button { class: "btn ghost", onclick: move |_| { let mut s = step; s.set(1); }, "Skip for now" }
            }
        }
    }
}

/// Chapter Roster (RO.4) — onboarding "Team" step. Shows the active roster (the
/// default Nonagon on a fresh install, fetched via `GetTeamRoster`) and routes
/// to the full Teams editor (RO.3). Keeping the default is a no-op; pack presets
/// are an `aivyx team init` CLI affordance (the team engine isn't wasm, so the
/// browser can't construct a pack — it edits the loaded one). Read-only here.
#[component]
fn OnboardingTeamStep(step: Signal<u8>, view: Signal<View>) -> Element {
    let ws = use_context::<Sender>();
    let teams = use_context::<Signal<TeamsState>>();
    use_effect(move || {
        ws.send(get_team_roster_query());
    });
    let roster = teams().roster;

    rsx! {
        div { class: "glass-card settings-section",
            div { class: "panel-head", h3 { "3 · Team — who works for you" } }
            p { class: "muted",
                "Your assistant leads a team of specialists. Keep the default Nonagon, or shape "
                "the lead, specialists, and their reach in the Teams editor (you can always change it later)."
            }
            {match roster {
                Some(t) => rsx! {
                    div { class: "kv-grid",
                        div { span { class: "label-tech", "Team" } div { "{t.name}" } }
                        div { span { class: "label-tech", "Lead" } div { "{t.lead}" } }
                        div { span { class: "label-tech", "Members" } div { "{t.members.len()}" } }
                    }
                    button { class: "btn btn-glass", onclick: move |_| view.set(View::Teams),
                        "Customize team →" }
                },
                None => rsx! { p { class: "label-tech", "Loading team…" } },
            }}
            div { class: "wizard-nav",
                button { class: "btn ghost", onclick: move |_| { let mut s = step; s.set(1); }, "Back" }
                button { class: "btn", onclick: move |_| { let mut s = step; s.set(3); }, "Continue →" }
            }
        }
    }
}

/// GE.3 step 3 — access level, via the existing `SetAccessLevel` (confirm-first
/// on any expansion beyond the sandbox).
#[component]
fn OnboardingAccessStep(step: Signal<u8>, view: Signal<View>) -> Element {
    let ws = use_context::<Sender>();
    let agents = use_context::<Signal<AgentsState>>();
    let mut level = use_signal(|| "sandbox".to_string());
    let st = agents();

    rsx! {
        div { class: "glass-card settings-section",
            div { class: "panel-head", h3 { "4 · Access — how far it reaches" } }
            p { class: "muted", "Start narrow; you can widen later in Settings. Expanding beyond the sandbox is confirmed first." }
            select {
                class: "input",
                value: "{level}",
                onchange: move |e| level.set(e.value()),
                option { value: "sandbox", "sandbox — ~/aivyx-sandbox" }
                option { value: "home", "home — your home directory" }
                option { value: "full", "full — the whole machine" }
            }
            if let Some((ok, msg)) = st.notice.clone() {
                div { class: if ok { "notice ok" } else { "notice err" }, "{msg}" }
            }
            div { class: "wizard-nav",
                button { class: "btn ghost", onclick: move |_| { let mut s = step; s.set(2); }, "Back" }
                button {
                    class: "btn",
                    onclick: move |_| {
                        // Confirm-first: the daemon gates any expansion beyond sandbox.
                        ws.send(set_access_query(level(), None, true));
                        view.set(View::Command);
                    },
                    "Finish — go to Command Center"
                }
            }
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

/// GE.3 — a comma-separated list field → `Some(cleaned)` of trimmed non-empty
/// entries, or `None` when the field is blank (clear the key).
fn csv_opt(s: &str) -> Option<Vec<String>> {
    let cleaned: Vec<String> = s
        .split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
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

// ── GE.3 — Genesis onboarding: LLM-drafted Profile (step 1). ──

fn draft_profile_query(
    intent: String,
    role: String,
    tone: String,
    never_do: String,
) -> FrontendMessage {
    FrontendMessage::DraftProfile {
        id: "mc-onboard-profile-draft".to_string(),
        intent,
        role,
        tone,
        never_do,
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
        TeamMissionPhase::Halted => "halted",
    }
}

fn phase_class(p: TeamMissionPhase) -> &'static str {
    match p {
        TeamMissionPhase::AwaitingApproval => "amber",
        TeamMissionPhase::Done => "sage",
        TeamMissionPhase::Rejected => "error",
        TeamMissionPhase::Halted => "error",
        _ => "",
    }
}

// ---------------------------------------------------------------------------
// Teams — the Nonagon roster (Chapters Y read + Roster RO.3 edit). Renders the
// daemon's active TeamConfig as an editable form: team name/description, the
// lead pick, and per-member role / trust / scopes / tools / soul, with
// add/remove specialist (≤9) and Save → SetTeamRoster (server-validated; the
// team is adopted on the next daemon restart). NT-02 is unchanged — a member
// scope the lead lacks is flagged inert, never granted.
// ---------------------------------------------------------------------------

#[component]
fn TeamsPanel() -> Element {
    let ws = use_context::<Sender>();
    let connected = use_context::<Signal<bool>>();
    let teams = use_context::<Signal<TeamsState>>();
    let missions = use_context::<Signal<Vec<TeamMissionView>>>();

    // The edit draft, seeded once from the loaded roster.
    let mut draft = use_signal(|| None::<TeamConfig>);
    use_future(move || async move {
        ws.send(get_team_roster_query());
    });
    use_effect(move || {
        if let Some(r) = teams().roster {
            if draft.peek().is_none() {
                draft.set(Some(r));
            }
        }
    });

    let st = teams();
    let Some(team) = draft() else {
        return rsx! {
            div { class: "teams",
                div { class: "panel-head", h3 { "Team" } }
                SkeletonList { rows: 4 }
            }
        };
    };

    // The lead's declared scopes — for the NT-02 "inert" hint on specialists.
    let lead_scopes: std::collections::HashSet<String> = team
        .members
        .iter()
        .find(|m| m.name == team.lead)
        .map(|m| m.capability_scopes.iter().cloned().collect())
        .unwrap_or_default();
    let specialist_count = team.members.iter().filter(|m| m.name != team.lead).count();
    let active = missions()
        .iter()
        .filter(|m| !matches!(m.phase, TeamMissionPhase::Done | TeamMissionPhase::Rejected))
        .count();
    let member_names: Vec<String> = team.members.iter().map(|m| m.name.clone()).collect();
    let dirty = st.roster.as_ref() != Some(&team);
    let can_add = specialist_count < 9;

    rsx! {
        div { class: "teams",
            if st.restart_required {
                div { class: "glass-card restart-banner",
                    strong { "Saved — restart the daemon to run the new team." }
                    p { class: "label-tech",
                        "The team is assembled at startup. Run  "
                        code { "aivyx daemon stop && aivyx daemon run" }
                    }
                }
            }
            if let Some((ok, msg)) = st.notice.clone() {
                div { class: if ok { "notice ok" } else { "notice err" }, "{msg}" }
            }

            // Team identity.
            div { class: "glass-card settings-section",
                div { class: "panel-head",
                    h3 { "Team" }
                    span { class: "chip", "{team.members.len()} members" }
                    span { class: "chip", "{active} active" }
                }
                div { class: "field-row",
                    label { class: "label-tech", "Name" }
                    input { class: "input", value: "{team.name}",
                        oninput: move |e| { if let Some(t) = draft.write().as_mut() { t.name = e.value(); } } }
                }
                div { class: "field-row",
                    label { class: "label-tech", "Description" }
                    input { class: "input", value: "{team.description}",
                        oninput: move |e| { if let Some(t) = draft.write().as_mut() { t.description = e.value(); } } }
                }
                div { class: "field-row",
                    label { class: "label-tech", "Lead" }
                    select { class: "input", value: "{team.lead}",
                        onchange: move |e| { if let Some(t) = draft.write().as_mut() { t.lead = e.value(); } },
                        for n in member_names.clone() {
                            option { value: "{n}", "{n}" }
                        }
                    }
                }
            }

            // Per-member editors.
            div { class: "roster-grid",
                {team.members.clone().into_iter().enumerate().map(|(i, m)| {
                    let is_lead = m.name == team.lead;
                    let scopes_text = m.capability_scopes.join("\n");
                    let tools_text = m.tool_allowlist.join("\n");
                    let widened: Vec<String> = if is_lead {
                        Vec::new()
                    } else {
                        m.capability_scopes.iter().filter(|s| !lead_scopes.contains(*s)).cloned().collect()
                    };
                    let widened_text = widened.join(", ");
                    rsx! {
                        div { key: "{i}",
                            class: if is_lead { "glass-card member-card lead" } else { "glass-card member-card" },
                            div { class: "panel-head",
                                if is_lead { span { class: "chip amber", "lead" } }
                                span { class: "chip {trust_class(m.trust_ceiling)}", "{trust_label(m.trust_ceiling)}" }
                                if !is_lead {
                                    button { class: "btn btn-ghost-danger btn-xs",
                                        onclick: move |_| { if let Some(t) = draft.write().as_mut() { t.members.remove(i); } },
                                        "Remove" }
                                }
                            }
                            div { class: "field-row",
                                label { class: "label-tech", "Name" }
                                input { class: "input", value: "{m.name}",
                                    oninput: move |e| { if let Some(t) = draft.write().as_mut() { t.members[i].name = e.value(); } } }
                            }
                            div { class: "field-row",
                                label { class: "label-tech", "Role" }
                                input { class: "input", value: "{m.role}",
                                    oninput: move |e| { if let Some(t) = draft.write().as_mut() { t.members[i].role = e.value(); } } }
                            }
                            div { class: "field-row",
                                label { class: "label-tech", "Trust" }
                                select { class: "input", value: "{trust_label(m.trust_ceiling)}",
                                    onchange: move |e| {
                                        if let Some(tt) = trust_from_label(&e.value()) {
                                            if let Some(t) = draft.write().as_mut() { t.members[i].trust_ceiling = tt; }
                                        }
                                    },
                                    for tt in [TrustTier::Untrusted, TrustTier::SemiTrusted, TrustTier::Trusted, TrustTier::Kernel] {
                                        option { value: "{trust_label(tt)}", "{trust_label(tt)}" }
                                    }
                                }
                            }
                            div { class: "field-row",
                                label { class: "label-tech", "Scopes" }
                                textarea { class: "input", rows: "2", placeholder: "fs.read\nmemory.write",
                                    value: "{scopes_text}",
                                    oninput: move |e| { if let Some(t) = draft.write().as_mut() { t.members[i].capability_scopes = parse_token_list(&e.value()); } } }
                            }
                            div { class: "field-row",
                                label { class: "label-tech", "Tools" }
                                textarea { class: "input", rows: "2", placeholder: "fs.read\nteam.message",
                                    value: "{tools_text}",
                                    oninput: move |e| { if let Some(t) = draft.write().as_mut() { t.members[i].tool_allowlist = parse_token_list(&e.value()); } } }
                            }
                            div { class: "field-row",
                                label { class: "label-tech", "Soul" }
                                textarea { class: "input", rows: "3", value: "{m.soul}",
                                    oninput: move |e| { if let Some(t) = draft.write().as_mut() { t.members[i].soul = e.value(); } } }
                            }
                            if !widened_text.is_empty() {
                                p { class: "label-tech sub",
                                    "Lead lacks {widened_text} — inert until the lead holds them (attenuated at spawn)." }
                            }
                        }
                    }
                })}
            }

            // Add specialist + Save / Discard.
            div { class: "actions",
                button { class: "btn btn-glass", disabled: !can_add,
                    onclick: move |_| {
                        if let Some(t) = draft.write().as_mut() {
                            let n = t.members.len();
                            t.members.push(TeamMember {
                                name: format!("specialist-{n}"),
                                role: "Specialist".to_string(),
                                soul: String::new(),
                                tool_allowlist: vec!["team.message".to_string()],
                                capability_scopes: Vec::new(),
                                trust_ceiling: TrustTier::SemiTrusted,
                                // Chapter Ensemble — per-role model/endpoint
                                // default to the team's shared backend.
                                model: None,
                                base_url: None,
                            });
                        }
                    },
                    {if can_add { "Add specialist" } else { "Max 9 specialists" }}
                }
                button { class: "btn btn-primary", disabled: !dirty || !connected(),
                    onclick: move |_| { if let Some(t) = draft() { ws.send(set_team_roster_query(&t)); } },
                    {if connected() { "Save team" } else { "reconnecting…" }} }
                button { class: "btn btn-glass", disabled: !dirty,
                    onclick: move |_| { draft.set(teams().roster); },
                    "Discard changes" }
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

/// Chapter Roster (RO.3) — persist the edited roster. The id is prefixed
/// `mc-teams` so a server-side validation `QueryError` routes to the Teams
/// banner.
fn set_team_roster_query(roster: &TeamConfig) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-teams-set".to_string(),
        payload: QueryPayload::SetTeamRoster { roster: roster.clone() },
    }
}

/// Parse a scopes/tools textarea (newline-, comma-, or space-separated) into a
/// trimmed, non-empty token list.
fn parse_token_list(raw: &str) -> Vec<String> {
    raw.split(|c: char| c == '\n' || c == ',' || c.is_whitespace())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

/// Inverse of [`trust_label`] — parse a trust tier from the editor's `<select>`.
fn trust_from_label(s: &str) -> Option<TrustTier> {
    match s {
        "untrusted" => Some(TrustTier::Untrusted),
        "semi-trusted" => Some(TrustTier::SemiTrusted),
        "trusted" => Some(TrustTier::Trusted),
        "kernel" => Some(TrustTier::Kernel),
        _ => None,
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

    // Toolbar create state: Some(true)=new file, Some(false)=new folder.
    let mut new_kind = use_signal(|| None::<bool>);
    let mut new_name = use_signal(String::new);
    // Per-entry rename (the entry name being renamed) + the pending value.
    let mut rename_of = use_signal(|| None::<String>);
    let mut rename_to = use_signal(String::new);
    // The entry path pending a delete confirmation (→ modal).
    let mut delete_of = use_signal(|| None::<String>);

    // First entry → default to the workspace root.
    use_future(move || async move {
        if documents.read().root.is_empty() {
            documents.write().root = "workspace".to_string();
            ws.send(list_dir_query("workspace", ""));
        }
    });

    let d = documents();
    let root = if d.root.is_empty() { "workspace".to_string() } else { d.root.clone() };
    let cur = d.path.clone();
    let viewing = d.file.is_some();

    rsx! {
        div { class: "documents",
            // Toolbar: root switcher + New file / New folder.
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
                if !viewing {
                    div { style: "flex:1" }
                    button { class: "btn btn-glass btn-xs", onclick: move |_| { new_name.set(String::new()); new_kind.set(Some(true)); }, "New file" }
                    button { class: "btn btn-glass btn-xs", onclick: move |_| { new_name.set(String::new()); new_kind.set(Some(false)); }, "New folder" }
                }
            }

            // New file/folder name input (inline).
            if let Some(is_file) = new_kind() {
                div { class: "add-row",
                    input { class: "input", placeholder: if is_file { "new-file.md" } else { "new-folder" },
                        value: "{new_name}", oninput: move |e| new_name.set(e.value()) }
                    {
                        let (r, base) = (root.clone(), cur.clone());
                        rsx! {
                            button {
                                class: "btn btn-primary btn-xs",
                                onclick: move |_| {
                                    let nm = new_name();
                                    if !nm.trim().is_empty() {
                                        let target = join_doc_path(&base, nm.trim());
                                        if is_file {
                                            ws.send(write_file_query(&r, &target, String::new(), false));
                                        } else {
                                            ws.send(make_dir_query(&r, &target));
                                        }
                                        ws.send(list_dir_refresh_query(&r, &base));
                                    }
                                    new_kind.set(None);
                                },
                                "Create"
                            }
                        }
                    }
                    button { class: "btn btn-glass btn-xs", onclick: move |_| new_kind.set(None), "Cancel" }
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

            if let Some((ok, msg)) = d.notice.clone() {
                div { class: if ok { "notice ok" } else { "notice err" }, "{msg}" }
            }

            // File viewer/editor (when one is open) else the directory listing.
            if let Some(file) = d.file.clone() {
                FileViewer { key: "{file.path}", file: file.clone(), root: root.clone() }
            } else if !d.loaded {
                SkeletonList { rows: 5 }
            } else {
                div { class: "glass-card doc-listing",
                    if d.entries.is_empty() {
                        p { class: "label-tech sub", "Empty directory." }
                    } else {
                        for e in d.entries.clone() {
                            {
                                let target = join_doc_path(&d.path, &e.name);
                                let (r, base) = (root.clone(), cur.clone());
                                let is_dir = e.kind == "dir";
                                let renaming = rename_of() == Some(e.name.clone());
                                rsx! {
                                    div { class: "doc-row",
                                        if renaming {
                                            input { class: "input", value: "{rename_to}", oninput: move |ev| rename_to.set(ev.value()) }
                                            {
                                                let (r2, base2, tgt2) = (r.clone(), base.clone(), target.clone());
                                                rsx! {
                                                    button { class: "btn btn-primary btn-xs",
                                                        onclick: move |_| {
                                                            let nm = rename_to();
                                                            if !nm.trim().is_empty() {
                                                                let new_target = join_doc_path(&base2, nm.trim());
                                                                ws.send(rename_path_query(&r2, &tgt2, &new_target));
                                                                ws.send(list_dir_refresh_query(&r2, &base2));
                                                            }
                                                            rename_of.set(None);
                                                        },
                                                        "Save"
                                                    }
                                                }
                                            }
                                            button { class: "btn btn-glass btn-xs", onclick: move |_| rename_of.set(None), "Cancel" }
                                        } else {
                                            {
                                                let (r3, tgt3) = (r.clone(), target.clone());
                                                rsx! {
                                                    button { class: "doc-open",
                                                        onclick: move |_| {
                                                            if is_dir { ws.send(list_dir_query(&r3, &tgt3)); }
                                                            else { ws.send(read_file_query(&r3, &tgt3)); }
                                                        },
                                                        span { class: "doc-ico", {kind_glyph(&e.kind)} }
                                                        span { class: "doc-name", "{e.name}" }
                                                        span { class: "doc-size label-tech",
                                                            {if e.kind == "file" { fmt_size(e.size_bytes) } else { String::new() }} }
                                                    }
                                                }
                                            }
                                            div { class: "doc-actions",
                                                {
                                                    let nm = e.name.clone();
                                                    rsx! {
                                                        button { class: "btn btn-glass btn-xs", title: "Rename", "aria-label": "Rename",
                                                            onclick: move |_| { rename_to.set(nm.clone()); rename_of.set(Some(nm.clone())); }, "✎" }
                                                    }
                                                }
                                                {
                                                    let tgt4 = target.clone();
                                                    rsx! {
                                                        button { class: "btn btn-glass btn-xs danger", title: "Delete", "aria-label": "Delete",
                                                            onclick: move |_| delete_of.set(Some(tgt4.clone())), "🗑" }
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
            }
        }

        // Delete confirm modal (the one hard-gated, no-undo action).
        if let Some(path) = delete_of() {
            div { class: "modal-scrim",
                div { class: "glass-card modal",
                    h3 { "Delete?" }
                    p { "Permanently delete  " code { "{path}" } "  from {root}? This cannot be undone." }
                    div { class: "actions",
                        button { class: "btn btn-glass", onclick: move |_| delete_of.set(None), "Cancel" }
                        {
                            let (r, base, p) = (root.clone(), cur.clone(), path.clone());
                            rsx! {
                                button { class: "btn btn-primary",
                                    onclick: move |_| {
                                        ws.send(delete_file_query(&r, &p));
                                        ws.send(list_dir_refresh_query(&r, &base));
                                        delete_of.set(None);
                                    },
                                    "Delete"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The file content pane — an editor for text files (textarea + Save), or a
/// "not shown" note for binary / over-cap files.
#[component]
fn FileViewer(file: DocFile, root: String) -> Element {
    let ws = use_context::<Sender>();
    let mut documents = use_context::<Signal<DocumentsState>>();
    // Seeded once per file (the panel keys this component by path, so it
    // remounts — and re-seeds — when a different file is opened).
    let mut edited = use_signal(|| file.content.clone().unwrap_or_default());
    let editable = file.content.is_some() && !file.binary;

    rsx! {
        div { class: "glass-card doc-viewer",
            div { class: "panel-head",
                h4 { "{file.path}" }
                span { class: "chip", {fmt_size(file.size_bytes)} }
                if editable {
                    {
                        let (r, p) = (root.clone(), file.path.clone());
                        rsx! {
                            button { class: "btn btn-primary btn-xs",
                                onclick: move |_| {
                                    ws.send(write_file_query(&r, &p, edited(), true));
                                    // Vitrine §8 — the bridge handles frames in
                                    // order, so this re-read returns the
                                    // post-write content and refreshes the open
                                    // file in place (no screen reload needed).
                                    ws.send(read_file_query(&r, &p));
                                },
                                "Save"
                            }
                        }
                    }
                }
                button { class: "btn btn-glass btn-xs", onclick: move |_| documents.write().file = None, "Close" }
            }
            if file.truncated {
                div { class: "notice err", "Showing the first 256 KB of a larger file — editing is disabled to avoid truncating it." }
            }
            if editable && !file.truncated {
                textarea { class: "doc-edit", spellcheck: "false",
                    value: "{edited}", oninput: move |e| edited.set(e.value()) }
            } else {
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

/// Re-list after a mutation — a distinct id so the ws_task keeps the outcome
/// notice (a navigation list clears it).
fn list_dir_refresh_query(root: &str, path: &str) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-docs-refresh".to_string(),
        payload: QueryPayload::ListDir { root: root.to_string(), path: path.to_string() },
    }
}

// ── DW — the Documents write queries. ──

fn write_file_query(root: &str, path: &str, content: String, overwrite: bool) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-docs-write".to_string(),
        payload: QueryPayload::WriteFile {
            root: root.to_string(),
            path: path.to_string(),
            content,
            overwrite,
        },
    }
}

fn delete_file_query(root: &str, path: &str) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-docs-delete".to_string(),
        payload: QueryPayload::DeleteFile {
            root: root.to_string(),
            path: path.to_string(),
            confirm: true,
        },
    }
}

fn rename_path_query(root: &str, path: &str, new_path: &str) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-docs-rename".to_string(),
        payload: QueryPayload::RenamePath {
            root: root.to_string(),
            path: path.to_string(),
            new_path: new_path.to_string(),
        },
    }
}

fn make_dir_query(root: &str, path: &str) -> FrontendMessage {
    FrontendMessage::Query {
        id: "mc-docs-mkdir".to_string(),
        payload: QueryPayload::MakeDir { root: root.to_string(), path: path.to_string() },
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
    missions: Signal<Vec<TeamMissionView>>,
    dashboard: Signal<Dashboard>,
    memory: Signal<MemoryState>,
    wiki: Signal<WikiState>,
    lattice: Signal<GraphKnowledgeState>,
    settings: Signal<SettingsState>,
    agents: Signal<AgentsState>,
    teams: Signal<TeamsState>,
    documents: Signal<DocumentsState>,
    voice: Signal<VoiceState>,
    skills: Signal<SkillsState>,
    mcp: Signal<McpState>,
    mut connected: Signal<bool>,
    session: Signal<Option<String>>,
    transcript: Signal<Vec<ChatLine>>,
    streaming: Signal<String>,
    gate: Signal<Option<GateInfo>>,
) {
    // Vitrine walkthrough fix (2026-07-05, third operator casualty): a
    // daemon restart used to END this task — the socket died, `connected`
    // flipped false (visible only as the Command Center chip), the write
    // loop broke, and every later `ws.send` from every screen vanished
    // silently into a dead coroutine. The page looked alive (stale
    // signals still rendered) while edits, mission starts, and roster
    // saves went nowhere. This loop reconnects with backoff, replays the
    // dashboard boot queries on every (re)connect, and re-sends the one
    // in-flight message a dying socket rejected. Outbound traffic flows
    // at least every POLL_INTERVAL_MS (the mission/audit poll), so a dead
    // socket is detected within one poll tick.
    let mut attempt: u32 = 0;
    // The message a dying socket refused — re-sent first on reconnect so
    // an operator action that raced the disconnect still lands.
    let mut unsent: Option<String> = None;
    loop {
        let ws = match WebSocket::open(&ws_url()) {
            Ok(ws) => ws,
            Err(_) => {
                connected.set(false);
                attempt = attempt.saturating_add(1);
                TimeoutFuture::new(reconnect_backoff_ms(attempt)).await;
                continue;
            }
        };
        connected.set(true);
        attempt = 0;
        let (mut write, read) = ws.split();

        spawn(read_task(
            read, missions, dashboard, memory, wiki, lattice, settings, agents, teams,
            documents, voice, skills, mcp, connected, session, transcript, streaming, gate,
        ));

        // (Re)hydrate the dashboard one-shots — on a fresh page load this
        // duplicates the boot `use_future` harmlessly; on a reconnect it is
        // what refreshes the stale screens.
        for q in reconnect_boot_queries() {
            if let Ok(json) = serde_json::to_string(&q) {
                let _ = write.send(Message::Text(json)).await;
            }
        }
        if let Some(json) = unsent.take() {
            if write.send(Message::Text(json.clone())).await.is_err() {
                // This socket is already dead — stash the message back
                // and go straight to the next reconnect attempt.
                unsent = Some(json);
                connected.set(false);
                attempt = attempt.saturating_add(1);
                TimeoutFuture::new(reconnect_backoff_ms(attempt)).await;
                continue;
            }
        }

        // Outbound relay: runs until the socket dies (write error) or the
        // app tears down (rx closed).
        loop {
            match rx.next().await {
                Some(msg) => {
                    let Ok(json) = serde_json::to_string(&msg) else {
                        continue;
                    };
                    if write.send(Message::Text(json.clone())).await.is_err() {
                        unsent = Some(json);
                        break;
                    }
                }
                None => return,
            }
        }
        connected.set(false);
        attempt = attempt.saturating_add(1);
        TimeoutFuture::new(reconnect_backoff_ms(attempt)).await;
    }
}

/// Reconnect backoff: 1s, 2s, 4s, then 8s forever. Fast enough that a
/// deploy restart heals in seconds; slow enough not to hammer a daemon
/// that is genuinely down.
fn reconnect_backoff_ms(attempt: u32) -> u32 {
    match attempt {
        0 | 1 => 1_000,
        2 => 2_000,
        3 => 4_000,
        _ => 8_000,
    }
}

/// The dashboard's boot queries, re-sent on every (re)connect so a
/// reconnected page refreshes without navigation. Mission list + audit
/// refresh via the standing poll; screen-local data refreshes on view
/// switch.
fn reconnect_boot_queries() -> Vec<FrontendMessage> {
    let q = |id: &str, payload: QueryPayload| FrontendMessage::Query {
        id: id.to_string(),
        payload,
    };
    vec![
        q("mc-profile", QueryPayload::GetProfile { from_disk: false }),
        q("mc-verify", QueryPayload::VerifyAuditChain),
        q("mc-settings", QueryPayload::GetSettings),
        q("mc-schedules", QueryPayload::GetSchedules),
        q("mc-teams-roster", QueryPayload::GetTeamRoster),
    ]
}

#[allow(clippy::too_many_arguments)]
async fn read_task(
    mut read: futures_util::stream::SplitStream<WebSocket>,
    mut missions: Signal<Vec<TeamMissionView>>,
    mut dashboard: Signal<Dashboard>,
    mut memory: Signal<MemoryState>,
    mut wiki: Signal<WikiState>,
    mut lattice: Signal<GraphKnowledgeState>,
    mut settings: Signal<SettingsState>,
    mut agents: Signal<AgentsState>,
    mut teams: Signal<TeamsState>,
    mut documents: Signal<DocumentsState>,
    mut voice: Signal<VoiceState>,
    mut skills: Signal<SkillsState>,
    mut mcp: Signal<McpState>,
    mut connected: Signal<bool>,
    mut session: Signal<Option<String>>,
    mut transcript: Signal<Vec<ChatLine>>,
    mut streaming: Signal<String>,
    mut gate: Signal<Option<GateInfo>>,
) {
    {
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
                    teams.write().roster = Some(cfg);
                }
                // Chapter Roster (RO.3) — a SetTeamRoster save was accepted: the
                // daemon echoes the validated, re-read roster + restart flag.
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::TeamRosterApplied { roster: cfg, restart_required },
                    ..
                } => {
                    let mut t = teams.write();
                    t.roster = Some(cfg);
                    t.restart_required = restart_required;
                    t.notice = Some((true, "Team saved to the team config file.".to_string()));
                }
                // Chapter Z — Documents browser: a directory listing arrived;
                // the echoed `path` is authoritative. A *refresh* re-list (after
                // a DW mutation) keeps the outcome notice; a *navigation* re-list
                // clears it.
                DaemonEnvelope::QueryResponse {
                    id,
                    payload: QueryResponsePayload::ListDir { entries, path },
                } => {
                    let mut d = documents.write();
                    d.entries = entries;
                    d.path = path;
                    d.loaded = true;
                    // A *refresh* re-list (after a DW mutation) keeps the
                    // outcome notice AND the open file — Vitrine §8: closing
                    // the viewer out from under an edit read as data loss. A
                    // navigation re-list clears both.
                    if !id.starts_with("mc-docs-refresh") {
                        d.file = None;
                        d.notice = None;
                    }
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::ReadFile { file },
                    ..
                } => {
                    let mut d = documents.write();
                    d.file = Some(file);
                    d.notice = None;
                }
                // DW — a write mutation acked: set the outcome notice + bump the
                // refresh tick so the panel re-lists the directory.
                DaemonEnvelope::QueryResponse {
                    id,
                    payload: QueryResponsePayload::FsMutation { ok, error },
                } if id.starts_with("mc-docs") => {
                    let mut d = documents.write();
                    d.notice = Some(if ok {
                        let what = if id.contains("delete") {
                            "Deleted."
                        } else if id.contains("rename") {
                            "Renamed."
                        } else if id.contains("mkdir") {
                            "Folder created."
                        } else {
                            "Saved."
                        };
                        (true, what.to_string())
                    } else {
                        (false, error.unwrap_or_else(|| "Operation failed.".to_string()))
                    });
                }
                // A denied path / read failure on the Documents screen (ids
                // prefixed `mc-docs`) → a notice, leaving the listing intact.
                DaemonEnvelope::QueryResponse {
                    id,
                    payload: QueryResponsePayload::QueryError { message, .. },
                } if id.starts_with("mc-docs") => {
                    documents.write().notice = Some((false, message));
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::ListAuditEntries { entries, total_len },
                    ..
                } => {
                    let mut d = dashboard.write();
                    d.audit_entries = entries;
                    d.audit_total = total_len;
                    // First dashboard snapshot in — switch off the skeleton.
                    d.loaded = true;
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
                    payload: QueryResponsePayload::GetMemoryGraph { nodes, edges },
                    ..
                } => {
                    let mut m = memory.write();
                    m.graph_nodes = nodes;
                    m.graph_edges = edges;
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::GetMemoryTopicEntries { entries },
                    ..
                } => {
                    let mut m = memory.write();
                    m.entries = entries;
                    m.fell_back = false;
                    m.loaded = true;
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::SearchMemory { matches, fell_back_to_keyword },
                    ..
                } => {
                    let mut m = memory.write();
                    m.entries = matches;
                    m.fell_back = fell_back_to_keyword;
                    m.loaded = true;
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::ListWikiPages { pages },
                    ..
                } => {
                    wiki.write().pages = pages;
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::GetSkills { skills: sk, pending_proposals },
                    ..
                } => {
                    let mut s = skills.write();
                    s.skills = sk;
                    s.pending_proposals = pending_proposals;
                    s.loaded = true;
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::GetMcpStatus { captured_unix, servers },
                    ..
                } => {
                    // Chapter Lantern — the MCP screen's snapshot.
                    let mut m = mcp.write();
                    m.servers = servers;
                    m.captured_unix = captured_unix;
                    m.loaded = true;
                }
                DaemonEnvelope::SkillForgotten { ok, removed, name, .. } if ok && removed => {
                    // Chapter Repertoire — drop the forgotten skill locally.
                    skills.write().skills.retain(|s| s.skill.name != name);
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::GetWikiPage { page },
                    ..
                } => {
                    wiki.write().selected = page;
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::GetKnowledgeGraph { entities, edges },
                    ..
                } => {
                    let mut l = lattice.write();
                    l.entities = entities;
                    l.edges = edges;
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::GetSettings { settings: snap },
                    ..
                } => {
                    // Feeds both the Settings screen and the Command Center's
                    // agent-vitals rail (same snapshot — model/provider/ctx/
                    // autonomy/access).
                    dashboard.write().settings = Some(snap.clone());
                    settings.write().snapshot = Some(snap);
                }
                DaemonEnvelope::QueryResponse {
                    payload: QueryResponsePayload::Schedules { schedules },
                    ..
                } => {
                    dashboard.write().schedules = schedules;
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
                // GE.3 — LLM Profile draft arrived (or failed). The onboarding
                // flow's step 1 watches `profile_draft_resp` to clear its
                // spinner + fill its six fields from `profile_draft`.
                DaemonEnvelope::ProfileDrafted { draft, error, .. } => {
                    let mut a = agents.write();
                    a.profile_draft_resp += 1;
                    let had_draft = draft.is_some();
                    a.profile_draft = draft;
                    if !had_draft {
                        a.notice = Some((
                            false,
                            error.unwrap_or_else(|| "couldn't draft a profile".to_string()),
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
                // Chapter Roster (RO.3) — a team save/read failure (e.g. the
                // server-side `TeamConfig::validate` rejected the roster). Ids
                // are prefixed `mc-teams` so it lands on the Teams banner.
                DaemonEnvelope::QueryResponse {
                    id,
                    payload: QueryResponsePayload::QueryError { message, .. },
                } if id.starts_with("mc-teams") => {
                    teams.write().notice = Some((false, message));
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
        // Socket died: flip the banner on immediately and clear the chat
        // session — the ws bridge mints a fresh one on reconnect.
        connected.set(false);
        session.set(None);
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
