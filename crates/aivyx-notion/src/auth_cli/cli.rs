//! CLI argument parsing for `aivyx-notion`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinaryMode {
    Help,
    Auth(AuthMode),
    IpcLoop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMode {
    /// Check whether the config file is present and the
    /// token is non-empty. Static check; no network call.
    Status,
    /// Actively poll Notion's `/users/me` endpoint to
    /// confirm the token has API access. Surfaces
    /// "not_shared" errors distinctly so the operator can
    /// fix the sharing step.
    Check,
}

pub fn parse_cli_args_from(argv: &[String]) -> Result<BinaryMode, String> {
    // Skip argv[0] (the binary path).
    let args: Vec<&str> = argv.iter().skip(1).map(String::as_str).collect();
    match args.as_slice() {
        [] => Ok(BinaryMode::IpcLoop),
        ["help"] | ["--help"] | ["-h"] => Ok(BinaryMode::Help),
        ["auth"] => Err(
            "`aivyx-notion auth` requires a subcommand; try \
             `auth status` or `auth check`. Run `aivyx-notion help` for usage."
                .to_string(),
        ),
        ["auth", "status"] => Ok(BinaryMode::Auth(AuthMode::Status)),
        ["auth", "check"] => Ok(BinaryMode::Auth(AuthMode::Check)),
        _ => Err(format!(
            "unknown subcommand: {}. Run `aivyx-notion help` for usage.",
            args.join(" ")
        )),
    }
}

pub fn help_text() -> &'static str {
    "aivyx-notion — Notion third-party tool process for Aivyx

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
(spawned by the Aivyx daemon via [[tool_process]] in
aivyx.toml).

CONFIG FILE
    Expected at ~/.aivyx/tool-processes/notion/config.toml
    with:

        notion_token = \"ntn_XXXXXXXXXX\"

    Obtain the token from Notion's Integrations dashboard
    (Settings → My integrations → New integration → Internal).

CRITICAL UX NOTE
    Notion integrations don't have implicit access to your
    workspace content. After creating the integration, you
    MUST explicitly share each page or database you want
    Aivyx to see — via the Notion UI's Share → Invite menu,
    selecting your integration. Without this step, all API
    calls return empty results or 404."
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(rest: &[&str]) -> Vec<String> {
        let mut v = vec!["aivyx-notion".to_string()];
        v.extend(rest.iter().map(|s| s.to_string()));
        v
    }

    #[test]
    fn no_args_means_ipc_loop() {
        assert_eq!(
            parse_cli_args_from(&argv(&[])).unwrap(),
            BinaryMode::IpcLoop
        );
    }

    #[test]
    fn help_variants_map_to_help() {
        for variant in ["help", "--help", "-h"] {
            assert_eq!(
                parse_cli_args_from(&argv(&[variant])).unwrap(),
                BinaryMode::Help,
                "variant {variant:?}"
            );
        }
    }

    #[test]
    fn auth_status_parses() {
        assert_eq!(
            parse_cli_args_from(&argv(&["auth", "status"])).unwrap(),
            BinaryMode::Auth(AuthMode::Status)
        );
    }

    #[test]
    fn auth_check_parses() {
        assert_eq!(
            parse_cli_args_from(&argv(&["auth", "check"])).unwrap(),
            BinaryMode::Auth(AuthMode::Check)
        );
    }

    #[test]
    fn bare_auth_errors_with_actionable_message() {
        let e = parse_cli_args_from(&argv(&["auth"])).expect_err("must error");
        assert!(e.contains("subcommand"), "{e}");
        assert!(e.contains("status"), "{e}");
        assert!(e.contains("check"), "{e}");
    }

    #[test]
    fn unknown_subcommand_errors() {
        let e = parse_cli_args_from(&argv(&["wat"])).expect_err("must error");
        assert!(e.contains("unknown"), "{e}");
    }

    #[test]
    fn help_text_mentions_share_with_integration() {
        let txt = help_text();
        // The critical UX note must be in the help output
        // so operators see it without scrolling INSTALL.md.
        assert!(txt.contains("MUST explicitly share"));
        assert!(txt.contains("integration"));
    }

    #[test]
    fn help_text_mentions_config_path() {
        let txt = help_text();
        assert!(txt.contains("tool-processes/notion/config.toml"));
        assert!(txt.contains("notion_token"));
    }
}
