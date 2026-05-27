//! Phase 107 Task 4 — `DiscordChannel` `ChannelContext`
//! impl. This file is a stub during Task 2; Task 4 fills it
//! in.
//!
//! The structure mirrors `TelegramChannel` (Phase 8):
//!
//! - A `DiscordChannel` struct holding a `Box<dyn
//!   DiscordTransport>`, an outgoing-buffer
//!   `Mutex<Vec<String>>` for the per-turn flush, and a
//!   per-turn `CancellationToken`.
//! - `trust_tier()` → `TrustTier::SemiTrusted` per the
//!   `docs/ADAPTER_PATTERN.md` tier-selection guidance.
//! - `platform()` → `ChannelPlatform::Discord` (variant
//!   already in `aivyx-core` since Phase 8).
//! - `session_partition()` → `Some(channel_id.to_string())`
//!   so DMs and channel messages get distinct memory
//!   partitions (matches Telegram's `chat_id`-based
//!   partitioning).
//! - `cancellation_token()` / `reset_cancellation()` follow
//!   the Phase 18/19 per-turn-token rotation pattern.
//!
//! See `crates/aivyx-telegram/src/telegram_channel.rs` for
//! the canonical reference implementation.
