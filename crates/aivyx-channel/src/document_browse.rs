//! Chapter Z — read-only filesystem browsing for the Studio's Documents screen.
//!
//! Both `list_dir` and `read_file` run a request through the **same guard the
//! fs/workspace tools use**: lexically resolve `rel` against a *pre-canonicalized*
//! root (`aivyx_core::tools::fs::lexical_resolve` rejects `..`-over-root by
//! construction), then `std::fs::canonicalize` resolves every symlink, then the
//! result must still `starts_with(root)`. `..` and symlink escapes are rejected.
//!
//! Read-only by design: there is no write/delete/rename here. File reads are
//! capped ([`READ_CAP_BYTES`]) and binary-aware (a NUL byte ⇒ `content = None`).
//! The caller passes an **already-canonical** root (the daemon canonicalizes the
//! `fs_root` / workspace root once at startup).

use std::path::{Path, PathBuf};

use aivyx_ipc::protocol::{DocEntry, DocFile};

/// Cap on the bytes returned for a file's text content. Larger files come back
/// with `content = None` + `truncated`/`size_bytes` so the viewer can say
/// "N KB — not shown".
pub const READ_CAP_BYTES: usize = 256 * 1024;

/// Why a browse request was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseError {
    /// `..` or a symlink resolved outside the root.
    PathEscape,
    /// The path does not exist.
    NotFound,
    /// `list_dir` was asked for a non-directory.
    NotADir,
    /// `read_file` was asked for a directory.
    NotAFile,
    /// Any other I/O failure.
    Io(String),
}

impl std::fmt::Display for BrowseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BrowseError::PathEscape => write!(f, "path escapes the allowed root"),
            BrowseError::NotFound => write!(f, "no such file or directory"),
            BrowseError::NotADir => write!(f, "not a directory"),
            BrowseError::NotAFile => write!(f, "not a file"),
            BrowseError::Io(e) => write!(f, "{e}"),
        }
    }
}

/// Resolve `rel` against the pre-canonicalized `root`, verifying it stays under
/// `root` after symlink resolution. The shared guard for both list + read.
fn safe_resolve(root: &Path, rel: &str) -> Result<PathBuf, BrowseError> {
    // Empty / "." ⇒ the root itself.
    let rel = if rel.trim().is_empty() { "." } else { rel };
    // Lexical resolution first — rejects more `..` than depth below the root.
    let lexical =
        aivyx_core::tools::fs::lexical_resolve(root, Path::new(rel)).ok_or(BrowseError::PathEscape)?;
    // Canonicalize resolves every symlink; the path must exist.
    let canonical = std::fs::canonicalize(&lexical).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => BrowseError::NotFound,
        _ => BrowseError::Io(e.to_string()),
    })?;
    // The canonical target must still live under the canonical root.
    if !canonical.starts_with(root) {
        return Err(BrowseError::PathEscape);
    }
    Ok(canonical)
}

