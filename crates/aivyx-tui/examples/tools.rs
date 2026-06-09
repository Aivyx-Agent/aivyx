//! `tools` — the agent's tool registry + observability, in the Aivyx
//! palette. The built-in counterpart to `mcp_servers`: where that lists
//! external MCP servers, this lists the **whole tool surface** the
//! single agent can call, the way `aivyx tools` does (Phase 102) —
//! each tool annotated with its provenance, capability scope, gate
//! state, and audit-derived call/outcome stats.
//!
//! Provenance is real: `built-in` (core daemon tools), `toolkit` (the
//! bundled multi-tool binary), `tool-proc` (per-service OAuth tool
//! processes), and `mcp` (tools surfaced by an MCP server — see the
//! `mcp_servers` example). Everything is capped by the active role's
//! trust ceiling; `⚑` tools gate for approval, `⛔` are stripped by the
//! current tier / sandbox.
//!
//! Run it live (truecolor, any key to exit):
//!     cargo run -p aivyx-tui --example tools
//! Or print the character grid:
//!     cargo run -p aivyx-tui --example tools -- --snapshot

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

use aivyx_tui::palette::{AMBER, BG, BORDER, DIM, DIMMER, ERR, FG, LAV, OK, STATUS_BG};

const W: u16 = 104;
const H: u16 = 36;
const SELECTED: usize = 0; // web.search, detailed on the right

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

#[derive(Clone, Copy, PartialEq)]
enum Gov {
    Open,  // callable, not gated
    Gated, // pauses for approval
    Off,   // stripped by trust tier / sandbox
}

struct Tool {
    name: &'static str,
    provenance: &'static str,
    gov: Gov,
    stats: &'static str,
}

fn tools() -> Vec<Tool> {
    use Gov::*;
    vec![
        Tool { name: "web.search", provenance: "toolkit", gov: Open, stats: "128 · 99%" },
        Tool { name: "web.fetch", provenance: "toolkit", gov: Open, stats: "54 · 96%" },
        Tool { name: "fs.read", provenance: "built-in", gov: Open, stats: "412 · 100%" },
        Tool { name: "fs.write", provenance: "built-in", gov: Gated, stats: "37 · 97%" },
        Tool { name: "shell.exec", provenance: "built-in", gov: Off, stats: "stripped" },
        Tool { name: "memory.recall", provenance: "built-in", gov: Open, stats: "1.2k · 100%" },
        Tool { name: "skills.invoke", provenance: "built-in", gov: Open, stats: "22 · 100%" },
        Tool { name: "remind.add", provenance: "built-in", gov: Open, stats: "9 · 100%" },
        Tool { name: "task.add", provenance: "toolkit", gov: Open, stats: "14 · 100%" },
        Tool { name: "gmail.send", provenance: "tool-proc", gov: Gated, stats: "4 · 100%" },
        Tool { name: "mcp.postgres.query", provenance: "mcp", gov: Open, stats: "18 · 100%" },
    ]
}

fn prov_style(p: &str) -> Style {
    // External surfaces (tool processes, MCP) in lavender; in-tree dim.
    if p == "tool-proc" || p == "mcp" {
        Style::default().fg(LAV)
    } else {
        Style::default().fg(DIM)
    }
}

fn draw(f: &mut Frame) {
    let area = f.area();
    f.render_widget(Block::new().style(Style::default().bg(BG)), area);

    let rows = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Length(1), // role line
        Constraint::Length(1), // spacer
        Constraint::Min(0),    // body
        Constraint::Length(1), // status
    ])
    .split(area);

    header(f, rows[0]);
    role_line(f, rows[1]);
    body(f, rows[3]);
    status_bar(f, rows[4]);
}

fn header(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("▌", bold(AMBER)),
            Span::styled(" AIVYX", bold(AMBER)),
            Span::styled("  Tools", bold(FG)),
        ])),
        area,
    );
    f.render_widget(
        Paragraph::new(
            Line::from(vec![
                Span::styled("11 callable", Style::default().fg(OK)),
                Span::styled("  ·  ", dim()),
                Span::styled("2 gated", Style::default().fg(AMBER)),
                Span::styled("  ·  ", dim()),
                Span::styled("1 off", Style::default().fg(DIM)),
            ])
            .right_aligned(),
        ),
        area,
    );
}

fn role_line(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" role ", dim()),
            Span::styled("researcher", Style::default().fg(LAV)),
            Span::styled(" · ", dim()),
            Span::styled("Trusted", bold(AMBER)),
            Span::styled(" · capability-capped — every call audited", dim()),
        ])),
        area,
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled("aivyx tools ", dim())]).right_aligned()),
        area,
    );
}

