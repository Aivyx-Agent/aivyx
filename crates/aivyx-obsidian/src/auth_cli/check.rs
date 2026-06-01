//! `aivyx-obsidian auth check` — verify the configured
//! vault path is usable.

use std::path::PathBuf;

use crate::auth_cli::config_file::{load_config, ConfigFileError};
use crate::{VaultClient, VaultError};

#[derive(Debug)]
pub struct CheckReport {
    pub config_path: PathBuf,
    pub vault_ok: bool,
    pub message: String,
}

impl std::fmt::Display for CheckReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.vault_ok {
            writeln!(
                f,
                "aivyx-obsidian auth check: OK\n  config: {:?}\n  {}",
                self.config_path, self.message
            )
        } else {
            writeln!(
                f,
                "aivyx-obsidian auth check: FAIL\n  config: {:?}\n  {}",
                self.config_path, self.message
            )
        }
    }
}

pub fn run_auth_check(config_path: &std::path::Path) -> CheckReport {
    let cfg = match load_config(config_path) {
        Ok(c) => c,
        Err(e) => {
            return CheckReport {
                config_path: config_path.to_path_buf(),
                vault_ok: false,
                message: format!("config load failed: {e}"),
            };
        }
    };
    match VaultClient::new(cfg.clone()) {
        Ok(client) => CheckReport {
            config_path: config_path.to_path_buf(),
            vault_ok: true,
            message: format!("vault root: {:?}", client.vault_root()),
        },
        Err(VaultError::VaultRootMissing(p)) => CheckReport {
            config_path: config_path.to_path_buf(),
            vault_ok: false,
            message: format!(
                "vault path does not exist or is not a directory: {p:?}"
            ),
        },
        Err(e) => CheckReport {
            config_path: config_path.to_path_buf(),
            vault_ok: false,
            message: format!("vault open failed: {e}"),
        },
    }
}

// Implement Clone for VaultConfig so the check can clone
// it cheaply.
impl Clone for ConfigFileError {
    fn clone(&self) -> Self {
        match self {
            Self::NotFound { path } => Self::NotFound { path: path.clone() },
            Self::Io { path, source } => Self::Io {
                path: path.clone(),
                source: std::io::Error::new(source.kind(), source.to_string()),
            },
            Self::Parse { path, reason } => Self::Parse {
                path: path.clone(),
                reason: reason.clone(),
            },
            Self::NotAbsolute { path, got } => Self::NotAbsolute {
                path: path.clone(),
                got: got.clone(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_report_display_distinguishes_ok_and_fail() {
        let ok = CheckReport {
            config_path: "/tmp/x".into(),
            vault_ok: true,
            message: "vault root: /vault".into(),
        };
        let fail = CheckReport {
            config_path: "/tmp/x".into(),
            vault_ok: false,
            message: "sad".into(),
        };
        assert!(ok.to_string().contains("OK"));
        assert!(fail.to_string().contains("FAIL"));
    }

    #[test]
    fn run_auth_check_surfaces_config_load_failure() {
        let report = run_auth_check(std::path::Path::new("/nope/missing.toml"));
        assert!(!report.vault_ok);
        assert!(report.message.contains("config load failed"));
    }
}
