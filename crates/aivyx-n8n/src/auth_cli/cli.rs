//! CLI argument parsing for `aivyx-n8n`.
//!
//! Thin wrapper over `aivyx_auth_cli` — Phase 132 lift.
//! Service-specific bits stay here: binary name in
//! errors, `help_text` with the n8n-specific config
//! shape (operator-supplied base URL + API key).

pub use aivyx_auth_cli::{AuthMode, BinaryMode};

const BINARY_NAME: &str = "aivyx-n8n";

pub fn parse_cli_args_from(argv: &[String]) -> Result<BinaryMode, String> {
    aivyx_auth_cli::parse_cli_args(argv, BINARY_NAME)
}

pub fn help_text() -> &'static str {
    "aivyx-n8n — n8n workflow automation third-party tool process for Aivyx

USAGE:
    aivyx-n8n [SUBCOMMAND]

SUBCOMMANDS:
    auth status    Check that config.toml exists and the
                   n8n_base_url + n8n_api_key fields are
                   non-empty. Static; no network call.
    auth check     Ping n8n's /api/v1/workflows?limit=1 to
                   verify the API key has API access.
    help           Show this text.

When run with no arguments, the binary enters IPC-loop mode.

CONFIG FILE
    Expected at ~/.aivyx/tool-processes/n8n/config.toml with:

        n8n_base_url = \"https://n8n.example.com\"
        n8n_api_key = \"ntn_XXXX\"

    Get the API key from n8n's Settings → API → Create API key.
    The base URL points at your self-hosted n8n instance — no
    trailing slash needed."
}

#[cfg(test)]
mod tests {
    use super::*;

    // Argument-parsing semantics are tested at the
    // substrate level. The tests here cover n8n-specific
    // bits: help text contents and binary-name threading.

    #[test]
    fn help_text_mentions_both_config_fields() {
        let txt = help_text();
        assert!(txt.contains("n8n_base_url"));
        assert!(txt.contains("n8n_api_key"));
    }

    #[test]
    fn help_text_mentions_settings_api_key_path() {
        let txt = help_text();
        assert!(txt.contains("Settings → API"));
    }

    #[test]
    fn parse_cli_args_threads_binary_name_into_errors() {
        let argv = vec!["aivyx-n8n".to_string(), "auth".to_string()];
        let e = parse_cli_args_from(&argv).expect_err("must error");
        assert!(e.contains("aivyx-n8n"), "{e}");
    }
}
