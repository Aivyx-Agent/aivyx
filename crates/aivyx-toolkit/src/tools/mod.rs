//! Tool-trait implementations registered by the IPC harness.
//!
//! Phase 125 — Chapter G #1. Each tool implements
//! [`aivyx_core::Tool`] so it can be served by the multi-tool
//! IPC harness (see [`crate::harness`]).
//!
//! ## Module layout
//!
//! - [`web_search`] — Task 3: `web.search` (Brave Search API,
//!   `web.search` scope).
//! - [`tasks`] — Task 4: `task.create / list / complete /
//!   delete` (`task.read` + `task.write` scopes).
//! - [`health_check`] — Task 6: `health.check.add / list /
//!   recent_changes` (`health.read` + `health.write` scopes;
//!   wraps Task 5's polling-loop substrate).
//! - [`calc`] — Chapter Abacus (AB.1): `calc.eval`, a
//!   pure-compute arithmetic evaluator (`calc.eval` scope,
//!   SemiTrusted-reachable — no I/O, no data).
//! - [`convert`] — Chapter Abacus (AB.2): `convert.units` +
//!   `convert.time`, pure-compute unit + timezone conversion
//!   (both gated by the `convert.units` group scope,
//!   SemiTrusted-reachable).
//! - [`date`] — Chapter Abacus (AB.3): `date.diff` + `date.add`,
//!   calendar-correct date arithmetic (both gated by the
//!   `date.compute` group scope, SemiTrusted-reachable; a missing
//!   date defaults to now — the pack's one clock dependence).

pub mod budget;
pub mod calc;
pub mod convert;
pub mod date;
pub mod health_check;
pub mod tasks;
pub mod web_search;

pub use budget::{
    BudgetCategoriesTool, BudgetDelete, BudgetRecord, BudgetSummaryTool,
    BudgetTrendTool, BudgetUpdate,
};
pub use calc::CalcEval;
pub use convert::{ConvertTime, ConvertUnits};
pub use date::{DateAdd, DateDiff};
pub use health_check::{
    HealthCheckAdd, HealthCheckList, HealthCheckRecentChanges, HealthCheckRemove,
};
pub use tasks::{TaskComplete, TaskCreate, TaskDelete, TaskList};
pub use web_search::WebSearch;