fn body(f: &mut Frame, area: Rect) {
    let cols = Layout::horizontal([Constraint::Percentage(52), Constraint::Percentage(48)])
        .spacing(1)
        .split(area);
    tool_list(f, cols[0]);
    detail(f, cols[1]);
}

fn tool_list(f: &mut Frame, area: Rect) {
    let lines: Vec<Line> = tools()
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let sel = i == SELECTED;
            let marker = if sel { Span::styled("▌ ", bold(AMBER)) } else { Span::styled("  ", dim()) };
            let name_style = if sel {
                bold(AMBER)
            } else if t.gov == Gov::Off {
                Style::default().fg(DIMMER)
            } else {
                Style::default().fg(FG)
            };
            let flag = match t.gov {
                Gov::Open => Span::styled("  ", dim()),
                Gov::Gated => Span::styled("⚑ ", bold(AMBER)),
                Gov::Off => Span::styled("⛔", Style::default().fg(ERR)),
            };
            Line::from(vec![
                marker,
                Span::styled(format!("{:<19}", t.name), name_style),
                flag,
                Span::styled(format!("{:<10}", t.provenance), prov_style(t.provenance)),
                Span::styled(format!("· {}", t.stats), Style::default().fg(DIMMER)),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(panel("REGISTERED TOOLS", true)), area);
}

fn detail(f: &mut Frame, area: Rect) {
    let parts = Layout::vertical([Constraint::Min(8), Constraint::Length(3)]).split(area);

    let kv = |k: &'static str, v: Span<'static>| {
        Line::from(vec![Span::styled(format!("{k:<11}"), dim()), v])
    };
    let lines = vec![
        Line::from(vec![
            Span::styled("web.search", bold(FG)),
            Span::styled("   toolkit · ", dim()),
            Span::styled("callable", Style::default().fg(OK)),
        ]),
        Line::from(Span::styled(
            "Search the web; returns ranked, titled results.",
            Style::default().fg(DIM),
        )),
        Line::from(""),
        kv("input", Span::styled("{ query, max_results? }", Style::default().fg(FG))),
        kv("scope", Span::styled("web.search · Trusted · not gated", Style::default().fg(FG))),
        kv("calls", Span::styled("128 · 127 ok · 1 fail · last 09:14", Style::default().fg(FG))),
        Line::from(""),
        Line::from(Span::styled("RECENT", dim())),
        Line::from(vec![
            Span::styled("09:14  ", Style::default().fg(DIMMER)),
            Span::styled("ok ", Style::default().fg(OK)),
            Span::styled("\"market sentiment 2026-06-07\" → 5", Style::default().fg(DIM)),
        ]),
        Line::from(vec![
            Span::styled("08:51  ", Style::default().fg(DIMMER)),
            Span::styled("ok ", Style::default().fg(OK)),
            Span::styled("\"fed rate decision\" → 5", Style::default().fg(DIM)),
        ]),
    ];
    f.render_widget(
        Paragraph::new(lines).block(panel("web.search · DETAILS", false)),
        parts[0],
    );

    let action_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .padding(Padding::horizontal(1))
        .style(Style::default().bg(BG));
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[ inspect audit ]", bold(AMBER)),
            Span::styled("   [ disable for role ]", Style::default().fg(FG)),
        ]))
        .block(action_block),
        parts[1],
    );
}

fn status_bar(f: &mut Frame, area: Rect) {
    let sep = || Span::styled(" · ", Style::default().fg(DIMMER));
    let left = Line::from(vec![
        Span::styled(" 14 registered", bold(AMBER)),
        Span::styled(" · ", dim()),
        Span::styled("2 gated", Style::default().fg(AMBER)),
        Span::styled(" · ", dim()),
        Span::styled("1 off", Style::default().fg(DIM)),
    ]);
    let right = Line::from(vec![
        Span::styled("↑↓ select", dim()),
        sep(),
        Span::styled("Enter inspect", dim()),
        sep(),
        Span::styled("d disable", dim()),
        sep(),
        Span::styled("/ filter", dim()),
        sep(),
        Span::styled("^Q quit ", dim()),
    ])
    .right_aligned();
    f.render_widget(Paragraph::new(left).style(Style::default().bg(STATUS_BG)), area);
    f.render_widget(Paragraph::new(right).style(Style::default().bg(STATUS_BG)), area);
}

fn main() -> io::Result<()> {
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
