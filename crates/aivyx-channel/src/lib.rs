//! # aivyx-channel
//!
//! The `ChannelContext` trait and the `LocalChannel` reference
//! implementation. Every agent turn is delivered through a channel —
//! there is **no bypass path**. The CLI, desktop GUI, REST API, and
//! remote channels (Telegram/Discord/Slack/Matrix/Email) all implement
//! this trait.
//!
//! See DESIGN.md Deliverable 1 (the "no bypass" commitment) and
//! Deliverable 3 (the `ChannelContext` trait sketch with `StreamEvent`,
//! `AttachmentKind`, and the cancellation token).
//!
//! ## Status: Phase 0 stub only
//!
//! Nothing implemented yet. Phase 1 will add the `ChannelContext` trait,
//! the `StreamEvent` enum, and the `LocalChannel` reference impl that
//! wires the CLI into the turn loop.

#![allow(dead_code)]
