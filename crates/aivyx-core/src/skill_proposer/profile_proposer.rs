//! Phase 118 — Profile/Role refinement draft payload types.
//!
//! Two new draft shapes the auto-proposer can emit, on top of
//! the Phase 110/112/114 LearnedSkill + ListAppend + ScalarSet
//! shapes:
//!
//! - [`ProfileFieldHint`] — a noted suggestion that the
//!   operator-declared `[profile]` block in `aivyx.toml`
//!   could be refined for one of the six declared Profile-
//!   config fields. Operator-staged regardless of confidence;
//!   does NOT auto-mutate `aivyx.toml`.
//! - [`RoleDraft`] — a noted draft for an entirely new Role
//!   definition the agent observes would fit the operator's
//!   recurring task shapes. Operator-staged regardless of
//!   confidence; does NOT auto-mutate `aivyx.toml`.
//!
//! Both payloads ride inside the existing Phase 110 list-
//! category substrate as JSON-serialized strings on
//! `PersonaDeltaOp::AppendList { value }`. The wire shape is:
//!
//! ```json
//! {
//!   "category": "ProfileHint",
//!   "op": { "kind": "AppendList", "value": "<json blob>" }
//! }
//! ```
//!
//! Where `<json blob>` is the serialized [`ProfileFieldHint`]
//! or [`RoleDraft`] body. Renders parse the blob back into
//! structured form (LearnedSkill precedent).
//!
//! ## Q2(a) at Phase 118 sign-off — always-staged
//!
//! Neither category supports auto-accept. The acceptance-
//! posture override is enforced at the auto-proposer's
//! `decide_routing` stage (Phase 118 Task 5): regardless of
//! judge confidence vs the global threshold, ProfileHint and
//! RoleDefinitionSuggestion route to
//! `SkillAutoProposalOutcomeSummary::Staged`. The P13
//! Profile-operator-owned (Phase 56 amendment) and P9 Role-
//! config operator-curated (Phase 13) contracts stay intact.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Profile field naming
// ---------------------------------------------------------------------------

/// The six declared `[profile]` fields in `aivyx.toml`. Mirrors
/// the field names operators see and edit directly in the TOML
/// config, NOT the [`PersonaDeltaCategory`] enum
/// (`aivyx-channel::persona`) that names the Persona-chain
/// refinement categories.
///
/// Why a distinct enum: the Persona-chain categories include
/// Persona-specific facets (`LearnedContext`, `CharacterTraits`,
/// …) that have no counterpart in `[profile]`. A
/// `ProfileFieldHint` only ever targets one of these six
/// declared-Profile field names.
///
/// [`PersonaDeltaCategory`]: aivyx-channel::persona::PersonaDeltaCategory
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProfileField {
    /// Scalar — `profile.assistant_name` in `aivyx.toml`.
    AssistantName,
    /// Scalar — `profile.operator_profile` in `aivyx.toml`.
    OperatorProfile,
    /// Scalar — `profile.communication_style` in `aivyx.toml`.
    CommunicationStyle,
    /// List — `profile.primary_use_cases` in `aivyx.toml`.
    PrimaryUseCases,
    /// List — `profile.behavioral_preferences` in `aivyx.toml`.
    BehavioralPreferences,
    /// List — `profile.behavioral_constraints` in `aivyx.toml`.
    BehavioralConstraints,
}

impl ProfileField {
    /// Stable string label — matches the field name as it
    /// appears in `aivyx.toml`. Used in operator-facing
    /// messages and in the audit-event `category` field
    /// (Phase 114 wire-compat: stays a string in
    /// `SkillAutoProposal.category`).
    pub fn label(self) -> &'static str {
        match self {
            ProfileField::AssistantName => "assistant_name",
            ProfileField::OperatorProfile => "operator_profile",
            ProfileField::CommunicationStyle => "communication_style",
            ProfileField::PrimaryUseCases => "primary_use_cases",
            ProfileField::BehavioralPreferences => "behavioral_preferences",
            ProfileField::BehavioralConstraints => "behavioral_constraints",
        }
    }

    /// `true` for the three scalar profile fields; `false` for
    /// the three list-shaped fields. The judge prompt uses this
    /// to know whether the suggested value should be a single
    /// value (scalar) or one entry to append (list).
    pub fn is_scalar(self) -> bool {
        matches!(
            self,
            ProfileField::AssistantName
                | ProfileField::OperatorProfile
                | ProfileField::CommunicationStyle
        )
    }
}

// ---------------------------------------------------------------------------
// ProfileFieldHint — one staged Profile-config refinement
// ---------------------------------------------------------------------------

/// A noted suggestion that the operator-declared `[profile]`
/// block in `aivyx.toml` could be refined.
///
/// The auto-proposer LLM judge drafts one of these when it
/// observes recurring task shapes that suggest the declared
/// Profile-config is incomplete or mis-shaped. The hint is
/// staged in the Persona chain for operator review; on
/// approval, it lands in [`EffectivePersona::profile_hints`]
/// as a record-of-suggestion. The operator decides whether
/// to act on the hint by editing `aivyx.toml`.
///
/// For scalar fields (`is_scalar` true on the field), the
/// `suggested_value` is the proposed new value. For list
/// fields, the `suggested_value` is one entry to append to
/// the existing list.
///
/// [`EffectivePersona::profile_hints`]: aivyx-channel::persona::EffectivePersona::profile_hints
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileFieldHint {
    /// Which declared Profile-config field the hint targets.
    pub field: ProfileField,
    /// Suggested new value (scalar) or new list entry (list).
    pub suggested_value: String,
    /// Short rationale operator-readable in the proposal
    /// review surface. Budget: 1–3 sentences. The judge prompt
    /// (Phase 118 Task 4) instructs the LLM to err on the side
    /// of explicit rationales.
    pub rationale: String,
}

