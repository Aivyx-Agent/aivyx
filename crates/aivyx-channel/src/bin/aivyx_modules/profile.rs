//! Operator-facing `aivyx profile` CLI surface — Phase 58.
//!
//! Phase 57 shipped the Profile substrate (`aivyx-config::Profile`,
//! `[profile]` TOML table, `assemble_session_prompt`, init wizard,
//! startup-banner row). This module ships the operator surface that
//! closes PRODUCT.md P13:
//!
//! - `aivyx profile show` — labeled, human-readable inspection.
//! - `aivyx profile edit` — surgical `[profile]` section edit in
//!   `$EDITOR`, preserving the rest of `aivyx.toml` via `toml_edit`
//!   (Task 3).
//!
//! Q3(a) resolution at Phase 58 sign-off: `show` reads `aivyx.toml`
//! from disk (no daemon dispatch needed). Profile is operator-mutable
//! only — the agent never writes to it — so disk state and live state
//! are always equivalent modulo a pending daemon restart.
//!
//! Q5(a) resolution at Phase 58 sign-off: edit changes take effect
//! on the next daemon startup. The edit subcommand prints a restart
//! reminder after a successful save (matches the existing
//! load-time-only semantics for role configs).

use std::path::Path;

use aivyx_config::{AivyxConfig, FieldSource, LoadOptions, Profile};

/// Default TOML path. Mirrors the binary's
/// [`crate::DEFAULT_TOML_PATH`] without depending on it (this module
/// is included via `#[path = ...]` and re-exporting from the binary
/// would create a cyclic-looking dependency).
const PROFILE_TOML_PATH: &str = "aivyx.toml";

/// Entry point for `aivyx profile show`. Loads `aivyx.toml` via the
/// same `aivyx-config` path the daemon uses at startup, then renders
/// the resolved [`Profile`] to stdout in a labeled format mirroring
/// the startup banner.
pub fn run_profile_show() -> Result<(), String> {
    let cfg = load_config_for_inspection()?;
    let rendered = render_profile_for_show(&cfg.profile);
    print!("{rendered}");
    Ok(())
}

/// Entry point for `aivyx profile edit`. Stub for Task 2 — the real
/// implementation lands in Task 3 (toml_edit-driven surgical
/// `[profile]` section edit, `$EDITOR` invocation, restart reminder).
pub fn run_profile_edit() -> Result<(), String> {
    Err("`aivyx profile edit` is not yet wired — Task 3.".to_string())
}

/// Load `AivyxConfig` with relaxed validation. The Profile-inspection
/// path does not require an API key, a Telegram token, or a
/// passphrase — it only needs the loader to parse `aivyx.toml` and
/// populate the `profile` field (or synthesize the default).
fn load_config_for_inspection() -> Result<AivyxConfig, String> {
    let opts = LoadOptions {
        toml_path: Some(Path::new(PROFILE_TOML_PATH).to_path_buf()),
        require_api_key: false,
        require_telegram_token: false,
        role_override: None,
    };
    AivyxConfig::load_from_env_and_toml(&opts)
        .map_err(|e| format!("failed to load {PROFILE_TOML_PATH}: {e}"))
}

