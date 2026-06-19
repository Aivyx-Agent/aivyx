//! Chapter U — section-scoped writes back to `aivyx.toml`.
//!
//! Before Chapter U, the only writer of `aivyx.toml` was the CLI's
//! `aivyx access set` (`crates/aivyx-cli/.../access.rs`): it loaded the file
//! into a [`toml_edit::DocumentMut`], patched the `[access]` keys in place,
//! and wrote the document back at `0600` — patching keys in place rather than
//! re-serializing the whole config (which would reflow the file and drop the
//! operator's comments).
//!
//! Chapter U adds a **second** writer: the daemon's Settings IPC handlers
//! (`SetAccessLevel` / `SetBudget`). Two independent writers of the same file
//! that must agree byte-for-byte on the schema is exactly the kind of drift
//! this module exists to prevent — so the rewrite logic lives here, once, and
//! both the CLI and the daemon call it.
//!
//! What stays with the *callers*, not here:
//! - the **confirm-first** decision for expanded access levels (the CLI prompts
//!   on stdin; the daemon enforces an explicit `confirm` flag) — see
//!   [`AccessLevel::is_expanded`]. This module performs the *write*; the policy
//!   of whether the operator may perform it is the caller's.
//!
//! What lives here:
//! - the structural **root rules** ([`AccessLevel::Workspace`] /
//!   [`AccessLevel::Custom`] require a `root`; the auto-derived levels reject an
//!   explicit one), so neither caller can write a nonsensical `[access]`;
//! - the **budget validation** mirrored from the loader (non-negative caps;
//!   `alert_at` within `[0.0, 1.0]`) so a bad cap is refused before it touches
//!   disk rather than failing the *next* daemon load;
//! - the `0600` permission posture (the file may carry secrets in other
//!   sections).

use std::path::Path;

use aivyx_cost::{BudgetAction, BudgetConfig};
use toml_edit::{value, DocumentMut};

use crate::AccessLevel;

/// Failure modes for a section-scoped `aivyx.toml` rewrite. Carries enough
/// structure that the daemon can map a write failure to a typed IPC error;
/// the CLI renders the `Display` string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigWriteError {
    /// `workspace` / `custom` were requested without a `root`.
    RootRequired { level: AccessLevel },
    /// A `root` was supplied for a level that derives its own.
    RootNotAllowed { level: AccessLevel },
    /// A budget field is out of range (negative cap, or `alert_at` outside
    /// `[0.0, 1.0]`). Mirrors the loader's validation so the write is refused
    /// before it can corrupt the next load.
    InvalidBudget { reason: String },
    /// The existing file did not parse as TOML.
    Parse { reason: String },
    /// The file could not be read or written.
    Io { reason: String },
}

impl std::fmt::Display for ConfigWriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigWriteError::RootRequired { level } => write!(
                f,
                "access level `{}` needs an explicit root directory",
                level.as_str()
            ),
            ConfigWriteError::RootNotAllowed { level } => write!(
                f,
                "access level `{}` derives its root automatically; an explicit root does not apply",
                level.as_str()
            ),
            ConfigWriteError::InvalidBudget { reason } => write!(f, "invalid budget: {reason}"),
            ConfigWriteError::Parse { reason } => write!(f, "failed to parse aivyx.toml: {reason}"),
            ConfigWriteError::Io { reason } => write!(f, "{reason}"),
        }
    }
}

impl std::error::Error for ConfigWriteError {}

