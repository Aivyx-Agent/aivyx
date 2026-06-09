//! `mission_new` — a New Mission / task creation form in the Aivyx
//! palette (sibling to `missions`, which monitors them).
//!
//! Unlike the other examples this isn't a translation of a GUI mockup
//! — it's the *creation* surface designed from Aivyx's own primitives:
//! an objective + instructions, the **role** the single agent runs it
//! in (which fixes trust tier + capability ceiling + tool allowlist), a
//! **trigger** (now / scheduled / on a file-watch), and a **gate
//! policy**. The right pane is a live "will run as" summary + a plan
//! preview, so the operator sees exactly what they're authorizing
//! before pressing Create.
//!
//! Run it live (truecolor, any key to exit):
//!     cargo run -p aivyx-tui --example mission_new
//! Or print the character grid:
//!     cargo run -p aivyx-tui --example mission_new -- --snapshot

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

/// `(•)` selected radio (amber) / `( )` unselected (dim).
fn radio(on: bool, label: &str) -> Vec<Span<'static>> {
    if on {
        vec![
            Span::styled("(•) ", bold(AMBER)),
            Span::styled(label.to_string(), Style::default().fg(FG)),
        ]
    } else {
        vec![
            Span::styled("( ) ", Style::default().fg(DIMMER)),
            Span::styled(label.to_string(), Style::default().fg(DIM)),
        ]
    }
}

fn checkbox(on: bool, label: &str) -> Vec<Span<'static>> {
    let (b, bc) = if on { ("[x] ", AMBER) } else { ("[ ] ", DIMMER) };
    vec![
        Span::styled(b, bold(bc)),
        Span::styled(label.to_string(), Style::default().fg(if on { FG } else { DIM })),
    ]
}

fn draw(f: &mut Frame) {
    let area = f.area();
    f.render_widget(Block::new().style(Style::default().bg(BG)), area);

    let rows = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Length(1), // breadcrumb
        Constraint::Length(1), // spacer
        Constraint::Min(0),    // body
        Constraint::Length(1), // status
    ])
    .split(area);

    header(f, rows[0]);
    breadcrumb(f, rows[1]);
    body(f, rows[3]);
    status_bar(f, rows[4]);
}

fn header(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("▌", bold(AMBER)),
            Span::styled(" AIVYX", bold(AMBER)),
            Span::styled("  New Mission", bold(FG)),
        ])),
        area,
    );
    f.render_widget(
        Paragraph::new(
            Line::from(vec![
                Span::styled("●", Style::default().fg(AMBER)),
                Span::styled(" draft · not saved", Style::default().fg(AMBER)),
            ])
            .right_aligned(),
        ),
        area,
    );
}

fn breadcrumb(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Missions ", dim()),
            Span::styled("›", Style::default().fg(DIMMER)),
            Span::styled(" New", bold(AMBER)),
        ])),
        area,
    );
}

fn body(f: &mut Frame, area: Rect) {
    let cols = Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)])
        .spacing(1)
        .split(area);

    let left = Layout::vertical([
        Constraint::Length(3),  // objective
        Constraint::Length(8),  // details
        Constraint::Length(9),  // execution
        Constraint::Length(1),  // spacer
        Constraint::Length(1),  // actions
    ])
    .split(cols[0]);

    objective(f, left[0]);
    details(f, left[1]);
    execution(f, left[2]);
    actions(f, left[4]);
    summary(f, cols[1]);
}

fn objective(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Draft & send the weekly market-sentiment digest", Style::default().fg(FG)),
            Span::styled("▏", bold(AMBER)), // focused field cursor
        ]))
        .block(panel("OBJECTIVE", true)),
        area,
    );
}

fn details(f: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(Span::styled(
            "Pull from the markets thread and three prior analyses.",
            Style::default().fg(FG),
        )),
        Line::from(Span::styled(
            "Save the digest to ./reports, then email the team a",
            Style::default().fg(FG),
        )),
        Line::from(Span::styled("two-paragraph summary.", Style::default().fg(FG))),
        Line::from(""),
        Line::from(Span::styled(
            "tokens 88 / 8192   ·   the agent plans the steps",
            Style::default().fg(DIMMER),
        )),
    ];
    f.render_widget(Paragraph::new(lines).block(panel("INSTRUCTIONS", false)), area);
}

