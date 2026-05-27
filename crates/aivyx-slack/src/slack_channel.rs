//! Phase 108 Task 4 — `SlackChannel` `ChannelContext` impl.
//! This file is a stub during Task 2; Task 4 fills it in.
//!
//! Structure mirrors `DiscordChannel` (Phase 107) and
//! `TelegramChannel` (Phase 8):
//!
//! - `SlackChannel<T: SlackTransport + 'static>` holding
//!   `Box<dyn SlackTransport>`, an outgoing-buffer
//!   `Mutex<String>`, and a per-turn `CancellationToken`.
//! - `trust_tier()` → `TrustTier::SemiTrusted`.
//! - `platform()` → `ChannelPlatform::Slack` (variant already
//!   in `aivyx-core` since Phase 8's forward-enumeration).
//! - `session_partition()` →
//!   `Some(format!("{team_id}:{channel_id}"))` per Q3a — the
//!   four-data-point confirmation that `Option<String>` is the
//!   right partition return type. A future Matrix adapter with
//!   `room_id + homeserver` is when the Phase 9 Q7
//!   richer-type question forces an answer.
//! - `cancellation_token()` / `reset_cancellation()` follow
//!   the Phase 18/19 per-turn-token rotation pattern.
//!
//! See `crates/aivyx-discord/src/discord_channel.rs` for the
//! canonical reference implementation Task 4 mirrors.
