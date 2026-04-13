//! # aivyx-llm
//!
//! The `LlmProvider` trait and reference implementations (Anthropic and
//! — optionally — Ollama) for Aivyx. The turn loop in `aivyx-core`
//! holds a provider and drives its chat/chat_stream methods during
//! each turn's tool-calling loop.
//!
//! See DESIGN.md Deliverable 1 (the paragraph mentions "its LlmProvider")
//! and Deliverable 6 (`AivyxError::Llm(LlmError)` wraps provider errors).
//!
//! ## Status: Phase 0 stub only
//!
//! Nothing implemented yet. The `LlmProvider` trait shape is carried
//! over from the archived codebase with no design changes — Phase 1
//! will re-derive it.

#![allow(dead_code)]
