//! Phase 70 — Persona proposal log (P14 self-learning closure).
//!
//! Persistent, HMAC-chained log of pending and resolved Persona
//! delta proposals. Parallel to [`crate::persona::PersistentPersonaLog`]
//! per Phase 70 Q4(a) sign-off: proposals are operator-pending
//! objects, deltas are operator-approved objects, so the persona
//! chain's invariant (every delta is operator-approved) stays
//! intact even as the agent's reflection auto-loop generates
//! candidate deltas asynchronously.
//!
//! ## Shape
//!
//! - [`PersonaProposal`] is the user-visible record: an `id`
//!   (stable across status transitions), the `proposed_op` the
//!   agent originally generated, the `source_reflection_session_id`
//!   that produced it, and a status of `Pending | Approved {…} |
//!   Rejected {…} | Superseded {…}`.
//! - [`SignedProposalEntry`] is what gets persisted: one row per
//!   status transition, HMAC-chained against the previous entry.
//!   `seq=0` is always a `Pending` for some id; later entries may
//!   carry `Approved` / `Rejected` / `Superseded` for the same
//!   id. The "current status of proposal X" is derived from the
//!   last entry referencing X.
//! - [`PersistentPersonaProposalLog`] reads every row at daemon
//!   startup, verifies the chain, then keeps an in-memory
//!   `ProposalChainLog` for hot-path lookups.
//!
//! ## Why status-as-new-entry rather than mutate-in-place?
//!
//! Two reasons. First, the HMAC chain only signs **immutable**
//! bytes — mutating a status field in place would break the
//! chain. Second, the proposal *history* is the audit story: an
//! operator who approves a proposal should see in the chain
//! exactly when it was proposed, when it was approved, and what
//! it was modified to before approval. Append-only preserves
//! that history.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use sha2::Sha256;

use aivyx_storage::DomainHandle;

use crate::persona::ProposedPersonaDelta;

// ---------------------------------------------------------------------------
// PersonaProposal — derived view returned by list/get APIs.
// ---------------------------------------------------------------------------

/// A single proposal record, status-derived from the chain.
/// `id` is stable across status transitions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaProposal {
    /// Stable proposal id. Generated when the proposal is first
    /// appended in `Pending` status; carries through every later
    /// status transition for the same proposal.
    pub id: String,
    /// Wall-clock when the proposal was first appended.
    pub proposed_at_unix_ms: u64,
    /// Session id of the reflection turn that produced the
    /// proposal. Lets operators trace a proposal back to the
    /// agent context that generated it.
    pub source_reflection_session_id: String,
    /// The proposal the agent originally generated. Distinct
    /// from `Approved::applied_op` so the audit trail captures
    /// any operator modification at approval time (Q3(a)).
    /// Phase 92's `supersedes_proposal_id` linkage lives on
    /// the inner `ProposedPersonaDelta` (chained) — the
    /// surface reads it via `proposed_op.supersedes_proposal_id`.
    pub proposed_op: ProposedPersonaDelta,
    /// Current status, derived from the latest chain entry
    /// referencing this proposal's `id`.
    pub status: ProposalStatus,
}

/// Status of a proposal. Status transitions are encoded as new
/// signed entries appended to the chain; this enum is the derived
/// view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ProposalStatus {
    /// The agent proposed it; the operator hasn't acted yet.
    Pending,
    /// Operator approved (optionally with edits). `applied_seq`
    /// is the seq in the [`KeyDomain::Persona`] chain where the
    /// resulting `PersonaDelta` was appended.
    Approved {
        applied_op: ProposedPersonaDelta,
        applied_seq: u64,
        resolved_at_unix_ms: u64,
    },
    /// Operator rejected the proposal. The optional `reason` is
    /// preserved in the audit trail.
    Rejected {
        reason: Option<String>,
        resolved_at_unix_ms: u64,
    },
    /// Auto-superseded — operator-direct-edit of the persona
    /// chain made this proposal redundant. Reserved for a
    /// follow-up phase; the variant is in tree so the chain
    /// schema doesn't need to change later.
    Superseded {
        by_proposal_id: String,
        resolved_at_unix_ms: u64,
    },
}

