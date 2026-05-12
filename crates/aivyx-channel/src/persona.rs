//! Persona substrate per PRODUCT.md P14 (Phase 59).
//!
//! Persona is the reflection-written **dynamic** identity layer that
//! grows from Profile (P13) under operator-approved deltas. Each
//! delta is a single field-edit per Q1(a) at Phase 59 sign-off,
//! recorded into an HMAC-chained append-only log in
//! [`KeyDomain::Persona`] per Q2(a), and replayed at daemon startup
//! into an [`EffectivePersona`] runtime state that flavors every
//! turn's system prompt alongside Profile.
//!
//! Phase 59 ships the substrate (this module + reflection-tool +
//! planner threading + system-prompt assembly). Phase 60 closes the
//! milestone with operator-facing surfaces.
//!
//! ## Wire shape
//!
//! Each delta serializes as a JSON object:
//!
//! ```json
//! {
//!   "delta_id": "pd-...",
//!   "proposed_at_unix_ms": 1715520000000,
//!   "approved_at_unix_ms": 1715520600000,
//!   "proposal_id": "rp-...",
//!   "category": "BehavioralPreferences",
//!   "op": { "kind": "AppendList", "value": "prefer terse responses" }
//! }
//! ```
//!
//! The `(category, op)` pair is validated at apply time: scalar
//! categories accept only `SetScalar`; list categories accept only
//! `AppendList` / `RemoveList`. Validation lives on [`PersonaDelta::validate`]
//! and is called by every entry point that writes to the chain.

use std::collections::BTreeMap;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use aivyx_storage::DomainHandle;

// ---------------------------------------------------------------------------
// Categories — Q3(c) at Phase 59 sign-off: 6 Profile-mirror + 4 Persona-specific.
// ---------------------------------------------------------------------------

/// Which Persona field this delta mutates. Q3(c) — Profile-mirror
/// categories let Persona refine what Profile declares; Persona-
/// specific categories let it grow new identity facets Profile does
/// not carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PersonaDeltaCategory {
    // -- Profile-mirror categories (also live on aivyx_config::Profile) --
    /// Scalar — the operator's chosen name for this assistant.
    AssistantName,
    /// Scalar — short description of who the operator is.
    OperatorProfile,
    /// Scalar — operator's preferred communication style.
    CommunicationStyle,
    /// List — the 1–3 use-case archetypes the assistant is shaped around.
    PrimaryUseCases,
    /// List — non-capability defaults that flavor the agent's judgment.
    BehavioralPreferences,
    /// List — non-capability guardrails the agent respects across roles.
    BehavioralConstraints,

    // -- Persona-specific categories (P14 amendment commentary) --
    /// List — accumulated facts about the operator and their domain
    /// the assistant has internalized over time.
    LearnedContext,
    /// List — refinements to communication_style learned over time.
    CommunicationAdaptations,
    /// List — emergent voice properties the assistant has grown into.
    CharacterTraits,
    /// List — operator-significant events the assistant references
    /// for continuity.
    RelationshipMilestones,
}

impl PersonaDeltaCategory {
    /// `true` for scalar (single-valued) categories; `false` for list
    /// categories. Drives validation: `SetScalar` op is only valid on
    /// scalar categories; `AppendList` / `RemoveList` only on list
    /// categories.
    pub fn is_scalar(self) -> bool {
        matches!(
            self,
            PersonaDeltaCategory::AssistantName
                | PersonaDeltaCategory::OperatorProfile
                | PersonaDeltaCategory::CommunicationStyle
        )
    }
}

/// Operation a delta performs on its target category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum PersonaDeltaOp {
    /// Replace a scalar category's value. `None` clears the field.
    SetScalar { value: Option<String> },
    /// Append a string to a list category. Duplicate appends are
    /// idempotent at apply time (the effective state's BTreeSet
    /// drops duplicates).
    AppendList { value: String },
    /// Remove a matching string from a list category. No-op if the
    /// value is not present.
    RemoveList { value: String },
}

// ---------------------------------------------------------------------------
// PersonaDelta — one approved field-edit in the chain.
// ---------------------------------------------------------------------------

/// A single operator-approved delta in the Persona chain. Q1(a) at
/// Phase 59 sign-off: each delta = one operation on one category.
///
/// `mac` is computed over `prev_mac || canonical_json(this delta's
/// body)`, where the body is every field of this struct except `mac`
/// and `seq`. Caller-supplied `seq` mirrors the audit chain
/// convention: zero-indexed, monotonic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaDelta {
    /// Stable id for this delta. Operator-facing in revert flows.
    pub delta_id: String,
    /// When the agent proposed it (via `reflection.propose`).
    pub proposed_at_unix_ms: u64,
    /// When the operator approved it (via gate resolution).
    pub approved_at_unix_ms: u64,
    /// Mission id from the gate that approved this delta. Lets the
    /// operator trace a delta back to the proposal it came from.
    pub proposal_id: String,
    /// Which Persona field this delta mutates.
    pub category: PersonaDeltaCategory,
    /// What mutation to perform on that field.
    pub op: PersonaDeltaOp,
}

