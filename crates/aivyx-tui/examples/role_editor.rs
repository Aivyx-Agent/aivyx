//! `role_editor` — a ratatui translation of the "Agent Editor" GUI
//! mockup, re-grounded in what Aivyx actually edits: a **Role**.
//!
//! Aivyx is one agent. What you configure are Roles (`default`,
//! `researcher`, `coder`) — each a system prompt + trust tier +
//! capability ceiling + tool allowlist — plus the Profile/Persona.
//! So this is a Role editor, not an "agent" roster: no named android
//! units, no "Tier 4", no 4.2TB memory. The fields map to the real
//! `[roles.<name>]` keys in `aivyx-pa.toml` / the `Role` struct.
//!
//! It keeps the GUI's identity (amber-on-near-black, panel separation)
//! and its strongest idea — the **capability permission checklist** —
//! which is the one part that translates cleanly to a terminal.
//!
//! Run it live (truecolor, any key to exit):
//!     cargo run -p aivyx-tui --example role_editor
//! Or print the character grid:
//!     cargo run -p aivyx-tui --example role_editor -- --snapshot

use std::io::{self, IsTerminal};
use std::time::Duration;

use crossterm::event::{self, Event};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::{CrosstermBackend, TestBackend};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Padding, Paragraph};
use ratatui::{Frame, Terminal};

// Canonical brand colors — shared with the shipped chat surface.
use aivyx_tui::palette::{AMBER, BG, BORDER, DIM, DIMMER, ERR, FG, LAV, OK, STATUS_BG};

const W: u16 = 104;
const H: u16 = 36;

fn dim() -> Style {
    Style::default().fg(DIM)
}
fn bold(c: Color) -> Style {
    Style::default().fg(c).add_modifier(Modifier::BOLD)
}

/// A titled panel; amber border when it's the focused field.
fn panel(title: &str, focused: bool) -> Block<'_> {
    let (bc, tc) = if focused {
        (AMBER, AMBER)
    } else {
        (BORDER, DIM)
    };
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(bc))
        .title(Span::styled(format!(" {title} "), bold(tc)))
        .padding(Padding::new(1, 1, 0, 0))
        .style(Style::default().bg(BG))
}

/// `[x]` (amber) / `[ ]` (dim) checkbox span.
fn check(on: bool) -> Span<'static> {
    if on {
        Span::styled("[x]", bold(AMBER))
    } else {
        Span::styled("[ ]", Style::default().fg(DIMMER))
    }
}

fn draw(f: &mut Frame) {
    let area = f.area();
    f.render_widget(Block::new().style(Style::default().bg(BG)), area);

    let rows = Layout::vertical([
        Constraint::Length(1),  // header
        Constraint::Length(1),  // identity subline
        Constraint::Length(1),  // spacer
        Constraint::Length(19), // editor body
        Constraint::Min(7),     // tool allowlist
        Constraint::Length(1),  // status bar
    ])
    .split(area);

    header(f, rows[0]);
    subline(f, rows[1]);
    body(f, rows[3]);
    tools(f, rows[4]);
    status_bar(f, rows[5]);
}

fn header(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("▌", bold(AMBER)),
            Span::styled(" AIVYX PA", bold(AMBER)),
            Span::styled("  Role Editor", bold(FG)),
        ])),
        area,
    );
    f.render_widget(
        Paragraph::new(
            Line::from(vec![
                Span::styled("editing ", dim()),
                Span::styled("researcher", Style::default().fg(LAV)),
                Span::styled("  ·  ", dim()),
                Span::styled("●", Style::default().fg(AMBER)),
                Span::styled(" unsaved", Style::default().fg(AMBER)),
            ])
            .right_aligned(),
        ),
        area,
    );
}

fn subline(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" based on ", dim()),
            Span::styled("default", Style::default().fg(FG)),
            Span::styled("   ·   trust ", dim()),
            Span::styled("Trusted", bold(AMBER)),
            Span::styled("   ·   ", dim()),
            Span::styled("6 of 8 scopes", Style::default().fg(FG)),
            Span::styled(" · ", dim()),
            Span::styled("9 of 14 tools", Style::default().fg(FG)),
        ])),
        area,
    );
}

fn body(f: &mut Frame, area: Rect) {
    let cols = Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)])
        .spacing(1)
        .split(area);

    let left = Layout::vertical([Constraint::Length(11), Constraint::Min(6)]).split(cols[0]);
    let right = Layout::vertical([Constraint::Length(5), Constraint::Min(12)]).split(cols[1]);

    system_prompt(f, left[0]);
    persona(f, left[1]);
    trust(f, right[0]);
    capabilities(f, right[1]);
}

fn system_prompt(f: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(Span::styled(
            "You are a meticulous research assistant. Prefer primary",
            Style::default().fg(FG),
        )),
        Line::from(Span::styled(
            "sources, cite them inline, and separate established fact",
            Style::default().fg(FG),
        )),
        Line::from(Span::styled(
            "from inference. When unsure, say so and propose how to",
            Style::default().fg(FG),
        )),
        Line::from(vec![
            Span::styled("verify. Keep answers concise and structured.", Style::default().fg(FG)),
            Span::styled("▏", bold(AMBER)), // cursor: this is the focused field
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("tokens 412 / 8192", dim()),
            Span::styled("   ·   [roles.researcher].system_prompt", Style::default().fg(DIMMER)),
        ]),
    ];
    f.render_widget(
        Paragraph::new(lines).block(panel("SYSTEM PROMPT", true)),
        area,
    );
}

