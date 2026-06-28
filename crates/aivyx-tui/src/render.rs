//! The ratatui render layer — Phase 185 Task 3.
//!
//! A pure function of [`AppState`] → terminal frame: a scrollable
//! chat pane on top, a one-line status bar, and a bordered input line
//! (which becomes an approve/reject prompt while an approval gate is
//! pending). The visual result is **operator-verified** (no terminal
//! in CI); the headless-testable pieces are the buffer smoke test
//! (via `TestBackend`) and the pure [`chat_scroll_offset`] math.
//!
//! Long chat lines are truncated at the right edge rather than
//! wrapped, which keeps the scroll arithmetic exact (one [`ChatLine`]
//! == one row). Wrapping + precise wrapped-scroll is a refinement for
//! a later Chapter I phase, alongside live token streaming.

use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Borders, Padding, Paragraph};
use ratatui::Frame;

use crate::model::{AppState, LineKind, MissionPhase, StepState, View};
use crate::palette::{self, bold, fg};

/// Compute the first visible chat row given the total line count, the
/// chat viewport height, and the scroll offset (`0` == pinned to the
/// bottom, larger == scrolled up toward older lines). Pure so the
/// scroll behaviour is unit-tested without a terminal.
pub fn chat_scroll_offset(total: usize, viewport: usize, scroll: usize) -> u16 {
    // The top row when pinned to the bottom: everything that doesn't
    // fit is above the viewport.
    let max_top = total.saturating_sub(viewport);
    // Scrolling up moves the top earlier, but never before line 0.
    let top = max_top.saturating_sub(scroll);
    top.min(u16::MAX as usize) as u16
}

/// The display prefix + base style for a line kind. The prefix keeps
/// provenance legible even where colour is unavailable (and is what
/// the headless smoke test can assert on).
fn kind_style(kind: LineKind) -> (&'static str, Style) {
    match kind {
        LineKind::Operator => ("❯ ", bold(palette::AMBER)),
        LineKind::Agent => ("", fg(palette::FG)),
        LineKind::Tool => ("  ", fg(palette::DIM)),
        LineKind::Status => (
            "  ",
            fg(palette::DIM).add_modifier(Modifier::ITALIC),
        ),
        LineKind::Gate => ("⚑ ", bold(palette::AMBER)),
        LineKind::System => ("· ", fg(palette::LAV)),
    }
}

/// Draw the whole UI for the current state.
pub fn render(frame: &mut Frame, state: &AppState) {
    // The near-black Aivyx canvas behind every pane.
    frame.render_widget(
        Block::new().style(Style::default().bg(palette::BG)),
        frame.area(),
    );

    // Tab bar on top of every view; the body below it.
    let outer = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(frame.area());
    render_tab_bar(frame, outer[0], state);
    let body = outer[1];

    match state.view {
        View::Chat => {
            let rows = Layout::vertical([
                Constraint::Min(1),    // chat pane
                Constraint::Length(1), // status bar
                Constraint::Length(3), // bordered input
            ])
            .split(body);
            render_chat(frame, rows[0], state);
            render_status(frame, rows[1], state);
            render_input(frame, rows[2], state);
        }
        View::Missions => {
            // A compose row appears while the operator is typing / starting a
            // new mission (Chapter L).
            let composing = state.mission_compose.is_some() || state.mission_starting;
            if composing {
                let rows = Layout::vertical([
                    Constraint::Min(1),    // master/detail body
                    Constraint::Length(3), // new-mission compose box
                    Constraint::Length(1), // status bar
                ])
                .split(body);
                render_missions(frame, rows[0], state);
                render_mission_compose(frame, rows[1], state);
                render_status(frame, rows[2], state);
            } else {
                let rows = Layout::vertical([
                    Constraint::Min(1),    // master/detail body
                    Constraint::Length(1), // status bar
                ])
                .split(body);
                render_missions(frame, rows[0], state);
                render_status(frame, rows[1], state);
            }
        }
        View::Dashboard | View::Audit | View::Tools => {
            let rows = Layout::vertical([
                Constraint::Min(1),    // panel
                Constraint::Length(1), // status bar
            ])
            .split(body);
            render_panel(frame, rows[0], state);
            render_status(frame, rows[1], state);
        }
    }
}