// ---------------------------------------------------------------------------
// Signed chain entry — what gets written to KeyDomain::PersonaProposals.
// ---------------------------------------------------------------------------

/// One signed entry in the proposal chain. Mirrors
/// [`crate::persona::SignedPersonaEntry`] in shape but stays
/// independent — schemas evolve separately.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedProposalEntry {
    /// Monotonic, zero-indexed across the whole chain (not per
    /// proposal id).
    pub seq: u64,
    /// The proposal id this entry refers to. Multiple entries
    /// share the same id when status transitions append new
    /// rows.
    pub proposal_id: String,
    /// Wall-clock when this entry was appended.
    pub at_unix_ms: u64,
    /// What this entry asserts about the proposal. The first
    /// entry for any given `proposal_id` is always `Pending {
    /// proposed_op, source_reflection_session_id }`.
    pub body: ProposalEntryBody,
    /// MAC of the entry preceding this one.
    pub prev_mac: [u8; 32],
    /// MAC over `prev_mac || serde_jcs(body envelope)`.
    pub mac: [u8; 32],
}

/// The signed payload of a chain entry. Splits cleanly along
/// what-this-entry-is-asserting lines.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ProposalEntryBody {
    /// First entry per proposal id; carries the original
    /// proposed op + reflection-session provenance.
    Pending {
        proposed_op: ProposedPersonaDelta,
        source_reflection_session_id: String,
    },
    /// Operator approved; carries the (possibly edited) op
    /// that was actually applied to the persona chain.
    Approved {
        applied_op: ProposedPersonaDelta,
        applied_seq: u64,
    },
    /// Operator rejected.
    Rejected {
        reason: Option<String>,
    },
    /// Auto-superseded. Reserved for a follow-up phase.
    Superseded {
        by_proposal_id: String,
    },
}

// ---------------------------------------------------------------------------
// Errors.
// ---------------------------------------------------------------------------

/// Typed errors for the proposal chain.
#[derive(Debug, thiserror::Error)]
pub enum ProposalChainError {
    #[error("proposal validation failed at seq {seq}: {reason}")]
    InvalidProposal { seq: u64, reason: String },
    #[error("proposal chain broken at seq {seq}: {reason}")]
    ChainBroken { seq: u64, reason: String },
    #[error("proposal chain serialize failed: {0}")]
    Serialize(String),
    #[error("proposal chain storage failed: {0}")]
    Storage(String),
    #[error("proposal `{0}` not found")]
    UnknownProposal(String),
    #[error(
        "proposal `{proposal_id}` cannot transition from {current} to {requested}"
    )]
    InvalidTransition {
        proposal_id: String,
        current: &'static str,
        requested: &'static str,
    },
}

// ---------------------------------------------------------------------------
// In-memory chain.
// ---------------------------------------------------------------------------

/// Distinct from the persona-chain genesis seed (`aivyx-persona-
/// genesis-v1`) so a chain-confusion attack (a Pending entry
/// slotted into the persona chain, or vice versa) is structurally
/// rejected. Must fit in the 32-byte genesis buffer.
const PROPOSAL_GENESIS_SEED: &[u8] = b"aivyx-proposal-genesis-v1";

/// In-memory proposal chain. Owns its HMAC key + the ordered
/// entry vector. Persistence layered on top via
/// [`PersistentPersonaProposalLog`].
pub struct ProposalChainLog {
    key: Vec<u8>,
    entries: std::sync::Mutex<Vec<SignedProposalEntry>>,
}

