//! Obsidian vault client — filesystem operations rooted
//! at a configured vault directory with **load-bearing
//! path-traversal protection**.
//!
//! Phase 130 Task 10. The Obsidian integration is unlike
//! the other Chapter F integrations because there's no
//! external API to call — every operation is a
//! filesystem read/write under the operator's vault
//! directory.
//!
//! ## The path-traversal guard is load-bearing for safety
//!
//! Every tool operation that touches the filesystem MUST
//! resolve its operator-supplied path via
//! [`VaultClient::resolve_under_vault`] before any I/O.
//! The guard:
//!
//! 1. Rejects absolute operator paths (the vault root is
//!    the only absolute path allowed; operators supply
//!    relative-to-vault paths).
//! 2. Joins the operator's relative path to the vault
//!    root.
//! 3. Canonicalizes the result (resolves `.`, `..`, and
//!    symlinks).
//! 4. Verifies the canonical result is still under the
//!    vault root. If a `..` or symlink would escape, the
//!    canonical path won't have the vault-root prefix and
//!    the resolver returns
//!    [`VaultError::PathEscapesVault`].
//!
//! For paths that don't exist yet (e.g.,
//! `obsidian.create_note`), canonicalization fails on the
//! leaf segment. The guard canonicalizes the parent
//! directory instead and verifies the parent is under the
//! vault root.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

/// File extension Obsidian uses for notes. Tools that
/// operate on "notes" check this extension to filter out
/// `.canvas` / `.excalidraw` / images / etc.
pub const MARKDOWN_EXT: &str = "md";

#[derive(Debug, Error)]
pub enum VaultError {
    #[error("vault root not configured — set `vault_path` in config.toml")]
    NoVaultConfigured,
    #[error("vault root does not exist or is not readable: {0:?}")]
    VaultRootMissing(PathBuf),
    #[error("path must be relative to the vault root (got absolute path: {0:?})")]
    AbsolutePathRejected(PathBuf),
    #[error("path escapes the vault root: {0:?}")]
    PathEscapesVault(PathBuf),
    #[error("I/O error on {path:?}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("note not found: {0:?}")]
    NoteNotFound(PathBuf),
    #[error("invalid path: {0}")]
    InvalidPath(String),
}

/// Operator-supplied configuration.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct VaultConfig {
    /// Absolute path to the Obsidian vault directory.
    /// Operators set this in config.toml as
    /// `vault_path = "/Users/me/Documents/MyVault"`.
    pub vault_path: PathBuf,
}

/// Vault-rooted filesystem client. Every operation goes
/// through [`Self::resolve_under_vault`] before any I/O.
#[derive(Debug)]
pub struct VaultClient {
    /// Canonicalized vault root. Stored canonicalized so
    /// the prefix check in `resolve_under_vault` is
    /// reliable.
    vault_root: PathBuf,
}

impl VaultClient {
    /// Build a `VaultClient` from a config-supplied vault
    /// path. Canonicalizes the path up front so subsequent
    /// `resolve_under_vault` calls compare against a
    /// consistent root.
    pub fn new(config: VaultConfig) -> Result<Self, VaultError> {
        let path = config.vault_path;
        if !path.exists() {
            return Err(VaultError::VaultRootMissing(path));
        }
        let canonical = std::fs::canonicalize(&path).map_err(|source| VaultError::Io {
            path: path.clone(),
            source,
        })?;
        if !canonical.is_dir() {
            return Err(VaultError::VaultRootMissing(canonical));
        }
        Ok(Self {
            vault_root: canonical,
        })
    }

    /// Test-only constructor for tests that already have a
    /// canonicalized root.
    #[cfg(test)]
    pub fn from_canonical_root(root: PathBuf) -> Self {
        Self { vault_root: root }
    }

    pub fn vault_root(&self) -> &Path {
        &self.vault_root
    }

