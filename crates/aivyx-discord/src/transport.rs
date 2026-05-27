//! Phase 107 Task 3 — `DiscordTransport` trait + production
//! `TwilightTransport` + scripted-double `ScriptedTransport`.
//! This file is a stub during Task 2; Task 3 fills it in.
//!
//! The trait surface will be exactly two methods, mirroring
//! `TelegramTransport`:
//!
//! ```text
//! pub(crate) trait DiscordTransport: Send + Sync {
//!     async fn next_message(&mut self)
//!         -> Result<IncomingMessage, TransportError>;
//!     async fn send_message(&self, msg: OutgoingMessage)
//!         -> Result<(), TransportError>;
//! }
//! ```
//!
//! Production impl wraps `twilight_gateway::Shard` (which
//! owns the Gateway state machine: identify, heartbeat,
//! sequence tracking, resume) and `twilight_http::Client`
//! for the single REST verb the channel issues
//! (`create_message`). Test impl is a scripted double with
//! the same capture-buffer-plus-replay-queue shape as
//! `aivyx-telegram`'s `ScriptedTransport`.
