//! `aivyx-kitchen-toolkit` binary entry point — Chapter Brigade (BG.1).
//!
//! The daemon spawns this via `[[tool_process]]` in `aivyx.toml`. On startup we:
//! 1. load the operator config from `~/.aivyx/tool-processes/kitchen/config.toml`
//!    (KitchenDB base_url + api_key + organization_id);
//! 2. build the shared `reqwest::Client` + the `KitchenClient`;
//! 3. register the `kitchen.read` tools and hand them to the multi-tool harness.
//!
//! Startup failures (missing config, `$HOME` unset, missing `[kitchen_db]`)
//! exit non-zero with an operator-actionable message so the daemon's
//! tool-process bridge reports a clear failure.

use std::process::ExitCode;
use std::sync::Arc;

use aivyx_core::Tool;
use aivyx_kitchen_toolkit::config::{default_config_path, load_config};
use aivyx_kitchen_toolkit::tools::{
    BatchComplete, BatchStart, InventoryAdjust, InventoryList, InventoryLowStock,
    InventoryValue, RecipeSearch, SupplierList,
};
use aivyx_kitchen_toolkit::{run_multi_tool_subprocess, KitchenClient};

#[tokio::main]
async fn main() -> ExitCode {
    let config_path = match default_config_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("aivyx-kitchen-toolkit: {e}");
            return ExitCode::from(2);
        }
    };
    let db = match load_config(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("aivyx-kitchen-toolkit: {e}");
            return ExitCode::from(2);
        }
    };

    let http = reqwest::Client::new();
    let client = Arc::new(KitchenClient::new(
        http,
        db.base_url,
        db.api_key,
        db.organization_id,
    ));

    let tools: Vec<Arc<dyn Tool>> = vec![
        // BG.1 — kitchen.read.
        Arc::new(InventoryList::new(Arc::clone(&client))),
        Arc::new(InventoryLowStock::new(Arc::clone(&client))),
        Arc::new(InventoryValue::new(Arc::clone(&client))),
        Arc::new(RecipeSearch::new(Arc::clone(&client))),
        Arc::new(SupplierList::new(Arc::clone(&client))),
        // BG.2 — kitchen.write.
        Arc::new(InventoryAdjust::new(Arc::clone(&client))),
        Arc::new(BatchStart::new(Arc::clone(&client))),
        Arc::new(BatchComplete::new(Arc::clone(&client))),
    ];

    match run_multi_tool_subprocess(tools, "aivyx-kitchen-toolkit").await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("aivyx-kitchen-toolkit: harness exited with error: {e}");
            ExitCode::from(1)
        }
    }
}