/// The view selector: `▌ AIVYX  1 Chat · 2 Dashboard · …` with the
/// active view amber, on the dark status fill.
fn render_tab_bar(frame: &mut Frame, area: Rect, state: &AppState) {
    let mut spans = vec![
        Span::styled("▌", bold(palette::AMBER)),
        Span::styled(" AIVYX  ", bold(palette::AMBER)),
    ];
    for (i, v) in View::ALL.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", fg(palette::DIMMER)));
        }
        let label = format!("{} {}", i + 1, v.label());
        spans.push(if *v == state.view {
            Span::styled(label, bold(palette::AMBER))
        } else {
            Span::styled(label, fg(palette::DIM))
        });
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(palette::STATUS_BG)),
        area,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled("Tab ⇄ views ", fg(palette::DIMMER))]))
            .right_aligned()
            .style(Style::default().bg(palette::STATUS_BG)),
        area,
    );
}

/// A bordered panel with an amber title (the read-only views' frame).
fn panel_block(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(fg(palette::BORDER))
        .title(Span::styled(format!(" {title} "), bold(palette::AMBER)))
        .padding(Padding::new(1, 1, 0, 0))
        .style(Style::default().bg(palette::BG))
}

/// Render the active read-only panel (Dashboard / Audit / Tools). These
/// show their frame + the state the model already holds; live IPC data
/// (mission / loop / reminders / audit stream / tool stats) is the
/// Phase 186 follow-on.
fn render_panel(frame: &mut Frame, area: Rect, state: &AppState) {
    let (title, lines) = match state.view {
        View::Dashboard => ("DASHBOARD", dashboard_lines(state)),
        View::Audit => (
            "AUDIT",
            placeholder_lines("the HMAC-chained audit stream — events, verification, JSONL export"),
        ),
        View::Tools => (
            "TOOLS",
            placeholder_lines("the registered tools — provenance, capability scope, and call stats"),
        ),
        View::Chat | View::Missions => return,
    };
    frame.render_widget(Paragraph::new(lines).block(panel_block(title)), area);
}

/// The Nonagon Missions/Fleet panel (Chapter J.7): a mission stream on the
/// left, the selected mission's step timeline on the right — the live render
/// of a team's mission DAG, fed by `Msg::MissionsUpdated`.
fn render_missions(frame: &mut Frame, area: Rect, state: &AppState) {
    let cols = Layout::horizontal([Constraint::Percentage(56), Constraint::Percentage(44)])
        .spacing(1)
        .split(area);
    render_mission_stream(frame, cols[0], state);
    render_mission_detail(frame, cols[1], state);
}

/// Phase → (badge text, style).
fn phase_badge(phase: MissionPhase) -> Span<'static> {
    let (text, color) = match phase {
        MissionPhase::Executing => ("● executing", palette::OK),
        MissionPhase::AwaitingApproval => ("⚑ approval", palette::AMBER),
        MissionPhase::Planning => ("◦ planning", palette::DIM),
        MissionPhase::Done => ("✓ done", palette::DIMMER),
        MissionPhase::Rejected => ("✗ rejected", palette::ERR),
        MissionPhase::Halted => ("⊘ halted (budget)", palette::ERR),
    };
    Span::styled(text, bold(color))
}

/// A `[████░░░]` progress bar `width` cells wide.
fn progress_bar(pct: u16, width: usize) -> Vec<Span<'static>> {
    let filled = (pct as usize * width / 100).min(width);
    vec![
        Span::styled("█".repeat(filled), fg(palette::AMBER)),
        Span::styled("░".repeat(width - filled), fg(palette::DIMMER)),
    ]
}