    /// **Load-bearing path-traversal guard.** Resolves an
    /// operator-supplied relative path to an absolute
    /// path under the vault root, rejecting any input
    /// that would escape via `..` or symlink. Used by
    /// every tool operation that touches the filesystem.
    ///
    /// `must_exist` controls how non-existent leaves are
    /// handled:
    /// - `true` — the resolved path must exist (used by
    ///   read tools like `obsidian.get_note`).
    /// - `false` — the leaf may not exist yet; only the
    ///   parent directory is canonicalized + checked
    ///   against the vault root (used by write tools
    ///   like `obsidian.create_note`).
    pub fn resolve_under_vault(
        &self,
        relative: &str,
        must_exist: bool,
    ) -> Result<PathBuf, VaultError> {
        let raw = PathBuf::from(relative);
        if raw.is_absolute() {
            return Err(VaultError::AbsolutePathRejected(raw));
        }
        // Reject any component that's literally `..` even
        // BEFORE canonicalization. Canonicalization would
        // resolve them, but rejecting at the input layer
        // makes the security posture more auditable —
        // operators see "rejected `..`" rather than
        // "rejected by canonicalization."
        for comp in raw.components() {
            if matches!(comp, std::path::Component::ParentDir) {
                return Err(VaultError::PathEscapesVault(raw));
            }
            if matches!(comp, std::path::Component::Prefix(_) | std::path::Component::RootDir)
            {
                return Err(VaultError::AbsolutePathRejected(raw));
            }
        }
        let joined = self.vault_root.join(&raw);

        if must_exist {
            let canonical = std::fs::canonicalize(&joined).map_err(|source| match source.kind() {
                std::io::ErrorKind::NotFound => VaultError::NoteNotFound(raw.clone()),
                _ => VaultError::Io {
                    path: raw.clone(),
                    source,
                },
            })?;
            if !canonical.starts_with(&self.vault_root) {
                return Err(VaultError::PathEscapesVault(raw));
            }
            Ok(canonical)
        } else {
            // For create/write tools, canonicalize the
            // PARENT directory and verify it's under the
            // vault root. The leaf may not exist yet.
            let parent = joined
                .parent()
                .ok_or_else(|| VaultError::InvalidPath("path has no parent".into()))?;
            // If the parent doesn't exist, we still need
            // to ensure no `..` is hiding inside the path
            // we're about to create. The `..` check above
            // already caught those; here we just verify
            // the parent (if it exists) is under the
            // vault root.
            if parent.exists() {
                let canonical_parent =
                    std::fs::canonicalize(parent).map_err(|source| VaultError::Io {
                        path: parent.to_path_buf(),
                        source,
                    })?;
                if !canonical_parent.starts_with(&self.vault_root) {
                    return Err(VaultError::PathEscapesVault(raw));
                }
                Ok(canonical_parent.join(joined.file_name().ok_or_else(|| {
                    VaultError::InvalidPath("path has no filename".into())
                })?))
            } else {
                // Parent doesn't exist yet — `..` check
                // above already protects against escape.
                Ok(joined)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_vault() -> PathBuf {
        // Per-process monotonic counter: pid + nanos alone can collide
        // when two tests build a path in the same clock tick, and one
        // test's `remove_dir_all` cleanup would then delete another's
        // vault (Phase 185 flaky-test isolation fix).
        static SEQ: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "aivyx-obs-vault-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        ));
        fs::create_dir_all(&path).unwrap();
        std::fs::canonicalize(&path).unwrap()
    }

    #[test]
    fn new_rejects_missing_vault_path() {
        let cfg = VaultConfig {
            vault_path: PathBuf::from("/totally/does/not/exist/aivyx-test"),
        };
        let e = VaultClient::new(cfg).expect_err("must error");
        assert!(matches!(e, VaultError::VaultRootMissing(_)));
    }

    #[test]
    fn new_rejects_non_directory_path() {
        let path = tmp_vault();
        // Create a file at the path location instead of dir.
        let file = path.join("not-a-dir.txt");
        fs::write(&file, "x").unwrap();
        let cfg = VaultConfig {
            vault_path: file.clone(),
        };
        let e = VaultClient::new(cfg).expect_err("must error");
        assert!(matches!(e, VaultError::VaultRootMissing(_)));
        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn new_accepts_existing_directory() {
        let path = tmp_vault();
        let client = VaultClient::new(VaultConfig {
            vault_path: path.clone(),
        })
        .expect("ok");
        assert_eq!(client.vault_root(), path.as_path());
        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn resolve_under_vault_accepts_simple_relative_path() {
        let path = tmp_vault();
        fs::write(path.join("note.md"), "hi").unwrap();
        let client = VaultClient::from_canonical_root(path.clone());
        let resolved = client
            .resolve_under_vault("note.md", true)
            .expect("ok");
        assert!(resolved.starts_with(&path));
        assert!(resolved.ends_with("note.md"));
        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn resolve_under_vault_accepts_subdirectory_path() {
        let path = tmp_vault();
        fs::create_dir(path.join("subfolder")).unwrap();
        fs::write(path.join("subfolder/note.md"), "hi").unwrap();
        let client = VaultClient::from_canonical_root(path.clone());
        let resolved = client
            .resolve_under_vault("subfolder/note.md", true)
            .expect("ok");
        assert!(resolved.starts_with(&path));
        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn resolve_under_vault_rejects_dotdot_at_input_layer() {
        let path = tmp_vault();
        let client = VaultClient::from_canonical_root(path.clone());
        let e = client
            .resolve_under_vault("../outside.md", true)
            .expect_err("must error");
        assert!(matches!(e, VaultError::PathEscapesVault(_)));
        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn resolve_under_vault_rejects_dotdot_in_middle() {
        let path = tmp_vault();
        let client = VaultClient::from_canonical_root(path.clone());
        let e = client
            .resolve_under_vault("subfolder/../../outside.md", true)
            .expect_err("must error");
        assert!(matches!(e, VaultError::PathEscapesVault(_)));
        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn resolve_under_vault_rejects_absolute_path() {
        let path = tmp_vault();
        let client = VaultClient::from_canonical_root(path.clone());
        let e = client
            .resolve_under_vault("/etc/passwd", true)
            .expect_err("must error");
        assert!(matches!(e, VaultError::AbsolutePathRejected(_)));
        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn resolve_under_vault_returns_not_found_for_missing_leaf_in_read_mode() {
        let path = tmp_vault();
        let client = VaultClient::from_canonical_root(path.clone());
        let e = client
            .resolve_under_vault("missing-note.md", true)
            .expect_err("must error");
        assert!(matches!(e, VaultError::NoteNotFound(_)));
        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn resolve_under_vault_accepts_nonexistent_leaf_in_write_mode() {
        let path = tmp_vault();
        let client = VaultClient::from_canonical_root(path.clone());
        let resolved = client
            .resolve_under_vault("new-note.md", false)
            .expect("ok");
        assert!(resolved.starts_with(&path));
        assert!(resolved.ends_with("new-note.md"));
        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn resolve_under_vault_rejects_dotdot_in_write_mode_too() {
        let path = tmp_vault();
        let client = VaultClient::from_canonical_root(path.clone());
        let e = client
            .resolve_under_vault("../outside-write.md", false)
            .expect_err("must error");
        assert!(matches!(e, VaultError::PathEscapesVault(_)));
        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn resolve_under_vault_handles_symlink_pointing_inside_vault() {
        // Symlinks pointing INSIDE the vault are OK —
        // canonicalization resolves and the result is
        // still under the vault root.
        let path = tmp_vault();
        let target = path.join("real-note.md");
        fs::write(&target, "x").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&target, path.join("link.md")).unwrap();
            let client = VaultClient::from_canonical_root(path.clone());
            let resolved = client.resolve_under_vault("link.md", true).expect("ok");
            assert!(resolved.starts_with(&path));
        }
        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn resolve_under_vault_rejects_symlink_pointing_outside_vault() {
        // Symlinks pointing OUTSIDE the vault are
        // load-bearing rejections — operators creating
        // such a symlink would otherwise let the agent
        // read /etc/passwd via a vault-relative path.
        let path = tmp_vault();
        let outside = std::env::temp_dir().join(format!(
            "aivyx-outside-{}.txt",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&outside, "secrets").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, path.join("evil.md")).unwrap();
            let client = VaultClient::from_canonical_root(path.clone());
            let e = client
                .resolve_under_vault("evil.md", true)
                .expect_err("must error");
            assert!(matches!(e, VaultError::PathEscapesVault(_)));
        }
        let _ = fs::remove_file(&outside);
        let _ = fs::remove_dir_all(&path);
    }
}
