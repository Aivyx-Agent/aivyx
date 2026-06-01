//! `aivyx-n8n auth status` (offline) + `auth check` (online).

use std::path::Path;

use crate::auth_cli::config_file::{load_config, ConfigFileError};
use crate::{N8nClient, N8nClientError};

#[derive(Debug)]
pub struct StatusReport {
    pub config_path: std::path::PathBuf,
    pub base_url: String,
    pub api_key_present: bool,
}

impl std::fmt::Display for StatusReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.api_key_present {
            writeln!(
                f,
                "aivyx-n8n auth status: OK\n  config: {:?}\n  base_url: {}\n  api_key: present (non-empty)\n  next: run `aivyx-n8n auth check` to ping the instance",
                self.config_path, self.base_url
            )
        } else {
            writeln!(
                f,
                "aivyx-n8n auth status: FAIL\n  config: {:?}\n  api_key MISSING",
                self.config_path
            )
        }
    }
}

pub fn run_auth_status(path: &Path) -> Result<StatusReport, ConfigFileError> {
    let cfg = load_config(path)?;
    Ok(StatusReport {
        config_path: path.to_path_buf(),
        base_url: cfg.n8n_base_url,
        api_key_present: !cfg.n8n_api_key.trim().is_empty(),
    })
}

#[derive(Debug)]
pub struct CheckReport {
    pub ok: bool,
    pub message: String,
}

impl std::fmt::Display for CheckReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.ok {
            writeln!(f, "aivyx-n8n auth check: OK — {}", self.message)
        } else {
            writeln!(f, "aivyx-n8n auth check: FAIL — {}", self.message)
        }
    }
}

/// Ping `/api/v1/workflows?limit=1` as a lightweight
/// "does the key work?" check.
pub async fn run_auth_check(client: &N8nClient) -> CheckReport {
    match client
        .get_json::<serde_json::Value>("/workflows", &[("limit", "1".to_string())])
        .await
    {
        Ok(_) => CheckReport {
            ok: true,
            message: "API key accepted; instance reachable".to_string(),
        },
        Err(N8nClientError::Api { status: 401, .. })
        | Err(N8nClientError::Api { status: 403, .. }) => CheckReport {
            ok: false,
            message:
                "key rejected (HTTP 401/403) — check that n8n_api_key matches Settings → API in your n8n instance"
                    .to_string(),
        },
        Err(N8nClientError::Transport(msg)) => CheckReport {
            ok: false,
            message: format!(
                "could not reach n8n at the configured base URL (transport error: {msg})"
            ),
        },
        Err(e) => CheckReport {
            ok: false,
            message: format!("n8n API error: {e}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_report_display_distinguishes_ok_and_fail() {
        let ok = StatusReport {
            config_path: "/tmp/x".into(),
            base_url: "https://n8n.example.com".into(),
            api_key_present: true,
        };
        let fail = StatusReport {
            config_path: "/tmp/x".into(),
            base_url: "https://x".into(),
            api_key_present: false,
        };
        assert!(ok.to_string().contains("OK"));
        assert!(ok.to_string().contains("aivyx-n8n auth check"));
        assert!(fail.to_string().contains("FAIL"));
    }

    #[test]
    fn check_report_distinguishes_ok_and_fail() {
        let ok = CheckReport {
            ok: true,
            message: "happy".into(),
        };
        let fail = CheckReport {
            ok: false,
            message: "sad".into(),
        };
        assert!(ok.to_string().contains("OK"));
        assert!(fail.to_string().contains("FAIL"));
    }
}
