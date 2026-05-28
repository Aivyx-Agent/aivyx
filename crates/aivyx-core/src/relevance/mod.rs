//! Phase 116 — Tool/skill selection learning from outcomes.
//!
//! Q1a's keyword-extraction primitive + (in future tasks) the
//! relevance-ledger surface. The agent's tool/skill selection
//! has been pure LLM intuition since Phase 0; Chapter E #3
//! builds the outcome-feedback substrate that augments it.
//!
//! ## Module layout
//!
//! - `keywords` (Task 2, this commit) — `extract_keywords`
//!   and `keyword_key` pure functions. Top-K alphanumeric
//!   tokens lowercased, stopword-filtered, length-ordered.
//!   Zero LLM cost; deterministic.
//!
//! The persistent ledger + system-prompt extension live in
//! `aivyx-channel` because they need `KeyDomain` storage and
//! the assemble-prompt call site. This `aivyx-core` module
//! ships the pure primitive so the storage-side caller can
//! depend on it without dep-cycling.

pub mod keywords;

pub use keywords::{extract_keywords, keyword_key};