/// Rewrite the `[access]` section of the TOML file at `path`, preserving every
/// other section and the operator's comments.
///
/// Sets `level`, sets-or-clears `root` (per the level's root rules), and sets
/// `confirm_destructive = is_expanded()` — exactly what `aivyx access set`
/// wrote before Chapter U. A missing file is treated as empty (the section is
/// created). The result is written at `0600`.
///
/// The caller is responsible for the **confirm-first** decision on expanded
/// levels; this function only enforces the structural root rules.
pub fn write_access_section(
    path: &Path,
    level: AccessLevel,
    root: Option<&str>,
) -> Result<(), ConfigWriteError> {
    // Structural root rules — neither caller may write a nonsensical access
    // section (workspace/custom need a directory; the derived levels reject
    // an explicit one that would shadow their derivation).
    match level {
        AccessLevel::Workspace | AccessLevel::Custom if root.is_none() => {
            return Err(ConfigWriteError::RootRequired { level });
        }
        AccessLevel::Sandbox | AccessLevel::Home | AccessLevel::Full if root.is_some() => {
            return Err(ConfigWriteError::RootNotAllowed { level });
        }
        _ => {}
    }

    let mut doc = load_document(path)?;

    doc["access"]["level"] = value(level.as_str());
    match root {
        Some(r) => doc["access"]["root"] = value(r),
        // Switching to a level that derives its root: drop any stale
        // `[access] root` so it doesn't shadow the derivation.
        None => {
            if let Some(t) = doc.get_mut("access").and_then(|a| a.as_table_mut()) {
                t.remove("root");
            }
        }
    }
    doc["access"]["confirm_destructive"] = value(level.is_expanded());

    write_toml_0600(path, &doc.to_string())
}

/// Rewrite the `[budget]` section of the TOML file at `path`, preserving every
/// other section and the operator's comments.
///
/// `None` caps clear their key (uncapped is the default). `on_exceeded` and
/// `alert_at` are always written. Validation mirrors the loader
/// (`AivyxConfig::load`): negative caps and an out-of-range `alert_at` are
/// refused here so a bad write can never reach the gate's reservation math.
pub fn write_budget_section(path: &Path, budget: &BudgetConfig) -> Result<(), ConfigWriteError> {
    for (name, cap) in [
        ("per_run_usd", budget.per_run_usd),
        ("per_day_usd", budget.per_day_usd),
    ] {
        if let Some(c) = cap {
            if c < 0.0 {
                return Err(ConfigWriteError::InvalidBudget {
                    reason: format!("{name} must be non-negative"),
                });
            }
        }
    }
    if let Some(frac) = budget.alert_at {
        if !(0.0..=1.0).contains(&frac) {
            return Err(ConfigWriteError::InvalidBudget {
                reason: "alert_at must be within [0.0, 1.0]".to_string(),
            });
        }
    }

    let mut doc = load_document(path)?;

    set_or_clear_f64(&mut doc, "per_run_usd", budget.per_run_usd);
    set_or_clear_f64(&mut doc, "per_day_usd", budget.per_day_usd);
    doc["budget"]["on_exceeded"] = value(budget_action_str(budget.on_exceeded));
    set_or_clear_f64(&mut doc, "alert_at", budget.alert_at);

    write_toml_0600(path, &doc.to_string())
}

/// The six operator-declared `[profile]` fields, in the carrier the daemon's
/// `SetProfile` handler fills from IPC. Mirrors the loader's `RawProfile`
/// shape (Chapter V §9.1): all fields optional, with **clear-on-`None`**
/// semantics — a `None` removes the key (the loader falls back to its default,
/// e.g. `assistant_name` → `"Aivyx"`), a `Some` writes it.
///
/// Values are normalized on write: scalars are trimmed (an all-whitespace
/// scalar clears the key), and list entries are trimmed with empties dropped —
/// so the form's blank rows never reach disk.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProfileWrite {
    pub assistant_name: Option<String>,
    pub operator_profile: Option<String>,
    pub communication_style: Option<String>,
    pub primary_use_cases: Option<Vec<String>>,
    pub behavioral_preferences: Option<Vec<String>>,
    pub behavioral_constraints: Option<Vec<String>>,
}

