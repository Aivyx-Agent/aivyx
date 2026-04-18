//! MCP client adapter — Phase 23 Task 3, Phase 32 transport refactor.
//!
//! Bridges tools discovered from MCP servers into the Aivyx `Tool` trait.
//! Supports stdio transport (local child processes) and will support
//! SSE transport (remote HTTP servers) after Phase 32 Task 3.

mod jsonrpc;
mod protocol;
mod proxy;
pub mod stdio;
pub mod transport_trait;
mod transport;

pub use proxy::McpToolProxy;
pub use stdio::StdioTransport;
pub use transport::McpServerBridge;
pub use transport_trait::McpTransport;
