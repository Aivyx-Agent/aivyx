//! Persona wire types (Phase 59–118) — moved to `aivyx-ipc` in M.2c.
//!
//! The operator-approved persona model: the [`PersonaDelta`] (category + op)
//! that the chain records, the folded [`EffectivePersona`] runtime state, and
//! the [`ProposedPersonaDelta`] the agent suggests. These ride on the wire
//! (`GetEffectivePersona`, `ListPersonaProposals`, …) so they live in the
//! wasm-clean protocol crate; the HMAC **chain**, the persistent store, and the
//! fold/replay functions stay in `aivyx-channel`.

use serde::{Deserialize, Serialize};

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
    /// Phase 110 — Skills Auto-Creation. List of procedural patterns
    /// the agent drafts after complex turns and the operator
    /// approves. Each list entry's `value` is JSON-serialized
    /// [`LearnedSkill`] payload ({name, trigger, procedure}).
    /// Stays inside PRODUCT.md P8's outcome-driven audited
    /// reflection envelope; the agent never applies these
    /// autonomously, the operator approves through the same
    /// persona-proposal surface as every other delta. Q1(a) at
    /// Phase 110 sign-off — chosen over a new KeyDomain::Skills
    /// to reuse the entire Phase 59/60/70 substrate (chain log,
    /// proposal flow, revert primitive, operator review surface).
    LearnedSkill,

    /// Phase 118 — Outcome-driven Profile-config refinement
    /// HINT. List of operator-staged suggestions to refine the
    /// declared `[profile]` block in `aivyx.toml`. Each list
    /// entry's `value` is a JSON-serialized
    /// `aivyx_core::skill_proposer::ProfileFieldHint` payload
    /// ({field, suggested_value, rationale}).
    ///
    /// Distinct from the six Profile-mirror categories above
    /// (`AssistantName`, …, `BehavioralConstraints`): those are
    /// Persona-chain refinements layered ON TOP of the operator-
    /// declared Profile (P13 — Persona grows from Profile). A
    /// `ProfileHint` is a NOTED suggestion that the operator-
    /// declared Profile itself could be refined — the operator
    /// reviews the hint and decides whether to edit `aivyx.toml`.
    /// Phase 118 does **not** auto-mutate `aivyx.toml`; the
    /// hint stays in the Persona chain as a record-of-suggestion.
    ///
    /// Q2(a) at Phase 118 sign-off — **always-staged for
    /// operator approval**, no auto-accept regardless of judge
    /// confidence. The P13 Profile-is-operator-owned contract
    /// stays intact: the agent observes patterns and *suggests*;
    /// the operator decides whether to amend.
    ProfileHint,

    /// Phase 118 — Outcome-driven new-Role suggestion. List of
    /// operator-staged draft Role definitions the agent observes
    /// would fit the operator's recurring task shapes better
    /// than the existing Role configuration. Each list entry's
    /// `value` is a JSON-serialized
    /// `aivyx_core::skill_proposer::RoleDraft` payload
    /// ({name, parent, system_prompt_addendum,
    /// tool_allowlist_additions, rationale}).
    ///
    /// The first phase with an auto-proposer for Role drafts.
    /// Roles in `aivyx-config` carry `system_prompt`,
    /// `tool_allowlist`, parent-chain inheritance, etc. (P9 —
    /// Per-Role Full Capability Declaration, Phase 13). Phase
    /// 118 does **not** auto-mutate `aivyx.toml`; the draft
    /// stays in the Persona chain for operator review +
    /// optional copy into the role config.
    ///
    /// Q2(a) at Phase 118 sign-off — **always-staged for
    /// operator approval**, no auto-accept regardless of judge
    /// confidence. The P9 Role-config operator-curated boundary
    /// stays intact.
    RoleDefinitionSuggestion,
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
    /// Phase 60 — operator-initiated revert (P14 commit 4). The
    /// referenced `target_delta_id` must name a prior entry in the
    /// same chain. At fold time, the runtime applies the *inverse*
    /// of the target's op: `AppendList` → `RemoveList`,
    /// `RemoveList` → `AppendList`, `SetScalar { new }` →
    /// `SetScalar { prior_value_from_chain }`, `Revert` →
    /// re-apply the original target (revert-of-revert restores
    /// the original delta's effect). The chain stays append-only
    /// — reverts grow the chain; they don't mutate prior entries.
    ///
    /// The `category` field on the [`PersonaDelta`] carrying a
    /// `Revert` op must equal the target's category. Validation at
    /// append time enforces this so the chain is self-consistent.
    Revert { target_delta_id: String },
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
    /// / `RemoveList`. `Revert` is valid on any category — the
    /// constraint is shifted to apply-time (the target delta must
    /// exist and its category must match this delta's category).
    /// Returns a human-readable reason on failure.
    pub fn validate(&self) -> Result<(), String> {
        match (self.category.is_scalar(), &self.op) {
            // Revert is acceptable on any category at append-time;
            // chain-walking validation happens in the folder.
            (_, PersonaDeltaOp::Revert { .. }) => Ok(()),
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

/// Replay of the Persona chain into a structured runtime state. Used
/// by `assemble_session_prompt` (Phase 59 Task 6) to compose the
/// "How I have learned to communicate" section alongside Profile.
///
/// Scalars hold `Option<String>` because a `SetScalar { value: None }`
/// delta clears the field. Lists hold `Vec<String>` in insertion
/// order with duplicates removed (last-wins on
/// `AppendList`-after-`RemoveList`).
///
/// `Serialize`/`Deserialize` added in Phase 64 Task 2 so the identity
/// export format can embed the effective state directly (instead of
/// projecting through a parallel wire type).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Phase 110 — approved skill payloads, JSON-serialized
    /// [`LearnedSkill`] objects (one per list entry). The
    /// `assemble_session_prompt` renderer parses these back
    /// into structured form at render time so the agent sees
    /// `name: trigger` bullets in the `## Learned skills`
    /// section without dragging the full procedure text
    /// through every system prompt.
    pub learned_skills: Vec<String>,
    /// Phase 118 — approved Profile-config hint payloads,
    /// JSON-serialized
    /// `aivyx_core::skill_proposer::ProfileFieldHint` objects
    /// (one per list entry). These are operator-approved
    /// observations that the declared `[profile]` block could
    /// be refined; they do NOT auto-mutate `aivyx.toml`. The
    /// operator reviews + optionally copies into the config.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profile_hints: Vec<String>,
    /// Phase 118 — approved Role-draft payloads, JSON-
    /// serialized `aivyx_core::skill_proposer::RoleDraft`
    /// objects (one per list entry). Operator-approved Role
    /// definition drafts the agent has observed would fit
    /// recurring task patterns. Do NOT auto-mutate
    /// `aivyx.toml`; operator reviews + optionally copies
    /// the rendered shape into the role config.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub role_drafts: Vec<String>,
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
            || !self.learned_skills.is_empty()
            || !self.profile_hints.is_empty()
            || !self.role_drafts.is_empty()
    }
}

