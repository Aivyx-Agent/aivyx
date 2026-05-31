//! `aivyx-notion auth status` + `auth check` operations.
//!
//! Two distinct commands:
//!
//! - **`status`** is offline — it just checks that the
//!   config file exists and the token field is non-empty.
//!   Fast; no network call.
//! - **`check`** is online — it actively polls Notion's
//!   `/users/me` endpoint to confirm the token has API
//!   access. Surfaces a "token works" vs "token rejected"
//!   distinction so operators can debug their setup.

use std::path::Path;

use crate::auth_cli::config_file::{load_config, ConfigFileError};
use crate::{NotionClient, NotionClientError};

#[derive(Debug)]
pub struct StatusReport {
    pub config_path: std::path::PathBuf,
    pub token_present: bool,
}

impl std::fmt::Display for StatusReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.token_present {
            writeln!(
                f,
                "aivyx-notion auth status: OK\n  config: {:?}\n  token: present (non-empty)\n  next: run `aivyx-notion auth check` to verify the token has API access",
                self.config_path
            )
        } else {
            writeln!(
                f,
                "aivyx-notion auth status: token MISSING\n  config: {:?}\n  fix: write `notion_token = \"ntn_...\"` into the config file",
                self.config_path
            )
        }
    }
}

/// Offline status check — does the config file exist with
/// a non-empty token? No network call.
pub fn run_auth_status(config_path: &Path) -> Result<StatusReport, ConfigFileError> {
    let cfg = load_config(config_path)?;
    Ok(StatusReport {
        config_path: config_path.to_path_buf(),
        token_present: !cfg.notion_token.trim().is_empty(),
    })
}

#[derive(Debug)]
pub struct CheckReport {
    pub token_works: bool,
    pub message: String,
}

impl std::fmt::Display for CheckReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.token_works {
            writeln!(f, "aivyx-notion auth check: OK — {}", self.message)
        } else {
            writeln!(f, "aivyx-notion auth check: FAIL — {}", self.message)
        }
    }
}

/// Online check — hits Notion's `/users/me` endpoint.
/// Returns a populated `CheckReport` rather than a Result
/// so the caller can render the outcome regardless of
/// whether the token works.
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
            CheckReport {
                token_works: true,
                message: format!(
                    "token authenticated as {bot_type} `{name}`"
                ),
            }
        }
        Err(NotionClientError::Api { status: 401, .. }) => CheckReport {
            token_works: false,
            message: "token rejected (HTTP 401) — check that the token in config.toml is the current Integration Token from Notion's Integrations dashboard".to_string(),
        },
        Err(NotionClientError::NotShared(_)) => CheckReport {
            token_works: true,
            // /users/me doesn't depend on sharing, so this
            // shouldn't happen — but if it does, fall
            // through as a soft warning.
            message: "token works for /users/me but Notion returned object_not_found — unusual; report this".to_string(),
        },
        Err(e) => CheckReport {
            token_works: false,
            message: format!("Notion API error: {e}"),
        },
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
                .as_nanos()
        ));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(contents.as_bytes()).unwrap();
        path
    }

    #[test]
    fn run_auth_status_reports_token_present_for_valid_config() {
        let path = tmpfile(r#"notion_token = "ntn_x""#);
        let report = run_auth_status(&path).expect("ok");
        assert!(report.token_present);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn status_report_display_includes_next_step_when_ok() {
        let report = StatusReport {
            config_path: "/tmp/x".into(),
            token_present: true,
        };
        let s = report.to_string();
        assert!(s.contains("OK"));
        assert!(s.contains("aivyx-notion auth check"));
    }

    #[test]
    fn check_report_display_distinguishes_ok_from_fail() {
        let ok = CheckReport {
            token_works: true,
            message: "happy path".into(),
        };
        let fail = CheckReport {
            token_works: false,
            message: "sad path".into(),
        };
        assert!(ok.to_string().contains("OK"));
        assert!(fail.to_string().contains("FAIL"));
    }
}
