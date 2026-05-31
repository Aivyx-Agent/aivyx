//! Multi-tool IPC harness — Phase 128 lift to substrate.
//!
//! Phase 123 (this crate) was the first reusable consumer
//! of a multi-tool IPC harness; Phase 125 (`aivyx-toolkit`)
//! was the second; Phase 128 (`aivyx-calendar`) would have
//! been the third copy. Per Phase 125's exit
//! recommendation, the implementation has been **lifted to
//! [`aivyx_tool::multi_harness`]** — all three (now four)
//! Chapter F / Chapter G crates share one substrate.
//!
//! This module is a thin re-export shim preserving
//! `aivyx_gmail::HarnessError` /
//! `aivyx_gmail::run_multi_tool_subprocess` for any
//! external consumer of the gmail public API. The
//! implementation lives in `aivyx_tool::multi_harness`.

pub use aivyx_tool::multi_harness::{run_multi_tool_subprocess, HarnessError};
