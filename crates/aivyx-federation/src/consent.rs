//! Operator consent for peer-initiated effect (FED.5 / Chapter Passport PP.5).
//!
//! `docs/FEDERATION.md` §4 / §7: *a peer can **request**; only the operator's
//! gate lets it **act**.* Anything with effect — a delegated task, a "hire", a
//! touch of an irreversible/outbound capability — routes through the operator's
//! existing confirm-first / approval-gate machinery before it can have effect.
//!
//! This module **classifies** a crossing; it does not reimplement the gate. The
//! daemon's existing confirm-first / [`AUTONOMY.md`](../../../docs/AUTONOMY.md)
//! posture machinery resolves an [`OperatorConsent`](ConsentRequirement::OperatorConsent)
//! (prompt / batch-approve / reject-in-headless) when transport wires a live
//! crossing — exactly as a local irreversible tool call is resolved today.
//!
//! **No cross-boundary self-escalation** (FED.0 §7): a peer can never *lower*
//! this requirement. The consent decision and the authority both come only from
//! *my* operator — the peer's request is input, never authority. (The authority
//! is already structurally bounded by [`effective_authority`](crate::trust::effective_authority),
//! which intersects with the host's own caps; this layer adds the gate on top.)

use aivyx_capability::is_irreversible_base;
use aivyx_ipc::RelayRequest;

use crate::received::PeerContent;

/// Whether a peer crossing may act without the operator, must be gated, or is
/// refused outright.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsentRequirement {
    /// Read-only / dialogue (a `chat` reply, a `search` over shareable
    /// artifacts) under attenuated read scopes — no per-crossing gate.
    NotRequired,
    /// Peer-initiated **effect** — route through the operator's confirm-first /
    /// approval gate before acting. `reason` is the prompt context.
    OperatorConsent { reason: String },
    /// The peer was granted nothing (deny-by-default) — refuse.
    Denied,
}

/// Classify whether handling `content` needs the operator's consent before it
/// can have effect.
///
/// - Denied content (empty authority) ⇒ [`Denied`](ConsentRequirement::Denied).
/// - A `task` (a peer asking my agent to *act on its behalf*) ⇒ always
///   [`OperatorConsent`](ConsentRequirement::OperatorConsent).
/// - Any crossing whose *attenuated* authority includes an irreversible /
///   outbound base ([`is_irreversible_base`]) ⇒ `OperatorConsent` (defense in
///   depth — even a `chat`/`search` that somehow holds such a scope is gated).
/// - Otherwise (read-only dialogue / knowledge) ⇒
///   [`NotRequired`](ConsentRequirement::NotRequired).
pub fn consent_for(content: &PeerContent) -> ConsentRequirement {
    if content.is_denied() {
        return ConsentRequirement::Denied;
    }
    let is_task = matches!(content.request(), RelayRequest::Task { .. });
    let touches_irreversible = content
        .authority()
        .scopes
        .iter()
        .any(|s| is_irreversible_base(s.base()));
    if is_task || touches_irreversible {
        ConsentRequirement::OperatorConsent {
            reason: format!(
                "peer '{}' requested a '{}' that would act on your instance — \
                 your approval is required before it runs",
                content.provenance().peer_id,
                content.verb(),
            ),
        }
    } else {
        ConsentRequirement::NotRequired
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::received::Provenance;
    use crate::trust::{TrustPolicy, effective_authority};
    use aivyx_capability::{CapabilitySet, Scope};
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

    /// Build a PeerContent for a request, attenuated to `granted` scopes.
    fn content(req: RelayRequest, granted: &[&str]) -> PeerContent {
        let policy = TrustPolicy::new(granted.iter().map(|s| scope(s)));
        let host = set(granted);
        let asked: Vec<Scope> = granted.iter().map(|s| scope(s)).collect();
        let auth = effective_authority(&asked, Some(&policy), &host, AutonomyLevel::Assisted);
        PeerContent::accept(prov(), auth, req).unwrap()
    }

    #[test]
    fn denied_content_is_refused() {
        let pc = PeerContent::accept(
            prov(),
            crate::trust::EffectiveAuthority::denied(),
            RelayRequest::Task { goal: "x".into() },
        )
        .unwrap();
        assert_eq!(consent_for(&pc), ConsentRequirement::Denied);
    }

    #[test]
    fn a_task_always_needs_operator_consent() {
        // even a read-only goal: a peer asking my agent to act is effect.
        let pc = content(RelayRequest::Task { goal: "read and report".into() }, &["memory.read"]);
        assert!(matches!(consent_for(&pc), ConsentRequirement::OperatorConsent { .. }));
    }

    #[test]
    fn chat_and_search_are_read_only_no_gate() {
        let chat = content(RelayRequest::Chat { message: "hello".into() }, &["memory.read"]);
        assert_eq!(consent_for(&chat), ConsentRequirement::NotRequired);
        let search = content(RelayRequest::Search { query: "pour-over".into() }, &["memory.read"]);
        assert_eq!(consent_for(&search), ConsentRequirement::NotRequired);
    }

    #[test]
    fn irreversible_scope_gates_even_a_chat() {
        // defense in depth: a crossing holding net.post (outbound) is gated
        // regardless of verb.
        let pc = content(RelayRequest::Chat { message: "ping".into() }, &["net.post"]);
        assert!(matches!(consent_for(&pc), ConsentRequirement::OperatorConsent { .. }));
    }

    #[test]
    fn consent_reason_names_the_peer_and_verb() {
        let pc = content(RelayRequest::Task { goal: "do it".into() }, &["memory.read"]);
        match consent_for(&pc) {
            ConsentRequirement::OperatorConsent { reason } => {
                assert!(reason.contains("peer-1"));
                assert!(reason.contains("task"));
            }
            other => panic!("expected OperatorConsent, got {other:?}"),
        }
    }
}
