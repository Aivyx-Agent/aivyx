//! # aivyx-config
//!
//! Configuration loading and validation for Aivyx. Handles parsing,
//! schema validation, and secret-aware fields (API keys go through
//! `aivyx-storage` via `KeyDomain::Secrets` rather than plain config).
//!
//! See DESIGN.md Deliverable 6 (`AivyxError::Config` variant) and
//! Deliverable 7 (storage secrets flow).
//!
//! ## Status: Phase 0 stub only
//!
//! Nothing implemented yet. Phase 1 will add the config schema,
//! TOML loading, and the secret-field resolution to the Secrets domain.

#![allow(dead_code)]
