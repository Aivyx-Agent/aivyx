//! Phase 64 — Identity export/import format.
//!
//! Closes the Phase 60 deferral: operator-driven backup/transfer
//! of Profile + Persona chain across hosts. The chain's HMAC
//! key is per-host (derived from the operator's passphrase via
//! the storage layer), so chain MACs are not portable. Per
//! **Q1(a) at sign-off**, the import path re-signs each delta
//! with the target host's key — chain *content* is portable;
//! cryptographic provenance from the source host is not. Trust
//! on import comes from the operator's authority, not from
//! cross-host MAC verification.
//!
//! ## Format (schema_version = 1)
//!
//! Pretty-printed JSON, 0600 file permissions. Operator can
//! inspect with `jq`, diff between exports, or wrap with `age`
//! / `gpg` externally if they want at-rest encryption.
//!
//! ```json
//! {
//!   "schema_version": 1,
//!   "exported_at": "2026-05-13T14:30:00Z",
//!   "source_host": "laptop.local",
//!   "profile": { "assistant_name": "Mira", … },
//!   "persona": {
//!     "deltas": [ { "seq": 0, "delta": { … } }, … ],
//!     "effective_at_export": { … }
//!   }
//! }
//! ```
//!
//! ## Validation on parse
//!
//! - `schema_version == 1`.
//! - Delta seq is monotonic (0, 1, 2, …) with no gaps or dups.
//! - Each delta passes [`PersonaDelta::validate`].
//! - Replaying the deltas produces the recorded
//!   `effective_at_export` (catches operator hand-edits).

use serde::{Deserialize, Serialize};

// moved to the wasm-clean aivyx-ipc crate (Chapter M.2d-3); re-exported here.
pub use aivyx_ipc::insights::DeltaExport;

use aivyx_config::Profile;

use crate::persona::{
    compute_effective_persona, EffectivePersona, SignedPersonaEntry,
};

/// Current export schema version. Bump only when the format
/// changes incompatibly; add a migrator at parse time.
pub const IDENTITY_EXPORT_SCHEMA_VERSION: u32 = 1;

/// Top-level identity bundle. Bundles Profile + Persona as
/// "snapshot of who I am" per Q2(a) at sign-off.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityExport {
    pub schema_version: u32,
    /// RFC 3339 UTC timestamp of when the export was written.
    pub exported_at: String,
    /// Hostname of the source host, if available. Informational
    /// only — the import path doesn't use it for anything.
    /// `None` when `hostname` lookup fails.
    pub source_host: Option<String>,
    pub profile: ProfileExport,
    pub persona: PersonaExport,
}

/// Plain-values projection of [`Profile`]. Drops the
/// `Sourced<…>` provenance metadata since import gets to
/// declare its own source. `assistant_name` is unwrapped from
/// `Sourced<String>` to the bare string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileExport {
    pub assistant_name: String,
    pub operator_profile: Option<String>,
    pub communication_style: Option<String>,
    pub primary_use_cases: Vec<String>,
    pub behavioral_preferences: Vec<String>,
    pub behavioral_constraints: Vec<String>,
}

impl From<&Profile> for ProfileExport {
    fn from(p: &Profile) -> Self {
        Self {
            assistant_name: p.assistant_name.value.clone(),
            operator_profile: p.operator_profile.clone(),
            communication_style: p.communication_style.clone(),
            primary_use_cases: p.primary_use_cases.clone(),
            behavioral_preferences: p.behavioral_preferences.clone(),
            behavioral_constraints: p.behavioral_constraints.clone(),
        }
    }
}

/// Persona half of the export bundle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonaExport {
    /// Chain entries with MACs stripped per Q1(a) — the import
    /// path re-signs against the target host's HMAC key.
    /// `seq` is preserved so parse-time validation can detect
    /// gaps or duplicates.
    pub deltas: Vec<DeltaExport>,
    /// Snapshot of EffectivePersona at export time per Q5(a).
    /// Import-side sanity check: replaying `deltas` must
    /// reproduce this state, or the export is rejected as
    /// hand-edited or otherwise corrupted.
    pub effective_at_export: EffectivePersona,
}


impl From<&SignedPersonaEntry> for DeltaExport {
    fn from(entry: &SignedPersonaEntry) -> Self {
        Self {
            seq: entry.seq,
            delta: entry.delta.clone(),
        }
    }
}