fn persona(f: &mut Frame, area: Rect) {
    let trait_chip = |t: &'static str| {
        vec![
            Span::styled("‹", Style::default().fg(DIMMER)),
            Span::styled(t, Style::default().fg(LAV)),
            Span::styled("› ", Style::default().fg(DIMMER)),
        ]
    };
    let mut traits = vec![Span::styled("traits   ", dim())];
    for t in ["Analytical", "Thorough", "Cautious"] {
        traits.extend(trait_chip(t));
    }
    traits.push(Span::styled("+ add", Style::default().fg(DIM)));

    let lines = vec![
        Line::from(traits),
        Line::from(""),
        Line::from(vec![
            Span::styled("style    ", dim()),
            Span::styled("❮ ", Style::default().fg(DIMMER)),
            Span::styled("Concise & direct", bold(AMBER)),
            Span::styled(" ❯", Style::default().fg(DIMMER)),
        ]),
        Line::from(vec![Span::styled(
            "         from Profile + reflection-grown Persona",
            Style::default().fg(DIMMER),
        )]),
    ];
    f.render_widget(Paragraph::new(lines).block(panel("PERSONA & STYLE", false)), area);
}

fn trust(f: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(vec![
            Span::styled("Untrusted   ", Style::default().fg(DIMMER)),
            Span::styled("SemiTrusted   ", Style::default().fg(DIM)),
            Span::styled("❮ Trusted ❯", bold(AMBER)),
        ]),
        Line::from(Span::styled(
            "enforced per turn · gates on writes",
            Style::default().fg(DIMMER),
        )),
    ];
    f.render_widget(Paragraph::new(lines).block(panel("TRUST TIER", false)), area);
}

fn capabilities(f: &mut Frame, area: Rect) {
    // (scope, enabled, qualifier/note) — real Aivyx capability bases.
    let scopes: [(&str, bool, &str); 8] = [
        ("fs.read", true, "./ (sandbox)"),
        ("fs.write", true, "./reports"),
        ("web.search", true, ""),
        ("web.fetch", false, ""),
        ("shell.exec", false, "off · secure default"),
        ("memory.read", true, ""),
        ("memory.write", true, ""),
        ("remind.add", true, ""),
    ];
    let lines: Vec<Line> = scopes
        .iter()
        .map(|(name, on, note)| {
            let name_style = if *on {
                Style::default().fg(FG)
            } else {
                Style::default().fg(DIMMER)
            };
            let note_style = if name.starts_with("shell") {
                Style::default().fg(ERR)
            } else {
                Style::default().fg(DIMMER)
            };
            Line::from(vec![
                check(*on),
                Span::styled(format!(" {name:<13}"), name_style),
                Span::styled(*note, note_style),
            ])
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines).block(panel("CAPABILITY SCOPES", false)),
        area,
    );
}

fn tools(f: &mut Frame, area: Rect) {
    // The real in-tree / productivity tool catalog, with allowlist state.
    let catalog: [(&str, bool); 14] = [
        ("web.search", true),
        ("fs.read", true),
        ("fs.write", true),
        ("shell.exec", false),
        ("memory.recall", true),
        ("memory.write", true),
        ("skills.invoke", true),
        ("remind.add", true),
        ("task.add", true),
        ("web.fetch", false),
        ("gmail.send", false),
        ("calendar.create", false),
        ("drive.search", false),
        ("notion.append", false),
    ];
    // Three columns.
    let per_col = catalog.len().div_ceil(3);
    let mut lines: Vec<Line> = Vec::new();
    for r in 0..per_col {
        let mut spans = Vec::new();
        for c in 0..3 {
            if let Some((name, on)) = catalog.get(c * per_col + r) {
                let ns = if *on {
                    Style::default().fg(FG)
                } else {
                    Style::default().fg(DIMMER)
                };
                spans.push(check(*on));
                spans.push(Span::styled(format!(" {name:<26}"), ns));
            }
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("/ search tools", dim()),
        Span::styled("   ·   ", Style::default().fg(DIMMER)),
        Span::styled("+ connect MCP server", dim()),
    ]));
    f.render_widget(
        Paragraph::new(lines).block(panel("TOOL ALLOWLIST  ·  9 of 14 enabled", false)),
        area,
    );
}

fn status_bar(f: &mut Frame, area: Rect) {
    let sep = || Span::styled(" · ", Style::default().fg(DIMMER));
    let left = Line::from(vec![
        Span::styled(" researcher", bold(AMBER)),
        Span::styled(" · ", dim()),
        Span::styled("Trusted", Style::default().fg(OK)),
        Span::styled(" · ", dim()),
        Span::styled("● unsaved", Style::default().fg(AMBER)),
    ]);
    let right = Line::from(vec![
        Span::styled("Tab field", dim()),
        sep(),
        Span::styled("Space toggle", dim()),
        sep(),
        Span::styled("^S save", Style::default().fg(OK)),
        sep(),
        Span::styled("^Q quit ", dim()),
    ])
    .right_aligned();
    f.render_widget(Paragraph::new(left).style(Style::default().bg(STATUS_BG)), area);
    f.render_widget(Paragraph::new(right).style(Style::default().bg(STATUS_BG)), area);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let snapshot = std::env::args().any(|a| a == "--snapshot") || !io::stdout().is_terminal();

    if snapshot {
        let mut term = Terminal::new(TestBackend::new(W, H))?;
        term.draw(draw)?;
        let buf = term.backend().buffer().clone();
        for y in 0..H {
            let mut s = String::new();
            for x in 0..W {
                s.push_str(buf[(x, y)].symbol());
            }
            println!("{}", s.trim_end());
        }
        return Ok(());
    }

    enable_raw_mode()?;
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen)?;
    let mut term = Terminal::new(CrosstermBackend::new(out))?;
    term.draw(draw)?;
    loop {
        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(_) = event::read()? {
                break;
            }
        }
    }
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}
