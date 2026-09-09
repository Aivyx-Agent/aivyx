//! CLI argument parsing for `aivyx-obsidian`.
//!
//! Thin wrapper over `aivyx_auth_cli` — Phase 132 lift.
//! Adopts the substrate's `BinaryMode::Auth(AuthMode)`
//! shape so Obsidian's CLI surface aligns with
//! `aivyx-notion` and `aivyx-n8n`. The previous
//! `check` shorthand (deprecated in favour of the
//! explicit `auth check` form) is dropped.

pub use aivyx_auth_cli::{AuthMode, BinaryMode};

const BINARY_NAME: &str = "aivyx-obsidian";

pub fn parse_cli_args_from(argv: &[String]) -> Result<BinaryMode, String> {
    aivyx_auth_cli::parse_cli_args(argv, BINARY_NAME)
}

pub fn help_text() -> &'static str {
    "aivyx-obsidian — Obsidian vault third-party tool process for Aivyx PA

USAGE:
    aivyx-obsidian [SUBCOMMAND]

SUBCOMMANDS:
    auth status    Check that config.toml exists, parses,
                   and the vault_path field is a valid
                   absolute path. Offline; no filesystem
                   probe.
    auth check     Verify the configured vault_path exists,
                   is a directory, and is readable. Hits the
                   filesystem.
    help           Show this text.

When run with no arguments, the binary enters IPC-loop mode
(spawned by the Aivyx PA daemon via [[tool_process]] in
aivyx-pa.toml).

CONFIG FILE
    Expected at ~/.aivyx-pa/tool-processes/obsidian/config.toml
    with:

        vault_path = \"/absolute/path/to/MyVault\"

    Use an absolute path. The path-traversal guard rejects
    any operator-supplied path that would escape this root."
}

#[cfg(test)]
mod tests {
    use super::*;

    // Argument-parsing semantics are tested at the
    // substrate level (`aivyx-auth-cli`). The tests here
    // cover Obsidian-specific bits: help text contents
    // and binary-name threading.

    #[test]
    fn help_text_mentions_vault_path_and_traversal_guard() {
        let txt = help_text();
        assert!(txt.contains("vault_path"));
        assert!(txt.contains("path-traversal guard"));
        assert!(txt.contains("absolute path"));
    }

    #[test]
    fn help_text_distinguishes_status_from_check() {
        let txt = help_text();
        assert!(txt.contains("auth status"));
        assert!(txt.contains("auth check"));
        assert!(txt.contains("Offline"));
        assert!(txt.contains("filesystem"));
    }

    #[test]
    fn parse_cli_args_threads_binary_name_into_errors() {
        let argv = vec!["aivyx-obsidian".to_string(), "wat".to_string()];
        let e = parse_cli_args_from(&argv).expect_err("must error");
        assert!(e.contains("aivyx-obsidian"), "{e}");
    }
}