/// Rewrite the `[profile]` section of the TOML file at `path`, preserving every
/// other section and the operator's comments — the surgical-splice posture of
/// `aivyx profile edit`, but driven by structured fields instead of an editor
/// round-trip.
///
/// Each field follows clear-on-`None` (see [`ProfileWrite`]): a present value
/// is normalized and written; an absent one removes the key so the loader's
/// default applies. There is nothing to *validate* here — Profile fields are
/// free-form declarations — so this never fails on content, only on I/O or a
/// malformed existing file.
pub fn write_profile_section(path: &Path, profile: &ProfileWrite) -> Result<(), ConfigWriteError> {
    let mut doc = load_document(path)?;

    set_or_clear_profile_str(&mut doc, "assistant_name", profile.assistant_name.as_deref());
    set_or_clear_profile_str(&mut doc, "operator_profile", profile.operator_profile.as_deref());
    set_or_clear_profile_str(
        &mut doc,
        "communication_style",
        profile.communication_style.as_deref(),
    );
    set_or_clear_profile_list(&mut doc, "primary_use_cases", profile.primary_use_cases.as_deref());
    set_or_clear_profile_list(
        &mut doc,
        "behavioral_preferences",
        profile.behavioral_preferences.as_deref(),
    );
    set_or_clear_profile_list(
        &mut doc,
        "behavioral_constraints",
        profile.behavioral_constraints.as_deref(),
    );

    write_toml_0600(path, &doc.to_string())
}

/// Set `[profile].<key>` to the trimmed scalar, or remove the key when the
/// value is `None` or trims to empty (the loader's default then applies).
fn set_or_clear_profile_str(doc: &mut DocumentMut, key: &str, v: Option<&str>) {
    match v.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => doc["profile"][key] = value(s),
        None => remove_profile_key(doc, key),
    }
}

/// Set `[profile].<key>` to a TOML array of the trimmed, non-empty entries, or
/// remove the key when the list is `None`. An explicit `Some(vec![])` (or a
/// list of only blanks) writes an empty array — "declared, but empty" — which
/// is distinct from the key being absent.
fn set_or_clear_profile_list(doc: &mut DocumentMut, key: &str, v: Option<&[String]>) {
    match v {
        Some(items) => {
            let mut arr = toml_edit::Array::new();
            for item in items {
                let t = item.trim();
                if !t.is_empty() {
                    arr.push(t);
                }
            }
            doc["profile"][key] = value(arr);
        }
        None => remove_profile_key(doc, key),
    }
}

/// Remove `key` from the `[profile]` table if the table exists.
fn remove_profile_key(doc: &mut DocumentMut, key: &str) {
    if let Some(t) = doc.get_mut("profile").and_then(|p| p.as_table_mut()) {
        t.remove(key);
    }
}

/// The `[voice]` fields the Studio's Voice screen edits, in the carrier the
/// daemon's `SetVoice` handler fills from IPC. Mirrors `VoiceOptions` (Chapter
/// Voice §12.1): all optional, **clear-on-`None`** — an absent field removes the
/// key (the voice loader's default then applies). Strings are trimmed (an
/// all-whitespace value clears the key); paths are carried as strings (TOML
/// stores them as strings regardless).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VoiceWrite {
    pub asr_engine: Option<String>,
    pub tts_engine: Option<String>,
    pub asr_model_path: Option<String>,
    pub asr_language: Option<String>,
    pub asr_beam_size: Option<u32>,
    pub tts_model_dir: Option<String>,
    pub tts_voice_name: Option<String>,
    pub tts_speed: Option<f32>,
    pub input_device: Option<String>,
    pub output_device: Option<String>,
}

