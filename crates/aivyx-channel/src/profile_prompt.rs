//! Profile-into-system-prompt assembly. Phase 57 Task 3 —
//! PRODUCT.md P13 (Assistant Profile).
//!
//! [`assemble_session_prompt`] composes the operator-declared
//! [`Profile`] alongside the active role's `system_prompt` into the
//! final system prompt the LLM sees. Per Q3(c) at Phase 57 sign-off,
//! the composition is **labeled** — not concatenated:
//!
//! ```text
//! ## About this assistant
//!
//! Your name is <name>.
//! About the operator: <...>
//! Communication style: <...>
//! Primary use cases:
//! - ...
//! Behavioral preferences:
//! - ...
//! Behavioral constraints:
//! - ...
//!
//! ## Active role: <role-name>
//!
//! <role.system_prompt>
//! ```
//!
//! Labels matter for two reasons:
//!
//! 1. **LLM interpretability.** The model sees a layered identity
//!    structure (Profile → role) rather than one undifferentiated
//!    paragraph.
//! 2. **Phase 60 insertion point.** When Persona (P14) lands, its
//!    delta-derived voice section inserts between the Profile
//!    section and the active-role section, with its own label. The
//!    label-shaped layout makes that insertion structural rather
//!    than a string-rewrite.
//!
//! **Non-invasive on legacy configs.** When [`Profile::is_operator_declared`]
//! returns `false` — i.e. no `[profile]` section in `aivyx.toml`,
//! only the synthesized default with `assistant_name = "Aivyx"` —
//! the helper returns the role's `system_prompt` unchanged. No
//! "Your name is Aivyx" noise prepended to every default config.

use aivyx_config::Profile;

use crate::persona::EffectivePersona;

/// Compose the final system prompt for one turn by layering Profile
/// (operator-declared identity per PRODUCT.md P13), Persona
/// (reflection-written identity per PRODUCT.md P14), and the active
/// role's `system_prompt` (per-role voice override per P9).
///
/// **Behavior:**
/// - If neither Profile nor Persona has content (Profile at the
///   synthesized default AND Persona empty / `None`), return
///   `role_system_prompt.to_string()` unchanged. Zero behavior
///   change for pre-Phase-57 configs.
/// - Otherwise return a labeled composition:
///   1. "## About this assistant" — Profile fields (omitted if
///      Profile is at the synthesized default).
///   2. "## How I have learned to communicate" — Persona fields
///      (omitted if Persona is empty / `None`). Phase 59 Q6(a).
///   3. "## Active role: <role_name>" — the role's prompt.
///
/// `role_name` is rendered into the third section's label so the
/// LLM (and a debugging operator reading prompt logs) can see which
/// role is active.
///
/// The helper allocates a fresh `String` per call. The allocation is
/// dwarfed by the per-turn LLM round-trip cost, so micro-optimizing
/// is not worth the trade against composition clarity.
pub fn assemble_session_prompt(
    profile: &Profile,
    persona: Option<&EffectivePersona>,
    role_name: &str,
    role_system_prompt: &str,
) -> String {
    let profile_active = profile.is_operator_declared();
    let persona_active = persona.map(|p| p.is_non_empty()).unwrap_or(false);

    if !profile_active && !persona_active {
        return role_system_prompt.to_string();
    }

    let mut out = String::new();
    if profile_active {
        out.push_str(&render_profile_section(profile));
        out.push_str("\n\n");
    }
    if persona_active {
        // Safe to unwrap — `persona_active` requires Some.
        out.push_str(&render_persona_section(persona.unwrap()));
        out.push_str("\n\n");
    }
    out.push_str(&format!(
        "## Active role: {role_name}\n\n{role_system_prompt}"
    ));
    out
}

// ---------------------------------------------------------------------------
// Phase 79 — adaptive Persona: reduced-Persona assembly
// ---------------------------------------------------------------------------

