//! Peer content = untrusted input (FED.4 / Chapter Passport PP.4).
//!
//! `docs/FEDERATION.md` §4 / §7: a skill, a delegated task, or a message from a
//! peer is treated like any hostile input — **schema-validated**,
//! **provenance-tagged** (which peer, which key), and run **only under that
//! peer's attenuated capabilities** (the FED.2 intersection). Anything with
//! *effect* additionally needs operator consent (FED.5 / PP.5).
//!
//! The keystone primitive here is [`PeerContent`]: it can only be built by
//! [`PeerContent::accept`], which takes the already-verified request (PP.1) and
//! the already-resolved [`EffectiveAuthority`] (PP.2). So a `PeerContent` *by
//! construction* carries its provenance and a bounded authority — there is no
//! way to hold peer content without also holding the limits on what it may do.
//! Reading the payload is free; **acting** on it must pass [`PeerContent::may`].

use aivyx_capability::Scope;
use aivyx_ipc::RelayRequest;

use crate::FederationError;
use crate::trust::EffectiveAuthority;

/// Upper bound on any single peer-supplied string field. A peer is untrusted —
/// it does not get to hand us an unbounded `goal`/`message`/`query`. 16 KiB is
/// generous for a task/skill description while bounding abuse.
pub const MAX_PEER_FIELD_LEN: usize = 16 * 1024;

/// Who a piece of peer content came from. The **public key is the trust
/// anchor** (the `peer_id` is a convenience label); both are recorded so a
/// crossing is forensically attributable (FED.0 §6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    /// The peer's `instance_id` (label).
    pub peer_id: String,
    /// The peer's Ed25519 public key, base64 (the identity that was verified).
    pub peer_public_key: String,
}

/// Untrusted content received from a peer, bound to its [`Provenance`] and the
/// [`EffectiveAuthority`] under which it may be handled.
///
/// Constructed only via [`accept`](Self::accept) — *after* signature
/// verification and trust resolution — so it can never exist without both.
#[derive(Debug, Clone)]
pub struct PeerContent {
    provenance: Provenance,
    authority: EffectiveAuthority,
    request: RelayRequest,
}

impl PeerContent {
    /// Accept a verified, trust-resolved peer request as untrusted input.
    ///
    /// Preconditions the caller must already have met (encoded by the argument
    /// types, not re-checked here): the [`SignedHeader`](crate::identity::SignedHeader)
    /// was verified against the peer's key (PP.1) and not replayed, and
    /// `authority` is the [`effective_authority`](crate::trust::effective_authority)
    /// for *this* peer and request (PP.2). This validates the **payload** —
    /// rejecting over-long fields — so downstream handling never sees an
    /// unbounded peer string.
    pub fn accept(
        provenance: Provenance,
        authority: EffectiveAuthority,
        request: RelayRequest,
    ) -> Result<Self, FederationError> {
        validate_request(&request)?;
        Ok(Self { provenance, authority, request })
    }

    /// Who sent this.
    pub fn provenance(&self) -> &Provenance {
        &self.provenance
    }

    /// The payload (read-only — *reading* is free; *acting* must pass
    /// [`may`](Self::may)).
    pub fn request(&self) -> &RelayRequest {
        &self.request
    }

    /// The relay verb (`"chat"` / `"task"` / `"search"`).
    pub fn verb(&self) -> &'static str {
        self.request.verb()
    }

    /// **The execution gate (FED.4).** Handling this peer content may touch
    /// `scope` *only if* the peer's attenuated authority grants it — never the
    /// host's own broader caps. A peer with no/empty authority (deny-by-default)
    /// can touch nothing.
    pub fn may(&self, scope: &Scope) -> bool {
        self.authority.scopes.grants(scope)
    }

    /// True when the peer was granted nothing — the content may be read for the
    /// record but must not act.
    pub fn is_denied(&self) -> bool {
        self.authority.is_denied()
    }

    /// The bounded authority under which this content runs.
    pub fn authority(&self) -> &EffectiveAuthority {
        &self.authority
    }
}

/// Bound every peer-supplied string in a [`RelayRequest`]. Returns
/// [`FederationError::Validation`] for an over-long field.
fn validate_request(req: &RelayRequest) -> Result<(), FederationError> {
    let field = match req {
        RelayRequest::Chat { message } => message,
        RelayRequest::Task { goal } => goal,
        RelayRequest::Search { query } => query,
    };
    if field.len() > MAX_PEER_FIELD_LEN {
        return Err(FederationError::Validation(format!(
            "peer {} field is {} bytes, over the {MAX_PEER_FIELD_LEN}-byte limit",
            req.verb(),
            field.len()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trust::{TrustPolicy, effective_authority};
    use aivyx_capability::CapabilitySet;
    use aivyx_config::AutonomyLevel;

    fn scope(s: &str) -> Scope {
        Scope::parse(s).expect("known base")
    }
    fn set(scopes: &[&str]) -> CapabilitySet {
        CapabilitySet::from_scopes(scopes.iter().map(|s| scope(s)))
    }
    fn prov() -> Provenance {
        Provenance { peer_id: "peer-1".into(), peer_public_key: "AAAA".into() }
    }

    #[test]
    fn accept_carries_provenance_and_payload() {
        let auth = EffectiveAuthority::denied();
        let pc = PeerContent::accept(
            prov(),
            auth,
            RelayRequest::Chat { message: "hi".into() },
        )
        .unwrap();
        assert_eq!(pc.provenance().peer_id, "peer-1");
        assert_eq!(pc.verb(), "chat");
    }

    #[test]
    fn may_gates_to_the_attenuated_authority_not_the_host() {
        // peer policy allows memory.read; my host ceiling is broad.
        let policy = TrustPolicy::new([scope("memory.read")]);
        let host = set(&["memory.read", "memory.write", "fs.read"]);
        let auth = effective_authority(
            &[scope("memory.read")],
            Some(&policy),
            &host,
            AutonomyLevel::Assisted,
        );
        let pc = PeerContent::accept(prov(), auth, RelayRequest::Task { goal: "read memory".into() })
            .unwrap();
        // granted by the attenuated authority
        assert!(pc.may(&scope("memory.read")));
        // NOT granted, even though the HOST could do it — a peer can't borrow my caps
        assert!(!pc.may(&scope("memory.write")));
        assert!(!pc.may(&scope("fs.read")));
        assert!(!pc.is_denied());
    }

    #[test]
    fn deny_by_default_content_may_touch_nothing() {
        let pc = PeerContent::accept(
            prov(),
            EffectiveAuthority::denied(), // no TrustPolicy upstream
            RelayRequest::Task { goal: "do something".into() },
        )
        .unwrap();
        assert!(pc.is_denied());
        assert!(!pc.may(&scope("memory.read")));
    }

    #[test]
    fn over_long_peer_field_is_rejected() {
        let huge = "x".repeat(MAX_PEER_FIELD_LEN + 1);
        let err = PeerContent::accept(
            prov(),
            EffectiveAuthority::denied(),
            RelayRequest::Search { query: huge },
        );
        assert!(err.is_err(), "an over-long peer field must be rejected");
    }

    #[test]
    fn at_limit_peer_field_is_accepted() {
        let ok = "x".repeat(MAX_PEER_FIELD_LEN);
        assert!(
            PeerContent::accept(
                prov(),
                EffectiveAuthority::denied(),
                RelayRequest::Search { query: ok },
            )
            .is_ok()
        );
    }
}
