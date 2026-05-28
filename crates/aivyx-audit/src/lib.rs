//! # aivyx-audit
//!
//! HMAC-chained append-only audit log for Aivyx agents.
//!
//! Every tool call, scope check, and turn outcome is appended to this log
//! synchronously as it happens — not batched at turn end. If the process
//! crashes mid-turn, the audit log still tells the truth about what got
//! executed.
//!
//! See DESIGN.md Deliverable 1 (audit is synchronous, inline, HMAC-chained)
//! and Deliverable 4 (the 5-variant `AuditEvent` enum — per-tool for
//! grants, per-scope for denials, with a dedicated `MemoryAccess` view).
//!
//! ## The chain property
//!
//! Each entry's MAC is computed over `prev_mac || canonical_bytes(event)`.
//! Tampering with any entry invalidates every subsequent MAC, so the chain
//! itself is the integrity proof — no per-entry signature required.
//!
//! The canonical bytes come from `serde_jcs` (RFC 8785 JSON
//! Canonicalization Scheme), so byte-identical output is guaranteed across
//! runs regardless of struct field declaration order.
//!
//! ## Phase 1 scope
//!
//! In-memory `HmacChainLog` only. Disk persistence will come when
//! `aivyx-storage`'s `KeyDomain::Audit` is wired in a later phase.

use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use thiserror::Error;

use aivyx_capability::{CapabilitySet, Scope};
use aivyx_core::{
    ChannelPlatform, SessionId, TokenUsage, ToolId, ToolOutcomeSummary, TurnId,
    TurnOutcomeSummary,
};

type HmacSha256 = Hmac<Sha256>;

/// Versioned seed for the genesis (pre-entry-0) MAC. Mirrors D7's versioned
/// HKDF salt — bumping to `"aivyx-audit-v2-genesis"` produces a different
/// chain lineage, enabling clean format rotation without in-place migration.
const GENESIS_SEED: &[u8] = b"aivyx-audit-v1-genesis";

// ---------------------------------------------------------------------------
// AuditEvent — the 5 variants from D4
// ---------------------------------------------------------------------------

/// The set of events appended to the audit log. Per D4:
///
/// - `ToolCall` — primary key `tool_id`; covers every tool execution
/// - `ScopeDenied` — primary key `scope`; every denied capability check
/// - `TurnStarted` / `TurnEnded` — paired via `turn_id` for correlation
/// - `MemoryAccess` — redundant with `ToolCall` but indexed for fast
///   memory-specific queries (D4's one deliberate deviation from strict
///   mixed naming)
///
/// Every variant is *self-contained* — readable without cross-referencing
/// other entries — so a single entry can be displayed in a UI or printed to
/// a log without joining against siblings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum AuditEvent {
    /// A tool executed.
    ToolCall {
        turn_id: TurnId,
        tool_id: ToolId,
        scope_used: Scope,
        /// SHA-256 of the raw input. D4: "hash, not raw input — secrets safety."
        input_hash: [u8; 32],
        outcome: ToolOutcomeSummary,
        duration: Duration,
    },

    /// A scope check denied a tool call.
    ScopeDenied {
        turn_id: TurnId,
        tool_attempted: ToolId,
        scope_requested: Scope,
        /// Snapshot of capabilities at denial time — not a reference, so the
        /// set is preserved even if the agent's caps change later.
        held_capabilities: CapabilitySet,
    },

    /// Turn started. Correlates with `TurnEnded` via `turn_id`.
    TurnStarted {
        turn_id: TurnId,
        session_id: SessionId,
        channel: ChannelPlatform,
        trust_tier: TrustTierSummary,
        /// The `agent_caps.intersect(tier_ceiling)` snapshot — authoritative
        /// for the whole turn, per D5.
        effective_capabilities: CapabilitySet,
    },

    /// Turn ended. Paired with `TurnStarted`.
    TurnEnded {
        turn_id: TurnId,
        outcome: TurnOutcomeSummary,
        tool_calls_made: usize,
        duration: Duration,
        usage: TokenUsage,
    },

    /// Dedicated view of a memory operation. Redundant with `ToolCall`
    /// (every memory op *is* also a tool call), but indexed for fast
    /// memory-specific queries. D4 justifies this as the one deviation
    /// from strict mixed-model naming.
    MemoryAccess {
        turn_id: TurnId,
        operation: MemoryOperation,
        scope: Scope,
        /// Free-form filter or key — for a recall this is the query string,
        /// for a write it's the storage key. Not hashed: memory queries /
        /// keys are already audit-safe (no raw secrets pass through them
        /// by convention).
        query_or_key: String,
    },

    /// Phase 67 — daemon-initiated auto-notify on a trigger fire.
    /// Distinct from `ToolCall` (which records agent-initiated
    /// `notify.send` calls) so forensic searches can tell apart
    /// "the agent decided to notify" from "the daemon's
    /// trigger-config sugar decided to notify."
    ///
    /// Correlation: `session_id` is recorded on the corresponding
    /// `TurnStarted` audit event from the same trigger fire — walk
    /// backward through the chain to find the matching turn.
    /// `turn_id` correlation is a Phase 67 deferral
    /// (`TurnOutcome` doesn't carry `turn_id` today; lifting it
    /// touches 120 match sites).
    /// Phase 117 — `skills.invoke` successful invocation.
    /// Emitted alongside the regular `ToolCall` audit entry
    /// for the same call so the skill name lands in cleartext
    /// without exposing the rest of the tool input. Phase 116's
    /// `record_turn_outcomes` reads this variant to populate
    /// per-skill ledger rows; the operator-side `aivyx audit
    /// export --event-type SkillInvocation` filter accepts the
    /// label.
    SkillInvocation {
        /// The turn that fired the invocation. Pair with the
        /// surrounding `TurnStarted` / `TurnEnded` via turn_id.
        turn_id: TurnId,
        /// The session whose turn fired it. Matches the
        /// surrounding `TurnStarted`.
        session_id: SessionId,
        /// The skill's stable kebab-case identifier from
        /// `LearnedSkill::name`.
        skill_name: String,
    },
    AutoNotifyDispatched {
        /// Session id minted by `TriggerDispatch::fire` for this
        /// trigger fire. Matches the `TurnStarted` /
        /// `TurnEnded` events from the same fire.
        session_id: SessionId,
        /// Which trigger kind fired (cron / webhook / file-watch).
        trigger_kind: TriggerKindSummary,
        /// Operator-declared id of the trigger that fired (e.g.
        /// `"morning-summary"`).
        trigger_id: String,
        /// Target name from `[[notify_target]]` that the auto-
        /// notify dispatched (or attempted to dispatch) to.
        target_name: String,
        /// What happened. Three forms: delivered, skipped because
        /// the agent's turn produced an empty response, failed
        /// during dispatch.
        outcome: AutoNotifyOutcomeSummary,
        /// Wall-clock timestamp of the dispatch attempt, ms since
        /// the Unix epoch. The chain's per-entry `appended_at` is
        /// the canonical audit timestamp; this is the moment the
        /// dispatcher was called, included for operator readability.
        dispatched_at_unix_ms: u64,
    },

    /// Phase 112 Task 6 — Skill Auto-Proposer fire record.
    ///
    /// Emitted once per turn where the auto-proposer ran past
    /// the heuristic gate. The outcome carries which terminal
    /// routing decision the proposer reached (or which failure
    /// mode it hit). Pair with the surrounding `TurnEnded`
    /// event via session_id to reconstruct what the agent
    /// learned (or didn't) from that turn.
    ///
    /// Confidence is stored as `confidence_thousandths` (a u32
    /// in `0..=1000`) rather than `f32` so the variant can stay
    /// `Eq` like the rest of `AuditEvent`. Read as `f32` via
    /// `confidence_thousandths as f32 / 1000.0`.
    SkillAutoProposal {
        /// The session whose turn fired the proposer. Matches
        /// the surrounding `TurnStarted` / `TurnEnded`.
        session_id: SessionId,
        /// Terminal routing outcome — what the proposer decided
        /// to do (or what error it hit).
        outcome: SkillAutoProposalOutcomeSummary,
        /// Judge confidence × 1000. Stored as integer to keep
        /// `AuditEvent: Eq` per the chain's invariant. None
        /// for outcomes that didn't reach the judge call
        /// (heuristic-gated, disabled, fuzzy-dropped pre-judge
        /// — note: Phase 112 runs fuzzy post-judge, so fuzzy
        /// dups DO have a confidence).
        confidence_thousandths: Option<u32>,
        /// Operator-readable name of the proposed draft. For
        /// `LearnedSkill` this is the kebab-case slug; for
        /// list/scalar categories (Phase 114) this is a
        /// truncated value. None for outcomes that didn't
        /// produce a draft.
        proposed_skill_name: Option<String>,
        /// Wall-clock duration of the judge call (Q1b stage 2).
        /// None for outcomes that didn't reach the judge.
        judge_latency_ms: Option<u64>,
        /// Which heuristic signals (Q1b stage 1) crossed
        /// during the candidate-gate evaluation. Lets forensic
        /// walks answer "what kind of turns are firing the
        /// proposer the most?" by tallying signal patterns.
        heuristic_signals_matched: HeuristicSignalsMatched,
        /// Phase 114 — the `PersonaDeltaCategory` label the
        /// judge picked. `None` for pre-Phase-114 entries
        /// (the field uses `#[serde(default,
        /// skip_serializing_if = "Option::is_none")]` so old
        /// chain entries round-trip byte-identically — Phase
        /// 92's `supersedes_proposal_id` precedent for wire-
        /// compatible audit-chain extensions).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        category: Option<String>,
        /// Phase 115 — what triggered this auto-proposer
        /// fire: a Phase 114 positive-pattern (Completed
        /// turn) or a Phase 115 negative-feedback path
        /// (failed turn). `None` for pre-Phase-115 entries;
        /// the absence of the field is semantically the
        /// CompletedTurn default. Same wire-compat pattern
        /// as the Phase 114 `category` field.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<ProposalSourceSummary>,
    },
}

