//! `aivyx-obsidian` binary entry point.
//!
//! Phase 130 Task 10 ships the skeleton. Per-tool
//! implementations land in Tasks 11-16.

use std::process::ExitCode;
use std::sync::Arc;

use aivyx_core::Tool;
use aivyx_obsidian::auth_cli::{
    check::run_auth_check,
    cli::{help_text, parse_cli_args_from, BinaryMode},
    config_file::{default_config_path, load_config},
};
use aivyx_obsidian::{run_multi_tool_subprocess, VaultClient};

#[tokio::main]
async fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    let mode = match parse_cli_args_from(&argv) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("aivyx-obsidian: {e}");
            eprintln!("Run `aivyx-obsidian help` for usage.");
            return ExitCode::from(2);
        }
    };

    match mode {
        BinaryMode::Help => {
            println!("{}", help_text());
            ExitCode::SUCCESS
        }
        BinaryMode::Check => {
            let config_path = match default_config_path() {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("aivyx-obsidian auth check: {e}");
                    return ExitCode::from(2);
                }
            };
            let report = run_auth_check(&config_path);
            let ok = report.vault_ok;
            print!("{report}");
            if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
        BinaryMode::IpcLoop => run_ipc_loop().await,
    }
}

async fn run_ipc_loop() -> ExitCode {
    let config_path = match default_config_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("aivyx-obsidian (ipc): {e}");
            return ExitCode::from(2);
        }
    };
    let cfg = match load_config(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("aivyx-obsidian (ipc): {e}");
            return ExitCode::from(2);
        }
    };
    let _client = match VaultClient::new(cfg) {
        Ok(c) => Arc::new(c),
        Err(e) => {
            eprintln!("aivyx-obsidian (ipc): vault open failed: {e}");
            return ExitCode::from(2);
        }
    };

    // Phase 130 Q2a — six-tool surface (search /
    // get_note / list_folder / create_note / update_note /
    // delete_note). Tasks 11-16 populate this incrementally;
    // Task 10 ships the scaffolding only.
    let tools: Vec<Arc<dyn Tool>> = Vec::new();

    if tools.is_empty() {
        eprintln!(
            "aivyx-obsidian (ipc): Phase 130 Task 10 scaffold — \
             no tools registered yet (Tasks 11-16 add them). Exiting cleanly."
        );
        return ExitCode::SUCCESS;
    }

    match run_multi_tool_subprocess(tools, "aivyx-obsidian").await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("aivyx-obsidian (ipc): harness exited with error: {e}");
            ExitCode::from(1)
        }
    }
}
