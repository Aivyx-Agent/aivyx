//! `aivyx-calendar auth revoke` — revoke tokens with Google +
//! delete the local token file.
//!
//! Phase 123 Task 3 — operator-facing.
//!
//! The flow:
//!
//! 1. Load tokens from disk. If absent, surface a friendly
//!    "nothing to revoke" message and exit success
//!    (idempotent).
//! 2. POST the refresh_token (preferred) or access_token to
//!    Google's revoke endpoint
//!    (`https://oauth2.googleapis.com/revoke`). Revoking the
//!    refresh_token invalidates BOTH refresh + access tokens
//!    in one call.
//! 3. Delete the local token file regardless of whether the
//!    HTTP call succeeded. The remote-side revoke is
//!    best-effort (network may be down, Google may be
//!    unreachable); the local file removal is the load-bearing
//!    operator-facing outcome.

use std::io;
use std::path::Path;

use reqwest::Client;
use thiserror::Error;
use tokio::fs;

use crate::oauth::{load_tokens, StorageError, TokenSet};

/// Google's OAuth 2.0 token-revoke endpoint.
pub const GOOGLE_REVOKE_ENDPOINT: &str = "https://oauth2.googleapis.com/revoke";

#[derive(Debug, Error)]
pub enum RevokeError {
    #[error("token storage failed: {0}")]
    Storage(#[from] StorageError),
    #[error("failed to delete token file at {path:?}: {source}")]
    DeleteFailed {
        path: std::path::PathBuf,
        source: io::Error,
    },
}

/// Outcome of a revoke run. Captures whether the remote
/// revoke succeeded separately from the local-file removal,
/// since the latter happens regardless and operators may want
/// to know the remote-side reached Google.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevokeOutcome {
    pub remote_status: RemoteRevokeStatus,
    pub local_file_deleted: bool,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteRevokeStatus {
    /// HTTP call succeeded with a 2xx response.
    Confirmed,
    /// HTTP call reached Google but the response was non-2xx.
    /// Common when the token was already expired/revoked.
    NonSuccess { status: u16, body: String },
    /// HTTP call did not reach Google (network down, DNS,
    /// etc). The local file is still removed.
    NetworkFailed { error: String },
    /// No tokens to revoke (file absent at start).
    NoTokensLoaded,
}

/// Revoke the on-disk tokens. The HTTP call is best-effort;
/// the local file is removed regardless so a partial revoke
/// (Google unreachable) still leaves the operator in a clean
/// "must re-run auth init" state.
pub async fn run_auth_revoke(
    client: &Client,
    revoke_endpoint: &str,
    token_path: &Path,
) -> Result<RevokeOutcome, RevokeError> {
    let tokens = match load_tokens(token_path).await? {
        Some(t) => t,
        None => {
            return Ok(RevokeOutcome {
                remote_status: RemoteRevokeStatus::NoTokensLoaded,
                local_file_deleted: false,
                message: format!(
                    "No tokens found at {path}; nothing to revoke.",
                    path = token_path.display(),
                ),
            });
        }
    };

    let remote_status = remote_revoke(client, revoke_endpoint, &tokens).await;

    // Local file removal is best-effort but failures DO surface
    // (operators would want to know if `.json` couldn't be
    // removed because the directory perms changed).
    let deleted = match fs::remove_file(token_path).await {
        Ok(()) => true,
        Err(e) if e.kind() == io::ErrorKind::NotFound => false,
        Err(e) => {
            return Err(RevokeError::DeleteFailed {
                path: token_path.to_path_buf(),
                source: e,
            });
        }
    };

    let message = format_revoke_message(&remote_status, deleted, token_path);

    Ok(RevokeOutcome {
        remote_status,
        local_file_deleted: deleted,
        message,
    })
}

async fn remote_revoke(
    client: &Client,
    endpoint: &str,
    tokens: &TokenSet,
) -> RemoteRevokeStatus {
    // Prefer revoking the refresh_token (invalidates both).
    // Fall back to access_token if no refresh available.
    let token_to_revoke = tokens
        .refresh_token
        .as_deref()
        .unwrap_or(&tokens.access_token);
    let params = [("token", token_to_revoke)];
    let response = match client.post(endpoint).form(&params).send().await {
        Ok(r) => r,
        Err(e) => {
            return RemoteRevokeStatus::NetworkFailed {
                error: e.to_string(),
            };
        }
    };
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if status.is_success() {
        RemoteRevokeStatus::Confirmed
    } else {
        RemoteRevokeStatus::NonSuccess {
            status: status.as_u16(),
            body,
        }
    }
}

fn format_revoke_message(
    remote: &RemoteRevokeStatus,
    deleted: bool,
    token_path: &Path,
) -> String {
    let remote_summary = match remote {
        RemoteRevokeStatus::Confirmed => {
            "Remote revoke: confirmed by Google.".to_string()
        }
        RemoteRevokeStatus::NonSuccess { status, .. } => {
            format!(
                "Remote revoke: Google returned status {status} \
                 (often means token was already expired/revoked)."
            )
        }
        RemoteRevokeStatus::NetworkFailed { error } => {
            format!(
                "Remote revoke: network call failed ({error}). \
                 Local file still removed; operator may want to \
                 manually revoke at https://myaccount.google.com/permissions."
            )
        }
        RemoteRevokeStatus::NoTokensLoaded => {
            return format!(
                "No tokens to revoke at {path}.",
                path = token_path.display()
            );
        }
    };
    let local_summary = if deleted {
        format!("Local file: deleted ({path}).", path = token_path.display())
    } else {
        format!("Local file: was absent at {path}.", path = token_path.display())
    };
    format!("{remote_summary}\n{local_summary}\nRun `aivyx-calendar auth init` to re-authorize.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth::{save_tokens, TokenSet};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    fn now_unix_secs() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    fn scratch_dir() -> std::path::PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let tmp = std::env::var("TMPDIR")
            .or_else(|_| std::env::var("TEMP"))
            .unwrap_or_else(|_| "/tmp".to_string());
        let dir = std::path::PathBuf::from(tmp).join(format!(
            "aivyx-calendar-revoke-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst),
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn sample_tokens() -> TokenSet {
        TokenSet {
            access_token: "ya29.access-x".to_string(),
            refresh_token: Some("1//refresh-x".to_string()),
            expires_at_unix_secs: now_unix_secs() + 3600,
            granted_scope: "scope".to_string(),
            token_type: "Bearer".to_string(),
        }
    }

    /// Minimal in-process mock revoke endpoint. Captures the
    /// POSTed `token=` value and responds with the canned
    /// status. Returns `(endpoint_url, captured_body)`.
    async fn spawn_mock_revoke_endpoint(
        canned_status: u16,
        canned_body: &'static str,
    ) -> (String, Arc<Mutex<String>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let captured = Arc::new(Mutex::new(String::new()));
        let captured_clone = Arc::clone(&captured);
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let (read_half, mut write_half) = sock.split();
            let mut reader = BufReader::new(read_half);
            let mut content_length: usize = 0;
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap() == 0 {
                    break;
                }
                if line == "\r\n" {
                    break;
                }
                if let Some(rest) =
                    line.to_ascii_lowercase().strip_prefix("content-length:")
                {
                    content_length = rest.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; content_length];
            reader.read_exact(&mut body).await.unwrap();
            *captured_clone.lock().unwrap() = String::from_utf8(body).unwrap();
            let phrase = if canned_status == 200 { "OK" } else { "Err" };
            let response = format!(
                "HTTP/1.1 {canned_status} {phrase}\r\n\
                 Content-Type: text/plain\r\n\
                 Content-Length: {}\r\n\
                 Connection: close\r\n\
                 \r\n\
                 {}",
                canned_body.len(),
                canned_body,
            );
            write_half.write_all(response.as_bytes()).await.unwrap();
            write_half.flush().await.unwrap();
        });
        (format!("http://127.0.0.1:{port}/revoke"), captured)
    }

    #[tokio::test]
    async fn revoke_prefers_refresh_token() {
        let dir = scratch_dir();
        let path = dir.join("tokens.json");
        save_tokens(&path, &sample_tokens()).await.unwrap();
        let (endpoint, captured) = spawn_mock_revoke_endpoint(200, "").await;
        let client = Client::new();
        let outcome = run_auth_revoke(&client, &endpoint, &path).await.expect("revoke");
        assert_eq!(outcome.remote_status, RemoteRevokeStatus::Confirmed);
        assert!(outcome.local_file_deleted);
        let body = captured.lock().unwrap().clone();
        // The refresh_token (preferred over access_token) was
        // sent; percent-encoded in the form body.
        assert!(body.contains("token=1%2F%2Frefresh-x"), "body: {body}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn revoke_falls_back_to_access_token_when_no_refresh() {
        let dir = scratch_dir();
        let path = dir.join("tokens.json");
        let mut tokens = sample_tokens();
        tokens.refresh_token = None;
        save_tokens(&path, &tokens).await.unwrap();
        let (endpoint, captured) = spawn_mock_revoke_endpoint(200, "").await;
        let client = Client::new();
        run_auth_revoke(&client, &endpoint, &path).await.expect("revoke");
        let body = captured.lock().unwrap().clone();
        assert!(body.contains("token=ya29.access-x"), "body: {body}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn revoke_deletes_file_even_when_remote_returns_non_success() {
        let dir = scratch_dir();
        let path = dir.join("tokens.json");
        save_tokens(&path, &sample_tokens()).await.unwrap();
        let (endpoint, _) = spawn_mock_revoke_endpoint(400, "invalid_token").await;
        let client = Client::new();
        let outcome = run_auth_revoke(&client, &endpoint, &path).await.expect("revoke");
        match outcome.remote_status {
            RemoteRevokeStatus::NonSuccess { status, body } => {
                assert_eq!(status, 400);
                assert!(body.contains("invalid_token"));
            }
            other => panic!("expected NonSuccess; got {other:?}"),
        }
        assert!(outcome.local_file_deleted);
        assert!(!path.exists(), "local file must be removed even on remote failure");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn revoke_deletes_file_even_when_network_unreachable() {
        let dir = scratch_dir();
        let path = dir.join("tokens.json");
        save_tokens(&path, &sample_tokens()).await.unwrap();
        let client = Client::new();
        // Point at an unbound port → connection refused.
        let outcome = run_auth_revoke(&client, "http://127.0.0.1:1/revoke", &path)
            .await
            .expect("revoke");
        match outcome.remote_status {
            RemoteRevokeStatus::NetworkFailed { .. } => {}
            other => panic!("expected NetworkFailed; got {other:?}"),
        }
        assert!(outcome.local_file_deleted);
        assert!(!path.exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn revoke_when_no_tokens_present_is_noop() {
        let dir = scratch_dir();
        let path = dir.join("missing.json");
        let client = Client::new();
        let outcome = run_auth_revoke(&client, "http://unused.invalid/r", &path)
            .await
            .expect("revoke");
        assert_eq!(outcome.remote_status, RemoteRevokeStatus::NoTokensLoaded);
        assert!(!outcome.local_file_deleted);
        assert!(outcome.message.contains("nothing to revoke"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn format_revoke_message_confirmed_path() {
        let m = format_revoke_message(
            &RemoteRevokeStatus::Confirmed,
            true,
            std::path::Path::new("/tmp/t.json"),
        );
        assert!(m.contains("confirmed by Google"));
        assert!(m.contains("Local file: deleted"));
        assert!(m.contains("re-authorize"));
    }

    #[test]
    fn format_revoke_message_network_failed_points_at_manual_revoke() {
        let m = format_revoke_message(
            &RemoteRevokeStatus::NetworkFailed {
                error: "connection refused".to_string(),
            },
            true,
            std::path::Path::new("/tmp/t.json"),
        );
        assert!(m.contains("network call failed"));
        // Critical operator-facing guidance — point them at the
        // browser-side revoke surface as the fallback.
        assert!(m.contains("myaccount.google.com/permissions"));
    }
}
