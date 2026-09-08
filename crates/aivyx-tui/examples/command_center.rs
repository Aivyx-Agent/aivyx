//! `command_center` — a ratatui translation of the "Command Center"
//! GUI mockup into an honest terminal UI (Phase 185 design study, a
//! seed for the Phase 186 state-panel view).
//!
//! It keeps the GUI's *identity* — amber-on-near-black, the mono
//! instrument feel, panel separation, the stat cells, the system
//! readout, and the audit-trail timeline — but drops everything a
//! terminal can't do (glass/blur, gradients, glows, icon fonts) and
//! re-grounds the *content* in what Aivyx actually is: one local
//! daemon, one agent, Persona/Roles/Skills/Loop/Memory, an
//! HMAC-verified audit chain. No clusters, no federation, no teams.
//!
//! Run it live (truecolor, any key to exit):
//!     cargo run -p aivyx-tui --example command_center
//! Or print the character grid (no TTY needed):
//!     cargo run -p aivyx-tui --example command_center -- --snapshot

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
use ratatui::widgets::{Block, BorderType, Paragraph};
use ratatui::{Frame, Terminal};

// Aivyx palette — the canonical brand colors, shared with the shipped
// chat surface (`crate::palette`). One source of truth, no drift.
use aivyx_tui::palette::{
    AMBER, BG, BORDER, DIM, DIMMER, ERR, FG, LAV, OK, PANEL, STATUS_BG,
};

const W: u16 = 104;
const H: u16 = 36;

fn dim() -> Style {
    Style::default().fg(DIM)
}
fn bold(c: Color) -> Style {
    Style::default().fg(c).add_modifier(Modifier::BOLD)
}

/// A panel: rounded border, dim outline, amber title, panel bg.
fn panel(title: &str) -> Block<'_> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(BORDER))
        .title(Span::styled(
            format!(" {title} "),
            bold(AMBER).add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(PANEL))
}

/// A text progress bar: `filled` amber, remainder faint, in `width`.
fn bar<'a>(pct: u16, width: usize) -> Vec<Span<'a>> {
    let filled = (pct as usize * width / 100).min(width);
    vec![
        Span::styled("█".repeat(filled), Style::default().fg(AMBER)),
        Span::styled("░".repeat(width - filled), Style::default().fg(DIMMER)),
    ]
}

fn draw(f: &mut Frame) {
    let area = f.area();
    f.render_widget(Block::new().style(Style::default().bg(BG)), area);

    let rows = Layout::vertical([
        Constraint::Length(1), // header
        Constraint::Length(1), // subtitle
        Constraint::Length(1), // spacer
        Constraint::Length(5), // stat cells
        Constraint::Length(11), // active mission + system
        Constraint::Min(7),    // audit trail
        Constraint::Length(1), // status bar
    ])
    .split(area);

    header(f, rows[0]);
    subtitle(f, rows[1]);
    stat_cells(f, rows[3]);
    mid(f, rows[4]);
    audit(f, rows[5]);
    status_bar(f, rows[6]);
}

fn header(f: &mut Frame, area: Rect) {
    let left = Line::from(vec![
        Span::styled("▌", bold(AMBER)),
        Span::styled(" AIVYX PA", bold(AMBER)),
        Span::styled("  Command Center", bold(FG)),
    ]);
    let right = Line::from(vec![
        Span::styled("ollama:llama3.1", Style::default().fg(LAV)),
        Span::styled("  ·  ", dim()),
        Span::styled("daemon ", dim()),
        Span::styled("●", Style::default().fg(OK)),
        Span::styled("  ·  ", dim()),
        Span::styled("audit ", dim()),
        Span::styled("✓ verified", Style::default().fg(OK)),
    ])
    .right_aligned();
    f.render_widget(Paragraph::new(left), area);
    f.render_widget(Paragraph::new(right), area);
}

fn subtitle(f: &mut Frame, area: Rect) {
    f.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Local · single daemon · ", dim()),
            Span::styled("all systems operational", Style::default().fg(DIM)),
            Span::styled("  — awaiting input.", dim()),
        ])),
        area,
    );
}

fn stat_cells(f: &mut Frame, area: Rect) {
    let cols = Layout::horizontal([Constraint::Ratio(1, 4); 4])
        .spacing(1)
        .split(area);
    let cells = [
        ("SESSIONS", "842", OK, "✓ persists on disk"),
        ("TURNS TODAY", "37", OK, "▲ +12% vs yesterday"),
        ("TOKEN COST", "$12.42", ERR, "! over daily budget"),
        ("TRUST TIER", "Trusted", AMBER, "⛨ ceiling enforced"),
    ];
    for (i, (label, value, sub_c, sub)) in cells.iter().enumerate() {
        let p = Paragraph::new(vec![
            Line::from(Span::styled(*label, dim())),
            Line::from(Span::styled(*value, bold(AMBER))),
            Line::from(Span::styled(*sub, Style::default().fg(*sub_c))),
        ])
        .block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(BORDER))
                .style(Style::default().bg(PANEL))
                .padding(ratatui::widgets::Padding::horizontal(1)),
        );
        f.render_widget(p, cols[i]);
    }
}

