//! MCP client adapter — Phase 23 Task 3.
//!
//! Bridges tools discovered from MCP servers into the Aivyx `Tool` trait.
//! Communicates over stdio using JSON-RPC 2.0, the standard MCP transport.

mod jsonrpc;
mod protocol;
mod proxy;
mod transport;

pub use proxy::McpToolProxy;
pub use transport::McpServerBridge;