/// Number of *reducible* (soft) Persona list entries. The
/// adaptive refiner uses this for its size-threshold fallback
/// (Q3a): below the threshold there is nothing worth selecting
/// over, so the full Persona is injected unchanged.
///
/// Excludes the protected fields — the scalar identity and
/// `behavioral_constraints` are never reduced, so they never
/// count toward "is the Soul big enough to bound."
pub fn reducible_facet_count(p: &EffectivePersona) -> usize {
    p.primary_use_cases.len()
        + p.behavioral_preferences.len()
        + p.learned_context.len()
        + p.communication_adaptations.len()
        + p.character_traits.len()
        + p.relationship_milestones.len()
}

/// Build a reduced [`EffectivePersona`] keeping only the soft
/// list entries for which `keep` returns `true`.
///
/// **Core invariant (Phase 79 Q2a), structurally enforced
/// here so no caller can violate it:** the scalar identity
/// (`assistant_name`, `operator_profile`, `communication_style`)
/// and `behavioral_constraints` are copied through **in full,
/// unconditionally** — `keep` is *only* ever applied to the six
/// soft list categories. Identity and guardrails are
/// non-negotiable and can never be selected away, regardless of
/// what the selector decides.
pub fn reduce_persona(
    full: &EffectivePersona,
    keep: &dyn Fn(&str) -> bool,
) -> EffectivePersona {
    let filter = |v: &[String]| -> Vec<String> {
        v.iter().filter(|s| keep(s)).cloned().collect()
    };
    EffectivePersona {
        // --- protected: always copied in full (the invariant) ---
        assistant_name: full.assistant_name.clone(),
        operator_profile: full.operator_profile.clone(),
        communication_style: full.communication_style.clone(),
        behavioral_constraints: full.behavioral_constraints.clone(),
        // --- reducible soft list categories ---
        primary_use_cases: filter(&full.primary_use_cases),
        behavioral_preferences: filter(&full.behavioral_preferences),
        learned_context: filter(&full.learned_context),
        communication_adaptations: filter(
            &full.communication_adaptations,
        ),
        character_traits: filter(&full.character_traits),
        relationship_milestones: filter(&full.relationship_milestones),
    }
}

/// Assemble the turn's system prompt with only the
/// contextually-selected Persona facets. Thin wrapper:
/// [`reduce_persona`] (invariant enforced) → the **unchanged**
/// [`assemble_session_prompt`]. Used by the Phase 79 refiner;
/// kept here so the reduction and the invariant are tested in
/// one place.
pub fn assemble_session_prompt_selected(
    profile: &Profile,
    full_persona: &EffectivePersona,
    keep: &dyn Fn(&str) -> bool,
    role_name: &str,
    role_system_prompt: &str,
) -> String {
    let reduced = reduce_persona(full_persona, keep);
    assemble_session_prompt(
        profile,
        Some(&reduced),
        role_name,
        role_system_prompt,
    )
}

fn render_profile_section(profile: &Profile) -> String {
    let mut out = String::from("## About this assistant\n\n");
    out.push_str(&format!(
        "Your name is {name}.\n",
        name = profile.assistant_name.value,
    ));
    if let Some(op) = &profile.operator_profile {
        out.push_str(&format!("\nAbout the operator: {op}\n"));
    }
    if let Some(style) = &profile.communication_style {
        out.push_str(&format!("\nCommunication style: {style}\n"));
    }
    if !profile.primary_use_cases.is_empty() {
        out.push_str("\nPrimary use cases:\n");
        for use_case in &profile.primary_use_cases {
            out.push_str(&format!("- {use_case}\n"));
        }
    }
    if !profile.behavioral_preferences.is_empty() {
        out.push_str("\nBehavioral preferences:\n");
        for pref in &profile.behavioral_preferences {
            out.push_str(&format!("- {pref}\n"));
        }
    }
    if !profile.behavioral_constraints.is_empty() {
        out.push_str("\nBehavioral constraints:\n");
        for c in &profile.behavioral_constraints {
            out.push_str(&format!("- {c}\n"));
        }
    }
    // Trim the trailing newline so the `\n\n## Active role` join
    // produces exactly one blank line between sections, not two.
    out.trim_end().to_string()
}

