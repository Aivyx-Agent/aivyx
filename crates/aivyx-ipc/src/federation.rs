//! Federation relay protocol — the wasm-clean wire **shape** (FED.3 / Chapter
//! Passport PP.3).
//!
//! `docs/FEDERATION.md` §5's design rule: *build the relay verbs "as if the peer
//! on the other side might be a stranger's agent on another machine," so Nexus
//! is an extension of the local protocol, not a rewrite.* Nonagon's
//! intra-operator delegation and a future cross-operator relay are the **same
//! protocol**; the trust boundary is the only difference. These verbs live here
//! in the wasm-clean protocol substrate (no crypto, no transport — just the
//! payload) so a browser Nexus client and the native daemon share one shape.
//!
//! The **signing envelope** (`SignedHeader`) and the Ed25519 sign/verify live in
//! `aivyx-federation`; this module is the body those carry. Authority on receipt
//! is computed by `aivyx-federation`'s cross-operator attenuation (FED.2), never
//! here.

use serde::{Deserialize, Serialize};

/// The three federation verbs (`docs/FEDERATION.md` §5) — one shape, two
/// boundaries. Tagged by `verb` so the wire form is self-describing and stable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verb", rename_all = "snake_case")]
pub enum RelayRequest {
    /// Agent ↔ agent dialogue (salvage `RelayChatRequest`).
    Chat { message: String },
    /// "Do this for me" — **gated + attenuated** on receipt (salvage
    /// `RelayTaskRequest`). The goal is the work; what the peer may actually do
    /// to fulfil it is the FED.2 intersection, never what it asked for.
    Task { goal: String },
    /// Share / search procedures + public knowledge (salvage
    /// `FederatedSearchRequest`). The **privacy line** holds: only shareable
    /// artifacts (procedures, capabilities, public personas) ever cross — never
    /// the operator's private memory or data.
    Search { query: String },
}

impl RelayRequest {
    /// Stable, peer-facing verb label for audit + logging.
    pub fn verb(&self) -> &'static str {
        match self {
            RelayRequest::Chat { .. } => "chat",
            RelayRequest::Task { .. } => "task",
            RelayRequest::Search { .. } => "search",
        }
    }
}

/// The response payload for each verb. Only shareable content crosses back (the
/// privacy line applies in both directions).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "verb", rename_all = "snake_case")]
pub enum RelayResponse {
    /// A reply to [`RelayRequest::Chat`].
    Chat { reply: String },
    /// The disposition of a delegated [`RelayRequest::Task`] — `accepted` is
    /// false when my operator's gate or trust policy refuses it.
    Task { accepted: bool, detail: String },
    /// Results for [`RelayRequest::Search`] — shareable artifacts only.
    Search { results: Vec<String> },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verb_labels_are_stable() {
        assert_eq!(RelayRequest::Chat { message: "hi".into() }.verb(), "chat");
        assert_eq!(RelayRequest::Task { goal: "x".into() }.verb(), "task");
        assert_eq!(RelayRequest::Search { query: "q".into() }.verb(), "search");
    }

    #[test]
    fn request_roundtrips_through_self_describing_json() {
        let req = RelayRequest::Task { goal: "summarize the docs".into() };
        let json = serde_json::to_string(&req).unwrap();
        // self-describing: the verb tag is on the wire
        assert!(json.contains("\"verb\":\"task\""));
        let back: RelayRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back, req);
    }

    #[test]
    fn response_roundtrips() {
        for resp in [
            RelayResponse::Chat { reply: "ok".into() },
            RelayResponse::Task { accepted: false, detail: "gate refused".into() },
            RelayResponse::Search { results: vec!["proc-a".into(), "proc-b".into()] },
        ] {
            let json = serde_json::to_string(&resp).unwrap();
            let back: RelayResponse = serde_json::from_str(&json).unwrap();
            assert_eq!(back, resp);
        }
    }
}
