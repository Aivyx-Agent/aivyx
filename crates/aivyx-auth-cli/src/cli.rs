//! CLI argument parsing — shared shape for every
//! Chapter F third-party tool process.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinaryMode {
    Help,
    Auth(AuthMode),
    IpcLoop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMode {
    /// Offline status check — does the config file
    /// exist with the required fields non-empty? No
    /// network call.
    Status,
    /// Online check — actively reach the configured
    /// service (HTTP endpoint, filesystem path, etc.)
    /// to confirm the configured credentials work.
    Check,
}

/// Parse a `Vec<String>` (from `std::env::args`) into a
/// [`BinaryMode`]. The binary name is threaded in so
/// error messages can surface the correct
/// `aivyx-<service>` prefix without each consumer
/// reimplementing the parser.
///
/// ## Argument shape
///
/// - `[]` (no args after the binary name) →
///   `BinaryMode::IpcLoop`. The Aivyx daemon spawns
///   third-party tool processes in this mode via
///   `[[tool_process]]` in `aivyx.toml`.
/// - `["help"]` / `["--help"]` / `["-h"]` →
///   `BinaryMode::Help`.
/// - `["auth"]` → error (subcommand required).
/// - `["auth", "status"]` →
///   `BinaryMode::Auth(AuthMode::Status)`.
/// - `["auth", "check"]` →
///   `BinaryMode::Auth(AuthMode::Check)`.
/// - anything else → error.
pub fn parse_cli_args(
    argv: &[String],
    binary_name: &str,
) -> Result<BinaryMode, String> {
    // Skip argv[0] (the binary path itself).
    let args: Vec<&str> = argv.iter().skip(1).map(String::as_str).collect();
    match args.as_slice() {
        [] => Ok(BinaryMode::IpcLoop),
        ["help"] | ["--help"] | ["-h"] => Ok(BinaryMode::Help),
        ["auth"] => Err(format!(
            "`{binary_name} auth` requires a subcommand; try \
             `auth status` or `auth check`. Run `{binary_name} help` for usage."
        )),
        ["auth", "status"] => Ok(BinaryMode::Auth(AuthMode::Status)),
        ["auth", "check"] => Ok(BinaryMode::Auth(AuthMode::Check)),
        _ => Err(format!(
            "unknown subcommand: {}. Run `{binary_name} help` for usage.",
            args.join(" ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(binary: &str, rest: &[&str]) -> Vec<String> {
        let mut v = vec![binary.to_string()];
        v.extend(rest.iter().map(|s| s.to_string()));
        v
    }

    #[test]
    fn no_args_means_ipc_loop() {
        assert_eq!(
            parse_cli_args(&argv("aivyx-notion", &[]), "aivyx-notion").unwrap(),
            BinaryMode::IpcLoop
        );
    }

    #[test]
    fn help_variants_all_parse() {
        for variant in ["help", "--help", "-h"] {
            assert_eq!(
                parse_cli_args(&argv("aivyx-x", &[variant]), "aivyx-x").unwrap(),
                BinaryMode::Help,
                "variant {variant:?}",
            );
        }
    }

    #[test]
    fn auth_status_parses() {
        assert_eq!(
            parse_cli_args(&argv("aivyx-x", &["auth", "status"]), "aivyx-x").unwrap(),
            BinaryMode::Auth(AuthMode::Status),
        );
    }

    #[test]
    fn auth_check_parses() {
        assert_eq!(
            parse_cli_args(&argv("aivyx-x", &["auth", "check"]), "aivyx-x").unwrap(),
            BinaryMode::Auth(AuthMode::Check),
        );
    }

    #[test]
    fn bare_auth_errors_with_actionable_message() {
        let e = parse_cli_args(&argv("aivyx-n8n", &["auth"]), "aivyx-n8n")
            .expect_err("must error");
        assert!(e.contains("aivyx-n8n"), "{e}");
        assert!(e.contains("subcommand"), "{e}");
        assert!(e.contains("status"), "{e}");
        assert!(e.contains("check"), "{e}");
    }

    #[test]
    fn unknown_subcommand_errors_with_binary_name() {
        let e = parse_cli_args(&argv("aivyx-obsidian", &["wat"]), "aivyx-obsidian")
            .expect_err("must error");
        assert!(e.contains("unknown"), "{e}");
        assert!(e.contains("aivyx-obsidian"), "{e}");
    }

    #[test]
    fn binary_name_threads_through_to_error_messages() {
        // Two distinct binaries; the binary name in the error
        // message reflects what the caller passed in.
        let e_notion =
            parse_cli_args(&argv("ignored", &["auth"]), "aivyx-notion").expect_err("e");
        let e_n8n = parse_cli_args(&argv("ignored", &["auth"]), "aivyx-n8n").expect_err("e");
        assert!(e_notion.contains("aivyx-notion"));
        assert!(e_n8n.contains("aivyx-n8n"));
        assert!(!e_notion.contains("aivyx-n8n"));
        assert!(!e_n8n.contains("aivyx-notion"));
    }
}
