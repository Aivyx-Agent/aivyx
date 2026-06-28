//! Identity (FED.1 / Chapter Passport PP.1) — *who is this agent?*
//!
//! An operator-owned **Ed25519 keypair per instance** (sovereignty: the
//! identity is the operator's, not the company's), and the **signed,
//! replay-guarded request envelope** every cross-boundary request carries.
//!
//! Salvage plan (PP.0 audit): the archived `auth.rs` lifts almost verbatim —
//! `FederationAuth` (generate / load-or-generate-encrypted, signing +
//! verification), `SignedHeader` (`{instance_id, timestamp, sig over
//! id:ts:body_hash}`), and `ReplayGuard` (timestamp-window dedup). The only
//! changes: swap `aivyx_core::AivyxError` → [`crate::FederationError`], and wrap
//! the key at rest with a key derived from the storage `MasterKey` (reuse the
//! salvage's ChaCha20-Poly1305 `load_or_generate_encrypted` path). Keep, intact:
//! the manual `Debug` that redacts the signing key, the `0o600` key-file
//! permissions, and the `instance_id` charset validation — **key material is
//! never logged** (the identity invariant).
//!
//! Filled in PP.1.
