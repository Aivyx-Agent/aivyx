//! Phase 108 Task 3 — `SlackTransport` trait + production
//! `SlackMorphismTransport` + scripted `ScriptedTransport`.
//! This file is a stub during Task 2; Task 3 fills it in.
//!
//! Trait surface mirrors `DiscordTransport` (Phase 107) and
//! `TelegramTransport` (Phase 8):
//!
//! - `next_message()` → `IncomingMessage` (Socket Mode
//!   WebSocket reads).
//! - `send_message(msg)` → `Result<(), TransportError>`
//!   (Slack REST `chat.postMessage`).
//!
//! `IncomingMessage` carries `team_id: String`,
//! `channel_id: String`, `user_id: String`, `text: String`,
//! `message_ts: String`. All Slack IDs are strings natively
//! (unlike Discord's u64 snowflakes or Telegram's i64
//! chat_ids), which is what makes the
//! `format!("{team_id}:{channel_id}")` partition key (Q3a)
//! a natural fit.
//!
//! Production impl wraps slack-morphism's `SlackClient` for
//! REST and `SlackClientSocketModeListener` for Socket Mode
//! events. Test impl is a scripted double living in
//! `transport.rs` (same convention Discord followed at
//! Phase 107).