/// Phase 115 — discriminator for what triggered an auto-
/// proposer fire. Mirrors
/// `aivyx_core::skill_proposer::ProposalSource` but lives
/// in `aivyx-audit` so the chain shape stays independent of
/// `aivyx-core`'s skill-proposer evolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum ProposalSourceSummary {
    /// Phase 114 positive-pattern path: `TurnOutcome::
    /// Completed`. Default for entries that omit the source
    /// field on read.
    CompletedTurn,
    /// Phase 115 negative-feedback path: a non-Completed
    /// `TurnOutcome`. The `failure_kind` field carries the
    /// specific variant (`"failed"` / `"cancelled"` /
    /// `"timed_out"` / `"escalated"`).
    FailedTurn { failure_kind: String },
}

/// Phase 112 — Skill Auto-Proposer outcome discriminator.
/// Mirrors `aivyx_channel::skill_auto_proposer::SkillProposerOutcome`
/// fused with the routing-decision label space (Task 5's
/// `SkillRoutingDecision::label()`). Lives here in `aivyx-audit`
/// so the chain shape stays independent of `aivyx-channel`'s
/// orchestration layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum SkillAutoProposalOutcomeSummary {
    /// Master switch was off; the proposer was bypassed.
    Disabled,
    /// Heuristic gate rejected the turn — no LLM call fired.
    HeuristicGated,
    /// Judge fired and judged the candidate worth proposing,
    /// confidence reached the auto-accept threshold, no dup
    /// detected on either the LLM-semantic or the fuzzy-title
    /// pre-filter. Landed in the LearnedSkill chain as an
    /// approved entry.
    AutoAccepted,
    /// Judge fired and judged the candidate worth proposing,
    /// but confidence was below the auto-accept threshold.
    /// Landed in the proposal chain as Pending — the
    /// operator will resolve via `aivyx persona proposals`.
    Staged,
    /// Judge declared the candidate a semantic duplicate of an
    /// existing skill. Nothing written.
    DuplicateOfExistingLlm { duplicate_of: String },
    /// Title fuzzy-match against existing skills caught a
    /// paraphrase / token-reorder the judge missed. Nothing
    /// written.
    DuplicateOfExistingFuzzy { matched_existing_name: String },
    /// Judge said `is_worth_proposing == false` (and not a
    /// dup). Nothing written.
    NotWorthProposing,
    /// Judge call failed (provider error, parse failure, or
    /// confidence out of range). Carries the error message
    /// for operator forensics.
    JudgeError { error_message: String },
}

/// Phase 112 — Bitmap-style record of which heuristic signals
/// (Q1b stage 1) crossed their thresholds during candidate
/// gating. All four fields are booleans, but we use a struct
/// rather than a `Vec<String>` so the audit chain stays
/// schema-stable and forensic queries can be exact-match
/// rather than substring.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeuristicSignalsMatched {
    pub tool_call_count: bool,
    pub distinct_tool_id_count: bool,
    pub duration: bool,
    pub gate_resolve: bool,
}

/// Phase 67 — auto-notify outcome discriminator.
///
/// Mirrors `aivyx_channel::trigger`'s three post-turn dispatch
/// paths: dispatch succeeded, dispatch deliberately skipped
/// because the turn produced an empty body, dispatch failed.
/// Carried as the `outcome` field of [`AuditEvent::AutoNotifyDispatched`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum AutoNotifyOutcomeSummary {
    /// The dispatcher backend returned `Ok(())`. The notification
    /// has been delivered to the target (or at least handed off
    /// to it — for webhooks, "delivered" means the endpoint
    /// returned 2xx).
    Delivered,
    /// The turn outcome's body was empty so the dispatcher was
    /// deliberately not called (Phase 63 Q2(a) at sign-off).
    /// Recording this in the chain means operators can answer
    /// "why didn't my notification arrive?" definitively.
    SkippedEmptyResponse,
    /// The dispatcher backend returned an error. `error_kind`
    /// mirrors the `notify.send` tool's classification:
    /// `"transport"`, `"auth"`, `"rejected"`, `"timeout"`,
    /// `"unknown_target"`.
    Failed {
        error_kind: String,
        error_message: String,
    },
    /// Phase 72 — the trigger's `notify_when` condition gate
    /// evaluated to false against the turn outcome, so the
    /// dispatcher was deliberately skipped. `condition` carries
    /// the stable string label (`"on_failed"`,
    /// `"on_completed_non_empty"`) so forensic searches can
    /// answer "why didn't this fire?" definitively.
    SkippedByCondition { condition: String },
    /// Phase 73 — the target's in-memory rate-limit token bucket
    /// was exhausted when this dispatch was attempted, so the
    /// backend call was deliberately skipped. `limit` and
    /// `window_secs` carry the effective policy at the time of
    /// the skip so audit forensics can answer "what was the
    /// rate limit when this was skipped?" without needing the
    /// live config.
    SkippedByRateLimit { limit: u32, window_secs: u64 },
}

/// Phase 67 — trigger kind label for the audit chain. Mirrors
/// `aivyx_channel::trigger::TriggerSource`; duplicated in this
/// crate to avoid a dep edge from `aivyx-audit` to
/// `aivyx-channel` (the audit chain shape is independent of the
/// channel adapter substrate).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum TriggerKindSummary {
    Cron,
    Webhook,
    FileWatch,
    /// Phase 71 — reflection-scheduler fire. Distinct from
    /// `Cron` so forensic searches can tell "the agent
    /// reflected on its own behavior" apart from "an operator-
    /// declared cron job ran." Reflection turns carry the
    /// canonical reflection prompt + outcome-summary input.
    Reflection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryOperation {
    Read,
    Write,
    Forget,
}

/// Mirror of `aivyx_capability::TrustTier` — duplicated here to avoid a
/// dependency from audit on capability's concrete enum, and because the
/// audit record only needs the tier *name*, not its behavior. The conversion
/// is one-way: `TrustTierSummary::from(tier)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrustTierSummary {
    Kernel,
    Trusted,
    SemiTrusted,
    Untrusted,
}

impl From<aivyx_capability::TrustTier> for TrustTierSummary {
    fn from(t: aivyx_capability::TrustTier) -> Self {
        use aivyx_capability::TrustTier;
        match t {
            TrustTier::Kernel => TrustTierSummary::Kernel,
            TrustTier::Trusted => TrustTierSummary::Trusted,
            TrustTier::SemiTrusted => TrustTierSummary::SemiTrusted,
            TrustTier::Untrusted => TrustTierSummary::Untrusted,
        }
    }
}