/// Render the Profile in the labeled format `show` writes to stdout.
/// Pure function — separated from `run_profile_show` so tests can
/// drive it against fixtures without touching the filesystem.
///
/// The format mirrors the startup-banner shape: one field per row,
/// `key = value (source)` for fields with provenance, `key = <unset>`
/// for absent optional fields, and bulleted lists for the three
/// `Vec<String>` categories.
///
/// A trailing `Profile injection: ENABLED/DISABLED` line tells the
/// operator at a glance whether the Profile will flavor every turn's
/// system prompt (`is_operator_declared()` predicate from Phase 57
/// Task 3).
fn render_profile_for_show(profile: &Profile) -> String {
    let mut out = String::new();
    out.push_str("Profile\n");
    out.push_str("=======\n\n");

    out.push_str(&format!(
        "  assistant_name             = {:?} ({})\n",
        profile.assistant_name.value,
        source_label(profile.assistant_name.source),
    ));

    match &profile.operator_profile {
        Some(s) => out.push_str(&format!(
            "  operator_profile           = {s:?}\n"
        )),
        None => out.push_str("  operator_profile           = <unset>\n"),
    }

    match &profile.communication_style {
        Some(s) => out.push_str(&format!(
            "  communication_style        = {s:?}\n"
        )),
        None => out.push_str("  communication_style        = <unset>\n"),
    }

    if profile.primary_use_cases.is_empty() {
        out.push_str("  primary_use_cases          = <unset>\n");
    } else {
        out.push_str("  primary_use_cases:\n");
        for case in &profile.primary_use_cases {
            out.push_str(&format!("    - {case}\n"));
        }
    }

    if profile.behavioral_preferences.is_empty() {
        out.push_str("  behavioral_preferences     = <unset>\n");
    } else {
        out.push_str("  behavioral_preferences:\n");
        for pref in &profile.behavioral_preferences {
            out.push_str(&format!("    - {pref}\n"));
        }
    }

    if profile.behavioral_constraints.is_empty() {
        out.push_str("  behavioral_constraints     = <unset>\n");
    } else {
        out.push_str("  behavioral_constraints:\n");
        for c in &profile.behavioral_constraints {
            out.push_str(&format!("    - {c}\n"));
        }
    }

    out.push('\n');
    if profile.is_operator_declared() {
        out.push_str(
            "Profile injection: ENABLED — Profile flavors every turn's system prompt.\n",
        );
    } else {
        out.push_str(
            "Profile injection: DISABLED — no operator content declared; daemon \
             runs without Profile injection.\n",
        );
    }
    out
}

fn source_label(src: FieldSource) -> &'static str {
    match src {
        FieldSource::Env => "env",
        FieldSource::Toml => "toml",
        FieldSource::EncryptedStore => "encrypted-store",
        FieldSource::Default => "default",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_config::Sourced;

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
            primary_use_cases: vec![
                "Rust systems programming".to_string(),
                "AI agent design".to_string(),
            ],
            behavioral_preferences: vec!["prefer integration tests".to_string()],
            behavioral_constraints: vec!["never auto-commit code".to_string()],
        }
    }

    #[test]
    fn show_default_profile_renders_aivyx_default_and_disabled_injection() {
        let out = render_profile_for_show(&default_profile());
        assert!(out.starts_with("Profile\n=======\n"));
        assert!(out.contains("assistant_name             = \"Aivyx\" (default)"));
        assert!(out.contains("operator_profile           = <unset>"));
        assert!(out.contains("communication_style        = <unset>"));
        assert!(out.contains("primary_use_cases          = <unset>"));
        assert!(out.contains("behavioral_preferences     = <unset>"));
        assert!(out.contains("behavioral_constraints     = <unset>"));
        assert!(out.contains("Profile injection: DISABLED"));
    }

    #[test]
    fn show_operator_declared_profile_renders_all_fields_and_enabled_injection() {
        let out = render_profile_for_show(&operator_declared_profile());

        // Assistant name with toml provenance.
        assert!(out.contains("assistant_name             = \"Codex\" (toml)"));
        // Free-text fields render with quoted value.
        assert!(out.contains("operator_profile           = \"Senior Rust"));
        assert!(out.contains("communication_style        = \"terse, conclusion-first\""));

        // List fields render as bulleted entries.
        assert!(out.contains("primary_use_cases:\n    - Rust systems programming"));
        assert!(out.contains("    - AI agent design"));
        assert!(out.contains("behavioral_preferences:\n    - prefer integration tests"));
        assert!(out.contains("behavioral_constraints:\n    - never auto-commit code"));

        assert!(out.contains("Profile injection: ENABLED"));
    }

    #[test]
    fn show_partial_profile_renders_mix_of_set_and_unset_fields() {
        // Only assistant_name + primary_use_cases declared.
        let profile = Profile {
            assistant_name: Sourced::new("Mira".to_string(), FieldSource::Toml),
            primary_use_cases: vec!["personal-finance analysis".to_string()],
            ..Profile::default()
        };

        let out = render_profile_for_show(&profile);

        // Declared fields render with their values.
        assert!(out.contains("assistant_name             = \"Mira\" (toml)"));
        assert!(out.contains("primary_use_cases:\n    - personal-finance analysis"));

        // Undeclared fields render as <unset>.
        assert!(out.contains("operator_profile           = <unset>"));
        assert!(out.contains("communication_style        = <unset>"));
        assert!(out.contains("behavioral_preferences     = <unset>"));
        assert!(out.contains("behavioral_constraints     = <unset>"));

        // Operator-declared assistant_name flips injection to ENABLED
        // even when most fields are unset.
        assert!(out.contains("Profile injection: ENABLED"));
    }
}
