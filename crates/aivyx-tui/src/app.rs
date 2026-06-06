//! The async terminal driver — Phase 185 Task 4.
//!
//! [`run`] is the TUI's entry point: it connects a [`DaemonSession`]
//! (auto-spawning the daemon if none is listening — exactly like the
//! REPL), enters the terminal via [`Tui`], and runs the event loop
//! that ties the pure [`crate::model`] / [`crate::event`] cores to
//! daemon I/O. Keystrokes become [`Action`]s; pure actions update the
//! state in-process, while submit / cancel / gate-resolve perform a
//! daemon round-trip and then apply the matching reducer message.
//!
//! The loop body is the integration surface — it talks to a live
//! daemon and a live terminal, so it is **operator-verified** rather
//! than unit-tested. The pieces it orchestrates ([`key_to_action`],
//! [`update`], [`render`](crate::render)) are tested in their own
//! modules. The pure CLI seams (`tui` arg parse) are tested in the
//! binary.

use std::path::PathBuf;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};

use aivyx_channel::daemon_client::{
    spawn_daemon_and_wait, DaemonCancelHandle, DaemonSession,
};
use aivyx_channel::daemon_ipc::{FrontendType, StreamEventPayload};

use crate::event::{key_to_action, Action};
use crate::model::{update, AppState, Msg};
use crate::terminal::Tui;

/// How long to wait for an auto-spawned daemon to come up. Matches the
/// REPL's `daemon_session` budget.
const AUTO_SPAWN_TIMEOUT: Duration = Duration::from_secs(10);

/// Poll cadence for terminal key events. Short enough that an
/// in-flight turn cancels promptly; long enough to idle near-zero CPU.
const POLL: Duration = Duration::from_millis(100);

/// Connect (auto-spawning the daemon if needed), enter the terminal,
/// and drive the TUI to completion. Restores the terminal on every
/// exit path (including panics, via [`Tui`]'s panic hook).
pub async fn run(socket_path: PathBuf, role: Option<String>) -> Result<(), String> {
    // Connect, or spawn a daemon and connect — same dance as the REPL.
    let mut session = match DaemonSession::connect(
        &socket_path,
        role.clone(),
        Some(FrontendType::Local),
    )
    .await
    {
        Ok(s) => s,
        Err(_) => {
            spawn_daemon_and_wait(&socket_path, AUTO_SPAWN_TIMEOUT)
                .await
                .map_err(|e| e.to_string())?;
            DaemonSession::connect(&socket_path, role.clone(), Some(FrontendType::Local))
                .await
                .map_err(|e| e.to_string())?
        }
    };
    let cancel = session.cancel_handle();

    let mut tui = Tui::init().map_err(|e| format!("terminal init: {e}"))?;
    let mut state = update(AppState::new(), Msg::Connected { role });

    let loop_result = run_loop(&mut tui, &mut session, &cancel, &mut state).await;

    // Restore the terminal *before* the disconnect / error surfaces.
    drop(tui);
    let _ = session.disconnect().await;
    loop_result
}

/// Apply a pure reducer message to the borrowed state in place.
fn apply(state: &mut AppState, msg: Msg) {
    *state = update(std::mem::take(state), msg);
}

/// The render → read-key → act loop. Returns on `Quit` (or a terminal
/// I/O error).
async fn run_loop(
    tui: &mut Tui,
    session: &mut DaemonSession,
    cancel: &DaemonCancelHandle,
    state: &mut AppState,
) -> Result<(), String> {
    loop {
        tui.draw(state).map_err(|e| format!("draw: {e}"))?;
        if state.should_quit {
            return Ok(());
        }

        let key = wait_for_key().await;
        match key_to_action(key, state) {
            Action::None | Action::Cancel => {
                // `Cancel` is only meaningful during a turn (handled in
                // `run_turn`); at idle there's nothing to cancel.
            }
            Action::Update(msg) => apply(state, msg),
            Action::Quit => apply(state, Msg::Quit),
            Action::Submit => {
                let Some(text) = state.submittable() else {
                    continue;
                };
                apply(state, Msg::Submit); // echo operator line + working
                tui.draw(state).map_err(|e| format!("draw: {e}"))?;
                match run_turn(session, cancel, text).await {
                    Ok((events, outcome)) => {
                        apply(state, Msg::TurnFinished { events, outcome })
                    }
                    Err(e) => apply(state, Msg::Error(e)),
                }
            }
            Action::ResolveGate(approved) => {
                if let Some(gate) = state.gate.clone() {
                    match session
                        .resolve_gate(gate.mission_id, gate.gate_id, approved)
                        .await
                    {
                        Ok(()) => apply(state, Msg::GateResolved { approved }),
                        Err(e) => apply(state, Msg::Error(e.to_string())),
                    }
                }
            }
        }
    }
}

/// Submit a turn and await its collected events, while concurrently
/// watching for an Esc / Ctrl-C that cancels the in-flight turn. The
/// daemon ends the turn early on `CancelTurn`, so the submit future
/// resolves with whatever it had plus the cancelled outcome.
async fn run_turn(
    session: &mut DaemonSession,
    cancel: &DaemonCancelHandle,
    text: String,
) -> Result<(Vec<StreamEventPayload>, String), String> {
    let turn = session.submit_input(text);
    tokio::pin!(turn);
    loop {
        tokio::select! {
            res = &mut turn => return res.map_err(|e| e.to_string()),
            maybe_key = poll_key(POLL) => {
                if let Some(k) = maybe_key {
                    if is_cancel_key(&k) {
                        cancel.cancel().await;
                    }
                }
            }
        }
    }
}

/// Esc or Ctrl-C — the in-turn cancel binding.
fn is_cancel_key(k: &KeyEvent) -> bool {
    matches!(k.code, KeyCode::Esc)
        || (k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL))
}

/// Block (without busy-spinning the terminal) until a key event
/// arrives, redraw-free between polls so an idle TUI uses ~no CPU.
async fn wait_for_key() -> KeyEvent {
    loop {
        if let Some(k) = poll_key(POLL).await {
            return k;
        }
    }
}

/// Poll for a single key event with a timeout, off the async runtime's
/// worker threads (crossterm's poll/read are blocking). Returns `None`
/// on timeout or a non-key event.
async fn poll_key(timeout: Duration) -> Option<KeyEvent> {
    tokio::task::spawn_blocking(move || match event::poll(timeout) {
        Ok(true) => match event::read() {
            Ok(Event::Key(k)) => Some(k),
            _ => None,
        },
        _ => None,
    })
    .await
    .ok()
    .flatten()
}
