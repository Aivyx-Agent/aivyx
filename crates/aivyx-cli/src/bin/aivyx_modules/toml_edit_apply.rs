//! Phase 119 Task 3 — Atomic `aivyx.toml` editing primitive for the
//! Phase 118 apply-side commands.
//!
//! Two operator-facing CLI commands need to mutate `aivyx.toml`:
//!
//! - `aivyx profile apply-hint <id>` (Task 4) — set or append a single
//!   `[profile]` field per an approved `ProfileFieldHint`.
//! - `aivyx role import <id>` (Task 5) — add a new `[roles.<name>]`
//!   section per an approved `RoleDraft`.
//!
//! Both mutations share the same shape: parse the existing TOML with
//! `toml_edit` (comment-preserving), apply a single surgical change,
//! write the new document atomically.
//!
//! ## Why a dedicated module
//!
//! Phase 58's `aivyx profile edit` (CLI editor flow in `profile.rs`)
//! already touches `aivyx.toml` via `toml_edit`. Phase 119 introduces
//! programmatic apply paths that don't go through `$EDITOR`; sharing
//! the atomicity contract via this module avoids duplicating the
//! write-to-tmp + rename pattern in two CLI subcommands.
//!
//! ## Atomicity
//!
//! Write-to-temporary + `rename` within the same directory. POSIX
//! guarantees `rename(2)` on the same filesystem is atomic — either
//! the target ends up fully written or it does not change at all. A
//! crash mid-write leaves the original file intact.
//!
//! The tmp file is created with `0o600` permissions before writing
//! to it; the rename inherits those permissions. Matches the
//! Phase 58 `write_aivyx_toml` posture and Phase 64 identity-export
//! posture (file permissions 0600 on Unix).
//!
//! ## Pure transforms vs. I/O
//!
//! The pure-function transforms (`apply_profile_hint_to_doc` and
//! `apply_role_draft_to_doc`) take a `DocumentMut` and a payload and
//! return the mutated document. The I/O wrappers
//! (`apply_profile_hint_to_path`, `apply_role_draft_to_path`)
//! handle read + transform + atomic write.
//!
//! Tests exercise the pure transforms directly without touching the
//! filesystem; the e2e tests at Task 7 exercise the full I/O path.

use std::path::Path;

use aivyx_core::skill_proposer::{ProfileFieldHint, RoleDraft};

/// What the apply step actually wrote, returned to the caller so the
/// audit-event emission (Phase 119 Tasks 4-5) has the exact applied
/// value at hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedProfileHint {
    /// `ProfileField::label()` — matches the audit event's `field`.
    pub field: String,
    /// For scalar fields: the new value written. For list fields: the
    /// entry appended.
    pub applied_value: String,
}

/// Mirror of `AppliedProfileHint` for the second category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedRoleDraft {
    pub role_name: String,
    pub parent: Option<String>,
}

/// Error shape for both apply paths. String-typed for CLI clarity;
/// the operator sees a one-line error.
#[derive(Debug, thiserror::Error)]
pub enum TomlApplyError {
    #[error("failed to read {path}: {reason}")]
    Read { path: String, reason: String },
    #[error("failed to parse {path} as TOML: {reason}")]
    Parse { path: String, reason: String },
    #[error("failed to write {path}: {reason}")]
    Write { path: String, reason: String },
    #[error("role `{name}` already exists in {path}; re-run with --force to overwrite")]
    RoleExists { name: String, path: String },
}

// ---------------------------------------------------------------------------
// ProfileHint apply path
// ---------------------------------------------------------------------------