impl ProposalChainLog {
    pub fn new(key: impl Into<Vec<u8>>) -> Self {
        ProposalChainLog {
            key: key.into(),
            entries: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn from_verified_entries(
        key: impl Into<Vec<u8>>,
        entries: Vec<SignedProposalEntry>,
    ) -> Self {
        ProposalChainLog {
            key: key.into(),
            entries: std::sync::Mutex::new(entries),
        }
    }

    /// Append a `Pending` entry for a fresh proposal. Validates
    /// the `proposed_op` before appending.
    pub fn append_pending(
        &self,
        proposal_id: String,
        at_unix_ms: u64,
        source_reflection_session_id: String,
        proposed_op: ProposedPersonaDelta,
    ) -> Result<u64, ProposalChainError> {
        proposed_op
            .validate()
            .map_err(|reason| ProposalChainError::InvalidProposal {
                seq: self.len() as u64,
                reason,
            })?;
        let body = ProposalEntryBody::Pending {
            proposed_op,
            source_reflection_session_id,
        };
        self.append_entry(proposal_id, at_unix_ms, body)
    }

    /// Append an `Approved` entry for an existing proposal.
    /// Caller is responsible for having validated `applied_op`
    /// + having actually appended the resulting `PersonaDelta`
    ///   into the persona log; this method records the proposal-
    ///   side resolution.
    pub fn append_approved(
        &self,
        proposal_id: String,
        at_unix_ms: u64,
        applied_op: ProposedPersonaDelta,
        applied_seq: u64,
    ) -> Result<u64, ProposalChainError> {
        self.assert_pending(&proposal_id, "Approved")?;
        applied_op
            .validate()
            .map_err(|reason| ProposalChainError::InvalidProposal {
                seq: self.len() as u64,
                reason,
            })?;
        let body = ProposalEntryBody::Approved {
            applied_op,
            applied_seq,
        };
        self.append_entry(proposal_id, at_unix_ms, body)
    }

    /// Append a `Rejected` entry for an existing proposal.
    pub fn append_rejected(
        &self,
        proposal_id: String,
        at_unix_ms: u64,
        reason: Option<String>,
    ) -> Result<u64, ProposalChainError> {
        self.assert_pending(&proposal_id, "Rejected")?;
        let body = ProposalEntryBody::Rejected { reason };
        self.append_entry(proposal_id, at_unix_ms, body)
    }

    fn assert_pending(
        &self,
        proposal_id: &str,
        requested: &'static str,
    ) -> Result<(), ProposalChainError> {
        let view = self
            .get(proposal_id)
            .ok_or_else(|| ProposalChainError::UnknownProposal(proposal_id.to_string()))?;
        match &view.status {
            ProposalStatus::Pending => Ok(()),
            ProposalStatus::Approved { .. } => Err(ProposalChainError::InvalidTransition {
                proposal_id: proposal_id.to_string(),
                current: "Approved",
                requested,
            }),
            ProposalStatus::Rejected { .. } => Err(ProposalChainError::InvalidTransition {
                proposal_id: proposal_id.to_string(),
                current: "Rejected",
                requested,
            }),
            ProposalStatus::Superseded { .. } => Err(ProposalChainError::InvalidTransition {
                proposal_id: proposal_id.to_string(),
                current: "Superseded",
                requested,
            }),
        }
    }

    fn append_entry(
        &self,
        proposal_id: String,
        at_unix_ms: u64,
        body: ProposalEntryBody,
    ) -> Result<u64, ProposalChainError> {
        let envelope = MacEnvelope {
            proposal_id: &proposal_id,
            at_unix_ms,
            body: &body,
        };
        let body_bytes = serde_jcs::to_vec(&envelope)
            .map_err(|e| ProposalChainError::Serialize(e.to_string()))?;
        let mut inner = self.entries.lock().unwrap();
        let seq = inner.len() as u64;
        let prev_mac = match inner.last() {
            Some(prev) => prev.mac,
            None => genesis_prev_mac(),
        };
        let mac = compute_mac(&self.key, &prev_mac, &body_bytes);
        inner.push(SignedProposalEntry {
            seq,
            proposal_id,
            at_unix_ms,
            body,
            prev_mac,
            mac,
        });
        Ok(seq)
    }

    /// All entries (cloned, chain order).
    pub fn entries(&self) -> Vec<SignedProposalEntry> {
        self.entries.lock().unwrap().clone()
    }

    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.lock().unwrap().is_empty()
    }

    /// Walk every entry; check `prev_mac` linkage + recompute
    /// every `mac`. Returns the first broken link or `Ok(())`
    /// on a clean chain.
    pub fn verify(&self) -> Result<(), ProposalChainError> {
        let inner = self.entries.lock().unwrap();
        let mut expected_prev = genesis_prev_mac();
        for (idx, entry) in inner.iter().enumerate() {
            if entry.seq != idx as u64 {
                return Err(ProposalChainError::ChainBroken {
                    seq: entry.seq,
                    reason: format!("expected seq {idx}, got {}", entry.seq),
                });
            }
            if entry.prev_mac != expected_prev {
                return Err(ProposalChainError::ChainBroken {
                    seq: entry.seq,
                    reason: "prev_mac does not match previous entry's mac".into(),
                });
            }
            let envelope = MacEnvelope {
                proposal_id: &entry.proposal_id,
                at_unix_ms: entry.at_unix_ms,
                body: &entry.body,
            };
            let body_bytes = serde_jcs::to_vec(&envelope)
                .map_err(|e| ProposalChainError::Serialize(e.to_string()))?;
            let recomputed = compute_mac(&self.key, &entry.prev_mac, &body_bytes);
            if recomputed != entry.mac {
                return Err(ProposalChainError::ChainBroken {
                    seq: entry.seq,
                    reason: "mac does not match recomputed value (tampered entry?)".into(),
                });
            }
            expected_prev = entry.mac;
        }
        Ok(())
    }

    /// Derive the current view of a single proposal from the
    /// chain, or `None` if no entries reference that id.
    pub fn get(&self, proposal_id: &str) -> Option<PersonaProposal> {
        let entries = self.entries.lock().unwrap();
        derive_one(&entries, proposal_id)
    }

    /// Derive every distinct proposal from the chain, optionally
    /// filtered by status discriminant.
    pub fn list(&self, filter: ProposalStatusFilter) -> Vec<PersonaProposal> {
        let entries = self.entries.lock().unwrap();
        derive_all(&entries, filter)
    }
}

/// Status filter for [`ProposalChainLog::list`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProposalStatusFilter {
    All,
    Pending,
    Approved,
    Rejected,
    Superseded,
}

