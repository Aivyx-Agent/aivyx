//! `connect` — guided OAuth onboarding for the productivity tools, in
//! the Aivyx palette. This is the `aivyx-pa connect` surface (Phase 182).
//!
//! Each productivity integration (Gmail, Calendar, Drive, Notion,
//! Obsidian, n8n) runs as its own sandboxed tool process with an
//! **operator-provided** OAuth app and a per-tool-process token file
//! (`~/.aivyx-pa/tool-processes/<svc>/token`, mode 0600). Aivyx never
//! holds a shared secret — you bring your own OAuth client. Left: the
//! services + connection state; right: the selected service's account,
//! scopes, token, and the guided flow.
//!
//! Run it live (truecolor, any key to exit):
//!     cargo run -p aivyx-tui --example connect
//! Or print the character grid:
//!     cargo run -p aivyx-tui --example connect -- --snapshot

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

use aivyx_tui::palette::{AMBER, BG, BORDER, DIM, DIMMER, FG, LAV, OK, STATUS_BG};

const W: u16 = 104;
const H: u16 = 36;
const SELECTED: usize = 0; // Gmail

fn dim() -> Style {
    Style::default().fg(DIM)
}
fn bold(c: Color) -> Style {
    Style::default().fg(c).add_modifier(Modifier::BOLD)
}

fn panel(title: &str, focused: bool) -> Block<'_> {
    let (bc, tc) = if focused { (AMBER, AMBER) } else { (BORDER, DIM) };
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(bc))
        .title(Span::styled(format!(" {title} "), bold(tc)))
        .padding(Padding::new(1, 1, 0, 0))
        .style(Style::default().bg(BG))
}

struct Svc {
    connected: bool,
    name: &'static str,
    note: &'static str,
}

fn services() -> Vec<Svc> {
    vec![
        Svc { connected: true, name: "Gmail", note: "connected · 6 tools" },
        Svc { connected: true, name: "Calendar", note: "connected · 4 tools" },
        Svc { connected: true, name: "Drive", note: "connected · 3 tools" },
        Svc { connected: false, name: "Notion", note: "not connected" },
        Svc { connected: false, name: "Obsidian", note: "local vault · no auth" },
        Svc { connected: false, name: "n8n", note: "not connected" },
    ]
}

fn draw(f: &mut Frame) {
    let area = f.area();
    f.render_widget(Block::new().style(Style::default().bg(BG)), area);

    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(area);

    header(f, rows[0]);
    sub(f, rows[1]);
    body(f, rows[3]);
    status_bar(f, rows[4]);
}

fn header(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("▌", bold(AMBER)),
            Span::styled(" AIVYX PA", bold(AMBER)),
            Span::styled("  Connect", bold(FG)),
        ])),
        area,
    );
    f.render_widget(
        Paragraph::new(
            Line::from(vec![
                Span::styled("3 connected", Style::default().fg(OK)),
                Span::styled("  ·  3 available", dim()),
            ])
            .right_aligned(),
        ),
        area,
    );
}

fn sub(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" guided OAuth · ", dim()),
            Span::styled("you bring the OAuth app", Style::default().fg(LAV)),
            Span::styled(" · per-service token, sandboxed", dim()),
        ])),
        area,
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled("aivyx-pa connect ", dim())]).right_aligned()),
        area,
    );
}

fn body(f: &mut Frame, area: Rect) {
    let cols = Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
        .spacing(1)
        .split(area);
    service_list(f, cols[0]);
    detail(f, cols[1]);
}

fn service_list(f: &mut Frame, area: Rect) {
    let lines: Vec<Line> = services()
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let sel = i == SELECTED;
            let marker = if sel { Span::styled("▌ ", bold(AMBER)) } else { Span::styled("  ", dim()) };
            let status = if s.connected {
                Span::styled("● ", Style::default().fg(OK))
            } else {
                Span::styled("○ ", Style::default().fg(DIMMER))
            };
            let ns = if sel { bold(AMBER) } else if s.connected { bold(FG) } else { Style::default().fg(DIM) };
            Line::from(vec![
                marker,
                status,
                Span::styled(format!("{:<10}", s.name), ns),
                Span::styled(s.note, Style::default().fg(DIMMER)),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(panel("SERVICES", true)), area);
}

fn detail(f: &mut Frame, area: Rect) {
    let parts = Layout::vertical([Constraint::Min(8), Constraint::Length(3)]).split(area);

    let kv = |k: &'static str, v: Span<'static>| {
        Line::from(vec![Span::styled(format!("{k:<9}"), dim()), v])
    };
    let step = |ok: bool, text: &'static str| {
        let g = if ok { Span::styled("✓ ", bold(OK)) } else { Span::styled("○ ", Style::default().fg(DIMMER)) };
        Line::from(vec![g, Span::styled(text, Style::default().fg(if ok { FG } else { DIM }))])
    };
    let lines = vec![
        Line::from(vec![
            Span::styled("Gmail", bold(FG)),
            Span::styled("   ● connected", Style::default().fg(OK)),
        ]),
        kv("account", Span::styled("you@gmail.com", Style::default().fg(FG))),
        kv("scopes", Span::styled("gmail.readonly + gmail.send", Style::default().fg(FG))),
        kv("token", Span::styled("…/tool-processes/gmail/token · 0600", Style::default().fg(DIMMER))),
        Line::from(""),
        Line::from(Span::styled("FLOW", dim())),
        step(true, "your OAuth client id + secret"),
        step(true, "browser consent (your account)"),
        step(true, "token stored per tool process"),
        Line::from(""),
        Line::from(Span::styled(
            "Aivyx PA never sees a shared secret — you own the app.",
            Style::default().fg(DIMMER),
        )),
    ];
    f.render_widget(Paragraph::new(lines).block(panel("Gmail · CONNECTION", false)), parts[0]);

    let action_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .padding(Padding::horizontal(1))
        .style(Style::default().bg(BG));
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[ reconnect ]", bold(AMBER)),
            Span::styled("   [ revoke ]   ", Style::default().fg(FG)),
            Span::styled("[ test ]", dim()),
        ]))
        .block(action_block),
        parts[1],
    );
}

fn status_bar(f: &mut Frame, area: Rect) {
    let sep = || Span::styled(" · ", Style::default().fg(DIMMER));
    let left = Line::from(vec![
        Span::styled(" Gmail", bold(AMBER)),
        Span::styled(" · ", dim()),
        Span::styled("connected", Style::default().fg(OK)),
        Span::styled(" · ", dim()),
        Span::styled("operator-owned OAuth", Style::default().fg(LAV)),
    ]);
    let right = Line::from(vec![
        Span::styled("↑↓ service", dim()),
        sep(),
        Span::styled("Enter connect", Style::default().fg(OK)),
        sep(),
        Span::styled("r revoke", dim()),
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
