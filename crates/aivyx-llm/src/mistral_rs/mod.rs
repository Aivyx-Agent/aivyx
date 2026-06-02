//! Phase 134 — embedded Rust-native inference via the
//! `mistralrs` crate.
//!
//! Direction B from the Phase 133 research note. Aivyx
//! can now run a local LLM **inside its own process**
//! by linking against `mistralrs`. The operator loads
//! a GGUF model from a local path; the model runs
//! in-process; zero outbound network calls during
//! inference (strongest possible privacy posture short
//! of air-gapping).
//!
//! ## Layout
//!
//! - [`provider`] — `MistralRsProvider` struct +
//!   `MistralRsConfig` + `LlmProvider` impl bridging
//!   Aivyx's request/response shape onto mistralrs's
//!   `GgufModelBuilder` / `stream_chat_request` API.
//! - [`convert`] — pure functions converting between
//!   Aivyx's `LlmMessage` / `LlmToolDescriptor` and
//!   mistralrs's `TextMessages` / `Tool` types.
//!   Substrate code; no async, no IO — directly unit-
//!   testable.
//!
//! ## Phase 134 scope cap
//!
//! Phase 134 ships a credible bridge: GGUF model
//! loading, text-only chat (no multimodal yet),
//! streaming text tokens, basic tool calling
//! (non-streamed tool args). What's deferred to
//! Phase 135+:
//!
//! - Multimodal (image / audio / video content blocks).
//!   mistralrs supports these; Aivyx's existing
//!   `ContentBlock::ImageBase64` would need converting.
//! - Streamed tool-call argument deltas. mistralrs
//!   surfaces full tool calls on the response object;
//!   Aivyx's planner accepts buffered tool calls
//!   today, so this isn't a blocker.
//! - Backend-specific tuning (`PagedAttention`
//!   configuration, ISQ-bit selection beyond the
//!   defaults). The provider exposes hooks; aggressive
//!   tuning is operator-driven.
//! - End-to-end hardware validation. Requires an
//!   operator-supplied GGUF model + matching
//!   backend (CPU / Metal / CUDA). Phase 134 ships
//!   the bridge under unit tests; Phase 135 codifies
//!   operator-reported empirical signal.

pub mod convert;
pub mod provider;

pub use provider::{MistralRsConfig, MistralRsProvider};
