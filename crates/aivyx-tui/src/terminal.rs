//! Terminal lifecycle — Phase 185 Task 3.
//!
//! [`Tui`] owns the raw-mode + alternate-screen state and restores it
//! on `Drop` *and* on panic (via a panic hook installed at `init`), so
//! a crash never leaves the operator's terminal in raw mode. This is
//! the load-bearing deliverable of Task 3 — it cannot be unit-tested
//! headlessly (there is no terminal in CI), so it stays small and
//! obviously correct, and the live behaviour is operator-verified.

use std::io::{self, Stdout};

use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::model::AppState;
use crate::render::render;

/// An initialized terminal: raw mode on, alternate screen entered,
/// with a panic hook that restores both before the default hook runs.
pub struct Tui {
    terminal: Terminal<CrosstermBackend<Stdout>>,
}

impl Tui {
    /// Enter raw mode + the alternate screen and install the
    /// panic-safe restore hook. Returns an error if the stream is not
    /// a real terminal (the caller falls back to the REPL).
    pub fn init() -> io::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;

        // Restore the terminal before the default panic handler prints
        // its message, so a panic doesn't strand the user in raw mode.
        let original = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = restore();
            original(info);
        }));

        let terminal = Terminal::new(CrosstermBackend::new(stdout))?;
        Ok(Self { terminal })
    }

    /// Render one frame for the current state.
    pub fn draw(&mut self, state: &AppState) -> io::Result<()> {
        self.terminal.draw(|frame| render(frame, state))?;
        Ok(())
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = restore();
    }
}

/// Leave the alternate screen and disable raw mode. Idempotent and
/// best-effort — safe to call from both `Drop` and the panic hook.
fn restore() -> io::Result<()> {
    execute!(io::stdout(), LeaveAlternateScreen)?;
    disable_raw_mode()?;
    Ok(())
}