fn render_mission_stream(frame: &mut Frame, area: Rect, state: &AppState) {
    let rows = &state.missions.rows;
    if rows.is_empty() {
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled("— no missions running —", fg(palette::DIM))),
            Line::from(Span::styled(
                "Run one with `aivyx team run \"<mission>\"`; the lead's DAG",
                fg(palette::DIMMER),
            )),
            Line::from(Span::styled(
                "and each specialist's progress stream in here live.",
                fg(palette::DIMMER),
            )),
        ];
        frame.render_widget(Paragraph::new(lines).block(panel_block("MISSIONS")), area);
        return;
    }

    let mut lines: Vec<Line> = Vec::new();
    for (i, m) in rows.iter().enumerate() {
        let sel = i == state.missions.selected;
        let marker = if sel {
            Span::styled("▌ ", bold(palette::AMBER))
        } else {
            Span::styled("  ", fg(palette::DIM))
        };
        let title_style = if sel { bold(palette::AMBER) } else { bold(palette::FG) };
        lines.push(Line::from(vec![
            marker,
            phase_badge(m.phase),
            Span::styled(format!("  {}  ", m.id), fg(palette::DIMMER)),
            Span::styled(m.title.clone(), title_style),
        ]));
        // Progress + lead on the meta line.
        let mut meta = vec![Span::styled("    ", fg(palette::DIM))];
        meta.extend(progress_bar(m.progress, 18));
        meta.push(Span::styled(format!("  {}%  · ", m.progress), fg(palette::DIM)));
        meta.push(Span::styled(m.lead.clone(), fg(palette::LAV)));
        lines.push(Line::from(meta));
        lines.push(Line::from(""));
    }
    frame.render_widget(Paragraph::new(lines).block(panel_block("MISSIONS")), area);
}

fn render_mission_detail(frame: &mut Frame, area: Rect, state: &AppState) {
    let Some(m) = state.missions.selected_row() else {
        frame.render_widget(
            Paragraph::new(Vec::<Line>::new()).block(panel_block("STEPS")),
            area,
        );
        return;
    };

    let mut lines: Vec<Line> = Vec::new();
    for step in &m.steps {
        let color = match step.state {
            StepState::Done => palette::OK,
            StepState::Running => palette::AMBER,
            StepState::Gated => palette::LAV,
            StepState::Failed => palette::ERR,
            StepState::Pending => palette::DIMMER,
        };
        lines.push(Line::from(vec![
            Span::styled(step.state.dot(), fg(color)),
            Span::styled("  ", fg(palette::DIM)),
            Span::styled(step.label.clone(), fg(palette::FG)),
        ]));
    }
    if m.steps.is_empty() {
        lines.push(Line::from(Span::styled("— no steps yet —", fg(palette::DIM))));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("lead · ", fg(palette::LAV)),
        Span::styled(m.lead.clone(), fg(palette::FG)),
        Span::styled(format!("   {}", m.phase.label()), fg(palette::DIMMER)),
    ]));
    // Chapter L.6 — when the mission is paused at a human gate, surface the
    // approve/reject affordance the Missions-panel keys drive.
    if m.phase == MissionPhase::AwaitingApproval {
        if let Some(gate) = &m.pending_gate {
            lines.push(Line::from(Span::styled(
                format!("⚑ gate `{gate}` awaiting your decision"),
                fg(palette::AMBER),
            )));
            lines.push(Line::from(vec![
                Span::styled("a/y", fg(palette::OK)),
                Span::styled(" approve   ", fg(palette::DIM)),
                Span::styled("r", fg(palette::ERR)),
                Span::styled(" reject", fg(palette::DIM)),
            ]));
        }
    }

    let title = format!("{} · STEPS", m.id);
    frame.render_widget(Paragraph::new(lines).block(panel_block(&title)), area);
}

fn kv<'a>(k: &'a str, v: Span<'a>) -> Line<'a> {
    Line::from(vec![Span::styled(format!("{k:<10}"), fg(palette::DIM)), v])
}

fn dashboard_lines(state: &AppState) -> Vec<Line<'_>> {
    let role = state.status.role.as_deref().unwrap_or("—");
    let daemon = if state.status.daemon_connected {
        Span::styled("connected ✓", fg(palette::OK))
    } else {
        Span::styled("offline ✗", fg(palette::ERR))
    };
    let status = if state.status.working {
        Span::styled("working…", fg(palette::LAV))
    } else {
        Span::styled("idle", fg(palette::FG))
    };
    vec![
        kv("role", Span::styled(role.to_string(), fg(palette::FG))),
        kv("daemon", daemon),
        kv("status", status),
        kv(
            "session",
            Span::styled(format!("{} lines", state.history.len()), fg(palette::FG)),
        ),
        Line::from(""),
        Line::from(Span::styled(
            "mission · loop · reminders · recent-audit panels land in Phase 186,",
            fg(palette::DIMMER),
        )),
        Line::from(Span::styled(
            "wired to the live daemon state the IPC already serves.",
            fg(palette::DIMMER),
        )),
    ]
}

