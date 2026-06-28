//! Trust + attenuation (FED.2 / Chapter Passport PP.2) — *how much may a peer do?*
//!
//! Per-peer [`TrustPolicy`], **deny-by-default** (no policy ⇒ the peer can do
//! nothing). It carries the capability bases a peer may ever invoke, in the new
//! core's [`Scope`] vocabulary, plus a **peer autonomy ceiling** = the maximum
//! Reins [`AutonomyLevel`] a peer's relayed request may run at (default
//! [`Manual`](AutonomyLevel::Manual) — confirm-first; the archive's `Leash`).
//!
//! The enforced authority of any peer-relayed action is the **intersection** —
//! the exact NT-02 team-attenuation rule, lifted across the operator boundary:
//!
//! ```text
//! effective(peer request) =
//!       what the peer asked for
//!     ∩ my TrustPolicy.allowed_scopes for that peer
//!     ∩ my channel/trust-tier ceiling
//!     ∩ my [autonomy] cap for peers
//! ```
//!
//! This reuses [`CapabilitySet`]/[`Scope::is_granted_by`] (the same machinery
//! behind `aivyx-team`'s `attenuate_for_member`), so a peer is *structurally
//! incapable* of exceeding what **both** sides allow. Trust is **revocable**:
//! tighten or drop the policy and the peer's reach narrows immediately.
//!
//! Modernized from the salvage `config.rs::TrustPolicy` — `allowed_scopes:
//! Vec<String>` → `Vec<Scope>`, `max_tier: AutonomyTier` → `AutonomyLevel`. The
//! transport-only config (`url`/`bearer_token`/`failover`) is **not** lifted.

use aivyx_capability::{CapabilitySet, Scope};
use aivyx_config::AutonomyLevel;
use serde::Deserialize;

/// Per-peer trust policy. **Deny-by-default lives at the lookup layer** (no
/// policy for a peer ⇒ [`effective_authority`] denies); this type expresses what
/// a peer *with* a policy may reach.
#[derive(Debug, Clone, Deserialize)]
pub struct TrustPolicy {
    /// The capability bases this peer may *ever* invoke, in the new-core
    /// [`Scope`] vocabulary (e.g. `["memory.read", "skills.read"]`). Only the
    /// peer's asked-for scopes that fall within these — and within *my* local
    /// ceiling — are ever granted.
    pub allowed_scopes: Vec<Scope>,

    /// The maximum [`AutonomyLevel`] a relayed request from this peer may run
    /// at. Defaults to [`Manual`](AutonomyLevel::Manual) (confirm every
    /// peer-initiated action) — a peer can never push my agent past it.
    #[serde(default = "default_peer_ceiling")]
    pub peer_ceiling: AutonomyLevel,
}

/// The conservative default peer ceiling: confirm-first (the archive's `Leash`).
/// A peer with no explicit ceiling still cannot act without my operator's gate.
pub fn default_peer_ceiling() -> AutonomyLevel {
    AutonomyLevel::Manual
}

impl TrustPolicy {
    /// A policy granting exactly `allowed_scopes` at the confirm-first ceiling.
    pub fn new(allowed_scopes: impl IntoIterator<Item = Scope>) -> Self {
        Self {
            allowed_scopes: allowed_scopes.into_iter().collect(),
            peer_ceiling: default_peer_ceiling(),
        }
    }

    /// Set the peer autonomy ceiling (builder-style).
    pub fn with_ceiling(mut self, ceiling: AutonomyLevel) -> Self {
        self.peer_ceiling = ceiling;
        self
    }
}

/// The computed authority of a single peer-relayed request — the result of the
/// cross-operator intersection. Always a subset of what *both* sides allow.
#[derive(Debug, Clone)]
pub struct EffectiveAuthority {
    /// The scopes actually granted for this request (∅ ⇒ fully denied).
    pub scopes: CapabilitySet,
    /// The autonomy level this request may run at — the more restrictive of the
    /// peer ceiling and my own peer-autonomy cap.
    pub autonomy: AutonomyLevel,
}

