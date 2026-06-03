//! # aivyx-toolkit
//!
//! Operator-facing personal assistant tool bundle for Aivyx.
//! Chapter G #1 — Phase 125. Ships as a single binary
//! registering 8 tools through Phase 123's multi-tool harness
//! substrate (per PRODUCT.md P10 — Aivyx core stays at the
//! thirteen-tools-forever cap; operator-facing capability
//! beyond substrate is third-party).
//!
//! ## Layout (Phase 125 Task 2 — this commit)
//!
//! - [`config`] — operator-supplied config at
//!   `~/.aivyx/tool-processes/toolkit/config.toml`. Holds
//!   the Brave Search API key + any per-tool tunables.
//! - [`harness`] — multi-tool IPC dispatcher (same shape as
//!   `aivyx-gmail`'s harness module; the Phase 123
//!   SDK-validation lift remains outstanding).
//!
//! ## Layout (Phase 125 Tasks 3-6 — future commits)
//!
//! - `tools::web_search` — Task 3: `web.search` (Brave API).
//! - `tools::task_*` — Task 4: `task.create/list/complete/delete`.
//! - `health::polling` — Task 5: tool-process-side polling
//!   loop substrate.
//! - `tools::health_check` — Task 6:
//!   `health.check.add/list/recent_changes`.
//!
//! ## Trust model
//!
//! All 8 tools require Trusted-tier scopes by default
//! (`web.search`, `task.read`, `task.write`, `health.read`,
//! `health.write` — all in
//! `aivyx_capability::CEILING_TRUSTED` only). SemiTrusted /
//! Untrusted roles get zero personal-assistant scopes by
//! default; operators who want narrow access from a remote
//! channel must grant individual bases via the role's
//! `capability_scopes` per Phase 62 Q2(a).

pub mod budget_store;
pub mod config;
pub mod harness;
pub mod health_polling;
pub mod health_store;
pub mod secure_io;
pub mod task_store;
pub mod tools;

pub use budget_store::{
    BudgetEntry, BudgetStore, BudgetStoreError, BudgetSummary, CategoryTotal,
};
pub use config::{ConfigFileError, ToolkitConfig};
pub use harness::{run_multi_tool_subprocess, HarnessError};
pub use health_polling::{probe, run_polling_loop, run_polling_tick};
pub use health_store::{
    HealthStore, HealthStoreError, ProbeOutcome, Transition, Watcher, WatcherState,
};
pub use task_store::{Task, TaskStatus, TaskStore, TaskStoreError};
pub use tools::{
    BudgetRecord, BudgetSummaryTool, HealthCheckAdd, HealthCheckList,
    HealthCheckRecentChanges, TaskComplete, TaskCreate, TaskDelete, TaskList,
    WebSearch,
};
