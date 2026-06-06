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
//! - The terminal driver + render (Task 3) and the `aivyx tui` command
//!   wiring (Task 4) land in follow-on tasks; their visual behaviour is
//!   operator-verified (no terminal in CI).

pub mod model;

pub use model::{update, AppState, ChatLine, LineKind, Msg, PendingGate, Status};
