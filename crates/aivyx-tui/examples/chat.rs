//! `chat` — the Phase 185 chat surface in the Aivyx palette (a design
//! study, sibling to the `command_center` example).
//!
//! This is the surface that actually ships in `render.rs`: a scrollback
//! chat pane, a status bar, and an input line. Here it's dressed in the
//! same amber-on-near-black identity as the command-center mock and
//! shown mid-session, paused on an **approval gate** — the input line
//! becomes the approve/reject prompt, exactly as the real TUI does.
//!
//! Run it live (truecolor, any key to exit):
//!     cargo run -p aivyx-tui --example chat
//! Or print the character grid (no TTY needed):
//!     cargo run -p aivyx-tui --example chat -- --snapshot

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

// Aivyx palette — the canonical brand colors, shared with the shipped
// chat surface (`crate::palette`). One source of truth, no drift.
use aivyx_tui::palette::{AMBER, BG, DIM, DIMMER, FG, LAV, OK, STATUS_BG};

const W: u16 = 104;
const H: u16 = 36;

fn dim() -> Style {
    Style::default().fg(DIM)
}
fn bold(c: Color) -> Style {
    Style::default().fg(c).add_modifier(Modifier::BOLD)
}

// ---- chat-line kinds, mirroring crate::model::LineKind --------------
enum Kind {
    Operator,
    Agent,
    Tool,
    Status,
    Gate,
    Marker,
}

fn line(kind: Kind, text: &str) -> Line<'static> {
    match kind {
        Kind::Operator => Line::from(vec![
            Span::styled("❯ ", bold(AMBER)),
            Span::styled(text.to_string(), bold(FG)),
        ]),
        Kind::Agent => Line::from(Span::styled(text.to_string(), Style::default().fg(FG))),
        Kind::Tool => Line::from(Span::styled(text.to_string(), Style::default().fg(DIM))),
        Kind::Status => Line::from(Span::styled(
            text.to_string(),
            Style::default().fg(DIM).add_modifier(Modifier::ITALIC),
        )),
        Kind::Gate => Line::from(Span::styled(text.to_string(), bold(AMBER))),
        Kind::Marker => Line::from(Span::styled(text.to_string(), Style::default().fg(DIMMER))),
    }
}

fn draw(f: &mut Frame) {
    let area = f.area();
    f.render_widget(Block::new().style(Style::default().bg(BG)), area);

    let rows = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Min(1),    // chat pane
        Constraint::Length(1), // status bar
        Constraint::Length(3), // input / gate prompt
    ])
    .split(area);

    header(f, rows[0]);
    chat(f, rows[1]);
    status_bar(f, rows[2]);
    gate_input(f, rows[3]);
}

fn header(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("▌", bold(AMBER)),
            Span::styled(" AIVYX", bold(AMBER)),
            Span::styled("  Chat", bold(FG)),
        ])),
        area,
    );
    f.render_widget(
        Paragraph::new(
            Line::from(vec![
                Span::styled("researcher", Style::default().fg(LAV)),
                Span::styled("  ·  ", dim()),
                Span::styled("daemon ", dim()),
                Span::styled("●", Style::default().fg(OK)),
                Span::styled("  ·  ", dim()),
                Span::styled("loop ", dim()),
                Span::styled("●", Style::default().fg(OK)),
            ])
            .right_aligned(),
        ),
        area,
    );
}

fn chat(f: &mut Frame, area: Rect) {
    let mut lines = vec![
        line(Kind::Marker, "── session · researcher · started 14:02 ─────────────────────"),
        Line::from(""),
        line(Kind::Operator, "summarize today's market-sentiment thread and save it to ./reports"),
        Line::from(""),
        line(Kind::Status, "  ⋯ planning · web.search, memory.recall, fs.write"),
        line(Kind::Tool, "  → web.search    {\"q\":\"market sentiment 2026-06-07\"}"),
        line(Kind::Tool, "  ← web.search    5 results · 1,820 tokens"),
        line(Kind::Tool, "  → memory.recall {\"topic\":\"markets\"}"),
        line(Kind::Tool, "  ← memory.recall 3 memories fused"),
        Line::from(""),
        line(Kind::Agent, "Sentiment skews cautiously bullish — rate-cut optimism is"),
        line(Kind::Agent, "offsetting soft manufacturing prints. I pulled three prior"),
        line(Kind::Agent, "analyses from memory to keep the framing consistent."),
        Line::from(""),
        line(Kind::Tool, "  → fs.write       {\"path\":\"./reports/sentiment-2026-06-07.md\"}"),
        Line::from(""),
        line(Kind::Gate, "⚑ APPROVAL GATE — fs.write wants to create"),
        line(Kind::Gate, "   ./reports/sentiment-2026-06-07.md   (scope: ./reports)"),
    ];
    // Bottom-anchor: pad the top so the latest lines sit near the input,
    // the way a real scrollback reads.
    let inner = area.height.saturating_sub(0) as usize;
    if lines.len() < inner {
        let pad = inner - lines.len();
        let mut padded = vec![Line::from(""); pad];
        padded.append(&mut lines);
        lines = padded;
    }
    f.render_widget(
        Paragraph::new(lines).block(Block::new().padding(Padding::horizontal(1))),
        area,
    );
}

fn status_bar(f: &mut Frame, area: Rect) {
    let left = Line::from(vec![
        Span::styled(" researcher", bold(AMBER)),
        Span::styled(" · ", dim()),
        Span::styled("daemon ✓", Style::default().fg(OK)),
        Span::styled(" · ", dim()),
        Span::styled("⚑ approval needed", bold(AMBER)),
    ]);
    let right = Line::from(vec![
        Span::styled("y approve", Style::default().fg(OK)),
        Span::styled(" · ", Style::default().fg(DIMMER)),
        Span::styled("n reject", Style::default().fg(FG)),
        Span::styled(" · ", Style::default().fg(DIMMER)),
        Span::styled("^Q quit ", dim()),
    ])
    .right_aligned();
    f.render_widget(Paragraph::new(left).style(Style::default().bg(STATUS_BG)), area);
    f.render_widget(Paragraph::new(right).style(Style::default().bg(STATUS_BG)), area);
}

fn gate_input(f: &mut Frame, area: Rect) {
    // While a gate is pending the input line becomes the approve/reject
    // prompt — exactly the swap `render.rs` performs.
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(AMBER))
        .title(Span::styled(" Approval gate ", bold(AMBER)))
        .padding(Padding::horizontal(1))
        .style(Style::default().bg(BG));
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Approve fs.write → ", Style::default().fg(FG)),
            Span::styled("./reports/sentiment-2026-06-07.md", Style::default().fg(AMBER)),
            Span::styled("  ?  ", Style::default().fg(FG)),
            Span::styled("[y/n]", bold(AMBER)),
        ]))
        .block(block),
        area,
    );
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
