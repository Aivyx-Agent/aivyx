//! `aivyx-notion auth status` + `auth check` operations.
//!
//! Thin wrappers around `aivyx_auth_cli::StatusReport` +
//! `CheckReport` — Phase 132 lift. Service-specific
//! bits (Notion's `/users/me` endpoint, the
//! `NotShared` error variant handling, the bot name
//! pulled from the response) live here.

use std::path::Path;

pub use aivyx_auth_cli::{CheckReport, StatusReport};

use crate::auth_cli::config_file::{load_config, ConfigFileError};
use crate::{NotionClient, NotionClientError};

const BINARY_NAME: &str = "aivyx-notion";

/// Offline status check — does the config file exist
/// with a non-empty token? No network call.
pub fn run_auth_status(config_path: &Path) -> Result<StatusReport, ConfigFileError> {
    // load_config returns Ok only when the file exists,
    // parses, and the token is non-empty (service-
    // specific validation). The OK report's `detail`
    // line points operators at the next step (`auth
    // check`) so they see what to do without having to
    // know the CLI surface.
    let _cfg = load_config(config_path)?;
    Ok(StatusReport::ok(
        BINARY_NAME,
        config_path.to_path_buf(),
        "token: present (non-empty)\n  next: run `aivyx-notion auth check` to verify the token has API access",
    ))
}

/// Online check — hits Notion's `/users/me` endpoint.
/// Returns a populated `CheckReport` rather than a
/// `Result` so the caller can render the outcome
/// regardless of whether the token works.
pub async fn run_auth_check(client: &NotionClient) -> CheckReport {
    match client.get_json::<serde_json::Value>("/users/me", &[]).await {
        Ok(body) => {
            // /users/me returns the bot user the integration
            // is bound to. Use its name (if present) for a
            // friendly success message.
            let name = body
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let bot_type = body
                .get("type")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            CheckReport::ok(
                BINARY_NAME,
                format!("token authenticated as {bot_type} `{name}`"),
            )
        }
        Err(NotionClientError::Api { status: 401, .. }) => CheckReport::fail(
            BINARY_NAME,
            "token rejected (HTTP 401) — check that the token in config.toml is the current Integration Token from Notion's Integrations dashboard",
        ),
        Err(NotionClientError::NotShared(_)) => CheckReport::ok(
            BINARY_NAME,
            // /users/me doesn't depend on sharing, so this
            // shouldn't happen — but if it does, fall
            // through as a soft warning.
            "token works for /users/me but Notion returned object_not_found — unusual; report this",
        ),
        Err(e) => CheckReport::fail(BINARY_NAME, format!("Notion API error: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmpfile(contents: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "aivyx-notion-status-{}-{}.toml",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        path
    }

    // Display impls for StatusReport / CheckReport are
    // tested in aivyx-auth-cli. These tests cover
    // Notion-specific wiring: run_auth_status calls
    // through to load_config and produces a populated
    // OK report; the next-step hint mentions
    // `aivyx-notion auth check`.

    #[test]
    fn run_auth_status_reports_ok_for_valid_config() {
        let path = tmpfile(r#"notion_token = "ntn_x""#);
        let report = run_auth_status(&path).expect("ok");
        assert!(report.ok);
        let s = report.to_string();
        assert!(s.contains("aivyx-notion auth status: OK"));
        assert!(s.contains("aivyx-notion auth check"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn run_auth_status_propagates_empty_token_failure() {
        let path = tmpfile(r#"notion_token = "   ""#);
        let e = run_auth_status(&path).expect_err("must error");
        assert!(matches!(e, ConfigFileError::EmptyToken { .. }));
        let _ = std::fs::remove_file(&path);
    }
}