fn placeholder_lines(desc: &str) -> Vec<Line<'_>> {
    vec![
        Line::from(""),
        Line::from(Span::styled("— not yet wired to the daemon —", fg(palette::DIM))),
        Line::from(Span::styled(desc.to_string(), fg(palette::DIMMER))),
        Line::from(""),
        Line::from(Span::styled(
            "Phase 186: this panel reads live state over the IPC.",
            fg(palette::DIMMER),
        )),
    ]
}

fn render_chat(frame: &mut Frame, area: Rect, state: &AppState) {
    let lines: Vec<Line> = state
        .history
        .iter()
        .map(|cl| {
            let (prefix, style) = kind_style(cl.kind);
            Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(cl.text.clone(), style),
            ])
        })
        .collect();

    let top = chat_scroll_offset(state.history.len(), area.height as usize, state.scroll);
    let para = Paragraph::new(Text::from(lines)).scroll((top, 0));
    frame.render_widget(para, area);
}

fn render_status(frame: &mut Frame, area: Rect, state: &AppState) {
    let sep = || Span::styled(" · ", fg(palette::DIM));

    // Left: role · daemon · (gate | working).
    let role = state.status.role.as_deref().unwrap_or("—");
    let mut left = vec![
        Span::styled(format!(" {role}"), bold(palette::AMBER)),
        sep(),
        if state.status.daemon_connected {
            Span::styled("daemon ✓", fg(palette::OK))
        } else {
            Span::styled("daemon ✗", fg(palette::ERR))
        },
    ];
    if state.gate.is_some() {
        left.push(sep());
        left.push(Span::styled("⚑ approval needed", bold(palette::AMBER)));
    } else if state.status.working {
        left.push(sep());
        left.push(Span::styled("working…", fg(palette::LAV)));
    }

    // Right: context-appropriate keybinding help.
    let help: Vec<Span> = if state.gate.is_some() {
        vec![
            Span::styled("y approve", fg(palette::OK)),
            sep(),
            Span::styled("n reject", fg(palette::FG)),
            sep(),
            Span::styled("^Q quit ", fg(palette::DIM)),
        ]
    } else if state.view == View::Missions && state.mission_compose.is_some() {
        vec![Span::styled(
            "type a goal · Enter start · Esc cancel ",
            fg(palette::DIM),
        )]
    } else if state.view == View::Missions {
        vec![Span::styled(
            "n new · ↑↓ select · a approve · r reject · Esc chat · ^Q quit ",
            fg(palette::DIM),
        )]
    } else if state.view != View::Chat {
        vec![Span::styled(
            "Tab views · 1-4 jump · ↑↓ scroll · Esc chat · ^Q quit ",
            fg(palette::DIM),
        )]
    } else {
        vec![Span::styled(
            "Tab views · ^Q quit · PgUp/PgDn scroll · Esc cancel ",
            fg(palette::DIM),
        )]
    };

    // Left status, right-aligned help on the same row.
    let span_w = |spans: &[Span]| spans.iter().map(|s| s.content.chars().count()).sum::<usize>();
    let pad = (area.width as usize).saturating_sub(span_w(&left) + span_w(&help));

    let mut spans = left;
    spans.push(Span::raw(" ".repeat(pad)));
    spans.extend(help);
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(palette::STATUS_BG)),
        area,
    );
}