// ---------------------------------------------------------------------------
// Signed entries and the chain
// ---------------------------------------------------------------------------

/// One signed entry in the chain. The MAC binds the entry to everything
/// preceding it via `prev_mac`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedEntry {
    /// Monotonic sequence number, 0-based. Not derived from the MAC — kept
    /// explicit so tools can reference entries by seq without computing the
    /// full chain.
    pub seq: u64,
    /// Wall-clock time of append, for human display. NOT part of the MAC
    /// input — clock skew must not break integrity.
    pub appended_at: SystemTime,
    pub event: AuditEvent,
    /// 32-byte HMAC-SHA256 tag over `prev_mac || canonical_bytes(event)`.
    pub mac: [u8; 32],
    /// The previous entry's MAC (or the genesis seed for entry 0).
    pub prev_mac: [u8; 32],
}

// ---------------------------------------------------------------------------
// AuditError
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum AuditError {
    #[error("canonical serialization failed: {0}")]
    Serialize(String),

    #[error("chain verification failed at seq {seq}: {reason}")]
    ChainBroken { seq: u64, reason: String },

    #[error("lock poisoned")]
    LockPoisoned,

    /// A put/get/scan call into `aivyx-storage` failed while reading or
    /// writing `KeyDomain::Audit`. Phase 7 task 1 wraps the upstream
    /// `StorageError` as a string so `aivyx-audit`'s public error
    /// surface does not leak storage internals to dependents.
    #[error("storage error: {0}")]
    Storage(String),

    /// An on-disk record could not be decoded, or its key/seq fields
    /// disagreed with its position in the scan. Surfaces at
    /// `PersistentAuditLog::open` only — once reopened, the in-memory
    /// chain is the source of truth.
    #[error("corrupt stored entry at seq {seq}: {reason}")]
    CorruptStoredEntry { seq: u64, reason: String },
}

// ---------------------------------------------------------------------------
// AuditWriter / AuditLog traits
// ---------------------------------------------------------------------------

/// Minimal append surface — what a `ToolContext` will hold a reference to.
/// Returned by `HmacChainLog` and by `NullAuditLog`.
pub trait AuditWriter: Send + Sync {
    /// Append an event. Synchronous per D1: "blocking, microseconds per
    /// call." Returns the sequence number assigned, or an error on failure.
    fn append(&self, event: AuditEvent) -> Result<u64, AuditError>;
}

/// Full audit surface — extends `AuditWriter` with read/verify. The turn
/// loop holds an `AuditWriter` reference; test code and admin tools hold
/// an `AuditLog` reference for inspection.
pub trait AuditLog: AuditWriter {
    /// Read the entry at `seq`, or `None` if past the end.
    fn get(&self, seq: u64) -> Option<SignedEntry>;

    /// Number of entries so far.
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Walk the chain from entry 0 and recompute every MAC. Returns
    /// `Ok(())` if all MACs match and every `prev_mac` refers to the
    /// previous entry's MAC; `Err(ChainBroken)` at the first discrepancy.
    fn verify(&self) -> Result<(), AuditError>;
}

// ---------------------------------------------------------------------------
// HmacChainLog — the concrete in-memory HMAC-chained implementation.
// ---------------------------------------------------------------------------

/// In-memory HMAC-chained audit log.
///
/// The secret key is held by value; in a real deployment it's derived via
/// HKDF from the master key (D7 `KeyDomain::Audit`). For Phase 1, callers
/// pass in a key directly — storage wiring comes later.
pub struct HmacChainLog {
    key: Vec<u8>,
    inner: Mutex<Inner>,
}

struct Inner {
    entries: Vec<SignedEntry>,
}

impl HmacChainLog {
    pub fn new(key: impl Into<Vec<u8>>) -> Self {
        HmacChainLog {
            key: key.into(),
            inner: Mutex::new(Inner {
                entries: Vec::new(),
            }),
        }
    }

    /// Construct a log pre-populated with entries recovered from durable
    /// storage.
    ///
    /// **Caller must have already verified the chain** (`AuditLog::verify`
    /// semantics) over `entries` against `key` before calling this. The
    /// constructor inserts them *as-is* into the in-memory entry vec
    /// without recomputing MACs — which is the only way to honour the
    /// invariant that in-memory entries are byte-identical to what the
    /// reopen path blessed on disk. Recomputing here would hide any
    /// tamper the caller's verify missed.
    ///
    /// Intended exclusively for `PersistentAuditLog::open`'s reopen path.
    /// Subsequent `append` calls chain off the last entry's `mac` as
    /// usual, so seq numbering continues monotonically from
    /// `entries.len()`.
    pub fn from_verified_entries(
        key: impl Into<Vec<u8>>,
        entries: Vec<SignedEntry>,
    ) -> Self {
        HmacChainLog {
            key: key.into(),
            inner: Mutex::new(Inner { entries }),
        }
    }

    /// Snapshot of all entries — cloned. Intended for tests and admin
    /// tools, not for hot paths.
    pub fn entries(&self) -> Result<Vec<SignedEntry>, AuditError> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| AuditError::LockPoisoned)?
            .entries
            .clone())
    }

    /// Ranged snapshot — clone at most `limit` entries starting at
    /// `from_seq`. Returns an empty vec if `from_seq` is past the end.
    ///
    /// Phase 47 — used by the daemon's `ListAuditEntries` query to
    /// satisfy the Web UI audit viewer without ever materializing the
    /// full chain into a single response. Caller-supplied `limit` is
    /// capped by the daemon at 500 (Phase 47 Q3); this method itself
    /// imposes no upper bound — short reads are returned verbatim.
    pub fn entries_range(
        &self,
        from_seq: u64,
        limit: usize,
    ) -> Result<Vec<SignedEntry>, AuditError> {
        let inner = self.inner.lock().map_err(|_| AuditError::LockPoisoned)?;
        let start = from_seq as usize;
        if start >= inner.entries.len() {
            return Ok(Vec::new());
        }
        let end = (start + limit).min(inner.entries.len());
        Ok(inner.entries[start..end].to_vec())
    }

    fn compute_mac(&self, prev_mac: &[u8; 32], event_bytes: &[u8]) -> [u8; 32] {
        let mut mac = <HmacSha256 as KeyInit>::new_from_slice(&self.key)
            .expect("HMAC accepts any key length");
        mac.update(prev_mac);
        mac.update(event_bytes);
        let out = mac.finalize().into_bytes();
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&out);
        arr
    }
}

impl AuditWriter for HmacChainLog {
    fn append(&self, event: AuditEvent) -> Result<u64, AuditError> {
        let event_bytes =
            serde_jcs::to_vec(&event).map_err(|e| AuditError::Serialize(e.to_string()))?;

        let mut inner = self.inner.lock().map_err(|_| AuditError::LockPoisoned)?;

        let seq = inner.entries.len() as u64;
        let prev_mac = match inner.entries.last() {
            Some(prev) => prev.mac,
            None => {
                let mut seed = [0u8; 32];
                let src = GENESIS_SEED;
                // Left-pad: copy the seed into the *end* of the array; leading
                // zeros fill the rest. Deterministic and future-proof against
                // lengthening the seed string.
                let start = seed.len() - src.len();
                seed[start..].copy_from_slice(src);
                seed
            }
        };
        let mac = self.compute_mac(&prev_mac, &event_bytes);

        let entry = SignedEntry {
            seq,
            appended_at: SystemTime::now(),
            event,
            mac,
            prev_mac,
        };
        inner.entries.push(entry);
        Ok(seq)
    }
}

impl AuditLog for HmacChainLog {
    fn get(&self, seq: u64) -> Option<SignedEntry> {
        self.inner.lock().ok()?.entries.get(seq as usize).cloned()
    }

    fn len(&self) -> usize {
        self.inner
            .lock()
            .map(|i| i.entries.len())
            .unwrap_or(0)
    }

