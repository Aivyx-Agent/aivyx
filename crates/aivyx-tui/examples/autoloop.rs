//! `autoloop` — the autonomous loop (the Aivyx "Ralph" loop), in the
//! Aivyx palette. (Named `autoloop` because `loop` is a Rust keyword.)
//!
//! Aivyx can run fully autonomously over an HMAC-chained backlog of
//! stories, re-arming itself each iteration until the backlog is empty
//! or a cap trips — with iteration / wall-clock / token caps and
//! **driver-side gate verification**, plus a cross-iteration progress
//! log. This is the `aivyx loop` surface: the backlog on the left, the
//! live run (current story, caps usage, gates, progress log) on the
//! right.
//!
//! Run it live (truecolor, any key to exit):
//!     cargo run -p aivyx-tui --example autoloop
//! Or print the character grid:
//!     cargo run -p aivyx-tui --example autoloop -- --snapshot

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

fn bar<'a>(pct: u16, width: usize) -> Vec<Span<'a>> {
    let filled = (pct as usize * width / 100).min(width);
    vec![
        Span::styled("█".repeat(filled), Style::default().fg(AMBER)),
        Span::styled("░".repeat(width - filled), Style::default().fg(DIMMER)),
    ]
}

enum St {
    Running,
    Queued,
    Done,
}

struct Story {
    st: St,
    title: &'static str,
    pri: &'static str,
}

fn backlog() -> Vec<Story> {
    use St::*;
    vec![
        Story { st: Running, title: "reconcile reminders for next week", pri: "p2" },
        Story { st: Queued, title: "draft the weekly market digest", pri: "p1" },
        Story { st: Queued, title: "summarize the API-docs PR", pri: "p3" },
        Story { st: Queued, title: "tidy the ./reports folder", pri: "p3" },
        Story { st: Done, title: "triage inbox flags", pri: "p2" },
    ]
}

fn badge(st: &St) -> Span<'static> {
    match st {
        St::Running => Span::styled("▶ running", bold(AMBER)),
        St::Queued => Span::styled("◦ queued ", Style::default().fg(DIM)),
        St::Done => Span::styled("✓ done   ", Style::default().fg(OK)),
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
    caps(f, rows[1]);
    body(f, rows[3]);
    status_bar(f, rows[4]);
}

fn header(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("▌", bold(AMBER)),
            Span::styled(" AIVYX", bold(AMBER)),
            Span::styled("  Autonomous Loop", bold(FG)),
        ])),
        area,
    );
    f.render_widget(
        Paragraph::new(
            Line::from(vec![
                Span::styled("● running", bold(OK)),
                Span::styled("  ·  iter 3/10", dim()),
            ])
            .right_aligned(),
        ),
        area,
    );
}

fn caps(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" caps ", dim()),
            Span::styled("10 iters · 30m · 50k tok", Style::default().fg(FG)),
            Span::styled("  ·  gates ", dim()),
            Span::styled("driver-verified", Style::default().fg(LAV)),
        ])),
        area,
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled("[ stop ] ", bold(AMBER))]).right_aligned()),
        area,
    );
}

fn body(f: &mut Frame, area: Rect) {
    let cols = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .spacing(1)
        .split(area);
    backlog_list(f, cols[0]);
    run(f, cols[1]);
}

fn backlog_list(f: &mut Frame, area: Rect) {
    let lines: Vec<Line> = backlog()
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let sel = i == SELECTED;
            let marker = if sel { Span::styled("▌ ", bold(AMBER)) } else { Span::styled("  ", dim()) };
            let ts = if sel {
                bold(AMBER)
            } else if matches!(s.st, St::Done) {
                Style::default().fg(DIMMER)
            } else {
                Style::default().fg(FG)
            };
            Line::from(vec![
                marker,
                badge(&s.st),
                Span::styled(format!("  {}", s.title), ts),
                Span::styled(format!("  {}", s.pri), Style::default().fg(DIMMER)),
            ])
        })
        .collect();
    f.render_widget(Paragraph::new(lines).block(panel("BACKLOG", true)), area);
}

fn run(f: &mut Frame, area: Rect) {
    // A labelled progress bar: `label  ████░░  suffix`.
    let cap = |label: &'static str, pct: u16, suffix: &'static str| {
        let mut spans = vec![Span::styled(format!("{label:<9}"), dim())];
        spans.extend(bar(pct, 18));
        spans.push(Span::styled(format!(" {suffix}"), Style::default().fg(DIM)));
        Line::from(spans)
    };

    let mut lines = vec![
        Line::from(vec![
            Span::styled(format!("{:<9}", "story"), dim()),
            Span::styled("reconcile reminders", Style::default().fg(FG)),
        ]),
        cap("iters", 30, "3/10"),
        cap("time", 40, "12/30m"),
        cap("tokens", 36, "18k/50k"),
        Line::from(vec![
            Span::styled(format!("{:<9}", "gates"), dim()),
            Span::styled("2 pending · verified", bold(AMBER)),
        ]),
        Line::from(""),
        Line::from(Span::styled("PROGRESS LOG", dim())),
    ];
    for (it, msg) in [
        ("iter 3", "pulled reminders + calendar"),
        ("iter 2", "closed: triage inbox flags"),
        ("iter 1", "planned the backlog order"),
    ] {
        lines.push(Line::from(vec![
            Span::styled(format!("{it}  "), Style::default().fg(DIMMER)),
            Span::styled(msg, Style::default().fg(DIM)),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "re-arms until backlog empties or a cap trips",
        Style::default().fg(DIMMER),
    )));
    f.render_widget(Paragraph::new(lines).block(panel("RUN", false)), area);
}

fn status_bar(f: &mut Frame, area: Rect) {
    let sep = || Span::styled(" · ", Style::default().fg(DIMMER));
    let left = Line::from(vec![
        Span::styled(" running", bold(AMBER)),
        Span::styled(" · ", dim()),
        Span::styled("iter 3/10", Style::default().fg(FG)),
        Span::styled(" · ", dim()),
        Span::styled("4 in backlog", Style::default().fg(LAV)),
    ]);
    let right = Line::from(vec![
        Span::styled("s stop", Style::default().fg(AMBER)),
        sep(),
        Span::styled("a add story", dim()),
        sep(),
        Span::styled("k skip", dim()),
        sep(),
        Span::styled("l log", dim()),
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
