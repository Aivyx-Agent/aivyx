//! # aivyx-channel
//!
//! The `LocalChannel` reference implementation and future remote-channel
//! adapters (Telegram/Discord/Slack/Matrix/Email).
//!
//! ## Where `ChannelContext` lives
//!
//! **The `ChannelContext` trait itself lives in `aivyx-core`**, not here.
//! This is a Phase 1 task 3 layering decision: `ToolContext` inside
//! `aivyx-core` needs to hold `&dyn ChannelContext`, and moving the trait
//! out of core would create a cycle (`aivyx-core` → `aivyx-channel` →
//! `aivyx-core`). The same reasoning applied to `ChannelPlatform` in
//! task 2.
//!
//! This crate re-exports the trait + helpers so that channel-crate
//! consumers have a stable import path (`use aivyx_channel::ChannelContext`).
//!
//! See DESIGN.md Deliverable 1 (the "no bypass" commitment) and
//! Deliverable 3 (the `ChannelContext` trait sketch).
//!
//! ## Status
//!
//! Phase 1 shipped re-exports only. Phase 3 task 1 adds [`LocalChannel`]
//! — the CLI reference implementation that streams tokens to stdout
//! (or any `std::io::Write` sink, for tests).

pub use aivyx_core::{
    AttachmentKind, ChannelContext, ChannelError, ChannelPlatform, StreamEvent,
};

pub mod daemon_client;
pub mod daemon_ipc;
pub mod daemon_scheduler;
pub mod daemon_server;
pub mod mission;
pub mod mission_tool;
pub mod schedule;
pub mod schedule_tool;
pub mod trigger;
pub mod webhook;
pub mod webhook_listener;
pub mod webhook_tool;
pub mod file_watch;
pub mod file_watch_tool;
pub mod file_watcher;
pub mod memory_gc_tool;
pub mod prune_sink;
pub mod reflection_tool;
pub mod role_overrides;
pub mod role_update_tool;
pub mod turn_history_tool;
pub mod ollama_tools;
/// Phase 62 — Agent-Initiated Outbound Notifications. The
/// dispatcher and `NotifyBackend` trait live here; per-kind
/// backend impls live in `notify_telegram` (Phase 62 Task 5) and
/// `notify_webhook` (Phase 62 Task 6); the `notify.send` tool
/// lives in `notify_tool` (Phase 62 Task 7).
pub mod notify_dispatcher;
pub mod notify_telegram;
mod daemon_session;
mod local;
pub mod passphrase;
pub mod persona;
pub mod profile_prompt;
mod render;
mod role_envelope;
mod role_render;
mod session;
pub mod telegram_daemon_frontend;
pub mod web_ui;

pub use local::LocalChannel;
pub use render::{render_finalize, render_stream_event, RenderMode};
pub use profile_prompt::assemble_session_prompt;
pub use role_envelope::{assemble_role_envelope, MAX_INHERITANCE_DEPTH};
pub use role_render::{render_role_envelope, ChannelKind};
pub use daemon_session::{run_daemon_session, run_daemon_session_connected, DaemonSessionConfig};
pub use session::{run_session, SessionConfig, SessionReport};
