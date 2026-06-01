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
use aivyx_obsidian::tools::{ObsidianCreateNote, ObsidianDeleteNote, ObsidianGetNote, ObsidianListFolder, ObsidianSearch, ObsidianUpdateNote};
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
    let client = match VaultClient::new(cfg) {
        Ok(c) => Arc::new(c),
        Err(e) => {
            eprintln!("aivyx-obsidian (ipc): vault open failed: {e}");
            return ExitCode::from(2);
        }
    };

    // Phase 130 Q2a — six-tool surface. Tasks 11-16
    // populate this incrementally; Task 11 adds search.
    let tools: Vec<Arc<dyn Tool>> = vec![Arc::new(ObsidianSearch::new(Arc::clone(&client))), Arc::new(ObsidianGetNote::new(Arc::clone(&client))), Arc::new(ObsidianListFolder::new(Arc::clone(&client))), Arc::new(ObsidianCreateNote::new(Arc::clone(&client))), Arc::new(ObsidianUpdateNote::new(Arc::clone(&client))), Arc::new(ObsidianDeleteNote::new(Arc::clone(&client)))];

    match run_multi_tool_subprocess(tools, "aivyx-obsidian").await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("aivyx-obsidian (ipc): harness exited with error: {e}");
            ExitCode::from(1)
        }
    }
}
