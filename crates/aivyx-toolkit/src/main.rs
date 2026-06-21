//! `aivyx-toolkit` binary entry point.
//!
//! Phase 125 Task 6 — full IPC loop wiring. The daemon spawns
//! this binary via `[[tool_process]]` in `aivyx.toml`; on
//! start-up we:
//!
//! 1. Load the operator config from
//!    `~/.aivyx/tool-processes/toolkit/config.toml`.
//! 2. Open the task store + health store at the toolkit
//!    state directory.
//! 3. Build the shared `reqwest::Client` used by `web.search`
//!    and the health polling loop.
//! 4. Spawn the health polling loop in a background tokio
//!    task (tokio aborts it when main returns on
//!    ToolShutdown).
//! 5. Register all 20 tools into a single
//!    `Vec<Arc<dyn Tool>>` and hand to
//!    `run_multi_tool_subprocess`. Phase 125
//!    shipped 8 tools; Phases 143, 144, 147,
//!    149, 150 expanded the surface with
//!    budget CRUD + trend + categories +
//!    health.check.remove; Chapter Abacus
//!    adds calc.eval (AB.1) + convert.units +
//!    convert.time (AB.2) + date.diff +
//!    date.add (AB.3).
//!
//! Operator-facing failure modes are surfaced at startup
//! (missing config file, $HOME unset, etc) with operator-
//! actionable error messages and non-zero exit so the
//! daemon's tool-process bridge reports a clear failure.

use std::process::ExitCode;
use std::sync::Arc;

use aivyx_core::Tool;
use aivyx_toolkit::budget_store::BudgetStore;
use aivyx_toolkit::config::{default_config_path, default_state_dir, load_config};
use aivyx_toolkit::health_polling::run_polling_loop;
use aivyx_toolkit::health_store::HealthStore;
use aivyx_toolkit::task_store::TaskStore;
use aivyx_toolkit::tools::{
    BudgetCategoriesTool, BudgetDelete, BudgetRecord, BudgetSummaryTool,
    BudgetTrendTool, BudgetUpdate, CalcEval, ConvertTime, ConvertUnits, DateAdd,
    DateDiff, HealthCheckAdd, HealthCheckList, HealthCheckRecentChanges,
    HealthCheckRemove, TaskComplete, TaskCreate, TaskDelete, TaskList, WebSearch,
};
use aivyx_toolkit::{run_multi_tool_subprocess, ToolkitConfig};

#[tokio::main]
async fn main() -> ExitCode {
    let config_path = match default_config_path() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("aivyx-toolkit: {e}");
            return ExitCode::from(2);
        }
    };
    let config: ToolkitConfig = match load_config(&config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("aivyx-toolkit: {e}");
            return ExitCode::from(2);
        }
    };

    let state_dir = match default_state_dir() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("aivyx-toolkit: {e}");
            return ExitCode::from(2);
        }
    };

    let task_store = match TaskStore::open(state_dir.join("tasks.json")).await {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("aivyx-toolkit: failed to open task store: {e}");
            return ExitCode::from(2);
        }
    };
    let health_store = match HealthStore::open(state_dir.join("health.json")).await {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("aivyx-toolkit: failed to open health store: {e}");
            return ExitCode::from(2);
        }
    };
    let budget_store = match BudgetStore::open(state_dir.join("budget.json")).await {
        Ok(s) => Arc::new(s),
        Err(e) => {
            eprintln!("aivyx-toolkit: failed to open budget store: {e}");
            return ExitCode::from(2);
        }
    };

    // Shared HTTP client for web.search and the health
    // polling loop. Brave's API + arbitrary watcher URLs are
    // routed through the same reqwest connection pool.
    let http = reqwest::Client::new();

    // Spawn the health polling loop in the background. Tokio
    // aborts the task when main() returns on ToolShutdown
    // (the harness loop terminates which returns from main).
    let polling_store = Arc::clone(&health_store);
    let polling_http = http.clone();
    tokio::spawn(async move {
        run_polling_loop(polling_store, polling_http).await
    });

    // Build the eight registered tools. `web.search` reports
    // its API-key absence at first invocation rather than at
    // startup so an operator who only uses task.* or
    // health.check.* tools doesn't have to populate a Brave
    // key.
    let brave_api_key = config
        .brave_search
        .as_ref()
        .map(|c| c.api_key.clone())
        .unwrap_or_default();

    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(WebSearch::new(http.clone(), brave_api_key)),
        Arc::new(TaskCreate::new(Arc::clone(&task_store))),
        Arc::new(TaskList::new(Arc::clone(&task_store))),
        Arc::new(TaskComplete::new(Arc::clone(&task_store))),
        Arc::new(TaskDelete::new(Arc::clone(&task_store))),
        Arc::new(HealthCheckAdd::new(Arc::clone(&health_store))),
        Arc::new(HealthCheckList::new(Arc::clone(&health_store))),
        Arc::new(HealthCheckRecentChanges::new(Arc::clone(&health_store))),
        // Phase 147 — Chapter G #2 final candidate.
        Arc::new(HealthCheckRemove::new(Arc::clone(&health_store))),
        // Phase 143 — Chapter G #2 budget tracking
        // (record + summary). Phase 144 reaches CRUD
        // parity with update + delete. Phase 149 adds
        // trend for month-over-month delta queries.
        // Phase 150 adds categories enumeration +
        // silent case-fold normalization +
        // category_suggestion enrichment on record/
        // update outputs.
        Arc::new(BudgetRecord::new(Arc::clone(&budget_store))),
        Arc::new(BudgetSummaryTool::new(Arc::clone(&budget_store))),
        Arc::new(BudgetUpdate::new(Arc::clone(&budget_store))),
        Arc::new(BudgetDelete::new(Arc::clone(&budget_store))),
        Arc::new(BudgetTrendTool::new(Arc::clone(&budget_store))),
        Arc::new(BudgetCategoriesTool::new(Arc::clone(&budget_store))),
        // Chapter Abacus — pure-compute utilities. No store, no keys,
        // no network; SemiTrusted-reachable (see docs/ABACUS.md §2).
        // AB.1: calc.eval. AB.2: convert.units + convert.time.
        // AB.3: date.diff + date.add.
        Arc::new(CalcEval::new()),
        Arc::new(ConvertUnits::new()),
        Arc::new(ConvertTime::new()),
        Arc::new(DateDiff::new()),
        Arc::new(DateAdd::new()),
    ];

    match run_multi_tool_subprocess(tools, "aivyx-toolkit").await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("aivyx-toolkit: harness exited with error: {e}");
            ExitCode::from(1)
        }
    }
}
