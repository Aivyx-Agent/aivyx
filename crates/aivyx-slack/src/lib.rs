//! `aivyx-slack` — Slack ChannelContext adapter (Phase 108).
//!
//! Fourth concrete channel after `LocalChannel` (Phase 0),
//! `TelegramChannel` (Phase 8), and `DiscordChannel` (Phase
//! 107). The crate's shape mirrors `aivyx-discord` deliberately
//! — same module layout (`slack_channel`, `transport`,
//! `session`, `tests`), same private-2-method-transport-trait
//! seam, same multi-channel multiplexer + per-channel mailbox
//! inner task pattern. See `docs/ADAPTER_PATTERN.md` for the
//! substrate contract every adapter honors.
//!
//! ## Task structure
//!
//! Phase 108 ships across six sub-tasks; this `lib.rs` is the
//! Task 2 skeleton. Each module starts as a one-paragraph stub
//! documenting what subsequent task fills it in.

pub mod session;
pub mod slack_channel;
pub mod transport;

#[cfg(test)]
mod tests;

// Phase 108 Task 5 — public surface the `aivyx` binary
// consumes from the `--channel slack` dispatch arm. Mirrors
// `aivyx-discord` / `aivyx-telegram` re-export shape.
pub use session::{
    run_slack_session, SlackMultiSessionReport, SlackSessionConfig, SlackSessionReport,
};
