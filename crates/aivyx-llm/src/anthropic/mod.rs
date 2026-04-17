//! Anthropic Messages-API reference implementation of [`LlmProvider`].
//!
//! Gated behind the `provider-anthropic` Cargo feature. Downstream crates
//! that only need the `LlmProvider` trait and `LlmError` type (e.g.
//! `aivyx-core` for its `AivyxError::Llm` variant) do not enable this
//! feature and therefore do not link `reqwest`, `rustls`, or `secrecy`.
//!
//! ## Layout
//!
//! - [`transport`] — the `HttpTransport` trait and the real
//!   `ReqwestTransport`. This is the seam that makes the provider
//!   testable offline: tests inject a fake transport that replays
//!   canned SSE bytes.
//! - [`sse`] — hand-rolled SSE parser that groups `\n\n`-delimited
//!   frames into `SseEvent { event, data }` values. Deliberately narrow:
//!   it does not implement the full Server-Sent Events spec, only the
//!   shape Anthropic actually emits.
//! - [`provider`] — the `AnthropicProvider` struct, the wire-format
//!   structs, and the streaming state machine that turns SSE events into
//!   `LlmStreamEvent` / `LlmStepEnd` values.
//!
//! ## What a real API smoke test looks like
//!
//! Not wired yet — will land as an `#[ignore]`-by-default integration
//! test under `tests/` that reads `ANTHROPIC_API_KEY` from the env and
//! calls a real `claude-haiku-4-5-20251001`. Run manually with
//! `cargo test --features provider-anthropic -- --ignored`.

pub mod provider;
pub mod sse;

pub use crate::transport::{ByteStream, HttpTransport, ReqwestTransport};
pub use provider::{AnthropicConfig, AnthropicProvider};