// ---------------------------------------------------------------------------
// RoleDraft — one staged new-Role suggestion
// ---------------------------------------------------------------------------

/// A noted draft for an entirely new Role definition.
///
/// Roles in `aivyx-config` (P9 — Per-Role Full Capability
/// Declaration, Phase 13) carry `system_prompt`,
/// `tool_allowlist`, an optional parent-chain `inherits_from`,
/// and other envelope fields. The auto-proposer LLM judge
/// drafts a `RoleDraft` when it observes recurring task shapes
/// the existing Role configuration doesn't fit well.
///
/// Phase 118 ships ONLY the proposer + chain entry — the
/// approved draft sits in the Persona chain for operator
/// review and optional copy into `[roles.*]` in `aivyx.toml`.
/// No auto-mutation of the role config; the P9 Role-config
/// operator-curated boundary stays intact.
///
/// `system_prompt_addendum` and `tool_allowlist_additions`
/// are framed as "what to ADD on top of the parent" rather
/// than "a full from-scratch role config" because Phase 13's
/// parent-chain inheritance primitive makes additive deltas
/// the natural shape. The operator can flatten the addendum
/// into a full role section when copying.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleDraft {
    /// Suggested role name — kebab-case slug, operator-readable.
    /// The operator can rename when copying into `[roles.*]`.
    pub name: String,
    /// Suggested parent role for inheritance. `None` means
    /// "no parent" (this would be a top-level role). The
    /// judge picks a parent from the existing role set when
    /// the recurring task shape extends an existing role; it
    /// leaves `None` when the task shape is genuinely
    /// orthogonal.
    pub parent: Option<String>,
    /// Suggested system_prompt addendum — the per-role prompt
    /// text the role would carry on top of any inherited
    /// prompt. Operator-readable markdown.
    pub system_prompt_addendum: String,
    /// Suggested tool_allowlist additions — names of tools
    /// the role would enable on top of any inherited
    /// allowlist. Empty when the addendum is purely a
    /// system_prompt refinement.
    pub tool_allowlist_additions: Vec<String>,
    /// Short rationale — what recurring shape justifies the
    /// new role. Budget: 1–3 sentences.
    pub rationale: String,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_field_labels_match_aivyx_toml_field_names() {
        // The labels are the operator-visible field names in
        // `aivyx.toml`. Drift would break the operator's
        // ability to map a hint to the field they need to edit.
        assert_eq!(ProfileField::AssistantName.label(), "assistant_name");
        assert_eq!(ProfileField::OperatorProfile.label(), "operator_profile");
        assert_eq!(
            ProfileField::CommunicationStyle.label(),
            "communication_style"
        );
        assert_eq!(ProfileField::PrimaryUseCases.label(), "primary_use_cases");
        assert_eq!(
            ProfileField::BehavioralPreferences.label(),
            "behavioral_preferences"
        );
        assert_eq!(
            ProfileField::BehavioralConstraints.label(),
            "behavioral_constraints"
        );
    }

    #[test]
    fn profile_field_is_scalar_matches_aivyx_config_shape() {
        // Mirrors the scalar/list split on
        // `aivyx_config::Profile` — drift would break the
        // judge prompt's "single value" vs "one append" guidance.
        assert!(ProfileField::AssistantName.is_scalar());
        assert!(ProfileField::OperatorProfile.is_scalar());
        assert!(ProfileField::CommunicationStyle.is_scalar());
        assert!(!ProfileField::PrimaryUseCases.is_scalar());
        assert!(!ProfileField::BehavioralPreferences.is_scalar());
        assert!(!ProfileField::BehavioralConstraints.is_scalar());
    }

    #[test]
    fn profile_field_hint_roundtrips_through_serde_json() {
        // Hint payload rides inside `AppendList { value }` as a
        // JSON-serialized string. Round-trip stability is the
        // contract the proposer + chain reader rely on.
        let original = ProfileFieldHint {
            field: ProfileField::CommunicationStyle,
            suggested_value: "terse and bullet-formatted".to_string(),
            rationale: "Operator consistently asks for shorter \
                        replies and uses bullet lists in their \
                        own messages."
                .to_string(),
        };
        let json = serde_json::to_string(&original).unwrap();
        let parsed: ProfileFieldHint = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn role_draft_roundtrips_through_serde_json() {
        let original = RoleDraft {
            name: "research-deploy".to_string(),
            parent: Some("research".to_string()),
            system_prompt_addendum: "When deploy artifacts are \
                                     ready, summarize the diff \
                                     and surface for approval."
                .to_string(),
            tool_allowlist_additions: vec!["git.commit".to_string(), "shell.deploy".to_string()],
            rationale: "Operator's 'research then deploy' \
                        pattern hit the existing research \
                        role's tool wall five times this week."
                .to_string(),
        };
        let json = serde_json::to_string(&original).unwrap();
        let parsed: RoleDraft = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn role_draft_with_no_parent_serializes_explicit_null() {
        // `parent: None` must round-trip cleanly. Phase 118
        // doesn't gate the wire shape on a serde-skip pattern
        // here; the field is always emitted (Option's default
        // wire shape) so both old and new readers see the
        // field explicitly.
        let draft = RoleDraft {
            name: "operator-mode".to_string(),
            parent: None,
            system_prompt_addendum: "Operator-direct mode.".to_string(),
            tool_allowlist_additions: vec![],
            rationale: "Top-level role distinct from anything \
                        existing."
                .to_string(),
        };
        let json = serde_json::to_string(&draft).unwrap();
        assert!(json.contains("\"parent\":null"));
        let parsed: RoleDraft = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.parent, None);
    }
}
