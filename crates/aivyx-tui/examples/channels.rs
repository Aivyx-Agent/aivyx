//! `channels` — the agent's reach surfaces, in the Aivyx palette.
//!
//! Channels are how the operator talks to the one agent: the remote
//! messaging adapters (Telegram, Discord, Slack), the local Voice
//! channel, and the local surfaces (Terminal, localhost Web UI). The
//! load-bearing Aivyx detail: **every channel carries a trust tier**
//! that intersects the capability ceiling on each turn — a local
//! channel is `Trusted`, a messaging channel is `SemiTrusted` (so
//! `shell.exec` is stripped and writes gate). Remote reach is the only
//! networked path; local-first is preserved.
//!
//! Run it live (truecolor, any key to exit):
//!     cargo run -p aivyx-tui --example channels
//! Or print the character grid:
//!     cargo run -p aivyx-tui --example channels -- --snapshot

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
const SELECTED: usize = 0; // Telegram, detailed on the right

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

struct Channel {
    connected: bool,
    name: &'static str,
    tier: &'static str,
    note: &'static str,
}

fn channels() -> Vec<Channel> {
    vec![
        Channel { connected: true, name: "Telegram", tier: "SemiTrusted", note: "142 msgs today" },
        Channel { connected: true, name: "Discord", tier: "SemiTrusted", note: "38 msgs" },
        Channel { connected: false, name: "Slack", tier: "SemiTrusted", note: "stubbed — not live" },
        Channel { connected: true, name: "Voice", tier: "Trusted", note: "local mic + TTS" },
        Channel { connected: true, name: "Web UI", tier: "Trusted", note: "localhost:7843" },
        Channel { connected: true, name: "Terminal", tier: "Trusted", note: "this session" },
    ]
}

fn tier_style(tier: &str) -> Style {
    match tier {
        "Trusted" => Style::default().fg(OK),
        "SemiTrusted" => Style::default().fg(LAV),
        _ => Style::default().fg(DIM),
    }
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
            Span::styled("  Channels", bold(FG)),
        ])),
        area,
    );
    f.render_widget(
        Paragraph::new(
            Line::from(vec![
                Span::styled("5 connected", Style::default().fg(OK)),
                Span::styled("  ·  ", dim()),
                Span::styled("3 local", Style::default().fg(FG)),
                Span::styled("  ·  ", dim()),
                Span::styled("2 remote", Style::default().fg(LAV)),
            ])
            .right_aligned(),
        ),
        area,
    );
}

fn actions(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" [ + connect channel ]", bold(AMBER)),
            Span::styled("   ·   remote reach is the only networked path", dim()),
        ])),
        area,
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled("[[channels]] in aivyx.toml ", dim())]).right_aligned()),
        area,
    );
}

fn body(f: &mut Frame, area: Rect) {
    let cols = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .spacing(1)
        .split(area);
    channel_list(f, cols[0]);
    detail(f, cols[1]);
}

fn channel_list(f: &mut Frame, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();
    for (i, c) in channels().iter().enumerate() {
        let sel = i == SELECTED;
        let marker = if sel { Span::styled("▌ ", bold(AMBER)) } else { Span::styled("  ", dim()) };
        let status = if c.connected {
            Span::styled("● ", Style::default().fg(OK))
        } else {
            Span::styled("○ ", Style::default().fg(DIMMER))
        };
        let ns = if sel {
            bold(AMBER)
        } else if c.connected {
            bold(FG)
        } else {
            Style::default().fg(DIM)
        };
        lines.push(Line::from(vec![
            marker,
            status,
            Span::styled(format!("{:<10}", c.name), ns),
            Span::styled(format!("{:<12}", c.tier), tier_style(c.tier)),
            Span::styled(format!("· {}", c.note), Style::default().fg(DIMMER)),
        ]));
    }
    f.render_widget(Paragraph::new(lines).block(panel("CHANNELS", true)), area);
}

fn detail(f: &mut Frame, area: Rect) {
    let parts = Layout::vertical([Constraint::Min(8), Constraint::Length(3)]).split(area);

    let kv = |k: &'static str, v: Span<'static>| {
        Line::from(vec![Span::styled(format!("{k:<10}"), dim()), v])
    };
    let lines = vec![
        Line::from(vec![
            Span::styled("Telegram", bold(FG)),
            Span::styled("   ● connected", Style::default().fg(OK)),
        ]),
        Line::from(Span::styled("@aivyx_bot · token from store", Style::default().fg(DIMMER))),
        Line::from(""),
        kv("trust", Span::styled("SemiTrusted", Style::default().fg(LAV))),
        kv("ceiling", Span::styled("shell.exec stripped · writes gated", Style::default().fg(FG))),
        kv("reach", Span::styled("remote — networked surface", Style::default().fg(FG))),
        Line::from(""),
        Line::from(Span::styled("RECENT", dim())),
        Line::from(vec![
            Span::styled("09:14  ", Style::default().fg(DIMMER)),
            Span::styled("you   ", Style::default().fg(LAV)),
            Span::styled("what's on my plate today?", Style::default().fg(DIM)),
        ]),
        Line::from(vec![
            Span::styled("09:14  ", Style::default().fg(DIMMER)),
            Span::styled("aivyx ", Style::default().fg(AMBER)),
            Span::styled("3 reminders due today", Style::default().fg(DIM)),
        ]),
        Line::from(vec![
            Span::styled("08:50  ", Style::default().fg(DIMMER)),
            Span::styled("you   ", Style::default().fg(LAV)),
            Span::styled("remind me to call mom at 6", Style::default().fg(DIM)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "SemiTrusted intersects the ceiling every turn",
            Style::default().fg(DIMMER),
        )),
    ];
    f.render_widget(
        Paragraph::new(lines).block(panel("Telegram · DETAILS", false)),
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
            Span::styled("   [ disconnect ]   ", Style::default().fg(FG)),
            Span::styled("[ logs ]", dim()),
        ]))
        .block(action_block),
        parts[1],
    );
}

fn status_bar(f: &mut Frame, area: Rect) {
    let sep = || Span::styled(" · ", Style::default().fg(DIMMER));
    let left = Line::from(vec![
        Span::styled(" Telegram", bold(AMBER)),
        Span::styled(" · ", dim()),
        Span::styled("SemiTrusted", Style::default().fg(LAV)),
        Span::styled(" · ", dim()),
        Span::styled("ceiling reduced", Style::default().fg(AMBER)),
    ]);
    let right = Line::from(vec![
        Span::styled("↑↓ select", dim()),
        sep(),
        Span::styled("c configure", dim()),
        sep(),
        Span::styled("a add", Style::default().fg(OK)),
        sep(),
        Span::styled("d disconnect", dim()),
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
