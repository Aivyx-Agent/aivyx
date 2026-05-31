//! Multi-tool IPC harness — Phase 128 lift to substrate.
//!
//! Phase 123 (`aivyx-gmail`) was the first reusable
//! consumer of a multi-tool IPC harness; Phase 125 (this
//! crate) was the second copy; Phase 128 (`aivyx-calendar`)
//! would have been the third. Per Phase 125's own exit
//! recommendation, the implementation has been **lifted
//! to [`aivyx_tool::multi_harness`]** — all three (now
//! four) Chapter F / Chapter G crates share one substrate.
//!
//! This module is a thin re-export shim preserving
//! `aivyx_toolkit::HarnessError` /
//! `aivyx_toolkit::run_multi_tool_subprocess` for any
//! external consumer of the toolkit public API. The
//! implementation lives in `aivyx_tool::multi_harness`.

pub use aivyx_tool::multi_harness::{run_multi_tool_subprocess, HarnessError};
