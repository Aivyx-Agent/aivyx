//! `audit` — the tamper-evident audit log, in the Aivyx palette.
//!
//! Every consequential action the agent takes is appended to an
//! HMAC-SHA256 hash chain over canonical JSON (RFC 8785 / JCS), so the
//! log is **offline-verifiable** and tamper-evident — a headline Aivyx
//! property and the payoff of the `⚑ / chain ✓` motif running through
//! the other screens. This is the `aivyx audit` surface: a left event
//! stream, and on the right the selected event's full record plus its
//! place in the chain (its HMAC, the previous link, verification).
//!
//! Run it live (truecolor, any key to exit):
//!     cargo run -p aivyx-tui --example audit
//! Or print the character grid:
//!     cargo run -p aivyx-tui --example audit -- --snapshot

use std::io::{self, IsTerminal};
use std::time::Duration;

use crossterm::event::{self, Event as CtEvent};
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
const SELECTED: usize = 0; // gate.resolved, detailed on the right

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

struct Ev {
    time: &'static str,
    kind: &'static str,
    summary: &'static str,
}

fn events() -> Vec<Ev> {
    vec![
        Ev { time: "09:15:30", kind: "gate.resolved", summary: "fs.write → ./reports ✓" },
        Ev { time: "09:15:12", kind: "gate.opened", summary: "fs.write needs approval" },
        Ev { time: "09:14:48", kind: "tool.finished", summary: "web.search · 5 results" },
        Ev { time: "09:14:44", kind: "tool.started", summary: "web.search {q: market…}" },
        Ev { time: "09:12:00", kind: "skill.learned", summary: "\"review a PR\" · persona Δ" },
        Ev { time: "09:05:33", kind: "role.import", summary: "researcher from proposal" },
        Ev { time: "08:50:10", kind: "memory.write", summary: "topic markets · +1 entry" },
        Ev { time: "08:41:02", kind: "notify.sent", summary: "Telegram · digest" },
        Ev { time: "08:30:00", kind: "daemon.start", summary: "v0.1.0 · chain resumed" },
    ]
}

fn kind_style(kind: &str) -> Style {
    if kind.starts_with("gate") {
        bold(AMBER)
    } else if kind.starts_with("skill") || kind.starts_with("role") || kind.starts_with("persona") {
        Style::default().fg(LAV)
    } else if kind.starts_with("daemon") {
        Style::default().fg(OK)
    } else {
        Style::default().fg(DIM)
    }
}

fn draw(f: &mut Frame) {
    let area = f.area();
    f.render_widget(Block::new().style(Style::default().bg(BG)), area);

    let rows = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Length(1), // verify line
        Constraint::Length(1), // spacer
        Constraint::Min(0),    // body
        Constraint::Length(1), // status
    ])
    .split(area);

    header(f, rows[0]);
    verify_line(f, rows[1]);
    body(f, rows[3]);
    status_bar(f, rows[4]);
}

fn header(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("▌", bold(AMBER)),
            Span::styled(" AIVYX", bold(AMBER)),
            Span::styled("  Audit Log", bold(FG)),
        ])),
        area,
    );
    f.render_widget(
        Paragraph::new(
            Line::from(vec![
                Span::styled("1,204 events", Style::default().fg(FG)),
                Span::styled("  ·  ", dim()),
                Span::styled("chain ✓ verified", bold(OK)),
            ])
            .right_aligned(),
        ),
        area,
    );
}

fn verify_line(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" verify ", dim()),
            Span::styled("✓ offline", Style::default().fg(OK)),
            Span::styled("  ·  HMAC-SHA256 over canonical JSON (RFC 8785)", dim()),
        ])),
        area,
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled("[ export jsonl ] ", bold(AMBER))]).right_aligned()),
        area,
    );
}

fn body(f: &mut Frame, area: Rect) {
    let cols = Layout::horizontal([Constraint::Percentage(54), Constraint::Percentage(46)])
        .spacing(1)
        .split(area);
    stream(f, cols[0]);
    detail(f, cols[1]);
}

fn stream(f: &mut Frame, area: Rect) {
    let lines: Vec<Line> = events()
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let sel = i == SELECTED;
            let marker = if sel { Span::styled("▌ ", bold(AMBER)) } else { Span::styled("  ", dim()) };
            Line::from(vec![
                marker,
                Span::styled(format!("{} ", e.time), Style::default().fg(DIMMER)),
                Span::styled(format!("{:<15}", e.kind), kind_style(e.kind)),
                Span::styled(e.summary, Style::default().fg(if sel { FG } else { DIM })),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(panel("EVENT STREAM", true)), area);
}

fn detail(f: &mut Frame, area: Rect) {
    let parts = Layout::vertical([Constraint::Min(8), Constraint::Length(3)]).split(area);

    let kv = |k: &'static str, v: Span<'static>| {
        Line::from(vec![Span::styled(format!("{k:<9}"), dim()), v])
    };
    let lines = vec![
        Line::from(Span::styled("gate.resolved", bold(AMBER))),
        kv("time", Span::styled("2026-06-07 09:15:30.142", Style::default().fg(FG))),
        kv("mission", Span::styled("m-8802", Style::default().fg(FG))),
        kv("scope", Span::styled("fs.write → ./reports", Style::default().fg(FG))),
        kv("actor", Span::styled("researcher · Trusted · Terminal", Style::default().fg(FG))),
        kv("outcome", Span::styled("approved by operator", Style::default().fg(OK))),
        Line::from(""),
        Line::from(Span::styled("CHAIN", dim())),
        kv("seq", Span::styled("#1204", Style::default().fg(FG))),
        kv("hmac", Span::styled("3f9a 71c0 … c21e", Style::default().fg(LAV))),
        kv("prev", Span::styled("a07b 9d33 … 44f1", Style::default().fg(DIMMER))),
        kv("verify", Span::styled("✓ valid · links #1203", bold(OK))),
    ];
    f.render_widget(
        Paragraph::new(lines).block(panel("EVENT #1204", false)),
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
            Span::styled("[ export ]", bold(AMBER)),
            Span::styled("   [ copy hash ]   ", Style::default().fg(FG)),
            Span::styled("✓ offline", Style::default().fg(OK)),
        ]))
        .block(action_block),
        parts[1],
    );
}

fn status_bar(f: &mut Frame, area: Rect) {
    let sep = || Span::styled(" · ", Style::default().fg(DIMMER));
    let left = Line::from(vec![
        Span::styled(" 1,204 events", bold(AMBER)),
        Span::styled(" · ", dim()),
        Span::styled("chain ✓", Style::default().fg(OK)),
        Span::styled(" · ", dim()),
        Span::styled("0 breaks", Style::default().fg(OK)),
    ]);
    let right = Line::from(vec![
        Span::styled("↑↓ select", dim()),
        sep(),
        Span::styled("v verify", Style::default().fg(OK)),
        sep(),
        Span::styled("e export", dim()),
        sep(),
        Span::styled("/ filter", dim()),
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
            if let Ok(CtEvent::Key(_)) = event::read() {
                break;
            }
        }
    }
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen)?;
    Ok(())
}