impl ProposalStatusFilter {
    fn accepts(self, status: &ProposalStatus) -> bool {
        matches!(
            (self, status),
            (ProposalStatusFilter::All, _)
                | (ProposalStatusFilter::Pending, ProposalStatus::Pending)
                | (ProposalStatusFilter::Approved, ProposalStatus::Approved { .. })
                | (ProposalStatusFilter::Rejected, ProposalStatus::Rejected { .. })
                | (ProposalStatusFilter::Superseded, ProposalStatus::Superseded { .. })
        )
    }
}

fn derive_one(
    entries: &[SignedProposalEntry],
    proposal_id: &str,
) -> Option<PersonaProposal> {
    let mut id_entries: Vec<&SignedProposalEntry> = entries
        .iter()
        .filter(|e| e.proposal_id == proposal_id)
        .collect();
    id_entries.sort_by_key(|e| e.seq);
    derive_from_id_entries(&id_entries)
}

fn derive_all(
    entries: &[SignedProposalEntry],
    filter: ProposalStatusFilter,
) -> Vec<PersonaProposal> {
    // Group entries by proposal_id preserving first-seen order.
    let mut order: Vec<String> = Vec::new();
    let mut groups: HashMap<&str, Vec<&SignedProposalEntry>> = HashMap::new();
    for entry in entries {
        if !groups.contains_key(entry.proposal_id.as_str()) {
            order.push(entry.proposal_id.clone());
        }
        groups
            .entry(entry.proposal_id.as_str())
            .or_default()
            .push(entry);
    }
    let mut out = Vec::with_capacity(order.len());
    for id in &order {
        if let Some(ents) = groups.get(id.as_str()) {
            if let Some(view) = derive_from_id_entries(ents) {
                if filter.accepts(&view.status) {
                    out.push(view);
                }
            }
        }
    }
    out
}

