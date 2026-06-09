//! The pure TUI model — Phase 185 Task 2.
//!
//! This module is the **testable core** of the terminal UI: an
//! [`AppState`] plus an [`update`] reducer (`(AppState, Msg) ->
//! AppState`) and the [`lines_from_event`] mapping that turns daemon
//! [`StreamEventPayload`]s into chat lines. It is deliberately free of
//! any `ratatui` / `crossterm` types — the terminal driver (Task 3)
//! translates key events into [`Msg`]s and renders [`AppState`], but
//! the *logic* lives here where it can be unit-tested without a
//! terminal (the same operator-verification split the phase doc calls
//! for: pure model tested in CI, visual rendering verified on the
//! operator's host).
//!
//! The daemon is the agent; this is just a render + interaction layer
//! over the IPC stream — no capability, trust, or audit concern.

use aivyx_channel::daemon_ipc::StreamEventPayload;

/// The provenance of a rendered chat line. The terminal driver maps
/// each kind to a style; the pure model only tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// Something the operator typed and submitted.
    Operator,
    /// Agent (assistant) text from a `Text` event.
    Agent,
    /// A tool-call breadcrumb (started / finished / output).
    Tool,
    /// A daemon status line (`⋯ working…`-style).
    Status,
    /// An approval-gate announcement.
    Gate,
    /// A locally-generated note (errors, cancellation, connection).
    System,
}

/// One rendered line in the scrollback. `text` is already a single
/// display line (no embedded newlines) — multi-line events are split
/// into several `ChatLine`s by [`lines_from_event`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatLine {
    pub kind: LineKind,
    pub text: String,
}

impl ChatLine {
    fn new(kind: LineKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
        }
    }
}

/// An approval gate awaiting an in-TUI approve/reject decision. While
/// `Some`, the driver renders an approve/reject prompt; resolving it
/// (via [`Msg::GateResolved`]) clears it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingGate {
    pub mission_id: String,
    pub gate_id: String,
    pub reason: String,
    pub scope: Option<String>,
}

/// The status bar's model: who we're talking as, whether the daemon
/// is connected, and whether a turn is in flight.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Status {
    pub role: Option<String>,
    pub daemon_connected: bool,
    /// A turn was submitted and we're awaiting its result — the
    /// "working…" state. Input submission is suppressed while true.
    pub working: bool,
}

/// A top-level view in the TUI. `Chat` is the shipped interactive
/// surface; the others are read-only panels (live-data wiring is the
/// Phase 186 follow-on). The order is the tab order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum View {
    #[default]
    Chat,
    Dashboard,
    Audit,
    Tools,
}

impl View {
    /// Every view, in tab order.
    pub const ALL: [View; 4] = [View::Chat, View::Dashboard, View::Audit, View::Tools];

    /// The tab label.
    pub fn label(self) -> &'static str {
        match self {
            View::Chat => "Chat",
            View::Dashboard => "Dashboard",
            View::Audit => "Audit",
            View::Tools => "Tools",
        }
    }

    fn index(self) -> usize {
        View::ALL.iter().position(|v| *v == self).unwrap_or(0)
    }

    /// The next view (wraps).
    pub fn next(self) -> View {
        View::ALL[(self.index() + 1) % View::ALL.len()]
    }

    /// The previous view (wraps).
    pub fn prev(self) -> View {
        View::ALL[(self.index() + View::ALL.len() - 1) % View::ALL.len()]
    }
}

/// The complete UI state. Owned, cloneable, and free of terminal
/// types so the reducer is pure.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AppState {
    /// The active top-level view (Chat by default).
    pub view: View,
    /// Scrollback, oldest first.
    pub history: Vec<ChatLine>,
    /// The current input buffer.
    pub input: String,
    /// Cursor position within `input`, as a **char** index in
    /// `[0, input.chars().count()]`.
    pub cursor: usize,
    /// Lines scrolled up from the bottom. `0` == pinned to the latest
    /// line. Bounded in `[0, history.len()]` by the reducer; the
    /// render layer further clamps to the viewport height.
    pub scroll: usize,
    pub status: Status,
    /// A pending approval gate, if any.
    pub gate: Option<PendingGate>,
    /// Set once the operator asks to quit; the driver's event loop
    /// observes this and tears down the terminal.
    pub should_quit: bool,
}