// ---------------------------------------------------------------------------
// LearnedSkill — Phase 110 schema for the procedural-pattern payload that
// rides inside `PersonaDeltaCategory::LearnedSkill + AppendList { value }`.
// ---------------------------------------------------------------------------

/// One operator-approved procedural pattern. The agent drafts
/// these after complex turns through `reflection.propose` with a
/// `LearnedSkill` delta; the operator approves through the
/// existing persona-proposal surface; the rendered system prompt
/// surfaces approved skills as `name: trigger` bullets in a
/// `## Learned skills` section (Phase 110 Task 5). The full
/// `procedure` text is available through the `skills.invoke`
/// tool (Phase 110 Task 4) on demand.
///
/// Stored inside `PersonaDeltaOp::AppendList { value }` as a
/// JSON-serialized string. The serialization stays inside the
/// existing list-category infrastructure rather than growing a
/// fourth `PersonaDeltaOp` variant; a future phase that wants
/// richer skill shapes can extend this struct without churning
/// the chain format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LearnedSkill {
    /// Short stable identifier — operator-facing in skill
    /// listings and `skills.invoke` lookups. Convention: kebab-
    /// case, dot-namespaced if useful (`code.review-checklist`).
    pub name: String,
    /// When this skill applies — the trigger description the
    /// agent reads on every turn to decide whether to follow
    /// the procedure. Short (one or two sentences).
    pub trigger: String,
    /// The skill's text — instructions, a tool sequence, an
    /// example, or any combination. Full text; the renderer
    /// elides this from the system prompt and reserves it for
    /// `skills.invoke` to avoid bloating every turn's prompt
    /// with every skill's full body.
    pub procedure: String,
}

impl LearnedSkill {
    /// Serialize for storage in `PersonaDeltaOp::AppendList`'s
    /// `value: String`.
    pub fn to_json_value(&self) -> String {
        serde_json::to_string(self).expect(
            "LearnedSkill serialization is infallible — all fields are owned Strings",
        )
    }

    /// Parse from a list-category entry. Returns `None` for
    /// malformed entries; the renderer skips malformed entries
    /// rather than failing the whole render.
    pub fn from_json_value(s: &str) -> Option<Self> {
        serde_json::from_str(s).ok()
    }
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
    /// Phase 92 — when this proposal is one half of a linked
    /// supersession pair (Phase 92 pattern-driven
    /// supersession), this field carries the OTHER half's
    /// `proposal_id`. The `AppendList`-side (the new facet)
    /// points at the `RemoveList`-side (the old facet); the
    /// `RemoveList`-side points back at the `AppendList`-side.
    /// `#[serde(default, skip_serializing_if = "Option::is_none")]`
    /// — full wire-compat (Phase 84 / Phase 91 precedent):
    /// `None` serializes without the field; old proposal-
    /// chain JSON decodes unchanged; the HMAC over the JCS
    /// bytes verifies against old entries because absent
    /// fields don't appear in the canonical bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes_proposal_id: Option<String>,
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


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_categories_reject_list_ops() {
        let scalar = PersonaDelta {
            delta_id: String::new(),
            proposed_at_unix_ms: 0,
            approved_at_unix_ms: 0,
            proposal_id: String::new(),
            category: PersonaDeltaCategory::AssistantName,
            op: PersonaDeltaOp::AppendList { value: "x".into() },
        };
        assert!(scalar.validate().is_err());
        assert!(PersonaDeltaCategory::AssistantName.is_scalar());
        assert!(!PersonaDeltaCategory::LearnedContext.is_scalar());
    }

    #[test]
    fn proposed_delta_validates_via_the_probe() {
        let ok = ProposedPersonaDelta {
            category: PersonaDeltaCategory::LearnedContext,
            op: PersonaDeltaOp::AppendList { value: "a fact".into() },
            reason: None,
            supersedes_proposal_id: None,
        };
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn learned_skill_round_trips_and_effective_persona_emptiness() {
        let s = LearnedSkill {
            name: "code.review".into(),
            trigger: "before merging".into(),
            procedure: "run the checklist".into(),
        };
        let back = LearnedSkill::from_json_value(&s.to_json_value()).unwrap();
        assert_eq!(back, s);

        let mut p = EffectivePersona::default();
        assert!(!p.is_non_empty());
        p.character_traits.push("dry wit".into());
        assert!(p.is_non_empty());
    }
}