fn render_input(frame: &mut Frame, area: Rect, state: &AppState) {
    if let Some(gate) = &state.gate {
        let prompt = format!("Approve? [y/n] — {}", gate.reason);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(Span::styled(" Approval gate ", bold(palette::AMBER)))
            .border_style(fg(palette::AMBER))
            .style(Style::default().bg(palette::BG));
        frame.render_widget(
            Paragraph::new(prompt).style(fg(palette::AMBER)).block(block),
            area,
        );
        return;
    }

    let title = if state.status.working {
        " Input (working…) "
    } else {
        " Input "
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(fg(palette::BORDER))
        .title(Span::styled(title, fg(palette::DIM)))
        .style(Style::default().bg(palette::BG));
    frame.render_widget(
        Paragraph::new(state.input.as_str())
            .style(fg(palette::FG))
            .block(block),
        area,
    );

    // Place the terminal cursor inside the bordered input at the
    // current char position. Approximate (char count, not grapheme
    // width); refined alongside wrapping in a later phase.
    let cx = area.x + 1 + state.cursor.min(area.width.saturating_sub(2) as usize) as u16;
    let cy = area.y + 1;
    frame.set_cursor_position(Position::new(cx, cy));
}

/// Chapter L — the "new mission" compose box: a bordered goal input, or a
/// "starting…" indicator while the daemon decomposes the goal.
fn render_mission_compose(frame: &mut Frame, area: Rect, state: &AppState) {
    if state.mission_starting {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(fg(palette::LAV))
            .title(Span::styled(" Starting mission… ", bold(palette::LAV)))
            .style(Style::default().bg(palette::BG));
        frame.render_widget(
            Paragraph::new("decomposing the goal into a plan…")
                .style(fg(palette::LAV))
                .block(block),
            area,
        );
        return;
    }

    let goal = state.mission_compose.as_deref().unwrap_or("");
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(fg(palette::AMBER))
        .title(Span::styled(
            " New mission · Enter start · Esc cancel ",
            bold(palette::AMBER),
        ))
        .style(Style::default().bg(palette::BG));
    frame.render_widget(Paragraph::new(goal).style(fg(palette::FG)).block(block), area);

    // Cursor at the end of the typed goal.
    let len = goal.chars().count();
    let cx = area.x + 1 + (len as u16).min(area.width.saturating_sub(2));
    frame.set_cursor_position(Position::new(cx, area.y + 1));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ChatLine, PendingGate};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn scroll_offset_pins_to_bottom() {
        // 10 lines, 4-row viewport, pinned: top = 6.
        assert_eq!(chat_scroll_offset(10, 4, 0), 6);
        // Scrolled up by 2: top = 4.
        assert_eq!(chat_scroll_offset(10, 4, 2), 4);
        // Scrolled up past the top clamps to 0.
        assert_eq!(chat_scroll_offset(10, 4, 100), 0);
        // Everything fits: no offset.
        assert_eq!(chat_scroll_offset(3, 10, 0), 0);
    }

    #[test]
    fn renders_chat_and_status_into_buffer() {
        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();

        let mut state = AppState::new();
        state.status.daemon_connected = true;
        state.status.role = Some("assistant".into());
        state.history.push(ChatLine {
            kind: LineKind::Operator,
            text: "hello there".into(),
        });
        state.history.push(ChatLine {
            kind: LineKind::Agent,
            text: "general kenobi".into(),
        });

        terminal.draw(|f| render(f, &state)).unwrap();
        let text = buffer_text(&terminal);

        assert!(text.contains("hello there"), "operator line rendered");
        assert!(text.contains("general kenobi"), "agent line rendered");
        assert!(text.contains("assistant"), "role in status bar");
        assert!(text.contains("daemon ✓"), "daemon status rendered");
        assert!(text.contains("Input"), "input block titled");
        // The tab bar is present on every view.
        assert!(text.contains("Chat"), "tab bar lists Chat");
        assert!(text.contains("Dashboard"), "tab bar lists Dashboard");
    }

    #[test]
    fn dashboard_view_renders_panel_not_chat_input() {
        let backend = TestBackend::new(72, 14);
        let mut terminal = Terminal::new(backend).unwrap();

        let mut state = AppState::new();
        state.view = View::Dashboard;
        state.status.daemon_connected = true;
        state.status.role = Some("researcher".into());

        terminal.draw(|f| render(f, &state)).unwrap();
        let text = buffer_text(&terminal);

        // The panel renders (not the chat input line).
        assert!(text.contains("DASHBOARD"), "panel titled");
        assert!(text.contains("researcher"), "role shown in panel");
        assert!(text.contains("Phase 186"), "honest live-wiring note");
        assert!(!text.contains(" Input "), "no chat input in a panel view");
        // Tab bar still present.
        assert!(text.contains("Audit"), "tab bar lists Audit");
    }

    #[test]
    fn missions_view_renders_stream_and_selected_steps() {
        use crate::model::{MissionPhase, MissionRow, MissionStep, StepState};
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        let mut state = AppState::new();
        state.view = View::Missions;
        state.missions.rows = vec![
            MissionRow {
                id: "m-aria".into(),
                title: "Run end-of-day BOH close".into(),
                lead: "aria".into(),
                phase: MissionPhase::Executing,
                progress: 50,
                steps: vec![
                    MissionStep { label: "stocktake — count".into(), state: StepState::Done },
                    MissionStep { label: "inventory — low stock".into(), state: StepState::Running },
                ],
                pending_gate: None,
            },
            MissionRow {
                id: "m-2".into(),
                title: "second mission".into(),
                lead: "coordinator".into(),
                phase: MissionPhase::Planning,
                progress: 0,
                steps: vec![],
                pending_gate: None,
            },
        ];

        terminal.draw(|f| render(f, &state)).unwrap();
        let text = buffer_text(&terminal);

        assert!(text.contains("MISSIONS"), "stream panel titled");
        assert!(text.contains("Run end-of-day BOH close"), "mission title rendered");
        assert!(text.contains("executing"), "phase badge rendered");
        // The selected mission's steps appear in the detail panel.
        assert!(text.contains("STEPS"), "detail panel titled");
        assert!(text.contains("stocktake — count"), "selected mission's steps shown");
        assert!(text.contains("aria"), "lead shown");
        assert!(!text.contains(" Input "), "no chat input in the Missions panel");
        // Tab bar lists the new view.
        assert!(text.contains("Missions"), "tab bar lists Missions");
    }

    #[test]
    fn awaiting_mission_renders_the_approve_reject_affordance() {
        use crate::model::{MissionPhase, MissionRow, MissionStep, StepState};
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();

        let mut state = AppState::new();
        state.view = View::Missions;
        state.missions.rows = vec![MissionRow {
            id: "m-1".into(),
            title: "ship the note".into(),
            lead: "coordinator".into(),
            phase: MissionPhase::AwaitingApproval,
            progress: 33,
            steps: vec![MissionStep {
                label: "approve — reviewer (gate)".into(),
                state: StepState::Gated,
            }],
            pending_gate: Some("approve".into()),
        }];

        terminal.draw(|f| render(f, &state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("gate `approve` awaiting"), "gate affordance shown");
        assert!(text.contains("approve"), "approve key hint shown");
        assert!(text.contains("reject"), "reject key hint shown");
    }

    #[test]
    fn new_mission_compose_box_renders_the_goal_and_hints() {
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = AppState::new();
        state.view = View::Missions;
        state.mission_compose = Some("close the kitchen".into());

        terminal.draw(|f| render(f, &state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("New mission"), "compose box titled");
        assert!(text.contains("close the kitchen"), "typed goal shown");
        assert!(text.contains("Enter start"), "submit hint shown");
    }

    #[test]
    fn starting_mission_shows_progress_indicator() {
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = AppState::new();
        state.view = View::Missions;
        state.mission_starting = true;

        terminal.draw(|f| render(f, &state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Starting mission"), "starting indicator shown");
        assert!(text.contains("decomposing"), "decomposition note shown");
    }

    #[test]
    fn missions_view_shows_empty_state() {
        let backend = TestBackend::new(90, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = AppState::new();
        state.view = View::Missions;
        terminal.draw(|f| render(f, &state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("no missions running"), "empty-state hint shown");
        assert!(text.contains("aivyx team run"), "points at the command");
    }

    #[test]
    fn renders_gate_prompt_when_pending() {
        let backend = TestBackend::new(60, 8);
        let mut terminal = Terminal::new(backend).unwrap();

        let mut state = AppState::new();
        state.gate = Some(PendingGate {
            mission_id: "m1".into(),
            gate_id: "g1".into(),
            reason: "writes a file".into(),
            scope: Some("fs.write".into()),
        });

        terminal.draw(|f| render(f, &state)).unwrap();
        let text = buffer_text(&terminal);

        assert!(text.contains("Approve?"), "gate prompt shown");
        assert!(text.contains("writes a file"), "gate reason shown");
        assert!(text.contains("approve"), "gate keybinding hint shown");
    }
}