impl EffectiveAuthority {
    /// Total denial — no scopes, confirm-first. The deny-by-default result.
    pub fn denied() -> Self {
        Self {
            scopes: CapabilitySet::empty(),
            autonomy: AutonomyLevel::Manual,
        }
    }

    /// True when nothing was granted (the peer may do nothing).
    pub fn is_denied(&self) -> bool {
        self.scopes.iter().next().is_none()
    }
}

/// **NT-02 generalized across the operator boundary.** Compute what a peer's
/// relayed request may actually do on *my* instance:
///
/// `effective = asked ∩ policy.allowed_scopes ∩ local_ceiling`, at autonomy
/// `min(policy.peer_ceiling, autonomy_cap)`.
///
/// - `policy = None` ⇒ [`EffectiveAuthority::denied`] (**deny-by-default** — a
///   peer with no `TrustPolicy` can do nothing).
/// - `local_ceiling` is *my* side's authority (e.g. the channel/trust-tier
///   ceiling) — a peer can never exceed what my own agent could do.
/// - `autonomy_cap` is my `[autonomy]` cap for peers; the result is floored to
///   the more restrictive of it and the peer ceiling.
pub fn effective_authority(
    asked: &[Scope],
    policy: Option<&TrustPolicy>,
    local_ceiling: &CapabilitySet,
    autonomy_cap: AutonomyLevel,
) -> EffectiveAuthority {
    let Some(policy) = policy else {
        return EffectiveAuthority::denied();
    };
    let policy_set = CapabilitySet::from_scopes(policy.allowed_scopes.iter().cloned());
    // asked ∩ policy ∩ local ceiling — each asked scope must be granted by BOTH
    // the peer's policy and my own authority (the double intersection is what
    // makes a peer structurally incapable of exceeding either side).
    let scopes = CapabilitySet::from_scopes(
        asked
            .iter()
            .filter(|s| policy_set.grants(s) && local_ceiling.grants(s))
            .cloned(),
    );
    EffectiveAuthority {
        scopes,
        autonomy: more_restrictive(policy.peer_ceiling, autonomy_cap),
    }
}

/// Ascending *permissiveness* rank — `Manual` (0) is the most restrictive,
/// `Unleashed` (4) the least. (`AutonomyLevel` derives `Eq` but not `Ord`, so
/// the floor is computed here rather than via `min`.)
fn permissiveness(level: AutonomyLevel) -> u8 {
    match level {
        AutonomyLevel::Manual => 0,
        AutonomyLevel::Assisted => 1,
        AutonomyLevel::Supervised => 2,
        AutonomyLevel::Autonomous => 3,
        AutonomyLevel::Unleashed => 4,
    }
}