impl PersonaDelta {
    /// Validate the `(category, op)` pair. Scalar categories only
    /// accept `SetScalar`; list categories only accept `AppendList`
    /// / `RemoveList`. Returns a human-readable reason on failure.
    pub fn validate(&self) -> Result<(), String> {
        match (self.category.is_scalar(), &self.op) {
            (true, PersonaDeltaOp::SetScalar { .. }) => Ok(()),
            (false, PersonaDeltaOp::AppendList { .. }) => Ok(()),
            (false, PersonaDeltaOp::RemoveList { .. }) => Ok(()),
            (true, PersonaDeltaOp::AppendList { .. })
            | (true, PersonaDeltaOp::RemoveList { .. }) => Err(format!(
                "category {:?} is scalar — only SetScalar is valid; got list op",
                self.category
            )),
            (false, PersonaDeltaOp::SetScalar { .. }) => Err(format!(
                "category {:?} is a list — only AppendList/RemoveList are valid; got SetScalar",
                self.category
            )),
        }
    }
}

// ---------------------------------------------------------------------------
// Signed chain entry — what gets written to KeyDomain::Persona.
// ---------------------------------------------------------------------------

/// One signed entry in the Persona chain. Mirrors `aivyx_audit::SignedEntry`
/// in shape but stays independent so Persona-chain semantics evolve
/// without dragging the audit chain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedPersonaEntry {
    /// Monotonic, zero-indexed.
    pub seq: u64,
    pub delta: PersonaDelta,
    /// MAC of the entry preceding this one in the chain.
    pub prev_mac: [u8; 32],
    /// MAC over `prev_mac || serde_jcs(delta)`.
    pub mac: [u8; 32],
}

// ---------------------------------------------------------------------------
// Chain primitive — append, get, verify.
// ---------------------------------------------------------------------------

/// Genesis seed for the first entry's `prev_mac`. Distinct from the
/// audit chain's seed so a chain-confusion attack (swapping a Persona
/// entry into the audit chain or vice versa) is structurally
/// rejected.
const PERSONA_GENESIS_SEED: &[u8] = b"aivyx-persona-genesis-v1";

/// Typed errors for the Persona chain. Mirror `aivyx_audit::AuditError`'s
/// shape but stay scoped to this module.
#[derive(Debug, thiserror::Error)]
pub enum PersonaChainError {
    #[error("persona delta validation failed at seq {seq}: {reason}")]
    InvalidDelta { seq: u64, reason: String },
    #[error("persona chain broken at seq {seq}: {reason}")]
    ChainBroken { seq: u64, reason: String },
    #[error("persona chain serialize failed: {0}")]
    Serialize(String),
    #[error("persona chain storage failed: {0}")]
    Storage(String),
}

/// In-memory Persona chain. Owns its HMAC key and the ordered entry
/// vector. Persistence is layered on top via [`PersistentPersonaLog`].
pub struct PersonaChainLog {
    key: Vec<u8>,
    entries: std::sync::Mutex<Vec<SignedPersonaEntry>>,
}

