//! # aivyx-crypto
//!
//! Cryptographic primitives for Aivyx: ChaCha20-Poly1305 (AEAD),
//! HKDF-SHA256 (key derivation), and Argon2id (passphrase hashing).
//!
//! This crate is pure commodity — it wraps well-reviewed Rust crypto
//! crates with Aivyx-specific conveniences. No novel crypto.
//!
//! See DESIGN.md Deliverable 7 for the crypto stack and the versioned
//! HKDF salt (`"aivyx-v1-storage"`) that gates key derivation.
//!
//! ## Status: Phase 0 stub only
//!
//! Nothing implemented yet. Phase 1 will add wrapper types for the
//! master key, domain subkeys, and the encryption/decryption helpers.

#![allow(dead_code)]
