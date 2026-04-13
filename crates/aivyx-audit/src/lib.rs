//! # aivyx-audit
//!
//! HMAC-chained append-only audit log for Aivyx agents.
//!
//! Every tool call, scope check, and turn outcome is appended to this
//! log synchronously as it happens — not batched at turn end. If the
//! process crashes mid-turn, the audit log still tells the truth about
//! what got executed.
//!
//! See DESIGN.md Deliverable 1 (audit is synchronous inline) and
//! Deliverable 4 (the `AuditEvent` enum — per-tool for grants, per-scope
//! for denials, with a dedicated `MemoryAccess` view).
//!
//! ## Status: Phase 0 stub only
//!
//! Nothing implemented yet. Phase 1 will add the `AuditWriter` trait,
//! the HMAC chain mechanics, and the `AuditEvent` enum.

#![allow(dead_code)]