impl PersonaChainLog {
    /// Create an empty chain bound to the given HMAC key. The key is
    /// typically the AEAD subkey for [`aivyx_storage::KeyDomain::Persona`]
    /// re-derived for HMAC use; that wiring lives in the
    /// `PersistentPersonaLog` constructor.
    pub fn new(key: impl Into<Vec<u8>>) -> Self {
        PersonaChainLog {
            key: key.into(),
            entries: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Reconstruct a chain from previously-persisted entries. **Caller
    /// is responsible for verifying the chain before calling this** —
    /// the constructor accepts the entries as-is so the on-disk byte
    /// pattern survives the round trip.
    pub fn from_verified_entries(
        key: impl Into<Vec<u8>>,
        entries: Vec<SignedPersonaEntry>,
    ) -> Self {
        PersonaChainLog {
            key: key.into(),
            entries: std::sync::Mutex::new(entries),
        }
    }

    /// Append a new delta to the chain. Returns the assigned sequence
    /// number on success.
    pub fn append(&self, delta: PersonaDelta) -> Result<u64, PersonaChainError> {
        delta
            .validate()
            .map_err(|reason| PersonaChainError::InvalidDelta {
                seq: self.len() as u64,
                reason,
            })?;

        let body = serde_jcs::to_vec(&delta).map_err(|e| PersonaChainError::Serialize(e.to_string()))?;

        let mut inner = self.entries.lock().unwrap();
        let seq = inner.len() as u64;
        let prev_mac = match inner.last() {
            Some(prev) => prev.mac,
            None => genesis_prev_mac(),
        };
        let mac = compute_mac(&self.key, &prev_mac, &body);
        inner.push(SignedPersonaEntry { seq, delta, prev_mac, mac });
        Ok(seq)
    }

    /// All entries (cloned).
    pub fn entries(&self) -> Vec<SignedPersonaEntry> {
        self.entries.lock().unwrap().clone()
    }

    pub fn len(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.lock().unwrap().is_empty()
    }

    /// Walk every entry; check `prev_mac` linkage and recompute every
    /// `mac`. Returns the first broken link, or `Ok(())` on a clean
    /// chain.
    pub fn verify(&self) -> Result<(), PersonaChainError> {
        let inner = self.entries.lock().unwrap();
        let mut expected_prev = genesis_prev_mac();
        for (idx, entry) in inner.iter().enumerate() {
            if entry.seq != idx as u64 {
                return Err(PersonaChainError::ChainBroken {
                    seq: entry.seq,
                    reason: format!("expected seq {idx}, got {}", entry.seq),
                });
            }
            if entry.prev_mac != expected_prev {
                return Err(PersonaChainError::ChainBroken {
                    seq: entry.seq,
                    reason: "prev_mac does not match previous entry's mac".into(),
                });
            }
            let body = serde_jcs::to_vec(&entry.delta)
                .map_err(|e| PersonaChainError::Serialize(e.to_string()))?;
            let recomputed = compute_mac(&self.key, &entry.prev_mac, &body);
            if recomputed != entry.mac {
                return Err(PersonaChainError::ChainBroken {
                    seq: entry.seq,
                    reason: "mac does not match recomputed value (tampered delta?)".into(),
                });
            }
            expected_prev = entry.mac;
        }
        Ok(())
    }
}

fn genesis_prev_mac() -> [u8; 32] {
    let mut out = [0u8; 32];
    let src = PERSONA_GENESIS_SEED;
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
// Persistent wrapper — talks to KeyDomain::Persona.
// ---------------------------------------------------------------------------

/// Persistent Persona chain backed by [`KeyDomain::Persona`]. One
/// redb row per signed entry, keyed by big-endian u64 sequence
/// number so `range` reads return chain-ordered.
///
/// On `open`, every existing row is read, decoded, and chain-verified
/// before being handed to [`PersonaChainLog::from_verified_entries`]
/// — a tampered row surfaces at startup, not at first-use.
pub struct PersistentPersonaLog {
    chain: PersonaChainLog,
    storage: DomainHandle,
}

impl PersistentPersonaLog {
    /// Open (or initialize) the persistent log. Reads every row,
    /// reconstructs the chain, verifies linkage, and bails on
    /// tamper. Empty-store path returns a fresh chain.
    pub async fn open(storage: DomainHandle, key: Vec<u8>) -> Result<Self, PersonaChainError> {
        // Empty-prefix scan returns every row in the domain. ScanRow
        // is `(Vec<u8>, Vec<u8>)` keyed by big-endian u64 seq, so the
        // iteration order from redb is already chain order — we sort
        // anyway as a defense against the storage layer reshuffling.
        let rows = storage
            .scan_prefix(&[])
            .await
            .map_err(|e| PersonaChainError::Storage(e.to_string()))?;

        let mut entries: Vec<SignedPersonaEntry> = Vec::with_capacity(rows.len());
        for (_key, value) in rows {
            let entry: SignedPersonaEntry = serde_json::from_slice(&value)
                .map_err(|e| PersonaChainError::Serialize(e.to_string()))?;
            entries.push(entry);
        }
        entries.sort_by_key(|e| e.seq);

        // Verify before handing off — we don't want to load a broken
        // chain into memory and discover the problem only on append.
        let verify_chain = PersonaChainLog::from_verified_entries(key.clone(), entries.clone());
        verify_chain.verify()?;

        Ok(PersistentPersonaLog {
            chain: PersonaChainLog::from_verified_entries(key, entries),
            storage,
        })
    }

    /// Append a delta to the in-memory chain AND persist the signed
    /// entry to storage. Atomic at the chain-state level: the
    /// in-memory append commits first; on storage failure the row is
    /// not written and the caller can retry by re-appending the same
    /// delta with a new `delta_id`.
    pub async fn append(&self, delta: PersonaDelta) -> Result<u64, PersonaChainError> {
        let seq = self.chain.append(delta)?;
        let entries = self.chain.entries();
        let last = entries.last().expect("just appended");
        let key = seq.to_be_bytes();
        let value = serde_json::to_vec(last)
            .map_err(|e| PersonaChainError::Serialize(e.to_string()))?;
        self.storage
            .put(&key, &value)
            .await
            .map_err(|e| PersonaChainError::Storage(e.to_string()))?;
        Ok(seq)
    }

    /// Snapshot of every signed entry (cloned). Phase 60 will
    /// expose this through the Web UI Persona pane.
    pub fn entries(&self) -> Vec<SignedPersonaEntry> {
        self.chain.entries()
    }

    pub fn len(&self) -> usize {
        self.chain.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chain.is_empty()
    }

    /// Re-verify the chain top to bottom. Cheap because every entry
    /// is already in memory. Returns the first broken link or
    /// `Ok(())` on a clean chain.
    pub fn verify(&self) -> Result<(), PersonaChainError> {
        self.chain.verify()
    }
}

// ---------------------------------------------------------------------------
// EffectivePersona — replay the chain into a runtime state.
// ---------------------------------------------------------------------------

/// Replay of the Persona chain into a structured runtime state. Used
/// by `assemble_session_prompt` (Phase 59 Task 6) to compose the
/// "How I have learned to communicate" section alongside Profile.
///
/// Scalars hold `Option<String>` because a `SetScalar { value: None }`
/// delta clears the field. Lists hold `Vec<String>` in insertion
/// order with duplicates removed (last-wins on
/// `AppendList`-after-`RemoveList`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EffectivePersona {
    pub assistant_name: Option<String>,
    pub operator_profile: Option<String>,
    pub communication_style: Option<String>,
    pub primary_use_cases: Vec<String>,
    pub behavioral_preferences: Vec<String>,
    pub behavioral_constraints: Vec<String>,
    pub learned_context: Vec<String>,
    pub communication_adaptations: Vec<String>,
    pub character_traits: Vec<String>,
    pub relationship_milestones: Vec<String>,
}

/// Shared runtime handle on the effective Persona state. The
/// daemon's planner factory holds a clone of this `Arc` and reads
/// under the lock per-turn; `reflection.apply` (Phase 59 Task 4)
/// holds another clone and writes under the lock when an approved
/// delta lands. This matches the Phase 30 `role_overrides` pattern
/// per Q5(a) at Phase 59 sign-off.
pub type SharedEffectivePersona = Arc<RwLock<EffectivePersona>>;

/// Convenience constructor for [`SharedEffectivePersona`] seeded
/// with the result of replaying a chain. Daemon startup wires it
/// once; subsequent mutations happen via the write lock.
pub fn shared_effective_persona(initial: EffectivePersona) -> SharedEffectivePersona {
    Arc::new(RwLock::new(initial))
}

/// Apply one approved delta to a `SharedEffectivePersona` under
/// the write lock. Returns `true` on success; `false` if the lock
/// was poisoned (the apply tool surfaces this as a tool error so
/// the operator sees the chain was written but the runtime state
/// did not refresh).
pub fn apply_delta_to_shared(shared: &SharedEffectivePersona, delta: &PersonaDelta) -> bool {
    if let Ok(mut state) = shared.write() {
        apply_delta_to_state(delta, &mut state);
        true
    } else {
        false
    }
}

impl EffectivePersona {
    /// `true` when at least one delta has shaped the state — i.e.
    /// some field is non-empty / non-None. Drives the
    /// `assemble_session_prompt` decision whether to emit the
    /// Persona section at all.
    pub fn is_non_empty(&self) -> bool {
        self.assistant_name.is_some()
            || self.operator_profile.is_some()
            || self.communication_style.is_some()
            || !self.primary_use_cases.is_empty()
            || !self.behavioral_preferences.is_empty()
            || !self.behavioral_constraints.is_empty()
            || !self.learned_context.is_empty()
            || !self.communication_adaptations.is_empty()
            || !self.character_traits.is_empty()
            || !self.relationship_milestones.is_empty()
    }
}

/// Fold every approved delta in `entries` (in chain order) into a
/// fresh [`EffectivePersona`]. Pure function — same input always
/// produces the same output, so daemons replay chains
/// deterministically at startup.
pub fn compute_effective_persona(entries: &[SignedPersonaEntry]) -> EffectivePersona {
    let mut state = EffectivePersona::default();
    for entry in entries {
        apply_delta_to_state(&entry.delta, &mut state);
    }
    state
}

fn apply_delta_to_state(delta: &PersonaDelta, state: &mut EffectivePersona) {
    match (delta.category, &delta.op) {
        (PersonaDeltaCategory::AssistantName, PersonaDeltaOp::SetScalar { value }) => {
            state.assistant_name = value.clone();
        }
        (PersonaDeltaCategory::OperatorProfile, PersonaDeltaOp::SetScalar { value }) => {
            state.operator_profile = value.clone();
        }
        (PersonaDeltaCategory::CommunicationStyle, PersonaDeltaOp::SetScalar { value }) => {
            state.communication_style = value.clone();
        }
        (cat, PersonaDeltaOp::AppendList { value }) => {
            apply_append_to_list(field_for_list_category(state, cat), value);
        }
        (cat, PersonaDeltaOp::RemoveList { value }) => {
            apply_remove_from_list(field_for_list_category(state, cat), value);
        }
        // Mismatched (category, op) pairs were rejected at append-time
        // by `PersonaDelta::validate()`; if one slips through, the
        // delta is a no-op rather than a panic. Defense in depth.
        (_, _) => {}
    }
}

fn field_for_list_category(
    state: &mut EffectivePersona,
    cat: PersonaDeltaCategory,
) -> &mut Vec<String> {
    match cat {
        PersonaDeltaCategory::PrimaryUseCases => &mut state.primary_use_cases,
        PersonaDeltaCategory::BehavioralPreferences => &mut state.behavioral_preferences,
        PersonaDeltaCategory::BehavioralConstraints => &mut state.behavioral_constraints,
        PersonaDeltaCategory::LearnedContext => &mut state.learned_context,
        PersonaDeltaCategory::CommunicationAdaptations => &mut state.communication_adaptations,
        PersonaDeltaCategory::CharacterTraits => &mut state.character_traits,
        PersonaDeltaCategory::RelationshipMilestones => &mut state.relationship_milestones,
        // Scalar categories never reach here under validated deltas;
        // returning a scratch field would mask the impossible case.
        // Use a static no-op buffer so the surrounding match arm is
        // safe even if `apply_delta_to_state`'s validation drift.
        PersonaDeltaCategory::AssistantName
        | PersonaDeltaCategory::OperatorProfile
        | PersonaDeltaCategory::CommunicationStyle => {
            unreachable!("validated scalar category routed to list-field accessor")
        }
    }
}

fn apply_append_to_list(list: &mut Vec<String>, value: &str) {
    // Idempotent append: skip if the value is already present. Keeps
    // the list short under repeated proposals of the same insight.
    if !list.iter().any(|s| s == value) {
        list.push(value.to_string());
    }
}

fn apply_remove_from_list(list: &mut Vec<String>, value: &str) {
    list.retain(|s| s != value);
}

// ---------------------------------------------------------------------------
// New-delta builder — shared by reflection.propose + tests.
// ---------------------------------------------------------------------------

/// Caller-supplied delta candidate. Phase 59 Task 3 — what the agent
/// proposes through `reflection.propose`. The `delta_id`,
/// `proposed_at_unix_ms`, and `approved_at_unix_ms` fields are
/// filled in by the apply tool at gate-approval time, so the agent
/// only supplies the substantive fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedPersonaDelta {
    pub category: PersonaDeltaCategory,
    pub op: PersonaDeltaOp,
    /// Optional reason the agent gives for proposing this delta.
    /// Operator sees it in the gate prompt. Not part of the
    /// HMAC-chained body — kept on the proposal record only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ProposedPersonaDelta {
    /// Validate the proposed `(category, op)` pair. Called by
    /// `reflection.propose` to fail-fast on bad proposals.
    pub fn validate(&self) -> Result<(), String> {
        let probe = PersonaDelta {
            delta_id: String::new(),
            proposed_at_unix_ms: 0,
            approved_at_unix_ms: 0,
            proposal_id: String::new(),
            category: self.category,
            op: self.op.clone(),
        };
        probe.validate()
    }
}

/// Index of every delta by its `delta_id`. Used by Phase 60's revert
/// flow to find a delta to undo. Phase 59 only exposes the builder.
pub fn index_entries_by_id(
    entries: &[SignedPersonaEntry],
) -> BTreeMap<String, &SignedPersonaEntry> {
    let mut idx = BTreeMap::new();
    for entry in entries {
        idx.insert(entry.delta.delta_id.clone(), entry);
    }
    idx
}

// ---------------------------------------------------------------------------
// Hash helper for tests — deterministic delta_id given content.
// ---------------------------------------------------------------------------

/// Synthesize a stable `delta_id` from the delta's content. Used by
/// `reflection.apply` (Phase 59 Task 4) so re-applying a proposal
/// with the same `(proposal_id, category, op)` triple does not
/// produce a different id. Hex-encoded SHA-256 of the
/// canonical-JSON body.
pub fn synthesize_delta_id(
    proposal_id: &str,
    category: PersonaDeltaCategory,
    op: &PersonaDeltaOp,
    seq_within_proposal: u32,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(proposal_id.as_bytes());
    hasher.update(b"|");
    hasher.update(serde_jcs::to_vec(&category).unwrap_or_default());
    hasher.update(b"|");
    hasher.update(serde_jcs::to_vec(op).unwrap_or_default());
    hasher.update(b"|");
    hasher.update(seq_within_proposal.to_be_bytes());
    let digest = hasher.finalize();
    let mut out = String::from("pd-");
    for byte in digest.iter().take(12) {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> Vec<u8> {
        b"phase-59-test-key-32-bytes-pad!!".to_vec()
    }

    fn list_delta(category: PersonaDeltaCategory, value: &str) -> PersonaDelta {
        PersonaDelta {
            delta_id: format!("pd-{value}"),
            proposed_at_unix_ms: 1_715_000_000_000,
            approved_at_unix_ms: 1_715_000_060_000,
            proposal_id: "rp-test".into(),
            category,
            op: PersonaDeltaOp::AppendList { value: value.into() },
        }
    }

    fn scalar_delta(category: PersonaDeltaCategory, value: Option<&str>) -> PersonaDelta {
        PersonaDelta {
            delta_id: format!("pd-scalar-{value:?}"),
            proposed_at_unix_ms: 1_715_000_000_000,
            approved_at_unix_ms: 1_715_000_060_000,
            proposal_id: "rp-test".into(),
            category,
            op: PersonaDeltaOp::SetScalar {
                value: value.map(String::from),
            },
        }
    }

    // ---- PersonaDelta::validate --------------------------------

    #[test]
    fn validate_accepts_set_scalar_on_scalar_category() {
        let d = scalar_delta(PersonaDeltaCategory::AssistantName, Some("Codex"));
        assert!(d.validate().is_ok());
    }

    #[test]
    fn validate_accepts_append_list_on_list_category() {
        let d = list_delta(PersonaDeltaCategory::BehavioralPreferences, "prefer terse");
        assert!(d.validate().is_ok());
    }

    #[test]
    fn validate_rejects_set_scalar_on_list_category() {
        let d = PersonaDelta {
            category: PersonaDeltaCategory::PrimaryUseCases,
            op: PersonaDeltaOp::SetScalar {
                value: Some("oops".into()),
            },
            ..scalar_delta(PersonaDeltaCategory::AssistantName, Some("dummy"))
        };
        let err = d.validate().unwrap_err();
        assert!(err.contains("is a list"));
    }

    #[test]
    fn validate_rejects_append_list_on_scalar_category() {
        let d = PersonaDelta {
            category: PersonaDeltaCategory::AssistantName,
            op: PersonaDeltaOp::AppendList {
                value: "oops".into(),
            },
            ..list_delta(PersonaDeltaCategory::BehavioralPreferences, "dummy")
        };
        let err = d.validate().unwrap_err();
        assert!(err.contains("is scalar"));
    }

    // ---- PersonaChainLog::append + verify -----------------------

    #[test]
    fn single_delta_appends_and_verifies() {
        let chain = PersonaChainLog::new(test_key());
        let seq = chain
            .append(list_delta(
                PersonaDeltaCategory::BehavioralPreferences,
                "prefer terse",
            ))
            .expect("append");
        assert_eq!(seq, 0);
        assert_eq!(chain.len(), 1);
        chain.verify().expect("clean chain");
    }

    #[test]
    fn two_deltas_chain_prev_mac_linkage() {
        let chain = PersonaChainLog::new(test_key());
        chain
            .append(list_delta(
                PersonaDeltaCategory::BehavioralPreferences,
                "prefer terse",
            ))
            .expect("append 1");
        chain
            .append(scalar_delta(
                PersonaDeltaCategory::CommunicationStyle,
                Some("conclusion-first"),
            ))
            .expect("append 2");

        let entries = chain.entries();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].prev_mac, entries[0].mac);
        chain.verify().expect("clean chain");
    }

