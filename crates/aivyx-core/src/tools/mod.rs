//! Concrete tool implementations that ship with the Aivyx core crate.
//!
//! ## Why tools live in `aivyx-core` (for now)
//!
//! Phase 4 Q1 asked whether a filesystem tool should live in `aivyx-core`
//! (keeping D8's 9-crate workspace lock), in a new `aivyx-tool-fs` crate
//! (requiring a D8 amendment), or in an `aivyx-tools` umbrella. The
//! resolution at Phase 4 task 2 entry is **option 1**: a small `tools`
//! sub-module inside `aivyx-core`. The reasoning, to be re-evaluated at
//! Phase 4 exit and carried into Phase 6:
//!
//! - Every type the filesystem tool needs (`Tool`, `ToolContext`,
//!   `ToolOutcome`, `Scope`, `Verification`, `AivyxError`) already lives
//!   in `aivyx-core`. Spinning up a new crate just to get a `pub use`
//!   of these types back would be pure overhead.
//! - One concrete tool is not evidence of a "tools umbrella" pattern.
//!   The right time to split is when there are at least two concrete
//!   tools and the split pays for itself in compile-time isolation or
//!   test-surface isolation. Today there's one — move it later, once
//!   Phase 6's memory-as-tool work gives us a second data point.
//! - D8's 9-crate lock stands without amendment, which keeps the
//!   DESIGN.md empty-diff streak alive (3 phases, heading for 4).
//!
//! The trade is that `aivyx-core` starts to accrete a "standard library
//! of tools" surface. Phase 4 exit will note this and either ratify it
//! or propose an amendment with real evidence behind it.

pub mod fs;
pub mod git;
pub mod net_dns;
pub mod role_switch;
pub mod shell;
pub mod web_fetch;

pub use fs::{
    FsDeleteTool, FsDeleteToolConfig, FsMetadataTool, FsMetadataToolConfig, FsReadTool,
    FsReadToolConfig, FsWriteTool, FsWriteToolConfig,
};
pub use git::{GitDiffTool, GitReadToolConfig, GitStatusTool};
pub use net_dns::NetDnsTool;
pub use role_switch::{ChildAgentFactory, RoleSwitchTool};
pub use shell::{ShellExecTool, ShellExecToolConfig};
pub use web_fetch::{WebFetchTool, WebFetchToolConfig, WebPostTool, WebPostToolConfig};
