//! Config file loading for `aivyx-obsidian`.
//!
//! Thin wrapper over `aivyx_auth_cli::load_toml` —
//! Phase 132 lift. Service-specific validation
//! (the vault path must be absolute, so the
//! path-traversal guard has a stable canonical root)
//! lives here.

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::VaultConfig;

const SERVICE_SUBDIR: &str = "obsidian";

/// Composes the substrate's IO/parse errors with
/// Obsidian-specific validation errors. See
/// `aivyx-notion::auth_cli::config_file` for the
/// shape rationale.
#[derive(Debug, Error)]
pub enum ConfigFileError {
    /// IO / parse error from `aivyx-auth-cli`.
    #[error(transparent)]
    Substrate(#[from] aivyx_auth_cli::ConfigFileError),

    /// The `vault_path` field deserialised but is not an
    /// absolute path. Load-bearing for the path-
    /// traversal guard (`VaultClient::resolve_under_vault`):
    /// a relative root would change behaviour based on
    /// the binary's CWD at canonicalize time, which is
    /// not what operators want.
    #[error("`vault_path` in {path:?} must be an absolute path (got: {got:?})")]
    NotAbsolute { path: PathBuf, got: PathBuf },
}

pub fn default_config_path() -> Result<PathBuf, ConfigFileError> {
    aivyx_auth_cli::default_config_path(SERVICE_SUBDIR).ok_or_else(|| {
        ConfigFileError::Substrate(aivyx_auth_cli::ConfigFileError::NotFound {
            path: PathBuf::from(format!(
                "~/.aivyx/tool-processes/{SERVICE_SUBDIR}/config.toml"
            )),
        })
    })
}

pub fn load_config(path: &Path) -> Result<VaultConfig, ConfigFileError> {
    let cfg: VaultConfig = aivyx_auth_cli::load_toml(path)?;
    if !cfg.vault_path.is_absolute() {
        return Err(ConfigFileError::NotAbsolute {
            path: path.to_path_buf(),
            got: cfg.vault_path.clone(),
        });
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmpfile(body: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "aivyx-obs-cfg-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        path
    }

    // Substrate behaviour (NotFound on missing path,
    // Parse on malformed TOML) is tested in
    // aivyx-auth-cli directly. These tests cover
    // Obsidian-specific validation: absolute-path
    // enforcement.

    #[test]
    fn load_config_rejects_relative_vault_path() {
        let path = tmpfile(r#"vault_path = "relative/path/MyVault""#);
        let e = load_config(&path).expect_err("must error");
        assert!(matches!(e, ConfigFileError::NotAbsolute { .. }));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_config_accepts_absolute_vault_path() {
        let path = tmpfile(r#"vault_path = "/tmp/MyVault""#);
        let cfg = load_config(&path).expect("ok");
        assert_eq!(cfg.vault_path, PathBuf::from("/tmp/MyVault"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn substrate_not_found_flows_through_transparently() {
        let e = load_config(Path::new("/totally/nope")).expect_err("must error");
        assert!(matches!(
            e,
            ConfigFileError::Substrate(aivyx_auth_cli::ConfigFileError::NotFound { .. })
        ));
    }

    #[test]
    fn not_absolute_error_message_includes_offender_and_path() {
        let e = ConfigFileError::NotAbsolute {
            path: PathBuf::from("/tmp/c.toml"),
            got: PathBuf::from("relative/x"),
        };
        let s = e.to_string();
        assert!(s.contains("/tmp/c.toml"));
        assert!(s.contains("relative/x"));
    }
}
