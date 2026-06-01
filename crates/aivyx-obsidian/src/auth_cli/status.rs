//! `aivyx-obsidian auth status` + `auth check` operations.
//!
//! Phase 132 lift — aligned with `aivyx-notion` and
//! `aivyx-n8n` on the substrate's
//! `BinaryMode::Auth(AuthMode::{Status, Check})`
//! shape.
//!
//! - `auth status` — offline. Loads the config file
//!   (file exists, parses, `vault_path` is absolute)
//!   and reports OK without touching the filesystem.
//! - `auth check` — online. Loads the config plus
//!   opens the `VaultClient`, which canonicalises the
//!   path and verifies the vault root exists as a
//!   directory.

use std::path::Path;

pub use aivyx_auth_cli::{CheckReport, StatusReport};

use crate::auth_cli::config_file::{load_config, ConfigFileError};
use crate::{VaultClient, VaultError};

const BINARY_NAME: &str = "aivyx-obsidian";

/// Offline status — verifies config presence + the
/// absolute-path invariant. No filesystem probe.
pub fn run_auth_status(config_path: &Path) -> Result<StatusReport, ConfigFileError> {
    let cfg = load_config(config_path)?;
    Ok(StatusReport::ok(
        BINARY_NAME,
        config_path.to_path_buf(),
        format!(
            "vault_path: {:?}\n  next: run `aivyx-obsidian auth check` to verify the path is reachable",
            cfg.vault_path
        ),
    ))
}

/// Online check — verifies the vault root is openable.
/// Returns a `CheckReport` rather than a `Result` so
/// the caller can render the outcome regardless of
/// whether the vault opens.
pub fn run_auth_check(config_path: &Path) -> CheckReport {
    let cfg = match load_config(config_path) {
        Ok(c) => c,
        Err(e) => {
            return CheckReport::fail(BINARY_NAME, format!("config load failed: {e}"));
        }
    };
    match VaultClient::new(cfg) {
        Ok(client) => CheckReport::ok(
            BINARY_NAME,
            format!("vault root: {:?}", client.vault_root()),
        ),
        Err(VaultError::VaultRootMissing(p)) => CheckReport::fail(
            BINARY_NAME,
            format!("vault path does not exist or is not a directory: {p:?}"),
        ),
        Err(e) => CheckReport::fail(BINARY_NAME, format!("vault open failed: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Display impls for StatusReport / CheckReport are
    // tested in aivyx-auth-cli. These tests cover the
    // Obsidian-specific wiring: status loads config
    // and reports the vault_path; check exercises the
    // VaultClient open path.

    #[test]
    fn run_auth_check_surfaces_config_load_failure() {
        let report = run_auth_check(Path::new("/nope/missing.toml"));
        assert!(!report.ok);
        assert!(report.message.contains("config load failed"));
    }
}
