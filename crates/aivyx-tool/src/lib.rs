//! # aivyx-tool
//!
//! Tool process IPC bridge — third-party tools as separate OS
//! processes, spawned by the daemon at startup, communicating via
//! length-prefixed JSON over stdin/stdout. See
//! [`docs/TOOL_SDK.md`](../../../docs/TOOL_SDK.md) for the contract.
//!
//! Phase 49 — Tool Process IPC Foundation (delivers PRODUCT.md P12).
//! Foundation phase: ships the third-party path. First-party
//! in-process unification is deferred (Phase 49 Q5).
//!
//! The crate is structured the same way `aivyx-mcp` is:
//!
//! - [`wire`] — the protocol message envelopes (`DaemonToTool`,
//!   `ToolToDaemon`, `ToolDescriptor`, `Verification`).
//! - [`frame`] — length-prefixed JSON I/O over async streams.
//! - [`bridge`] — `ToolProcessBridge` spawns + manages a child
//!   process and dispatches invocations.
//! - Task 4 will add `ToolProxy` (an `aivyx_core::Tool` impl that
//!   delegates to a bridge) here at the crate root.

pub mod bridge;
pub mod frame;
pub mod harness;
pub mod multi_harness;
pub mod proxy;
pub mod sandbox;
pub mod wire;

pub use bridge::{
    InvocationOutcome, SandboxConfig, ToolBridgeError, ToolProcessBridge,
    ToolProcessConfig,
};
pub use sandbox::{
    detect_sandbox_backend, preset_for, resolve_sandbox,
    SandboxBackend, SandboxChoice,
};
pub use frame::{encode_frame, read_frame, write_frame, FrameError, MAX_PAYLOAD_SIZE};
pub use harness::{run_tool_as_subprocess, HarnessError};
// Phase 128 — multi-tool harness lifted from
// aivyx-gmail + aivyx-toolkit. The single-tool
// `HarnessError` re-export above stays canonical at the
// crate root; multi-tool consumers use the
// `multi_harness::HarnessError` path explicitly to
// disambiguate. Gmail / toolkit re-export from there.
pub use multi_harness::run_multi_tool_subprocess;
pub use proxy::ToolProxy;
pub use wire::{
    DaemonToTool, ToolDescriptor, ToolEventPayload, ToolToDaemon, Verification,
    TOOL_PROTOCOL_VERSION,
};
