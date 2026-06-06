//! Keystroke → action translation — Phase 185 Task 3.
//!
//! The crossterm event loop (the async daemon-driven `run` loop lands
//! in Task 4) reads a [`KeyEvent`] and asks [`key_to_action`] what to
//! do. The translation is **pure** and context-aware: most keys
//! become a reducer [`Msg`] applied in-process, but the handful that
//! require daemon I/O — submitting a turn, cancelling it, resolving an
//! approval gate, quitting — surface as distinct [`Action`] variants
//! so the driver can perform the round-trip and *then* apply the
//! matching reducer message. Keeping this pure is what makes the
//! keybindings unit-testable without a terminal.

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

use crate::model::{AppState, Msg};

/// How many lines a PageUp / PageDn scrolls.
const PAGE: usize = 10;

/// What the event loop should do in response to a keystroke. Pure
/// reducer transitions are wrapped in [`Action::Update`]; the rest
/// name daemon-side effects the driver performs.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Apply a pure reducer message in-process.
    Update(Msg),
    /// The operator pressed Enter with submittable input — the driver
    /// captures the text, applies [`Msg::Submit`], and sends the turn.
    Submit,
    /// Cancel the in-flight turn (daemon `CancelTurn`).
    Cancel,
    /// Resolve the pending approval gate with the given verdict.
    ResolveGate(bool),
    /// Tear down and exit.
    Quit,
    /// Ignore this keystroke.
    None,
}

/// Translate a key event into an [`Action`], given the current state
/// (which decides context-dependent bindings: gate prompt, working).
pub fn key_to_action(key: KeyEvent, state: &AppState) -> Action {
    // Ignore key-release events (Windows / kitty protocol deliver
    // both press and release); we act only on press/repeat.
    if key.kind == KeyEventKind::Release {
        return Action::None;
    }

    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

    // An open approval gate captures input until it's resolved.
    if state.gate.is_some() {
        return match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => Action::ResolveGate(true),
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                Action::ResolveGate(false)
            }
            _ => Action::None,
        };
    }

    match key.code {
        // Quit / cancel.
        KeyCode::Char('q') if ctrl => Action::Quit,
        KeyCode::Char('c') if ctrl => {
            if state.status.working {
                Action::Cancel
            } else {
                Action::Quit
            }
        }
        KeyCode::Esc => {
            if state.status.working {
                Action::Cancel
            } else {
                Action::None
            }
        }

        // Submission.
        KeyCode::Enter => {
            if state.status.working || state.submittable().is_none() {
                Action::None
            } else {
                Action::Submit
            }
        }

        // Scrolling.
        KeyCode::PageUp => Action::Update(Msg::ScrollUp(PAGE)),
        KeyCode::PageDown => Action::Update(Msg::ScrollDown(PAGE)),
        KeyCode::Up => Action::Update(Msg::ScrollUp(1)),
        KeyCode::Down => Action::Update(Msg::ScrollDown(1)),

        // Input editing.
        KeyCode::Left => Action::Update(Msg::CursorLeft),
        KeyCode::Right => Action::Update(Msg::CursorRight),
        KeyCode::Home => Action::Update(Msg::CursorHome),
        KeyCode::End => Action::Update(Msg::CursorEnd),
        KeyCode::Backspace => Action::Update(Msg::Backspace),
        KeyCode::Delete => Action::Update(Msg::Delete),
        // A bare printable char (no modifiers, or just Shift) types.
        KeyCode::Char(c) if !ctrl => Action::Update(Msg::InsertChar(c)),

        _ => Action::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PendingGate;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn ctrl_key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn printable_char_types() {
        let s = AppState::new();
        assert_eq!(
            key_to_action(key(KeyCode::Char('a')), &s),
            Action::Update(Msg::InsertChar('a'))
        );
    }

    #[test]
    fn editing_and_cursor_keys_map() {
        let s = AppState::new();
        assert_eq!(
            key_to_action(key(KeyCode::Backspace), &s),
            Action::Update(Msg::Backspace)
        );
        assert_eq!(
            key_to_action(key(KeyCode::Left), &s),
            Action::Update(Msg::CursorLeft)
        );
        assert_eq!(
            key_to_action(key(KeyCode::Home), &s),
            Action::Update(Msg::CursorHome)
        );
    }

    #[test]
    fn scroll_keys_map() {
        let s = AppState::new();
        assert_eq!(
            key_to_action(key(KeyCode::PageUp), &s),
            Action::Update(Msg::ScrollUp(PAGE))
        );
        assert_eq!(
            key_to_action(key(KeyCode::Down), &s),
            Action::Update(Msg::ScrollDown(1))
        );
    }

    #[test]
    fn enter_submits_only_with_text() {
        let mut s = AppState::new();
        assert_eq!(key_to_action(key(KeyCode::Enter), &s), Action::None);
        s.input = "hi".into();
        s.cursor = 2;
        assert_eq!(key_to_action(key(KeyCode::Enter), &s), Action::Submit);
    }

    #[test]
    fn enter_ignored_while_working() {
        let mut s = AppState::new();
        s.input = "hi".into();
        s.status.working = true;
        assert_eq!(key_to_action(key(KeyCode::Enter), &s), Action::None);
    }

    #[test]
    fn ctrl_q_quits_ctrl_c_quits_when_idle() {
        let s = AppState::new();
        assert_eq!(key_to_action(ctrl_key('q'), &s), Action::Quit);
        assert_eq!(key_to_action(ctrl_key('c'), &s), Action::Quit);
    }

    #[test]
    fn ctrl_c_and_esc_cancel_while_working() {
        let mut s = AppState::new();
        s.status.working = true;
        assert_eq!(key_to_action(ctrl_key('c'), &s), Action::Cancel);
        assert_eq!(key_to_action(key(KeyCode::Esc), &s), Action::Cancel);
    }

    #[test]
    fn esc_idle_is_noop() {
        let s = AppState::new();
        assert_eq!(key_to_action(key(KeyCode::Esc), &s), Action::None);
    }

    #[test]
    fn gate_captures_yes_no_and_swallows_other_keys() {
        let mut s = AppState::new();
        s.gate = Some(PendingGate {
            mission_id: "m".into(),
            gate_id: "g".into(),
            reason: "r".into(),
            scope: None,
        });
        assert_eq!(
            key_to_action(key(KeyCode::Char('y')), &s),
            Action::ResolveGate(true)
        );
        assert_eq!(
            key_to_action(key(KeyCode::Char('n')), &s),
            Action::ResolveGate(false)
        );
        assert_eq!(
            key_to_action(key(KeyCode::Esc), &s),
            Action::ResolveGate(false)
        );
        // A normal letter does not type while the gate is up.
        assert_eq!(key_to_action(key(KeyCode::Char('a')), &s), Action::None);
    }

    #[test]
    fn key_release_is_ignored() {
        let s = AppState::new();
        let mut k = key(KeyCode::Char('a'));
        k.kind = KeyEventKind::Release;
        assert_eq!(key_to_action(k, &s), Action::None);
    }

    #[test]
    fn ctrl_modified_letters_do_not_type() {
        // ctrl+a is not a quit/cancel binding and must not insert 'a'.
        let s = AppState::new();
        assert_eq!(key_to_action(ctrl_key('a'), &s), Action::None);
    }
}
