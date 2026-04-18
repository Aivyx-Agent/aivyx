//! Transport abstraction for MCP server communication.
//!
//! The trait captures the two-way JSON-RPC I/O that both
//! `McpServerBridge` (for lifecycle methods like `initialize`)
//! and `McpToolProxy` (for per-tool `tools/call`) need.
//! Implementors handle the physical layer (stdio pipes, HTTP+SSE)
//! while the bridge and proxy handle the JSON-RPC framing.

use async_trait::async_trait;

/// A bidirectional JSON-RPC transport for MCP communication.
///
/// `send` writes one newline-terminated JSON-RPC message.
/// `receive` reads one complete JSON-RPC response message.
///
/// Implementations must be `Send + Sync` because the transport
/// is shared (`Arc<dyn McpTransport>`) across the bridge and
/// all tool proxies discovered from the same server.
#[async_trait]
pub trait McpTransport: Send + Sync {
    /// Send a serialized JSON-RPC request (already newline-terminated).
    async fn send(&self, message: &str) -> Result<(), String>;

    /// Read one complete JSON-RPC response. Returns the raw JSON
    /// string (without trailing newline).
    async fn receive(&self) -> Result<String, String>;
}
