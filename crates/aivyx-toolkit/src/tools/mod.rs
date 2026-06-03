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

pub mod budget;
pub mod health_check;
pub mod tasks;
pub mod web_search;

pub use budget::{BudgetDelete, BudgetRecord, BudgetSummaryTool, BudgetUpdate};
pub use health_check::{HealthCheckAdd, HealthCheckList, HealthCheckRecentChanges};
pub use tasks::{TaskComplete, TaskCreate, TaskDelete, TaskList};
pub use web_search::WebSearch;
