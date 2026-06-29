//! `aivyx-apps` binary entry point (Chapter Deckhand).
//!
//! The daemon spawns this binary via `[[tool_process]]` in `aivyx.toml` when
//! the operator opts in with `[applications]`. It registers the six `app.*`
//! tools and runs the multi-tool harness (same shape as `aivyx-toolkit`).
//!
//! No config file, no state, no network: the tools shell out to `xdotool`
//! (and a screenshot CLI) on demand and report a missing-binary error with an
//! install hint at call time, so an operator who hasn't installed the tools
//! still gets a clean message rather than a startup failure.

use std::process::ExitCode;
use std::sync::Arc;

use aivyx_apps::tools::{AppClick, AppFocus, AppKey, AppList, AppScreenshot, AppType};
use aivyx_core::Tool;
use aivyx_tool::run_multi_tool_subprocess;

#[tokio::main]
async fn main() -> ExitCode {
    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(AppList::new()),
        Arc::new(AppScreenshot::new()),
        Arc::new(AppFocus::new()),
        Arc::new(AppType::new()),
        Arc::new(AppKey::new()),
        Arc::new(AppClick::new()),
    ];

    match run_multi_tool_subprocess(tools, "aivyx-apps").await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("aivyx-apps: harness exited with error: {e}");
            ExitCode::from(1)
        }
    }
}
