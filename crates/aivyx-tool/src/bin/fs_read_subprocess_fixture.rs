//! Phase 50 conformance fixture — wraps `FsReadTool` as a tool
//! process using `run_tool_as_subprocess`.
//!
//! Built only as a test/conformance binary. The Phase 50 Task 5
//! integration test spawns this and drives it through
//! `ToolProcessBridge`, then asserts that the same input through
//! the in-process `FsReadTool::execute(...)` path produces an
//! equivalent `ToolOutcome`.
//!
//! The binary takes the sandbox root as `argv[1]`. Operators do
//! not run this directly.
//!
//! ## Why this matters
//!
//! This binary is the canonical proof of **PRODUCT.md P12**:
//! "first-party tools speak the same protocol third-party tools
//! speak — extractable without rewriting." `FsReadTool` is
//! unchanged from its in-tree form; the only difference is the
//! `run_tool_as_subprocess` wrapper.

use std::process::ExitCode;

use aivyx_core::FsReadToolConfig;
use aivyx_tool::run_tool_as_subprocess;

#[tokio::main]
async fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    let sandbox_root = match args.next() {
        Some(p) => p,
        None => {
            eprintln!(
                "fs_read_subprocess_fixture: missing sandbox-root argument. \
                 Usage: fs_read_subprocess_fixture <sandbox-root>"
            );
            return ExitCode::from(2);
        }
    };

    let tool = match FsReadToolConfig::new(&sandbox_root).build() {
        Ok(t) => t,
        Err(e) => {
            eprintln!(
                "fs_read_subprocess_fixture: failed to build FsReadTool for \
                 sandbox {sandbox_root:?}: {e}"
            );
            return ExitCode::from(3);
        }
    };

    match run_tool_as_subprocess(tool, "fs.read-subprocess-fixture").await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fs_read_subprocess_fixture: harness error: {e}");
            ExitCode::from(4)
        }
    }
}
