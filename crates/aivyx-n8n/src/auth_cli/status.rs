//! `aivyx-n8n auth status` (offline) + `auth check` (online).
//!
//! Thin wrappers around the substrate's `StatusReport`
//! and `CheckReport` — Phase 132 lift. Service-
//! specific bits: the n8n base URL is surfaced in the
//! status report; the check pings
//! `/api/v1/workflows?limit=1` with the n8n-specific
//! `X-N8N-API-KEY` header.

use std::path::Path;

pub use aivyx_auth_cli::{CheckReport, StatusReport};

use crate::auth_cli::config_file::{load_config, ConfigFileError};
use crate::{N8nClient, N8nClientError};

const BINARY_NAME: &str = "aivyx-n8n";

pub fn run_auth_status(path: &Path) -> Result<StatusReport, ConfigFileError> {
    let cfg = load_config(path)?;
    Ok(StatusReport::ok(
        BINARY_NAME,
        path.to_path_buf(),
        format!(
            "base_url: {}\n  api_key: present (non-empty)\n  next: run `aivyx-n8n auth check` to ping the instance",
            cfg.n8n_base_url
        ),
    ))
}

/// Ping `/api/v1/workflows?limit=1` as a lightweight
/// "does the key work?" check.
pub async fn run_auth_check(client: &N8nClient) -> CheckReport {
    match client
        .get_json::<serde_json::Value>("/workflows", &[("limit", "1".to_string())])
        .await
    {
        Ok(_) => CheckReport::ok(
            BINARY_NAME,
            "API key accepted; instance reachable",
        ),
        Err(N8nClientError::Api { status: 401, .. })
        | Err(N8nClientError::Api { status: 403, .. }) => CheckReport::fail(
            BINARY_NAME,
            "key rejected (HTTP 401/403) — check that n8n_api_key matches Settings → API in your n8n instance",
        ),
        Err(N8nClientError::Transport(msg)) => CheckReport::fail(
            BINARY_NAME,
            format!(
                "could not reach n8n at the configured base URL (transport error: {msg})"
            ),
        ),
        Err(e) => CheckReport::fail(BINARY_NAME, format!("n8n API error: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmpfile(body: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "aivyx-n8n-status-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(body.as_bytes()).unwrap();
        path
    }

    // Display impls are tested in aivyx-auth-cli.
    // The test here covers n8n-specific wiring:
    // run_auth_status loads the config and surfaces the
    // base_url in the detail line.

    #[test]
    fn run_auth_status_includes_base_url_in_detail() {
        let path = tmpfile(
            r#"n8n_base_url = "https://n8n.example.com"
n8n_api_key = "ntn_x""#,
        );
        let report = run_auth_status(&path).expect("ok");
        assert!(report.ok);
        let s = report.to_string();
        assert!(s.contains("aivyx-n8n auth status: OK"));
        assert!(s.contains("https://n8n.example.com"));
        assert!(s.contains("aivyx-n8n auth check"));
        let _ = std::fs::remove_file(&path);
    }
}
