//! `aivyx access` — the operator-facing access-level Settings command
//! (Chapter N). `show` prints the resolved access level, the fs-root reach
//! it derives, and the confirm-first posture; `set <level>` rewrites the
//! `[access]` section of `aivyx.toml` (re-confirming the expanded levels).
//!
//! Like `aivyx profile`, these are synchronous file operations — no daemon,
//! no passphrase, no API key. The change takes effect on the next daemon
//! start (access level is load-time, same as roles).

use std::io::{BufRead, Write};
use std::path::Path;

use aivyx_config::{AccessLevel, AivyxConfig, FieldSource, LoadOptions};
use toml_edit::{value, DocumentMut};

/// Module-local copy of the default config path (mirrors
/// [`crate::DEFAULT_TOML_PATH`] without coupling to it, same as the other
/// subcommand modules).
const ACCESS_TOML_PATH: &str = "aivyx.toml";

/// `aivyx access show` — print the current access level + resolved reach.
pub fn run_access_show() -> Result<(), String> {
    let cfg = load_config_for_inspection()?;
    print!("{}", render_access_for_show(&cfg));
    Ok(())
}

/// `aivyx access set <level> [--root <dir>] [--yes]` — rewrite the
/// `[access]` section. `workspace`/`custom` require `--root`; expanded
/// levels (anything but `sandbox`) require a confirmation unless `--yes`.
pub fn run_access_set(
    level: AccessLevel,
    root: Option<String>,
    yes: bool,
) -> Result<(), String> {
    if matches!(level, AccessLevel::Workspace | AccessLevel::Custom) && root.is_none() {
        return Err(format!(
            "access level `{level}` needs a directory — pass `--root <dir>`."
        ));
    }
    if matches!(level, AccessLevel::Sandbox | AccessLevel::Home | AccessLevel::Full)
        && root.is_some()
    {
        return Err(format!(
            "access level `{level}` derives its root automatically — `--root` \
             only applies to `workspace`/`custom`."
        ));
    }

    // Expanded levels are deliberate: confirm before granting (unless --yes).
    if level.is_expanded() && !yes {
        let warning = match level {
            AccessLevel::Full => {
                " — this grants access to the ENTIRE filesystem, including \
                 system files"
            }
            AccessLevel::Home => " — this grants full read/write/shell over your home directory",
            _ => "",
        };
        if !confirm(&format!("Grant '{level}' access to the agent{warning}?"))? {
            return Err("aborted — access level unchanged.".into());
        }
    }

    let path = Path::new(ACCESS_TOML_PATH);
    let original = if path.exists() {
        std::fs::read_to_string(path)
            .map_err(|e| format!("failed to read {}: {e}", path.display()))?
    } else {
        String::new()
    };
    let mut doc = original
        .parse::<DocumentMut>()
        .map_err(|e| format!("failed to parse {} as TOML: {e}", path.display()))?;

    doc["access"]["level"] = value(level.as_str());
    match &root {
        Some(r) => doc["access"]["root"] = value(r.as_str()),
        // Switching to a level that derives its root: drop any stale
        // `[access] root` so it doesn't shadow the derivation.
        None => {
            if let Some(t) = doc.get_mut("access").and_then(|a| a.as_table_mut()) {
                t.remove("root");
            }
        }
    }
    doc["access"]["confirm_destructive"] = value(level.is_expanded());

    write_aivyx_toml(path, &doc.to_string())?;

    // A stray `[fs] root` would override the level-derived reach — warn so
    // the operator isn't surprised that `set home` didn't widen anything.
    if level.is_expanded()
        && doc
            .get("fs")
            .and_then(|f| f.get("root"))
            .is_some()
    {
        eprintln!(
            "  \u{26a0} note: an explicit `[fs] root` is still present and \
             overrides the `{level}` level — remove it to use the derived root."
        );
    }

    eprintln!("Access level set to `{level}` in {}.", path.display());
    eprintln!(
        "Restart the daemon for the change to take effect: \
         `aivyx daemon stop && aivyx daemon run`."
    );
    Ok(())
}

