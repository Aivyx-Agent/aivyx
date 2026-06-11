//! `genesis` — the first-run setup wizard, in the Aivyx palette.
//!
//! Aivyx is **one** agent — Genesis isn't minting one of many, it's the
//! guided first launch where the End User shapes *the* assistant: its
//! identity (Profile, P13), a Persona seed (P14, which then grows from
//! use), a default role + trust posture, and a provider. It's the
//! `aivyx init` wizard + the Phase 66 templates + the Phase 181 guided
//! identity builder (optionally LLM-assisted — describe it in words and
//! Aivyx drafts the rest). A stepper on top, the current step's form on
//! the left, and a live "your assistant" preview on the right.
//!
//! Run it live (truecolor, any key to exit):
//!     cargo run -p aivyx-tui --example genesis
//! Or print the character grid:
//!     cargo run -p aivyx-tui --example genesis -- --snapshot

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

fn draw(f: &mut Frame) {
    let area = f.area();
    f.render_widget(Block::new().style(Style::default().bg(BG)), area);

    let rows = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Length(1), // stepper
        Constraint::Length(1), // spacer
        Constraint::Min(0),    // body
        Constraint::Length(1), // actions
        Constraint::Length(1), // status
    ])
    .split(area);

    header(f, rows[0]);
    stepper(f, rows[1]);
    body(f, rows[3]);
    actions(f, rows[4]);
    status_bar(f, rows[5]);
}

fn header(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("▌", bold(AMBER)),
            Span::styled(" AIVYX", bold(AMBER)),
            Span::styled("  Genesis", bold(FG)),
            Span::styled("   set up your assistant", dim()),
        ])),
        area,
    );
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled("step 2 of 6 ", bold(AMBER))]).right_aligned()),
        area,
    );
}

fn stepper(f: &mut Frame, area: Rect) {
    // (label, state) — done / current / future
    let steps = [
        ("Template", 2u8),
        ("Identity", 1),
        ("Role", 0),
        ("Provider", 0),
        ("Store", 0),
        ("Review", 0),
    ];
    let mut spans = vec![Span::styled(" ", dim())];
    for (label, state) in steps {
        let (glyph, st) = match state {
            2 => ("✓ ", Style::default().fg(OK)),
            1 => ("● ", bold(AMBER)),
            _ => ("○ ", Style::default().fg(DIMMER)),
        };
        spans.push(Span::styled(glyph, st));
        spans.push(Span::styled(format!("{label}   "), st));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn body(f: &mut Frame, area: Rect) {
    let cols = Layout::horizontal([Constraint::Percentage(58), Constraint::Percentage(42)])
        .spacing(1)
        .split(area);
    identity(f, cols[0]);
    preview(f, cols[1]);
}

fn identity(f: &mut Frame, area: Rect) {
    let chip = |t: &'static str| {
        vec![
            Span::styled("‹", Style::default().fg(DIMMER)),
            Span::styled(t, Style::default().fg(LAV)),
            Span::styled("› ", Style::default().fg(DIMMER)),
        ]
    };
    let mut traits = vec![Span::styled("traits     ", dim())];
    for t in ["Analytical", "Warm", "Concise"] {
        traits.extend(chip(t));
    }
    traits.push(Span::styled("+ add", Style::default().fg(DIM)));

    let lines = vec![
        Line::from(vec![
            Span::styled("name       ", dim()),
            Span::styled("❮ ", Style::default().fg(DIMMER)),
            Span::styled("Aria", bold(AMBER)),
            Span::styled(" ❯", Style::default().fg(DIMMER)),
            Span::styled("▏", bold(AMBER)),
        ]),
        Line::from(vec![
            Span::styled("describe   ", dim()),
            Span::styled("A calm companion that cites its sources.", Style::default().fg(FG)),
        ]),
        Line::from(""),
        Line::from(traits),
        Line::from(""),
        Line::from(vec![
            Span::styled("style      ", dim()),
            Span::styled("❮ ", Style::default().fg(DIMMER)),
            Span::styled("Concise & direct", bold(AMBER)),
            Span::styled(" ❯", Style::default().fg(DIMMER)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("✨ ", Style::default().fg(AMBER)),
            Span::styled("describe it in plain words — Aivyx drafts the rest", Style::default().fg(DIM)),
        ]),
        Line::from(Span::styled(
            "this becomes your Profile · the Persona grows from use",
            Style::default().fg(DIMMER),
        )),
    ];
    f.render_widget(Paragraph::new(lines).block(panel("IDENTITY", true)), area);
}

fn preview(f: &mut Frame, area: Rect) {
    let kv = |k: &'static str, v: Span<'static>| {
        Line::from(vec![Span::styled(format!("{k:<9}"), dim()), v])
    };
    let lines = vec![
        kv("name", Span::styled("Aria", bold(AMBER))),
        kv("from", Span::styled("Personal template", Style::default().fg(FG))),
        kv("role", Span::styled("assistant (default)", Style::default().fg(FG))),
        kv("trust", Span::styled("Trusted · gates writes", Style::default().fg(FG))),
        kv("provider", Span::styled("ollama · llama3.1", Style::default().fg(LAV))),
        kv("memory", Span::styled("fresh start", Style::default().fg(FG))),
        kv("audit", Span::styled("new chain", Style::default().fg(OK))),
        Line::from(""),
        Line::from(Span::styled(
            "change anything later in Settings.",
            Style::default().fg(DIMMER),
        )),
    ];
    f.render_widget(Paragraph::new(lines).block(panel("YOUR ASSISTANT", false)), area);
}

fn actions(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" [ ← Back ]", Style::default().fg(DIM)),
            Span::styled("    ", dim()),
            Span::styled("[ Continue → ]", bold(AMBER)),
            Span::styled("    finish to create the assistant + its encrypted store", dim()),
        ])),
        area,
    );
}

fn status_bar(f: &mut Frame, area: Rect) {
    let sep = || Span::styled(" · ", Style::default().fg(DIMMER));
    let left = Line::from(vec![
        Span::styled(" Genesis", bold(AMBER)),
        Span::styled(" · ", dim()),
        Span::styled("step 2/6 · Identity", Style::default().fg(LAV)),
    ]);
    let right = Line::from(vec![
        Span::styled("Tab field", dim()),
        sep(),
        Span::styled("Enter continue", Style::default().fg(OK)),
        sep(),
        Span::styled("← back", dim()),
        sep(),
        Span::styled("^Q cancel ", dim()),
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
