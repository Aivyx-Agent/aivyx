//! NT-02 — the capability-attenuation primitive.
//!
//! A specialist can **never exceed its lead**. Because the new core's
//! `Scope` / `CapabilitySet` already encode the grant relation, the port
//! of the archive's `attenuate_for_member` (which needed a whole
//! capability-object graph) collapses to a filter over `grants`.

use aivyx_capability::{CapabilitySet, Scope, TrustTier};

/// A specialist's effective capabilities = the scopes it **declares** that
/// the **lead actually grants**. Every scope in the result is granted by
/// the lead, so the result is always a subset of the lead's authority
/// (invariant **NT-02**). Scopes the lead doesn't grant are silently
/// dropped — declaring them buys a specialist nothing.
pub fn attenuate_for_member(lead: &CapabilitySet, declared: &[Scope]) -> CapabilitySet {
    CapabilitySet::from_scopes(declared.iter().filter(|s| lead.grants(s)).cloned())
}

/// A specialist's effective trust tier — capped at the lead's. The member
/// may *declare* a ceiling, but it is floored to the lead's tier so a
/// specialist is never more trusted than the agent that convened it.
/// (`TrustTier`'s `Ord` is ascending trust, so `min` is the floor.)
pub fn effective_trust(member_ceiling: TrustTier, lead_tier: TrustTier) -> TrustTier {
    member_ceiling.min(lead_tier)
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
    fn declared_subset_passes_through() {
        let lead = set(&["fs.read", "fs.write", "net.fetch"]);
        let got = attenuate_for_member(&lead, &[scope("fs.read"), scope("net.fetch")]);
        assert!(got.grants(&scope("fs.read")));
        assert!(got.grants(&scope("net.fetch")));
        assert!(!got.grants(&scope("fs.write")), "only what was declared");
    }

    #[test]
    fn scope_not_granted_by_lead_is_dropped() {
        // NT-02: a specialist cannot acquire a capability the lead lacks.
        let lead = set(&["fs.read"]);
        let got = attenuate_for_member(&lead, &[scope("fs.read"), scope("memory.write")]);
        assert!(got.grants(&scope("fs.read")));
        assert!(!got.grants(&scope("memory.write")), "lead never granted it");
    }

    #[test]
    fn result_is_always_a_subset_of_the_lead() {
        // The load-bearing invariant: every result scope is granted by the lead.
        let lead = set(&["fs.read", "memory.read"]);
        let declared = [
            scope("fs.read"),
            scope("memory.read"),
            scope("shell.exec"), // not in lead → must not survive
            scope("net.fetch"),  // not in lead → must not survive
        ];
        let got = attenuate_for_member(&lead, &declared);
        for s in got.iter() {
            assert!(lead.grants(s), "specialist scope {s:?} exceeds the lead");
        }
        assert!(!got.grants(&scope("shell.exec")));
        assert!(!got.grants(&scope("net.fetch")));
    }

    #[test]
    fn attenuates_qualified_path_scopes_within_the_lead() {
        // The realistic delegation case: the lead holds a broad path glob;
        // a specialist declares a sub-path (kept) and an outside path (dropped).
        let lead = CapabilitySet::from_scopes([scope("fs.read:/project/**")]);

        let within = attenuate_for_member(&lead, &[scope("fs.read:/project/docs/notes.md")]);
        assert!(
            within.grants(&scope("fs.read:/project/docs/notes.md")),
            "a sub-path of the lead's glob is within authority"
        );

        let outside = attenuate_for_member(&lead, &[scope("fs.read:/etc/passwd")]);
        assert!(
            !outside.grants(&scope("fs.read:/etc/passwd")),
            "NT-02: a path outside the lead's glob is dropped"
        );
        for s in outside.iter() {
            assert!(lead.grants(s));
        }
    }

    #[test]
    fn empty_declared_yields_empty() {
        let lead = set(&["fs.read", "fs.write"]);
        let got = attenuate_for_member(&lead, &[]);
        assert!(!got.grants(&scope("fs.read")));
    }

    #[test]
    fn empty_lead_grants_nothing() {
        let lead = CapabilitySet::empty();
        let got = attenuate_for_member(&lead, &[scope("fs.read")]);
        assert!(!got.grants(&scope("fs.read")), "no parent authority to inherit");
    }

    #[test]
    fn trust_is_floored_to_the_lead() {
        // A member declaring Kernel under a Trusted lead is capped at Trusted.
        assert_eq!(
            effective_trust(TrustTier::Kernel, TrustTier::Trusted),
            TrustTier::Trusted
        );
        // A member declaring less than the lead keeps its lower ceiling.
        assert_eq!(
            effective_trust(TrustTier::SemiTrusted, TrustTier::Trusted),
            TrustTier::SemiTrusted
        );
    }
}
