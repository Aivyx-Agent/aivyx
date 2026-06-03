//! Shared on-disk write helpers for the toolkit's
//! JSON stores.
//!
//! Phase 144 — extracted from the duplicated copies
//! that had appeared in `task_store` (Phase 125) and
//! `budget_store` (Phase 143). A third store would
//! have compounded the drift risk; one copy now
//! covers all of them.
//!
//! Each store wraps these in its own
//! `save_to_disk(path, &payload)` because the
//! serializable wrapper type differs per store
//! (`StoredTasks`, `StoredEntries`, …); the
//! OS-level primitives (atomic rename, secure
//! perms, parent-dir creation) are what's shared.
//!
//! ## Posture
//!
//! - **0600 file perms** (read/write for the
//!   operator only).
//! - **0700 parent-dir perms** (search/list/write
//!   for the operator only).
//! - **Atomic write-then-rename** so a crash
//!   mid-write can never leave a half-written file
//!   visible at the canonical path. The temp
//!   suffix is `.tmp` on the same filesystem so
//!   `rename` is an atomic syscall.

use std::io;
use std::path::{Path, PathBuf};

use tokio::fs;

/// Append `.tmp` to a path's final segment.
/// Returned path lives on the same directory
/// (same filesystem) so `rename` is atomic.
pub fn with_tmp_suffix(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".tmp");
    PathBuf::from(s)
}

/// Recursively `mkdir -p` a directory and, on
/// Unix, narrow its permissions to 0700.
pub async fn create_dir_all_secure(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o700);
        fs::set_permissions(dir, perms).await?;
    }
    Ok(())
}

/// Write `body` to `path` with 0600 permissions
/// from creation time and fsync the file before
/// returning. On Unix this is TOCTOU-safe via
/// `O_CREAT` with mode (the file never exists at
/// a wider permission than 0600). On non-Unix
/// platforms falls back to a plain write.
///
/// Caller is responsible for ensuring the parent
/// directory exists; pair with
/// [`create_dir_all_secure`].
pub async fn write_secure(path: &Path, body: &[u8]) -> io::Result<()> {
    #[cfg(unix)]
    {
        use tokio::io::AsyncWriteExt;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .await?;
        file.write_all(body).await?;
        file.sync_all().await?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        fs::write(path, body).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn scratch_dir() -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let tmp = std::env::var("TMPDIR")
            .or_else(|_| std::env::var("TEMP"))
            .unwrap_or_else(|_| "/tmp".to_string());
        let dir = PathBuf::from(tmp).join(format!(
            "aivyx-toolkit-secureio-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn with_tmp_suffix_appends_dot_tmp_to_final_segment() {
        let p = Path::new("/some/dir/budget.json");
        let tmp = with_tmp_suffix(p);
        assert_eq!(tmp, PathBuf::from("/some/dir/budget.json.tmp"));
    }

    #[test]
    fn with_tmp_suffix_handles_bare_filename() {
        let p = Path::new("ledger");
        let tmp = with_tmp_suffix(p);
        assert_eq!(tmp, PathBuf::from("ledger.tmp"));
    }

    #[tokio::test]
    async fn write_secure_sets_0600_on_unix() {
        let dir = scratch_dir();
        let path = dir.join("payload.json");
        write_secure(&path, b"hello").await.unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert_eq!(body, "hello");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            // Mask off the file-type bits; only
            // the perm triplet should remain.
            assert_eq!(
                mode & 0o777,
                0o600,
                "expected 0600 after write_secure, got {:o}",
                mode & 0o777,
            );
        }
    }

    #[tokio::test]
    async fn create_dir_all_secure_sets_0700_on_unix() {
        let parent = scratch_dir();
        let nested = parent.join("a").join("b");
        create_dir_all_secure(&nested).await.unwrap();
        assert!(nested.is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&nested).unwrap().permissions().mode();
            assert_eq!(
                mode & 0o777,
                0o700,
                "expected 0700 after create_dir_all_secure, got {:o}",
                mode & 0o777,
            );
        }
    }

    #[tokio::test]
    async fn write_secure_overwrites_existing_file() {
        let dir = scratch_dir();
        let path = dir.join("payload.json");
        write_secure(&path, b"first").await.unwrap();
        write_secure(&path, b"second").await.unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert_eq!(body, "second");
    }
}