fn derive_from_id_entries(
    entries: &[&SignedProposalEntry],
) -> Option<PersonaProposal> {
    let first = entries.first()?;
    let (proposed_op, source_reflection_session_id) = match &first.body {
        ProposalEntryBody::Pending {
            proposed_op,
            source_reflection_session_id,
        } => (proposed_op.clone(), source_reflection_session_id.clone()),
        _ => return None, // chain invariant violation; defensive None
    };
    let last = entries.last()?;
    let status = match &last.body {
        ProposalEntryBody::Pending { .. } => ProposalStatus::Pending,
        ProposalEntryBody::Approved {
            applied_op,
            applied_seq,
        } => ProposalStatus::Approved {
            applied_op: applied_op.clone(),
            applied_seq: *applied_seq,
            resolved_at_unix_ms: last.at_unix_ms,
        },
        ProposalEntryBody::Rejected { reason } => ProposalStatus::Rejected {
            reason: reason.clone(),
            resolved_at_unix_ms: last.at_unix_ms,
        },
        ProposalEntryBody::Superseded { by_proposal_id } => {
            ProposalStatus::Superseded {
                by_proposal_id: by_proposal_id.clone(),
                resolved_at_unix_ms: last.at_unix_ms,
            }
        }
    };
    Some(PersonaProposal {
        id: first.proposal_id.clone(),
        proposed_at_unix_ms: first.at_unix_ms,
        source_reflection_session_id,
        proposed_op,
        status,
    })
}

// Envelope serialized for MAC computation. Stable JCS shape so
// the byte pattern is canonical across implementations.
#[derive(Serialize)]
struct MacEnvelope<'a> {
    proposal_id: &'a str,
    at_unix_ms: u64,
    body: &'a ProposalEntryBody,
}

fn genesis_prev_mac() -> [u8; 32] {
    let mut out = [0u8; 32];
    let src = PROPOSAL_GENESIS_SEED;
    let start = out.len() - src.len();
    out[start..].copy_from_slice(src);
    out
}

fn compute_mac(key: &[u8], prev_mac: &[u8; 32], body: &[u8]) -> [u8; 32] {
    use hmac::{Hmac, KeyInit, Mac};
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = <HmacSha256 as KeyInit>::new_from_slice(key)
        .expect("HMAC accepts any key length");
    mac.update(prev_mac);
    mac.update(body);
    let out = mac.finalize().into_bytes();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    arr
}

// ---------------------------------------------------------------------------
// Persistent wrapper — talks to KeyDomain::PersonaProposals.
// ---------------------------------------------------------------------------

/// Persistent proposal chain backed by
/// [`aivyx_storage::KeyDomain::PersonaProposals`]. One redb row
/// per signed entry, keyed by big-endian u64 sequence number so
/// scan reads return chain-ordered.
pub struct PersistentPersonaProposalLog {
    chain: ProposalChainLog,
    storage: DomainHandle,
}

impl PersistentPersonaProposalLog {
    /// Open (or initialize) the persistent log. Reads every row,
    /// reconstructs the chain, verifies linkage, and bails on
    /// tamper.
    pub async fn open(
        storage: DomainHandle,
        key: Vec<u8>,
    ) -> Result<Self, ProposalChainError> {
        let rows = storage
            .scan_prefix(&[])
            .await
            .map_err(|e| ProposalChainError::Storage(e.to_string()))?;
        let mut entries: Vec<SignedProposalEntry> = Vec::with_capacity(rows.len());
        for (_k, v) in rows {
            let entry: SignedProposalEntry = serde_json::from_slice(&v)
                .map_err(|e| ProposalChainError::Serialize(e.to_string()))?;
            entries.push(entry);
        }
        entries.sort_by_key(|e| e.seq);
        let verify_chain =
            ProposalChainLog::from_verified_entries(key.clone(), entries.clone());
        verify_chain.verify()?;
        Ok(PersistentPersonaProposalLog {
            chain: ProposalChainLog::from_verified_entries(key, entries),
            storage,
        })
    }

    /// Append a `Pending` entry + persist it.
    pub async fn append_pending(
        &self,
        proposal_id: String,
        at_unix_ms: u64,
        source_reflection_session_id: String,
        proposed_op: ProposedPersonaDelta,
    ) -> Result<u64, ProposalChainError> {
        let seq = self.chain.append_pending(
            proposal_id,
            at_unix_ms,
            source_reflection_session_id,
            proposed_op,
        )?;
        self.persist_last(seq).await
    }

