//! `aivyx-gmail` binary entry point.
//!
//! Phase 123 Task 3 ships the operator-facing CLI dispatch
//! (`auth init`, `auth status`, `auth revoke`). The IPC loop
//! (the no-args mode the daemon spawns via `[[tool_process]]`)
//! still falls through to a stub here — Tasks 4-7 replace it.

use std::process::ExitCode;
use std::time::Duration;

use aivyx_gmail::auth_cli::{
    cli::{help_text, parse_cli_args_from, AuthMode, BinaryMode},
    config_file::{default_config_path, load_oauth_config},
    init::{generate_state_token, run_auth_init},
    revoke::{run_auth_revoke, GOOGLE_REVOKE_ENDPOINT},
    status::run_auth_status,
};
use aivyx_gmail::oauth::storage::default_token_path;

/// Operator-facing default for how long `auth init` waits for
/// the browser callback. 5 minutes accommodates a slow consent
/// flow (2FA prompts, re-auth, re-typing scopes, etc).
const AUTH_INIT_CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);

#[tokio::main]
async fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    let mode = match parse_cli_args_from(&argv) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("aivyx-gmail: {e}");
            eprintln!("Run `aivyx-gmail help` for usage.");
            return ExitCode::from(2);
        }
    };

    match mode {
        BinaryMode::Help => {
            println!("{}", help_text());
            ExitCode::SUCCESS
        }
        BinaryMode::Auth(AuthMode::Init) => run_init().await,
        BinaryMode::Auth(AuthMode::Status) => run_status().await,
        BinaryMode::Auth(AuthMode::Revoke) => run_revoke().await,
        BinaryMode::IpcLoop => {
            // Tasks 4-7 will replace this with the multi-tool
            // IPC harness loop. For Phase 123 Task 3, an
            // operator-actionable error so a daemon
            // `[[tool_process]]` spawn doesn't silently hang
            // waiting for stdin.
            eprintln!(
                "aivyx-gmail: the IPC tool-process loop lands in Phase 123 Tasks 4-7.\n\
                 Task 3 ships the operator-facing CLI only (`auth init / status / revoke`).\n\
                 Once Tasks 4-7 ship, the daemon's `[[tool_process]]` spawn will land here\n\
                 with no args and run the IPC handler. For now, this exits non-zero so the\n\
                 daemon's tool-process bridge surfaces a clear startup failure."
            );
            ExitCode::from(2)
        }
    }
}

async fn run_init() -> ExitCode {
    let config_path = match default_config_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("aivyx-gmail auth init: {e}");
            return ExitCode::from(2);
        }
    };
    let config = match load_oauth_config(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("aivyx-gmail auth init: {e}");
            return ExitCode::from(2);
        }
    };
    let token_path = match default_token_path() {
        Some(p) => p,
        None => {
            eprintln!("aivyx-gmail auth init: $HOME unset; cannot resolve token path");
            return ExitCode::from(2);
        }
    };
    let state = generate_state_token();
    match run_auth_init(&config, &token_path, AUTH_INIT_CALLBACK_TIMEOUT, &state).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("aivyx-gmail auth init: {e}");
            ExitCode::from(1)
        }
    }
}

async fn run_status() -> ExitCode {
    let token_path = match default_token_path() {
        Some(p) => p,
        None => {
            eprintln!("aivyx-gmail auth status: $HOME unset; cannot resolve token path");
            return ExitCode::from(2);
        }
    };
    match run_auth_status(&token_path).await {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("aivyx-gmail auth status: {e}");
            ExitCode::from(1)
        }
    }
}

async fn run_revoke() -> ExitCode {
    let token_path = match default_token_path() {
        Some(p) => p,
        None => {
            eprintln!("aivyx-gmail auth revoke: $HOME unset; cannot resolve token path");
            return ExitCode::from(2);
        }
    };
    let client = reqwest::Client::new();
    match run_auth_revoke(&client, GOOGLE_REVOKE_ENDPOINT, &token_path).await {
        Ok(outcome) => {
            println!("{}", outcome.message);
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("aivyx-gmail auth revoke: {e}");
            ExitCode::from(1)
        }
    }
}
