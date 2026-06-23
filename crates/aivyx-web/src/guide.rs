//! End-user guide content for the Studio's **Guide** screen.
//!
//! The pages are authored as plain task-oriented markdown under `docs/guide/`
//! (the single source of truth — also readable on GitHub / reusable for a docs
//! site). They are bundled into the WASM binary with `include_str!` and
//! rendered to HTML at runtime by [`render`]. No network fetch, no separate
//! deploy: the guide ships inside the Studio and works fully offline.
//!
//! Cross-page links are authored as ordinary **relative `.md` links** (e.g.
//! `[Teams](07-teams.md)`) so they resolve correctly when the files are read on
//! GitHub, *and* drive in-app navigation: the Guide panel intercepts clicks on
//! those links and maps the filename back to a page via [`index_for_href`].
//!
//! Adding a page = drop a `NN-name.md` in `docs/guide/` and add one [`Page`]
//! entry below (order here is the order in the screen's page list).

use pulldown_cmark::{html, Options, Parser};

/// One guide page: a stable `id`, the source `file` name (used to resolve
/// in-app `.md` cross-links), a short `title` for the page list, and the raw
/// markdown `body` bundled from `docs/guide/`.
pub struct Page {
    pub id: &'static str,
    pub file: &'static str,
    pub title: &'static str,
    pub body: &'static str,
}

/// The ordered guide. `include_str!` paths are relative to this file
/// (`crates/aivyx-web/src/guide.rs` → `../../../docs/guide/`).
pub const PAGES: &[Page] = &[
    Page {
        id: "welcome",
        file: "01-welcome.md",
        title: "Welcome",
        body: include_str!("../../../docs/guide/01-welcome.md"),
    },
    Page {
        id: "getting-started",
        file: "02-getting-started.md",
        title: "Getting started",
        body: include_str!("../../../docs/guide/02-getting-started.md"),
    },
    Page {
        id: "create-your-agent",
        file: "03-create-your-agent.md",
        title: "Create your agent",
        body: include_str!("../../../docs/guide/03-create-your-agent.md"),
    },
    Page {
        id: "chat-and-missions",
        file: "04-chat-and-missions.md",
        title: "Chat & missions",
        body: include_str!("../../../docs/guide/04-chat-and-missions.md"),
    },
    Page {
        id: "memory",
        file: "05-memory.md",
        title: "Memory",
        body: include_str!("../../../docs/guide/05-memory.md"),
    },
    Page {
        id: "skills-and-persona",
        file: "06-skills-and-persona.md",
        title: "Skills & personality",
        body: include_str!("../../../docs/guide/06-skills-and-persona.md"),
    },
    Page {
        id: "teams",
        file: "07-teams.md",
        title: "Teams",
        body: include_str!("../../../docs/guide/07-teams.md"),
    },
    Page {
        id: "access-and-settings",
        file: "08-access-and-settings.md",
        title: "Access & settings",
        body: include_str!("../../../docs/guide/08-access-and-settings.md"),
    },
    Page {
        id: "screens",
        file: "09-screens-reference.md",
        title: "Screens reference",
        body: include_str!("../../../docs/guide/09-screens-reference.md"),
    },
    Page {
        id: "desktop-app",
        file: "11-desktop-app.md",
        title: "Desktop app",
        body: include_str!("../../../docs/guide/11-desktop-app.md"),
    },
    Page {
        id: "troubleshooting",
        file: "10-troubleshooting.md",
        title: "Troubleshooting",
        body: include_str!("../../../docs/guide/10-troubleshooting.md"),
    },
];

/// Resolve an in-app cross-page link `href` to a [`PAGES`] index.
///
/// Pages link to each other with relative `.md` filenames (`07-teams.md`). The
/// rendered anchors keep that verbatim in their `href` attribute, so we match
/// on the bare filename — tolerating an optional `./` prefix and any `#anchor`
/// fragment. Returns `None` for external links (`http(s)://`, `mailto:`) and
/// anything not naming a known page, so those fall through to default handling.
pub fn index_for_href(href: &str) -> Option<usize> {
    let name = href.trim_start_matches("./");
    let name = name.split('#').next().unwrap_or(name);
    PAGES.iter().position(|p| p.file == name)
}

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
