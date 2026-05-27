//! Phase 107 Task 6 — scripted e2e suite. This file is a
//! stub during Task 2; Task 6 fills it in.
//!
//! Planned coverage (mirrors `aivyx-telegram`'s `tests.rs`):
//!
//! - `discord_session_smoke_e2e` — one-turn round-trip
//!   through `ScriptedTransport`, asserts the inbound
//!   message reached the agent and the outgoing reply was
//!   captured.
//! - `discord_two_chats_persistent_e2e` — two channel_id
//!   partitions, one redb store, asserts memory
//!   partitioning works the same way it does for Telegram.
//! - `discord_approve_command_resolves_gate_e2e` — agent
//!   escalates to a mission gate; operator scripts `/approve`
//!   as inbound; turn resumes.
//! - `discord_cancellation_token_rotates_between_turns` —
//!   the per-turn token shape Phase 18/19 introduced.
//!
//! Real-protocol smoke against a live Discord bot is
//! **deferred to the Channel Activation Milestone** per
//! `docs/ADAPTER_PATTERN.md` checklist item 7.
