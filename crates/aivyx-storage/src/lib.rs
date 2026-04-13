//! # aivyx-storage
//!
//! Encrypted redb-based storage for Aivyx, with HKDF-derived subkeys
//! per data domain (Sessions, Memory, Audit, Secrets, ChannelState).
//!
//! See DESIGN.md Deliverable 7 for the storage stack and trait sketch.
//!
//! ## Key commitments
//!
//! - Single-writer, single-process (redb file lock enforces this)
//! - One storage handle per process — not per request
//! - Passphrase is obtained by the channel adapter, not storage
//! - Versioned HKDF salt (`"aivyx-v1-storage"`) allows clean key rotation
//!
//! ## Status: Phase 0 stubs only

#![allow(dead_code)]

/// Placeholder for the `KeyDomain` enum. See DESIGN.md Deliverable 7.
///
/// Real variants (Phase 1):
/// `Sessions | Memory | Audit | Secrets | ChannelState`
pub struct KeyDomain;
