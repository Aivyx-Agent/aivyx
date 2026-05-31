//! `aivyx_core::Tool` implementations for the Notion tool
//! process.
//!
//! Phase 130 Q1a operator-picked surface (7 tools).
//! Per-tool modules ship in Tasks 3-9.

pub mod get_page;
pub mod search;

pub use get_page::NotionGetPage;
pub use search::NotionSearch;