/// The more restrictive (lower-permissiveness) of two levels — the autonomy
/// floor across the boundary.
fn more_restrictive(a: AutonomyLevel, b: AutonomyLevel) -> AutonomyLevel {
    if permissiveness(a) <= permissiveness(b) {
        a
    } else {
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(s: &str) -> Scope {
        Scope::parse(s).expect("known base")
    }
    fn set(scopes: &[&str]) -> CapabilitySet {
        CapabilitySet::from_scopes(scopes.iter().map(|s| scope(s)))
    }

    #[test]
    fn no_policy_denies_everything() {
        // Deny-by-default: a peer with no TrustPolicy gets nothing, even when
        // my own ceiling is broad.
        let eff = effective_authority(
            &[scope("memory.read")],
            None,
            &set(&["memory.read", "fs.read"]),
            AutonomyLevel::Autonomous,
        );
        assert!(eff.is_denied());
        assert_eq!(eff.autonomy, AutonomyLevel::Manual);
    }

    #[test]
    fn grants_only_the_intersection_of_asked_policy_and_local() {
        let policy = TrustPolicy::new([scope("memory.read"), scope("net.fetch")]);
        let local = set(&["memory.read", "fs.read"]); // my ceiling lacks net.fetch
        let eff = effective_authority(
            &[scope("memory.read"), scope("net.fetch"), scope("fs.read")],
            Some(&policy),
            &local,
            AutonomyLevel::Assisted,
        );
        // memory.read: asked ∩ policy ∩ local ✓
        assert!(eff.scopes.grants(&scope("memory.read")));
        // net.fetch: in policy + asked but NOT in my local ceiling → dropped
        assert!(!eff.scopes.grants(&scope("net.fetch")));
        // fs.read: asked + local but NOT in the peer's policy → dropped
        assert!(!eff.scopes.grants(&scope("fs.read")));
    }

    #[test]
    fn asking_beyond_policy_buys_nothing() {
        let policy = TrustPolicy::new([scope("memory.read")]);
        let local = set(&["memory.read", "memory.write", "fs.read"]);
        let eff = effective_authority(
            &[scope("memory.write")], // never in the policy
            Some(&policy),
            &local,
            AutonomyLevel::Autonomous,
        );
        assert!(eff.is_denied(), "a scope outside the policy is never granted");
    }

    #[test]
    fn autonomy_is_floored_to_the_more_restrictive_side() {
        let policy = TrustPolicy::new([scope("memory.read")]).with_ceiling(AutonomyLevel::Supervised);
        let local = set(&["memory.read"]);
        // peer ceiling Supervised, my cap Assisted → Assisted (more restrictive)
        let eff = effective_authority(
            &[scope("memory.read")],
            Some(&policy),
            &local,
            AutonomyLevel::Assisted,
        );
        assert_eq!(eff.autonomy, AutonomyLevel::Assisted);
        // and the reverse: peer ceiling Manual caps a broad local autonomy
        let strict = TrustPolicy::new([scope("memory.read")]); // default Manual
        let eff2 = effective_authority(
            &[scope("memory.read")],
            Some(&strict),
            &local,
            AutonomyLevel::Unleashed,
        );
        assert_eq!(eff2.autonomy, AutonomyLevel::Manual);
    }

    #[test]
    fn default_ceiling_is_confirm_first() {
        assert_eq!(TrustPolicy::new([]).peer_ceiling, AutonomyLevel::Manual);
    }

    #[test]
    fn revocation_narrows_reach_immediately() {
        let local = set(&["memory.read", "memory.write"]);
        let asked = [scope("memory.read"), scope("memory.write")];
        // broad policy grants both
        let broad = TrustPolicy::new([scope("memory.read"), scope("memory.write")]);
        let before = effective_authority(&asked, Some(&broad), &local, AutonomyLevel::Assisted);
        assert!(before.scopes.grants(&scope("memory.write")));
        // tighten the policy → write is gone on the very next request
        let tightened = TrustPolicy::new([scope("memory.read")]);
        let after = effective_authority(&asked, Some(&tightened), &local, AutonomyLevel::Assisted);
        assert!(after.scopes.grants(&scope("memory.read")));
        assert!(!after.scopes.grants(&scope("memory.write")));
    }

    #[test]
    fn policy_deserializes_with_default_ceiling() {
        // allowed_scopes given, peer_ceiling omitted → defaults to Manual.
        let policy: TrustPolicy =
            serde_json::from_str(r#"{"allowed_scopes":["memory.read"]}"#).unwrap();
        assert_eq!(policy.peer_ceiling, AutonomyLevel::Manual);
        assert_eq!(policy.allowed_scopes.len(), 1);
        // explicit ceiling round-trips through the lowercase wire name.
        let p2: TrustPolicy = serde_json::from_str(
            r#"{"allowed_scopes":["memory.read"],"peer_ceiling":"supervised"}"#,
        )
        .unwrap();
        assert_eq!(p2.peer_ceiling, AutonomyLevel::Supervised);
    }
}
