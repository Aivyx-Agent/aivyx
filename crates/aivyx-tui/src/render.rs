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

/// The Audit view's own version of [`chat_scroll_offset`] — same
/// signature and same key bindings (`Msg::ScrollUp`/`ScrollDown`), but
/// anchored at the opposite end: the Audit panel renders its page
/// newest-entry-first (see `render_panel`'s `.rev()`), so `0` scroll
/// pins to the *top* of the buffer (freshest visible, header included)
/// rather than chat's bottom-pinned "latest message" convention.
/// Scrolling up (`Msg::ScrollUp`, growing `scroll`) still means "go see
/// less-recent content" in both views — here that means moving the
/// viewport further down the page, toward its older tail.
pub fn audit_scroll_offset(total: usize, viewport: usize, scroll: usize) -> u16 {
    let max_top = total.saturating_sub(viewport);
    scroll.min(max_top).min(u16::MAX as usize) as u16
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
/// show their frame + the state the model already holds. Audit is wired
/// to live daemon data (`/classic` retirement, Task 5) and scrolls via
/// [`audit_scroll_offset`] (the Audit-page counterpart of
/// [`chat_scroll_offset`], which `render_chat` uses for the same
/// purpose); Dashboard's mission/loop/reminders detail and Tools'
/// capability/call-stats detail remain the Phase 186 follow-on.
fn render_panel(frame: &mut Frame, area: Rect, state: &AppState) {
    let (title, lines) = match state.view {
        View::Dashboard => ("DASHBOARD", dashboard_lines(state)),
        View::Audit => {
            let mut lines: Vec<Line> = vec![Line::from(Span::styled(
                format!("{} total events — ← / → to page", state.audit_total),
                fg(palette::DIMMER),
            ))];
            if state.audit_entries.is_empty() {
                lines.push(Line::from(Span::styled("No entries loaded.", fg(palette::DIM))));
            } else {
                for e in state.audit_entries.iter().rev() {
                    lines.push(Line::from(vec![
                        Span::styled(format!("#{} ", e.seq), fg(palette::DIMMER)),
                        Span::styled(e.event_type.clone(), fg(palette::AMBER)),
                    ]));
                }
            }
            ("AUDIT", lines)
        }
        View::Tools => {
            let mut lines: Vec<Line> = vec![Line::from(Span::styled(
                format!("{} tool(s) — whole audit chain", state.tool_stats.len()),
                fg(palette::DIMMER),
            ))];
            if state.tool_stats.is_empty() {
                lines.push(Line::from(Span::styled("No tools loaded.", fg(palette::DIM))));
            } else {
                for t in state.tool_stats.iter() {
                    let avg_ms = t.total_duration_ms.checked_div(t.calls).unwrap_or(0);
                    let marker = if t.registered { "" } else { " [unregistered]" };
                    lines.push(Line::from(vec![
                        Span::styled(format!("{}{marker} ", t.name), fg(palette::AMBER)),
                        Span::styled(format!("calls={} avg={avg_ms}ms", t.calls), fg(palette::DIMMER)),
                    ]));
                    if t.calls > 0 {
                        let parts: Vec<String> = t
                            .outcomes
                            .iter()
                            .map(|(label, count)| format!("{label}={count}"))
                            .collect();
                        lines.push(Line::from(Span::styled(
                            format!("  {}", parts.join(" ")),
                            fg(palette::DIM),
                        )));
                    }
                }
            }
            ("TOOLS", lines)
        }
        View::Chat | View::Missions => return,
    };

    let line_count = lines.len();
    let mut para = Paragraph::new(lines).block(panel_block(title));
    if state.view == View::Audit || state.view == View::Tools {
        // `panel_block` draws a top+bottom border (and no vertical
        // padding), so the visible text viewport is 2 rows shorter than
        // `area` — the same `chat_scroll_offset` math `render_chat` uses
        // for its own unbordered Paragraph, adjusted for that border.
        let viewport = area.height.saturating_sub(2) as usize;
        let top = audit_scroll_offset(line_count, viewport, state.scroll);
        para = para.scroll((top, 0));
    }
    frame.render_widget(para, area);
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
        MissionPhase::Paused => ("⏸ paused", palette::AMBER),
        MissionPhase::Planning => ("◦ planning", palette::DIM),
        MissionPhase::Done => ("✓ done", palette::DIMMER),
        MissionPhase::Rejected => ("✗ rejected", palette::ERR),
        MissionPhase::Halted => ("⊘ halted", palette::ERR),
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

fn render_chat(frame: &mut Frame, area: Rect, state: &AppState) {
    // Wrap each history line to the pane width (coalesced agent
    // replies are full paragraphs now — Vitrine §12); continuation
    // rows indent under the prefix so provenance stays scannable.
    // Wrapping here (not via Paragraph::wrap) keeps the scroll
    // offset math operating on real visual-line counts.
    let width = area.width.max(1) as usize;
    let mut lines: Vec<Line> = Vec::new();
    for cl in &state.history {
        let (prefix, style) = kind_style(cl.kind);
        let avail = width.saturating_sub(prefix.chars().count()).max(1);
        for (i, chunk) in wrap_line(&cl.text, avail).into_iter().enumerate() {
            let lead = if i == 0 {
                prefix.to_string()
            } else {
                " ".repeat(prefix.chars().count())
            };
            lines.push(Line::from(vec![
                Span::styled(lead, style),
                Span::styled(chunk, style),
            ]));
        }
    }

    let top = chat_scroll_offset(lines.len(), area.height as usize, state.scroll);
    let para = Paragraph::new(Text::from(lines)).scroll((top, 0));
    frame.render_widget(para, area);
}

/// Greedy width-wrap, breaking at the last space when one exists in
/// the overflowing line (words longer than the width hard-break).
/// Empty text yields one empty chunk so intended blank lines survive.
fn wrap_line(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    let mut count = 0usize;
    for ch in text.chars() {
        line.push(ch);
        count += 1;
        if count >= width {
            if let Some(pos) = line.rfind(' ') {
                let rest = line.split_off(pos + 1);
                out.push(std::mem::take(&mut line));
                line = rest;
            } else {
                out.push(std::mem::take(&mut line));
            }
            count = line.chars().count();
        }
    }
    if !line.is_empty() || out.is_empty() {
        out.push(line);
    }
    out
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
    fn audit_scroll_offset_pins_to_top() {
        // 10 lines, 4-row viewport, unscrolled: shows the top (freshest).
        assert_eq!(audit_scroll_offset(10, 4, 0), 0);
        // Scrolled down (toward older entries) by 2: top = 2.
        assert_eq!(audit_scroll_offset(10, 4, 2), 2);
        // Scrolled past the bottom clamps to max_top, not past it.
        assert_eq!(audit_scroll_offset(10, 4, 100), 6);
        // Everything fits: no offset regardless of scroll.
        assert_eq!(audit_scroll_offset(3, 10, 5), 0);
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
    fn wrap_line_breaks_at_spaces_and_preserves_content() {
        let chunks = wrap_line("the quick brown fox jumps over the lazy dog", 12);
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|c| c.chars().count() <= 12));
        assert_eq!(chunks.concat(), "the quick brown fox jumps over the lazy dog");
        // Blank lines survive as one empty chunk.
        assert_eq!(wrap_line("", 12), vec![String::new()]);
        // Overlong single words hard-break instead of overflowing.
        let long = wrap_line("abcdefghijklmnop", 5);
        assert!(long.iter().all(|c| c.chars().count() <= 5));
        assert_eq!(long.concat(), "abcdefghijklmnop");
    }

    #[test]
    fn audit_view_shows_empty_state_before_any_fetch() {
        let backend = TestBackend::new(72, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = AppState::new();
        state.view = View::Audit;

        terminal.draw(|f| render(f, &state)).unwrap();
        let text = buffer_text(&terminal);

        assert!(text.contains("AUDIT"), "panel titled");
        assert!(text.contains("0 total events"), "total shown even at zero");
        assert!(text.contains("No entries loaded"), "empty state shown");
        assert!(!text.contains("Phase 186"), "placeholder copy is gone");
    }

    #[test]
    fn audit_view_renders_entries_newest_first() {
        use aivyx_channel::daemon_ipc::AuditEntrySummary;
        let backend = TestBackend::new(72, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = AppState::new();
        state.view = View::Audit;
        state.audit_total = 2;
        state.audit_entries = vec![
            AuditEntrySummary {
                seq: 0,
                appended_at_unix_ms: 1_000,
                event_type: "TurnStarted".into(),
                event: serde_json::json!({}),
                mac_hex: "aaa".into(),
            },
            AuditEntrySummary {
                seq: 1,
                appended_at_unix_ms: 2_000,
                event_type: "TurnEnded".into(),
                event: serde_json::json!({}),
                mac_hex: "bbb".into(),
            },
        ];

        terminal.draw(|f| render(f, &state)).unwrap();
        let text = buffer_text(&terminal);

        assert!(text.contains("2 total events"), "total shown");
        assert!(text.contains("#0"), "seq 0 rendered");
        assert!(text.contains("#1"), "seq 1 rendered");
        assert!(text.contains("TurnStarted"), "event type rendered");
        assert!(text.contains("TurnEnded"), "event type rendered");
        // Newest first: seq 1 ("TurnEnded") appears before seq 0
        // ("TurnStarted") in the rendered buffer.
        let pos_1 = text.find("TurnEnded").unwrap();
        let pos_0 = text.find("TurnStarted").unwrap();
        assert!(pos_1 < pos_0, "newest entry (seq 1) renders above seq 0");
    }

    #[test]
    fn tools_view_renders_placeholder_copy_is_gone() {
        let backend = TestBackend::new(72, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = AppState::new();
        state.view = View::Tools;

        terminal.draw(|f| render(f, &state)).unwrap();
        let text = buffer_text(&terminal);

        assert!(text.contains("No tools loaded"), "empty state shown");
        assert!(!text.contains("capability scope"), "placeholder copy is gone");
    }

    #[test]
    fn tools_view_renders_call_stats() {
        use aivyx_channel::daemon_ipc::ToolStat;
        let backend = TestBackend::new(72, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = AppState::new();
        state.view = View::Tools;
        let mut outcomes = std::collections::BTreeMap::new();
        outcomes.insert("completed".to_string(), 3u64);
        state.tool_stats = vec![ToolStat {
            name: "fs.read".to_string(),
            description: "read a file".to_string(),
            scope_base: "fs.read".to_string(),
            registered: true,
            calls: 3,
            outcomes,
            total_duration_ms: 30,
        }];

        terminal.draw(|f| render(f, &state)).unwrap();
        let text = buffer_text(&terminal);

        assert!(text.contains("fs.read"), "tool name rendered");
        assert!(text.contains("calls=3"), "call count rendered");
        assert!(text.contains("completed=3"), "outcome breakdown rendered");
    }

    #[test]
    fn tools_view_marks_unregistered_tools() {
        use aivyx_channel::daemon_ipc::ToolStat;
        let backend = TestBackend::new(72, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = AppState::new();
        state.view = View::Tools;
        state.tool_stats = vec![ToolStat {
            name: "old.removed.tool".to_string(),
            description: "(no registered tool)".to_string(),
            scope_base: "old.removed.tool".to_string(),
            registered: false,
            calls: 1,
            outcomes: std::collections::BTreeMap::new(),
            total_duration_ms: 5,
        }];

        terminal.draw(|f| render(f, &state)).unwrap();
        let text = buffer_text(&terminal);

        assert!(text.contains("[unregistered]"), "unregistered marker rendered");
    }

    #[test]
    fn audit_view_scrolls_to_reveal_entries_below_the_fold() {
        // Final-review finding 2 — a short terminal + many entries means
        // the page overflows the panel; before this fix `render_panel`
        // never called `.scroll(...)` at all, so the oldest entries were
        // permanently unreachable no matter what `state.scroll` held.
        use aivyx_channel::daemon_ipc::AuditEntrySummary;
        let backend = TestBackend::new(40, 8); // ~6 text rows inside the border
        let mut terminal = Terminal::new(backend).unwrap();
        let mut state = AppState::new();
        state.view = View::Audit;
        state.audit_total = 20;
        state.audit_entries = (0..20u64)
            .map(|seq| AuditEntrySummary {
                seq,
                appended_at_unix_ms: seq * 1_000,
                event_type: format!("Event{seq}"),
                event: serde_json::json!({}),
                mac_hex: "aaa".into(),
            })
            .collect();

        terminal.draw(|f| render(f, &state)).unwrap();
        let unscrolled = buffer_text(&terminal);
        assert!(unscrolled.contains("Event19"), "newest entry visible by default");
        assert!(
            !unscrolled.contains("Event0 "),
            "the oldest entry doesn't fit before scrolling"
        );

        // Scroll all the way toward the older tail of the page (a scroll
        // request larger than the page clamps at the oldest entry rather
        // than panicking or no-oping).
        state = crate::model::update(state, crate::model::Msg::ScrollUp(30));
        terminal.draw(|f| render(f, &state)).unwrap();
        let scrolled = buffer_text(&terminal);
        assert!(
            scrolled.contains("Event0"),
            "scrolling to the bottom of the page must reach the oldest entry: {scrolled}"
        );
        assert_ne!(scrolled, unscrolled, "scrolling must actually change what's rendered");
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
