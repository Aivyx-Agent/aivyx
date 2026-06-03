//! `aivyx_core::Tool` implementations for the Drive tool
//! process.
//!
//! Phase 129 Q2b operator-picked surface (7 tools).
//! Per-tool modules ship in Tasks 4-10. This `mod.rs` is
//! the entry-point that `main.rs` reaches into to build
//! the harness's `Vec<Arc<dyn Tool>>`.

pub mod create_folder;
pub mod delete_file;
pub mod download_file;
pub mod get_metadata;
pub mod list_drives;
pub mod list_folder;
pub mod recent_changes;
pub mod recent_files;
pub mod search;
pub mod upload_file;

pub use create_folder::DriveCreateFolder;
pub use delete_file::DriveDeleteFile;
pub use download_file::DriveDownloadFile;
pub use get_metadata::DriveGetMetadata;
pub use list_drives::DriveListDrives;
pub use list_folder::DriveListFolder;
pub use recent_changes::DriveRecentChanges;
pub use recent_files::DriveRecentFiles;
pub use search::DriveSearch;
pub use upload_file::DriveUploadFile;

/// Inline cap on file content for the base64-in-JSON
/// substrate. Files above this size return metadata-only
/// from `drive.download_file` with `content_truncated:
/// true` and a clear error; uploads above this size are
/// rejected at input validation. The cap reflects Phase
/// 129 Q3a Recommended.
pub const CONTENT_INLINE_CAP_BYTES: usize = 10 * 1024 * 1024;

/// Phase 153 — defaults for the recursive folder
/// walk. The recent_* tools call
/// [`walk_folder_tree`] with these caps unless an
/// operator-tunable knob lands in a follow-on
/// phase.
pub(crate) const RECURSIVE_MAX_DEPTH: usize = 5;
pub(crate) const RECURSIVE_MAX_FOLDERS: usize = 100;

/// Phase 153 — BFS-walk the Google Drive folder
/// tree rooted at `root_folder_id` and return
/// every folder ID encountered (including the
/// root). Used by `drive.recent_files` /
/// `drive.recent_changes` when the operator
/// passes `recursive: true` with a
/// `parent_folder_id` — the returned IDs feed
/// the `'<id>' in parents or ...` q-clause
/// expansion.
///
/// Hard caps:
/// - `max_depth` levels of descent. Phase 153
///   uses [`RECURSIVE_MAX_DEPTH`] = 5.
/// - `max_folders` total folders returned. Phase
///   153 uses [`RECURSIVE_MAX_FOLDERS`] = 100.
///
/// On cap-hit (either bound) the walk stops
/// early and returns the partial list — no
/// error. Callers can detect "we hit the cap"
/// by comparing the returned length to
/// `max_folders`.
pub(crate) async fn walk_folder_tree(
    client: &crate::drive_client::SharedDriveClient,
    root_folder_id: &str,
    max_depth: usize,
    max_folders: usize,
) -> Result<Vec<String>, crate::drive_client::DriveClientError> {
    let mut visited: Vec<String> = vec![root_folder_id.to_string()];
    // Each entry is (folder_id, depth). Depth 0
    // is the root.
    let mut queue: std::collections::VecDeque<(String, usize)> =
        std::collections::VecDeque::new();
    queue.push_back((root_folder_id.to_string(), 0));

    while let Some((folder_id, depth)) = queue.pop_front() {
        if visited.len() >= max_folders {
            break;
        }
        if depth >= max_depth {
            // Don't descend further from this
            // node — but the node itself stays in
            // `visited`.
            continue;
        }
        let q = format!(
            "'{}' in parents and mimeType = 'application/vnd.google-apps.folder' and trashed = false",
            folder_id
        );
        let query: Vec<(&str, String)> = vec![
            ("pageSize", "100".to_string()),
            ("fields", "files(id),nextPageToken".to_string()),
            ("q", q),
        ];
        let body: serde_json::Value = client.get_json("/files", &query).await?;
        if let Some(files) = body.get("files").and_then(|v| v.as_array()) {
            for f in files {
                if let Some(id) = f.get("id").and_then(|v| v.as_str()) {
                    if visited.len() >= max_folders {
                        break;
                    }
                    visited.push(id.to_string());
                    queue.push_back((id.to_string(), depth + 1));
                }
            }
        }
        // Phase 153 doesn't paginate per-folder
        // children — typical folder children
        // count is well below Google's default
        // pageSize. Phase 154+ candidate if
        // ultra-wide folder operators surface.
    }

    Ok(visited)
}