/// Build an [`IdentityExport`] from in-memory Profile + chain
/// entries + effective state. Pure assembly; no I/O. The
/// `exported_at` field is set to the current time; `source_host`
/// is read from the OS `hostname` if available.
pub fn build(
    profile: &Profile,
    chain: &[SignedPersonaEntry],
    effective: &EffectivePersona,
) -> IdentityExport {
    IdentityExport {
        schema_version: IDENTITY_EXPORT_SCHEMA_VERSION,
        exported_at: chrono::Utc::now().to_rfc3339(),
        source_host: hostname(),
        profile: ProfileExport::from(profile),
        persona: PersonaExport {
            deltas: chain.iter().map(DeltaExport::from).collect(),
            effective_at_export: effective.clone(),
        },
    }
}

fn hostname() -> Option<String> {
    // libc::gethostname is the portable path; for now the
    // env-var fallback covers the common case without adding a
    // dep. The hostname is informational only.
    std::env::var("HOSTNAME").ok().or_else(|| {
        // Try `hostname` command as a last resort.
        std::process::Command::new("hostname")
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    })
}

// ---------------------------------------------------------------------------
// Parsing + validation (Task 4)
// ---------------------------------------------------------------------------

/// Failure modes for [`parse_and_validate`]. Each variant
/// carries enough context for the operator to fix the offending
/// export file or rebuild from a known-good source.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    #[error("JSON parse failed: {0}")]
    Json(String),

    #[error(
        "schema version mismatch: file has version {found}, this build supports \
         version {supported}"
    )]
    SchemaVersion { found: u32, supported: u32 },

    #[error("non-monotonic delta sequence at index {index}: expected seq {expected}, got {found}")]
    NonMonotonicSeq {
        index: usize,
        expected: u64,
        found: u64,
    },

    #[error("delta at index {index} (seq {seq}) failed validation: {reason}")]
    InvalidDelta {
        index: usize,
        seq: u64,
        reason: String,
    },

    #[error(
        "replayed effective state does not match the export's `effective_at_export` \
         snapshot — the JSON may have been hand-edited or corrupted"
    )]
    EffectiveMismatch,
}