    fn verify(&self) -> Result<(), AuditError> {
        let entries = self
            .inner
            .lock()
            .map_err(|_| AuditError::LockPoisoned)?
            .entries
            .clone();

        let mut expected_prev = {
            let mut seed = [0u8; 32];
            let src = GENESIS_SEED;
            let start = seed.len() - src.len();
            seed[start..].copy_from_slice(src);
            seed
        };

        for (idx, entry) in entries.iter().enumerate() {
            if entry.seq != idx as u64 {
                return Err(AuditError::ChainBroken {
                    seq: idx as u64,
                    reason: format!("seq field = {}, expected {}", entry.seq, idx),
                });
            }
            if entry.prev_mac != expected_prev {
                return Err(AuditError::ChainBroken {
                    seq: entry.seq,
                    reason: "prev_mac does not match previous entry's mac".into(),
                });
            }
            let bytes = serde_jcs::to_vec(&entry.event)
                .map_err(|e| AuditError::Serialize(e.to_string()))?;
            let expected_mac = self.compute_mac(&expected_prev, &bytes);
            if expected_mac != entry.mac {
                return Err(AuditError::ChainBroken {
                    seq: entry.seq,
                    reason: "MAC does not match recomputation over canonical event bytes".into(),
                });
            }
            expected_prev = entry.mac;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// NullAuditLog — no-op writer for tests that don't need integrity.
// ---------------------------------------------------------------------------

/// Appends are accepted and dropped. `len()` always reports 0; `verify()`
/// always succeeds. Intended for fake `ToolContext` wiring in Phase 1
/// task 4 where the test cares about control flow, not audit.
pub struct NullAuditLog;

impl AuditWriter for NullAuditLog {
    fn append(&self, _event: AuditEvent) -> Result<u64, AuditError> {
        Ok(0)
    }
}

impl AuditLog for NullAuditLog {
    fn get(&self, _seq: u64) -> Option<SignedEntry> {
        None
    }

    fn len(&self) -> usize {
        0
    }

    fn verify(&self) -> Result<(), AuditError> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Bridge: aivyx_core::AuditHook → aivyx_audit::AuditWriter
// ---------------------------------------------------------------------------
//
// `aivyx-core` declares a forward `AuditHook` trait with an `AuditTag` enum
// so the turn loop can emit audit events without depending on this crate.
// The bridge below closes the loop: any `AuditWriter` (e.g. `HmacChainLog`)
// can be wrapped in an `AuditBridge` and handed to `ConcreteAgent::new` as
// an `Arc<dyn AuditHook>`.
//
// Why an explicit adapter and not a blanket `impl<W: AuditWriter> AuditHook
// for W`? D4 commits to audit failures being *visible* — not swallowed. A
// blanket impl has nowhere to route an `AuditError`, so the bridge forces
// callers to declare their error-handling strategy at construction time.
// The default (`AuditBridge::new`) panics, matching D4's "audit must
// complete before the call returns" spirit: a broken chain is a
// configuration bug, not an operational condition.

/// Translates `aivyx_core::MemoryOperation` into the audit-owned copy. The
/// two enums are kept separate so core does not depend on audit's serde
/// machinery; this impl is the single translation point.
impl From<aivyx_core::MemoryOperation> for MemoryOperation {
    fn from(op: aivyx_core::MemoryOperation) -> Self {
        match op {
            aivyx_core::MemoryOperation::Read => MemoryOperation::Read,
            aivyx_core::MemoryOperation::Write => MemoryOperation::Write,
            aivyx_core::MemoryOperation::Forget => MemoryOperation::Forget,
        }
    }
}

/// Translates a forward-declared `AuditTag` from the turn loop into the
/// `AuditEvent` shape the HMAC chain appends. The translation is mostly
/// field-for-field — only `trust_tier` and `operation` need conversion
/// through their respective `From` impls.
impl From<aivyx_core::AuditTag> for AuditEvent {
    fn from(tag: aivyx_core::AuditTag) -> Self {
        use aivyx_core::AuditTag;
        match tag {
            AuditTag::TurnStarted {
                turn_id,
                session_id,
                channel,
                trust_tier,
                effective_capabilities,
            } => AuditEvent::TurnStarted {
                turn_id,
                session_id,
                channel,
                trust_tier: trust_tier.into(),
                effective_capabilities,
            },
            AuditTag::TurnEnded {
                turn_id,
                outcome,
                tool_calls_made,
                duration,
                usage,
            } => AuditEvent::TurnEnded {
                turn_id,
                outcome,
                tool_calls_made,
                duration,
                usage,
            },
            AuditTag::ToolCall {
                turn_id,
                tool_id,
                scope_used,
                input_hash,
                outcome,
                duration,
            } => AuditEvent::ToolCall {
                turn_id,
                tool_id,
                scope_used,
                input_hash,
                outcome,
                duration,
            },
            AuditTag::ScopeDenied {
                turn_id,
                tool_attempted,
                scope_requested,
                held_capabilities,
            } => AuditEvent::ScopeDenied {
                turn_id,
                tool_attempted,
                scope_requested,
                held_capabilities,
            },
            AuditTag::MemoryAccess {
                turn_id,
                operation,
                scope,
                query_or_key,
            } => AuditEvent::MemoryAccess {
                turn_id,
                operation: operation.into(),
                scope,
                query_or_key,
            },
            AuditTag::SkillInvocation {
                turn_id,
                session_id,
                skill_name,
            } => AuditEvent::SkillInvocation {
                turn_id,
                session_id,
                skill_name,
            },
        }
    }
}

/// Adapter that lets any `AuditWriter` satisfy `aivyx_core::AuditHook`.
///
/// Construct with [`AuditBridge::new`] for the D4-aligned panic-on-error
/// default, or with [`AuditBridge::with_error_handler`] to supply a custom
/// strategy (log, metric, soft-fail, etc.).
pub struct AuditBridge<W: AuditWriter> {
    writer: W,
    on_error: Box<dyn Fn(AuditError) + Send + Sync>,
}

impl<W: AuditWriter> AuditBridge<W> {
    /// Default bridge: panics on any `AuditError`. This matches D4's
    /// commitment that audit failures must be visible — a broken chain in
    /// a running agent is a misconfiguration, not something to log away.
    pub fn new(writer: W) -> Self {
        AuditBridge {
            writer,
            on_error: Box::new(|e| panic!("audit bridge: append failed: {e}")),
        }
    }

    /// Escape hatch for callers that need a non-panic strategy (e.g. an
    /// ops dashboard where logging the error and keeping the process up is
    /// preferable to crashing mid-turn). Use sparingly — every error that
    /// this handler swallows is an invariant from D4 that no longer holds.
    pub fn with_error_handler(
        writer: W,
        on_error: impl Fn(AuditError) + Send + Sync + 'static,
    ) -> Self {
        AuditBridge {
            writer,
            on_error: Box::new(on_error),
        }
    }

    /// Access the wrapped writer for verification / read-only queries.
    /// Used by tests that want to assert chain length or replay entries
    /// after a turn has run through the bridge.
    pub fn writer(&self) -> &W {
        &self.writer
    }
}

impl<W: AuditWriter + 'static> aivyx_core::AuditHook for AuditBridge<W> {
    fn on_event(&self, tag: aivyx_core::AuditTag) {
        let event: AuditEvent = tag.into();
        if let Err(e) = self.writer.append(event) {
            (self.on_error)(e);
        }
    }
}

// ---------------------------------------------------------------------------
// PersistentAuditLog — Phase 7 task 1 durable wrapper
// ---------------------------------------------------------------------------

mod persistent;
pub use persistent::PersistentAuditLog;

// ---------------------------------------------------------------------------
// Utility: input_hash helper for ToolCall events.
// ---------------------------------------------------------------------------

/// Hash a raw tool input (as bytes) into the 32-byte digest stored in
/// `AuditEvent::ToolCall::input_hash`. Provided here so every call site
/// uses the same hash family; the output is SHA-256.
pub fn hash_tool_input(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    let mut h = Sha256::new();
    h.update(bytes);
    let out = h.finalize();
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    arr
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_capability::{CapabilitySet, Scope, TrustTier};

    fn test_key() -> Vec<u8> {
        b"phase1-test-key-do-not-ship".to_vec()
    }

    fn sample_scope() -> Scope {
        Scope::parse("memory.read:session:abc").unwrap()
    }

    fn sample_capset() -> CapabilitySet {
        CapabilitySet::from_scopes([
            Scope::parse("memory.read").unwrap(),
            Scope::parse("llm.call").unwrap(),
        ])
    }

    fn sample_turn_started() -> AuditEvent {
        AuditEvent::TurnStarted {
            turn_id: TurnId::new(),
            session_id: SessionId::new(),
            channel: ChannelPlatform::Local,
            trust_tier: TrustTierSummary::from(TrustTier::Trusted),
            effective_capabilities: sample_capset(),
        }
    }

    fn sample_tool_call() -> AuditEvent {
        AuditEvent::ToolCall {
            turn_id: TurnId::new(),
            tool_id: ToolId::new(),
            scope_used: sample_scope(),
            input_hash: hash_tool_input(b"{\"query\":\"yesterday\"}"),
            outcome: ToolOutcomeSummary::Completed {
                verified: aivyx_core::VerificationSummary::NotApplicable,
            },
            duration: Duration::from_millis(37),
        }
    }

    // ---- Chain basics ----

    #[test]
    fn empty_chain_verifies() {
        let log = HmacChainLog::new(test_key());
        assert!(log.verify().is_ok());
        assert_eq!(AuditLog::len(&log), 0);
    }

    #[test]
    fn single_append_produces_seq_zero() {
        let log = HmacChainLog::new(test_key());
        let seq = log.append(sample_turn_started()).unwrap();
        assert_eq!(seq, 0);
        assert_eq!(AuditLog::len(&log), 1);
        log.verify().unwrap();
    }

    #[test]
    fn multi_append_produces_valid_chain() {
        let log = HmacChainLog::new(test_key());
        log.append(sample_turn_started()).unwrap();
        log.append(sample_tool_call()).unwrap();
        log.append(AuditEvent::TurnEnded {
            turn_id: TurnId::new(),
            outcome: TurnOutcomeSummary::Completed,
            tool_calls_made: 1,
            duration: Duration::from_secs(2),
            usage: TokenUsage::default(),
        })
        .unwrap();
        assert_eq!(AuditLog::len(&log), 3);
        log.verify().unwrap();
    }

    // ---- Phase 47 — entries_range ----

    #[test]
    fn entries_range_returns_requested_window() {
        let log = HmacChainLog::new(test_key());
        for _ in 0..5 {
            log.append(sample_tool_call()).unwrap();
        }
        // First two entries.
        let window = log.entries_range(0, 2).unwrap();
        assert_eq!(window.len(), 2);
        assert_eq!(window[0].seq, 0);
        assert_eq!(window[1].seq, 1);

        // Middle slice.
        let window = log.entries_range(2, 2).unwrap();
        assert_eq!(window.len(), 2);
        assert_eq!(window[0].seq, 2);
        assert_eq!(window[1].seq, 3);
    }

    #[test]
    fn entries_range_short_read_when_limit_exceeds_chain() {
        let log = HmacChainLog::new(test_key());
        log.append(sample_tool_call()).unwrap();
        log.append(sample_tool_call()).unwrap();
        // Asking for 10 from seq 1 should yield only 1.
        let window = log.entries_range(1, 10).unwrap();
        assert_eq!(window.len(), 1);
        assert_eq!(window[0].seq, 1);
    }

    #[test]
    fn entries_range_returns_empty_when_from_seq_past_end() {
        let log = HmacChainLog::new(test_key());
        log.append(sample_tool_call()).unwrap();
        let window = log.entries_range(5, 10).unwrap();
        assert!(window.is_empty());
    }

    #[test]
    fn entries_range_zero_limit_returns_empty() {
        let log = HmacChainLog::new(test_key());
        log.append(sample_tool_call()).unwrap();
        let window = log.entries_range(0, 0).unwrap();
        assert!(window.is_empty());
    }

    #[test]
    fn prev_mac_of_entry_n_matches_mac_of_entry_n_minus_1() {
        let log = HmacChainLog::new(test_key());
        log.append(sample_turn_started()).unwrap();
        log.append(sample_tool_call()).unwrap();
        let entries = log.entries().unwrap();
        assert_eq!(entries[1].prev_mac, entries[0].mac);
    }

    // ---- Tamper detection ----

    #[test]
    fn tampering_with_an_entry_breaks_chain() {
        let log = HmacChainLog::new(test_key());
        log.append(sample_turn_started()).unwrap();
        log.append(sample_tool_call()).unwrap();
        log.append(AuditEvent::TurnEnded {
            turn_id: TurnId::new(),
            outcome: TurnOutcomeSummary::Completed,
            tool_calls_made: 1,
            duration: Duration::from_secs(2),
            usage: TokenUsage::default(),
        })
        .unwrap();
        log.verify().unwrap();

        // Mutate entry 1's event in place. We reach into the Mutex for this
        // test only — real callers cannot do this because `entries()`
        // returns a clone.
        {
            let mut inner = log.inner.lock().unwrap();
            if let AuditEvent::ToolCall {
                ref mut duration, ..
            } = inner.entries[1].event
            {
                *duration = Duration::from_secs(999);
            } else {
                panic!("expected ToolCall at index 1");
            }
        }

        let err = log.verify().unwrap_err();
        match err {
            AuditError::ChainBroken { seq, .. } => {
                assert_eq!(seq, 1, "tamper on entry 1 must be detected at seq 1");
            }
            _ => panic!("expected ChainBroken, got {err:?}"),
        }
    }

    #[test]
    fn tampering_with_prev_mac_breaks_chain() {
        let log = HmacChainLog::new(test_key());
        log.append(sample_turn_started()).unwrap();
        log.append(sample_tool_call()).unwrap();

        {
            let mut inner = log.inner.lock().unwrap();
            inner.entries[1].prev_mac[0] ^= 0xFF;
        }

        let err = log.verify().unwrap_err();
        match err {
            AuditError::ChainBroken { seq, .. } => assert_eq!(seq, 1),
            _ => panic!("expected ChainBroken"),
        }
    }

    // ---- Canonical-bytes determinism across logs with the same key ----

    #[test]
    fn two_logs_same_key_same_events_produce_identical_macs() {
        // The "determinism that makes HMAC-chained audit useful" test:
        // build two separate logs from the same key, append the same
        // sequence of logically-equal events, and confirm the MAC chain
        // is identical byte-for-byte.
        let event_a = sample_turn_started();
        let event_b = match &event_a {
            AuditEvent::TurnStarted {
                turn_id,
                session_id,
                channel,
                trust_tier,
                effective_capabilities,
            } => AuditEvent::TurnStarted {
                turn_id: *turn_id,
                session_id: *session_id,
                channel: *channel,
                trust_tier: *trust_tier,
                effective_capabilities: effective_capabilities.clone(),
            },
            _ => unreachable!(),
        };

        let log1 = HmacChainLog::new(test_key());
        let log2 = HmacChainLog::new(test_key());
        log1.append(event_a).unwrap();
        log2.append(event_b).unwrap();

        let e1 = log1.entries().unwrap();
        let e2 = log2.entries().unwrap();
        assert_eq!(e1[0].mac, e2[0].mac, "same event + same key → same MAC");
        assert_eq!(e1[0].prev_mac, e2[0].prev_mac);
    }

    #[test]
    fn different_keys_produce_different_macs() {
        let log1 = HmacChainLog::new(b"key-one".to_vec());
        let log2 = HmacChainLog::new(b"key-two".to_vec());
        let ev = sample_turn_started();
        let ev2 = match &ev {
            AuditEvent::TurnStarted {
                turn_id,
                session_id,
                channel,
                trust_tier,
                effective_capabilities,
            } => AuditEvent::TurnStarted {
                turn_id: *turn_id,
                session_id: *session_id,
                channel: *channel,
                trust_tier: *trust_tier,
                effective_capabilities: effective_capabilities.clone(),
            },
            _ => unreachable!(),
        };
        log1.append(ev).unwrap();
        log2.append(ev2).unwrap();
        assert_ne!(
            log1.entries().unwrap()[0].mac,
            log2.entries().unwrap()[0].mac
        );
    }

    // ---- Round-trip all 5 variants ----

    #[test]
    fn all_five_variants_round_trip_through_canonical_json() {
        let turn_id = TurnId::new();

        let events = vec![
            sample_tool_call(),
            AuditEvent::ScopeDenied {
                turn_id,
                tool_attempted: ToolId::new(),
                scope_requested: Scope::parse("shell.exec:rm").unwrap(),
                held_capabilities: sample_capset(),
            },
            sample_turn_started(),
            AuditEvent::TurnEnded {
                turn_id,
                outcome: TurnOutcomeSummary::Cancelled,
                tool_calls_made: 2,
                duration: Duration::from_millis(500),
                usage: TokenUsage::default(),
            },
            AuditEvent::MemoryAccess {
                turn_id,
                operation: MemoryOperation::Read,
                scope: Scope::parse("memory.read:session:abc").unwrap(),
                query_or_key: "yesterday".to_string(),
            },
        ];

        for ev in events {
            let bytes = serde_jcs::to_vec(&ev).expect("jcs must accept");
            let back: AuditEvent = serde_json::from_slice(&bytes).expect("round trip");
            assert_eq!(ev, back);
        }
    }

    // ---- Phase 67 — AutoNotifyDispatched variant ----

    #[test]
    fn auto_notify_dispatched_round_trips_through_canonical_json() {
        let cases = vec![
            AuditEvent::AutoNotifyDispatched {
                session_id: SessionId::new(),
                trigger_kind: TriggerKindSummary::Cron,
                trigger_id: "morning-briefing".into(),
                target_name: "phone".into(),
                outcome: AutoNotifyOutcomeSummary::Delivered,
                dispatched_at_unix_ms: 1_715_000_000_000,
            },
            AuditEvent::AutoNotifyDispatched {
                session_id: SessionId::new(),
                trigger_kind: TriggerKindSummary::Webhook,
                trigger_id: "ci-events".into(),
                target_name: "ops-alerts".into(),
                outcome: AutoNotifyOutcomeSummary::SkippedEmptyResponse,
                dispatched_at_unix_ms: 1_715_000_000_001,
            },
            AuditEvent::AutoNotifyDispatched {
                session_id: SessionId::new(),
                trigger_kind: TriggerKindSummary::FileWatch,
                trigger_id: "notes-dir".into(),
                target_name: "phone".into(),
                outcome: AutoNotifyOutcomeSummary::Failed {
                    error_kind: "rejected".into(),
                    error_message: "HTTP 429".into(),
                },
                dispatched_at_unix_ms: 1_715_000_000_002,
            },
        ];

        for ev in cases {
            let bytes = serde_jcs::to_vec(&ev).expect("jcs serializes");
            let back: AuditEvent = serde_json::from_slice(&bytes).expect("round trip");
            assert_eq!(ev, back);
        }
    }

    #[test]
    fn auto_notify_outcome_summary_serializes_with_kind_tag() {
        // Sanity check: the #[serde(tag = "kind")] makes the
        // wire shape `{"kind": "Delivered"}` etc., which the
        // existing Web UI chain reader handles cleanly.
        let delivered = AutoNotifyOutcomeSummary::Delivered;
        let json = serde_json::to_value(&delivered).unwrap();
        assert_eq!(json["kind"], "Delivered");

        let failed = AutoNotifyOutcomeSummary::Failed {
            error_kind: "auth".into(),
            error_message: "HTTP 401".into(),
        };
        let json = serde_json::to_value(&failed).unwrap();
        assert_eq!(json["kind"], "Failed");
        assert_eq!(json["error_kind"], "auth");
        assert_eq!(json["error_message"], "HTTP 401");
    }

    #[test]
    fn trigger_kind_summary_serializes_with_kind_tag() {
        let cron = TriggerKindSummary::Cron;
        let json = serde_json::to_value(cron).unwrap();
        assert_eq!(json["kind"], "Cron");

        let webhook: TriggerKindSummary = serde_json::from_value(
            serde_json::json!({"kind": "Webhook"}),
        )
        .expect("Webhook variant parses");
        assert_eq!(webhook, TriggerKindSummary::Webhook);
    }

    // ---- Phase 117 — SkillInvocation variant ----

    #[test]
    fn skill_invocation_round_trips_through_canonical_json() {
        let ev = AuditEvent::SkillInvocation {
            turn_id: TurnId::new(),
            session_id: SessionId::new(),
            skill_name: "research-multi-source".into(),
        };
        let bytes = serde_jcs::to_vec(&ev).expect("jcs serializes");
        let back: AuditEvent =
            serde_json::from_slice(&bytes).expect("round trip");
        assert_eq!(ev, back);
    }

    #[test]
    fn skill_invocation_serializes_with_kind_tag() {
        let ev = AuditEvent::SkillInvocation {
            turn_id: TurnId::new(),
            session_id: SessionId::new(),
            skill_name: "x".into(),
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["kind"], "SkillInvocation");
        assert_eq!(json["skill_name"], "x");
    }

    #[test]
    fn skill_invocation_can_be_hmac_chained() {
        let log = HmacChainLog::new(test_key());
        log.append(sample_tool_call()).unwrap();
        log.append(AuditEvent::SkillInvocation {
            turn_id: TurnId::new(),
            session_id: SessionId::new(),
            skill_name: "research-topic".into(),
        })
        .unwrap();
        log.verify().unwrap();
        assert_eq!(AuditLog::len(&log), 2);
    }

    // ---- Phase 112 — SkillAutoProposal variant ----

    fn no_signals() -> HeuristicSignalsMatched {
        HeuristicSignalsMatched {
            tool_call_count: false,
            distinct_tool_id_count: false,
            duration: false,
            gate_resolve: false,
        }
    }

    fn all_signals() -> HeuristicSignalsMatched {
        HeuristicSignalsMatched {
            tool_call_count: true,
            distinct_tool_id_count: true,
            duration: true,
            gate_resolve: true,
        }
    }

    #[test]
    fn skill_auto_proposal_round_trips_for_every_outcome() {
        let cases = vec![
            AuditEvent::SkillAutoProposal {
                session_id: SessionId::new(),
                outcome: SkillAutoProposalOutcomeSummary::Disabled,
                confidence_thousandths: None,
                proposed_skill_name: None,
                judge_latency_ms: None,
                heuristic_signals_matched: no_signals(),
                category: None,
                source: None,
            },
            AuditEvent::SkillAutoProposal {
                session_id: SessionId::new(),
                outcome: SkillAutoProposalOutcomeSummary::HeuristicGated,
                confidence_thousandths: None,
                proposed_skill_name: None,
                judge_latency_ms: None,
                heuristic_signals_matched: no_signals(),
                category: None,
                source: None,
            },
            AuditEvent::SkillAutoProposal {
                session_id: SessionId::new(),
                outcome: SkillAutoProposalOutcomeSummary::AutoAccepted,
                confidence_thousandths: Some(910),
                proposed_skill_name: Some("research-topic".into()),
                judge_latency_ms: Some(1450),
                heuristic_signals_matched: all_signals(),
                category: None,
                source: None,
            },
            AuditEvent::SkillAutoProposal {
                session_id: SessionId::new(),
                outcome: SkillAutoProposalOutcomeSummary::Staged,
                confidence_thousandths: Some(720),
                proposed_skill_name: Some("research-topic".into()),
                judge_latency_ms: Some(1320),
                heuristic_signals_matched: HeuristicSignalsMatched {
                    tool_call_count: true,
                    distinct_tool_id_count: true,
                    duration: false,
                    gate_resolve: false,
                },
                category: None,
                source: None,
            },
            AuditEvent::SkillAutoProposal {
                session_id: SessionId::new(),
                outcome:
                    SkillAutoProposalOutcomeSummary::DuplicateOfExistingLlm {
                        duplicate_of: "summarize-pdf".into(),
                    },
                confidence_thousandths: Some(960),
                proposed_skill_name: None,
                judge_latency_ms: Some(1100),
                heuristic_signals_matched: all_signals(),
                category: None,
                source: None,
            },
            AuditEvent::SkillAutoProposal {
                session_id: SessionId::new(),
                outcome:
                    SkillAutoProposalOutcomeSummary::DuplicateOfExistingFuzzy {
                        matched_existing_name: "summarize-doc".into(),
                    },
                confidence_thousandths: Some(880),
                proposed_skill_name: Some("summarize-pdf".into()),
                judge_latency_ms: Some(1200),
                heuristic_signals_matched: all_signals(),
                category: None,
                source: None,
            },
            AuditEvent::SkillAutoProposal {
                session_id: SessionId::new(),
                outcome: SkillAutoProposalOutcomeSummary::NotWorthProposing,
                confidence_thousandths: Some(300),
                proposed_skill_name: None,
                judge_latency_ms: Some(900),
                heuristic_signals_matched: all_signals(),
                category: None,
                source: None,
            },
            AuditEvent::SkillAutoProposal {
                session_id: SessionId::new(),
                outcome: SkillAutoProposalOutcomeSummary::JudgeError {
                    error_message: "provider: HTTP 429".into(),
                },
                confidence_thousandths: None,
                proposed_skill_name: None,
                judge_latency_ms: Some(420),
                heuristic_signals_matched: all_signals(),
                category: None,
                source: None,
            },
        ];

        for ev in cases {
            let bytes = serde_jcs::to_vec(&ev).expect("jcs serializes");
            let back: AuditEvent =
                serde_json::from_slice(&bytes).expect("round trip");
            assert_eq!(ev, back);
        }
    }

    #[test]
    fn skill_auto_proposal_outcome_summary_serializes_with_kind_tag() {
        let auto = SkillAutoProposalOutcomeSummary::AutoAccepted;
        let json = serde_json::to_value(&auto).unwrap();
        assert_eq!(json["kind"], "AutoAccepted");

        let dup = SkillAutoProposalOutcomeSummary::DuplicateOfExistingFuzzy {
            matched_existing_name: "x".into(),
        };
        let json = serde_json::to_value(&dup).unwrap();
        assert_eq!(json["kind"], "DuplicateOfExistingFuzzy");
        assert_eq!(json["matched_existing_name"], "x");

        let err = SkillAutoProposalOutcomeSummary::JudgeError {
            error_message: "boom".into(),
        };
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(json["kind"], "JudgeError");
        assert_eq!(json["error_message"], "boom");
    }

    // ---- Phase 114 — backward-compatible `category` field ----

    #[test]
    fn skill_auto_proposal_with_category_round_trips() {
        // Phase 114: a SkillAutoProposal with a populated
        // category field round-trips through canonical JSON.
        let ev = AuditEvent::SkillAutoProposal {
            session_id: SessionId::new(),
            outcome: SkillAutoProposalOutcomeSummary::AutoAccepted,
            confidence_thousandths: Some(910),
            proposed_skill_name: Some("prefer terse replies".into()),
            judge_latency_ms: Some(1450),
            heuristic_signals_matched: all_signals(),
            category: Some("BehavioralPreferences".into()),
            source: None,
        };
        let bytes = serde_jcs::to_vec(&ev).expect("jcs serializes");
        let back: AuditEvent =
            serde_json::from_slice(&bytes).expect("round trip");
        assert_eq!(ev, back);
    }

    #[test]
    fn skill_auto_proposal_without_category_round_trips_byte_identically() {
        // Phase 114 backward-compatibility: a SkillAutoProposal
        // with category=None serializes to canonical JSON that
        // OMITS the field entirely (per `skip_serializing_if`),
        // so a pre-Phase-114 entry decoded into the new struct
        // and re-serialized produces the same bytes.
        let ev = AuditEvent::SkillAutoProposal {
            session_id: SessionId::new(),
            outcome: SkillAutoProposalOutcomeSummary::AutoAccepted,
            confidence_thousandths: Some(910),
            proposed_skill_name: Some("research-topic".into()),
            judge_latency_ms: Some(1450),
            heuristic_signals_matched: all_signals(),
            category: None,
            source: None,
        };
        let bytes = serde_jcs::to_vec(&ev).expect("jcs serializes");
        // The JSON output should NOT contain "category" when None.
        let s = std::str::from_utf8(&bytes).unwrap();
        assert!(
            !s.contains("\"category\""),
            "category=None must serialize as absent field: {s}"
        );
        let back: AuditEvent =
            serde_json::from_slice(&bytes).expect("round trip");
        assert_eq!(ev, back);
    }

    #[test]
    fn skill_auto_proposal_decodes_pre_phase_114_entry_with_no_category() {
        // Simulates a Phase 112-113 chain entry: the JSON has
        // no "category" field. Decoding into the Phase 114
        // struct must succeed with category=None.
        let raw_pre_114 = serde_json::json!({
            "kind": "SkillAutoProposal",
            "session_id": SessionId::new(),
            "outcome": {"kind": "AutoAccepted"},
            "confidence_thousandths": 910u32,
            "proposed_skill_name": "research-topic",
            "judge_latency_ms": 1450u64,
            "heuristic_signals_matched": {
                "tool_call_count": true,
                "distinct_tool_id_count": true,
                "duration": true,
                "gate_resolve": true,
            },
        });
        let decoded: AuditEvent =
            serde_json::from_value(raw_pre_114).expect("decode");
        match decoded {
            AuditEvent::SkillAutoProposal { category, .. } => {
                assert!(category.is_none());
            }
            _ => panic!("expected SkillAutoProposal"),
        }
    }

    // ---- Phase 115 — `source` field backward-compat ----

    #[test]
    fn skill_auto_proposal_with_failed_turn_source_round_trips() {
        let ev = AuditEvent::SkillAutoProposal {
            session_id: SessionId::new(),
            outcome: SkillAutoProposalOutcomeSummary::AutoAccepted,
            confidence_thousandths: Some(910),
            proposed_skill_name: Some("never run rm -rf".into()),
            judge_latency_ms: Some(1450),
            heuristic_signals_matched: all_signals(),
            category: Some("BehavioralConstraints".into()),
            source: Some(ProposalSourceSummary::FailedTurn {
                failure_kind: "failed".into(),
            }),
        };
        let bytes = serde_jcs::to_vec(&ev).expect("jcs serializes");
        let back: AuditEvent =
            serde_json::from_slice(&bytes).expect("round trip");
        assert_eq!(ev, back);
    }

    #[test]
    fn skill_auto_proposal_without_source_round_trips_byte_identically() {
        // Phase 115 backward-compatibility: source=None
        // serializes as ABSENT field, so a pre-Phase-115
        // entry decoded into the new struct and re-encoded
        // produces the same canonical bytes.
        let ev = AuditEvent::SkillAutoProposal {
            session_id: SessionId::new(),
            outcome: SkillAutoProposalOutcomeSummary::AutoAccepted,
            confidence_thousandths: Some(910),
            proposed_skill_name: Some("research-topic".into()),
            judge_latency_ms: Some(1450),
            heuristic_signals_matched: all_signals(),
            category: None,
            source: None,
        };
        let bytes = serde_jcs::to_vec(&ev).expect("jcs serializes");
        let s = std::str::from_utf8(&bytes).unwrap();
        assert!(
            !s.contains("\"source\""),
            "source=None must serialize as absent field: {s}"
        );
        let back: AuditEvent =
            serde_json::from_slice(&bytes).expect("round trip");
        assert_eq!(ev, back);
    }

    #[test]
    fn skill_auto_proposal_decodes_pre_phase_115_entry_with_no_source() {
        // Simulates a Phase 112-114 chain entry: the JSON
        // has no "source" field. Decoding into the Phase
        // 115 struct must succeed with source=None.
        let raw_pre_115 = serde_json::json!({
            "kind": "SkillAutoProposal",
            "session_id": SessionId::new(),
            "outcome": {"kind": "AutoAccepted"},
            "confidence_thousandths": 910u32,
            "proposed_skill_name": "research-topic",
            "judge_latency_ms": 1450u64,
            "heuristic_signals_matched": {
                "tool_call_count": true,
                "distinct_tool_id_count": true,
                "duration": true,
                "gate_resolve": true,
            },
            "category": "LearnedSkill",
        });
        let decoded: AuditEvent =
            serde_json::from_value(raw_pre_115).expect("decode");
        match decoded {
            AuditEvent::SkillAutoProposal { source, .. } => {
                assert!(source.is_none());
            }
            _ => panic!("expected SkillAutoProposal"),
        }
    }

    #[test]
    fn proposal_source_summary_serializes_with_kind_tag() {
        let completed = ProposalSourceSummary::CompletedTurn;
        let json = serde_json::to_value(&completed).unwrap();
        assert_eq!(json["kind"], "CompletedTurn");

        let failed = ProposalSourceSummary::FailedTurn {
            failure_kind: "timed_out".into(),
        };
        let json = serde_json::to_value(&failed).unwrap();
        assert_eq!(json["kind"], "FailedTurn");
        assert_eq!(json["failure_kind"], "timed_out");
    }

    #[test]
    fn skill_auto_proposal_can_be_hmac_chained() {
        // Same proof-of-life test the other variants have: an
        // entry of the new variant lands in the HmacChainLog
        // without breaking the chain verification.
        let log = HmacChainLog::new(test_key());
        log.append(sample_tool_call()).unwrap();
        log.append(AuditEvent::SkillAutoProposal {
            session_id: SessionId::new(),
            outcome: SkillAutoProposalOutcomeSummary::AutoAccepted,
            confidence_thousandths: Some(910),
            proposed_skill_name: Some("research-topic".into()),
            judge_latency_ms: Some(1450),
            heuristic_signals_matched: all_signals(),
            category: None,
            source: None,
        })
        .unwrap();
        log.verify().unwrap();
        assert_eq!(AuditLog::len(&log), 2);
    }

    // ---- NullAuditLog ----

    #[test]
    fn null_audit_log_accepts_and_verifies() {
        let null = NullAuditLog;
        null.append(sample_tool_call()).unwrap();
        assert_eq!(AuditLog::len(&null), 0);
        null.verify().unwrap();
    }

    // ---- D1 Scenario 3 audit trail: rm -rf denied on Tier 2 ----

    #[test]
    fn d1_scenario3_produces_denied_audit_trail() {
        // Walks the audit trail a denied Telegram rm -rf would emit:
        // TurnStarted → ScopeDenied → TurnEnded, all MAC-chained.
        let log = HmacChainLog::new(test_key());
        let turn_id = TurnId::new();
        let session_id = SessionId::new();

        let agent = CapabilitySet::from_scopes([Scope::parse("shell.exec").unwrap()]);
        let effective = agent.intersect(TrustTier::SemiTrusted.default_ceiling());

        log.append(AuditEvent::TurnStarted {
            turn_id,
            session_id,
            channel: ChannelPlatform::Telegram,
            trust_tier: TrustTierSummary::SemiTrusted,
            effective_capabilities: effective.clone(),
        })
        .unwrap();

        log.append(AuditEvent::ScopeDenied {
            turn_id,
            tool_attempted: ToolId::new(),
            scope_requested: Scope::parse("shell.exec:rm").unwrap(),
            held_capabilities: effective,
        })
        .unwrap();

        log.append(AuditEvent::TurnEnded {
            turn_id,
            outcome: TurnOutcomeSummary::Completed,
            tool_calls_made: 0,
            duration: Duration::from_millis(42),
            usage: TokenUsage::default(),
        })
        .unwrap();

        log.verify().unwrap();
        assert_eq!(AuditLog::len(&log), 3);

        // Spot-check the denial was captured with the right scope.
        match log.get(1).unwrap().event {
            AuditEvent::ScopeDenied {
                scope_requested, ..
            } => {
                assert_eq!(scope_requested.base(), "shell.exec");
                assert_eq!(scope_requested.qualifier(), Some("rm"));
            }
            other => panic!("expected ScopeDenied, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Bridge tests: aivyx_core::AuditHook <-> aivyx_audit::AuditWriter
    // -----------------------------------------------------------------------

    /// An `AuditWriter` that always fails. Used by the with_error_handler
    /// test to verify the handler path runs instead of panicking.
    struct AlwaysBroken;

    impl AuditWriter for AlwaysBroken {
        fn append(&self, _event: AuditEvent) -> Result<u64, AuditError> {
            Err(AuditError::ChainBroken {
                seq: 0,
                reason: "synthetic test failure".to_string(),
            })
        }
    }

    #[test]
    fn bridge_writes_tool_call_to_chain() {
        use aivyx_core::{AuditHook, AuditTag, ToolId};
        use std::time::Duration;

        let log = HmacChainLog::new(test_key());
        let bridge = AuditBridge::new(log);

        // Feed one ToolCall through the AuditHook surface.
        bridge.on_event(AuditTag::ToolCall {
            turn_id: TurnId::new(),
            tool_id: ToolId::new(),
            scope_used: sample_scope(),
            input_hash: [7u8; 32],
            outcome: ToolOutcomeSummary::Completed {
                verified: aivyx_core::VerificationSummary::NotApplicable,
            },
            duration: Duration::from_millis(3),
        });

        // Chain length went up, verification still holds.
        assert_eq!(bridge.writer().len(), 1);
        bridge.writer().verify().unwrap();

        // And the entry is actually a ToolCall with the right input_hash.
        match bridge.writer().get(0).unwrap().event {
            AuditEvent::ToolCall { input_hash, .. } => {
                assert_eq!(input_hash, [7u8; 32]);
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn bridge_translates_all_five_variants() {
        use aivyx_core::{AuditHook, AuditTag, TokenUsage, ToolId};
        use std::time::Duration;

        let bridge = AuditBridge::new(HmacChainLog::new(test_key()));
        let turn_id = TurnId::new();
        let session_id = SessionId::new();

        // One of each D4 variant — the bridge must translate every shape.
        bridge.on_event(AuditTag::TurnStarted {
            turn_id,
            session_id,
            channel: ChannelPlatform::Local,
            trust_tier: TrustTier::Trusted,
            effective_capabilities: sample_capset(),
        });
        bridge.on_event(AuditTag::ToolCall {
            turn_id,
            tool_id: ToolId::new(),
            scope_used: sample_scope(),
            input_hash: [1u8; 32],
            outcome: ToolOutcomeSummary::Completed {
                verified: aivyx_core::VerificationSummary::NotApplicable,
            },
            duration: Duration::from_millis(1),
        });
        bridge.on_event(AuditTag::ScopeDenied {
            turn_id,
            tool_attempted: ToolId::new(),
            scope_requested: Scope::parse("shell.exec:rm").unwrap(),
            held_capabilities: sample_capset(),
        });
        bridge.on_event(AuditTag::MemoryAccess {
            turn_id,
            operation: aivyx_core::MemoryOperation::Read,
            scope: sample_scope(),
            query_or_key: "yesterday".to_string(),
        });
        bridge.on_event(AuditTag::TurnEnded {
            turn_id,
            outcome: TurnOutcomeSummary::Completed,
            tool_calls_made: 1,
            duration: Duration::from_millis(5),
            usage: TokenUsage::default(),
        });

        // All five entries present, chain still verifies.
        assert_eq!(bridge.writer().len(), 5);
        bridge.writer().verify().unwrap();

        // Spot-check the TurnStarted translation picked the right tier
        // and the MemoryAccess translation picked the right op kind.
        match bridge.writer().get(0).unwrap().event {
            AuditEvent::TurnStarted { trust_tier, .. } => {
                assert_eq!(trust_tier, TrustTierSummary::Trusted);
            }
            other => panic!("expected TurnStarted at seq 0, got {other:?}"),
        }
        match bridge.writer().get(3).unwrap().event {
            AuditEvent::MemoryAccess { operation, .. } => {
                assert_eq!(operation, MemoryOperation::Read);
            }
            other => panic!("expected MemoryAccess at seq 3, got {other:?}"),
        }
    }

    #[test]
    fn bridge_with_handler_captures_error_instead_of_panicking() {
        use aivyx_core::{AuditHook, AuditTag, ToolId};
        use std::sync::{Arc, Mutex};
        use std::time::Duration;

        let captured: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let captured_clone = Arc::clone(&captured);

        let bridge = AuditBridge::with_error_handler(AlwaysBroken, move |e| {
            captured_clone.lock().unwrap().push(e.to_string());
        });

        // This would panic under the default bridge — with a handler it
        // routes to the closure instead.
        bridge.on_event(AuditTag::ToolCall {
            turn_id: TurnId::new(),
            tool_id: ToolId::new(),
            scope_used: sample_scope(),
            input_hash: [0u8; 32],
            outcome: ToolOutcomeSummary::Completed {
                verified: aivyx_core::VerificationSummary::NotApplicable,
            },
            duration: Duration::from_millis(1),
        });

        let errors = captured.lock().unwrap();
        assert_eq!(errors.len(), 1);
        assert!(
            errors[0].contains("chain verification failed"),
            "expected AuditError::ChainBroken message, got {:?}",
            errors[0]
        );
    }
}
