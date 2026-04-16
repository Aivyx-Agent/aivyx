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

mod local;
pub mod passphrase;
mod render;
mod role_envelope;
mod role_render;
mod session;

pub use local::LocalChannel;
pub use render::{render_finalize, render_stream_event, RenderMode};
pub use role_envelope::{assemble_role_envelope, MAX_INHERITANCE_DEPTH};
pub use role_render::{render_role_envelope, ChannelKind};
pub use session::{run_session, SessionConfig, SessionReport};
