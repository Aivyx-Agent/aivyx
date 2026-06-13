//! Native Ollama provider implementation of [`LlmProvider`].
//!
//! Phase 121 — replaces the Phase 25 / Phase 34 path that routed
//! `ProviderKind::Ollama` through the OpenAI-compat
//! `/v1/chat/completions` endpoint. This module targets Ollama's
//! native `/api/chat` endpoint and JSONL streaming protocol
//! directly, giving Ollama-specific options (`num_ctx`,
//! `num_predict`, `num_thread`, `mirostat`) first-class treatment
//! and removing the OpenAI-compat translation layer for local-LLM
//! operators.
//!
//! Gated behind the `provider-ollama` Cargo feature. Coexists
//! with `provider-openai`: the binary's dispatch picks the
//! provider based on `ProviderKind` (Phase 121 Task 6 wires
//! `ProviderKind::Ollama` to this adapter by default; explicit
//! `ProviderKind::OpenAi` against an Ollama base_url is still
//! valid for operators who want OpenAI-compat behavior
//! specifically).
//!
//! ## Layout
//!
//! - [`provider`] — Phase 121 Task 2: `OllamaProvider` struct,
//!   `OllamaConfig`, `OllamaOptions`, request body builder.
//! - JSONL streaming line reader, stream state machine, and
//!   `LlmProvider::chat_stream` integration land in Tasks 3-5.
//!
//! Reuses `HttpTransport` and `ByteStream` from
//! `crate::transport` — the transport seam stays provider-
//! agnostic.

pub mod jsonl;
pub mod provider;
pub mod stream;

pub use jsonl::JsonlReader;
pub use provider::{
    build_request_body, OllamaConfig, OllamaOptions, OllamaProvider,
    AUTO_NUM_CTX_CAP, DEFAULT_OLLAMA_BASE_URL, RECOMMENDED_LOCAL_MODEL,
};
pub use stream::OllamaStream;