/// Pure transform: apply a `ProfileFieldHint` to a parsed `DocumentMut`,
/// returning the description of what got written for downstream audit.
///
/// Scalar fields overwrite the existing scalar (the hint IS the
/// operator's decision; the routing forced Staged precisely so the
/// operator could review the overwrite explicitly).
///
/// List fields append the suggested value IF it isn't already present
/// (idempotent — re-applying the same hint is a no-op).
pub fn apply_profile_hint_to_doc(
    doc: &mut toml_edit::DocumentMut,
    hint: &ProfileFieldHint,
) -> AppliedProfileHint {
    // Ensure the [profile] table exists.
    if !doc.contains_table("profile") {
        let table = toml_edit::Table::new();
        doc.insert("profile", toml_edit::Item::Table(table));
    }
    let profile_table = doc
        .get_mut("profile")
        .and_then(|item| item.as_table_mut())
        .expect("profile table just inserted");

    let key = hint.field.label();
    if hint.field.is_scalar() {
        profile_table.insert(
            key,
            toml_edit::value(hint.suggested_value.clone()),
        );
    } else {
        // List field — append if absent.
        let array = match profile_table.get_mut(key) {
            Some(toml_edit::Item::Value(toml_edit::Value::Array(arr))) => arr,
            _ => {
                profile_table.insert(
                    key,
                    toml_edit::value(toml_edit::Array::new()),
                );
                match profile_table.get_mut(key).unwrap() {
                    toml_edit::Item::Value(toml_edit::Value::Array(arr)) => {
                        arr
                    }
                    _ => unreachable!("just inserted an Array"),
                }
            }
        };
        let already_present = array.iter().any(|v| {
            v.as_str().map(|s| s == hint.suggested_value).unwrap_or(false)
        });
        if !already_present {
            array.push(hint.suggested_value.clone());
        }
    }
    AppliedProfileHint {
        field: key.to_string(),
        applied_value: hint.suggested_value.clone(),
    }
}

/// I/O wrapper: read `path`, apply, write atomically. The atomic-
/// write contract (tmp + rename) lives in [`write_atomic_with_0600`].
pub fn apply_profile_hint_to_path(
    path: &Path,
    hint: &ProfileFieldHint,
) -> Result<AppliedProfileHint, TomlApplyError> {
    let original = read_or_empty(path)?;
    let mut doc = parse_document(path, &original)?;
    let applied = apply_profile_hint_to_doc(&mut doc, hint);
    write_atomic_with_0600(path, &doc.to_string())?;
    Ok(applied)
}

// ---------------------------------------------------------------------------
// RoleDraft apply path
// ---------------------------------------------------------------------------

/// Pure transform: apply a `RoleDraft` to a parsed `DocumentMut`,
/// returning the description of what got written for downstream audit.
///
/// Refuses to overwrite an existing `[roles.<name>]` section unless
/// `force` is set — operator opts in to overwrite by passing
/// `--force` to the CLI command.
///
/// The written section uses the addendum fields directly: Phase 13's
/// role inheritance primitive applies the addendum over any inherited
/// envelope at role-resolution time, so the section captures just the
/// delta the draft drafted.
pub fn apply_role_draft_to_doc(
    doc: &mut toml_edit::DocumentMut,
    role: &RoleDraft,
    force: bool,
    path_for_error: &str,
) -> Result<AppliedRoleDraft, TomlApplyError> {
    // Ensure the top-level [roles] table exists. TOML dotted-key
    // syntax `roles.<name>` works without an explicit [roles]
    // wrapper, but creating the wrapper first keeps the layout
    // predictable across multiple appends.
    if !doc.contains_table("roles") {
        let mut table = toml_edit::Table::new();
        // Mark the table as implicit so it doesn't print a bare
        // `[roles]` header above the per-role tables.
        table.set_implicit(true);
        doc.insert("roles", toml_edit::Item::Table(table));
    }
    let roles_table = doc
        .get_mut("roles")
        .and_then(|item| item.as_table_mut())
        .expect("roles table just inserted");

    if roles_table.contains_key(&role.name) && !force {
        return Err(TomlApplyError::RoleExists {
            name: role.name.clone(),
            path: path_for_error.to_string(),
        });
    }

    let mut new_table = toml_edit::Table::new();
    if let Some(parent) = &role.parent {
        new_table.insert("inherits_from", toml_edit::value(parent.clone()));
    }
    if !role.system_prompt_addendum.is_empty() {
        new_table.insert(
            "system_prompt",
            toml_edit::value(role.system_prompt_addendum.clone()),
        );
    }
    if !role.tool_allowlist_additions.is_empty() {
        let mut array = toml_edit::Array::new();
        for tool in &role.tool_allowlist_additions {
            array.push(tool.clone());
        }
        new_table.insert("tool_allowlist", toml_edit::value(array));
    }
    roles_table.insert(&role.name, toml_edit::Item::Table(new_table));

    Ok(AppliedRoleDraft {
        role_name: role.name.clone(),
        parent: role.parent.clone(),
    })
}