/// Render the Persona section. Phase 59 Q6(a): a labeled
/// "## How I have learned to communicate" block whose body
/// enumerates whichever Persona categories carry content.
///
/// Persona's scalar categories (assistant_name,
/// operator_profile, communication_style) intentionally render
/// *underneath* the Persona header rather than overriding the
/// Profile section's analogous fields — Persona refinements are
/// learned, not declared, and operators reading the prompt should
/// see them in the learned-section so they can tell what the
/// agent has decided versus what they originally declared.
fn render_persona_section(persona: &EffectivePersona) -> String {
    let mut out = String::from("## How I have learned to communicate\n\n");

    // Scalars are emitted as `Refined X: <value>` lines so the
    // operator reading the prompt understands the field was
    // refined by reflection, not declared by them.
    if let Some(name) = &persona.assistant_name {
        out.push_str(&format!("Refined name: {name}\n"));
    }
    if let Some(op) = &persona.operator_profile {
        out.push_str(&format!("Refined operator profile: {op}\n"));
    }
    if let Some(style) = &persona.communication_style {
        out.push_str(&format!("Refined communication style: {style}\n"));
    }

    if !persona.primary_use_cases.is_empty() {
        out.push_str("\nLearned use cases:\n");
        for case in &persona.primary_use_cases {
            out.push_str(&format!("- {case}\n"));
        }
    }
    if !persona.behavioral_preferences.is_empty() {
        out.push_str("\nLearned behavioral preferences:\n");
        for pref in &persona.behavioral_preferences {
            out.push_str(&format!("- {pref}\n"));
        }
    }
    if !persona.behavioral_constraints.is_empty() {
        out.push_str("\nLearned behavioral constraints:\n");
        for c in &persona.behavioral_constraints {
            out.push_str(&format!("- {c}\n"));
        }
    }
    if !persona.learned_context.is_empty() {
        out.push_str("\nLearned context:\n");
        for c in &persona.learned_context {
            out.push_str(&format!("- {c}\n"));
        }
    }
    if !persona.communication_adaptations.is_empty() {
        out.push_str("\nCommunication adaptations:\n");
        for c in &persona.communication_adaptations {
            out.push_str(&format!("- {c}\n"));
        }
    }
    if !persona.character_traits.is_empty() {
        out.push_str("\nCharacter traits:\n");
        for c in &persona.character_traits {
            out.push_str(&format!("- {c}\n"));
        }
    }
    if !persona.relationship_milestones.is_empty() {
        out.push_str("\nRelationship milestones:\n");
        for c in &persona.relationship_milestones {
            out.push_str(&format!("- {c}\n"));
        }
    }
    out.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_config::{FieldSource, Sourced, DEFAULT_ASSISTANT_NAME};

    fn default_profile() -> Profile {
        Profile::default()
    }

    fn operator_declared_profile() -> Profile {
        Profile {
            assistant_name: Sourced::new("Codex".to_string(), FieldSource::Toml),
            operator_profile: Some(
                "Senior Rust engineer focused on systems.".to_string(),
            ),
            communication_style: Some("terse, conclusion-first".to_string()),
            primary_use_cases: vec!["Rust systems programming".to_string()],
            behavioral_preferences: vec!["prefer integration tests".to_string()],
            behavioral_constraints: vec!["never auto-commit code".to_string()],
        }
    }

    #[test]
    fn default_profile_returns_role_prompt_unchanged() {
        let profile = default_profile();
        let role_prompt = "You are a coding assistant.";
        let assembled = assemble_session_prompt(&profile, None, "default", role_prompt);
        assert_eq!(assembled, role_prompt);
    }

    #[test]
    fn default_profile_with_empty_role_prompt_returns_empty_string() {
        let profile = default_profile();
        let assembled = assemble_session_prompt(&profile, None, "default", "");
        assert_eq!(assembled, "");
    }

    #[test]
    fn operator_declared_profile_produces_labeled_composition() {
        let profile = operator_declared_profile();
        let role_prompt = "You are a coding assistant.";
        let assembled = assemble_session_prompt(&profile, None, "coder", role_prompt);

        // Profile section appears first.
        assert!(assembled.starts_with("## About this assistant\n\n"));
        // Assistant name renders inside Profile section.
        assert!(assembled.contains("Your name is Codex."));
        // Operator profile renders.
        assert!(assembled.contains("About the operator: Senior Rust"));
        // Communication style renders.
        assert!(assembled.contains("Communication style: terse"));
        // Primary use cases render as bullets.
        assert!(assembled.contains("Primary use cases:\n- Rust systems programming"));
        // Behavioral preferences render as bullets.
        assert!(assembled.contains("Behavioral preferences:\n- prefer integration tests"));
        // Behavioral constraints render as bullets.
        assert!(assembled.contains("Behavioral constraints:\n- never auto-commit code"));
        // Active role section appears after Profile.
        assert!(assembled.contains("\n\n## Active role: coder\n\n"));
        // Role's system_prompt appears at the end.
        assert!(assembled.ends_with("You are a coding assistant."));
    }

    #[test]
    fn operator_declared_profile_with_empty_role_prompt_still_renders_active_role_label() {
        let profile = operator_declared_profile();
        let assembled = assemble_session_prompt(&profile, None, "coder", "");
        // The Active role label is still rendered even when the
        // role's system_prompt is empty — gives the LLM structural
        // cue that the role itself has no per-role voice override.
        assert!(assembled.contains("## Active role: coder"));
    }

    #[test]
    fn profile_with_only_assistant_name_overridden_still_renders() {
        // Operator overrides assistant_name but leaves everything
        // else empty. is_operator_declared() must return true
        // because assistant_name's source is now Toml.
        let profile = Profile {
            assistant_name: Sourced::new("Mira".to_string(), FieldSource::Toml),
            ..Profile::default()
        };

        assert!(profile.is_operator_declared());

        let assembled = assemble_session_prompt(&profile, None, "default", "You are helpful.");
        assert!(assembled.contains("Your name is Mira."));
        // None of the optional sections render.
        assert!(!assembled.contains("About the operator"));
        assert!(!assembled.contains("Communication style"));
        assert!(!assembled.contains("Primary use cases"));
        assert!(!assembled.contains("Behavioral preferences"));
        assert!(!assembled.contains("Behavioral constraints"));
        // Role's system_prompt still appears.
        assert!(assembled.ends_with("You are helpful."));
    }

    #[test]
    fn same_profile_yields_same_profile_section_across_roles() {
        // Phase 57 Task 3 property: a role-switch child session
        // sees the same Profile section as its parent. The
        // "## Active role: <name>" label differs, but everything
        // above that line is identical. Validates the structural
        // invariant the binary's parent path and role-switch
        // factory both rely on.
        let profile = operator_declared_profile();
        let parent = assemble_session_prompt(&profile, None, "default", "Parent prompt.");
        let child = assemble_session_prompt(&profile, None, "junior_researcher", "Child prompt.");

        // The Profile section (everything before "## Active role")
        // must be byte-identical across the two assemblies.
        let parent_profile = parent.split("\n\n## Active role:").next().unwrap();
        let child_profile = child.split("\n\n## Active role:").next().unwrap();
        assert_eq!(parent_profile, child_profile);

        // Active-role labels differ.
        assert!(parent.contains("## Active role: default"));
        assert!(child.contains("## Active role: junior_researcher"));

        // Each section ends with its own role prompt.
        assert!(parent.ends_with("Parent prompt."));
        assert!(child.ends_with("Child prompt."));
    }

    #[test]
    fn default_assistant_name_is_aivyx() {
        // Sanity guard for DEFAULT_ASSISTANT_NAME wiring — the
        // assemble helper relies on the synthesized default name
        // for is_operator_declared() short-circuiting.
        let p = Profile::default();
        assert_eq!(p.assistant_name.value, DEFAULT_ASSISTANT_NAME);
        assert_eq!(p.assistant_name.source, FieldSource::Default);
        assert!(!p.is_operator_declared());
    }

    // -------------------------------------------------------------
    // Phase 59 Task 6 — Persona section integration.
    // -------------------------------------------------------------

    fn operator_declared_persona() -> EffectivePersona {
        EffectivePersona {
            assistant_name: None,
            operator_profile: None,
            communication_style: Some(
                "refined: terse, no preamble, ASCII-only".to_string(),
            ),
            primary_use_cases: vec![],
            behavioral_preferences: vec!["always cite sources".to_string()],
            behavioral_constraints: vec![],
            learned_context: vec!["operator uses Vim".to_string()],
            communication_adaptations: vec![
                "operator prefers conclusion-first paragraphs".to_string(),
            ],
            character_traits: vec![],
            relationship_milestones: vec![],
        }
    }

    #[test]
    fn empty_persona_with_default_profile_returns_role_prompt_unchanged() {
        let profile = default_profile();
        let persona = EffectivePersona::default();
        let assembled = assemble_session_prompt(
            &profile,
            Some(&persona),
            "default",
            "You are helpful.",
        );
        assert_eq!(assembled, "You are helpful.");
    }

    #[test]
    fn persona_section_renders_under_labeled_header() {
        let profile = default_profile();
        let persona = operator_declared_persona();
        let assembled = assemble_session_prompt(
            &profile,
            Some(&persona),
            "coder",
            "You are a coding assistant.",
        );

        // Persona section appears (no Profile section since profile is default).
        assert!(assembled.starts_with("## How I have learned to communicate"));
        // Refined scalars render as "Refined ..." lines.
        assert!(assembled.contains("Refined communication style: refined: terse"));
        // List entries render as bullets under category-specific headers.
        assert!(assembled.contains("Learned behavioral preferences:\n- always cite sources"));
        assert!(assembled.contains("Learned context:\n- operator uses Vim"));
        assert!(assembled.contains("Communication adaptations:\n- operator prefers"));
        // Active role section follows.
        assert!(assembled.contains("\n\n## Active role: coder\n\n"));
        assert!(assembled.ends_with("You are a coding assistant."));
    }

    #[test]
    fn profile_and_persona_compose_three_section_layout() {
        // Profile present + Persona present + role prompt → all
        // three sections render in the Q6(a) order.
        let profile = operator_declared_profile();
        let persona = operator_declared_persona();
        let assembled = assemble_session_prompt(
            &profile,
            Some(&persona),
            "coder",
            "You are a coding assistant.",
        );

        // Profile section first.
        assert!(assembled.starts_with("## About this assistant"));
        // Persona section second.
        let profile_end = assembled.find("## How I have learned to communicate").unwrap();
        let role_start = assembled.find("## Active role: coder").unwrap();
        assert!(profile_end < role_start);
        // Role section last.
        assert!(assembled.ends_with("You are a coding assistant."));
    }

    #[test]
    fn persona_none_is_equivalent_to_empty_persona_when_profile_empty() {
        // Passing None for persona must behave identically to
        // passing Some(&EffectivePersona::default()) — both mean
        // "no Persona content, render passthrough."
        let profile = default_profile();
        let with_none = assemble_session_prompt(&profile, None, "default", "hi");
        let with_empty = assemble_session_prompt(
            &profile,
            Some(&EffectivePersona::default()),
            "default",
            "hi",
        );
        assert_eq!(with_none, with_empty);
        assert_eq!(with_none, "hi");
    }

    // ---- Phase 79 — reduced-Persona assembly + invariant -------

    fn rich_persona() -> EffectivePersona {
        EffectivePersona {
            assistant_name: Some("Ada".to_string()),
            operator_profile: Some("staff SRE".to_string()),
            communication_style: Some("terse".to_string()),
            primary_use_cases: vec!["oncall".to_string()],
            behavioral_preferences: vec!["cite sources".to_string()],
            behavioral_constraints: vec![
                "never run destructive cmds unprompted".to_string(),
            ],
            learned_context: vec![
                "operator uses Vim".to_string(),
                "deploys on Fridays".to_string(),
            ],
            communication_adaptations: vec![
                "conclusion-first".to_string(),
            ],
            character_traits: vec!["dry wit".to_string()],
            relationship_milestones: vec!["shipped v1".to_string()],
        }
    }

    #[test]
    fn reduce_persona_keeps_protected_fields_even_when_keep_rejects_all()
    {
        let full = rich_persona();
        // keep = reject everything.
        let r = reduce_persona(&full, &|_| false);

        // Invariant: scalars + constraints copied in full.
        assert_eq!(r.assistant_name.as_deref(), Some("Ada"));
        assert_eq!(r.operator_profile.as_deref(), Some("staff SRE"));
        assert_eq!(r.communication_style.as_deref(), Some("terse"));
        assert_eq!(
            r.behavioral_constraints,
            vec!["never run destructive cmds unprompted".to_string()]
        );
        // Every soft list emptied.
        assert!(r.primary_use_cases.is_empty());
        assert!(r.behavioral_preferences.is_empty());
        assert!(r.learned_context.is_empty());
        assert!(r.communication_adaptations.is_empty());
        assert!(r.character_traits.is_empty());
        assert!(r.relationship_milestones.is_empty());
    }

    #[test]
    fn reduce_persona_keeps_only_selected_soft_entries() {
        let full = rich_persona();
        let r = reduce_persona(&full, &|s| s == "deploys on Fridays");
        assert_eq!(
            r.learned_context,
            vec!["deploys on Fridays".to_string()]
        );
        // Other soft categories lose their (non-matching) entries.
        assert!(r.character_traits.is_empty());
        // Protected still intact.
        assert_eq!(r.assistant_name.as_deref(), Some("Ada"));
        assert_eq!(r.behavioral_constraints.len(), 1);
    }

    #[test]
    fn reducible_facet_count_excludes_protected() {
        // rich_persona soft entries: 1+1+2+1+1+1 = 7.
        // behavioral_constraints (1) + scalars must NOT count.
        assert_eq!(reducible_facet_count(&rich_persona()), 7);
        assert_eq!(
            reducible_facet_count(&EffectivePersona::default()),
            0
        );
    }

    #[test]
    fn selected_assembly_matches_reduce_then_assemble_and_keeps_constraint(
    ) {
        let profile = operator_declared_profile();
        let full = rich_persona();
        let keep = |s: &str| s == "oncall";

        let via_wrapper = assemble_session_prompt_selected(
            &profile, &full, &keep, "default", "role prompt",
        );
        let manual = assemble_session_prompt(
            &profile,
            Some(&reduce_persona(&full, &keep)),
            "default",
            "role prompt",
        );
        assert_eq!(via_wrapper, manual);

        // End-to-end invariant: a behavioral constraint the
        // selector rejected is STILL in the rendered prompt.
        assert!(via_wrapper
            .contains("never run destructive cmds unprompted"));
        // And the selected soft facet is present...
        assert!(via_wrapper.contains("oncall"));
        // ...while a rejected soft facet is gone.
        assert!(!via_wrapper.contains("dry wit"));
    }
}
