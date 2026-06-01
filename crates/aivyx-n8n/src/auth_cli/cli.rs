//! CLI argument parsing for `aivyx-n8n`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinaryMode {
    Help,
    Auth(AuthMode),
    IpcLoop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMode {
    Status,
    Check,
}

pub fn parse_cli_args_from(argv: &[String]) -> Result<BinaryMode, String> {
    let args: Vec<&str> = argv.iter().skip(1).map(String::as_str).collect();
    match args.as_slice() {
        [] => Ok(BinaryMode::IpcLoop),
        ["help"] | ["--help"] | ["-h"] => Ok(BinaryMode::Help),
        ["auth"] => Err(
            "`aivyx-n8n auth` requires a subcommand; try `auth status` or `auth check`."
                .to_string(),
        ),
        ["auth", "status"] => Ok(BinaryMode::Auth(AuthMode::Status)),
        ["auth", "check"] => Ok(BinaryMode::Auth(AuthMode::Check)),
        _ => Err(format!(
            "unknown subcommand: {}. Run `aivyx-n8n help` for usage.",
            args.join(" ")
        )),
    }
}

pub fn help_text() -> &'static str {
    "aivyx-n8n — n8n workflow automation third-party tool process for Aivyx

USAGE:
    aivyx-n8n [SUBCOMMAND]

SUBCOMMANDS:
    auth status    Check config.toml exists and the
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

    fn argv(rest: &[&str]) -> Vec<String> {
        let mut v = vec!["aivyx-n8n".to_string()];
        v.extend(rest.iter().map(|s| s.to_string()));
        v
    }

    #[test]
    fn no_args_means_ipc_loop() {
        assert_eq!(parse_cli_args_from(&argv(&[])).unwrap(), BinaryMode::IpcLoop);
    }

    #[test]
    fn help_variants() {
        for h in ["help", "--help", "-h"] {
            assert_eq!(parse_cli_args_from(&argv(&[h])).unwrap(), BinaryMode::Help);
        }
    }

    #[test]
    fn auth_subcommands_parse() {
        assert_eq!(
            parse_cli_args_from(&argv(&["auth", "status"])).unwrap(),
            BinaryMode::Auth(AuthMode::Status)
        );
        assert_eq!(
            parse_cli_args_from(&argv(&["auth", "check"])).unwrap(),
            BinaryMode::Auth(AuthMode::Check)
        );
    }

    #[test]
    fn bare_auth_errors() {
        let e = parse_cli_args_from(&argv(&["auth"])).expect_err("must error");
        assert!(e.contains("subcommand"), "{e}");
    }

    #[test]
    fn unknown_errors() {
        let e = parse_cli_args_from(&argv(&["wat"])).expect_err("must error");
        assert!(e.contains("unknown"), "{e}");
    }

    #[test]
    fn help_text_mentions_config_fields() {
        let txt = help_text();
        assert!(txt.contains("n8n_base_url"));
        assert!(txt.contains("n8n_api_key"));
        assert!(txt.contains("Settings"));
    }
}
