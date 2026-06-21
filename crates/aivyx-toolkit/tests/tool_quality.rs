//! Chapter Abacus (AB.4) — metadata quality sweep over the
//! **tool-process** tier (`aivyx-toolkit`).
//!
//! Atlas (AT.3) shipped `check_tool_quality` and swept the substrate +
//! infrastructure tiers, recommending the out-of-process tool-process
//! crates run the same guard in their own tests. This is the toolkit's
//! sweep: every tool the binary registers must meet the name /
//! description / schema correctness floor the LLM relies on, so a thin
//! description or malformed schema fails CI rather than shipping. It
//! builds the full set exactly as `main.rs` does (cheap scratch stores
//! for the stateful tools; the metadata methods never touch them).

use std::sync::Arc;

use aivyx_core::tools::check_tool_quality;
use aivyx_core::Tool;
use aivyx_toolkit::budget_store::BudgetStore;
use aivyx_toolkit::health_store::HealthStore;
use aivyx_toolkit::task_store::TaskStore;
use aivyx_toolkit::tools::{
    BudgetCategoriesTool, BudgetDelete, BudgetRecord, BudgetSummaryTool,
    BudgetTrendTool, BudgetUpdate, CalcEval, ConvertTime, ConvertUnits, DateAdd,
    DateDiff, HealthCheckAdd, HealthCheckList, HealthCheckRecentChanges,
    HealthCheckRemove, TaskComplete, TaskCreate, TaskDelete, TaskList, WebSearch,
};

#[tokio::test]
async fn toolkit_tools_meet_quality_floor() {
    let dir = std::env::temp_dir().join(format!("aivyx-toolkit-quality-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("scratch dir");

    let task_store = Arc::new(TaskStore::open(dir.join("tasks.json")).await.expect("task store"));
    let health_store =
        Arc::new(HealthStore::open(dir.join("health.json")).await.expect("health store"));
    let budget_store =
        Arc::new(BudgetStore::open(dir.join("budget.json")).await.expect("budget store"));

    let tools: Vec<Arc<dyn Tool>> = vec![
        Arc::new(WebSearch::new(reqwest::Client::new(), String::new())),
        Arc::new(TaskCreate::new(Arc::clone(&task_store))),
        Arc::new(TaskList::new(Arc::clone(&task_store))),
        Arc::new(TaskComplete::new(Arc::clone(&task_store))),
        Arc::new(TaskDelete::new(Arc::clone(&task_store))),
        Arc::new(HealthCheckAdd::new(Arc::clone(&health_store))),
        Arc::new(HealthCheckList::new(Arc::clone(&health_store))),
        Arc::new(HealthCheckRecentChanges::new(Arc::clone(&health_store))),
        Arc::new(HealthCheckRemove::new(Arc::clone(&health_store))),
        Arc::new(BudgetRecord::new(Arc::clone(&budget_store))),
        Arc::new(BudgetSummaryTool::new(Arc::clone(&budget_store))),
        Arc::new(BudgetUpdate::new(Arc::clone(&budget_store))),
        Arc::new(BudgetDelete::new(Arc::clone(&budget_store))),
        Arc::new(BudgetTrendTool::new(Arc::clone(&budget_store))),
        Arc::new(BudgetCategoriesTool::new(Arc::clone(&budget_store))),
        // Chapter Abacus — pure-compute utilities (AB.1–AB.3).
        Arc::new(CalcEval::new()),
        Arc::new(ConvertUnits::new()),
        Arc::new(ConvertTime::new()),
        Arc::new(DateDiff::new()),
        Arc::new(DateAdd::new()),
    ];

    let issues: Vec<String> = tools
        .iter()
        .flat_map(|t| check_tool_quality(t.as_ref()))
        .collect();

    let _ = std::fs::remove_dir_all(&dir);

    assert!(
        issues.is_empty(),
        "toolkit tool quality issues:\n  {}",
        issues.join("\n  "),
    );
}
