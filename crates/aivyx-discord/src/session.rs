//! Phase 107 Task 5 — `run_discord_session` driver. This
//! file is a stub during Task 2; Task 5 fills it in.
//!
//! Two entry points (mirrors `aivyx-telegram` Phase 8/9):
//!
//! - `run_discord_session(config)` — production entry point.
//!   Constructs a `TwilightTransport`, wraps a
//!   `DiscordChannel`, drives the outer Gateway-event loop
//!   per chat partition, demultiplexes inbound
//!   `Event::MessageCreate` events to per-channel-id turns.
//! - `run_discord_session_with_transport(config, transport)`
//!   — test-facing inner. The scripted e2e suite drives
//!   this directly with `ScriptedTransport`.
//!
//! The planner / agent / audit construction is identical
//! copy from `aivyx_telegram::session` (~50 lines per
//! `docs/ADAPTER_PATTERN.md`); the outer loop is fresh —
//! Discord's `Shard::next_event` stream replaces Telegram's
//! `get_updates` long-poll.
//!
//! `/approve` and `/reject` mission-gate commands route to
//! the existing Phase 21 mission-gate plumbing — text-
//! pattern match on inbound, same regex as Telegram, no
//! Discord-native slash-command registration (Q3a at
//! sign-off).