fn mid(f: &mut Frame, area: Rect) {
    let cols = Layout::horizontal([Constraint::Percentage(62), Constraint::Percentage(38)])
        .spacing(1)
        .split(area);

    // ---- Active work (mission + autonomous loop) ----
    let mission = vec![
        Line::from(vec![
            Span::styled("Mission  ", dim()),
            Span::styled("Market Sentiment Analysis", bold(FG)),
            Span::styled("   in progress", Style::default().fg(AMBER)),
        ]),
        Line::from(vec![
            Span::styled("role · researcher", dim()),
            Span::styled("        ", dim()),
        ]),
        Line::from({
            let mut s = vec![Span::styled("  ", dim())];
            s.extend(bar(65, 56));
            s.push(Span::styled("  65%", Style::default().fg(DIM)));
            s
        }),
        Line::from(""),
        Line::from(vec![
            Span::styled("Autonomous loop  ", dim()),
            Span::styled("●", Style::default().fg(OK)),
            Span::styled(" running", Style::default().fg(OK)),
            Span::styled("   iter 3/10 · 2 gates pending", dim()),
        ]),
        Line::from({
            let mut s = vec![Span::styled("  ", dim())];
            s.extend(bar(30, 56));
            s.push(Span::styled("  30%", Style::default().fg(DIM)));
            s
        }),
    ];
    f.render_widget(
        Paragraph::new(mission).block(panel("ACTIVE WORK").padding(
            ratatui::widgets::Padding::new(1, 1, 1, 0),
        )),
        cols[0],
    );

    // ---- System readout ----
    let kv = |k: &'static str, v: Span<'static>| {
        Line::from(vec![
            Span::styled(format!("{k:<11}"), dim()),
            v,
        ])
    };
    let mut sys = vec![
        kv("provider", Span::styled("ollama · llama3.1", Style::default().fg(LAV))),
        kv("role", Span::styled("researcher", Style::default().fg(FG))),
        kv("memory", Span::styled("12,482 objects", Style::default().fg(FG))),
        kv("persona Δ", Span::styled("14 deltas", Style::default().fg(FG))),
        kv("skills", Span::styled("6 learned", Style::default().fg(FG))),
        Line::from(""),
        Line::from(vec![
            Span::styled("context ", dim()),
            Span::styled("42% of window", Style::default().fg(DIM)),
        ]),
    ];
    let mut b = vec![Span::styled("", dim())];
    b.extend(bar(42, 28));
    sys.push(Line::from(b));
    f.render_widget(
        Paragraph::new(sys)
            .block(panel("SYSTEM").padding(ratatui::widgets::Padding::new(1, 1, 1, 0))),
        cols[1],
    );
}

fn audit(f: &mut Frame, area: Rect) {
    // Real Aivyx event kinds on the HMAC-chained audit log.
    let ev = |dot: Color, time: &'static str, title: &'static str, detail: &'static str| {
        vec![
            Line::from(vec![
                Span::styled("● ", Style::default().fg(dot)),
                Span::styled(time, Style::default().fg(DIM)),
                Span::styled("  ", dim()),
                Span::styled(title, bold(FG)),
            ]),
            Line::from(vec![
                Span::styled("│  ", Style::default().fg(DIMMER)),
                Span::styled(detail, Style::default().fg(DIMMER)),
            ]),
        ]
    };
    let mut lines = Vec::new();
    lines.extend(ev(AMBER, "14:22:04", "skills.teach — \"review a PR\" saved", "Persona delta appended · chain verified"));
    lines.extend(ev(LAV, "13:45:12", "gate resolved — fs.write approved", "mission Market-Sentiment · scope: ./reports"));
    lines.extend(ev(OK, "12:10:55", "recall — 3 memories fused into context", "topics: markets, prior-analysis"));
    lines.extend(ev(DIM, "11:58:30", "loop — iteration 2 complete", "backlog story closed · progress logged"));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "tail -f · 1,204 events · HMAC chain ✓ · ⌘? for full log",
        Style::default().fg(DIMMER),
    )));
    f.render_widget(
        Paragraph::new(lines)
            .block(panel("AUDIT TRAIL").padding(ratatui::widgets::Padding::new(1, 1, 1, 0))),
        area,
    );
}

fn status_bar(f: &mut Frame, area: Rect) {
    let left = Line::from(vec![
        Span::styled(" researcher", bold(AMBER)),
        Span::styled(" · ", dim()),
        Span::styled("daemon ✓", Style::default().fg(OK)),
        Span::styled(" · ", dim()),
        Span::styled("loop running", Style::default().fg(LAV)),
    ]);
    let sep = || Span::styled(" · ", Style::default().fg(DIMMER));
    let right = Line::from(vec![
        Span::styled("Tab panels", dim()),
        sep(),
        Span::styled("/ search", dim()),
        sep(),
        Span::styled("^Q quit ", dim()),
    ])
    .right_aligned();
    f.render_widget(
        Paragraph::new(left).style(Style::default().bg(STATUS_BG)),
        area,
    );
    f.render_widget(
        Paragraph::new(right).style(Style::default().bg(STATUS_BG)),
        area,
    );
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let snapshot =
        std::env::args().any(|a| a == "--snapshot") || !io::stdout().is_terminal();

    if snapshot {
        let mut term = Terminal::new(TestBackend::new(W, H))?;
        term.draw(draw)?;
        let buf = term.backend().buffer().clone();
        for y in 0..H {
            let mut line = String::new();
            for x in 0..W {
                line.push_str(buf[(x, y)].symbol());
            }
            println!("{}", line.trim_end());
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