/// A message into the reducer. Key events become editing / scroll /
/// submit messages; daemon round-trips become result messages.
#[derive(Debug, Clone, PartialEq)]
pub enum Msg {
    // ---- input editing ----
    /// Insert a character at the cursor.
    InsertChar(char),
    /// Delete the character before the cursor.
    Backspace,
    /// Delete the character at the cursor.
    Delete,
    /// Move the cursor one char left.
    CursorLeft,
    /// Move the cursor one char right.
    CursorRight,
    /// Move the cursor to the start of the input.
    CursorHome,
    /// Move the cursor to the end of the input.
    CursorEnd,

    // ---- submission ----
    /// Submit the current input as a turn. A no-op while a turn is in
    /// flight or when the input is blank. On success the input is
    /// echoed as an [`LineKind::Operator`] line, the buffer cleared,
    /// and [`Status::working`] set.
    Submit,

    // ---- view navigation ----
    /// Switch to the next view (wraps).
    NextView,
    /// Switch to the previous view (wraps).
    PrevView,
    /// Jump directly to a view.
    SwitchView(View),

    // ---- scrolling ----
    /// Scroll up (toward older lines) by `n` lines.
    ScrollUp(usize),
    /// Scroll down (toward newer lines) by `n` lines.
    ScrollDown(usize),
    /// Pin back to the latest line.
    ScrollToBottom,

    // ---- daemon round-trips ----
    /// A turn finished: append its events as chat lines, clear
    /// `working`, and pick up any approval gate it surfaced.
    TurnFinished {
        events: Vec<StreamEventPayload>,
        outcome: String,
    },
    /// An approval gate was resolved in-TUI.
    GateResolved { approved: bool },
    /// The in-flight turn was cancelled.
    Cancelled,
    /// An error occurred talking to the daemon.
    Error(String),

    // ---- lifecycle ----
    /// The session connected; records the role for the status bar.
    Connected { role: Option<String> },
    /// Request to quit the application.
    Quit,
}

impl AppState {
    /// A fresh state with the input cursor at 0.
    pub fn new() -> Self {
        Self::default()
    }

    /// The number of chars in the input buffer.
    fn input_char_len(&self) -> usize {
        self.input.chars().count()
    }

    /// Byte offset in `input` for char index `cursor`.
    fn cursor_byte(&self) -> usize {
        self.input
            .char_indices()
            .nth(self.cursor)
            .map(|(b, _)| b)
            .unwrap_or(self.input.len())
    }