fn execution(f: &mut Frame, area: Rect) {
    let mut when = vec![Span::styled("when      ", dim())];
    when.extend(radio(true, "now   "));
    when.extend(radio(false, "schedule…   "));
    when.extend(radio(false, "on file change"));

    let mut approval = vec![Span::styled("approval  ", dim())];
    approval.extend(checkbox(true, "gate writes & sends   "));
    approval.extend(checkbox(false, "autonomous"));

    let lines = vec![
        Line::from(vec![
            Span::styled("role      ", dim()),
            Span::styled("❮ ", Style::default().fg(DIMMER)),
            Span::styled("researcher", bold(AMBER)),
            Span::styled(" ❯   ", Style::default().fg(DIMMER)),
            Span::styled("Trusted · scopes from role", Style::default().fg(DIM)),
        ]),
        Line::from(""),
        Line::from(when),
        Line::from(""),
        Line::from(approval),
        Line::from(""),
        Line::from(Span::styled(
            "          gates pause the mission for your ⚑ approval",
            Style::default().fg(DIMMER),
        )),
    ];
    f.render_widget(Paragraph::new(lines).block(panel("EXECUTION", false)), area);
}

fn actions(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" [ Create & run ] ", bold(BG).bg(AMBER)),
            Span::styled("   ", dim()),
            Span::styled("[ Save draft ]", Style::default().fg(FG)),
            Span::styled("   ", dim()),
            Span::styled("Cancel", Style::default().fg(DIM)),
        ])),
        area,
    );
}

fn summary(f: &mut Frame, area: Rect) {
    let kv = |k: &'static str, v: Span<'static>| {
        Line::from(vec![Span::styled(format!("{k:<10}"), dim()), v])
    };
    let lines = vec![
        kv("role", Span::styled("researcher · Trusted", Style::default().fg(FG))),
        kv("scopes", Span::styled("fs.* · web.search · memory.*", Style::default().fg(FG))),
        kv("tools", Span::styled("9 of 14 enabled", Style::default().fg(FG))),
        kv("trigger", Span::styled("now", Style::default().fg(FG))),
        kv("gates", Span::styled("writes · external sends", bold(AMBER))),
        Line::from(""),
        Line::from(Span::styled("PLAN PREVIEW", dim())),
        Line::from(vec![
            Span::styled("1  ", Style::default().fg(DIMMER)),
            Span::styled("web.search · memory.recall", Style::default().fg(FG)),
        ]),
        Line::from(vec![
            Span::styled("2  ", Style::default().fg(DIMMER)),
            Span::styled("draft → ./reports ", Style::default().fg(FG)),
            Span::styled("(fs.write ⚑)", Style::default().fg(AMBER)),
        ]),
        Line::from(vec![
            Span::styled("3  ", Style::default().fg(DIMMER)),
            Span::styled("email the team ", Style::default().fg(FG)),
            Span::styled("(gmail.send ⚑)", Style::default().fg(AMBER)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "⚑ = pauses for approval",
            Style::default().fg(DIMMER),
        )),
    ];
    f.render_widget(Paragraph::new(lines).block(panel("WILL RUN AS", false)), area);
}

fn status_bar(f: &mut Frame, area: Rect) {
    let sep = || Span::styled(" · ", Style::default().fg(DIMMER));
    let left = Line::from(vec![
        Span::styled(" new mission", bold(AMBER)),
        Span::styled(" · ", dim()),
        Span::styled("researcher", Style::default().fg(LAV)),
        Span::styled(" · ", dim()),
        Span::styled("● draft", Style::default().fg(AMBER)),
    ]);
    let right = Line::from(vec![
        Span::styled("Tab field", dim()),
        sep(),
        Span::styled("^S create & run", Style::default().fg(OK)),
        sep(),
        Span::styled("^D save draft", dim()),
        sep(),
        Span::styled("Esc cancel ", dim()),
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
