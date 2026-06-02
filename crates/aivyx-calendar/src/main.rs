//! `aivyx-calendar` binary entry point.
//!
//! Phase 128 Task 3 ships the operator-facing CLI dispatch
//! (`auth init`, `auth status`, `auth revoke`) plus the
//! IPC-loop scaffolding. Per-tool implementations land in
//! Tasks 4-8; until then the IPC mode registers zero tools
//! and exits with a clear "no tools yet" message so the
//! binary is operator-installable for the auth flow before
//! the full tool surface is ready.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use aivyx_calendar::auth_cli::{
    cli::{help_text, parse_cli_args_from, AuthMode, BinaryMode},
    config_file::{default_config_path, load_oauth_config},
    init::{generate_state_token, run_auth_init},
    revoke::{run_auth_revoke, GOOGLE_REVOKE_ENDPOINT},
    status::run_auth_status,
};
use aivyx_calendar::oauth::{load_tokens, storage::default_token_path};
use aivyx_calendar::tools::{
    CalendarCreateEvent, CalendarDeleteEvent, CalendarGetEvent, CalendarListEvents,
    CalendarUpcoming, CalendarUpdateEvent,
};
use aivyx_calendar::{run_multi_tool_subprocess, CalendarClient};
use aivyx_core::Tool;

/// Operator-facing default for how long `auth init` waits
/// for the browser callback. Mirrors aivyx-gmail's 5-minute
/// budget; accommodates slow consent flows (2FA, scope
/// re-grant, etc).
const AUTH_INIT_CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);

#[tokio::main]
async fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    let mode = match parse_cli_args_from(&argv) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("aivyx-calendar: {e}");
            eprintln!("Run `aivyx-calendar help` for usage.");
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
            eprintln!("aivyx-calendar auth init: {e}");
            return ExitCode::from(2);
        }
    };
    let config = match load_oauth_config(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("aivyx-calendar auth init: {e}");
            return ExitCode::from(2);
        }
    };
    let token_path = match default_token_path() {
        Some(p) => p,
        None => {
            eprintln!(
                "aivyx-calendar auth init: $HOME unset; cannot resolve token path"
            );
            return ExitCode::from(2);
        }
    };
    let state = generate_state_token();
    match run_auth_init(&config, &token_path, AUTH_INIT_CALLBACK_TIMEOUT, &state).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("aivyx-calendar auth init: {e}");
            ExitCode::from(1)
        }
    }
}

async fn run_status() -> ExitCode {
    let token_path = match default_token_path() {
        Some(p) => p,
        None => {
            eprintln!(
                "aivyx-calendar auth status: $HOME unset; cannot resolve token path"
            );
            return ExitCode::from(2);
        }
    };
    match run_auth_status(&token_path).await {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("aivyx-calendar auth status: {e}");
            ExitCode::from(1)
        }
    }
}

/// Daemon-spawned IPC loop. Loads OAuth config + tokens,
/// constructs the shared `CalendarClient`, registers every
/// Calendar tool, and hands the registry to the lifted
/// multi-tool harness which drives the IPC loop against
/// stdin/stdout. Returns only on `ToolShutdown` or stream
/// EOF.
///
/// **Phase 128 Task 3:** the per-tool registrations land
/// in Tasks 4-8. Until then this function loads the config
/// and tokens so we know the auth substrate works, then
/// exits with a clear "no tools yet" message rather than
/// invoking the harness with an empty tool list (which
/// would error with `HarnessError::EmptyToolList`).
async fn run_ipc_loop() -> ExitCode {
    let config_path = match default_config_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("aivyx-calendar (ipc): {e}");
            return ExitCode::from(2);
        }
    };
    let oauth_config = match load_oauth_config(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("aivyx-calendar (ipc): {e}");
            return ExitCode::from(2);
        }
    };
    let token_path = match default_token_path() {
        Some(p) => p,
        None => {
            eprintln!(
                "aivyx-calendar (ipc): $HOME unset; cannot resolve token path"
            );
            return ExitCode::from(2);
        }
    };
    let tokens = match load_tokens(&token_path).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            eprintln!(
                "aivyx-calendar (ipc): no tokens at {token_path:?} — run `aivyx-calendar auth init` first"
            );
            return ExitCode::from(2);
        }
        Err(e) => {
            eprintln!("aivyx-calendar (ipc): token load failed: {e}");
            return ExitCode::from(2);
        }
    };

    let client = Arc::new(CalendarClient::new(
        reqwest::Client::new(),
        oauth_config,
        tokens,
        token_path,
    ));

    // Phase 128 Q3b — five-tool surface (list / get /
    // create / update / delete). Phase 141 adds a sixth
    // read-side tool: calendar.upcoming, the LLM-
    // ergonomic shape for "what's coming up in the next
    // N hours" queries.
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(CalendarListEvents::new(Arc::clone(&client))),
        Arc::new(CalendarGetEvent::new(Arc::clone(&client))),
        Arc::new(CalendarCreateEvent::new(Arc::clone(&client))),
        Arc::new(CalendarUpdateEvent::new(Arc::clone(&client))),
        Arc::new(CalendarDeleteEvent::new(Arc::clone(&client))),
        Arc::new(CalendarUpcoming::new(Arc::clone(&client))),
    ];

    match run_multi_tool_subprocess(tools, "aivyx-calendar").await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("aivyx-calendar (ipc): harness exited with error: {e}");
            ExitCode::from(1)
        }
    }
}

async fn run_revoke() -> ExitCode {
    let token_path = match default_token_path() {
        Some(p) => p,
        None => {
            eprintln!(
                "aivyx-calendar auth revoke: $HOME unset; cannot resolve token path"
            );
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
            eprintln!("aivyx-calendar auth revoke: {e}");
            ExitCode::from(1)
        }
    }
}