/// Phase 153 — compose an OR-joined `'<id>' in
/// parents` clause from a slice of folder IDs.
/// Pure substrate; the recent_* tools call this
/// after [`walk_folder_tree`].
///
/// Examples:
/// - `&[]` → `""` (caller should use
///   `Option::None` rather than passing
///   empty).
/// - `&["f1"]` → `"'f1' in parents"`.
/// - `&["f1", "f2"]` → `"'f1' in parents or
///   'f2' in parents"`.
pub(crate) fn compose_recursive_parent_clause(folder_ids: &[String]) -> String {
    folder_ids
        .iter()
        .map(|id| format!("'{}' in parents", id))
        .collect::<Vec<_>>()
        .join(" or ")
}

/// Minimal URL path-segment encoding shared across the
/// Drive tools. Drive file IDs are typically opaque
/// alphanumeric tokens (no encoding needed), but we
/// future-proof against IDs containing reserved chars.
#[allow(dead_code)]
pub(crate) fn drive_urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => out.push(ch),
            '@' => out.push_str("%40"),
            '/' => out.push_str("%2F"),
            ':' => out.push_str("%3A"),
            other => {
                let mut buf = [0u8; 4];
                let encoded = other.encode_utf8(&mut buf);
                for byte in encoded.bytes() {
                    out.push_str(&format!("%{:02X}", byte));
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod shared_tests {
    use super::{drive_urlencode, CONTENT_INLINE_CAP_BYTES};

    #[test]
    fn drive_urlencode_passes_safe_chars_through() {
        assert_eq!(drive_urlencode("1AbCdEfG-_.~"), "1AbCdEfG-_.~");
    }

    #[test]
    fn drive_urlencode_encodes_reserved_chars() {
        assert_eq!(drive_urlencode("a/b:c@d"), "a%2Fb%3Ac%40d");
    }

    // ---- Phase 153 — compose_recursive_parent_clause ----

    #[test]
    fn compose_recursive_parent_clause_empty_input_yields_empty_string() {
        let s = super::compose_recursive_parent_clause(&[]);
        assert_eq!(s, "");
    }

    #[test]
    fn compose_recursive_parent_clause_single_id_matches_phase_148_shape() {
        let s = super::compose_recursive_parent_clause(&["f1".to_string()]);
        assert_eq!(s, "'f1' in parents");
    }

    #[test]
    fn compose_recursive_parent_clause_multiple_ids_or_joined() {
        let s = super::compose_recursive_parent_clause(&[
            "f1".to_string(),
            "f2".to_string(),
            "f3".to_string(),
        ]);
        assert_eq!(s, "'f1' in parents or 'f2' in parents or 'f3' in parents");
    }

    #[test]
    fn recursive_caps_pin_to_documented_values() {
        // Regression boundary — Phase 153 hardcoded
        // these; if a future phase widens them, the
        // INSTALL.md doc + open doc need to track.
        assert_eq!(super::RECURSIVE_MAX_DEPTH, 5);
        assert_eq!(super::RECURSIVE_MAX_FOLDERS, 100);
    }

    #[test]
    fn content_inline_cap_is_ten_megabytes() {
        // Pin the cap so future tasks (or operator-facing
        // INSTALL.md) stay in sync with the constant.
        assert_eq!(CONTENT_INLINE_CAP_BYTES, 10_485_760);
    }
}
