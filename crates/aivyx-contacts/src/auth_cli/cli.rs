//! CLI argument parsing for `aivyx-contacts auth ...`.
//!
//! Hand-rolled to keep the dep tree minimal (no `clap` /
//! `argh`). Mirrors the parsing pattern used in
//! `crates/aivyx-channel/src/bin/aivyx.rs` so contributors
//! reading both binaries see the same shape.

/// One of the three operator-facing auth subcommands, or `None`
/// when the binary was invoked for the IPC loop (no args /
/// Tasks 4-7 dispatch).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthMode {
    /// `aivyx-contacts auth init` — run the OAuth flow.
    Init,
    /// `aivyx-contacts auth status` — print token health.
    Status,
    /// `aivyx-contacts auth revoke` — revoke + delete tokens.
    Revoke,
}

/// What the binary should do after parsing argv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinaryMode {
    /// Operator-facing auth subcommand.
    Auth(AuthMode),
    /// No args / unrecognized — Tasks 4-7 will route this to
    /// the IPC loop. For Task 3 it falls through to the stub.
    IpcLoop,
    /// Help text request (`-h` / `--help` / `help`).
    Help,
}

/// Parse argv into a [`BinaryMode`].
///
/// `argv` is the full argv vector (`argv[0]` is the program
/// name; `argv[1..]` are the user-supplied args).
pub fn parse_cli_args_from(argv: &[String]) -> Result<BinaryMode, String> {
    // No args → IPC loop (daemon invocation pattern).
    if argv.len() < 2 {
        return Ok(BinaryMode::IpcLoop);
    }

    let first = argv[1].as_str();
    match first {
        "-h" | "--help" | "help" => Ok(BinaryMode::Help),
        "auth" => parse_auth(&argv[2..]),
        // Unrecognized first arg — surface an error so an
        // operator typo doesn't silently fall through to the
        // IPC loop (which would then hang waiting for stdin).
        other => Err(format!(
            "unrecognized subcommand {other:?}; \
             valid: `auth init`, `auth status`, `auth revoke`, `help`"
        )),
    }
}

fn parse_auth(args: &[String]) -> Result<BinaryMode, String> {
    let sub = args.first().map(String::as_str).unwrap_or("");
    match sub {
        "init" => Ok(BinaryMode::Auth(AuthMode::Init)),
        "status" => Ok(BinaryMode::Auth(AuthMode::Status)),
        "revoke" => Ok(BinaryMode::Auth(AuthMode::Revoke)),
        "" => Err(
            "`aivyx-contacts auth` requires a subcommand; \
             valid: `init`, `status`, `revoke`"
                .to_string(),
        ),
        other => Err(format!(
            "unrecognized auth subcommand {other:?}; \
             valid: `init`, `status`, `revoke`"
        )),
    }
}

/// Render the operator-facing help text. Lives next to the
/// parser so a future flag addition stays in lockstep.
pub fn help_text() -> &'static str {
    "aivyx-contacts — Contacts third-party tool process for Aivyx PA

USAGE:
    aivyx-contacts [SUBCOMMAND]

SUBCOMMANDS:
    auth init      Run the Google OAuth consent flow and save
                   tokens to `~/.aivyx-pa/tool-processes/contacts/tokens.json`.
                   Requires `~/.aivyx-pa/tool-processes/contacts/config.toml`
                   with the operator's OAuth client_id +
                   client_secret + redirect_uri.

    auth status    Print token health (granted scopes, expiry,
                   refresh-available?).

    auth revoke    Call Google's revoke endpoint and delete the
                   local token file. `auth init` must be re-run
                   before any Contacts tool will work.

    help           Print this text.

When invoked with NO subcommand, the binary enters the Aivyx PA
tool-process IPC loop and expects to be spawned by the Aivyx PA
daemon via a
[[tool_process]] entry in aivyx-pa.toml. Operators should not
invoke this mode directly.
"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(args: &[&str]) -> Vec<String> {
        std::iter::once("aivyx-contacts")
            .chain(args.iter().copied())
            .map(String::from)
            .collect()
    }

    #[test]
    fn empty_args_parses_as_ipc_loop() {
        let p = parse_cli_args_from(&argv(&[])).expect("parse");
        assert_eq!(p, BinaryMode::IpcLoop);
    }

    #[test]
    fn help_alias_short() {
        let p = parse_cli_args_from(&argv(&["-h"])).expect("parse");
        assert_eq!(p, BinaryMode::Help);
    }

    #[test]
    fn help_alias_long() {
        let p = parse_cli_args_from(&argv(&["--help"])).expect("parse");
        assert_eq!(p, BinaryMode::Help);
    }

    #[test]
    fn help_alias_bareword() {
        let p = parse_cli_args_from(&argv(&["help"])).expect("parse");
        assert_eq!(p, BinaryMode::Help);
    }

    #[test]
    fn auth_init_parses() {
        let p = parse_cli_args_from(&argv(&["auth", "init"])).expect("parse");
        assert_eq!(p, BinaryMode::Auth(AuthMode::Init));
    }

    #[test]
    fn auth_status_parses() {
        let p = parse_cli_args_from(&argv(&["auth", "status"])).expect("parse");
        assert_eq!(p, BinaryMode::Auth(AuthMode::Status));
    }

    #[test]
    fn auth_revoke_parses() {
        let p = parse_cli_args_from(&argv(&["auth", "revoke"])).expect("parse");
        assert_eq!(p, BinaryMode::Auth(AuthMode::Revoke));
    }

    #[test]
    fn unrecognized_top_level_subcommand_errors() {
        let e = parse_cli_args_from(&argv(&["bogus"])).expect_err("must error");
        assert!(e.contains("unrecognized"), "{e}");
        assert!(e.contains("auth init"), "help text should list valid: {e}");
    }

    #[test]
    fn auth_without_subcommand_errors() {
        let e = parse_cli_args_from(&argv(&["auth"])).expect_err("must error");
        assert!(e.contains("requires a subcommand"), "{e}");
        assert!(e.contains("init"), "should list valid sub: {e}");
    }

    #[test]
    fn auth_with_unknown_subcommand_errors() {
        let e =
            parse_cli_args_from(&argv(&["auth", "bogus"])).expect_err("must error");
        assert!(e.contains("unrecognized auth subcommand"), "{e}");
    }

    #[test]
    fn help_text_documents_all_subcommands_and_default_path() {
        let txt = help_text();
        assert!(txt.contains("auth init"));
        assert!(txt.contains("auth status"));
        assert!(txt.contains("auth revoke"));
        // The help text must mention the config file path so
        // an operator hitting "missing config" knows where to
        // create the file.
        assert!(txt.contains("config.toml"));
        // And the tokens path so an operator who runs `auth
        // status` and sees nothing knows where to look.
        assert!(txt.contains("tokens.json"));
    }
}
