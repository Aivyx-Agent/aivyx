//! `aivyx-telegram` — Phase 8's Telegram `ChannelContext` adapter.
//!
//! This crate is the second concrete [`ChannelContext`] implementation
//! after `LocalChannel`. It exists to close the D2 loop: the channel
//! abstraction has been waiting for its second impl since Phase 3, and
//! Phase 7's hardening work (persistent audit, memory tripwire,
//! `chmod 0600`, interactive passphrase) finally made it safe to put
//! the agent behind a network-facing surface.
//!
//! ## Why Telegram, and why `frankenstein`
//!
//! Telegram is the simplest credible non-local channel — one bot
//! token, long-poll-or-webhook, a small message model, no OAuth to
//! invent. See `docs/ROADMAP.md` for the full Matrix/Discord/Slack
//! comparison.
//!
//! `frankenstein` is a deliberately-thin wrapper around the Bot API
//! (just serde types + an `AsyncTelegramApi` trait) rather than a
//! framework like `teloxide`. We want that thinness: **aivyx's turn
//! loop is already the dispatcher**, and a framework that owns its
//! own event loop would fight the agent's control flow. Frankenstein
//! exposes `get_updates` and `send_message` as plain async calls, and
//! that's all we need.
//!
//! ## Trust tier
//!
//! [`TelegramChannel`] advertises [`TrustTier::SemiTrusted`] — D4's
//! "authenticated user on a remote channel." A Telegram chat carries
//! a stable `chat_id` + `user_id` (authenticated by Telegram's own
//! backend), so it is *not* [`TrustTier::Untrusted`], which is
//! reserved for anonymous-webhook-style surfaces.
//!
//! The `SemiTrusted` ceiling (see `aivyx-capability::CEILING_SEMITRUSTED`)
//! grants `memory.read`, `memory.write`, `llm.call`, `llm.embed`,
//! `net.fetch`, `net.dns`, `fs.metadata`, and `config.read`. That is
//! the right minimum for a bot that can answer questions, persist
//! memory, and do web lookups, but cannot (for example) write local
//! files — which matches the "Telegram is a remote hand, not a local
//! shell" mental model.
//!
//! Phase 8's ROADMAP and PHASE_8.md entry draft said `Untrusted`.
//! That was wrong: `CEILING_UNTRUSTED` only grants
//! `memory.read:scope:public:*` + `audit.read:public`, which is
//! insufficient for any useful Telegram bot. The correction is
//! recorded here in Task 1's ship record per the Phase 7 convention
//! of "correct in the task that lands, don't retro-edit frozen entry
//! docs."
//!
//! ## Transport indirection
//!
//! The real `frankenstein::AsyncTelegramApi` implementation is held
//! behind a private [`TelegramTransport`] trait so unit tests can
//! swap in a `ScriptedTransport` test double that drives the channel
//! with pre-canned updates and captures outgoing messages into a
//! `Vec`. The production impl (`ReqwestTransport`) is a thin adapter
//! over `frankenstein::client_reqwest::Bot`; none of the Task 1
//! tests exercise it, which is the whole point of the trait seam.

#![forbid(unsafe_code)]

// Phase 8 Task 1 lands the scaffold; the `TelegramChannel` struct,
// the private transport trait, and the scripted test double ship in
// the next two writes below.

// `TelegramChannel` is `pub(crate)` in Phase 8 Task 1 because the
// private [`transport::TelegramTransport`] trait is a real dependency
// of the public type — exposing the struct would leak the private
// bound. Task 4 (binary wiring) is where a public constructor lands,
// and it will build a concrete `TelegramChannel<ReqwestTransport>`
// behind an opaque handle — the transport trait will stay private.
//
// The type is imported directly from `telegram_channel` in the tests
// module; no re-export at the lib root is needed yet.
mod telegram_channel;
mod transport;

// Phase 8 Task 4 — the Telegram analogue of
// `aivyx_channel::run_session`. Owns its own long-poll loop.
mod session;

pub use session::{run_telegram_session, TelegramSessionConfig, TelegramSessionReport};

#[cfg(test)]
mod tests;