    /// The text the operator would submit, trimmed — `None` if blank.
    /// The driver uses this to decide whether to spawn a turn.
    pub fn submittable(&self) -> Option<String> {
        let trimmed = self.input.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    /// Push a chat line and re-pin to the bottom so new content is
    /// always visible.
    fn push_line(&mut self, line: ChatLine) {
        self.history.push(line);
        self.scroll = 0;
    }
}

/// The reducer. Pure: `(state, msg) -> state`. Owns the editing,
/// submission, scroll, and daemon-result transitions.
pub fn update(mut state: AppState, msg: Msg) -> AppState {
    match msg {
        Msg::InsertChar(c) => {
            let at = state.cursor_byte();
            state.input.insert(at, c);
            state.cursor += 1;
        }
        Msg::Backspace => {
            if state.cursor > 0 {
                state.cursor -= 1;
                let at = state.cursor_byte();
                state.input.remove(at);
            }
        }
        Msg::Delete => {
            if state.cursor < state.input_char_len() {
                let at = state.cursor_byte();
                state.input.remove(at);
            }
        }
        Msg::CursorLeft => {
            state.cursor = state.cursor.saturating_sub(1);
        }
        Msg::CursorRight => {
            if state.cursor < state.input_char_len() {
                state.cursor += 1;
            }
        }
        Msg::CursorHome => state.cursor = 0,
        Msg::CursorEnd => state.cursor = state.input_char_len(),

        Msg::NextView => state.view = state.view.next(),
        Msg::PrevView => state.view = state.view.prev(),
        Msg::SwitchView(v) => state.view = v,

        Msg::Submit => {
            if state.status.working {
                return state; // ignore while a turn is in flight
            }
            if let Some(text) = state.submittable() {
                state.push_line(ChatLine::new(LineKind::Operator, text));
                state.input.clear();
                state.cursor = 0;
                state.status.working = true;
            }
        }

        Msg::ScrollUp(n) => {
            let max = state.history.len();
            state.scroll = (state.scroll + n).min(max);
        }
        Msg::ScrollDown(n) => {
            state.scroll = state.scroll.saturating_sub(n);
        }
        Msg::ScrollToBottom => state.scroll = 0,

        Msg::TurnFinished { events, .. } => {
            for event in &events {
                for line in lines_from_event(event) {
                    state.push_line(line);
                }
                if let StreamEventPayload::ApprovalGate {
                    mission_id,
                    gate_id,
                    reason,
                    scope,
                } = event
                {
                    state.gate = Some(PendingGate {
                        mission_id: mission_id.clone(),
                        gate_id: gate_id.clone(),
                        reason: reason.clone(),
                        scope: scope.clone(),
                    });
                }
            }
            state.status.working = false;
        }

        Msg::GateResolved { approved } => {
            let verdict = if approved { "approved" } else { "rejected" };
            state.push_line(ChatLine::new(
                LineKind::System,
                format!("Gate {verdict}."),
            ));
            state.gate = None;
        }

        Msg::Cancelled => {
            state.status.working = false;
            state.push_line(ChatLine::new(LineKind::System, "Turn cancelled."));
        }

        Msg::Error(e) => {
            state.status.working = false;
            state.push_line(ChatLine::new(LineKind::System, format!("Error: {e}")));
        }

        Msg::Connected { role } => {
            state.status.daemon_connected = true;
            state.status.role = role;
        }

        Msg::Quit => state.should_quit = true,
    }
    state
}

/// Map a single daemon [`StreamEventPayload`] to one or more chat
/// lines. Reuses the event's [`StreamEventPayload::render_for_cli`]
/// text content (the same rendering the REPL prints) and splits it
/// into single display lines, tagging each with the right
/// [`LineKind`]. Blank trailing lines from the CLI formatting are
/// dropped so the scrollback stays tight.
pub fn lines_from_event(event: &StreamEventPayload) -> Vec<ChatLine> {
    let kind = match event {
        StreamEventPayload::Text { .. } => LineKind::Agent,
        StreamEventPayload::Status { .. } => LineKind::Status,
        StreamEventPayload::ToolCallStarted { .. }
        | StreamEventPayload::ToolCallFinished { .. }
        | StreamEventPayload::ToolOutput { .. } => LineKind::Tool,
        StreamEventPayload::ApprovalGate { .. } => LineKind::Gate,
    };

    let rendered = event.render_for_cli();
    let lines: Vec<ChatLine> = rendered
        .split('\n')
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.trim().is_empty())
        .map(|l| ChatLine::new(kind, l.to_string()))
        .collect();

