//! CLI argument parsing for `aivyx-notion`.
//!
//! Thin wrapper over `aivyx_auth_cli` — Phase 132 lift.
//! Service-specific bits (binary name in errors,
//! `help_text` with Notion's critical "share with
//! integration" UX note) stay here.

pub use aivyx_auth_cli::{AuthMode, BinaryMode};

const BINARY_NAME: &str = "aivyx-notion";

pub fn parse_cli_args_from(argv: &[String]) -> Result<BinaryMode, String> {
    aivyx_auth_cli::parse_cli_args(argv, BINARY_NAME)
}

pub fn help_text() -> &'static str {
    "aivyx-notion — Notion third-party tool process for Aivyx PA

USAGE:
    aivyx-notion [SUBCOMMAND]

SUBCOMMANDS:
    auth status    Check that config.toml exists and the
                   notion_token field is non-empty.
    auth check     Actively poll Notion's /users/me endpoint
                   to confirm the token has API access.
                   Reports \"not_shared\" errors distinctly.
    help           Show this text.

When run with no arguments, the binary enters IPC-loop mode
(spawned by the Aivyx PA daemon via [[tool_process]] in
aivyx-pa.toml).

CONFIG FILE
    Expected at ~/.aivyx-pa/tool-processes/notion/config.toml
    with:

        notion_token = \"ntn_XXXXXXXXXX\"

    Obtain the token from Notion's Integrations dashboard
    (Settings → My integrations → New integration → Internal).

CRITICAL UX NOTE
    Notion integrations don't have implicit access to your
    workspace content. After creating the integration, you
    MUST explicitly share each page or database you want
    Aivyx PA to see — via the Notion UI's Share → Invite menu,
    selecting your integration. Without this step, all API
    calls return empty results or 404."
}

#[cfg(test)]
mod tests {
    use super::*;

    // The bulk of CLI parsing semantics is tested in the
    // `aivyx-auth-cli` substrate's own tests. The tests
    // here cover only what's service-specific to
    // `aivyx-notion`: the help text contents (Notion's
    // critical "share with integration" UX note must be
    // surfaced) and the binary-name threading.

    #[test]
    fn help_text_mentions_share_with_integration() {
        let txt = help_text();
        assert!(txt.contains("MUST explicitly share"));
        assert!(txt.contains("integration"));
    }

    #[test]
    fn help_text_mentions_config_path() {
        let txt = help_text();
        assert!(txt.contains("tool-processes/notion/config.toml"));
        assert!(txt.contains("notion_token"));
    }

    #[test]
    fn parse_cli_args_threads_binary_name_into_errors() {
        let argv = vec!["aivyx-notion".to_string(), "auth".to_string()];
        let e = parse_cli_args_from(&argv).expect_err("must error");
        assert!(e.contains("aivyx-notion"), "{e}");
    }
}
