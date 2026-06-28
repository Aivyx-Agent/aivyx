//! Trust + attenuation (FED.2 / Chapter Passport PP.2) — *how much may a peer do?*
//!
//! Per-peer **`TrustPolicy`**, **deny-by-default** (no policy ⇒ the peer can do
//! nothing). It carries the capability bases a peer may ever invoke, in the new
//! core's [`aivyx_capability::Scope`] vocabulary, plus a **peer autonomy
//! ceiling** = the maximum Reins `AutonomyLevel` a peer's relayed request may
//! run at (default = confirm-first).
//!
//! The enforced authority of any peer-relayed action is the **intersection** —
//! NT-02 team-attenuation, lifted across the operator boundary:
//!
//! ```text
//! effective(peer request) =
//!       what the peer asked for
//!     ∩ my TrustPolicy.allowed_scopes for that peer
//!     ∩ my channel/trust-tier ceiling
//!     ∩ my [autonomy] cap for peers
//! ```
//!
//! Modernization (PP.0 audit): salvage `config.rs::TrustPolicy` —
//! `allowed_scopes: Vec<String>` → `Vec<Scope>`, `max_tier: AutonomyTier` →
//! `AutonomyLevel` (default confirm-first). The intersection reuses
//! [`aivyx_capability::Scope::is_granted_by`] / `CapabilitySet`, so a peer is
//! *structurally incapable* of exceeding what **both** sides allow; trust is
//! revocable by tightening or dropping the policy. The salvage's transport-only
//! config (`url` / `bearer_token` / `failover`, and `relay.rs` / `client.rs`)
//! is **not** lifted — that is the deferred Nexus transport.
//!
//! Filled in PP.2.
