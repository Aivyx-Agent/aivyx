//! Re-export shim + calendar-specific `default_token_path`.
//!
//! Phase 129 Task 2 lift: the implementation of
//! `save_tokens` / `load_tokens` now lives in
//! [`aivyx_google_oauth::storage`].

use std::path::PathBuf;

pub use aivyx_google_oauth::storage::{load_tokens, save_tokens, StorageError};

/// Resolves the default token storage path for the
/// Calendar tool process:
/// `$HOME/.aivyx-pa/tool-processes/calendar/tokens.json`.
pub fn default_token_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(
        PathBuf::from(home)
            .join(".aivyx-pa")
            .join("tool-processes")
            .join("calendar")
            .join("tokens.json"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_token_path_includes_calendar_segment() {
        let Some(p) = default_token_path() else {
            return;
        };
        let s = p.to_string_lossy();
        assert!(s.contains("tool-processes"), "{s}");
        assert!(s.contains("calendar"), "{s}");
        assert!(s.ends_with("tokens.json"), "{s}");
    }
}
