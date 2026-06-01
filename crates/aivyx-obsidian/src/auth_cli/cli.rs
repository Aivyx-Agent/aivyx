//! CLI argument parsing for `aivyx-obsidian`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinaryMode {
    Help,
    Check,
    IpcLoop,
}

pub fn parse_cli_args_from(argv: &[String]) -> Result<BinaryMode, String> {
    let args: Vec<&str> = argv.iter().skip(1).map(String::as_str).collect();
    match args.as_slice() {
        [] => Ok(BinaryMode::IpcLoop),
        ["help"] | ["--help"] | ["-h"] => Ok(BinaryMode::Help),
        ["auth", "check"] | ["check"] => Ok(BinaryMode::Check),
        ["auth"] => Err(
            "`aivyx-obsidian auth` requires a subcommand; try `auth check`."
                .to_string(),
        ),
        _ => Err(format!(
            "unknown subcommand: {}. Run `aivyx-obsidian help` for usage.",
            args.join(" ")
        )),
    }
}

pub fn help_text() -> &'static str {
    "aivyx-obsidian — Obsidian vault third-party tool process for Aivyx

USAGE:
    aivyx-obsidian [SUBCOMMAND]

SUBCOMMANDS:
    auth check     Verify the configured vault_path exists,
                   is a directory, and is readable.
    help           Show this text.

When run with no arguments, the binary enters IPC-loop mode
(spawned by the Aivyx daemon via [[tool_process]] in
aivyx.toml).

CONFIG FILE
    Expected at ~/.aivyx/tool-processes/obsidian/config.toml
    with:

        vault_path = \"/absolute/path/to/MyVault\"

    Use an absolute path. The path-traversal guard rejects
    any operator-supplied path that would escape this root."
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(rest: &[&str]) -> Vec<String> {
        let mut v = vec!["aivyx-obsidian".to_string()];
        v.extend(rest.iter().map(|s| s.to_string()));
        v
    }

    #[test]
    fn no_args_means_ipc_loop() {
        assert_eq!(parse_cli_args_from(&argv(&[])).unwrap(), BinaryMode::IpcLoop);
    }

    #[test]
    fn help_variants_parse() {
        for h in ["help", "--help", "-h"] {
            assert_eq!(parse_cli_args_from(&argv(&[h])).unwrap(), BinaryMode::Help);
        }
    }

    #[test]
    fn check_parses_both_forms() {
        assert_eq!(
            parse_cli_args_from(&argv(&["auth", "check"])).unwrap(),
            BinaryMode::Check
        );
        assert_eq!(
            parse_cli_args_from(&argv(&["check"])).unwrap(),
            BinaryMode::Check
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
    fn help_text_mentions_vault_path_and_traversal_guard() {
        let txt = help_text();
        assert!(txt.contains("vault_path"));
        assert!(txt.contains("path-traversal guard"));
        assert!(txt.contains("absolute path"));
    }
}
