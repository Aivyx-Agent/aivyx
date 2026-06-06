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
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use crate::model::{AppState, LineKind};

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
        LineKind::Operator => (
            "❯ ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        LineKind::Agent => ("", Style::default()),
        LineKind::Tool => (
            "  ",
            Style::default().fg(Color::DarkGray),
        ),
        LineKind::Status => (
            "  ",
            Style::default()
                .fg(Color::DarkGray)
                .add_modifier(Modifier::ITALIC),
        ),
        LineKind::Gate => (
            "⚑ ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        LineKind::System => (
            "· ",
            Style::default().fg(Color::Magenta),
        ),
    }
}

/// Draw the whole UI for the current state.
pub fn render(frame: &mut Frame, state: &AppState) {
    let chunks = Layout::vertical([
        Constraint::Min(1),    // chat pane
        Constraint::Length(1), // status bar
        Constraint::Length(3), // bordered input
    ])
    .split(frame.area());

    render_chat(frame, chunks[0], state);
    render_status(frame, chunks[1], state);
    render_input(frame, chunks[2], state);
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
    let role = state.status.role.as_deref().unwrap_or("—");
    let daemon = if state.status.daemon_connected {
        "daemon ✓"
    } else {
        "daemon ✗"
    };

    let mut left = format!(" {role} · {daemon}");
    if state.gate.is_some() {
        left.push_str(" · ⚑ approval needed");
    } else if state.status.working {
        left.push_str(" · working…");
    }

    let help = if state.gate.is_some() {
        "y approve · n reject "
    } else {
        "^Q quit · PgUp/PgDn scroll · Esc cancel "
    };

    // Left status, right-aligned help on the same row.
    let used = left.chars().count() + help.chars().count();
    let pad = (area.width as usize).saturating_sub(used);
    let line = Line::from(vec![
        Span::styled(left, Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" ".repeat(pad)),
        Span::styled(help, Style::default().fg(Color::DarkGray)),
    ]);
    frame.render_widget(
        Paragraph::new(line).style(Style::default().bg(Color::Black)),
        area,
    );
}

fn render_input(frame: &mut Frame, area: Rect, state: &AppState) {
    if let Some(gate) = &state.gate {
        let prompt = format!("Approve? [y/n] — {}", gate.reason);
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Approval gate ")
            .border_style(Style::default().fg(Color::Yellow));
        frame.render_widget(
            Paragraph::new(prompt)
                .style(Style::default().fg(Color::Yellow))
                .block(block),
            area,
        );
        return;
    }

    let title = if state.status.working {
        " Input (working…) "
    } else {
        " Input "
    };
    let block = Block::default().borders(Borders::ALL).title(title);
    frame.render_widget(Paragraph::new(state.input.as_str()).block(block), area);

    // Place the terminal cursor inside the bordered input at the
    // current char position. Approximate (char count, not grapheme
    // width); refined alongside wrapping in a later phase.
    let cx = area.x + 1 + state.cursor.min(area.width.saturating_sub(2) as usize) as u16;
    let cy = area.y + 1;
    frame.set_cursor_position(Position::new(cx, cy));
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
