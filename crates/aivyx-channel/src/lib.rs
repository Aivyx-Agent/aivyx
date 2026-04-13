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
//! ## Phase 1 status
//!
//! Re-exports only. Phase 1 task 4 will add `LocalChannel` — the CLI
//! reference implementation that the first end-to-end turn-loop test
//! runs through.

#![allow(dead_code)]

pub use aivyx_core::{
    AttachmentKind, ChannelContext, ChannelError, ChannelPlatform, StreamEvent,
};
