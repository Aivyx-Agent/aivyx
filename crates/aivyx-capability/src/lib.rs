//! # aivyx-capability
//!
//! Capability-based security for Aivyx. Defines the `Scope` type, the
//! `CapabilitySet`, the `TrustTier` enum, and the attenuation rules
//! that let trust tiers cap what an agent can do per turn.
//!
//! See DESIGN.md Deliverable 4 (capability taxonomy) and Deliverable 5
//! (trust tier model) for the locked design this crate implements.
//!
//! ## Key rule
//!
//! Effective capabilities for a turn are computed as:
//! `effective = agent.capabilities().intersect(tier.default_ceiling())`.
//! This intersection happens **once per turn**, before any LLM call.
//!
//! ## Status: Phase 0 stubs only

#![allow(dead_code)]

/// Placeholder for the `Scope` type. See DESIGN.md Deliverable 4.
pub struct Scope;

/// Placeholder for the `CapabilitySet` type. See DESIGN.md Deliverable 4.
pub struct CapabilitySet;

/// Placeholder for the `TrustTier` enum. See DESIGN.md Deliverable 5.
///
/// Real declaration order (Phase 1): `Untrusted, SemiTrusted, Trusted, Kernel`
/// so that derived `Ord` gives `Kernel > Trusted > SemiTrusted > Untrusted`.
pub struct TrustTier;