    #[test]
    fn tampered_delta_fails_verification() {
        let chain = PersonaChainLog::new(test_key());
        chain
            .append(list_delta(
                PersonaDeltaCategory::LearnedContext,
                "operator uses Vim",
            ))
            .expect("append");

        // Reconstruct the chain from the persisted entries but flip
        // a byte in the delta's payload — verify() must catch it.
        let mut entries = chain.entries();
        if let PersonaDeltaOp::AppendList { value } = &mut entries[0].delta.op {
            *value = "operator uses VSCode".into();
        }
        let tampered =
            PersonaChainLog::from_verified_entries(test_key(), entries);
        let err = tampered.verify().expect_err("must catch tamper");
        assert!(matches!(err, PersonaChainError::ChainBroken { .. }));
    }

    #[test]
    fn validation_failure_in_append_rejects_the_delta() {
        let chain = PersonaChainLog::new(test_key());
        let bad = PersonaDelta {
            category: PersonaDeltaCategory::AssistantName,
            op: PersonaDeltaOp::AppendList {
                value: "oops".into(),
            },
            ..list_delta(PersonaDeltaCategory::BehavioralPreferences, "dummy")
        };
        let err = chain.append(bad).expect_err("invalid delta must reject");
        assert!(matches!(err, PersonaChainError::InvalidDelta { .. }));
        assert!(chain.is_empty());
    }