/// Parse a JSON export and run every validation that doesn't
/// require touching the local storage layer. On success the
/// returned [`IdentityExport`] is structurally sound and the
/// chain is replay-self-consistent.
pub fn parse_and_validate(json: &str) -> Result<IdentityExport, ImportError> {
    let parsed: IdentityExport =
        serde_json::from_str(json).map_err(|e| ImportError::Json(e.to_string()))?;

    if parsed.schema_version != IDENTITY_EXPORT_SCHEMA_VERSION {
        return Err(ImportError::SchemaVersion {
            found: parsed.schema_version,
            supported: IDENTITY_EXPORT_SCHEMA_VERSION,
        });
    }

    // Monotonic seq check + per-delta validation.
    for (index, d) in parsed.persona.deltas.iter().enumerate() {
        let expected = index as u64;
        if d.seq != expected {
            return Err(ImportError::NonMonotonicSeq {
                index,
                expected,
                found: d.seq,
            });
        }
        d.delta.validate().map_err(|reason| ImportError::InvalidDelta {
            index,
            seq: d.seq,
            reason,
        })?;
    }

    // Replay-consistency check. Synthesize SignedPersonaEntry
    // values with placeholder MACs so we can reuse the existing
    // folder (which is the source-of-truth replay logic for
    // Revert resolution etc.). The folder doesn't inspect MACs;
    // they're verified on append at import time.
    let replay_entries: Vec<SignedPersonaEntry> = parsed
        .persona
        .deltas
        .iter()
        .map(|d| SignedPersonaEntry {
            seq: d.seq,
            delta: d.delta.clone(),
            prev_mac: [0u8; 32],
            mac: [0u8; 32],
        })
        .collect();
    let replayed = compute_effective_persona(&replay_entries);
    if replayed != parsed.persona.effective_at_export {
        return Err(ImportError::EffectiveMismatch);
    }

    Ok(parsed)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persona::{PersonaDelta, PersonaDeltaCategory, PersonaDeltaOp};

    fn sample_delta(seq_label: u64) -> PersonaDelta {
        PersonaDelta {
            delta_id: format!("d-{seq_label}"),
            proposed_at_unix_ms: 1_715_000_000_000,
            approved_at_unix_ms: 1_715_000_060_000,
            proposal_id: format!("m-{seq_label}"),
            category: PersonaDeltaCategory::AssistantName,
            op: PersonaDeltaOp::SetScalar {
                value: Some(format!("Name{seq_label}")),
            },
        }
    }

    fn sample_export() -> IdentityExport {
        let profile = Profile::default();
        let entries = vec![SignedPersonaEntry {
            seq: 0,
            delta: sample_delta(0),
            prev_mac: [0u8; 32],
            mac: [0u8; 32],
        }];
        let effective = compute_effective_persona(&entries);
        build(&profile, &entries, &effective)
    }

    #[test]
    fn export_round_trips_through_json() {
        let export = sample_export();
        let json = serde_json::to_string_pretty(&export).expect("serialize");
        let parsed: IdentityExport =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(export, parsed);
    }

    #[test]
    fn parse_and_validate_accepts_happy_path() {
        let export = sample_export();
        let json = serde_json::to_string_pretty(&export).unwrap();
        let parsed = parse_and_validate(&json).expect("must parse");
        assert_eq!(parsed.schema_version, IDENTITY_EXPORT_SCHEMA_VERSION);
        assert_eq!(parsed.persona.deltas.len(), 1);
    }

    #[test]
    fn parse_rejects_invalid_json() {
        let err = parse_and_validate("not json").expect_err("must error");
        assert!(matches!(err, ImportError::Json(_)));
    }

    #[test]
    fn parse_rejects_schema_version_mismatch() {
        let json = r#"{
            "schema_version": 99,
            "exported_at": "2026-05-13T00:00:00Z",
            "source_host": null,
            "profile": {
                "assistant_name": "Aivyx",
                "operator_profile": null,
                "communication_style": null,
                "primary_use_cases": [],
                "behavioral_preferences": [],
                "behavioral_constraints": []
            },
            "persona": {
                "deltas": [],
                "effective_at_export": {
                    "assistant_name": null,
                    "operator_profile": null,
                    "communication_style": null,
                    "primary_use_cases": [],
                    "behavioral_preferences": [],
                    "behavioral_constraints": [],
                    "learned_context": [],
                    "communication_adaptations": [],
                    "character_traits": [],
                    "relationship_milestones": [],
                    "learned_skills": []
                }
            }
        }"#;
        let err = parse_and_validate(json).expect_err("must error");
        match err {
            ImportError::SchemaVersion { found, supported } => {
                assert_eq!(found, 99);
                assert_eq!(supported, 1);
            }
            other => panic!("expected SchemaVersion, got {other:?}"),
        }
    }

    #[test]
    fn parse_rejects_non_monotonic_seq() {
        // Build an export with a seq-gap in the deltas.
        let profile = Profile::default();
        let entries = vec![
            SignedPersonaEntry {
                seq: 0,
                delta: sample_delta(0),
                prev_mac: [0u8; 32],
                mac: [0u8; 32],
            },
            // Deliberately wrong seq (5 instead of 1).
            SignedPersonaEntry {
                seq: 5,
                delta: sample_delta(1),
                prev_mac: [0u8; 32],
                mac: [0u8; 32],
            },
        ];
        let effective = compute_effective_persona(&entries);
        let export = build(&profile, &entries, &effective);
        let json = serde_json::to_string(&export).unwrap();
        let err = parse_and_validate(&json).expect_err("must error");
        match err {
            ImportError::NonMonotonicSeq {
                index,
                expected,
                found,
            } => {
                assert_eq!(index, 1);
                assert_eq!(expected, 1);
                assert_eq!(found, 5);
            }
            other => panic!("expected NonMonotonicSeq, got {other:?}"),
        }
    }

    #[test]
    fn parse_rejects_effective_mismatch() {
        // Hand-edit the effective_at_export to disagree with
        // the deltas. Replay should catch this.
        let mut export = sample_export();
        export.persona.effective_at_export.assistant_name =
            Some("HandEdited".to_string());
        let json = serde_json::to_string(&export).unwrap();
        let err = parse_and_validate(&json).expect_err("must error");
        assert!(matches!(err, ImportError::EffectiveMismatch));
    }

    #[test]
    fn profile_export_drops_sourced_metadata() {
        let p = Profile::default();
        let pe = ProfileExport::from(&p);
        // assistant_name unwrapped from Sourced<String>.
        assert_eq!(pe.assistant_name, p.assistant_name.value);
    }

    #[test]
    fn delta_export_strips_macs() {
        let entry = SignedPersonaEntry {
            seq: 7,
            delta: sample_delta(7),
            prev_mac: [42u8; 32],
            mac: [99u8; 32],
        };
        let de = DeltaExport::from(&entry);
        assert_eq!(de.seq, 7);
        assert_eq!(de.delta, entry.delta);
        // No mac/prev_mac fields on DeltaExport — compile-time
        // check via JSON serialization.
        let json = serde_json::to_string(&de).unwrap();
        assert!(!json.contains("mac"));
        assert!(!json.contains("prev_mac"));
    }

    #[test]
    fn schema_version_constant_is_one() {
        assert_eq!(IDENTITY_EXPORT_SCHEMA_VERSION, 1);
    }
}
