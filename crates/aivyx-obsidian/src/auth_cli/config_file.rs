//! Config file loading for `aivyx-obsidian`.

use std::io;
use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::VaultConfig;

#[derive(Debug, Error)]
pub enum ConfigFileError {
    #[error("config file at {path:?} not found — create it with `vault_path = \"/absolute/path/to/MyVault\"`. See `aivyx-obsidian help`.")]
    NotFound { path: PathBuf },
    #[error("I/O error reading {path:?}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("parse error in {path:?}: {reason}")]
    Parse { path: PathBuf, reason: String },
    #[error("`vault_path` in {path:?} must be an absolute path (got: {got:?})")]
    NotAbsolute { path: PathBuf, got: PathBuf },
}

pub fn default_config_path() -> Result<PathBuf, ConfigFileError> {
    crate::default_config_path().ok_or_else(|| ConfigFileError::NotFound {
        path: PathBuf::from("~/.aivyx/tool-processes/obsidian/config.toml"),
    })
}

pub fn load_config(path: &Path) -> Result<VaultConfig, ConfigFileError> {
    let body = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Err(ConfigFileError::NotFound {
                path: path.to_path_buf(),
            });
        }
        Err(e) => {
            return Err(ConfigFileError::Io {
                path: path.to_path_buf(),
                source: e,
            });
        }
    };
    let cfg: VaultConfig = toml::from_str(&body).map_err(|e| ConfigFileError::Parse {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;
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

    #[test]
    fn load_config_returns_not_found_for_missing_path() {
        let e = load_config(Path::new("/totally/nope")).expect_err("must error");
        assert!(matches!(e, ConfigFileError::NotFound { .. }));
    }

    #[test]
    fn load_config_returns_parse_for_malformed_toml() {
        let path = tmpfile("not = valid = toml ==");
        let e = load_config(&path).expect_err("must error");
        assert!(matches!(e, ConfigFileError::Parse { .. }));
        let _ = std::fs::remove_file(&path);
    }

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
}
