//! `aivyx-gmail` binary entry point.
//!
//! Phase 123 Task 3 ships the operator-facing CLI dispatch
//! (`auth init`, `auth status`, `auth revoke`). The IPC loop
//! (the no-args mode the daemon spawns via `[[tool_process]]`)
//! still falls through to a stub here — Tasks 4-7 replace it.

use std::process::ExitCode;
use std::time::Duration;

use std::sync::Arc;

use aivyx_gmail::auth_cli::{
    cli::{help_text, parse_cli_args_from, AuthMode, BinaryMode},
    config_file::{default_config_path, load_oauth_config},
    init::{generate_state_token, run_auth_init},
    revoke::{run_auth_revoke, GOOGLE_REVOKE_ENDPOINT},
    status::run_auth_status,
};
use aivyx_gmail::gmail_client::GmailClient;
use aivyx_gmail::harness::run_multi_tool_subprocess;
use aivyx_gmail::oauth::{load_tokens, storage::default_token_path};
use aivyx_gmail::tools::GmailSearch;
use aivyx_core::Tool;

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
        BinaryMode::IpcLoop => run_ipc_loop().await,
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

/// Daemon-spawned IPC loop. Loads OAuth config + tokens,
/// constructs the shared GmailClient, registers every Gmail
/// tool, and hands the registry to
/// [`run_multi_tool_subprocess`] which drives the IPC loop
/// against stdin/stdout. Returns only on `ToolShutdown` or
/// stream EOF.
async fn run_ipc_loop() -> ExitCode {
    let config_path = match default_config_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("aivyx-gmail (ipc): {e}");
            return ExitCode::from(2);
        }
    };
    let oauth_config = match load_oauth_config(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("aivyx-gmail (ipc): {e}");
            return ExitCode::from(2);
        }
    };
    let token_path = match default_token_path() {
        Some(p) => p,
        None => {
            eprintln!("aivyx-gmail (ipc): $HOME unset; cannot resolve token path");
            return ExitCode::from(2);
        }
    };
    let tokens = match load_tokens(&token_path).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            eprintln!(
                "aivyx-gmail (ipc): no tokens at {token_path:?} — run `aivyx-gmail auth init` first"
            );
            return ExitCode::from(2);
        }
        Err(e) => {
            eprintln!("aivyx-gmail (ipc): token load failed: {e}");
            return ExitCode::from(2);
        }
    };

    let client = Arc::new(GmailClient::new(
        reqwest::Client::new(),
        oauth_config,
        tokens,
        token_path,
    ));

    // Task 4: gmail.search. Tasks 5-7 will push read / draft /
    // send into this Vec.
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(GmailSearch::new(Arc::clone(&client))),
    ];

    match run_multi_tool_subprocess(tools, "aivyx-gmail").await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("aivyx-gmail (ipc): harness exited with error: {e}");
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
