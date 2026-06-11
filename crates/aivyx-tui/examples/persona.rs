//! `persona` — the identity layer (Profile + grown Persona), in the
//! Aivyx palette.
//!
//! Aivyx's identity is two layers: the **Profile** you declare (P13)
//! and the **Persona** that grows from use (P14) — a chain of approved
//! deltas on the same HMAC log as the audit chain, every one
//! revertable. This screen shows the growth timeline on the left and
//! the **effective** merged persona on the right, with its Profile
//! provenance. It's the `aivyx persona` / `aivyx profile` surface and
//! the headline self-learning differentiator.
//!
//! Run it live (truecolor, any key to exit):
//!     cargo run -p aivyx-tui --example persona
//! Or print the character grid:
//!     cargo run -p aivyx-tui --example persona -- --snapshot

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
const SELECTED: usize = 0;

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

struct Delta {
    when: &'static str,
    kind: &'static str,
    summary: &'static str,
}

fn deltas() -> Vec<Delta> {
    vec![
        Delta { when: "Jun 7", kind: "learned", summary: "prefers cited primary sources" },
        Delta { when: "Jun 5", kind: "trait+", summary: "Cautious" },
        Delta { when: "Jun 5", kind: "skill", summary: "\"review a PR\"" },
        Delta { when: "Jun 2", kind: "pref", summary: "two-paragraph summaries" },
        Delta { when: "May 28", kind: "style", summary: "TL;DR first, then detail" },
        Delta { when: "May 21", kind: "trait+", summary: "Analytical" },
        Delta { when: "May 14", kind: "seed", summary: "Profile: \"research companion\"" },
    ]
}

fn kind_style(kind: &str) -> Style {
    match kind {
        "trait+" => Style::default().fg(LAV),
        "skill" => bold(AMBER),
        "seed" => Style::default().fg(OK),
        _ => Style::default().fg(DIM),
    }
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
            Span::styled(" AIVYX", bold(AMBER)),
            Span::styled("  Persona", bold(FG)),
        ])),
        area,
    );
    f.render_widget(
        Paragraph::new(
            Line::from(vec![
                Span::styled("14 deltas", Style::default().fg(FG)),
                Span::styled("  ·  ", dim()),
                Span::styled("chain ✓", bold(OK)),
            ])
            .right_aligned(),
        ),
        area,
    );
}

fn sub(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Profile ", dim()),
            Span::styled("you set", Style::default().fg(LAV)),
            Span::styled("  +  Persona ", dim()),
            Span::styled("grows from use", Style::default().fg(LAV)),
            Span::styled("  ·  every delta revertable", dim()),
        ])),
        area,
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled("[ export identity ] ", bold(AMBER))]).right_aligned()),
        area,
    );
}

fn body(f: &mut Frame, area: Rect) {
    let cols = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .spacing(1)
        .split(area);
    timeline(f, cols[0]);
    effective(f, cols[1]);
}

fn timeline(f: &mut Frame, area: Rect) {
    let lines: Vec<Line> = deltas()
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let sel = i == SELECTED;
            let marker = if sel { Span::styled("▌ ", bold(AMBER)) } else { Span::styled("  ", dim()) };
            Line::from(vec![
                marker,
                Span::styled(format!("{:<7}", d.when), Style::default().fg(DIMMER)),
                Span::styled(format!("{:<8}", d.kind), kind_style(d.kind)),
                Span::styled(d.summary, Style::default().fg(if sel { FG } else { DIM })),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(panel("PERSONA GROWTH", true)), area);
}

fn effective(f: &mut Frame, area: Rect) {
    let parts = Layout::vertical([Constraint::Min(8), Constraint::Length(3)]).split(area);

    let kv = |k: &'static str, v: Span<'static>| {
        Line::from(vec![Span::styled(format!("{k:<12}"), dim()), v])
    };
    let lines = vec![
        kv("traits", Span::styled("Analytical · Warm · Cautious", Style::default().fg(FG))),
        kv("preferences", Span::styled("cited sources · 2-para", Style::default().fg(FG))),
        kv("style", Span::styled("concise · TL;DR first", Style::default().fg(FG))),
        kv("skills", Span::styled("review a PR · weekly digest", bold(AMBER))),
        Line::from(""),
        Line::from(Span::styled("FROM PROFILE", dim())),
        kv("name", Span::styled("Aria", Style::default().fg(FG))),
        kv("role", Span::styled("\"research companion\"", Style::default().fg(FG))),
        Line::from(""),
        Line::from(Span::styled(
            "14 deltas on the HMAC chain · each revertable",
            Style::default().fg(DIMMER),
        )),
    ];
    f.render_widget(Paragraph::new(lines).block(panel("EFFECTIVE NOW", false)), parts[0]);

    let action_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .padding(Padding::horizontal(1))
        .style(Style::default().bg(BG));
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[ revert ]", bold(AMBER)),
            Span::styled("   [ proposals (2) ]   ", Style::default().fg(FG)),
            Span::styled("[ export ]", dim()),
        ]))
        .block(action_block),
        parts[1],
    );
}

fn status_bar(f: &mut Frame, area: Rect) {
    let sep = || Span::styled(" · ", Style::default().fg(DIMMER));
    let left = Line::from(vec![
        Span::styled(" Persona", bold(AMBER)),
        Span::styled(" · ", dim()),
        Span::styled("14 deltas", Style::default().fg(FG)),
        Span::styled(" · ", dim()),
        Span::styled("2 proposals pending", Style::default().fg(LAV)),
    ]);
    let right = Line::from(vec![
        Span::styled("↑↓ delta", dim()),
        sep(),
        Span::styled("r revert", Style::default().fg(FG)),
        sep(),
        Span::styled("p proposals", dim()),
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
