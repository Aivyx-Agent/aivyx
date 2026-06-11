//! `mcp_servers` — a ratatui translation of the "Plugin Registry" GUI
//! mockup, re-grounded in Aivyx's real MCP support.
//!
//! Aivyx connects to Model Context Protocol servers configured in
//! `aivyx.toml` (`[[mcp_servers]]`) — each a local stdio command or an
//! SSE URL that advertises tools, surfaced to the agent subject to the
//! active role's trust ceiling + gates. There's a curated catalog
//! behind `aivyx mcp recipes` (Phase 106). So this is an MCP-servers
//! registry — a master/detail list + the selected server's transport,
//! status, and advertised tools — not a generic "plugin store". No
//! agent roster, no "4.2k API calls / 12ms latency" vanity metrics; the
//! servers are real ecosystem ones, and the "locked" tool is *gated*.
//!
//! Run it live (truecolor, any key to exit):
//!     cargo run -p aivyx-tui --example mcp_servers
//! Or print the character grid:
//!     cargo run -p aivyx-tui --example mcp_servers -- --snapshot

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
const SELECTED: usize = 2; // postgres, detailed on the right

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

struct Server {
    running: bool,
    name: &'static str,
    transport: &'static str,
    tools: u8,
    /// stdio package / SSE URL shown in the list.
    origin: &'static str,
}

fn servers() -> Vec<Server> {
    vec![
        Server { running: true, name: "filesystem", transport: "stdio", tools: 5, origin: "@modelcontextprotocol/server-filesystem" },
        Server { running: true, name: "github", transport: "stdio", tools: 8, origin: "@modelcontextprotocol/server-github" },
        Server { running: true, name: "postgres", transport: "stdio", tools: 6, origin: "@modelcontextprotocol/server-postgres" },
        Server { running: true, name: "company-wiki", transport: "sse", tools: 3, origin: "https://wiki.internal/mcp" },
        Server { running: false, name: "brave-search", transport: "stdio", tools: 2, origin: "@modelcontextprotocol/server-brave-search" },
    ]
}

fn draw(f: &mut Frame) {
    let area = f.area();
    f.render_widget(Block::new().style(Style::default().bg(BG)), area);

    let rows = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Length(1), // actions
        Constraint::Length(1), // spacer
        Constraint::Min(0),    // body
        Constraint::Length(1), // status
    ])
    .split(area);

    header(f, rows[0]);
    actions(f, rows[1]);
    body(f, rows[3]);
    status_bar(f, rows[4]);
}

fn header(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("▌", bold(AMBER)),
            Span::styled(" AIVYX", bold(AMBER)),
            Span::styled("  MCP Servers", bold(FG)),
        ])),
        area,
    );
    f.render_widget(
        Paragraph::new(
            Line::from(vec![
                Span::styled("4 connected", Style::default().fg(OK)),
                Span::styled("  ·  ", dim()),
                Span::styled("1 stopped", Style::default().fg(DIM)),
            ])
            .right_aligned(),
        ),
        area,
    );
}

fn actions(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" [ + add server ]", bold(AMBER)),
            Span::styled("   ·   browse the catalog: ", dim()),
            Span::styled("aivyx mcp recipes", Style::default().fg(LAV)),
        ])),
        area,
    );
    f.render_widget(
        Paragraph::new(
            Line::from(vec![Span::styled("[[mcp_servers]] in aivyx.toml ", dim())]).right_aligned(),
        ),
        area,
    );
}

fn body(f: &mut Frame, area: Rect) {
    let cols = Layout::horizontal([Constraint::Percentage(52), Constraint::Percentage(48)])
        .spacing(1)
        .split(area);
    server_list(f, cols[0]);
    detail(f, cols[1]);
}

fn server_list(f: &mut Frame, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();
    for (i, s) in servers().iter().enumerate() {
        let sel = i == SELECTED;
        let marker = if sel { Span::styled("▌ ", bold(AMBER)) } else { Span::styled("  ", dim()) };
        let status = if s.running {
            Span::styled("● running", bold(OK))
        } else {
            Span::styled("○ stopped", Style::default().fg(DIMMER))
        };
        let name_style = if sel {
            bold(AMBER)
        } else if s.running {
            bold(FG)
        } else {
            Style::default().fg(DIM)
        };
        lines.push(Line::from(vec![
            marker,
            status,
            Span::styled(format!("  {:<13}", s.name), name_style),
            Span::styled(format!("{} · {} tools", s.transport, s.tools), Style::default().fg(DIM)),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    ", dim()),
            Span::styled(s.origin, Style::default().fg(DIMMER)),
        ]));
        lines.push(Line::from(""));
    }
    f.render_widget(Paragraph::new(lines).block(panel("SERVERS", true)), area);
}

fn detail(f: &mut Frame, area: Rect) {
    let parts = Layout::vertical([Constraint::Min(8), Constraint::Length(3)]).split(area);

    // (tool, description, gated)
    let tools: [(&str, &str, bool); 4] = [
        ("query", "read-only SQL query", false),
        ("list_tables", "tables in the public schema", false),
        ("describe_table", "column types + foreign keys", false),
        ("execute", "mutating SQL", true),
    ];
    let mut lines = vec![
        Line::from(vec![
            Span::styled("postgres", bold(FG)),
            Span::styled("   stdio · ", dim()),
            Span::styled("● connected", Style::default().fg(OK)),
        ]),
        Line::from(Span::styled(
            "npx -y @modelcontextprotocol/server-postgres",
            Style::default().fg(DIMMER),
        )),
        Line::from(""),
        Line::from(Span::styled("AVAILABLE TOOLS  ·  6", dim())),
    ];
    for (name, desc, gated) in tools {
        let mut spans = vec![Span::styled(format!("{name:<15}"), Style::default().fg(AMBER))];
        if gated {
            spans[0] = Span::styled(format!("{name:<15}"), Style::default().fg(FG));
            spans.push(Span::styled("⚑ ", bold(AMBER)));
            spans.push(Span::styled(format!("{desc} — gated"), Style::default().fg(DIM)));
        } else {
            spans.push(Span::styled(desc, Style::default().fg(DIM)));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "surfaced as mcp.postgres.* · role-capped",
        Style::default().fg(DIMMER),
    )));
    lines.push(Line::from(Span::styled(
        "⚑ tools pause for your approval",
        Style::default().fg(DIMMER),
    )));
    f.render_widget(
        Paragraph::new(lines).block(panel("postgres · DETAILS", false)),
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
            Span::styled("[ configure ]", bold(AMBER)),
            Span::styled("   [ disable ]   ", Style::default().fg(FG)),
            Span::styled("edit aivyx.toml", dim()),
        ]))
        .block(action_block),
        parts[1],
    );
}

fn status_bar(f: &mut Frame, area: Rect) {
    let sep = || Span::styled(" · ", Style::default().fg(DIMMER));
    let left = Line::from(vec![
        Span::styled(" 5 servers", bold(AMBER)),
        Span::styled(" · ", dim()),
        Span::styled("24 tools", Style::default().fg(FG)),
        Span::styled(" · ", dim()),
        Span::styled("1 gated", Style::default().fg(AMBER)),
    ]);
    let right = Line::from(vec![
        Span::styled("↑↓ select", dim()),
        sep(),
        Span::styled("Space enable", dim()),
        sep(),
        Span::styled("a add", Style::default().fg(OK)),
        sep(),
        Span::styled("c configure", dim()),
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