/// Rewrite the `[voice]` section of the TOML file at `path`, preserving every
/// other section and the operator's comments — the section-scoped `toml_edit`
/// posture of the access/budget/profile writers.
///
/// Each field follows clear-on-`None`: a present value is normalized and
/// written; an absent one removes the key. There is nothing to *validate* here
/// (the voice channel validates engine names + that the model files exist at
/// launch), so this never fails on content — only on I/O or a malformed file.
pub fn write_voice_section(path: &Path, voice: &VoiceWrite) -> Result<(), ConfigWriteError> {
    let mut doc = load_document(path)?;

    set_or_clear_voice_str(&mut doc, "asr_engine", voice.asr_engine.as_deref());
    set_or_clear_voice_str(&mut doc, "tts_engine", voice.tts_engine.as_deref());
    set_or_clear_voice_str(&mut doc, "asr_model_path", voice.asr_model_path.as_deref());
    set_or_clear_voice_str(&mut doc, "asr_language", voice.asr_language.as_deref());
    match voice.asr_beam_size {
        Some(n) => doc["voice"]["asr_beam_size"] = value(n as i64),
        None => remove_voice_key(&mut doc, "asr_beam_size"),
    }
    set_or_clear_voice_str(&mut doc, "tts_model_dir", voice.tts_model_dir.as_deref());
    set_or_clear_voice_str(&mut doc, "tts_voice_name", voice.tts_voice_name.as_deref());
    match voice.tts_speed {
        Some(s) => doc["voice"]["tts_speed"] = value(s as f64),
        None => remove_voice_key(&mut doc, "tts_speed"),
    }
    set_or_clear_voice_str(&mut doc, "input_device", voice.input_device.as_deref());
    set_or_clear_voice_str(&mut doc, "output_device", voice.output_device.as_deref());

    write_toml_0600(path, &doc.to_string())
}

/// Set `[voice].<key>` to the trimmed string, or remove the key when the value
/// is `None` or trims to empty.
fn set_or_clear_voice_str(doc: &mut DocumentMut, key: &str, v: Option<&str>) {
    match v.map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => doc["voice"][key] = value(s),
        None => remove_voice_key(doc, key),
    }
}

/// Remove `key` from the `[voice]` table if the table exists.
fn remove_voice_key(doc: &mut DocumentMut, key: &str) {
    if let Some(t) = doc.get_mut("voice").and_then(|v| v.as_table_mut()) {
        t.remove(key);
    }
}

/// Stable `[budget] on_exceeded` token — matches `BudgetAction`'s
/// `#[serde(rename_all = "snake_case")]` repr so a written file round-trips
/// through the loader unchanged.
fn budget_action_str(action: BudgetAction) -> &'static str {
    match action {
        BudgetAction::Alert => "alert",
        BudgetAction::Deny => "deny",
    }
}

/// Set `[budget].<key>` to `v`, or remove the key when `v` is `None`
/// (uncapped — the loader's default).
fn set_or_clear_f64(doc: &mut DocumentMut, key: &str, v: Option<f64>) {
    match v {
        Some(n) => doc["budget"][key] = value(n),
        None => {
            if let Some(t) = doc.get_mut("budget").and_then(|b| b.as_table_mut()) {
                t.remove(key);
            }
        }
    }
}

/// Parse the file at `path` into an editable document; a missing file is an
/// empty document (the caller's section is created).
fn load_document(path: &Path) -> Result<DocumentMut, ConfigWriteError> {
    let original = if path.exists() {
        std::fs::read_to_string(path).map_err(|e| ConfigWriteError::Io {
            reason: format!("failed to read {}: {e}", path.display()),
        })?
    } else {
        String::new()
    };
    original
        .parse::<DocumentMut>()
        .map_err(|e| ConfigWriteError::Parse {
            reason: format!("{} is not valid TOML: {e}", path.display()),
        })
}

