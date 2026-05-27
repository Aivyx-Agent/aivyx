//! Phase 108 Task 5 — `run_slack_session` driver. This file
//! is a stub during Task 2; Task 5 fills it in.
//!
//! Two entry points (mirrors `aivyx-discord` Phase 107):
//!
//! - `run_slack_session(config)` — production entry point.
//!   Constructs a `SlackMorphismTransport` from the operator's
//!   bot + app tokens, wraps the Socket Mode shard, drives
//!   the multi-channel multiplexer loop.
//! - `run_slack_session_with_transport(config, transport)` —
//!   test-facing inner. The scripted e2e suite drives this
//!   directly with `ScriptedTransport`.
//!
//! Slack's Socket Mode is push-based like Discord's Gateway,
//! so the session driver follows the Discord-shape:
//! - One outer multiplexer pumps the Socket Mode WebSocket
//!   via `next_message`.
//! - Per-channel mailbox inner tasks routed by
//!   `format!("{team_id}:{channel_id}")` partition keys.
//! - No `scan_for_cancel` probe (Slack events arrive through
//!   the same WebSocket; the inner task's biased select
//!   against `mailbox.recv()` is the cancel detector).
//! - No `get_updates` cursor (push-based protocol).
//!
//! `/cancel` / `/approve` / `/reject` text-command routing is
//! the Phase-108-internal deferral that bundles with the
//! Phase 107 daemon-frontend follow-on. The in-process path
//! Task 5 lands handles `/cancel` at the mailbox boundary;
//! `/approve` and `/reject` route through the daemon-frontend
//! once it lands.
