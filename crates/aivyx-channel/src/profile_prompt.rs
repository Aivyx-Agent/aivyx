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

/// Compose the final system prompt for one turn by layering Profile
/// (operator-declared identity per PRODUCT.md P13) alongside the
/// active role's `system_prompt` (per-role voice override per P9).
///
/// **Behavior:**
/// - If [`Profile::is_operator_declared`] returns `false`, return
///   `role_system_prompt.to_string()` unchanged. Zero behavior
///   change for pre-Phase-57 configs.
/// - Otherwise return a labeled composition: a "## About this
///   assistant" section rendering Profile fields, then a "## Active
///   role: <role_name>" section containing `role_system_prompt`.
///
/// `role_name` is rendered into the second section's label so the
/// LLM (and a debugging operator reading prompt logs) can see which
/// role is active. Pass `role.name.value` from the resolved active
/// role.
///
/// The helper allocates a fresh `String` per call. The allocation is
/// dwarfed by the per-turn LLM round-trip cost, so micro-optimizing
/// is not worth the trade against composition clarity.
pub fn assemble_session_prompt(
    profile: &Profile,
    role_name: &str,
    role_system_prompt: &str,
) -> String {
    if !profile.is_operator_declared() {
        return role_system_prompt.to_string();
    }

    let profile_section = render_profile_section(profile);

    // Operator-declared Profile produces a non-empty section. Even
    // if `role_system_prompt` is empty, the labeled "## Active role"
    // header is still rendered — it provides structural cue to the
    // LLM that the role itself has no per-role voice override.
    format!(
        "{profile_section}\n\n## Active role: {role_name}\n\n{role_system_prompt}"
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
        let assembled = assemble_session_prompt(&profile, "default", role_prompt);
        assert_eq!(assembled, role_prompt);
    }

    #[test]
    fn default_profile_with_empty_role_prompt_returns_empty_string() {
        let profile = default_profile();
        let assembled = assemble_session_prompt(&profile, "default", "");
        assert_eq!(assembled, "");
    }

    #[test]
    fn operator_declared_profile_produces_labeled_composition() {
        let profile = operator_declared_profile();
        let role_prompt = "You are a coding assistant.";
        let assembled = assemble_session_prompt(&profile, "coder", role_prompt);

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
        let assembled = assemble_session_prompt(&profile, "coder", "");
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

        let assembled = assemble_session_prompt(&profile, "default", "You are helpful.");
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
        let parent = assemble_session_prompt(&profile, "default", "Parent prompt.");
        let child = assemble_session_prompt(&profile, "junior_researcher", "Child prompt.");

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
}
