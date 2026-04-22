//! `aivyx mcp-server <name>` — bundled MCP server runner (Phase 46).
//!
//! Runs an MCP-compliant stdio server inside the `aivyx` binary.
//! Currently supports one server name: `"web-search"`.

/// Entry point called from `run()` in `aivyx.rs`.
///
/// Validates the server name and dispatches to the appropriate
/// server implementation. Returns `Err` for unknown names.
pub async fn run_mcp_server(name: &str) -> Result<(), String> {
    match name {
        "web-search" => {
            eprintln!("aivyx mcp-server: starting web-search server on stdio");
            // Task 2 will build the stdio harness here.
            Ok(())
        }
        other => Err(format!(
            "unknown MCP server name: `{other}`. Supported: web-search"
        )),
    }
}
