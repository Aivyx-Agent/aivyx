//! Phase 108 Task 6 — scripted e2e suite. This file is a
//! stub during Task 2; Task 6 fills it in.
//!
//! Planned coverage (mirrors `aivyx-discord`'s `tests.rs`):
//!
//! - `slack_session_smoke_e2e` — one (team_id, channel_id)
//!   partition, two scripted inbound messages, two real agent
//!   turns through a `ConcreteAgent` wired to a scripted
//!   `LlmProvider`.
//! - `slack_session_two_partitions_persistent_e2e` — two
//!   distinct `(team_id, channel_id)` pairs (proving the
//!   partition-key stringification handles the multi-workspace
//!   case Q3a anticipates). Asserts each partition gets its
//!   own inner task and outbound routing.
//! - `slack_session_shutdown_drains_inflight_turns` — the
//!   multi-channel multiplexer's shutdown-drain contract for
//!   Slack: one turn completes, shutdown fires, every
//!   per-channel sender drops, inner tasks resolve to
//!   `SlackSessionReport`.
//!
//! Real-protocol smoke against a live Slack bot is **deferred
//! to the Channel Activation Milestone** per
//! `docs/ADAPTER_PATTERN.md` checklist item 7.
