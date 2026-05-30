//! Tool-trait implementations registered by the IPC harness.
//!
//! Phase 123 — Chapter F #1. Each tool wraps a
//! [`crate::GmailClient`] and implements
//! [`aivyx_core::Tool`] so it can be served by the multi-tool
//! IPC harness (see [`crate::harness`]).
//!
//! ## Module layout
//!
//! - [`search`] — Task 4: `gmail.search` (email.read scope).
//! - [`read`] — Task 5: `gmail.read` (email.read scope).
//! - [`draft`] — Task 6: `gmail.draft` (email.write scope).
//! - `send` — Task 7: `gmail.send` (email.send scope, Trusted-
//!   gated).

pub mod draft;
pub mod read;
pub mod search;

pub use draft::GmailDraft;
pub use read::GmailRead;
pub use search::GmailSearch;