/// Write `contents` to `path` and pin it to `0600` (the file may carry secrets
/// in other sections — same posture every config writer uses).
fn write_toml_0600(path: &Path, contents: &str) -> Result<(), ConfigWriteError> {
    std::fs::write(path, contents).map_err(|e| ConfigWriteError::Io {
        reason: format!("failed to write {}: {e}", path.display()),
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, perms).map_err(|e| ConfigWriteError::Io {
            reason: format!("failed to set permissions on {}: {e}", path.display()),
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unique temp file path for one test (no external tempdir dep).
    fn temp_toml(tag: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "aivyx-cfgwrite-{}-{}-{tag}.toml",
            std::process::id(),
            // a cheap per-call nonce so parallel tests don't collide
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        p
    }

    #[test]
    fn access_writes_level_confirm_and_drops_stale_root() {
        let path = temp_toml("access");
        std::fs::write(&path, "[access]\nlevel = \"workspace\"\nroot = \"/old\"\n").unwrap();
        write_access_section(&path, AccessLevel::Home, None).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("level = \"home\""), "{out}");
        assert!(out.contains("confirm_destructive = true"), "{out}");
        assert!(!out.contains("/old"), "stale root must be dropped: {out}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn access_sandbox_clears_confirm() {
        let path = temp_toml("sandbox");
        write_access_section(&path, AccessLevel::Sandbox, None).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("level = \"sandbox\""), "{out}");
        assert!(out.contains("confirm_destructive = false"), "{out}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn access_level_from_wire_round_trips_as_str() {
        for lvl in [
            AccessLevel::Sandbox,
            AccessLevel::Workspace,
            AccessLevel::Home,
            AccessLevel::Full,
            AccessLevel::Custom,
        ] {
            assert_eq!(AccessLevel::from_wire(lvl.as_str()), Some(lvl));
        }
        assert_eq!(AccessLevel::from_wire("bogus"), None);
    }

    #[test]
    fn access_workspace_requires_root() {
        let path = temp_toml("ws");
        let err = write_access_section(&path, AccessLevel::Workspace, None).unwrap_err();
        assert_eq!(err, ConfigWriteError::RootRequired { level: AccessLevel::Workspace });
        assert!(!path.exists(), "no file should be written on a validation error");
    }

    #[test]
    fn access_home_rejects_explicit_root() {
        let path = temp_toml("homeroot");
        let err = write_access_section(&path, AccessLevel::Home, Some("/x")).unwrap_err();
        assert_eq!(err, ConfigWriteError::RootNotAllowed { level: AccessLevel::Home });
    }

    #[test]
    fn access_custom_writes_root() {
        let path = temp_toml("custom");
        write_access_section(&path, AccessLevel::Custom, Some("/srv/agent")).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("level = \"custom\""), "{out}");
        assert!(out.contains("root = \"/srv/agent\""), "{out}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn access_preserves_other_sections_and_comments() {
        let path = temp_toml("preserve");
        std::fs::write(
            &path,
            "# my config\n[profile]\nassistant_name = \"Aivyx\"\n\n[access]\nlevel = \"sandbox\"\n",
        )
        .unwrap();
        write_access_section(&path, AccessLevel::Full, None).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("# my config"), "comment preserved: {out}");
        assert!(out.contains("assistant_name = \"Aivyx\""), "other section preserved: {out}");
        assert!(out.contains("level = \"full\""), "{out}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn budget_writes_caps_action_and_alert() {
        let path = temp_toml("budget");
        let b = BudgetConfig {
            per_run_usd: Some(5.0),
            per_day_usd: Some(20.0),
            on_exceeded: BudgetAction::Deny,
            alert_at: Some(0.8),
        };
        write_budget_section(&path, &b).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("per_run_usd = 5.0"), "{out}");
        assert!(out.contains("per_day_usd = 20.0"), "{out}");
        assert!(out.contains("on_exceeded = \"deny\""), "{out}");
        assert!(out.contains("alert_at = 0.8"), "{out}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn budget_clears_none_caps() {
        let path = temp_toml("budget-clear");
        std::fs::write(&path, "[budget]\nper_run_usd = 9.0\nper_day_usd = 9.0\n").unwrap();
        let b = BudgetConfig {
            per_run_usd: None,
            per_day_usd: Some(15.0),
            on_exceeded: BudgetAction::Alert,
            alert_at: None,
        };
        write_budget_section(&path, &b).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(!out.contains("per_run_usd"), "None cap must be cleared: {out}");
        assert!(out.contains("per_day_usd = 15.0"), "{out}");
        assert!(out.contains("on_exceeded = \"alert\""), "{out}");
        assert!(!out.contains("alert_at"), "None alert_at must be cleared: {out}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn budget_rejects_negative_cap() {
        let path = temp_toml("budget-neg");
        let b = BudgetConfig {
            per_run_usd: Some(-1.0),
            ..Default::default()
        };
        let err = write_budget_section(&path, &b).unwrap_err();
        assert!(matches!(err, ConfigWriteError::InvalidBudget { .. }), "{err:?}");
        assert!(!path.exists(), "no file should be written on a validation error");
    }

    #[test]
    fn budget_rejects_out_of_range_alert() {
        let path = temp_toml("budget-alert");
        let b = BudgetConfig {
            alert_at: Some(1.5),
            ..Default::default()
        };
        let err = write_budget_section(&path, &b).unwrap_err();
        assert!(matches!(err, ConfigWriteError::InvalidBudget { .. }), "{err:?}");
    }

    #[test]
    fn profile_writes_scalars_and_lists() {
        let path = temp_toml("profile");
        let p = ProfileWrite {
            assistant_name: Some("Aria".to_string()),
            operator_profile: Some("Indie game dev".to_string()),
            communication_style: Some("terse".to_string()),
            primary_use_cases: Some(vec!["coding".to_string(), "research".to_string()]),
            behavioral_preferences: Some(vec!["cite sources".to_string()]),
            behavioral_constraints: Some(vec!["no secrets in logs".to_string()]),
        };
        write_profile_section(&path, &p).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("assistant_name = \"Aria\""), "{out}");
        assert!(out.contains("operator_profile = \"Indie game dev\""), "{out}");
        assert!(out.contains("communication_style = \"terse\""), "{out}");
        assert!(out.contains(r#"primary_use_cases = ["coding", "research"]"#), "{out}");
        assert!(out.contains(r#"behavioral_preferences = ["cite sources"]"#), "{out}");
        assert!(out.contains(r#"behavioral_constraints = ["no secrets in logs"]"#), "{out}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn profile_clears_none_fields() {
        let path = temp_toml("profile-clear");
        std::fs::write(
            &path,
            "[profile]\nassistant_name = \"Old\"\noperator_profile = \"gone\"\nprimary_use_cases = [\"a\"]\n",
        )
        .unwrap();
        let p = ProfileWrite {
            assistant_name: Some("New".to_string()),
            // operator_profile + primary_use_cases left None → cleared
            ..Default::default()
        };
        write_profile_section(&path, &p).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("assistant_name = \"New\""), "{out}");
        assert!(!out.contains("operator_profile"), "None scalar must be cleared: {out}");
        assert!(!out.contains("primary_use_cases"), "None list must be cleared: {out}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn profile_normalizes_blanks() {
        let path = temp_toml("profile-blank");
        let p = ProfileWrite {
            // all-whitespace scalar → cleared (treated as None)
            assistant_name: Some("   ".to_string()),
            // list with blanks → trimmed, empties dropped
            primary_use_cases: Some(vec![
                "  coding  ".to_string(),
                "".to_string(),
                "   ".to_string(),
                "ops".to_string(),
            ]),
            ..Default::default()
        };
        write_profile_section(&path, &p).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(!out.contains("assistant_name"), "blank scalar must clear: {out}");
        assert!(out.contains(r#"primary_use_cases = ["coding", "ops"]"#), "{out}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn profile_explicit_empty_list_is_declared_empty() {
        let path = temp_toml("profile-empty-list");
        let p = ProfileWrite {
            behavioral_preferences: Some(vec![]),
            ..Default::default()
        };
        write_profile_section(&path, &p).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        // Some(vec![]) is "declared but empty" — distinct from absent.
        assert!(out.contains("behavioral_preferences = []"), "{out}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn profile_preserves_other_sections_and_comments() {
        let path = temp_toml("profile-preserve");
        std::fs::write(
            &path,
            "# top comment\n[agent]\nprovider = \"ollama\"\n\n[access]\nlevel = \"home\"\n",
        )
        .unwrap();
        let p = ProfileWrite {
            assistant_name: Some("Aivyx".to_string()),
            ..Default::default()
        };
        write_profile_section(&path, &p).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("# top comment"), "comment preserved: {out}");
        assert!(out.contains("provider = \"ollama\""), "[agent] preserved: {out}");
        assert!(out.contains("level = \"home\""), "[access] preserved: {out}");
        assert!(out.contains("assistant_name = \"Aivyx\""), "{out}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn voice_writes_strings_paths_and_beam() {
        let path = temp_toml("voice");
        let v = VoiceWrite {
            asr_engine: Some("whisper-rs".to_string()),
            tts_engine: Some("kokoro".to_string()),
            asr_model_path: Some("/models/whisper.bin".to_string()),
            asr_language: Some("en".to_string()),
            asr_beam_size: Some(5),
            tts_model_dir: Some("/models/kokoro".to_string()),
            tts_voice_name: Some("af_heart".to_string()),
            tts_speed: Some(1.25),
            input_device: None,
            output_device: None,
        };
        write_voice_section(&path, &v).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("asr_engine = \"whisper-rs\""), "{out}");
        assert!(out.contains("asr_model_path = \"/models/whisper.bin\""), "{out}");
        assert!(out.contains("asr_beam_size = 5"), "{out}");
        assert!(out.contains("tts_model_dir = \"/models/kokoro\""), "{out}");
        assert!(out.contains("tts_voice_name = \"af_heart\""), "{out}");
        assert!(out.contains("tts_speed = 1.25"), "{out}");
        assert!(!out.contains("input_device"), "None device must be absent: {out}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn voice_clears_none_and_blank_fields() {
        let path = temp_toml("voice-clear");
        std::fs::write(
            &path,
            "[voice]\nasr_model_path = \"/old.bin\"\nasr_beam_size = 8\nasr_language = \"en\"\n",
        )
        .unwrap();
        let v = VoiceWrite {
            asr_language: Some("  ".to_string()), // whitespace → cleared
            // asr_model_path + asr_beam_size left None → cleared
            ..Default::default()
        };
        write_voice_section(&path, &v).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(!out.contains("asr_model_path"), "None path must be cleared: {out}");
        assert!(!out.contains("asr_beam_size"), "None beam must be cleared: {out}");
        assert!(!out.contains("asr_language"), "blank string must be cleared: {out}");
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn voice_preserves_other_sections() {
        let path = temp_toml("voice-preserve");
        std::fs::write(&path, "# cfg\n[agent]\nprovider = \"ollama\"\n").unwrap();
        let v = VoiceWrite {
            tts_engine: Some("kokoro".to_string()),
            ..Default::default()
        };
        write_voice_section(&path, &v).unwrap();
        let out = std::fs::read_to_string(&path).unwrap();
        assert!(out.contains("# cfg"), "comment preserved: {out}");
        assert!(out.contains("provider = \"ollama\""), "[agent] preserved: {out}");
        assert!(out.contains("tts_engine = \"kokoro\""), "{out}");
        std::fs::remove_file(&path).ok();
    }

    #[cfg(unix)]
    #[test]
    fn written_file_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_toml("perms");
        write_access_section(&path, AccessLevel::Sandbox, None).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "got {mode:o}");
        std::fs::remove_file(&path).ok();
    }
}
