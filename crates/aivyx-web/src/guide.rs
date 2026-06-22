//! End-user guide content for the Studio's **Guide** screen.
//!
//! The pages are authored as plain task-oriented markdown under `docs/guide/`
//! (the single source of truth — also readable on GitHub / reusable for a docs
//! site). They are bundled into the WASM binary with `include_str!` and
//! rendered to HTML at runtime by [`render`]. No network fetch, no separate
//! deploy: the guide ships inside the Studio and works fully offline.
//!
//! Adding a page = drop a `NN-name.md` in `docs/guide/` and add one [`Page`]
//! entry below (order here is the order in the screen's page list).

use pulldown_cmark::{html, Options, Parser};

/// One guide page: a stable `id`, a short `title` for the page list, and the
/// raw markdown `body` bundled from `docs/guide/`.
pub struct Page {
    pub id: &'static str,
    pub title: &'static str,
    pub body: &'static str,
}

/// The ordered guide. `include_str!` paths are relative to this file
/// (`crates/aivyx-web/src/guide.rs` → `../../../docs/guide/`).
pub const PAGES: &[Page] = &[
    Page {
        id: "welcome",
        title: "Welcome",
        body: include_str!("../../../docs/guide/01-welcome.md"),
    },
    Page {
        id: "getting-started",
        title: "Getting started",
        body: include_str!("../../../docs/guide/02-getting-started.md"),
    },
    Page {
        id: "create-your-agent",
        title: "Create your agent",
        body: include_str!("../../../docs/guide/03-create-your-agent.md"),
    },
    Page {
        id: "chat-and-missions",
        title: "Chat & missions",
        body: include_str!("../../../docs/guide/04-chat-and-missions.md"),
    },
    Page {
        id: "memory",
        title: "Memory",
        body: include_str!("../../../docs/guide/05-memory.md"),
    },
    Page {
        id: "skills-and-persona",
        title: "Skills & personality",
        body: include_str!("../../../docs/guide/06-skills-and-persona.md"),
    },
    Page {
        id: "teams",
        title: "Teams",
        body: include_str!("../../../docs/guide/07-teams.md"),
    },
    Page {
        id: "access-and-settings",
        title: "Access & settings",
        body: include_str!("../../../docs/guide/08-access-and-settings.md"),
    },
    Page {
        id: "screens",
        title: "Screens reference",
        body: include_str!("../../../docs/guide/09-screens-reference.md"),
    },
    Page {
        id: "troubleshooting",
        title: "Troubleshooting",
        body: include_str!("../../../docs/guide/10-troubleshooting.md"),
    },
];

/// Render a guide page's markdown to an HTML string for injection via
/// `dangerous_inner_html`. The input is our own committed, trusted content
/// (never user input), so raw-HTML injection is safe here. Tables and
/// strikethrough are enabled to match the markdown the pages use.
pub fn render(markdown: &str) -> String {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    let parser = Parser::new_ext(markdown, options);
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}
