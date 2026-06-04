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
pub mod recent_activity;
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
pub use recent_activity::DriveRecentActivity;
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
/// Phase 157 — Level-parallel BFS. At each
/// depth, all per-folder children-queries fire
/// concurrently via
/// `futures_util::future::join_all`, then
/// results merge, then the next level fires.
/// Latency is now bounded by the slowest
/// single-level fan-out instead of the sum of
/// per-folder sequential queries.
///
/// Phase 157 — Optional `drive_id` scope. When
/// supplied, each per-folder query adds the
/// shared-drives parameters (corpora=drive +
/// driveId + includeItemsFromAllDrives +
/// supportsAllDrives) so the walk traverses a
/// Team Drive instead of the operator's My
/// Drive.
///
/// Hard caps (operator-tunable in Phase 157 via
/// recent_* input fields):
/// `max_depth` levels of descent;
/// `max_folders` total folders returned. On
/// cap-hit (either bound) the walk stops early
/// and returns the partial list — no error.
/// Callers can detect "we hit the cap" by
/// comparing the returned length to
/// `max_folders`.
pub(crate) async fn walk_folder_tree(
    client: &crate::drive_client::SharedDriveClient,
    root_folder_id: &str,
    max_depth: usize,
    max_folders: usize,
    drive_id: Option<&str>,
) -> Result<Vec<String>, crate::drive_client::DriveClientError> {
    let mut visited: Vec<String> = vec![root_folder_id.to_string()];
    let mut current_level: Vec<String> = vec![root_folder_id.to_string()];

    for depth in 0..max_depth {
        if visited.len() >= max_folders || current_level.is_empty() {
            break;
        }
        // Fire all per-folder children-queries
        // in this level concurrently. Each
        // future returns the Vec of child folder
        // IDs (or an error).
        let futures = current_level.iter().map(|folder_id| {
            let client = client.clone();
            let folder_id = folder_id.clone();
            let drive_id = drive_id.map(str::to_string);
            async move {
                children_of(&client, &folder_id, drive_id.as_deref()).await
            }
        });
        let results = futures_util::future::join_all(futures).await;

        let mut next_level: Vec<String> = Vec::new();
        for result in results {
            let children = result?;
            for child in children {
                if visited.len() >= max_folders {
                    break;
                }
                visited.push(child.clone());
                next_level.push(child);
            }
            if visited.len() >= max_folders {
                break;
            }
        }
        let _ = depth; // we use the loop counter via iteration count
        current_level = next_level;
    }

    Ok(visited)
}

/// Phase 157 — pure helper that queries one
/// folder's direct-child folders. Lifted from
/// the inline pre-157 body of
/// [`walk_folder_tree`] so the level-parallel
/// caller can fire many of these concurrently.
async fn children_of(
    client: &crate::drive_client::SharedDriveClient,
    folder_id: &str,
    drive_id: Option<&str>,
) -> Result<Vec<String>, crate::drive_client::DriveClientError> {
    let q = format!(
        "'{}' in parents and mimeType = 'application/vnd.google-apps.folder' and trashed = false",
        folder_id
    );
    let mut query: Vec<(&str, String)> = vec![
        ("pageSize", "100".to_string()),
        ("fields", "files(id),nextPageToken".to_string()),
        ("q", q),
    ];
    // Phase 157 — drive_id scope. When present,
    // query the shared drive's corpus instead
    // of the operator's My Drive.
    if let Some(did) = drive_id {
        query.push(("corpora", "drive".to_string()));
        query.push(("driveId", did.to_string()));
        query.push(("includeItemsFromAllDrives", "true".to_string()));
        query.push(("supportsAllDrives", "true".to_string()));
    }
    let body: serde_json::Value = client.get_json("/files", &query).await?;
    let mut out: Vec<String> = Vec::new();
    if let Some(files) = body.get("files").and_then(|v| v.as_array()) {
        for f in files {
            if let Some(id) = f.get("id").and_then(|v| v.as_str()) {
                out.push(id.to_string());
            }
        }
    }
    Ok(out)
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
