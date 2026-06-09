//! `missions` — a ratatui translation of the "Mission Orchestration"
//! GUI mockup, re-grounded in what Aivyx actually runs.
//!
//! Missions are a real Aivyx primitive (Phase 21): units of work the
//! **single** agent executes (optionally in a role). So this is a
//! master/detail missions view — a stream on the left, the selected
//! mission's step timeline on the right — not a multi-agent swarm
//! console. No swarms, no per-mission agent rosters, no
//! "98.4% efficiency / 400Gbps" vanity metrics; the content is real
//! assistant work, and the human-in-the-loop card is an approval gate.
//!
//! Run it live (truecolor, any key to exit):
//!     cargo run -p aivyx-tui --example missions
//! Or print the character grid:
//!     cargo run -p aivyx-tui --example missions -- --snapshot

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
const SELECTED: usize = 0; // the executing mission, detailed on the right

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

#[derive(Clone, Copy)]
enum State {
    Executing,
    Gate,
    Planning,
    Done,
}

struct Mission {
    state: State,
    id: &'static str,
    title: &'static str,
    desc: &'static str,
    role: &'static str,
    progress: u16,
}

fn missions() -> Vec<Mission> {
    vec![
        Mission {
            state: State::Executing,
            id: "m-8802",
            title: "Weekly market-sentiment digest",
            desc: "research the thread, draft a summary to ./reports",
            role: "researcher",
            progress: 65,
        },
        Mission {
            state: State::Gate,
            id: "m-9112",
            title: "Send the team update email",
            desc: "draft ready — needs approval to call gmail.send",
            role: "default",
            progress: 0,
        },
        Mission {
            state: State::Planning,
            id: "m-7409",
            title: "Reconcile next week's reminders",
            desc: "gather due reminders + calendar, propose a plan",
            role: "personal",
            progress: 0,
        },
        Mission {
            state: State::Done,
            id: "m-7301",
            title: "Summarize the API-docs PR",
            desc: "posted the summary to the PR thread",
            role: "coder",
            progress: 100,
        },
    ]
}

fn badge(state: State) -> Span<'static> {
    match state {
        State::Executing => Span::styled("● executing", bold(OK)),
        State::Gate => Span::styled("⚑ approval", bold(AMBER)),
        State::Planning => Span::styled("◦ planning", Style::default().fg(DIM)),
        State::Done => Span::styled("✓ done", Style::default().fg(DIMMER)),
    }
}

fn draw(f: &mut Frame) {
    let area = f.area();
    f.render_widget(Block::new().style(Style::default().bg(BG)), area);

    let rows = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Length(1), // filters + counts
        Constraint::Length(1), // spacer
        Constraint::Min(0),    // body
        Constraint::Length(1), // status
    ])
    .split(area);

    header(f, rows[0]);
    filters(f, rows[1]);
    body(f, rows[3]);
    status_bar(f, rows[4]);
}

fn header(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("▌", bold(AMBER)),
            Span::styled(" AIVYX", bold(AMBER)),
            Span::styled("  Missions", bold(FG)),
        ])),
        area,
    );
    f.render_widget(
        Paragraph::new(
            Line::from(vec![
                Span::styled("daemon ", dim()),
                Span::styled("●", Style::default().fg(OK)),
                Span::styled("  ·  agent: ", dim()),
                Span::styled("researcher", Style::default().fg(LAV)),
            ])
            .right_aligned(),
        ),
        area,
    );
}

fn filters(f: &mut Frame, area: Rect) {
    let tab = |t: &'static str, on: bool| {
        if on {
            Span::styled(format!(" {t} "), bold(AMBER))
        } else {
            Span::styled(format!(" {t} "), Style::default().fg(DIM))
        }
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![
            tab("All", true),
            tab("Executing", false),
            tab("Pending", false),
            tab("Completed", false),
        ])),
        area,
    );
    f.render_widget(
        Paragraph::new(
            Line::from(vec![
                Span::styled("3 active", Style::default().fg(FG)),
                Span::styled(" · ", dim()),
                Span::styled("1 awaiting approval ", bold(AMBER)),
            ])
            .right_aligned(),
        ),
        area,
    );
}

fn body(f: &mut Frame, area: Rect) {
    let cols = Layout::horizontal([Constraint::Percentage(56), Constraint::Percentage(44)])
        .spacing(1)
        .split(area);
    mission_stream(f, cols[0]);
    detail(f, cols[1]);
}