    // An all-whitespace event (e.g. a bare newline `Text`) still
    // deserves a blank line so spacing the agent intended survives.
    if lines.is_empty() && !rendered.is_empty() {
        return vec![ChatLine::new(kind, String::new())];
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(state: AppState, s: &str) -> AppState {
        s.chars().fold(state, |st, c| update(st, Msg::InsertChar(c)))
    }

    // ---- input editing ----

    #[test]
    fn insert_appends_and_advances_cursor() {
        let s = typed(AppState::new(), "hi");
        assert_eq!(s.input, "hi");
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn insert_at_cursor_midline() {
        let mut s = typed(AppState::new(), "ac");
        s = update(s, Msg::CursorLeft); // between a and c
        s = update(s, Msg::InsertChar('b'));
        assert_eq!(s.input, "abc");
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn backspace_removes_before_cursor() {
        let mut s = typed(AppState::new(), "abc");
        s = update(s, Msg::Backspace);
        assert_eq!(s.input, "ab");
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn backspace_at_start_is_noop() {
        let mut s = typed(AppState::new(), "x");
        s = update(s, Msg::CursorHome);
        s = update(s, Msg::Backspace);
        assert_eq!(s.input, "x");
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn delete_removes_at_cursor() {
        let mut s = typed(AppState::new(), "abc");
        s = update(s, Msg::CursorHome);
        s = update(s, Msg::Delete);
        assert_eq!(s.input, "bc");
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn cursor_bounds_clamp() {
        let mut s = typed(AppState::new(), "ab");
        s = update(s, Msg::CursorRight); // already at end
        assert_eq!(s.cursor, 2);
        for _ in 0..5 {
            s = update(s, Msg::CursorLeft);
        }
        assert_eq!(s.cursor, 0);
        s = update(s, Msg::CursorEnd);
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn editing_handles_multibyte_chars() {
        // 'é' and '🦀' are multi-byte; cursor is a char index, so
        // byte-offset math must not panic on a char boundary.
        let mut s = typed(AppState::new(), "é🦀z");
        assert_eq!(s.cursor, 3);
        s = update(s, Msg::CursorLeft); // before z
        s = update(s, Msg::Backspace); // remove 🦀
        assert_eq!(s.input, "éz");
        assert_eq!(s.cursor, 1);
    }

    // ---- submission ----

    #[test]
    fn submit_appends_operator_line_and_clears() {
        let mut s = typed(AppState::new(), "hello");
        s = update(s, Msg::Submit);
        assert_eq!(s.history, vec![ChatLine::new(LineKind::Operator, "hello")]);
        assert_eq!(s.input, "");
        assert_eq!(s.cursor, 0);
        assert!(s.status.working);
    }

    #[test]
    fn submit_trims_whitespace() {
        let mut s = typed(AppState::new(), "  hi  ");
        s = update(s, Msg::Submit);
        assert_eq!(s.history[0].text, "hi");
    }

    #[test]
    fn submit_blank_is_noop() {
        let mut s = typed(AppState::new(), "   ");
        s = update(s, Msg::Submit);
        assert!(s.history.is_empty());
        assert!(!s.status.working);
        assert!(s.submittable().is_none());
    }

    #[test]
    fn submit_ignored_while_working() {
        let mut s = typed(AppState::new(), "first");
        s = update(s, Msg::Submit);
        // Still working; type + submit again — must be ignored.
        s = typed(s, "second");
        s = update(s, Msg::Submit);
        assert_eq!(s.history.len(), 1);
        assert_eq!(s.input, "second");
    }

    // ---- scrolling ----

    #[test]
    fn scroll_bounds() {
        let mut s = AppState::new();
        for i in 0..3 {
            s.history.push(ChatLine::new(LineKind::Agent, format!("l{i}")));
        }
        s = update(s, Msg::ScrollUp(10));
        assert_eq!(s.scroll, 3, "clamps to history length");
        s = update(s, Msg::ScrollDown(1));
        assert_eq!(s.scroll, 2);
        s = update(s, Msg::ScrollDown(10));
        assert_eq!(s.scroll, 0, "clamps to bottom");
        s = update(s, Msg::ScrollUp(1));
        s = update(s, Msg::ScrollToBottom);
        assert_eq!(s.scroll, 0);
    }

    #[test]
    fn new_content_repins_to_bottom() {
        let mut s = AppState::new();
        s.history.push(ChatLine::new(LineKind::Agent, "old"));
        s = update(s, Msg::ScrollUp(1));
        assert_eq!(s.scroll, 1);
        s = update(
            s,
            Msg::TurnFinished {
                events: vec![StreamEventPayload::Text {
                    text: "new".into(),
                }],
                outcome: "completed: new".into(),
            },
        );
        assert_eq!(s.scroll, 0, "appending re-pins to the latest line");
    }

    // ---- status transitions ----

    #[test]
    fn connected_sets_status() {
        let s = update(
            AppState::new(),
            Msg::Connected {
                role: Some("assistant".into()),
            },
        );
        assert!(s.status.daemon_connected);
        assert_eq!(s.status.role.as_deref(), Some("assistant"));
    }

    #[test]
    fn turn_finished_clears_working_and_appends() {
        let mut s = typed(AppState::new(), "q");
        s = update(s, Msg::Submit);
        assert!(s.status.working);
        s = update(
            s,
            Msg::TurnFinished {
                events: vec![StreamEventPayload::Text {
                    text: "an answer".into(),
                }],
                outcome: "completed: an answer".into(),
            },
        );
        assert!(!s.status.working);
        assert_eq!(s.history.last().unwrap().kind, LineKind::Agent);
        assert_eq!(s.history.last().unwrap().text, "an answer");
    }

    #[test]
    fn cancelled_clears_working() {
        let mut s = typed(AppState::new(), "q");
        s = update(s, Msg::Submit);
        s = update(s, Msg::Cancelled);
        assert!(!s.status.working);
        assert_eq!(s.history.last().unwrap().kind, LineKind::System);
    }

    #[test]
    fn error_clears_working_and_notes() {
        let mut s = typed(AppState::new(), "q");
        s = update(s, Msg::Submit);
        s = update(s, Msg::Error("socket closed".into()));
        assert!(!s.status.working);
        assert!(s.history.last().unwrap().text.contains("socket closed"));
    }

    #[test]
    fn quit_sets_flag() {
        let s = update(AppState::new(), Msg::Quit);
        assert!(s.should_quit);
    }

    // ---- view navigation ----

    #[test]
    fn default_view_is_chat() {
        assert_eq!(AppState::new().view, View::Chat);
    }

    #[test]
    fn next_view_cycles_and_wraps() {
        let mut s = AppState::new();
        for expected in [View::Dashboard, View::Audit, View::Tools, View::Chat] {
            s = update(s, Msg::NextView);
            assert_eq!(s.view, expected);
        }
    }

    #[test]
    fn prev_view_wraps_backwards() {
        let s = update(AppState::new(), Msg::PrevView);
        assert_eq!(s.view, View::Tools);
    }

    #[test]
    fn switch_view_jumps_directly() {
        let s = update(AppState::new(), Msg::SwitchView(View::Audit));
        assert_eq!(s.view, View::Audit);
    }

    // ---- approval-gate transitions ----

    #[test]
    fn gate_pending_set_on_turn_then_cleared() {
        let mut s = AppState::new();
        s = update(
            s,
            Msg::TurnFinished {
                events: vec![StreamEventPayload::ApprovalGate {
                    mission_id: "m1".into(),
                    gate_id: "g1".into(),
                    reason: "writes a file".into(),
                    scope: Some("fs.write".into()),
                }],
                outcome: "escalated: gate".into(),
            },
        );
        let gate = s.gate.clone().expect("gate set");
        assert_eq!(gate.mission_id, "m1");
        assert_eq!(gate.gate_id, "g1");
        assert_eq!(gate.scope.as_deref(), Some("fs.write"));

        s = update(s, Msg::GateResolved { approved: true });
        assert!(s.gate.is_none());
        assert!(s.history.last().unwrap().text.contains("approved"));
    }

    // ---- event -> line mapping ----

    #[test]
    fn maps_text_event_to_agent_lines() {
        let ev = StreamEventPayload::Text {
            text: "line one\nline two\n".into(),
        };
        let lines = lines_from_event(&ev);
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|l| l.kind == LineKind::Agent));
        assert_eq!(lines[0].text, "line one");
        assert_eq!(lines[1].text, "line two");
    }

    #[test]
    fn maps_status_event() {
        let ev = StreamEventPayload::Status {
            status: "thinking".into(),
        };
        let lines = lines_from_event(&ev);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].kind, LineKind::Status);
        assert!(lines[0].text.contains("thinking"));
    }

    #[test]
    fn maps_tool_call_events_to_tool_kind() {
        let started = StreamEventPayload::ToolCallStarted {
            tool_id: "t1".into(),
            tool_name: "web.search".into(),
            input: serde_json::json!({"q": "rust"}),
        };
        let finished = StreamEventPayload::ToolCallFinished {
            tool_id: "t1".into(),
            tool_name: "web.search".into(),
            outcome_summary: "3 results".into(),
        };
        for ev in [started, finished] {
            let lines = lines_from_event(&ev);
            assert_eq!(lines.len(), 1);
            assert_eq!(lines[0].kind, LineKind::Tool);
            assert!(lines[0].text.contains("web.search"));
        }
    }

    #[test]
    fn maps_gate_event_to_gate_kind() {
        let ev = StreamEventPayload::ApprovalGate {
            mission_id: "m1".into(),
            gate_id: "g1".into(),
            reason: "deletes data".into(),
            scope: None,
        };
        let lines = lines_from_event(&ev);
        assert!(!lines.is_empty());
        assert!(lines.iter().all(|l| l.kind == LineKind::Gate));
        assert!(lines.iter().any(|l| l.text.contains("deletes data")));
    }
}
