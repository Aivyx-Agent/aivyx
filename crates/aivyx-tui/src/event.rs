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

use crate::model::{AppState, MissionPhase, Msg, View};

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
    /// Chapter L.6 — resolve the selected Missions-panel team mission's
    /// human-approval gate (approve = `true`). The driver reads the selected
    /// row's `id` + `pending_gate` and sends `ResolveTeamGate`.
    ResolveTeamGate(bool),
    /// Chapter L — submit the composed "new mission" goal: the driver sends
    /// `TeamRunGoal`, the daemon decomposes + runs it.
    SubmitMission,
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

    // While a goal is being decomposed/started, the panel is inert.
    if state.mission_starting {
        return Action::None;
    }

    // The "new mission" compose box (Chapter L) captures input until Enter
    // submits or Esc cancels.
    if state.mission_compose.is_some() {
        return match key.code {
            KeyCode::Enter => Action::SubmitMission,
            KeyCode::Esc => Action::Update(Msg::MissionComposeCancel),
            KeyCode::Backspace => Action::Update(Msg::MissionComposeBackspace),
            KeyCode::Char(c) if !ctrl => Action::Update(Msg::MissionComposeChar(c)),
            KeyCode::Char('c') if ctrl => Action::Update(Msg::MissionComposeCancel),
            _ => Action::None,
        };
    }

    // Tab cycles the top-level views (universal — Tab is not text input).
    match key.code {
        KeyCode::Tab => return Action::Update(Msg::NextView),
        KeyCode::BackTab => return Action::Update(Msg::PrevView),
        _ => {}
    }

    // Read-only panel views take no text input: number keys jump to a
    // view, arrows scroll (or, in Missions, move the selection), Esc
    // returns to Chat, ^Q quits.
    if state.view != View::Chat {
        // In the Missions panel, ↑↓ move the mission selection instead of
        // scrolling chat.
        if state.view == View::Missions {
            // When the selected mission is paused at a human gate, a/y approve
            // and r/n reject it (Chapter L.6). Guarded on the selection's
            // phase so the keys are inert for non-gated missions.
            let awaiting = matches!(
                state.missions.selected_row().map(|m| m.phase),
                Some(MissionPhase::AwaitingApproval)
            );
            match key.code {
                KeyCode::Up => return Action::Update(Msg::MissionSelectPrev),
                KeyCode::Down => return Action::Update(Msg::MissionSelectNext),
                // `n` opens the "new mission" compose box (Chapter L).
                KeyCode::Char('n') => return Action::Update(Msg::MissionComposeOpen),
                KeyCode::Char('a') | KeyCode::Char('y') if awaiting => {
                    return Action::ResolveTeamGate(true)
                }
                KeyCode::Char('r') if awaiting => return Action::ResolveTeamGate(false),
                _ => {}
            }
        }
        return match key.code {
            KeyCode::Char('q') if ctrl => Action::Quit,
            KeyCode::Esc => Action::Update(Msg::SwitchView(View::Chat)),
            KeyCode::Char('1') => Action::Update(Msg::SwitchView(View::Chat)),
            KeyCode::Char('2') => Action::Update(Msg::SwitchView(View::Missions)),
            KeyCode::Char('3') => Action::Update(Msg::SwitchView(View::Dashboard)),
            KeyCode::Char('4') => Action::Update(Msg::SwitchView(View::Audit)),
            KeyCode::Char('5') => Action::Update(Msg::SwitchView(View::Tools)),
            KeyCode::PageUp => Action::Update(Msg::ScrollUp(PAGE)),
            KeyCode::PageDown => Action::Update(Msg::ScrollDown(PAGE)),
            KeyCode::Up => Action::Update(Msg::ScrollUp(1)),
            KeyCode::Down => Action::Update(Msg::ScrollDown(1)),
            _ => Action::None,
        };
    }

    // Chat view — text input + the existing bindings.
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

    // ---- view navigation ----

    #[test]
    fn tab_cycles_views_from_any_view() {
        let chat = AppState::new();
        assert_eq!(
            key_to_action(key(KeyCode::Tab), &chat),
            Action::Update(Msg::NextView)
        );
        assert_eq!(
            key_to_action(key(KeyCode::BackTab), &chat),
            Action::Update(Msg::PrevView)
        );
        // Also works inside a panel.
        let mut panel = AppState::new();
        panel.view = View::Dashboard;
        assert_eq!(
            key_to_action(key(KeyCode::Tab), &panel),
            Action::Update(Msg::NextView)
        );
    }

    #[test]
    fn panels_take_no_text_input_but_digits_jump() {
        let mut s = AppState::new();
        s.view = View::Audit;
        // A letter does not type in a panel.
        assert_eq!(key_to_action(key(KeyCode::Char('a')), &s), Action::None);
        // Digit jumps directly to a view (2 = Missions, 3 = Dashboard).
        assert_eq!(
            key_to_action(key(KeyCode::Char('2')), &s),
            Action::Update(Msg::SwitchView(View::Missions))
        );
        assert_eq!(
            key_to_action(key(KeyCode::Char('3')), &s),
            Action::Update(Msg::SwitchView(View::Dashboard))
        );
        // Esc returns to Chat.
        assert_eq!(
            key_to_action(key(KeyCode::Esc), &s),
            Action::Update(Msg::SwitchView(View::Chat))
        );
    }

    #[test]
    fn missions_panel_arrows_move_the_selection() {
        let mut s = AppState::new();
        s.view = View::Missions;
        assert_eq!(
            key_to_action(key(KeyCode::Down), &s),
            Action::Update(Msg::MissionSelectNext)
        );
        assert_eq!(
            key_to_action(key(KeyCode::Up), &s),
            Action::Update(Msg::MissionSelectPrev)
        );
        // Other panels still scroll on the arrows.
        s.view = View::Audit;
        assert_eq!(
            key_to_action(key(KeyCode::Down), &s),
            Action::Update(Msg::ScrollDown(1))
        );
    }

    #[test]
    fn chat_view_still_types() {
        // The Chat view is unchanged: printable chars insert.
        let s = AppState::new();
        assert_eq!(
            key_to_action(key(KeyCode::Char('a')), &s),
            Action::Update(Msg::InsertChar('a'))
        );
    }

    // ---- Chapter L.6 — Missions-panel gate keys ----

    fn missions_with_phase(phase: crate::model::MissionPhase, gate: Option<&str>) -> AppState {
        let mut s = AppState::new();
        s.view = View::Missions;
        s.missions.rows = vec![crate::model::MissionRow {
            id: "m-1".into(),
            title: "t".into(),
            lead: "coordinator".into(),
            phase,
            progress: 0,
            steps: vec![],
            pending_gate: gate.map(str::to_string),
        }];
        s
    }

    #[test]
    fn awaiting_mission_maps_approve_and_reject_keys() {
        let s = missions_with_phase(
            crate::model::MissionPhase::AwaitingApproval,
            Some("approve"),
        );
        assert_eq!(key_to_action(key(KeyCode::Char('a')), &s), Action::ResolveTeamGate(true));
        assert_eq!(key_to_action(key(KeyCode::Char('y')), &s), Action::ResolveTeamGate(true));
        assert_eq!(key_to_action(key(KeyCode::Char('r')), &s), Action::ResolveTeamGate(false));
        // Arrows still move the selection.
        assert_eq!(
            key_to_action(key(KeyCode::Down), &s),
            Action::Update(Msg::MissionSelectNext)
        );
    }

    #[test]
    fn non_awaiting_mission_leaves_gate_keys_inert() {
        // An executing mission isn't gated — a/r/y do nothing.
        let s = missions_with_phase(crate::model::MissionPhase::Executing, None);
        assert_eq!(key_to_action(key(KeyCode::Char('a')), &s), Action::None);
        assert_eq!(key_to_action(key(KeyCode::Char('r')), &s), Action::None);
    }

    // ---- Chapter L — new-mission compose box ----

    #[test]
    fn n_opens_the_new_mission_compose_box() {
        let s = missions_with_phase(crate::model::MissionPhase::Executing, None);
        assert_eq!(
            key_to_action(key(KeyCode::Char('n')), &s),
            Action::Update(Msg::MissionComposeOpen)
        );
    }

    #[test]
    fn compose_box_captures_typing_enter_and_esc() {
        let mut s = missions_with_phase(crate::model::MissionPhase::Executing, None);
        s.mission_compose = Some("clo".into());
        // Letters (incl. those that are panel hotkeys when closed) type.
        assert_eq!(
            key_to_action(key(KeyCode::Char('n')), &s),
            Action::Update(Msg::MissionComposeChar('n'))
        );
        assert_eq!(
            key_to_action(key(KeyCode::Backspace), &s),
            Action::Update(Msg::MissionComposeBackspace)
        );
        assert_eq!(key_to_action(key(KeyCode::Enter), &s), Action::SubmitMission);
        assert_eq!(
            key_to_action(key(KeyCode::Esc), &s),
            Action::Update(Msg::MissionComposeCancel)
        );
    }

    #[test]
    fn input_is_inert_while_a_mission_is_starting() {
        let mut s = missions_with_phase(crate::model::MissionPhase::Executing, None);
        s.mission_starting = true;
        assert_eq!(key_to_action(key(KeyCode::Char('a')), &s), Action::None);
        assert_eq!(key_to_action(key(KeyCode::Enter), &s), Action::None);
    }
}