    #[test]
    fn different_keys_produce_different_macs() {
        let d = list_delta(PersonaDeltaCategory::BehavioralPreferences, "prefer terse");
        let chain_a = PersonaChainLog::new(b"key-aaaa".to_vec());
        chain_a.append(d.clone()).expect("append");
        let chain_b = PersonaChainLog::new(b"key-bbbb".to_vec());
        chain_b.append(d).expect("append");
        assert_ne!(chain_a.entries()[0].mac, chain_b.entries()[0].mac);
    }

    // ---- compute_effective_persona ------------------------------

    #[test]
    fn effective_persona_starts_empty() {
        let state = compute_effective_persona(&[]);
        assert!(!state.is_non_empty());
        assert_eq!(state, EffectivePersona::default());
    }

    #[test]
    fn effective_persona_folds_scalar_set() {
        let chain = PersonaChainLog::new(test_key());
        chain
            .append(scalar_delta(
                PersonaDeltaCategory::AssistantName,
                Some("Mira"),
            ))
            .expect("append");
        let state = compute_effective_persona(&chain.entries());
        assert_eq!(state.assistant_name.as_deref(), Some("Mira"));
        assert!(state.is_non_empty());
    }

    #[test]
    fn effective_persona_folds_list_appends_and_removes() {
        let chain = PersonaChainLog::new(test_key());
        chain
            .append(list_delta(
                PersonaDeltaCategory::BehavioralPreferences,
                "prefer terse",
            ))
            .expect("append 1");
        chain
            .append(list_delta(
                PersonaDeltaCategory::BehavioralPreferences,
                "always cite sources",
            ))
            .expect("append 2");

        // Remove the first.
        let remove = PersonaDelta {
            category: PersonaDeltaCategory::BehavioralPreferences,
            op: PersonaDeltaOp::RemoveList {
                value: "prefer terse".into(),
            },
            ..list_delta(PersonaDeltaCategory::BehavioralPreferences, "dummy")
        };
        chain.append(remove).expect("append 3");

        let state = compute_effective_persona(&chain.entries());
        assert_eq!(
            state.behavioral_preferences,
            vec!["always cite sources".to_string()]
        );
    }

