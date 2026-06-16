//! `aivyx-contacts` binary entry point.
//!
//! CT.2 ships the operator-facing CLI dispatch (`auth init /
//! status / revoke`) plus the IPC-loop scaffolding. The six
//! People API tool impls land in CT.3 (read) + CT.4 (write).
//! Until then the IPC mode registers zero tools and exits
//! cleanly, so the binary is operator-installable for the auth
//! flow before the full tool surface is ready (the same staging
//! aivyx-drive used at its Task 3).

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use aivyx_contacts::auth_cli::{
    cli::{help_text, parse_cli_args_from, AuthMode, BinaryMode},
    config_file::{default_config_path, load_oauth_config},
    init::{generate_state_token, run_auth_init},
    revoke::{run_auth_revoke, GOOGLE_REVOKE_ENDPOINT},
    status::run_auth_status,
};
use aivyx_contacts::{
    default_token_path, load_tokens, run_multi_tool_subprocess, ContactsClient,
};
use aivyx_core::Tool;

/// Operator-facing default for how long `auth init` waits for
/// the browser callback. Mirrors aivyx-drive's 5-minute budget.
const AUTH_INIT_CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);

#[tokio::main]
async fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    let mode = match parse_cli_args_from(&argv) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("aivyx-contacts: {e}");
            eprintln!("Run `aivyx-contacts help` for usage.");
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
        BinaryMode::IpcLoop => run_ipc_loop().await,
    }
}

async fn run_init() -> ExitCode {
    let config_path = match default_config_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("aivyx-contacts auth init: {e}");
            return ExitCode::from(2);
        }
    };
    let config = match load_oauth_config(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("aivyx-contacts auth init: {e}");
            return ExitCode::from(2);
        }
    };
    let token_path = match default_token_path() {
        Some(p) => p,
        None => {
            eprintln!("aivyx-contacts auth init: $HOME unset; cannot resolve token path");
            return ExitCode::from(2);
        }
    };
    let state = generate_state_token();
    match run_auth_init(&config, &token_path, AUTH_INIT_CALLBACK_TIMEOUT, &state).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("aivyx-contacts auth init: {e}");
            ExitCode::from(1)
        }
    }
}

async fn run_status() -> ExitCode {
    let token_path = match default_token_path() {
        Some(p) => p,
        None => {
            eprintln!("aivyx-contacts auth status: $HOME unset; cannot resolve token path");
            return ExitCode::from(2);
        }
    };
    match run_auth_status(&token_path).await {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("aivyx-contacts auth status: {e}");
            ExitCode::from(1)
        }
    }
}

async fn run_ipc_loop() -> ExitCode {
    let config_path = match default_config_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("aivyx-contacts (ipc): {e}");
            return ExitCode::from(2);
        }
    };
    let oauth_config = match load_oauth_config(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("aivyx-contacts (ipc): {e}");
            return ExitCode::from(2);
        }
    };
    let token_path = match default_token_path() {
        Some(p) => p,
        None => {
            eprintln!("aivyx-contacts (ipc): $HOME unset; cannot resolve token path");
            return ExitCode::from(2);
        }
    };
    let tokens = match load_tokens(&token_path).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            eprintln!(
                "aivyx-contacts (ipc): no tokens at {token_path:?} — run `aivyx-contacts auth init` first"
            );
            return ExitCode::from(2);
        }
        Err(e) => {
            eprintln!("aivyx-contacts (ipc): token load failed: {e}");
            return ExitCode::from(2);
        }
    };

    let client = Arc::new(ContactsClient::new(
        reqwest::Client::new(),
        oauth_config,
        tokens,
        token_path,
    ));

    // CT.2 scaffold — the six People API tools land in CT.3
    // (search / list / get) + CT.4 (create / update / delete).
    // `client` is constructed (token refresh + path wiring
    // verified) but not yet handed to any tool.
    let _ = &client;
    let tools: Vec<Arc<dyn Tool>> = vec![];

    match run_multi_tool_subprocess(tools, "aivyx-contacts").await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("aivyx-contacts (ipc): harness exited with error: {e}");
            ExitCode::from(1)
        }
    }
}

async fn run_revoke() -> ExitCode {
    let token_path = match default_token_path() {
        Some(p) => p,
        None => {
            eprintln!("aivyx-contacts auth revoke: $HOME unset; cannot resolve token path");
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
            eprintln!("aivyx-contacts auth revoke: {e}");
            ExitCode::from(1)
        }
    }
}
