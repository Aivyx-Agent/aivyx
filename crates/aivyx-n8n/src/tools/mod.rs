//! `aivyx_core::Tool` implementations for the n8n tool
//! process.
//!
//! Phase 131 Q1c operator-picked surface (10 tools).
//! Per-tool modules ship in Tasks 3-12.

pub mod activate_workflow;
pub mod create_workflow;
pub mod deactivate_workflow;
pub mod execute_workflow;
pub mod get_execution;
pub mod get_workflow;
pub mod list_executions;
pub mod list_workflows;
pub mod update_workflow;

pub use activate_workflow::N8nActivateWorkflow;
pub use create_workflow::N8nCreateWorkflow;
pub use deactivate_workflow::N8nDeactivateWorkflow;
pub use execute_workflow::N8nExecuteWorkflow;
pub use get_execution::N8nGetExecution;
pub use get_workflow::N8nGetWorkflow;
pub use list_executions::N8nListExecutions;
pub use list_workflows::N8nListWorkflows;
pub use update_workflow::N8nUpdateWorkflow;