/// List the directory at `rel` under `root` (read-only). Entries are sorted
/// directories-first, then case-insensitively by name.
pub fn list_dir(root: &Path, rel: &str) -> Result<Vec<DocEntry>, BrowseError> {
    let dir = safe_resolve(root, rel)?;
    let md = std::fs::metadata(&dir).map_err(|e| BrowseError::Io(e.to_string()))?;
    if !md.is_dir() {
        return Err(BrowseError::NotADir);
    }
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&dir)
        .map_err(|e| BrowseError::Io(e.to_string()))?
        .flatten()
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        let (kind, size_bytes) = match entry.file_type() {
            Ok(t) if t.is_dir() => ("dir", 0),
            Ok(t) if t.is_file() => ("file", entry.metadata().map(|m| m.len()).unwrap_or(0)),
            Ok(t) if t.is_symlink() => ("symlink", 0),
            _ => ("other", 0),
        };
        entries.push(DocEntry { name, kind: kind.to_string(), size_bytes });
    }
    entries.sort_by(|a, b| {
        let a_dir = a.kind == "dir";
        let b_dir = b.kind == "dir";
        b_dir
            .cmp(&a_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(entries)
}

/// Read the file at `rel` under `root`, capped at [`READ_CAP_BYTES`]. A file
/// containing a NUL byte is reported as binary (`content = None`); otherwise the
/// (possibly truncated) bytes are lossily decoded to UTF-8.
pub fn read_file(root: &Path, rel: &str) -> Result<DocFile, BrowseError> {
    use std::io::Read;
    let path = safe_resolve(root, rel)?;
    let md = std::fs::metadata(&path).map_err(|e| BrowseError::Io(e.to_string()))?;
    if md.is_dir() {
        return Err(BrowseError::NotAFile);
    }
    let size_bytes = md.len();

    let file = std::fs::File::open(&path).map_err(|e| BrowseError::Io(e.to_string()))?;
    let mut bytes = Vec::new();
    file.take(READ_CAP_BYTES as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| BrowseError::Io(e.to_string()))?;

    let truncated = size_bytes > bytes.len() as u64;
    let binary = bytes.contains(&0);
    let content = if binary {
        None
    } else {
        // Lossy decode tolerates a multi-byte char split at the cap boundary.
        Some(String::from_utf8_lossy(&bytes).into_owned())
    };

    Ok(DocFile {
        path: rel.to_string(),
        size_bytes,
        content,
        truncated,
        binary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A canonical scratch root with a small seeded tree.
    fn seeded_root() -> PathBuf {
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "aivyx-docbrowse-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("readme.txt"), b"hello world").unwrap();
        std::fs::write(dir.join("sub").join("nested.md"), b"# nested").unwrap();
        std::fs::write(dir.join("blob.bin"), [0u8, 1, 2, 3, 0]).unwrap();
        // Canonicalize — the primitive expects a canonical root.
        std::fs::canonicalize(&dir).unwrap()
    }

    #[test]
    fn list_dir_sorts_dirs_first_and_reports_kinds() {
        let root = seeded_root();
        let entries = list_dir(&root, "").unwrap();
        // "sub" (dir) sorts before the files.
        assert_eq!(entries[0].name, "sub");
        assert_eq!(entries[0].kind, "dir");
        let readme = entries.iter().find(|e| e.name == "readme.txt").unwrap();
        assert_eq!(readme.kind, "file");
        assert_eq!(readme.size_bytes, 11);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn list_dir_descends_into_subdir() {
        let root = seeded_root();
        let entries = list_dir(&root, "sub").unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "nested.md");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn read_file_returns_text() {
        let root = seeded_root();
        let f = read_file(&root, "readme.txt").unwrap();
        assert_eq!(f.content.as_deref(), Some("hello world"));
        assert!(!f.binary);
        assert!(!f.truncated);
        assert_eq!(f.size_bytes, 11);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn read_file_flags_binary() {
        let root = seeded_root();
        let f = read_file(&root, "blob.bin").unwrap();
        assert!(f.binary);
        assert!(f.content.is_none(), "binary content is withheld");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn read_file_caps_and_marks_truncated() {
        let root = seeded_root();
        let big = "a".repeat(READ_CAP_BYTES + 5000);
        std::fs::write(root.join("big.txt"), &big).unwrap();
        let f = read_file(&root, "big.txt").unwrap();
        assert!(f.truncated);
        assert_eq!(f.content.as_ref().unwrap().len(), READ_CAP_BYTES);
        assert_eq!(f.size_bytes, (READ_CAP_BYTES + 5000) as u64);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn dotdot_escape_is_rejected() {
        let root = seeded_root();
        // Climb above the root.
        let err = list_dir(&root, "../../../etc").unwrap_err();
        assert_eq!(err, BrowseError::PathEscape);
        // And via a file path.
        assert_eq!(read_file(&root, "../secret").unwrap_err(), BrowseError::PathEscape);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn list_on_a_file_and_read_on_a_dir_are_typed_errors() {
        let root = seeded_root();
        assert_eq!(list_dir(&root, "readme.txt").unwrap_err(), BrowseError::NotADir);
        assert_eq!(read_file(&root, "sub").unwrap_err(), BrowseError::NotAFile);
        assert_eq!(list_dir(&root, "nope").unwrap_err(), BrowseError::NotFound);
        std::fs::remove_dir_all(&root).ok();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_escape_is_rejected() {
        let root = seeded_root();
        // A symlink inside the root pointing outside it.
        let outside = std::env::temp_dir().join(format!("aivyx-doc-outside-{}", std::process::id()));
        std::fs::write(&outside, b"secret").ok();
        let link = root.join("escape");
        let _ = std::os::unix::fs::symlink(&outside, &link);
        // Reading through the symlink canonicalizes outside the root → rejected.
        assert_eq!(read_file(&root, "escape").unwrap_err(), BrowseError::PathEscape);
        std::fs::remove_file(&outside).ok();
        std::fs::remove_dir_all(&root).ok();
    }
}
