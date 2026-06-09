//! `memory` — the agent's memory browser, in the Aivyx palette.
//!
//! Aivyx keeps an encrypted local memory store (redb) organized by
//! topic, with semantic recall over embeddings and a recall-feedback
//! loop that learns which memories actually helped. This is the
//! `aivyx memory list / show / search` surface: a topics column, the
//! entries under the selected topic (the selected one expanded with
//! its metadata), and a search bar with a keyword↔semantic toggle +
//! recall stats. No "4.2TB indexed" — it's a local KB/MB store of
//! thousands of objects.
//!
//! Run it live (truecolor, any key to exit):
//!     cargo run -p aivyx-tui --example memory
//! Or print the character grid:
//!     cargo run -p aivyx-tui --example memory -- --snapshot

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
const SEL_TOPIC: usize = 0; // markets

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

fn draw(f: &mut Frame) {
    let area = f.area();
    f.render_widget(Block::new().style(Style::default().bg(BG)), area);

    let rows = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Length(1), // search bar
        Constraint::Length(1), // spacer
        Constraint::Min(0),    // body
        Constraint::Length(1), // status
    ])
    .split(area);

    header(f, rows[0]);
    search(f, rows[1]);
    body(f, rows[3]);
    status_bar(f, rows[4]);
}

fn header(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("▌", bold(AMBER)),
            Span::styled(" AIVYX", bold(AMBER)),
            Span::styled("  Memory", bold(FG)),
        ])),
        area,
    );
    f.render_widget(
        Paragraph::new(
            Line::from(vec![
                Span::styled("12,482 objects", Style::default().fg(FG)),
                Span::styled("  ·  38 topics  ·  ", dim()),
                Span::styled("embeddings ●", Style::default().fg(OK)),
            ])
            .right_aligned(),
        ),
        area,
    );
}

fn search(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" / ", bold(AMBER)),
            Span::styled("search memory…", Style::default().fg(DIMMER)),
            Span::styled("        [x] ", bold(AMBER)),
            Span::styled("semantic", Style::default().fg(FG)),
            Span::styled("  ( ) keyword", Style::default().fg(DIM)),
        ])),
        area,
    );
    f.render_widget(
        Paragraph::new(
            Line::from(vec![
                Span::styled("recall ", dim()),
                Span::styled("1,204 hits", Style::default().fg(FG)),
                Span::styled(" · ", dim()),
                Span::styled("86% kept ", Style::default().fg(OK)),
            ])
            .right_aligned(),
        ),
        area,
    );
}

fn body(f: &mut Frame, area: Rect) {
    let cols = Layout::horizontal([Constraint::Percentage(32), Constraint::Percentage(68)])
        .spacing(1)
        .split(area);
    topics(f, cols[0]);
    entries(f, cols[1]);
}

fn topics(f: &mut Frame, area: Rect) {
    let list = [
        ("markets", 142),
        ("prior-analysis", 38),
        ("preferences", 27),
        ("people", 19),
        ("projects", 64),
        ("facts", 88),
        ("reminders-notes", 12),
        ("style", 9),
    ];
    let lines: Vec<Line> = list
        .iter()
        .enumerate()
        .map(|(i, (name, n))| {
            let sel = i == SEL_TOPIC;
            let marker = if sel { Span::styled("▌ ", bold(AMBER)) } else { Span::styled("  ", dim()) };
            let ns = if sel { bold(AMBER) } else { Style::default().fg(FG) };
            Line::from(vec![
                marker,
                Span::styled(format!("{name:<17}"), ns),
                Span::styled(format!("{n:>4}"), Style::default().fg(DIMMER)),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(panel("TOPICS", true)), area);
}

fn entries(f: &mut Frame, area: Rect) {
    // Selected (expanded) entry — full text + metadata.
    let mut lines: Vec<Line> = vec![
        Line::from(vec![
            Span::styled("▌ ", bold(AMBER)),
            Span::styled("09:14 today", Style::default().fg(DIM)),
            Span::styled("  · turn · ", Style::default().fg(DIMMER)),
            Span::styled("recalled 12×", Style::default().fg(DIM)),
            Span::styled("  helpful 9/12", Style::default().fg(OK)),
        ]),
        Line::from(Span::styled(
            "  Tracks the market-sentiment thread weekly; prefers a",
            Style::default().fg(FG),
        )),
        Line::from(Span::styled(
            "  cautiously-bullish framing with cited primary sources.",
            Style::default().fg(FG),
        )),
        Line::from(Span::styled(
            "  embedding ✓ · key markets · id mem-4f2a",
            Style::default().fg(DIMMER),
        )),
        Line::from(""),
    ];

    // Collapsed entries — snippet only.
    let collapsed = [
        ("Jun 5", "reflection", "recalled 5×", "Fed rate-cut optimism is the dominant driver this cycle."),
        ("Jun 2", "user-stated", "recalled 2×", "Prefers concise two-paragraph summaries with a TL;DR."),
        ("May 28", "turn", "recalled 8×", "Watches L2 rotation as a leading risk-appetite signal."),
        ("May 21", "reflection", "recalled 1×", "Skeptical of single-source sentiment scores."),
    ];
    for (when, source, recall, text) in collapsed {
        lines.push(Line::from(vec![
            Span::styled(format!("  {when:<7}"), Style::default().fg(DIM)),
            Span::styled(format!("· {source} · "), Style::default().fg(DIMMER)),
            Span::styled(recall, Style::default().fg(DIMMER)),
        ]));
        lines.push(Line::from(Span::styled(
            format!("  {text}"),
            Style::default().fg(DIM),
        )));
        lines.push(Line::from(""));
    }

    f.render_widget(
        Paragraph::new(lines).block(panel("markets · 142 entries", false)),
        area,
    );
}

fn status_bar(f: &mut Frame, area: Rect) {
    let sep = || Span::styled(" · ", Style::default().fg(DIMMER));
    let left = Line::from(vec![
        Span::styled(" markets", bold(AMBER)),
        Span::styled(" · ", dim()),
        Span::styled("142 entries", Style::default().fg(FG)),
        Span::styled(" · ", dim()),
        Span::styled("semantic recall", Style::default().fg(LAV)),
    ]);
    let right = Line::from(vec![
        Span::styled("↑↓ entries", dim()),
        sep(),
        Span::styled("Tab topics", dim()),
        sep(),
        Span::styled("/ search", dim()),
        sep(),
        Span::styled("x evict", Style::default().fg(FG)),
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
