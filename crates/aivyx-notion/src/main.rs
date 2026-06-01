//! `aivyx-notion` binary entry point.
//!
//! Phase 130 Task 2 ships the operator-facing CLI dispatch
//! plus the IPC-loop scaffolding. Per-tool implementations
//! land in Tasks 3-9. Until then the IPC mode loads the
//! config (verifies the token is there) but registers zero
//! tools and exits cleanly.

use std::process::ExitCode;
use std::sync::Arc;

use aivyx_core::Tool;
use aivyx_notion::auth_cli::{
    cli::{help_text, parse_cli_args_from, AuthMode, BinaryMode},
    config_file::{default_config_path, load_config},
    status::{run_auth_check, run_auth_status},
};
use aivyx_notion::tools::{
    NotionCreatePage, NotionGetPage, NotionListDatabase, NotionSearch,
};
use aivyx_notion::{run_multi_tool_subprocess, NotionClient};

#[tokio::main]
async fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    let mode = match parse_cli_args_from(&argv) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("aivyx-notion: {e}");
            eprintln!("Run `aivyx-notion help` for usage.");
            return ExitCode::from(2);
        }
    };

    match mode {
        BinaryMode::Help => {
            println!("{}", help_text());
            ExitCode::SUCCESS
        }
        BinaryMode::Auth(AuthMode::Status) => run_status_cmd(),
        BinaryMode::Auth(AuthMode::Check) => run_check_cmd().await,
        BinaryMode::IpcLoop => run_ipc_loop().await,
    }
}

fn run_status_cmd() -> ExitCode {
    let config_path = match default_config_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("aivyx-notion auth status: {e}");
            return ExitCode::from(2);
        }
    };
    match run_auth_status(&config_path) {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("aivyx-notion auth status: {e}");
            ExitCode::from(1)
        }
    }
}

async fn run_check_cmd() -> ExitCode {
    let config_path = match default_config_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("aivyx-notion auth check: {e}");
            return ExitCode::from(2);
        }
    };
    let config = match load_config(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("aivyx-notion auth check: {e}");
            return ExitCode::from(2);
        }
    };
    let client = NotionClient::new(reqwest::Client::new(), config);
    let report = run_auth_check(&client).await;
    let ok = report.token_works;
    print!("{report}");
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

async fn run_ipc_loop() -> ExitCode {
    let config_path = match default_config_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("aivyx-notion (ipc): {e}");
            return ExitCode::from(2);
        }
    };
    let config = match load_config(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("aivyx-notion (ipc): {e}");
            return ExitCode::from(2);
        }
    };
    let client = Arc::new(NotionClient::new(reqwest::Client::new(), config));

    // Phase 130 Q1a — seven-tool surface (search /
    // get_page / list_database / create_page /
    // append_blocks / update_page_properties /
    // archive_page). Tasks 3-9 populate this vector
    // incrementally; Task 3 adds search.
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(NotionSearch::new(Arc::clone(&client))),
        Arc::new(NotionGetPage::new(Arc::clone(&client))),
        Arc::new(NotionListDatabase::new(Arc::clone(&client))),
        Arc::new(NotionCreatePage::new(Arc::clone(&client))),
    ];

    match run_multi_tool_subprocess(tools, "aivyx-notion").await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("aivyx-notion (ipc): harness exited with error: {e}");
            ExitCode::from(1)
        }
    }
}