    /// Append an `Approved` entry + persist it.
    pub async fn append_approved(
        &self,
        proposal_id: String,
        at_unix_ms: u64,
        applied_op: ProposedPersonaDelta,
        applied_seq: u64,
    ) -> Result<u64, ProposalChainError> {
        let seq = self
            .chain
            .append_approved(proposal_id, at_unix_ms, applied_op, applied_seq)?;
        self.persist_last(seq).await
    }

    /// Append a `Rejected` entry + persist it.
    pub async fn append_rejected(
        &self,
        proposal_id: String,
        at_unix_ms: u64,
        reason: Option<String>,
    ) -> Result<u64, ProposalChainError> {
        let seq = self
            .chain
            .append_rejected(proposal_id, at_unix_ms, reason)?;
        self.persist_last(seq).await
    }

    async fn persist_last(&self, seq: u64) -> Result<u64, ProposalChainError> {
        let entries = self.chain.entries();
        let last = entries.last().expect("just appended");
        let key = seq.to_be_bytes();
        let value = serde_json::to_vec(last)
            .map_err(|e| ProposalChainError::Serialize(e.to_string()))?;
        self.storage
            .put(&key, &value)
            .await
            .map_err(|e| ProposalChainError::Storage(e.to_string()))?;
        Ok(seq)
    }

    /// Snapshot every signed entry (cloned).
    pub fn entries(&self) -> Vec<SignedProposalEntry> {
        self.chain.entries()
    }

    pub fn len(&self) -> usize {
        self.chain.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chain.is_empty()
    }

    pub fn verify(&self) -> Result<(), ProposalChainError> {
        self.chain.verify()
    }

    pub fn get(&self, proposal_id: &str) -> Option<PersonaProposal> {
        self.chain.get(proposal_id)
    }

    pub fn list(&self, filter: ProposalStatusFilter) -> Vec<PersonaProposal> {
        self.chain.list(filter)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persona::{PersonaDeltaCategory, PersonaDeltaOp};

    fn fixture_op() -> ProposedPersonaDelta {
        ProposedPersonaDelta {
            category: PersonaDeltaCategory::BehavioralPreferences,
            op: PersonaDeltaOp::AppendList {
                value: "prefer terse responses".into(),
            },
            reason: Some("operator confirmed preference 3 turns in a row".into()),
            supersedes_proposal_id: None,
        }
    }

    #[test]
    fn append_pending_then_list_returns_one_proposal() {
        let chain = ProposalChainLog::new(b"k".to_vec());
        let seq = chain
            .append_pending(
                "p-1".into(),
                1_000,
                "ses-abc".into(),
                fixture_op(),
            )
            .expect("append");
        assert_eq!(seq, 0);
        let all = chain.list(ProposalStatusFilter::All);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "p-1");
        assert_eq!(all[0].proposed_at_unix_ms, 1_000);
        assert_eq!(all[0].source_reflection_session_id, "ses-abc");
        assert!(matches!(all[0].status, ProposalStatus::Pending));
    }

    #[test]
    fn approved_status_overrides_pending_for_same_id() {
        let chain = ProposalChainLog::new(b"k".to_vec());
        chain
            .append_pending("p-1".into(), 100, "ses".into(), fixture_op())
            .unwrap();
        chain
            .append_approved("p-1".into(), 200, fixture_op(), 7)
            .unwrap();
        let view = chain.get("p-1").expect("present");
        match view.status {
            ProposalStatus::Approved {
                applied_seq,
                resolved_at_unix_ms,
                ..
            } => {
                assert_eq!(applied_seq, 7);
                assert_eq!(resolved_at_unix_ms, 200);
            }
            other => panic!("expected Approved, got {other:?}"),
        }
    }

