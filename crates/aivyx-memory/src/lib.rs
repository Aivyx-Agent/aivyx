//! # aivyx-memory
//!
//! Memory substrate for Aivyx agents. Implements the `memory.read`,
//! `memory.write`, and `memory.forget` tools that the turn loop invokes
//! when the LLM chooses to recall or persist.
//!
//! Per DESIGN.md Deliverable 1, memory is **a tool, not an ambient
//! system**. There is no hidden "memory injection" at turn start —
//! every memory access is an explicit, scope-checked, audited tool call.
//!
//! See also Deliverable 4 (the `memory.*` scope family) and
//! Deliverable 7 (`KeyDomain::Memory` for the encrypted substrate).
//!
//! ## Status: Phase 0 stub only
//!
//! Nothing implemented yet. Phase 1 will add the `Memory` trait, the
//! default redb-backed implementation, and the three tool wrappers.

#![allow(dead_code)]