/// Parse the `<level>` token of `aivyx access set`.
pub fn parse_level(s: &str) -> Result<AccessLevel, String> {
    match s {
        "sandbox" => Ok(AccessLevel::Sandbox),
        "workspace" => Ok(AccessLevel::Workspace),
        "home" => Ok(AccessLevel::Home),
        "full" => Ok(AccessLevel::Full),
        "custom" => Ok(AccessLevel::Custom),
        other => Err(format!(
            "unknown access level `{other}`. \
             Supported: sandbox, workspace, home, full, custom"
        )),
    }
}

fn render_access_for_show(cfg: &AivyxConfig) -> String {
    let src = |s: FieldSource| match s {
        FieldSource::Default => "default",
        FieldSource::Toml => "aivyx.toml",
        FieldSource::Env => "env",
        FieldSource::EncryptedStore => "encrypted-store",
    };
    let mut out = String::new();
    out.push_str("aivyx access:\n");
    out.push_str(&format!(
        "  level              = {} ({})\n",
        cfg.access_level.value,
        src(cfg.access_level.source),
    ));
    out.push_str(&format!(
        "  reach (fs_root)    = {} ({})\n",
        cfg.fs_root.value.display(),
        src(cfg.fs_root.source),
    ));
    out.push_str(&format!(
        "  confirm_destructive = {} ({})\n",
        cfg.confirm_destructive.value,
        src(cfg.confirm_destructive.source),
    ));
    out.push_str(
        "  (the access level applies to the local operator; remote channels \
         stay tier-attenuated)\n",
    );
    out
}

/// Prompt the operator for a yes/no on stdin; defaults to NO on empty input.
fn confirm(question: &str) -> Result<bool, String> {
    print!("{question} [y/N]: ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| format!("failed to read confirmation: {e}"))?;
    Ok(matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes"))
}

fn load_config_for_inspection() -> Result<AivyxConfig, String> {
    let opts = LoadOptions {
        toml_path: Some(Path::new(ACCESS_TOML_PATH).to_path_buf()),
        require_api_key: false,
        require_telegram_token: false,
        require_discord_token: false,
        require_slack_tokens: false,
        role_override: None,
    };
    AivyxConfig::load_from_env_and_toml(&opts)
        .map_err(|e| format!("failed to load {ACCESS_TOML_PATH}: {e}"))
}

fn write_aivyx_toml(path: &Path, contents: &str) -> Result<(), String> {
    std::fs::write(path, contents)
        .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    // Match the 0600 posture the other config writers use (the file may
    // carry secrets in other sections).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(path, perms)
            .map_err(|e| format!("failed to set permissions on {}: {e}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_level_accepts_known_levels() {
        assert_eq!(parse_level("home").unwrap(), AccessLevel::Home);
        assert_eq!(parse_level("full").unwrap(), AccessLevel::Full);
        assert_eq!(parse_level("sandbox").unwrap(), AccessLevel::Sandbox);
        assert!(parse_level("bogus").is_err());
    }

    #[test]
    fn set_workspace_requires_root() {
        let err = run_access_set(AccessLevel::Workspace, None, true).unwrap_err();
        assert!(err.contains("--root"), "got: {err}");
    }

    #[test]
    fn set_home_rejects_explicit_root() {
        let err = run_access_set(AccessLevel::Home, Some("/x".into()), true).unwrap_err();
        assert!(err.contains("derives its root"), "got: {err}");
    }

    #[test]
    fn set_writes_access_section_and_drops_stale_root() {
        // `set home --yes` writes [access] level=home + confirm, and
        // removes a stale [access] root, in an isolated temp cwd.
        let dir = std::env::temp_dir().join(format!("aivyx-access-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let toml = dir.join("aivyx.toml");
        std::fs::write(&toml, "[access]\nlevel = \"workspace\"\nroot = \"/old\"\n").unwrap();
        let prev = std::env::current_dir().unwrap();
        std::env::set_current_dir(&dir).unwrap();
        let res = run_access_set(AccessLevel::Home, None, true);
        std::env::set_current_dir(prev).unwrap();
        res.unwrap();
        let written = std::fs::read_to_string(&toml).unwrap();
        assert!(written.contains("level = \"home\""), "{written}");
        assert!(written.contains("confirm_destructive = true"), "{written}");
        assert!(!written.contains("/old"), "stale root must be dropped: {written}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
