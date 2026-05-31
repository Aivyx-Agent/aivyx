//! Re-export shim + gmail-specific `default_token_path`.
//!
//! Phase 129 Task 2 lift: the implementation of
//! `save_tokens` / `load_tokens` now lives in
//! [`aivyx_google_oauth::storage`]. The service-specific
//! token-file path resolution
//! (`~/.aivyx/tool-processes/gmail/tokens.json`) stays
//! here because each Google integration has its own
//! service-name segment.

use std::path::PathBuf;

pub use aivyx_google_oauth::storage::{load_tokens, save_tokens, StorageError};

/// Resolves the default token storage path for the Gmail
/// tool process: `$HOME/.aivyx/tool-processes/gmail/tokens.json`.
/// Returns `None` when `$HOME` is unset (CI or other non-
/// interactive contexts).
pub fn default_token_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".aivyx")
            .join("tool-processes")
            .join("gmail")
            .join("tokens.json"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_token_path_includes_gmail_segment() {
        // Test only runs when HOME is set (always true on
        // dev/CI Linux + macOS); skipped otherwise.
        let Some(p) = default_token_path() else {
            return;
        };
        let s = p.to_string_lossy();
        assert!(s.contains("tool-processes"), "{s}");
        assert!(s.contains("gmail"), "{s}");
        assert!(s.ends_with("tokens.json"), "{s}");
    }
}