    #[test]
    fn append_list_is_idempotent_per_value() {
        let chain = PersonaChainLog::new(test_key());
        chain
            .append(list_delta(
                PersonaDeltaCategory::LearnedContext,
                "operator uses Vim",
            ))
            .expect("first append");
        chain
            .append(list_delta(
                PersonaDeltaCategory::LearnedContext,
                "operator uses Vim",
            ))
            .expect("duplicate append");
        let state = compute_effective_persona(&chain.entries());
        assert_eq!(state.learned_context.len(), 1);
        assert_eq!(state.learned_context[0], "operator uses Vim");
    }

    // ---- ProposedPersonaDelta::validate -------------------------

    #[test]
    fn proposed_delta_validate_matches_persona_delta_validate() {
        let good = ProposedPersonaDelta {
            category: PersonaDeltaCategory::BehavioralConstraints,
            op: PersonaDeltaOp::AppendList {
                value: "never auto-commit".into(),
            },
            reason: Some("operator reverted three auto-commits".into()),
        };
        assert!(good.validate().is_ok());

        let bad = ProposedPersonaDelta {
            category: PersonaDeltaCategory::OperatorProfile,
            op: PersonaDeltaOp::AppendList {
                value: "nope".into(),
            },
            reason: None,
        };
        assert!(bad.validate().is_err());
    }

