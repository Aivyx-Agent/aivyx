//! Status + check report types — the shared Display
//! shape every `aivyx-<service> auth {status,check}`
//! subcommand returns.
//!
//! Two report types because the two subcommands have
//! different semantics:
//!
//! - **`auth status`** is offline. It reports "is the
//!   config file present + are the required fields
//!   populated?" A single boolean (`ok`) plus a
//!   free-form `detail` line is enough — the operator
//!   reading the output wants "did it work? if not,
//!   what's missing?"
//!
//! - **`auth check`** is online. It reports "does the
//!   configured credential / path / API key actually
//!   work against the target?" Again a single boolean
//!   plus a free-form `message` — the operator wants
//!   "is the integration usable end-to-end?"
//!
//! Both reports carry the binary name so a multi-line
//! display starts with `aivyx-notion auth status: OK`
//! rather than just `OK`, matching the verbose output
//! shape each consumer used before the lift.

use std::path::PathBuf;

/// Result of an offline `auth status` run.
#[derive(Debug, Clone)]
pub struct StatusReport {
    pub binary_name: String,
    pub config_path: PathBuf,
    /// `true` when the config is present, parseable,
    /// and the service-specific validation passed.
    pub ok: bool,
    /// Free-form diagnostic line — on `ok`, this is
    /// typically a "next step" hint pointing the
    /// operator at `auth check`; on failure, an
    /// actionable description of what is missing.
    pub detail: String,
}

impl StatusReport {
    pub fn ok(binary_name: impl Into<String>, config_path: PathBuf, detail: impl Into<String>) -> Self {
        StatusReport {
            binary_name: binary_name.into(),
            config_path,
            ok: true,
            detail: detail.into(),
        }
    }

    pub fn fail(
        binary_name: impl Into<String>,
        config_path: PathBuf,
        detail: impl Into<String>,
    ) -> Self {
        StatusReport {
            binary_name: binary_name.into(),
            config_path,
            ok: false,
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for StatusReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let verdict = if self.ok { "OK" } else { "FAIL" };
        writeln!(
            f,
            "{} auth status: {}\n  config: {:?}\n  {}",
            self.binary_name, verdict, self.config_path, self.detail
        )
    }
}

/// Result of an online `auth check` run.
#[derive(Debug, Clone)]
pub struct CheckReport {
    pub binary_name: String,
    /// `true` when the credential / path / API
    /// reachability check succeeded.
    pub ok: bool,
    pub message: String,
}

impl CheckReport {
    pub fn ok(binary_name: impl Into<String>, message: impl Into<String>) -> Self {
        CheckReport {
            binary_name: binary_name.into(),
            ok: true,
            message: message.into(),
        }
    }

    pub fn fail(binary_name: impl Into<String>, message: impl Into<String>) -> Self {
        CheckReport {
            binary_name: binary_name.into(),
            ok: false,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for CheckReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let verdict = if self.ok { "OK" } else { "FAIL" };
        writeln!(
            f,
            "{} auth check: {} — {}",
            self.binary_name, verdict, self.message
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_ok_display_includes_binary_name_and_path() {
        let r = StatusReport::ok(
            "aivyx-notion",
            PathBuf::from("/tmp/cfg.toml"),
            "token: present",
        );
        let s = r.to_string();
        assert!(s.contains("aivyx-notion auth status: OK"));
        assert!(s.contains("/tmp/cfg.toml"));
        assert!(s.contains("token: present"));
    }

    #[test]
    fn status_fail_display_says_fail() {
        let r = StatusReport::fail(
            "aivyx-n8n",
            PathBuf::from("/x"),
            "n8n_api_key missing",
        );
        let s = r.to_string();
        assert!(s.contains("aivyx-n8n auth status: FAIL"));
        assert!(s.contains("n8n_api_key missing"));
    }

    #[test]
    fn check_ok_display_includes_message() {
        let r = CheckReport::ok("aivyx-obsidian", "vault reachable at /vault");
        let s = r.to_string();
        assert!(s.contains("aivyx-obsidian auth check: OK"));
        assert!(s.contains("vault reachable at /vault"));
    }

    #[test]
    fn check_fail_display_says_fail() {
        let r = CheckReport::fail("aivyx-notion", "HTTP 401");
        let s = r.to_string();
        assert!(s.contains("aivyx-notion auth check: FAIL"));
        assert!(s.contains("HTTP 401"));
    }

    #[test]
    fn binary_name_threads_through_each_consumer_correctly() {
        // The substrate is one crate but serves three+
        // consumers; the binary name must reflect the calling
        // consumer in every report.
        let n = StatusReport::ok("aivyx-notion", "/x".into(), "ok");
        let o = StatusReport::ok("aivyx-obsidian", "/x".into(), "ok");
        let a = StatusReport::ok("aivyx-n8n", "/x".into(), "ok");
        assert!(n.to_string().contains("aivyx-notion"));
        assert!(o.to_string().contains("aivyx-obsidian"));
        assert!(a.to_string().contains("aivyx-n8n"));
    }
}
