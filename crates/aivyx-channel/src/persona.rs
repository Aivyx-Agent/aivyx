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

// The persona **data** types (PersonaDelta(Category/Op), EffectivePersona,
// LearnedSkill, ProposedPersonaDelta) moved to the wasm-clean `aivyx-ipc` crate
// (Chapter M.2c) so the browser app shares them; re-exported here so this
// module's HMAC chain / store / fold code and the IPC surface are unchanged.
pub use aivyx_ipc::persona::{
    EffectivePersona, LearnedSkill, PersonaDelta, PersonaDeltaCategory, PersonaDeltaOp,
    ProposedPersonaDelta, SkillAuthor, SkillProvenance,
};

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

    /// Phase 65 — wipe every entry from the in-memory chain.
    /// Used by [`PersistentPersonaLog::clear`] after the storage
    /// rows are deleted. The HMAC key is preserved; subsequent
    /// `append` calls produce a chain starting at seq 0 again,
    /// signing against the same key.
    pub fn clear(&self) {
        self.entries.lock().unwrap().clear();
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

    /// Phase 65 — wipe the chain. Deletes every persisted row under
    /// the underlying `KeyDomain::Persona` handle, then clears the
    /// in-memory chain. The HMAC key is preserved — subsequent
    /// appends produce a fresh chain starting at seq 0 signed
    /// against the same key.
    ///
    /// **Not atomic.** A daemon crash partway through the per-row
    /// deletes leaves the chain in an inconsistent state (some
    /// rows gone from storage, the in-memory chain still showing
    /// them). The Phase 65 Q1(a) sign-off accepted this: real
    /// atomic-tx wrapping is deferred to a follow-on phase if
    /// pressure surfaces. Operators can recover by re-importing.
    ///
    /// Used by the `ImportPersonaChain` IPC handler when `force =
    /// true`.
    pub async fn clear(&self) -> Result<(), PersonaChainError> {
        let current_entries = self.chain.entries();
        for entry in &current_entries {
            let key = entry.seq.to_be_bytes();
            self.storage
                .delete(&key)
                .await
                .map_err(|e| PersonaChainError::Storage(e.to_string()))?;
        }
        self.chain.clear();
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// EffectivePersona — replay the chain into a runtime state.
// ---------------------------------------------------------------------------


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

/// Recompute the shared effective Persona from the full chain
/// under the write lock. Returns `true` on success; `false` if the
/// lock was poisoned.
///
/// Phase 60 — replaces the Phase 59 `apply_delta_to_shared(delta)`
/// API. The Revert op needs chain context to find its target, so
/// the single-delta apply is no longer sufficient.
/// `reflection.apply` calls this after each successful chain append
/// to refresh the runtime state.
pub fn recompute_shared_from_entries(
    shared: &SharedEffectivePersona,
    entries: &[SignedPersonaEntry],
) -> bool {
    if let Ok(mut state) = shared.write() {
        *state = compute_effective_persona(entries);
        true
    } else {
        false
    }
}



/// Fold every approved delta in `entries` (in chain order) into a
/// fresh [`EffectivePersona`]. Pure function — same input always
/// produces the same output, so daemons replay chains
/// deterministically at startup.
///
/// Phase 60 — handles `PersonaDeltaOp::Revert` by replaying the
/// chain up to the target entry and applying its inverse. Cycles
/// are structurally impossible: Revert targets must be prior
/// entries in the chain (enforced by chain append order).
pub fn compute_effective_persona(entries: &[SignedPersonaEntry]) -> EffectivePersona {
    let mut state = EffectivePersona::default();
    for i in 0..entries.len() {
        apply_entry_to_state(i, entries, &mut state);
    }
    state
}

/// Apply the entry at `entries[idx]` to `state`. Dispatches on the
/// delta's op kind: forward-shape ops apply normally; Revert ops
/// apply the inverse of their target.
fn apply_entry_to_state(
    idx: usize,
    entries: &[SignedPersonaEntry],
    state: &mut EffectivePersona,
) {
    let delta = &entries[idx].delta;
    match &delta.op {
        PersonaDeltaOp::Revert { target_delta_id } => {
            if let Some(target_idx) = entries
                .iter()
                .position(|e| &e.delta.delta_id == target_delta_id)
            {
                // Phase 60: a revert can only refer to a prior entry.
                // Forward-pointing reverts are silently no-ops —
                // defense against chain corruption / malicious tampering
                // (the chain MAC would catch tamper, but the fold path
                // gives a second line of defense).
                if target_idx < idx {
                    apply_inverse_of_entry(target_idx, entries, state);
                }
            }
        }
        _ => apply_forward_op(delta, state),
    }
}

/// Apply a forward-shape op (SetScalar / AppendList / RemoveList) to
/// `state`. Revert is handled separately by [`apply_entry_to_state`].
fn apply_forward_op(delta: &PersonaDelta, state: &mut EffectivePersona) {
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

/// Apply the inverse of `entries[idx]`'s op to `state`. Phase 60 —
/// the core of revert semantics.
///
/// - `AppendList` ↔ `RemoveList`
/// - `SetScalar { value: <new> }` → `SetScalar { value: <prior> }`
///   where `<prior>` is the most-recent scalar value for the same
///   category before this entry, found by replaying `entries[..idx]`
///   into a probe state.
/// - `Revert { target }` → re-apply `target` forward. This is the
///   revert-of-revert case: undoing a revert restores the original
///   effect.
fn apply_inverse_of_entry(
    idx: usize,
    entries: &[SignedPersonaEntry],
    state: &mut EffectivePersona,
) {
    let entry = &entries[idx];
    match &entry.delta.op {
        PersonaDeltaOp::AppendList { value } => {
            apply_remove_from_list(
                field_for_list_category(state, entry.delta.category),
                value,
            );
        }
        PersonaDeltaOp::RemoveList { value } => {
            apply_append_to_list(
                field_for_list_category(state, entry.delta.category),
                value,
            );
        }
        PersonaDeltaOp::SetScalar { .. } => {
            let prior = find_prior_scalar(entries, idx, entry.delta.category);
            match entry.delta.category {
                PersonaDeltaCategory::AssistantName => state.assistant_name = prior,
                PersonaDeltaCategory::OperatorProfile => state.operator_profile = prior,
                PersonaDeltaCategory::CommunicationStyle => {
                    state.communication_style = prior
                }
                _ => {} // SetScalar is only valid on scalar categories.
            }
        }
        PersonaDeltaOp::Revert { target_delta_id } => {
            if let Some(t_idx) = entries
                .iter()
                .position(|e| &e.delta.delta_id == target_delta_id)
            {
                if t_idx < idx {
                    // Re-apply the original target. Recursive — but
                    // bounded by the chain length since each step
                    // strictly decreases the index.
                    apply_entry_to_state(t_idx, entries, state);
                }
            }
        }
    }
}

/// Find the scalar value for `category` that would have been in
/// effect immediately before `entries[at_idx]` was applied. Used by
/// `apply_inverse_of_entry` to invert a `SetScalar`.
fn find_prior_scalar(
    entries: &[SignedPersonaEntry],
    at_idx: usize,
    category: PersonaDeltaCategory,
) -> Option<String> {
    let mut probe = EffectivePersona::default();
    for i in 0..at_idx {
        apply_entry_to_state(i, entries, &mut probe);
    }
    match category {
        PersonaDeltaCategory::AssistantName => probe.assistant_name,
        PersonaDeltaCategory::OperatorProfile => probe.operator_profile,
        PersonaDeltaCategory::CommunicationStyle => probe.communication_style,
        _ => None,
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
        PersonaDeltaCategory::LearnedSkill => &mut state.learned_skills,
        PersonaDeltaCategory::ProfileHint => &mut state.profile_hints,
        PersonaDeltaCategory::RoleDefinitionSuggestion => &mut state.role_drafts,
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

/// Chapter W — `proposal_id` sentinel stamped on every onboarding-seed delta,
/// so the operator can tell a genesis-seeded delta from a learned one (the V.4
/// Change-History viewer carries `proposal_id`).
pub const SEED_PROPOSAL_ID: &str = "genesis-seed";

/// Chapter W — plant the operator's onboarding `[persona_seed]` onto the persona
/// chain, **once**, iff the chain is empty.
///
/// The seed is the operator's first *authored* content on the chain, so it is
/// appended directly as approved [`PersonaDelta`]s (one per facet / skill) via
/// the signed [`PersistentPersonaLog::append`] — exactly like the import-replay
/// path, **not** routed through the proposal/approval gate. After appending, the
/// shared effective persona is recomputed so the very next turn's system prompt
/// includes the seed (live adoption). One [`AuditEvent::PersonaSeeded`] entry
/// records the shape.
///
/// Returns the number of seed deltas appended. Returns `Ok(0)` (a no-op) when:
/// - the chain already has any entry (never overwrite a grown persona), or
/// - the seed is empty after normalization.
///
/// Only the **learned** categories are seeded (`LearnedContext`,
/// `CommunicationAdaptations`, `CharacterTraits`, `RelationshipMilestones`) plus
/// starter [`LearnedSkill`]s — the Profile-mirror scalars stay declared in
/// `[profile]`.
pub async fn seed_persona_chain_if_empty(
    log: &PersistentPersonaLog,
    shared: &SharedEffectivePersona,
    audit: Option<&aivyx_audit::PersistentAuditLog>,
    seed: &aivyx_config::PersonaSeed,
) -> Result<u64, PersonaChainError> {
    // Never overwrite a grown persona — the seed is a one-time genesis.
    if !log.entries().is_empty() {
        return Ok(0);
    }

    // Build the (category, op) seed list in a stable order.
    let mut ops: Vec<(PersonaDeltaCategory, PersonaDeltaOp)> = Vec::new();

    for (cat, values) in [
        (PersonaDeltaCategory::LearnedContext, &seed.learned_context),
        (
            PersonaDeltaCategory::CommunicationAdaptations,
            &seed.communication_adaptations,
        ),
        (PersonaDeltaCategory::CharacterTraits, &seed.character_traits),
        (
            PersonaDeltaCategory::RelationshipMilestones,
            &seed.relationship_milestones,
        ),
    ] {
        for v in values {
            ops.push((cat, PersonaDeltaOp::AppendList { value: v.clone() }));
        }
    }

    // Starter skills → a `LearnedSkill`-category AppendList of the JSON payload.
    for sk in &seed.skills {
        let learned = LearnedSkill {
            name: sk.name.clone(),
            trigger: sk.trigger.clone(),
            procedure: sk.procedure.clone(),
            ..Default::default()
        };
        ops.push((
            PersonaDeltaCategory::LearnedSkill,
            PersonaDeltaOp::AppendList {
                value: learned.to_json_value(),
            },
        ));
    }

    if ops.is_empty() {
        return Ok(0);
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let mut count: u64 = 0;
    for (i, (category, op)) in ops.into_iter().enumerate() {
        let delta = PersonaDelta {
            delta_id: synthesize_delta_id(SEED_PROPOSAL_ID, category, &op, i as u32),
            proposed_at_unix_ms: now,
            approved_at_unix_ms: now,
            proposal_id: SEED_PROPOSAL_ID.to_string(),
            category,
            op,
        };
        log.append(delta).await?;
        count += 1;
    }

    // Live adoption — recompute the shared effective persona from the chain so
    // the next turn picks the seed up (no restart).
    recompute_shared_from_entries(shared, &log.entries());

    // Best-effort audit: one PersonaSeeded entry recording the shape. (Callers
    // that don't yet have an audit handle — e.g. the daemon's boot path, where
    // the audit log opens later — pass `None` and emit it themselves via
    // [`seed_category_labels`].)
    if let Some(a) = audit {
        use aivyx_audit::AuditWriter;
        let _ = a.append(aivyx_audit::AuditEvent::PersonaSeeded {
            entries: count,
            categories: seed_category_labels(seed),
        });
    }

    Ok(count)
}

/// Chapter W — the comma-joined category labels a [`PersonaSeed`] would seed
/// (only the non-empty ones), for the [`AuditEvent::PersonaSeeded`] summary.
/// Shared by [`seed_persona_chain_if_empty`] and the daemon's deferred-audit
/// boot path (which seeds before its audit log is open).
pub fn seed_category_labels(seed: &aivyx_config::PersonaSeed) -> String {
    let mut labels: Vec<&str> = Vec::new();
    if !seed.learned_context.is_empty() {
        labels.push("learned_context");
    }
    if !seed.communication_adaptations.is_empty() {
        labels.push("communication_adaptations");
    }
    if !seed.character_traits.is_empty() {
        labels.push("character_traits");
    }
    if !seed.relationship_milestones.is_empty() {
        labels.push("relationship_milestones");
    }
    if !seed.skills.is_empty() {
        labels.push("skill");
    }
    labels.join(", ")
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
            supersedes_proposal_id: None,
        };
        assert!(good.validate().is_ok());

        let bad = ProposedPersonaDelta {
            category: PersonaDeltaCategory::OperatorProfile,
            op: PersonaDeltaOp::AppendList {
                value: "nope".into(),
            },
            reason: None,
            supersedes_proposal_id: None,
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
    fn recompute_shared_from_entries_mutates_under_write_lock() {
        let shared = shared_effective_persona(EffectivePersona::default());
        let chain = PersonaChainLog::new(test_key());
        chain
            .append(list_delta(
                PersonaDeltaCategory::BehavioralPreferences,
                "prefer terse",
            ))
            .expect("append");
        let ok = recompute_shared_from_entries(&shared, &chain.entries());
        assert!(ok);
        let read = shared.read().unwrap();
        assert_eq!(read.behavioral_preferences, vec!["prefer terse"]);
    }

    #[test]
    fn recompute_shared_from_entries_idempotent_on_repeated_appends() {
        let shared = shared_effective_persona(EffectivePersona::default());
        let chain = PersonaChainLog::new(test_key());
        chain
            .append(list_delta(
                PersonaDeltaCategory::LearnedContext,
                "operator uses Vim",
            ))
            .expect("append 1");
        chain
            .append(list_delta(
                PersonaDeltaCategory::LearnedContext,
                "operator uses Vim",
            ))
            .expect("append 2");
        recompute_shared_from_entries(&shared, &chain.entries());
        let read = shared.read().unwrap();
        assert_eq!(read.learned_context.len(), 1);
    }

    // ---- PersistentPersonaLog round-trip ------------------------

    // ---- Revert mechanism (Phase 60) ----------------------------

    fn revert_delta(category: PersonaDeltaCategory, target_id: &str) -> PersonaDelta {
        PersonaDelta {
            delta_id: format!("pd-revert-{target_id}"),
            proposed_at_unix_ms: 1_715_000_000_000,
            approved_at_unix_ms: 1_715_000_060_000,
            proposal_id: "rp-revert".into(),
            category,
            op: PersonaDeltaOp::Revert {
                target_delta_id: target_id.to_string(),
            },
        }
    }

    #[test]
    fn revert_validates_on_any_category() {
        // Revert is accepted on both scalar and list categories;
        // the category constraint comes from the target's category
        // checked at fold time.
        let r1 = revert_delta(PersonaDeltaCategory::AssistantName, "pd-x");
        assert!(r1.validate().is_ok());
        let r2 = revert_delta(PersonaDeltaCategory::BehavioralPreferences, "pd-y");
        assert!(r2.validate().is_ok());
    }

    #[test]
    fn revert_undoes_append_list() {
        let chain = PersonaChainLog::new(test_key());
        chain
            .append(list_delta(
                PersonaDeltaCategory::BehavioralPreferences,
                "prefer terse",
            ))
            .expect("append");
        let target_id = chain.entries()[0].delta.delta_id.clone();
        chain
            .append(revert_delta(
                PersonaDeltaCategory::BehavioralPreferences,
                &target_id,
            ))
            .expect("revert");
        let state = compute_effective_persona(&chain.entries());
        assert!(
            state.behavioral_preferences.is_empty(),
            "reverted AppendList should leave the list empty"
        );
    }

    #[test]
    fn revert_undoes_remove_list() {
        let chain = PersonaChainLog::new(test_key());
        // Seed: append.
        chain
            .append(list_delta(
                PersonaDeltaCategory::LearnedContext,
                "operator uses Vim",
            ))
            .expect("append");
        // Remove the value.
        let remove = PersonaDelta {
            category: PersonaDeltaCategory::LearnedContext,
            op: PersonaDeltaOp::RemoveList {
                value: "operator uses Vim".into(),
            },
            ..list_delta(PersonaDeltaCategory::LearnedContext, "dummy")
        };
        chain.append(remove).expect("remove");
        let remove_id = chain.entries()[1].delta.delta_id.clone();
        // Revert the remove → the value comes back.
        chain
            .append(revert_delta(
                PersonaDeltaCategory::LearnedContext,
                &remove_id,
            ))
            .expect("revert");
        let state = compute_effective_persona(&chain.entries());
        assert_eq!(state.learned_context, vec!["operator uses Vim".to_string()]);
    }

    #[test]
    fn revert_undoes_set_scalar_to_prior_value() {
        let chain = PersonaChainLog::new(test_key());
        // First SetScalar — prior value baseline.
        chain
            .append(scalar_delta(
                PersonaDeltaCategory::AssistantName,
                Some("Codex"),
            ))
            .expect("set 1");
        // Second SetScalar — what the revert will undo.
        chain
            .append(scalar_delta(
                PersonaDeltaCategory::AssistantName,
                Some("Mira"),
            ))
            .expect("set 2");
        let target_id = chain.entries()[1].delta.delta_id.clone();
        // Revert the second SetScalar — assistant_name reverts to "Codex".
        chain
            .append(revert_delta(
                PersonaDeltaCategory::AssistantName,
                &target_id,
            ))
            .expect("revert");
        let state = compute_effective_persona(&chain.entries());
        assert_eq!(state.assistant_name.as_deref(), Some("Codex"));
    }

    #[test]
    fn revert_undoes_set_scalar_to_none_when_no_prior() {
        let chain = PersonaChainLog::new(test_key());
        // Only one SetScalar — reverting it should clear the field.
        chain
            .append(scalar_delta(
                PersonaDeltaCategory::CommunicationStyle,
                Some("terse"),
            ))
            .expect("set");
        let target_id = chain.entries()[0].delta.delta_id.clone();
        chain
            .append(revert_delta(
                PersonaDeltaCategory::CommunicationStyle,
                &target_id,
            ))
            .expect("revert");
        let state = compute_effective_persona(&chain.entries());
        assert!(state.communication_style.is_none());
    }

    #[test]
    fn revert_of_revert_restores_original() {
        let chain = PersonaChainLog::new(test_key());
        chain
            .append(list_delta(
                PersonaDeltaCategory::BehavioralPreferences,
                "prefer terse",
            ))
            .expect("append");
        let original_id = chain.entries()[0].delta.delta_id.clone();
        chain
            .append(revert_delta(
                PersonaDeltaCategory::BehavioralPreferences,
                &original_id,
            ))
            .expect("revert 1");
        let revert_id = chain.entries()[1].delta.delta_id.clone();
        chain
            .append(revert_delta(
                PersonaDeltaCategory::BehavioralPreferences,
                &revert_id,
            ))
            .expect("revert 2");
        let state = compute_effective_persona(&chain.entries());
        assert_eq!(
            state.behavioral_preferences,
            vec!["prefer terse".to_string()],
            "revert-of-revert should restore the original effect"
        );
    }

    #[test]
    fn revert_with_missing_target_is_no_op() {
        let chain = PersonaChainLog::new(test_key());
        chain
            .append(revert_delta(
                PersonaDeltaCategory::BehavioralPreferences,
                "pd-does-not-exist",
            ))
            .expect("append revert");
        let state = compute_effective_persona(&chain.entries());
        // No prior matching delta → revert is a no-op rather than
        // a panic; the runtime stays empty.
        assert!(state.behavioral_preferences.is_empty());
    }

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

    /// Open a fresh empty persona log backed by a scratch redb store. Returns
    /// the log + the `TempDir` (which must outlive the log — it owns the file).
    async fn fresh_seed_log() -> (PersistentPersonaLog, TempDir) {
        use aivyx_crypto::MasterKey;
        use aivyx_storage::{KeyDomain, RedbStorage, Storage, StorageConfig};
        use std::sync::Arc;
        let dir = tempdir();
        let store: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.path().join("store.redb")),
            MasterKey::from_raw([7u8; 32]),
        )
        .await
        .expect("scratch storage opens");
        let log = PersistentPersonaLog::open(store.domain(KeyDomain::Persona), test_key())
            .await
            .expect("empty log opens");
        (log, dir)
    }

    fn sample_seed() -> aivyx_config::PersonaSeed {
        aivyx_config::PersonaSeed {
            learned_context: vec!["operator builds Aivyx".into()],
            communication_adaptations: vec![],
            character_traits: vec!["precise".into(), "pragmatic".into()],
            relationship_milestones: vec!["genesis: first launch".into()],
            skills: vec![aivyx_config::SeedSkill {
                name: "rust-review".into(),
                trigger: "when reviewing Rust".into(),
                procedure: "check unwraps + lifetimes".into(),
            }],
        }
    }

    #[tokio::test]
    async fn seed_appends_facets_and_skills_and_adopts() {
        let (log, _dir) = fresh_seed_log().await;
        let shared = shared_effective_persona(EffectivePersona::default());

        let n = seed_persona_chain_if_empty(&log, &shared, None, &sample_seed())
            .await
            .expect("seed ok");
        // 1 learned_context + 2 character_traits + 1 milestone + 1 skill = 5.
        assert_eq!(n, 5);
        assert_eq!(log.len(), 5);
        log.verify().expect("seeded chain verifies");

        // Live adoption — the shared effective persona reflects the seed.
        let snap = shared.read().unwrap();
        assert_eq!(snap.learned_context, vec!["operator builds Aivyx".to_string()]);
        assert!(snap.character_traits.contains(&"precise".to_string()));
        assert!(snap.character_traits.contains(&"pragmatic".to_string()));
        assert_eq!(snap.learned_skills.len(), 1);
        let skill = LearnedSkill::from_json_value(&snap.learned_skills[0]).expect("skill parses");
        assert_eq!(skill.name, "rust-review");

        // Every seed delta carries the genesis-seed sentinel.
        for e in log.entries() {
            assert_eq!(e.delta.proposal_id, SEED_PROPOSAL_ID);
        }
    }

    #[tokio::test]
    async fn seed_is_noop_on_non_empty_chain() {
        let (log, _dir) = fresh_seed_log().await;
        // Pre-seed the chain with one learned delta — simulating a grown persona.
        log.append(list_delta(PersonaDeltaCategory::CharacterTraits, "curious"))
            .await
            .expect("pre-append");
        let shared = shared_effective_persona(EffectivePersona::default());

        let n = seed_persona_chain_if_empty(&log, &shared, None, &sample_seed())
            .await
            .expect("seed ok");
        assert_eq!(n, 0, "must never overwrite a grown persona");
        assert_eq!(log.len(), 1, "chain unchanged");
    }

    #[tokio::test]
    async fn seed_empty_is_noop() {
        let (log, _dir) = fresh_seed_log().await;
        let shared = shared_effective_persona(EffectivePersona::default());
        let n = seed_persona_chain_if_empty(
            &log,
            &shared,
            None,
            &aivyx_config::PersonaSeed::default(),
        )
        .await
        .expect("seed ok");
        assert_eq!(n, 0);
        assert!(log.is_empty());
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

    #[tokio::test]
    async fn persistent_log_clear_wipes_storage_and_memory() {
        // Phase 65 — `PersistentPersonaLog::clear` is the
        // load-bearing primitive for the import-with-force
        // path. Verify it removes both the in-memory chain
        // and the persisted rows.
        use aivyx_crypto::MasterKey;
        use aivyx_storage::{KeyDomain, RedbStorage, Storage, StorageConfig};
        use std::sync::Arc;

        let dir = tempdir();
        let store: Arc<dyn Storage> = RedbStorage::open(
            StorageConfig::new(dir.path().join("store.redb")),
            MasterKey::from_raw([77u8; 32]),
        )
        .await
        .expect("storage opens");
        let handle = store.domain(KeyDomain::Persona);

        let key = test_key();
        let log = PersistentPersonaLog::open(handle.clone(), key.clone())
            .await
            .expect("log opens");
        log.append(scalar_delta(
            PersonaDeltaCategory::AssistantName,
            Some("Codex"),
        ))
        .await
        .expect("append 1");
        log.append(list_delta(
            PersonaDeltaCategory::BehavioralPreferences,
            "prefer tests over mocks",
        ))
        .await
        .expect("append 2");
        assert_eq!(log.len(), 2);

        // Clear.
        log.clear().await.expect("clear ok");
        assert_eq!(log.len(), 0);
        assert!(log.is_empty());
        drop(log);

        // Re-open: the persisted rows are gone too.
        let reopened = PersistentPersonaLog::open(handle, key)
            .await
            .expect("reopen succeeds");
        assert_eq!(reopened.len(), 0);

        // Subsequent append produces seq 0 (fresh chain).
        let seq = reopened
            .append(scalar_delta(
                PersonaDeltaCategory::AssistantName,
                Some("Mira"),
            ))
            .await
            .expect("append after clear");
        assert_eq!(seq, 0);
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
            // A UUID, not just pid+nanos: two parallel tests in the same
            // process can hit the same nanosecond (coarse clock resolution)
            // and collide on the same redb path → a transient flake.
            let suffix = format!("aivyx-persona-test-{}", uuid::Uuid::new_v4());
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

    // ----- Phase 118 — ProfileHint + RoleDefinitionSuggestion categories -----

    #[test]
    fn profile_hint_and_role_suggestion_categories_are_list_shaped() {
        // Both new Phase 118 categories are list-shaped: each
        // approved entry appends a JSON-serialized payload.
        // `is_scalar` returning `false` is what drives
        // PersonaDelta::validate to accept AppendList /
        // RemoveList ops on these categories.
        assert!(!PersonaDeltaCategory::ProfileHint.is_scalar());
        assert!(!PersonaDeltaCategory::RoleDefinitionSuggestion.is_scalar());
    }

    #[test]
    fn profile_hint_append_list_validates() {
        let d = list_delta(PersonaDeltaCategory::ProfileHint, "{\"field\":\"...\"}");
        d.validate()
            .expect("AppendList on list-shaped ProfileHint must validate");
    }

    #[test]
    fn role_definition_suggestion_append_list_validates() {
        let d = list_delta(
            PersonaDeltaCategory::RoleDefinitionSuggestion,
            "{\"name\":\"research-deploy\"}",
        );
        d.validate()
            .expect("AppendList on list-shaped RoleDefinitionSuggestion must validate");
    }

    #[test]
    fn profile_hint_set_scalar_rejected() {
        // SetScalar on a list-shaped category must be
        // rejected — same validation contract as
        // BehavioralPreferences etc.
        let d = scalar_delta(PersonaDeltaCategory::ProfileHint, Some("v"));
        assert!(d.validate().is_err());
    }

    #[test]
    fn role_definition_suggestion_set_scalar_rejected() {
        let d = scalar_delta(
            PersonaDeltaCategory::RoleDefinitionSuggestion,
            Some("v"),
        );
        assert!(d.validate().is_err());
    }

    #[test]
    fn profile_hint_folds_into_profile_hints_field() {
        let chain = PersonaChainLog::new(test_key());
        chain
            .append(list_delta(
                PersonaDeltaCategory::ProfileHint,
                "{\"field\":\"CommunicationStyle\",\"suggested_value\":\"terse\",\"rationale\":\"x\"}",
            ))
            .expect("append");
        let state = compute_effective_persona(&chain.entries());
        assert_eq!(state.profile_hints.len(), 1);
        assert!(state.profile_hints[0].contains("CommunicationStyle"));
        // Untouched fields stay default — the new fold only
        // routes into `profile_hints`.
        assert!(state.role_drafts.is_empty());
        assert!(state.behavioral_preferences.is_empty());
    }

    #[test]
    fn role_definition_suggestion_folds_into_role_drafts_field() {
        let chain = PersonaChainLog::new(test_key());
        chain
            .append(list_delta(
                PersonaDeltaCategory::RoleDefinitionSuggestion,
                "{\"name\":\"research-deploy\",\"parent\":null,\
                  \"system_prompt_addendum\":\"...\",\
                  \"tool_allowlist_additions\":[],\"rationale\":\"...\"}",
            ))
            .expect("append");
        let state = compute_effective_persona(&chain.entries());
        assert_eq!(state.role_drafts.len(), 1);
        assert!(state.role_drafts[0].contains("research-deploy"));
        assert!(state.profile_hints.is_empty());
    }

    #[test]
    fn is_non_empty_fires_on_profile_hints_only() {
        let mut p = EffectivePersona::default();
        assert!(!p.is_non_empty());
        p.profile_hints
            .push("{\"field\":\"AssistantName\"}".to_string());
        assert!(p.is_non_empty());
    }

    #[test]
    fn is_non_empty_fires_on_role_drafts_only() {
        let mut p = EffectivePersona::default();
        assert!(!p.is_non_empty());
        p.role_drafts.push("{\"name\":\"x\"}".to_string());
        assert!(p.is_non_empty());
    }
}
