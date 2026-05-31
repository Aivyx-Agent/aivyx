//! `aivyx-drive auth status` — print token health.
//!
//! Phase 123 Task 3 — operator-facing.
//!
//! Reads the token file at the supplied path; renders a
//! human-readable report covering:
//!
//! - Token presence (absent → "not initialized" guidance).
//! - Granted scope (so the operator can confirm `drive.send`
//!   is in the set when they want to use the send tool).
//! - Access-token expiry (absolute UTC timestamp + seconds-until
//!   so refresh status is unambiguous).
//! - Refresh availability (yes/no — `no` means re-init required
//!   on next refresh attempt).
//!
//! Formatting lives in `format_status_report` so unit tests can
//! pin the report shape against synthesized [`TokenSet`]s
//! without touching disk.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

use crate::{load_tokens, StorageError, TokenSet};

#[derive(Debug, Error)]
pub enum StatusError {
    #[error("token storage failed: {0}")]
    Storage(#[from] StorageError),
}

/// What the operator sees when they run `auth status`. One of:
/// - Tokens-present report (granted scope, expiry, refresh
///   status).
/// - Tokens-absent message pointing the operator at `auth init`.
pub async fn run_auth_status(token_path: &Path) -> Result<String, StatusError> {
    match load_tokens(token_path).await? {
        Some(tokens) => Ok(format_status_report(&tokens, token_path, now_unix_secs())),
        None => Ok(format_no_tokens_message(token_path)),
    }
}

fn format_status_report(tokens: &TokenSet, path: &Path, now_unix_secs: i64) -> String {
    let secs_remaining = tokens.expires_at_unix_secs - now_unix_secs;
    let expires_label = if secs_remaining <= 0 {
        format!("EXPIRED ({} seconds ago)", -secs_remaining)
    } else {
        format!("in {secs_remaining} seconds")
    };
    let needs_refresh = if tokens.needs_refresh_at(now_unix_secs) {
        "yes (within refresh-leeway window)"
    } else {
        "no (access token still fresh)"
    };
    let refresh_available = if tokens.can_refresh() {
        "yes"
    } else {
        "NO — `auth init` must be re-run before next refresh"
    };

    format!(
        "Aivyx Drive — token status\n\
         --------------------------\n\
         Token file:        {path}\n\
         Granted scope:     {scope}\n\
         Access token:      {token_hint}\n\
         Expires at:        {expires_at_unix_secs} unix ({expires_label})\n\
         Needs refresh:     {needs_refresh}\n\
         Refresh available: {refresh_available}\n",
        path = path.display(),
        scope = tokens.granted_scope,
        token_hint = redact_token(&tokens.access_token),
        expires_at_unix_secs = tokens.expires_at_unix_secs,
        expires_label = expires_label,
        needs_refresh = needs_refresh,
        refresh_available = refresh_available,
    )
}

fn format_no_tokens_message(path: &Path) -> String {
    format!(
        "Aivyx Drive — token status\n\
         --------------------------\n\
         No tokens found at {path}.\n\
         Run `aivyx-drive auth init` to complete the OAuth flow.\n",
        path = path.display(),
    )
}

/// Print a length-hint + the last 4 chars instead of the
/// full access token. The token is sensitive enough that
/// terminal scrollback / screen-share leaks shouldn't expose
/// it. The hint is enough for the operator to confirm "yes,
/// a token is present" without revealing it.
fn redact_token(token: &str) -> String {
    if token.len() <= 4 {
        return format!("<{} chars>", token.len());
    }
    let tail = &token[token.len() - 4..];
    format!("<{} chars, …{tail}>", token.len())
}

fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample_tokens() -> TokenSet {
        TokenSet {
            access_token: "ya29.aaaabbbbccccDDDD-token-suffix".to_string(),
            refresh_token: Some("1//refresh-x".to_string()),
            expires_at_unix_secs: 0, // overridden in each test
            granted_scope: "https://www.googleapis.com/auth/drive.readonly https://www.googleapis.com/auth/drive.send".to_string(),
            token_type: "Bearer".to_string(),
        }
    }

    #[test]
    fn format_status_report_renders_fresh_token() {
        let mut ts = sample_tokens();
        let now = 1_700_000_000;
        ts.expires_at_unix_secs = now + 3600;
        let out = format_status_report(&ts, &PathBuf::from("/tmp/t.json"), now);
        assert!(out.contains("Aivyx Drive — token status"));
        assert!(out.contains("Token file:        /tmp/t.json"));
        assert!(out.contains("drive.readonly"));
        assert!(out.contains("drive.send"));
        assert!(out.contains("in 3600 seconds"));
        assert!(out.contains("Needs refresh:     no"));
        assert!(out.contains("Refresh available: yes"));
    }

    #[test]
    fn format_status_report_flags_expired() {
        let mut ts = sample_tokens();
        let now = 1_700_000_000;
        ts.expires_at_unix_secs = now - 500;
        let out = format_status_report(&ts, &PathBuf::from("/tmp/t.json"), now);
        assert!(out.contains("EXPIRED (500 seconds ago)"));
        assert!(out.contains("Needs refresh:     yes"));
    }

    #[test]
    fn format_status_report_within_refresh_leeway_flags_refresh() {
        let mut ts = sample_tokens();
        let now = 1_700_000_000;
        ts.expires_at_unix_secs = now + 30; // < 60s leeway
        let out = format_status_report(&ts, &PathBuf::from("/tmp/t.json"), now);
        assert!(out.contains("Needs refresh:     yes"));
    }

    #[test]
    fn format_status_report_flags_missing_refresh_token() {
        let mut ts = sample_tokens();
        ts.refresh_token = None;
        let out = format_status_report(&ts, &PathBuf::from("/tmp/t.json"), 0);
        assert!(out.contains("Refresh available: NO"), "{out}");
        assert!(
            out.contains("`auth init` must be re-run"),
            "should guide operator to re-init; got: {out}"
        );
    }

    #[test]
    fn format_status_report_redacts_access_token() {
        let mut ts = sample_tokens();
        ts.expires_at_unix_secs = 1_700_003_600;
        let out = format_status_report(&ts, &PathBuf::from("/tmp/t.json"), 1_700_000_000);
        // Full token must NOT appear.
        assert!(
            !out.contains("ya29.aaaabbbbccccDDDD-token-suffix"),
            "full access token must not appear in status output; got: {out}"
        );
        // But the hint shows the tail so operator can confirm.
        assert!(out.contains("ffix"), "tail should appear; got: {out}");
        // And the length.
        assert!(out.contains("chars"));
    }

    #[test]
    fn format_no_tokens_message_points_at_auth_init() {
        let out = format_no_tokens_message(&PathBuf::from("/tmp/missing.json"));
        assert!(out.contains("No tokens found"));
        assert!(out.contains("aivyx-drive auth init"));
    }

    #[test]
    fn redact_token_handles_short_token() {
        assert_eq!(redact_token("abc"), "<3 chars>");
        assert_eq!(redact_token(""), "<0 chars>");
    }

    #[test]
    fn redact_token_shows_tail_for_long_token() {
        let redacted = redact_token("ya29.0123456789tail");
        assert!(redacted.contains("tail"));
        assert!(redacted.contains("19 chars"));
        assert!(!redacted.contains("ya29.012"));
    }
}
