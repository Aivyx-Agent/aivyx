//! `aivyx_core::Tool` implementations for the Drive tool
//! process.
//!
//! Phase 129 Q2b operator-picked surface (7 tools).
//! Per-tool modules ship in Tasks 4-10. This `mod.rs` is
//! the entry-point that `main.rs` reaches into to build
//! the harness's `Vec<Arc<dyn Tool>>`.

pub mod search;

pub use search::DriveSearch;

/// Inline cap on file content for the base64-in-JSON
/// substrate. Files above this size return metadata-only
/// from `drive.download_file` with `content_truncated:
/// true` and a clear error; uploads above this size are
/// rejected at input validation. The cap reflects Phase
/// 129 Q3a Recommended.
pub const CONTENT_INLINE_CAP_BYTES: usize = 10 * 1024 * 1024;

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

    #[test]
    fn content_inline_cap_is_ten_megabytes() {
        // Pin the cap so future tasks (or operator-facing
        // INSTALL.md) stay in sync with the constant.
        assert_eq!(CONTENT_INLINE_CAP_BYTES, 10_485_760);
    }
}
