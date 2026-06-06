//! `aivyx-tui` — the terminal UI frontend for Aivyx (Phase 185,
//! Chapter I #1).
//!
//! The daemon is the agent; this crate is just another **frontend
//! client** over the local Unix-socket IPC — exactly like the REPL
//! and the Web UI. It connects via `DaemonSession`, submits input,
//! and renders the same `StreamEventPayload` stream the REPL prints,
//! only into `ratatui` widgets instead of a byte sink. No daemon,
//! capability, or audit concern lives here: this is a pure render +
//! interaction layer.
//!
//! ## Quarantine invariant
//!
//! This is the **only** crate that depends on `ratatui` / `crossterm`
//! (plus the binary that launches it). The substrate crates stay
//! dependency-clean; see the workspace manifest's Phase 185 note.
//!
//! ## Layout
//!
//! - [`model`] — the pure, terminal-free core: [`AppState`], the
//!   [`update`](model::update) reducer, and the
//!   [`lines_from_event`](model::lines_from_event) mapping. Unit-tested
//!   in CI.
//! - [`event`] — the pure keystroke → [`Action`](event::Action)
//!   translation that feeds the reducer. Unit-tested in CI.
//! - [`render`] — the ratatui layout (chat pane + status bar + input).
//!   Smoke-tested headlessly via `TestBackend`; visually
//!   operator-verified.
//! - [`terminal`] — the [`Tui`](terminal::Tui) RAII guard with
//!   panic-safe raw-mode / alternate-screen restore. Operator-verified.
//! - [`app`] — [`run`](app::run): the async event loop that connects
//!   the daemon (auto-spawning if needed), reads keys, and performs
//!   submit / cancel / gate-resolve round-trips. The `aivyx tui`
//!   command dispatches into it. Operator-verified.

pub mod app;
pub mod event;
pub mod model;
pub mod render;
pub mod terminal;

pub use app::run;
pub use event::{key_to_action, Action};
pub use model::{update, AppState, ChatLine, LineKind, Msg, PendingGate, Status};
pub use render::render;
pub use terminal::Tui;
