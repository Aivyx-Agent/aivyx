//! `aivyx-discord` — Discord ChannelContext adapter (Phase 107).
//!
//! Third concrete channel after `LocalChannel` (Phase 0) and
//! `TelegramChannel` (Phase 8). The crate's shape mirrors
//! `aivyx-telegram` deliberately — same module layout
//! (`discord_channel`, `transport`, `session`, `tests`), same
//! private-2-method-transport-trait seam, same
//! `run_*_session` + `_with_transport` sibling pattern. See
//! `docs/ADAPTER_PATTERN.md` for the substrate contract every
//! adapter honors.
//!
//! ## Task structure
//!
//! Phase 107 ships across six sub-tasks; this `lib.rs` is the
//! Task 2 skeleton. Each module starts as a one-paragraph
//! stub documenting what lands in which task; subsequent
//! tasks fill the modules in.

pub mod discord_channel;
pub mod session;
pub mod transport;

#[cfg(test)]
mod tests;