    #[test]
    fn rejected_status_records_reason() {
        let chain = ProposalChainLog::new(b"k".to_vec());
        chain
            .append_pending("p-1".into(), 100, "ses".into(), fixture_op())
            .unwrap();
        chain
            .append_rejected("p-1".into(), 200, Some("too aggressive".into()))
            .unwrap();
        let view = chain.get("p-1").unwrap();
        match view.status {
            ProposalStatus::Rejected { reason, .. } => {
                assert_eq!(reason.as_deref(), Some("too aggressive"));
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    #[test]
    fn list_with_filter_returns_only_matching_status() {
        let chain = ProposalChainLog::new(b"k".to_vec());
        // p-1: stays Pending.
        chain
            .append_pending("p-1".into(), 100, "ses".into(), fixture_op())
            .unwrap();
        // p-2: Pending → Approved.
        chain
            .append_pending("p-2".into(), 200, "ses".into(), fixture_op())
            .unwrap();
        chain
            .append_approved("p-2".into(), 250, fixture_op(), 0)
            .unwrap();
        // p-3: Pending → Rejected.
        chain
            .append_pending("p-3".into(), 300, "ses".into(), fixture_op())
            .unwrap();
        chain
            .append_rejected("p-3".into(), 350, None)
            .unwrap();

        let pending = chain.list(ProposalStatusFilter::Pending);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "p-1");

        let approved = chain.list(ProposalStatusFilter::Approved);
        assert_eq!(approved.len(), 1);
        assert_eq!(approved[0].id, "p-2");

        let rejected = chain.list(ProposalStatusFilter::Rejected);
        assert_eq!(rejected.len(), 1);
        assert_eq!(rejected[0].id, "p-3");

        let all = chain.list(ProposalStatusFilter::All);
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn cannot_approve_an_already_approved_proposal() {
        let chain = ProposalChainLog::new(b"k".to_vec());
        chain
            .append_pending("p-1".into(), 100, "ses".into(), fixture_op())
            .unwrap();
        chain
            .append_approved("p-1".into(), 200, fixture_op(), 0)
            .unwrap();
        let err = chain
            .append_approved("p-1".into(), 300, fixture_op(), 1)
            .expect_err("must error");
        match err {
            ProposalChainError::InvalidTransition {
                current, requested, ..
            } => {
                assert_eq!(current, "Approved");
                assert_eq!(requested, "Approved");
            }
            other => panic!("expected InvalidTransition, got {other:?}"),
        }
    }

    #[test]
    fn cannot_reject_an_already_rejected_proposal() {
        let chain = ProposalChainLog::new(b"k".to_vec());
        chain
            .append_pending("p-1".into(), 100, "ses".into(), fixture_op())
            .unwrap();
        chain
            .append_rejected("p-1".into(), 200, None)
            .unwrap();
        let err = chain
            .append_rejected("p-1".into(), 300, None)
            .expect_err("must error");
        assert!(
            matches!(err, ProposalChainError::InvalidTransition { current: "Rejected", .. })
        );
    }

    #[test]
    fn cannot_resolve_unknown_proposal() {
        let chain = ProposalChainLog::new(b"k".to_vec());
        let err = chain
            .append_approved("p-missing".into(), 100, fixture_op(), 0)
            .expect_err("must error");
        assert!(matches!(err, ProposalChainError::UnknownProposal(_)));
    }

    #[test]
    fn verify_succeeds_on_clean_chain() {
        let chain = ProposalChainLog::new(b"k".to_vec());
        chain
            .append_pending("p-1".into(), 100, "ses".into(), fixture_op())
            .unwrap();
        chain
            .append_approved("p-1".into(), 200, fixture_op(), 0)
            .unwrap();
        chain
            .append_pending("p-2".into(), 300, "ses".into(), fixture_op())
            .unwrap();
        chain.verify().expect("clean chain");
    }

    #[test]
    fn verify_fails_when_entry_is_tampered() {
        let chain = ProposalChainLog::new(b"k".to_vec());
        chain
            .append_pending("p-1".into(), 100, "ses".into(), fixture_op())
            .unwrap();
        // Tamper: replace the entries vector with a same-shape
        // entry whose proposal_id has been swapped under the
        // MAC. The verifier should reject the recomputed mac.
        let entries = chain.entries();
        let mut tampered = entries.clone();
        tampered[0].proposal_id = "p-EVIL".into();
        let tampered_chain =
            ProposalChainLog::from_verified_entries(b"k".to_vec(), tampered);
        let err = tampered_chain.verify().expect_err("must error");
        match err {
            ProposalChainError::ChainBroken { seq, .. } => assert_eq!(seq, 0),
            other => panic!("expected ChainBroken, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn persistent_log_round_trips_through_redb() {
        use aivyx_crypto::MasterKey;
        use aivyx_storage::{KeyDomain, RedbStorage, Storage, StorageConfig};
        use std::sync::Arc;

        let dir = TempDir::new();
        let store: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.path().join("store.redb")),
            MasterKey::from_raw([7u8; 32]),
        )
        .await
        .expect("storage opens");
        let handle = store.domain(KeyDomain::PersonaProposals);

        let key = b"phase-70-test-key".to_vec();
        let log = PersistentPersonaProposalLog::open(handle.clone(), key.clone())
            .await
            .expect("empty log opens clean");
        assert!(log.is_empty());

        log.append_pending("p-1".into(), 100, "ses-a".into(), fixture_op())
            .await
            .unwrap();
        log.append_pending("p-2".into(), 200, "ses-b".into(), fixture_op())
            .await
            .unwrap();
        log.append_approved("p-1".into(), 250, fixture_op(), 0)
            .await
            .unwrap();
        log.verify().expect("clean chain");
        drop(log);

        // Re-open + verify the chain replays.
        let reopened = PersistentPersonaProposalLog::open(handle, key)
            .await
            .expect("reopen");
        assert_eq!(reopened.len(), 3);
        reopened.verify().expect("clean after reopen");

        let pending = reopened.list(ProposalStatusFilter::Pending);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "p-2");

        let approved = reopened.list(ProposalStatusFilter::Approved);
        assert_eq!(approved.len(), 1);
        assert_eq!(approved[0].id, "p-1");
    }

    /// Cross-platform tempdir helper mirroring `persona.rs`'s
    /// pattern so this module's tests don't pull `tempfile`.
    struct TempDir {
        path: std::path::PathBuf,
    }
    impl TempDir {
        fn new() -> Self {
            let mut path = std::env::temp_dir();
            // A UUID, not pid+nanos: avoids a same-nanosecond collision
            // between parallel tests (the transient-flake class).
            let suffix =
                format!("aivyx-persona-proposal-test-{}", uuid::Uuid::new_v4());
            path.push(suffix);
            std::fs::create_dir_all(&path).expect("tempdir create");
            TempDir { path }
        }
        fn path(&self) -> &std::path::Path {
            &self.path
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn chain_is_distinct_from_persona_chain_seed() {
        // Two chains with the same key + identical first append
        // produce DIFFERENT macs because of the genesis-seed
        // domain separation. Defends against the chain-confusion
        // attack noted in the module doc.
        let proposal_chain = ProposalChainLog::new(b"k".to_vec());
        proposal_chain
            .append_pending("p-1".into(), 100, "ses".into(), fixture_op())
            .unwrap();
        let proposal_mac = proposal_chain.entries()[0].mac;

        let persona_chain = crate::persona::PersonaChainLog::new(b"k".to_vec());
        // Append a structurally minimal persona delta against
        // the same key. (Persona deltas carry more fields, but
        // we just need the mac for comparison.)
        persona_chain
            .append(crate::persona::PersonaDelta {
                delta_id: "d-1".into(),
                proposed_at_unix_ms: 100,
                approved_at_unix_ms: 100,
                proposal_id: "p-1".into(),
                category: PersonaDeltaCategory::BehavioralPreferences,
                op: PersonaDeltaOp::AppendList {
                    value: "prefer terse responses".into(),
                },
            })
            .unwrap();
        let persona_mac = persona_chain.entries()[0].mac;

        assert_ne!(
            proposal_mac, persona_mac,
            "proposal + persona chains must use distinct genesis seeds"
        );
    }
}