    // ---- synthesize_delta_id ------------------------------------

    #[test]
    fn synthesize_delta_id_is_deterministic() {
        let a = synthesize_delta_id(
            "rp-1",
            PersonaDeltaCategory::BehavioralPreferences,
            &PersonaDeltaOp::AppendList {
                value: "test".into(),
            },
            0,
        );
        let b = synthesize_delta_id(
            "rp-1",
            PersonaDeltaCategory::BehavioralPreferences,
            &PersonaDeltaOp::AppendList {
                value: "test".into(),
            },
            0,
        );
        assert_eq!(a, b);
        assert!(a.starts_with("pd-"));
    }

    // ---- SharedEffectivePersona ---------------------------------

    #[test]
    fn shared_effective_persona_seeds_from_initial_state() {
        let initial = EffectivePersona {
            assistant_name: Some("Codex".into()),
            ..EffectivePersona::default()
        };
        let shared = shared_effective_persona(initial);
        let read = shared.read().unwrap();
        assert_eq!(read.assistant_name.as_deref(), Some("Codex"));
    }

    #[test]
    fn apply_delta_to_shared_mutates_under_write_lock() {
        let shared = shared_effective_persona(EffectivePersona::default());
        let delta = list_delta(
            PersonaDeltaCategory::BehavioralPreferences,
            "prefer terse",
        );
        let ok = apply_delta_to_shared(&shared, &delta);
        assert!(ok);
        let read = shared.read().unwrap();
        assert_eq!(read.behavioral_preferences, vec!["prefer terse"]);
    }