/// I/O wrapper for the RoleDraft apply path.
pub fn apply_role_draft_to_path(
    path: &Path,
    role: &RoleDraft,
    force: bool,
) -> Result<AppliedRoleDraft, TomlApplyError> {
    let original = read_or_empty(path)?;
    let mut doc = parse_document(path, &original)?;
    let path_str = path.display().to_string();
    let applied = apply_role_draft_to_doc(&mut doc, role, force, &path_str)?;
    write_atomic_with_0600(path, &doc.to_string())?;
    Ok(applied)
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn read_or_empty(path: &Path) -> Result<String, TomlApplyError> {
    if !path.exists() {
        return Ok(String::new());
    }
    std::fs::read_to_string(path).map_err(|e| TomlApplyError::Read {
        path: path.display().to_string(),
        reason: e.to_string(),
    })
}

fn parse_document(
    path: &Path,
    text: &str,
) -> Result<toml_edit::DocumentMut, TomlApplyError> {
    text.parse::<toml_edit::DocumentMut>().map_err(|e| {
        TomlApplyError::Parse {
            path: path.display().to_string(),
            reason: e.to_string(),
        }
    })
}

/// Atomic write to `path`: create a sibling tmp file with `0o600`
/// permissions on Unix, write the new contents, then `rename` over the
/// destination. POSIX guarantees `rename(2)` is atomic on the same
/// filesystem; a crash mid-write leaves the original intact.
///
/// The tmp filename derives from the destination plus a per-process
/// unique suffix so concurrent applies (unlikely but possible) don't
/// clobber each other's tmp files. The destination's parent
/// directory is the tmp file's directory so the rename stays
/// same-filesystem.
fn write_atomic_with_0600(
    path: &Path,
    contents: &str,
) -> Result<(), TomlApplyError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "aivyx.toml".to_string());
    let tmp_name = format!(".{}.phase119-tmp.{}", file_name, std::process::id());
    let tmp_path = parent.join(tmp_name);

    // Write the tmp file.
    std::fs::write(&tmp_path, contents).map_err(|e| TomlApplyError::Write {
        path: tmp_path.display().to_string(),
        reason: e.to_string(),
    })?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        if let Err(e) = std::fs::set_permissions(&tmp_path, perms) {
            // Best-effort cleanup of the tmp file on permission
            // failure. The rename below won't have happened yet.
            let _ = std::fs::remove_file(&tmp_path);
            return Err(TomlApplyError::Write {
                path: tmp_path.display().to_string(),
                reason: format!("set_permissions: {e}"),
            });
        }
    }

    // Atomic rename. On any failure we clean up the tmp file so a
    // failed apply doesn't leave debris.
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(TomlApplyError::Write {
            path: path.display().to_string(),
            reason: format!("rename: {e}"),
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use aivyx_core::skill_proposer::{ProfileField, ProfileFieldHint, RoleDraft};

    fn parse(text: &str) -> toml_edit::DocumentMut {
        text.parse().expect("test fixture parses")
    }

    // ----- ProfileHint apply (pure transform) -----

    #[test]
    fn profile_hint_scalar_sets_field_on_empty_document() {
        let mut doc = parse("");
        let hint = ProfileFieldHint {
            field: ProfileField::CommunicationStyle,
            suggested_value: "terse and bullet-formatted".into(),
            rationale: "...".into(),
        };
        let applied = apply_profile_hint_to_doc(&mut doc, &hint);
        assert_eq!(applied.field, "communication_style");
        assert_eq!(applied.applied_value, "terse and bullet-formatted");
        let out = doc.to_string();
        assert!(out.contains("[profile]"));
        assert!(out.contains("communication_style = \"terse and bullet-formatted\""));
    }

    #[test]
    fn profile_hint_scalar_overwrites_existing_value() {
        let mut doc = parse(
            "[profile]\n\
             communication_style = \"old verbose style\"\n",
        );
        let hint = ProfileFieldHint {
            field: ProfileField::CommunicationStyle,
            suggested_value: "terse".into(),
            rationale: "...".into(),
        };
        apply_profile_hint_to_doc(&mut doc, &hint);
        let out = doc.to_string();
        assert!(out.contains("communication_style = \"terse\""));
        assert!(!out.contains("old verbose style"));
    }

    #[test]
    fn profile_hint_list_appends_new_entry() {
        let mut doc = parse(
            "[profile]\n\
             behavioral_preferences = [\"cite sources\"]\n",
        );
        let hint = ProfileFieldHint {
            field: ProfileField::BehavioralPreferences,
            suggested_value: "use code blocks".into(),
            rationale: "...".into(),
        };
        apply_profile_hint_to_doc(&mut doc, &hint);
        let out = doc.to_string();
        assert!(out.contains("\"cite sources\""));
        assert!(out.contains("\"use code blocks\""));
    }

    #[test]
    fn profile_hint_list_apply_is_idempotent() {
        let mut doc = parse(
            "[profile]\n\
             behavioral_preferences = [\"cite sources\"]\n",
        );
        let hint = ProfileFieldHint {
            field: ProfileField::BehavioralPreferences,
            suggested_value: "cite sources".into(),
            rationale: "...".into(),
        };
        apply_profile_hint_to_doc(&mut doc, &hint);
        let out = doc.to_string();
        // The single existing entry remains; no duplicate.
        let occurrences = out.matches("\"cite sources\"").count();
        assert_eq!(occurrences, 1, "idempotent append left {out}");
    }

    #[test]
    fn profile_hint_creates_list_field_when_absent() {
        let mut doc = parse("[profile]\n");
        let hint = ProfileFieldHint {
            field: ProfileField::PrimaryUseCases,
            suggested_value: "oncall investigations".into(),
            rationale: "...".into(),
        };
        apply_profile_hint_to_doc(&mut doc, &hint);
        let out = doc.to_string();
        assert!(out.contains("primary_use_cases"));
        assert!(out.contains("\"oncall investigations\""));
    }

    #[test]
    fn profile_hint_preserves_other_sections_and_comments() {
        let mut doc = parse(
            "# Top-level comment.\n\
             \n\
             [profile]\n\
             # Operator-declared identity.\n\
             assistant_name = \"Aivyx\"\n\
             \n\
             [roles.coder]\n\
             tool_allowlist = [\"fs.read\"]\n",
        );
        let hint = ProfileFieldHint {
            field: ProfileField::CommunicationStyle,
            suggested_value: "terse".into(),
            rationale: "...".into(),
        };
        apply_profile_hint_to_doc(&mut doc, &hint);
        let out = doc.to_string();
        // Top-level comment retained.
        assert!(out.contains("# Top-level comment."));
        // Profile comment retained.
        assert!(out.contains("# Operator-declared identity."));
        // Other section retained.
        assert!(out.contains("[roles.coder]"));
        assert!(out.contains("\"fs.read\""));
        // The scalar still set.
        assert!(out.contains("communication_style = \"terse\""));
    }

    // ----- RoleDraft apply (pure transform) -----

    #[test]
    fn role_draft_adds_section_to_empty_document() {
        let mut doc = parse("");
        let role = RoleDraft {
            name: "research-deploy".into(),
            parent: Some("research".into()),
            system_prompt_addendum: "After research, summarize the diff.".into(),
            tool_allowlist_additions: vec![
                "git.commit".into(),
                "shell.deploy".into(),
            ],
            rationale: "...".into(),
        };
        let applied =
            apply_role_draft_to_doc(&mut doc, &role, false, "test.toml")
                .expect("applies");
        assert_eq!(applied.role_name, "research-deploy");
        assert_eq!(applied.parent.as_deref(), Some("research"));
        let out = doc.to_string();
        assert!(out.contains("[roles.research-deploy]"));
        assert!(out.contains("inherits_from = \"research\""));
        assert!(out.contains("system_prompt = \"After research, summarize the diff.\""));
        assert!(out.contains("\"git.commit\""));
        assert!(out.contains("\"shell.deploy\""));
    }

    #[test]
    fn role_draft_omits_inherits_from_for_top_level_role() {
        let mut doc = parse("");
        let role = RoleDraft {
            name: "operator-mode".into(),
            parent: None,
            system_prompt_addendum: "Direct operator mode.".into(),
            tool_allowlist_additions: vec![],
            rationale: "...".into(),
        };
        let applied =
            apply_role_draft_to_doc(&mut doc, &role, false, "test.toml")
                .expect("applies");
        assert_eq!(applied.parent, None);
        let out = doc.to_string();
        assert!(out.contains("[roles.operator-mode]"));
        assert!(!out.contains("inherits_from"));
    }

    #[test]
    fn role_draft_omits_tool_allowlist_when_empty() {
        let mut doc = parse("");
        let role = RoleDraft {
            name: "prompt-only".into(),
            parent: Some("base".into()),
            system_prompt_addendum: "Just a system prompt extension.".into(),
            tool_allowlist_additions: vec![],
            rationale: "...".into(),
        };
        apply_role_draft_to_doc(&mut doc, &role, false, "test.toml")
            .expect("applies");
        let out = doc.to_string();
        assert!(out.contains("system_prompt"));
        assert!(!out.contains("tool_allowlist"));
    }

    #[test]
    fn role_draft_refuses_to_overwrite_existing_role_without_force() {
        let mut doc = parse(
            "[roles.research-deploy]\n\
             tool_allowlist = [\"git.status\"]\n",
        );
        let role = RoleDraft {
            name: "research-deploy".into(),
            parent: None,
            system_prompt_addendum: "new".into(),
            tool_allowlist_additions: vec![],
            rationale: "...".into(),
        };
        let err =
            apply_role_draft_to_doc(&mut doc, &role, false, "aivyx.toml")
                .unwrap_err();
        match err {
            TomlApplyError::RoleExists { name, path } => {
                assert_eq!(name, "research-deploy");
                assert_eq!(path, "aivyx.toml");
            }
            other => panic!("expected RoleExists, got {other:?}"),
        }
        // The original section MUST stay intact — the refusal
        // didn't mutate anything.
        let out = doc.to_string();
        assert!(out.contains("\"git.status\""));
    }

    #[test]
    fn role_draft_overwrites_existing_role_with_force() {
        let mut doc = parse(
            "[roles.research-deploy]\n\
             tool_allowlist = [\"git.status\"]\n",
        );
        let role = RoleDraft {
            name: "research-deploy".into(),
            parent: Some("research".into()),
            system_prompt_addendum: "new".into(),
            tool_allowlist_additions: vec!["git.commit".into()],
            rationale: "...".into(),
        };
        apply_role_draft_to_doc(&mut doc, &role, true, "aivyx.toml")
            .expect("applies with force");
        let out = doc.to_string();
        assert!(out.contains("inherits_from = \"research\""));
        assert!(out.contains("\"git.commit\""));
        // The pre-existing tool_allowlist value gone (force = full replace).
        assert!(!out.contains("\"git.status\""));
    }

    #[test]
    fn role_draft_preserves_other_sections() {
        let mut doc = parse(
            "[profile]\n\
             assistant_name = \"Aivyx\"\n\
             \n\
             [roles.coder]\n\
             tool_allowlist = [\"fs.read\"]\n",
        );
        let role = RoleDraft {
            name: "researcher".into(),
            parent: None,
            system_prompt_addendum: "Research mode.".into(),
            tool_allowlist_additions: vec!["web.fetch".into()],
            rationale: "...".into(),
        };
        apply_role_draft_to_doc(&mut doc, &role, false, "test.toml")
            .expect("applies");
        let out = doc.to_string();
        assert!(out.contains("[profile]"));
        assert!(out.contains("\"Aivyx\""));
        assert!(out.contains("[roles.coder]"));
        assert!(out.contains("\"fs.read\""));
        assert!(out.contains("[roles.researcher]"));
        assert!(out.contains("\"web.fetch\""));
    }

    // ----- Atomic I/O wrapper -----

    fn tempdir(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "aivyx-phase119-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn apply_profile_hint_to_path_creates_file_when_absent() {
        let dir = tempdir("create");
        let path = dir.join("aivyx.toml");
        assert!(!path.exists());
        let hint = ProfileFieldHint {
            field: ProfileField::CommunicationStyle,
            suggested_value: "terse".into(),
            rationale: "...".into(),
        };
        let applied = apply_profile_hint_to_path(&path, &hint).expect("applies");
        assert_eq!(applied.field, "communication_style");
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("communication_style = \"terse\""));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_profile_hint_to_path_is_atomic_via_rename() {
        // The tmp-file + rename pattern leaves no debris behind
        // after a successful apply. Concretely: walk the parent
        // directory after the apply and assert only `aivyx.toml`
        // exists (no `.aivyx.toml.phase119-tmp.*` survivor).
        let dir = tempdir("atomic");
        let path = dir.join("aivyx.toml");
        std::fs::write(&path, "[profile]\n").unwrap();
        let hint = ProfileFieldHint {
            field: ProfileField::AssistantName,
            suggested_value: "Aivyx".into(),
            rationale: "...".into(),
        };
        apply_profile_hint_to_path(&path, &hint).expect("applies");
        let entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(entries, vec!["aivyx.toml".to_string()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[cfg(unix)]
    #[test]
    fn apply_profile_hint_to_path_sets_0600_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir("perms");
        let path = dir.join("aivyx.toml");
        let hint = ProfileFieldHint {
            field: ProfileField::AssistantName,
            suggested_value: "Aivyx".into(),
            rationale: "...".into(),
        };
        apply_profile_hint_to_path(&path, &hint).expect("applies");
        let meta = std::fs::metadata(&path).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "atomic write must preserve 0600 perms");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn apply_role_draft_to_path_refuses_overwrite_without_force() {
        let dir = tempdir("role-refuse");
        let path = dir.join("aivyx.toml");
        std::fs::write(
            &path,
            "[roles.research-deploy]\ntool_allowlist = [\"git.status\"]\n",
        )
        .unwrap();
        let role = RoleDraft {
            name: "research-deploy".into(),
            parent: None,
            system_prompt_addendum: "...".into(),
            tool_allowlist_additions: vec![],
            rationale: "...".into(),
        };
        let err = apply_role_draft_to_path(&path, &role, false).unwrap_err();
        assert!(matches!(err, TomlApplyError::RoleExists { .. }));
        // The original file is untouched.
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("\"git.status\""));
        std::fs::remove_dir_all(&dir).ok();
    }
}