fn mission_stream(f: &mut Frame, area: Rect) {
    let mut lines: Vec<Line> = Vec::new();
    for (i, m) in missions().iter().enumerate() {
        let sel = i == SELECTED;
        let marker = if sel {
            Span::styled("▌ ", bold(AMBER))
        } else {
            Span::styled("  ", dim())
        };
        let title_style = if sel { bold(AMBER) } else { bold(FG) };

        lines.push(Line::from(vec![
            marker,
            badge(m.state),
            Span::styled(format!("  {}  ", m.id), Style::default().fg(DIMMER)),
            Span::styled(m.title, title_style),
        ]));
        lines.push(Line::from(vec![
            Span::styled("    ", dim()),
            Span::styled(m.desc, Style::default().fg(DIM)),
        ]));

        // State-specific meta line.
        let meta: Vec<Span> = match m.state {
            State::Executing => {
                let mut s = vec![Span::styled("    ", dim())];
                s.extend(bar(m.progress, 22));
                s.push(Span::styled(format!("  {}%  · ", m.progress), Style::default().fg(DIM)));
                s.push(Span::styled(m.role, Style::default().fg(LAV)));
                s
            }
            State::Gate => vec![
                Span::styled("    ", dim()),
                Span::styled("[ approve ]", bold(AMBER)),
                Span::styled("  [ view logs ]   · ", Style::default().fg(DIM)),
                Span::styled(m.role, Style::default().fg(LAV)),
            ],
            State::Planning => vec![
                Span::styled("    queued · ", Style::default().fg(DIM)),
                Span::styled(m.role, Style::default().fg(LAV)),
            ],
            State::Done => vec![
                Span::styled("    done 14:02 · ", Style::default().fg(DIMMER)),
                Span::styled(m.role, Style::default().fg(DIMMER)),
            ],
        };
        lines.push(Line::from(meta));
        lines.push(Line::from(""));
    }
    f.render_widget(Paragraph::new(lines).block(panel("MISSIONS", true)), area);
}

fn detail(f: &mut Frame, area: Rect) {
    let parts = Layout::vertical([Constraint::Min(8), Constraint::Length(5)]).split(area);

    // Step timeline of the selected (executing) mission.
    let step = |dot: Span<'static>, time: &'static str, title: &'static str, sub: &'static str, title_c: Color| {
        vec![
            Line::from(vec![
                dot,
                Span::styled(format!(" {time}  "), Style::default().fg(DIM)),
                Span::styled(title, bold(title_c)),
            ]),
            Line::from(vec![
                Span::styled("│  ", Style::default().fg(DIMMER)),
                Span::styled(sub, Style::default().fg(DIMMER)),
            ]),
        ]
    };
    let mut lines = Vec::new();
    lines.extend(step(
        Span::styled("●", Style::default().fg(OK)),
        "09:12:44",
        "web.search — 5 sources",
        "fused into the context",
        FG,
    ));
    lines.extend(step(
        Span::styled("●", Style::default().fg(OK)),
        "09:14:02",
        "memory.recall — 3 prior",
        "topics: markets",
        FG,
    ));
    lines.extend(step(
        Span::styled("◐", Style::default().fg(AMBER)),
        "09:15:30",
        "drafting digest → ./reports",
        "writing sentiment-2026-06-07.md",
        AMBER,
    ));
    lines.extend(step(
        Span::styled("○", Style::default().fg(DIMMER)),
        "waiting ",
        "fs.write — gated",
        "needs approval before writing",
        DIM,
    ));
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("ⓘ gate · ", Style::default().fg(LAV)),
        Span::styled("fs.write → ./reports", Style::default().fg(FG)),
    ]));
    lines.push(Line::from(Span::styled(
        "  Trusted role · gates on writes",
        Style::default().fg(DIMMER),
    )));
    f.render_widget(
        Paragraph::new(lines).block(panel("m-8802 · DETAILED BREAKDOWN", false)),
        parts[0],
    );

    // Real signals instead of "efficiency / swarm health".
    let widgets = Layout::horizontal([Constraint::Ratio(1, 2); 2])
        .spacing(1)
        .split(parts[1]);
    let w = |label: &'static str, value: Span<'static>| {
        Paragraph::new(Line::from(value)).block(panel(label, false))
    };
    f.render_widget(
        w("THIS TURN", Span::styled("1,820 tok · 4 tools", bold(AMBER))),
        widgets[0],
    );
    f.render_widget(
        w("AUDIT", Span::styled("6 events · chain ✓", bold(OK))),
        widgets[1],
    );
}

fn status_bar(f: &mut Frame, area: Rect) {
    let sep = || Span::styled(" · ", Style::default().fg(DIMMER));
    let left = Line::from(vec![
        Span::styled(" 3 active", bold(AMBER)),
        Span::styled(" · ", dim()),
        Span::styled("1 awaiting approval", Style::default().fg(AMBER)),
        Span::styled(" · ", dim()),
        Span::styled("1 done", Style::default().fg(DIM)),
    ]);
    let right = Line::from(vec![
        Span::styled("↑↓ select", dim()),
        sep(),
        Span::styled("Enter open", dim()),
        sep(),
        Span::styled("a approve", Style::default().fg(OK)),
        sep(),
        Span::styled("n new", dim()),
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