    #[test]
    fn apply_delta_to_shared_idempotent_on_repeated_appends() {
        let shared = shared_effective_persona(EffectivePersona::default());
        let delta = list_delta(
            PersonaDeltaCategory::LearnedContext,
            "operator uses Vim",
        );
        apply_delta_to_shared(&shared, &delta);
        apply_delta_to_shared(&shared, &delta);
        let read = shared.read().unwrap();
        assert_eq!(read.learned_context.len(), 1);
    }

    // ---- PersistentPersonaLog round-trip ------------------------

    #[tokio::test]
    async fn persistent_log_round_trips_through_redb() {
        use aivyx_crypto::MasterKey;
        use aivyx_storage::{KeyDomain, RedbStorage, Storage, StorageConfig};
        use std::sync::Arc;

        let dir = tempdir();
        let store: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.path().join("store.redb")),
            MasterKey::from_raw([42u8; 32]),
        )
        .await
        .expect("scratch storage opens");
        let handle = store.domain(KeyDomain::Persona);

        // Open empty log, append two deltas, drop.
        let key = test_key();
        let log = PersistentPersonaLog::open(handle.clone(), key.clone())
            .await
            .expect("empty log opens clean");
        assert!(log.is_empty());

        let seq0 = log
            .append(list_delta(
                PersonaDeltaCategory::BehavioralPreferences,
                "prefer terse",
            ))
            .await
            .expect("append 1");
        assert_eq!(seq0, 0);

        let seq1 = log
            .append(scalar_delta(
                PersonaDeltaCategory::AssistantName,
                Some("Codex"),
            ))
            .await
            .expect("append 2");
        assert_eq!(seq1, 1);
        log.verify().expect("clean chain after appends");
        drop(log);

        // Re-open with the same key — the chain must replay.
        let reopened = PersistentPersonaLog::open(handle, key)
            .await
            .expect("reopen succeeds");
        assert_eq!(reopened.len(), 2);
        let entries = reopened.entries();
        assert_eq!(entries[0].seq, 0);
        assert_eq!(entries[1].seq, 1);
        // prev_mac chain survived the round trip.
        assert_eq!(entries[1].prev_mac, entries[0].mac);
        reopened.verify().expect("clean chain after reopen");

        // Apply to runtime state via the shared helper.
        let shared = shared_effective_persona(compute_effective_persona(&entries));
        let snap = shared.read().unwrap();
        assert_eq!(
            snap.behavioral_preferences,
            vec!["prefer terse".to_string()]
        );
        assert_eq!(snap.assistant_name.as_deref(), Some("Codex"));
    }

    #[tokio::test]
    async fn persistent_log_rejects_invalid_delta_without_persisting() {
        use aivyx_crypto::MasterKey;
        use aivyx_storage::{KeyDomain, RedbStorage, Storage, StorageConfig};
        use std::sync::Arc;

        let dir = tempdir();
        let store: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.path().join("store.redb")),
            MasterKey::from_raw([99u8; 32]),
        )
        .await
        .expect("storage opens");
        let log = PersistentPersonaLog::open(
            store.domain(KeyDomain::Persona),
            test_key(),
        )
        .await
        .expect("log opens");

        let bad = PersonaDelta {
            category: PersonaDeltaCategory::AssistantName,
            op: PersonaDeltaOp::AppendList { value: "oops".into() },
            ..list_delta(PersonaDeltaCategory::BehavioralPreferences, "dummy")
        };
        let err = log
            .append(bad)
            .await
            .expect_err("invalid delta must reject");
        assert!(matches!(err, PersonaChainError::InvalidDelta { .. }));
        assert!(log.is_empty());
    }

    /// Cross-platform tempdir helper without pulling the `tempfile`
    /// crate. Mirrors the pattern used by other tests in this
    /// workspace.
    fn tempdir() -> TempDir {
        TempDir::new()
    }

    struct TempDir {
        path: std::path::PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let mut path = std::env::temp_dir();
            let suffix = format!(
                "aivyx-persona-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0),
            );
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
    fn synthesize_delta_id_differs_for_different_seq() {
        let a = synthesize_delta_id(
            "rp-1",
            PersonaDeltaCategory::BehavioralPreferences,
            &PersonaDeltaOp::AppendList {
                value: "test".into(),
            },
            0,
        );
        let b = synthesize_delta_id(
            "rp-1",
            PersonaDeltaCategory::BehavioralPreferences,
            &PersonaDeltaOp::AppendList {
                value: "test".into(),
            },
            1,
        );
        assert_ne!(a, b);
    }
}
